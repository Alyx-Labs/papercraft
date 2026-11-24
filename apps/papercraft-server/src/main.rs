#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod controller;
mod icons;
mod platform;
mod settings;
mod theme;
mod tray;
mod ui;

use std::sync::mpsc;
use std::time::Duration;

use controller::ServerController;
use tray::{TrayAction, TrayController};

struct ServerControlApp {
    controller: ServerController,
    tray: Option<TrayController>,
    tray_rx: mpsc::Receiver<TrayAction>,
}

impl ServerControlApp {
    fn show_window(&mut self, ctx: &egui::Context) {
        self.controller.window_visible = true;
        self.controller.refresh_stats();
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        self.controller.window_visible = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn quit_cleanly(&mut self, ctx: &egui::Context) {
        self.controller.allow_close = true;
        self.controller.auto_save_settings();
        self.controller.stop_server();
        self.tray = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for ServerControlApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.controller.allow_close
            && self.tray.is_some()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(ctx);
        }

        self.controller.poll_background_events();

        while let Ok(action) = self.tray_rx.try_recv() {
            let (vis_change, should_quit) = self.controller.handle_tray_action(action);
            if should_quit {
                self.tray = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            match vis_change {
                Some(true) => self.show_window(ctx),
                Some(false) => self.hide_window(ctx),
                None => {}
            }
        }

        if let Some(ref mut tray) = self.tray {
            tray.sync_state(self.controller.tray_visual_state());
        }

        if self.controller.copied_url_feedback.is_some()
            || self.controller.is_picking_folder()
            || self.controller.updater.is_busy()
        {
            ctx.request_repaint_after(Duration::from_millis(500));
        } else if self.controller.window_visible && self.controller.is_running() {
            ctx.request_repaint_after(Duration::from_secs(5));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let theme = self.controller.theme();
        theme::apply_visuals(ui.ctx(), theme);
        let tokens = theme::Tokens::for_theme(theme);
        let actions = ui::render_window(&mut self.controller, ui, &tokens);

        if actions.quit_app {
            self.quit_cleanly(ui.ctx());
        } else if actions.install_update {
            let was_running = self.controller.is_running();
            self.controller.auto_save_settings();
            self.controller.stop_server();
            match self.controller.apply_ready_update_and_relaunch() {
                Ok(()) => {
                    self.controller.allow_close = true;
                    self.tray = None;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(err) => {
                    self.controller.notice_banner = Some(controller::NoticeBanner {
                        tone: controller::NoticeTone::Error,
                        text: format!("Could not install update: {err}"),
                    });
                    if was_running {
                        self.controller.start_server();
                    }
                }
            }
        } else if actions.hide_to_tray {
            if self.tray.is_some() {
                self.hide_window(ui.ctx());
            } else {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
    }

    fn on_exit(&mut self) {
        self.controller.auto_save_settings();
        self.controller.stop_server();
        self.tray = None;
    }
}

fn main() -> eframe::Result {
    papercraft_updater::cleanup_stale_files(papercraft_updater::AppId::PaperCraftServer);
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let config_path = settings::default_config_path();
    let mut controller = ServerController::new(config_path, true);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PaperCraft Server")
            .with_inner_size([560.0, 680.0])
            .with_min_inner_size([460.0, 520.0])
            .with_visible(controller.window_visible)
            .with_icon(egui::IconData {
                rgba: include_bytes!("../../papercraft/assets/branding/papercraft-256.rgba")
                    .to_vec(),
                width: 256,
                height: 256,
            }),
        ..Default::default()
    };

    eframe::run_native(
        "PaperCraft Server",
        native_options,
        Box::new(move |cc| {
            controller.set_repaint_ctx(cc.egui_ctx.clone());
            controller.start_startup_update_check_if_enabled();
            let (tray_tx, tray_rx) = mpsc::channel();
            let tray = TrayController::try_new(
                controller.tray_visual_state(),
                tray_tx,
                cc.egui_ctx.clone(),
            );
            if tray.is_none() && !controller.window_visible {
                controller.window_visible = true;
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
            Ok(Box::new(ServerControlApp {
                controller,
                tray,
                tray_rx,
            }))
        }),
    )
}
