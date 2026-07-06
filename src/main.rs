mod config;
mod connection;
mod input;
mod session;

use egui::{ColorImage, TextureHandle, TextureOptions};

use config::{Config, Server};
use ironrdp_input::Operation;
use session::{SessionHandle, ToUi, ToWorker};

fn main() -> eframe::Result<()> {
    // Install the default rustls crypto provider once, so TLS setup on worker
    // threads does not panic due to an ambiguous/absent process default.
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "RustRDP",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}

/// Desktop resolution selection for a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    /// Adapt the remote desktop to the size of the drawing area.
    FitToWindow,
    /// A fixed resolution.
    Fixed(u16, u16),
}

impl Resolution {
    /// Preset options shown in the dropdown.
    const PRESETS: &'static [Resolution] = &[
        Resolution::FitToWindow,
        Resolution::Fixed(1920, 1080),
        Resolution::Fixed(1600, 900),
        Resolution::Fixed(1440, 900),
        Resolution::Fixed(1366, 768),
        Resolution::Fixed(1280, 800),
        Resolution::Fixed(1024, 768),
    ];

    fn label(self) -> String {
        match self {
            Resolution::FitToWindow => "Fit to window".to_owned(),
            Resolution::Fixed(w, h) => format!("{w} x {h}"),
        }
    }
}

/// Clamp a desired desktop size to the RDP-valid range (200..=8192) and make
/// the width even, as required by the Display Control protocol.
fn clamp_desktop((w, h): (u16, u16)) -> (u16, u16) {
    let w = w.clamp(200, 8192) & !1;
    let h = h.clamp(200, 8192);
    (w, h)
}

/// State of a single open connection tab.
struct ConnTab {
    server_name: String,
    handle: SessionHandle,
    texture: Option<TextureHandle>,
    desktop_size: (u16, u16),
    /// Last desktop resolution we asked the server to switch to.
    requested_size: (u16, u16),
    /// Chosen resolution mode.
    resolution: Resolution,
    status: String,
    connected: bool,
    // Keyboard scancodes currently held down, so we can release them.
    held_keys: std::collections::HashSet<u16>,
}

/// Modal editor state for adding/editing a server.
struct Editor {
    /// Index into config.servers being edited, or None if adding new.
    index: Option<usize>,
    server: Server,
}

struct App {
    config: Config,
    selected: Option<usize>,
    tabs: Vec<ConnTab>,
    active_tab: Option<usize>,
    editor: Option<Editor>,
    /// Last known size of the connection drawing area, used to pick the
    /// initial resolution when connecting in "Fit to window" mode.
    last_central_size: (u16, u16),
}

impl App {
    fn new() -> Self {
        Self {
            config: Config::load(),
            selected: None,
            tabs: Vec::new(),
            active_tab: None,
            editor: None,
            last_central_size: (1280, 800),
        }
    }

    fn connect(&mut self, index: usize) {
        let server = self.config.servers[index].clone();
        // Adapt the initial desktop size to the current drawing area so the
        // first frame already matches the window ("Fit to window" default).
        let (w, h) = clamp_desktop(self.last_central_size);
        let handle = session::spawn(server.clone(), w, h);
        self.tabs.push(ConnTab {
            server_name: server.name.clone(),
            handle,
            texture: None,
            desktop_size: (w, h),
            requested_size: (w, h),
            resolution: Resolution::FitToWindow,
            status: "Connecting...".to_owned(),
            connected: false,
            held_keys: std::collections::HashSet::new(),
        });
        self.active_tab = Some(self.tabs.len() - 1);
    }

    fn close_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            let tab = self.tabs.remove(i);
            let _ = tab.handle.to_worker.send(ToWorker::Shutdown);
            if self.tabs.is_empty() {
                self.active_tab = None;
            } else if let Some(active) = self.active_tab {
                self.active_tab = Some(active.min(self.tabs.len() - 1));
            }
        }
    }

    /// Drain worker messages for all tabs and update textures/status.
    fn pump_workers(&mut self, ctx: &egui::Context) {
        for tab in &mut self.tabs {
            loop {
                match tab.handle.from_worker.try_recv() {
                    Ok(ToUi::Connected { width, height }) => {
                        tab.desktop_size = (width, height);
                        // Treat the server-negotiated size as satisfying the
                        // current request, so we only resize again when the UI
                        // area actually changes.
                        tab.requested_size = (width, height);
                        tab.connected = true;
                        tab.status = format!("Connected ({width}x{height})");
                    }
                    Ok(ToUi::Frame { width, height, rgba }) => {
                        let image = ColorImage::from_rgba_premultiplied(
                            [width as usize, height as usize],
                            &rgba,
                        );
                        match &mut tab.texture {
                            Some(tex) => tex.set(image, TextureOptions::NEAREST),
                            None => {
                                tab.texture = Some(ctx.load_texture(
                                    format!("rdp-{}", tab.server_name),
                                    image,
                                    TextureOptions::NEAREST,
                                ));
                            }
                        }
                    }
                    Ok(ToUi::Disconnected(reason)) => {
                        tab.connected = false;
                        tab.status = format!("Disconnected: {reason}");
                    }
                    Ok(ToUi::Error(e)) => {
                        tab.connected = false;
                        tab.status = format!("Error: {e}");
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump_workers(&ctx);

        // Keep repainting so live frames flow in.
        ctx.request_repaint_after(std::time::Duration::from_millis(16));

        self.left_panel(ui);
        self.central_panel(ui);
        self.editor_window(&ctx);
    }
}

impl App {
    fn left_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("servers")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                ui.heading("Servers");
                ui.separator();

                let mut connect_index = None;
                for (i, server) in self.config.servers.iter().enumerate() {
                    let selected = self.selected == Some(i);
                    let label = if server.name.is_empty() {
                        server.host.clone()
                    } else {
                        server.name.clone()
                    };
                    let resp = ui.selectable_label(selected, label);
                    if resp.clicked() {
                        self.selected = Some(i);
                    }
                    if resp.double_clicked() {
                        connect_index = Some(i);
                    }
                }
                if let Some(i) = connect_index {
                    self.connect(i);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() {
                        self.editor = Some(Editor {
                            index: None,
                            server: Server::default(),
                        });
                    }
                    if ui.button("Edit").clicked() {
                        if let Some(i) = self.selected {
                            self.editor = Some(Editor {
                                index: Some(i),
                                server: self.config.servers[i].clone(),
                            });
                        }
                    }
                    if ui.button("Remove").clicked() {
                        if let Some(i) = self.selected {
                            self.config.servers.remove(i);
                            self.selected = None;
                            let _ = self.config.save();
                        }
                    }
                });
                if ui.button("Connect").clicked() {
                    if let Some(i) = self.selected {
                        self.connect(i);
                    }
                }
            });
    }

    fn central_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            // Tab bar.
            let mut close_tab = None;
            ui.horizontal(|ui| {
                for i in 0..self.tabs.len() {
                    let active = self.active_tab == Some(i);
                    let name = self.tabs[i].server_name.clone();
                    if ui.selectable_label(active, &name).clicked() {
                        self.active_tab = Some(i);
                    }
                    if ui.small_button("x").clicked() {
                        close_tab = Some(i);
                    }
                }
            });
            if let Some(i) = close_tab {
                self.close_tab(i);
            }

            // Resolution selector for the active tab.
            if let Some(active) = self.active_tab {
                if active < self.tabs.len() {
                    let tab = &mut self.tabs[active];
                    ui.horizontal(|ui| {
                        ui.label("Resolution:");
                        egui::ComboBox::from_id_salt("resolution")
                            .selected_text(tab.resolution.label())
                            .show_ui(ui, |ui| {
                                for preset in Resolution::PRESETS {
                                    ui.selectable_value(
                                        &mut tab.resolution,
                                        *preset,
                                        preset.label(),
                                    );
                                }
                            });
                    });
                }
            }

            ui.separator();

            // Remember the drawing area so a new connection can adapt to it.
            let avail = ui.available_size();
            self.last_central_size = (
                (avail.x.max(1.0) as u16).max(1),
                (avail.y.max(1.0) as u16).max(1),
            );

            if let Some(active) = self.active_tab {
                if active < self.tabs.len() {
                    Self::render_tab(ui, &mut self.tabs[active]);
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a server and click Connect.");
                });
            }
        });
    }

    fn render_tab(ui: &mut egui::Ui, tab: &mut ConnTab) {
        ui.label(&tab.status);

        // Determine the desired remote resolution and request it from the
        // server (debounced in the worker). In "Fit to window" mode this
        // tracks the drawing area; otherwise it is the chosen fixed size.
        let avail = ui.available_size();
        let (target_w, target_h) = match tab.resolution {
            Resolution::FitToWindow => clamp_desktop((avail.x as u16, avail.y as u16)),
            Resolution::Fixed(w, h) => clamp_desktop((w, h)),
        };
        // Small tolerance avoids oscillation from even-width rounding done by
        // the server during negotiation.
        let (rw, rh) = tab.requested_size;
        let changed = target_w.abs_diff(rw) > 2 || target_h.abs_diff(rh) > 2;
        if tab.connected && changed {
            tab.requested_size = (target_w, target_h);
            let _ = tab.handle.to_worker.send(ToWorker::Resize {
                width: target_w,
                height: target_h,
            });
        }

        let Some(texture) = &tab.texture else {
            return;
        };

        let (dw, dh) = tab.desktop_size;
        // Fit while preserving aspect ratio (no upscaling beyond the desktop).
        let scale = (avail.x / dw as f32).min(avail.y / dh as f32).min(1.0);
        let display_size = egui::vec2(dw as f32 * scale, dh as f32 * scale);

        let image = egui::Image::new(texture)
            .fit_to_exact_size(display_size)
            .sense(egui::Sense::click_and_drag());
        let resp = ui.add(image);

        let mut ops: Vec<Operation> = Vec::new();

        // Map pointer position within the image to desktop coordinates.
        let rect = resp.rect;
        if let Some(pos) = resp.hover_pos() {
            let rel_x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let rel_y = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            let x = (rel_x * dw as f32) as u16;
            let y = (rel_y * dh as f32) as u16;
            ops.push(input::mouse_move(x, y));
        }

        // Mouse buttons and keyboard are handled via raw input events below,
        // for precise press/release semantics.
        ui.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::PointerButton {
                        button, pressed, ..
                    } => {
                        if resp.hovered() || resp.is_pointer_button_down_on() {
                            if let Some(mb) = input::mouse_button(*button) {
                                if *pressed {
                                    ops.push(Operation::MouseButtonPressed(mb));
                                } else {
                                    ops.push(Operation::MouseButtonReleased(mb));
                                }
                            }
                        }
                    }
                    egui::Event::MouseWheel { delta, .. } => {
                        if resp.hovered() {
                            ops.extend(input::wheel(delta.x, delta.y));
                        }
                    }
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers,
                        ..
                    } => {
                        // Apply modifier keys around the main key.
                        let mods = input::modifier_scancodes(modifiers);
                        if *pressed {
                            for m in &mods {
                                if tab.held_keys.insert(*m) {
                                    ops.push(input::key_pressed(*m));
                                }
                            }
                            if let Some(sc) = input::key_scancode(*key) {
                                tab.held_keys.insert(sc);
                                ops.push(input::key_pressed(sc));
                            }
                        } else {
                            if let Some(sc) = input::key_scancode(*key) {
                                tab.held_keys.remove(&sc);
                                ops.push(input::key_released(sc));
                            }
                            for m in &mods {
                                if tab.held_keys.remove(m) {
                                    ops.push(input::key_released(*m));
                                }
                            }
                        }
                    }
                    egui::Event::Text(text) => {
                        for ch in text.chars() {
                            ops.push(Operation::UnicodeKeyPressed(ch));
                            ops.push(Operation::UnicodeKeyReleased(ch));
                        }
                    }
                    _ => {}
                }
            }
        });

        if !ops.is_empty() && tab.connected {
            let _ = tab.handle.to_worker.send(ToWorker::Input(ops));
        }
    }

    fn editor_window(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;

        egui::Window::new(if editor.index.is_some() {
            "Edit Server"
        } else {
            "Add Server"
        })
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            egui::Grid::new("editor_grid").num_columns(2).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut editor.server.name);
                ui.end_row();

                ui.label("Host");
                ui.text_edit_singleline(&mut editor.server.host);
                ui.end_row();

                ui.label("Port");
                let mut port_str = editor.server.port.to_string();
                if ui.text_edit_singleline(&mut port_str).changed() {
                    if let Ok(p) = port_str.parse::<u16>() {
                        editor.server.port = p;
                    }
                }
                ui.end_row();

                ui.label("Username");
                ui.text_edit_singleline(&mut editor.server.username);
                ui.end_row();

                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut editor.server.password).password(true));
                ui.end_row();

                ui.label("Domain");
                ui.text_edit_singleline(&mut editor.server.domain);
                ui.end_row();
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

        if save {
            match editor.index {
                Some(i) => self.config.servers[i] = editor.server.clone(),
                None => self.config.servers.push(editor.server.clone()),
            }
            let _ = self.config.save();
        } else if !cancel && open {
            // Window still open, keep editing.
            self.editor = Some(editor);
        }
    }
}
