//! Cross-platform OS system-tray icon, menu, and status badge builder.

use std::sync::mpsc;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

// TODO(branding): Replace shared PaperCraft RGBA icon with a dedicated PaperCraft Server logo once designed.
const LOGO_RGBA_256: &[u8] =
    include_bytes!("../../../apps/papercraft/assets/branding/papercraft-256.rgba");
const LOGO_DIM: usize = 256;
const TRAY_DIM: usize = 32;
const WINDOWS_TRAY_GUID: u128 = 0x9b4d_7c1e_3f0a_4c89_a5b2_e814_d62c_9107;

const ID_STATUS: &str = "bnc_srv_status";
const ID_OPEN: &str = "bnc_srv_open";
const ID_HIDE: &str = "bnc_srv_hide";
const ID_TOGGLE_SERVER: &str = "bnc_srv_toggle";
const ID_RESTART_SERVER: &str = "bnc_srv_restart";
const ID_COPY_ADDRESS: &str = "bnc_srv_copy";
const ID_OPEN_FOLDER: &str = "bnc_srv_folder";
const ID_QUIT: &str = "bnc_srv_quit";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayStatusKind {
    Running,
    Stopped,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayVisualState {
    pub kind: TrayStatusKind,
    pub status_line: String,
    pub tooltip: String,
    pub window_visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow,
    HideWindow,
    ToggleServer,
    RestartServer,
    CopyAddress,
    OpenDataFolder,
    Quit,
}

pub struct TrayController {
    tray: TrayIcon,
    status_item: MenuItem,
    hide_item: MenuItem,
    toggle_item: MenuItem,
    restart_item: MenuItem,
    last_state: Option<TrayVisualState>,
}

impl TrayController {
    /// Creates the OS system-tray icon and registers non-blocking event handlers.
    ///
    /// Returns `None` if the environment has no tray host (for example headless tests).
    pub fn try_new(
        initial: TrayVisualState,
        tx: mpsc::Sender<TrayAction>,
        ctx: egui::Context,
    ) -> Option<Self> {
        let tray_tx = tx.clone();
        let tray_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            let activate = matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            );
            if activate {
                let _ = tray_tx.send(TrayAction::ShowWindow);
                tray_ctx.request_repaint();
            }
        }));

        MenuEvent::set_event_handler(Some(move |event: tray_icon::menu::MenuEvent| {
            if let Some(action) = action_for_menu_id(event.id.as_ref()) {
                let _ = tx.send(action);
                ctx.request_repaint();
            }
        }));

        let menu = Menu::new();
        let status_item = MenuItem::with_id(ID_STATUS, &initial.status_line, false, None);
        let open_item = MenuItem::with_id(ID_OPEN, "Open Control Window", true, None);
        let hide_item = MenuItem::with_id(ID_HIDE, "Hide Window", initial.window_visible, None);
        let toggle_label = if initial.kind == TrayStatusKind::Running {
            "Stop Server"
        } else {
            "Start Server"
        };
        let toggle_item = MenuItem::with_id(ID_TOGGLE_SERVER, toggle_label, true, None);
        let restart_item = MenuItem::with_id(
            ID_RESTART_SERVER,
            "Restart Server",
            initial.kind == TrayStatusKind::Running,
            None,
        );
        let copy_item = MenuItem::with_id(ID_COPY_ADDRESS, "Copy Server Address", true, None);
        let folder_item = MenuItem::with_id(ID_OPEN_FOLDER, "Open Data Folder", true, None);
        let quit_item = MenuItem::with_id(ID_QUIT, "Quit PaperCraft Server", true, None);

        let sep1 = PredefinedMenuItem::separator();
        let sep2 = PredefinedMenuItem::separator();
        let sep3 = PredefinedMenuItem::separator();
        let _ = menu.append_items(&[
            &status_item,
            &sep1,
            &open_item,
            &hide_item,
            &sep2,
            &toggle_item,
            &restart_item,
            &copy_item,
            &folder_item,
            &sep3,
            &quit_item,
        ]);

        let app_id = papercraft_updater::AppId::PaperCraftServer.bundle_identifier();
        let mut builder = TrayIconBuilder::new()
            .with_id(app_id)
            .with_autosave_name(app_id)
            .with_menu(Box::new(menu))
            .with_tooltip(&initial.tooltip)
            .with_menu_on_left_click(false)
            .with_guid(WINDOWS_TRAY_GUID);

        if let Some(icon) = build_tray_icon(initial.kind) {
            builder = builder.with_icon(icon);
        }

        match builder.build() {
            Ok(tray) => Some(Self {
                tray,
                status_item,
                hide_item,
                toggle_item,
                restart_item,
                last_state: Some(initial),
            }),
            Err(err) => {
                log::warn!("System tray unavailable; running with control window only: {err}");
                None
            }
        }
    }

    /// Synchronizes the tray icon badge, tooltip, and context-menu labels with current state.
    pub fn sync_state(&mut self, next: TrayVisualState) {
        if self.last_state.as_ref() == Some(&next) {
            return;
        }
        let prev_kind = self.last_state.as_ref().map(|s| s.kind);
        self.status_item.set_text(&next.status_line);
        self.hide_item.set_enabled(next.window_visible);
        let running = next.kind == TrayStatusKind::Running;
        self.toggle_item.set_text(if running {
            "Stop Server"
        } else {
            "Start Server"
        });
        self.restart_item.set_enabled(running);
        let _ = self.tray.set_tooltip(Some(&next.tooltip));
        if prev_kind != Some(next.kind) {
            let _ = self.tray.set_icon(build_tray_icon(next.kind));
        }
        self.last_state = Some(next);
    }
}

fn action_for_menu_id(id: &str) -> Option<TrayAction> {
    match id {
        ID_OPEN => Some(TrayAction::ShowWindow),
        ID_HIDE => Some(TrayAction::HideWindow),
        ID_TOGGLE_SERVER => Some(TrayAction::ToggleServer),
        ID_RESTART_SERVER => Some(TrayAction::RestartServer),
        ID_COPY_ADDRESS => Some(TrayAction::CopyAddress),
        ID_OPEN_FOLDER => Some(TrayAction::OpenDataFolder),
        ID_QUIT => Some(TrayAction::Quit),
        _ => None,
    }
}

/// Generates a crisp 32x32 RGBA tray icon with a bottom-right status indicator dot.
pub fn build_tray_icon(kind: TrayStatusKind) -> Option<tray_icon::Icon> {
    let rgba = render_tray_rgba(kind);
    tray_icon::Icon::from_rgba(rgba, TRAY_DIM as u32, TRAY_DIM as u32).ok()
}

pub fn render_tray_rgba(kind: TrayStatusKind) -> Vec<u8> {
    let mut out = vec![0_u8; TRAY_DIM * TRAY_DIM * 4];
    let scale = LOGO_DIM / TRAY_DIM; // 8
    let samples = (scale * scale) as f32;

    if LOGO_RGBA_256.len() == LOGO_DIM * LOGO_DIM * 4 {
        for ty in 0..TRAY_DIM {
            for tx in 0..TRAY_DIM {
                let mut r_sum = 0.0_f32;
                let mut g_sum = 0.0_f32;
                let mut b_sum = 0.0_f32;
                let mut a_sum = 0.0_f32;
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = ((ty * scale + sy) * LOGO_DIM + (tx * scale + sx)) * 4;
                        let a = f32::from(LOGO_RGBA_256[px + 3]) / 255.0;
                        r_sum += f32::from(LOGO_RGBA_256[px]) * a;
                        g_sum += f32::from(LOGO_RGBA_256[px + 1]) * a;
                        b_sum += f32::from(LOGO_RGBA_256[px + 2]) * a;
                        a_sum += a;
                    }
                }
                let idx = (ty * TRAY_DIM + tx) * 4;
                let a_avg = (a_sum / samples).clamp(0.0, 1.0);
                if a_sum > 1e-4 {
                    out[idx] = (r_sum / a_sum).round().clamp(0.0, 255.0) as u8;
                    out[idx + 1] = (g_sum / a_sum).round().clamp(0.0, 255.0) as u8;
                    out[idx + 2] = (b_sum / a_sum).round().clamp(0.0, 255.0) as u8;
                }
                out[idx + 3] = (a_avg * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    let dot_rgb = match kind {
        TrayStatusKind::Running => [0x22_u8, 0xC5_u8, 0x5E_u8],
        TrayStatusKind::Stopped => [0x64_u8, 0x74_u8, 0x8B_u8],
        TrayStatusKind::Error => [0xEF_u8, 0x44_u8, 0x44_u8],
    };
    composite_status_dot(&mut out, dot_rgb);
    out
}

fn composite_status_dot(rgba: &mut [u8], dot_rgb: [u8; 3]) {
    let cx = 24.5_f32;
    let cy = 24.5_f32;
    let inner_r = 4.6_f32;
    let border_r = 6.2_f32;

    for y in 16..TRAY_DIM {
        for x in 16..TRAY_DIM {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > border_r + 0.7 {
                continue;
            }

            let idx = (y * TRAY_DIM + x) * 4;
            let outer_cov = (border_r + 0.5 - dist).clamp(0.0, 1.0);
            let inner_cov = (inner_r + 0.5 - dist).clamp(0.0, 1.0);
            let border_cov = (outer_cov - inner_cov).max(0.0);

            let src_r = f32::from(dot_rgb[0]) * inner_cov + 255.0 * border_cov;
            let src_g = f32::from(dot_rgb[1]) * inner_cov + 255.0 * border_cov;
            let src_b = f32::from(dot_rgb[2]) * inner_cov + 255.0 * border_cov;
            let src_a = outer_cov;

            let dst_a = f32::from(rgba[idx + 3]) / 255.0;
            let out_a = src_a + dst_a * (1.0 - src_a);
            if out_a > 1e-4 {
                let dst_r = f32::from(rgba[idx]) * dst_a;
                let dst_g = f32::from(rgba[idx + 1]) * dst_a;
                let dst_b = f32::from(rgba[idx + 2]) * dst_a;
                rgba[idx] = ((src_r + dst_r * (1.0 - src_a)) / out_a)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                rgba[idx + 1] = ((src_g + dst_g * (1.0 - src_a)) / out_a)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                rgba[idx + 2] = ((src_b + dst_b * (1.0 - src_a)) / out_a)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                rgba[idx + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn tray_rgba_icon_renders_valid_32x32_pixels_with_status_dot() {
        let running = render_tray_rgba(TrayStatusKind::Running);
        let stopped = render_tray_rgba(TrayStatusKind::Stopped);
        let err = render_tray_rgba(TrayStatusKind::Error);
        assert_eq!(running.len(), 32 * 32 * 4);
        assert_eq!(stopped.len(), 32 * 32 * 4);
        assert_eq!(err.len(), 32 * 32 * 4);

        let center_px = (24 * 32 + 24) * 4;
        assert!(running[center_px + 1] > running[center_px]); // green dot
        assert!(err[center_px] > err[center_px + 1]); // red dot
        assert_ne!(
            &running[center_px..center_px + 4],
            &stopped[center_px..center_px + 4]
        );
        assert!(build_tray_icon(TrayStatusKind::Running).is_some());
    }
}
