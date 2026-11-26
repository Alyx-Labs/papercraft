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

#[cfg(target_os = "windows")]
struct HiddenWindowShield {
    window: Option<std::sync::Arc<winit::window::Window>>,
    cloaked: bool,
    needs_taskbar_restore: bool,
}

#[cfg(target_os = "windows")]
impl HiddenWindowShield {
    fn new(cc: &eframe::CreationContext<'_>, start_hidden: bool) -> Self {
        let mut shield = Self {
            window: cc.winit_window().cloned(),
            cloaked: false,
            needs_taskbar_restore: false,
        };
        if start_hidden {
            shield.apply();
        }
        shield
    }

    fn hwnd_raw(&self) -> Option<isize> {
        use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        let window = self.window.as_ref()?;
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        }
    }

    fn apply(&mut self) {
        use winit::platform::windows::WindowExtWindows as _;
        if let Some(ref window) = self.window {
            window.set_skip_taskbar(true);
            if let Some(hwnd) = self.hwnd_raw()
                && papercraft_tablet::windows::set_window_cloaked(hwnd, true)
            {
                self.cloaked = true;
            }
            self.needs_taskbar_restore = true;
        }
    }

    fn prepare_show(&mut self) {
        use winit::platform::windows::WindowExtWindows as _;
        if self.cloaked {
            if let Some(hwnd) = self.hwnd_raw() {
                let _ = papercraft_tablet::windows::set_window_cloaked(hwnd, false);
            }
            self.cloaked = false;
        }
        if self.needs_taskbar_restore
            && let Some(ref window) = self.window
        {
            window.set_skip_taskbar(false);
        }
    }

    fn sync_taskbar_when_visible(&mut self) {
        use winit::platform::windows::WindowExtWindows as _;
        if self.needs_taskbar_restore
            && let Some(ref window) = self.window
            && window.is_visible() == Some(true)
        {
            window.set_skip_taskbar(false);
            self.needs_taskbar_restore = false;
        }
    }

    fn is_os_visible(&self) -> Option<bool> {
        self.window.as_ref().and_then(|w| w.is_visible())
    }
}

#[cfg(not(target_os = "windows"))]
struct HiddenWindowShield;

#[cfg(not(target_os = "windows"))]
impl HiddenWindowShield {
    const fn new(_cc: &eframe::CreationContext<'_>, _start_hidden: bool) -> Self {
        Self
    }

    fn prepare_show(&mut self) {}

    fn sync_taskbar_when_visible(&mut self) {}

    fn is_os_visible(&self) -> Option<bool> {
        None
    }
}

struct ServerControlApp {
    controller: ServerController,
    tray: Option<TrayController>,
    tray_rx: mpsc::Receiver<TrayAction>,
    needs_initial_hide: bool,
    hidden_shield: HiddenWindowShield,
}

impl ServerControlApp {
    fn show_window(&mut self, ctx: &egui::Context) {
        self.needs_initial_hide = false;
        self.hidden_shield.prepare_show();
        self.controller.window_visible = true;
        self.controller.refresh_stats();
        self.controller.refresh_users();
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        self.needs_initial_hide = false;
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

    fn enforce_hidden_window_state(&mut self, ctx: &egui::Context) {
        if self.controller.window_visible {
            self.hidden_shield.sync_taskbar_when_visible();
        } else if self.tray.is_some()
            && !self.needs_initial_hide
            && self.hidden_shield.is_os_visible() == Some(true)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
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

        self.enforce_hidden_window_state(ctx);

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
        // eframe's first-frame `post_rendering` always calls `window.set_visible(true)`.
        // Sending `Visible(false)` on that same frame keeps startup hidden on every platform.
        if !self.controller.window_visible && self.tray.is_some() {
            if self.needs_initial_hide {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.needs_initial_hide = false;
            }
            return;
        }

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

fn is_autostart_launch<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .skip(1)
        .any(|arg| arg.as_ref() == papercraft_updater::AUTOSTART_ARG)
}

fn register_platform_hooks(controller: &mut ServerController) {
    controller.set_desktop_shortcut_sync(Box::new(|enabled| {
        papercraft_updater::sync_desktop_shortcut(
            papercraft_updater::AppId::PaperCraftServer,
            enabled,
        )
        .map(|outcome| outcome.status_message())
    }));
    controller.set_autostart_sync(Box::new(|enabled, clear_os_override| {
        papercraft_updater::sync_autostart_with_options(
            papercraft_updater::AppId::PaperCraftServer,
            enabled,
            clear_os_override,
        )
        .map(|outcome| outcome.status_message())
    }));
}

fn main() -> eframe::Result {
    papercraft_updater::cleanup_stale_files(papercraft_updater::AppId::PaperCraftServer);
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let config_path = settings::default_config_path();
    let autostart_launch = is_autostart_launch(std::env::args());
    let mut controller = ServerController::new_for_launch(config_path, true, autostart_launch);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PaperCraft Server")
            .with_app_id(papercraft_updater::AppId::PaperCraftServer.manifest_key())
            .with_inner_size([560.0, 680.0])
            .with_min_inner_size([460.0, 520.0])
            .with_visible(controller.window_visible)
            .with_active(controller.window_visible)
            // TODO(branding): Replace shared PaperCraft icon bytes with a dedicated PaperCraft Server icon once designed.
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
            register_platform_hooks(&mut controller);
            controller.sync_desktop_shortcut_on_startup_if_enabled();
            controller.sync_autostart_on_startup_if_enabled();
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
                cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            let start_hidden = !controller.window_visible && tray.is_some();
            let hidden_shield = HiddenWindowShield::new(cc, start_hidden);
            Ok(Box::new(ServerControlApp {
                controller,
                tray,
                tray_rx,
                needs_initial_hide: start_hidden,
                hidden_shield,
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::is_autostart_launch;

    #[test]
    fn autostart_arg_detection_checks_cli_flags_only() {
        assert!(!is_autostart_launch(["papercraft-server.exe"]));
        assert!(!is_autostart_launch([papercraft_updater::AUTOSTART_ARG]));
        assert!(is_autostart_launch([
            "papercraft-server.exe",
            papercraft_updater::AUTOSTART_ARG
        ]));
        assert!(!is_autostart_launch(["papercraft-server.exe", "--other"]));
    }
}
