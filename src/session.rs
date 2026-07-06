//! RDP worker thread and UI <-> worker message channels.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use ironrdp_input::Operation;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageOutput};

use crate::config::Server;
use crate::connection;

/// Messages from the worker thread to the UI.
pub enum ToUi {
    Connected { width: u16, height: u16 },
    /// Full RGBA framebuffer snapshot (width * height * 4 bytes).
    Frame {
        width: u16,
        height: u16,
        rgba: Vec<u8>,
    },
    Disconnected(String),
    Error(String),
}

/// Messages from the UI to the worker thread.
pub enum ToWorker {
    Input(Vec<Operation>),
    Shutdown,
}

pub struct SessionHandle {
    pub to_worker: Sender<ToWorker>,
    pub from_worker: Receiver<ToUi>,
}

pub fn spawn(server: Server, width: u16, height: u16) -> SessionHandle {
    let (to_worker_tx, to_worker_rx) = std::sync::mpsc::channel::<ToWorker>();
    let (to_ui_tx, to_ui_rx) = std::sync::mpsc::channel::<ToUi>();

    std::thread::spawn(move || {
        if let Err(e) = run(server, width, height, &to_ui_tx, &to_worker_rx) {
            let _ = to_ui_tx.send(ToUi::Error(format!("{e:#}")));
        }
    });

    SessionHandle {
        to_worker: to_worker_tx,
        from_worker: to_ui_rx,
    }
}

fn run(
    server: Server,
    width: u16,
    height: u16,
    to_ui: &Sender<ToUi>,
    from_ui: &Receiver<ToWorker>,
) -> anyhow::Result<()> {
    let config = connection::build_config(&server, width, height);
    let (connection_result, mut framed) = connection::connect(config, server.host.clone(), server.port)?;

    let desktop = connection_result.desktop_size;
    let mut image = DecodedImage::new(
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        desktop.width,
        desktop.height,
    );

    let _ = to_ui.send(ToUi::Connected {
        width: desktop.width,
        height: desktop.height,
    });

    let mut active_stage = ActiveStage::new(connection_result);
    let mut input_db = ironrdp_input::Database::new();

    // Send an initial (blank) frame so the UI has a texture.
    send_frame(to_ui, &image);

    loop {
        // Drain any pending input from the UI and forward it to the server.
        loop {
            match from_ui.try_recv() {
                Ok(ToWorker::Input(ops)) => {
                    let events = input_db.apply(ops);
                    if !events.is_empty() {
                        let outputs = active_stage.process_fastpath_input(&mut image, &events)?;
                        for out in outputs {
                            if let ActiveStageOutput::ResponseFrame(frame) = out {
                                framed.write_all(&frame)?;
                            }
                        }
                    }
                }
                Ok(ToWorker::Shutdown) => return Ok(()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        // Read one PDU from the server (short read timeout keeps this responsive).
        let (action, payload) = match framed.read_pdu() {
            Ok((action, payload)) => (action, payload),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => return Err(anyhow::Error::new(e).context("read frame")),
        };

        let outputs = active_stage.process(&mut image, action, &payload)?;

        let mut dirty = false;
        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => framed.write_all(&frame)?,
                ActiveStageOutput::GraphicsUpdate(_) => dirty = true,
                ActiveStageOutput::Terminate(reason) => {
                    let _ = to_ui.send(ToUi::Disconnected(reason.description()));
                    return Ok(());
                }
                _ => {}
            }
        }

        if dirty {
            send_frame(to_ui, &image);
        }
    }
}

fn send_frame(to_ui: &Sender<ToUi>, image: &DecodedImage) {
    let _ = to_ui.send(ToUi::Frame {
        width: image.width(),
        height: image.height(),
        rgba: image.data().to_vec(),
    });
}
