//! RDP worker thread and UI <-> worker message channels.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_connector::connection_activation::ConnectionActivationState;
use ironrdp_core::WriteBuf;
use ironrdp_input::Operation;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageOutput};

use crate::clipboard::ClipboardEvent;
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
    /// Request the remote desktop be resized to the given dimensions.
    Resize { width: u16, height: u16 },
    Shutdown,
}

pub struct SessionHandle {
    pub to_worker: Sender<ToWorker>,
    pub from_worker: Receiver<ToUi>,
}

pub fn spawn(server: Server, username: String, password: String, domain: String, clipboard_passthrough: bool, width: u16, height: u16) -> SessionHandle {
    let (to_worker_tx, to_worker_rx) = std::sync::mpsc::channel::<ToWorker>();
    let (to_ui_tx, to_ui_rx) = std::sync::mpsc::channel::<ToUi>();

    std::thread::spawn(move || {
        if let Err(e) = run(server, username, password, domain, clipboard_passthrough, width, height, &to_ui_tx, &to_worker_rx) {
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
    username: String,
    password: String,
    domain: String,
    clipboard_passthrough: bool,
    width: u16,
    height: u16,
    to_ui: &Sender<ToUi>,
    from_ui: &Receiver<ToWorker>,
) -> anyhow::Result<()> {
    // If clipboard passthrough is enabled, create a channel the CliprdrBackend
    // uses to send events back to this run loop.
    let (clipboard_tx, clipboard_rx) = if clipboard_passthrough {
        let (tx, rx) = std::sync::mpsc::channel::<ClipboardEvent>();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let config = connection::build_config(&server, &username, &password, &domain, width, height);
    let (connection_result, mut framed) = connection::connect(config, server.host.clone(), server.port, clipboard_tx)?;

    // Now that the handshake is done, switch to a short read timeout so the
    // active-stage loop stays responsive to input and shutdown requests.
    connection::set_read_timeout(&mut framed, Some(std::time::Duration::from_millis(16)))?;

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

    // Debounced pending resize request. We only forward one resize to the
    // server after the UI stops changing size for a short interval, so we
    // don't flood the server during a live window drag.
    let mut pending_resize: Option<(u16, u16)> = None;
    let mut last_sent_size: (u16, u16) = (desktop.width, desktop.height);
    let mut resize_deadline: Option<std::time::Instant> = None;
    const RESIZE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

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
                Ok(ToWorker::Resize { width, height }) => {
                    // Coalesce; only act once the size settles (debounce).
                    if (width, height) != last_sent_size {
                        pending_resize = Some((width, height));
                        resize_deadline = Some(std::time::Instant::now() + RESIZE_DEBOUNCE);
                    }
                }
                Ok(ToWorker::Shutdown) => return Ok(()),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        // Drain clipboard events produced by the CliprdrBackend callbacks and
        // forward them to the server via the CliprdrClient.
        if let Some(ref rx) = clipboard_rx {
            loop {
                match rx.try_recv() {
                    Ok(ClipboardEvent::InitiateCopy(formats)) => {
                        if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                            if let Ok(msgs) = cliprdr.initiate_copy(&formats) {
                                if let Ok(bytes) = active_stage.process_svc_processor_messages(msgs) {
                                    framed.write_all(&bytes)?;
                                }
                            }
                        }
                    }
                    Ok(ClipboardEvent::SubmitFormatData(response)) => {
                        if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                            if let Ok(msgs) = cliprdr.submit_format_data(response) {
                                if let Ok(bytes) = active_stage.process_svc_processor_messages(msgs) {
                                    framed.write_all(&bytes)?;
                                }
                            }
                        }
                    }
                    Ok(ClipboardEvent::RequestFormatData(format_id)) => {
                        if let Some(cliprdr) = active_stage.get_svc_processor_mut::<CliprdrClient>() {
                            if let Ok(msgs) = cliprdr.initiate_paste(format_id) {
                                if let Ok(bytes) = active_stage.process_svc_processor_messages(msgs) {
                                    framed.write_all(&bytes)?;
                                }
                            }
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
        }

        // If a debounced resize is due, send it to the server.
        if let (Some((w, h)), Some(deadline)) = (pending_resize, resize_deadline) {
            if std::time::Instant::now() >= deadline {
                pending_resize = None;
                resize_deadline = None;
                // adjust_display_size enforces the protocol's valid range and
                // even-width requirement.
                let (aw, ah) = ironrdp_displaycontrol::pdu::MonitorLayoutEntry::adjust_display_size(
                    u32::from(w),
                    u32::from(h),
                );
                if let Some(res) = active_stage.encode_resize(aw, ah, None, None) {
                    let frame = res?;
                    framed.write_all(&frame)?;
                    last_sent_size = (w, h);
                }
                // If encode_resize returned None the DVC isn't ready yet; the
                // next UI resize event will retry.
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
                ActiveStageOutput::DeactivateAll(mut cas) => {
                    // The server accepted a resize (or otherwise wants to
                    // reactivate). Drive the reactivation sequence to
                    // completion, then rebuild our image at the new size.
                    let new_size = reactivate(&mut framed, &mut cas)?;
                    if let Some((share_id, w, h)) = new_size {
                        active_stage.set_share_id(share_id);
                        image = DecodedImage::new(
                            ironrdp_graphics::image_processing::PixelFormat::RgbA32,
                            w,
                            h,
                        );
                        last_sent_size = (w, h);
                        let _ = to_ui.send(ToUi::Connected { width: w, height: h });
                        send_frame(to_ui, &image);
                    }
                }
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

/// Drive a deactivation-reactivation sequence to completion.
///
/// Returns the new `(share_id, width, height)` on success.
fn reactivate(
    framed: &mut connection::UpgradedFramed,
    cas: &mut ironrdp_connector::connection_activation::ConnectionActivationSequence,
) -> anyhow::Result<Option<(u32, u16, u16)>> {
    // The reactivation handshake needs blocking reads (like the initial
    // connection). Clear the short active-stage read timeout for the duration,
    // then restore it before returning to the active-stage loop.
    connection::set_read_timeout(framed, None)?;
    let result = drive_reactivation(framed, cas);
    connection::set_read_timeout(framed, Some(std::time::Duration::from_millis(16)))?;
    result
}

fn drive_reactivation(
    framed: &mut connection::UpgradedFramed,
    cas: &mut ironrdp_connector::connection_activation::ConnectionActivationSequence,
) -> anyhow::Result<Option<(u32, u16, u16)>> {
    use ironrdp_connector::Sequence as _;

    let mut buf = WriteBuf::new();
    loop {
        buf.clear();
        let written = if let Some(hint) = cas.next_pdu_hint() {
            let pdu = framed.read_by_hint(hint)?;
            cas.step(&pdu, &mut buf)?
        } else {
            cas.step_no_input(&mut buf)?
        };
        if let Some(n) = written.size() {
            framed.write_all(&buf[..n])?;
        }
        if cas.state().is_terminal() {
            break;
        }
    }

    if let ConnectionActivationState::Finalized {
        desktop_size,
        share_id,
        ..
    } = cas.connection_activation_state()
    {
        Ok(Some((share_id, desktop_size.width, desktop_size.height)))
    } else {
        Ok(None)
    }
}

fn send_frame(to_ui: &Sender<ToUi>, image: &DecodedImage) {
    let _ = to_ui.send(ToUi::Frame {
        width: image.width(),
        height: image.height(),
        rgba: image.data().to_vec(),
    });
}
