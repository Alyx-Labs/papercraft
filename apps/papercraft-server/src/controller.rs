//! Lifecycle and settings controller for the PaperCraft Server tray application.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use papercraft_doc::UiTheme;
use papercraft_server_cli::db::{PasswordResetOutcome, TrashedUserSummary, UserSummary};
use papercraft_server_cli::{LocalDiscoverySummary, ServerRuntime};
use papercraft_updater::{AppId, UpdateStatus, UpdateWorker};

use crate::platform;
use crate::settings::{ConnectionUrls, ServerControlSettings, detect_local_ipv4};
use crate::tray::{TrayAction, TrayStatusKind, TrayVisualState};

const STATS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const COPY_FEEDBACK_DURATION: Duration = Duration::from_millis(2400);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlTab {
    #[default]
    Overview,
    Users,
    Trash,
    Settings,
}

/// Builds the exact manual confirmation phrase (`delete <username>`).
pub fn account_delete_confirmation_phrase(username: &str) -> String {
    format!("delete {}", username.trim())
}

fn normalize_confirmation_words(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Returns `true` when `input` matches `expected` ignoring extra spaces and case.
pub fn account_delete_confirmation_matches(input: &str, expected: &str) -> bool {
    normalize_confirmation_words(input).to_lowercase()
        == normalize_confirmation_words(expected).to_lowercase()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerStatus {
    Running,
    Stopped,
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatsSnapshot {
    pub user_count: usize,
    pub trashed_user_count: usize,
    pub active_notebook_count: usize,
    pub active_session_count: usize,
    pub db_size_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeTone {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeBanner {
    pub tone: NoticeTone,
    pub text: String,
}

pub type DesktopShortcutSyncFn = Box<dyn FnMut(bool) -> Result<String, String>>;
pub type AutostartSyncFn = Box<dyn FnMut(bool, bool) -> Result<String, String>>;

pub struct ServerController {
    pub config_path: PathBuf,
    pub saved_settings: ServerControlSettings,
    pub draft_settings: ServerControlSettings,
    pub active_settings: Option<ServerControlSettings>,
    runtime: Option<ServerRuntime>,
    pub status: ServerStatus,
    pub lan_ip: Option<IpAddr>,
    pub stats_snapshot: Option<StatsSnapshot>,
    pub local_discovery: LocalDiscoverySummary,
    pub users_snapshot: Vec<UserSummary>,
    pub trashed_users_snapshot: Vec<TrashedUserSummary>,
    pub users_error: Option<String>,
    pub active_tab: ControlTab,
    pub user_search: String,
    pub pending_delete_user: Option<String>,
    pub delete_confirm_input: String,
    pub delete_confirm_paste_blocked: bool,
    pub delete_confirm_focus_pending: bool,
    pub pending_reset_user: Option<String>,
    pub last_password_reset: Option<PasswordResetOutcome>,
    last_stats_refresh: Option<Instant>,
    pub copied_url_feedback: Option<(String, Instant)>,
    pub notice_banner: Option<NoticeBanner>,
    pub window_visible: bool,
    pub allow_close: bool,
    pub pending_folder_rx: Option<mpsc::Receiver<Option<PathBuf>>>,
    repaint_ctx: Option<egui::Context>,
    server_stopped_tx: mpsc::Sender<(u64, Result<(), String>)>,
    server_stopped_rx: mpsc::Receiver<(u64, Result<(), String>)>,
    run_generation: u64,
    pub updater: UpdateWorker,
    pub desktop_shortcut_feedback: Option<(bool, String)>,
    desktop_shortcut_sync: Option<DesktopShortcutSyncFn>,
    pub autostart_feedback: Option<(bool, String)>,
    autostart_sync: Option<AutostartSyncFn>,
    #[cfg_attr(all(debug_assertions, not(test)), allow(dead_code))]
    is_first_launch: bool,
}

impl ServerController {
    #[cfg(test)]
    pub fn new(config_path: PathBuf, allow_auto_start: bool) -> Self {
        Self::new_for_launch(config_path, allow_auto_start, false)
    }

    pub fn new_for_launch(
        config_path: PathBuf,
        allow_auto_start: bool,
        autostart_launch: bool,
    ) -> Self {
        let is_first_launch = !config_path.exists();
        let (settings, mut load_warning) = ServerControlSettings::load_or_init(&config_path);
        if allow_auto_start
            && is_first_launch
            && let Err(err) = settings.save_atomic(&config_path)
        {
            load_warning = Some(err);
        }
        let start_visible = settings.initial_window_visible(autostart_launch);
        let auto_start = allow_auto_start && settings.auto_start_server;
        let (server_stopped_tx, server_stopped_rx) = mpsc::channel();

        let mut controller = Self {
            config_path,
            saved_settings: settings.clone(),
            draft_settings: settings,
            active_settings: None,
            runtime: None,
            status: ServerStatus::Stopped,
            lan_ip: detect_local_ipv4(),
            stats_snapshot: None,
            local_discovery: LocalDiscoverySummary::default(),
            users_snapshot: Vec::new(),
            trashed_users_snapshot: Vec::new(),
            users_error: None,
            active_tab: ControlTab::Overview,
            user_search: String::new(),
            pending_delete_user: None,
            delete_confirm_input: String::new(),
            delete_confirm_paste_blocked: false,
            delete_confirm_focus_pending: false,
            pending_reset_user: None,
            last_password_reset: None,
            last_stats_refresh: None,
            copied_url_feedback: None,
            notice_banner: load_warning.map(|text| NoticeBanner {
                tone: NoticeTone::Warning,
                text,
            }),
            window_visible: start_visible,
            allow_close: false,
            pending_folder_rx: None,
            repaint_ctx: None,
            server_stopped_tx,
            server_stopped_rx,
            run_generation: 0,
            updater: UpdateWorker::new(AppId::PaperCraftServer, env!("CARGO_PKG_VERSION")),
            desktop_shortcut_feedback: None,
            desktop_shortcut_sync: None,
            autostart_feedback: None,
            autostart_sync: None,
            is_first_launch,
        };

        if auto_start {
            controller.start_server();
        } else {
            controller.refresh_users();
        }
        controller
    }

    pub fn set_repaint_ctx(&mut self, ctx: egui::Context) {
        self.repaint_ctx = Some(ctx);
    }

    fn updater_notify_cb(&self) -> Arc<dyn Fn() + Send + Sync> {
        let ctx = self.repaint_ctx.clone();
        Arc::new(move || {
            if let Some(ref c) = ctx {
                c.request_repaint();
            }
        })
    }

    pub fn start_startup_update_check_if_enabled(&mut self) {
        if self.draft_settings.auto_check_updates {
            self.updater.check_now(true, self.updater_notify_cb());
        }
    }

    pub fn set_desktop_shortcut_sync(&mut self, hook: DesktopShortcutSyncFn) {
        self.desktop_shortcut_sync = Some(hook);
    }

    pub fn set_desktop_shortcut_enabled(&mut self, enabled: bool) {
        self.draft_settings.auto_create_desktop_shortcut = enabled;
        self.auto_save_settings();
        self.desktop_shortcut_feedback = None;
        if let Some(sync_fn) = self.desktop_shortcut_sync.as_mut()
            && let Err(err) = sync_fn(enabled)
        {
            self.desktop_shortcut_feedback = Some((true, err));
        }
    }

    pub fn sync_desktop_shortcut_on_startup_if_enabled(&mut self) {
        #[cfg(not(debug_assertions))]
        if self.draft_settings.auto_create_desktop_shortcut
            && let Some(sync_fn) = self.desktop_shortcut_sync.as_mut()
            && let Err(err) = sync_fn(true)
        {
            log::warn!("Startup Desktop shortcut sync failed: {err}");
            self.desktop_shortcut_feedback = Some((true, err));
        }
    }

    pub fn set_autostart_sync(&mut self, hook: AutostartSyncFn) {
        self.autostart_sync = Some(hook);
    }

    pub fn set_launch_at_login_enabled(&mut self, enabled: bool) {
        self.draft_settings.launch_at_login = enabled;
        self.auto_save_settings();
        self.autostart_feedback = None;
        if let Some(sync_fn) = self.autostart_sync.as_mut()
            && let Err(err) = sync_fn(enabled, true)
        {
            self.autostart_feedback = Some((true, err));
        }
    }

    pub fn sync_autostart_on_startup_if_enabled(&mut self) {
        #[cfg(not(debug_assertions))]
        self.sync_autostart_on_startup_if_enabled_inner();
    }

    #[cfg(any(not(debug_assertions), test))]
    fn sync_autostart_on_startup_if_enabled_inner(&mut self) {
        if self.draft_settings.launch_at_login
            && let Some(sync_fn) = self.autostart_sync.as_mut()
            && let Err(err) = sync_fn(true, self.is_first_launch)
        {
            log::warn!("Startup autostart sync failed: {err}");
            self.autostart_feedback = Some((true, err));
        }
    }

    pub fn check_for_updates(&mut self) {
        self.updater.check_now(false, self.updater_notify_cb());
    }

    pub fn start_update_download(&mut self) {
        self.updater.start_download(self.updater_notify_cb());
    }

    pub fn cancel_update_download(&mut self) {
        self.updater.cancel_download();
    }

    pub fn open_update_release_page(&mut self) {
        let url = match self.updater.status() {
            UpdateStatus::Available(u) => u.release_url.clone(),
            UpdateStatus::Downloading { update, .. }
            | UpdateStatus::ReadyToInstall { update, .. } => update.release_url.clone(),
            UpdateStatus::NoPlatformAsset { release_url, .. }
            | UpdateStatus::Error { release_url, .. } => release_url.clone(),
            _ => papercraft_updater::DEFAULT_RELEASES_PAGE_URL.to_string(),
        };
        if let Err(err) = platform::open_url_in_browser(&url) {
            self.notice_banner = Some(NoticeBanner {
                tone: NoticeTone::Error,
                text: err,
            });
        }
    }

    pub fn apply_ready_update_and_relaunch(&mut self) -> Result<(), String> {
        self.updater.apply_ready_update_and_relaunch_with_shortcut(
            self.draft_settings.auto_create_desktop_shortcut,
        )
    }

    pub fn is_running(&self) -> bool {
        self.runtime.is_some()
    }

    pub fn theme(&self) -> UiTheme {
        self.draft_settings.theme
    }

    pub fn has_pending_restart(&self) -> bool {
        self.is_running()
            && self
                .active_settings
                .as_ref()
                .is_some_and(|active| !active.same_runtime_config(&self.draft_settings))
    }

    pub fn is_picking_folder(&self) -> bool {
        self.pending_folder_rx.is_some()
    }

    pub fn start_server(&mut self) {
        if self.runtime.is_some() {
            return;
        }
        self.lan_ip = detect_local_ipv4();
        let cfg = match self.draft_settings.to_server_config() {
            Ok(cfg) => cfg,
            Err(err) => {
                self.status = ServerStatus::Error(err);
                return;
            }
        };

        self.auto_save_settings();

        self.run_generation = self.run_generation.saturating_add(1);
        let generation = self.run_generation;
        let tx = self.server_stopped_tx.clone();
        let ctx = self.repaint_ctx.clone();

        match ServerRuntime::start(
            cfg,
            Some(Box::new(move |res| {
                let _ = tx.send((generation, res));
                if let Some(egui_ctx) = ctx {
                    egui_ctx.request_repaint();
                }
            })),
        ) {
            Ok(rt) => {
                self.runtime = Some(rt);
                self.active_settings = Some(self.draft_settings.clone());
                self.status = ServerStatus::Running;
                self.notice_banner = None;
                self.refresh_stats();
                self.refresh_users();
            }
            Err(err) => {
                self.status = ServerStatus::Error(err);
                self.stats_snapshot = None;
                self.last_stats_refresh = None;
            }
        }
    }

    pub fn stop_server(&mut self) {
        self.run_generation = self.run_generation.saturating_add(1);
        if let Some(rt) = self.runtime.take() {
            match rt.stop() {
                Ok(()) => {
                    self.status = ServerStatus::Stopped;
                }
                Err(err) => {
                    self.status = ServerStatus::Error(err);
                }
            }
        } else if !matches!(self.status, ServerStatus::Error(_)) {
            self.status = ServerStatus::Stopped;
        }
        self.active_settings = None;
        self.stats_snapshot = None;
        self.last_stats_refresh = None;
        self.refresh_users();
    }

    pub fn restart_server(&mut self) {
        if let Err(err) = self.draft_settings.to_server_config() {
            self.notice_banner = Some(NoticeBanner {
                tone: NoticeTone::Error,
                text: err,
            });
            return;
        }
        self.stop_server();
        self.start_server();
        if self.is_running() {
            self.notice_banner = Some(NoticeBanner {
                tone: NoticeTone::Success,
                text: "Server restarted with updated settings.".to_string(),
            });
        }
    }

    pub fn toggle_server(&mut self) {
        if self.is_running() {
            self.stop_server();
        } else {
            self.start_server();
        }
    }

    pub fn revert_draft(&mut self) {
        if let Some(ref active) = self.active_settings {
            self.draft_settings.data_dir = active.data_dir.clone();
            self.draft_settings.network_scope = active.network_scope;
            self.draft_settings.custom_ip = active.custom_ip.clone();
            self.draft_settings.port = active.port;
            self.draft_settings.allow_registration = active.allow_registration;
            self.draft_settings.max_notebook_mb = active.max_notebook_mb;
            self.draft_settings.refresh_ttl_days = active.refresh_ttl_days;
            self.draft_settings.access_ttl_hours = active.access_ttl_hours;
            self.notice_banner = None;
            self.auto_save_settings();
        }
    }

    pub fn reset_defaults(&mut self) {
        let current_theme = self.draft_settings.theme;
        let prev_shortcut = self.draft_settings.auto_create_desktop_shortcut;
        let prev_autostart = self.draft_settings.launch_at_login;
        self.draft_settings = ServerControlSettings {
            theme: current_theme,
            ..ServerControlSettings::default()
        };
        if prev_shortcut != self.draft_settings.auto_create_desktop_shortcut {
            self.set_desktop_shortcut_enabled(self.draft_settings.auto_create_desktop_shortcut);
        }
        if prev_autostart != self.draft_settings.launch_at_login {
            self.set_launch_at_login_enabled(self.draft_settings.launch_at_login);
        }
        self.notice_banner = Some(NoticeBanner {
            tone: NoticeTone::Info,
            text: "Restored default settings.".to_string(),
        });
        self.auto_save_settings();
    }

    pub fn auto_save_settings(&mut self) {
        if self.draft_settings == self.saved_settings {
            return;
        }
        match self.draft_settings.save_atomic(&self.config_path) {
            Ok(()) => {
                self.saved_settings = self.draft_settings.clone();
            }
            Err(err) => {
                self.notice_banner = Some(NoticeBanner {
                    tone: NoticeTone::Warning,
                    text: err,
                });
            }
        }
    }

    pub fn refresh_stats(&mut self) {
        let Some(rt) = self.runtime.as_ref() else {
            self.stats_snapshot = None;
            self.last_stats_refresh = None;
            self.local_discovery = papercraft_server_cli::local_discovery_in_dir(
                &self.draft_settings.resolved_data_path(),
                &[],
            );
            return;
        };
        if let Ok(stats) = rt.stats() {
            self.stats_snapshot = Some(StatsSnapshot {
                user_count: stats.user_count,
                trashed_user_count: stats.trashed_user_count,
                active_notebook_count: stats.active_notebook_count,
                active_session_count: stats.active_session_count,
                db_size_bytes: rt.db_file_size_bytes(),
            });
            self.local_discovery = rt.local_discovery_summary();
            self.last_stats_refresh = Some(Instant::now());
        }
    }

    pub fn refresh_users(&mut self) {
        let res = match self.runtime.as_ref() {
            Some(rt) => match (rt.list_users(), rt.list_trashed_users()) {
                (Ok(users), Ok(trashed)) => Ok((users, trashed)),
                (Err(err), _) | (_, Err(err)) => Err(err),
            },
            None => papercraft_server_cli::list_users_and_trash_in_dir(
                &self.draft_settings.resolved_data_path(),
            ),
        };
        match res {
            Ok((users, trashed)) => {
                if let Some(ref target) = self.pending_delete_user
                    && !users.iter().any(|u| u.user_id == *target)
                {
                    self.pending_delete_user = None;
                    self.delete_confirm_input.clear();
                    self.delete_confirm_paste_blocked = false;
                    self.delete_confirm_focus_pending = false;
                }
                if let Some(ref target) = self.pending_reset_user
                    && !users.iter().any(|u| u.user_id == *target)
                {
                    self.pending_reset_user = None;
                }
                self.users_snapshot = users;
                self.trashed_users_snapshot = trashed;
                self.users_error = None;
            }
            Err(err) => {
                self.users_snapshot.clear();
                self.trashed_users_snapshot.clear();
                self.users_error = Some(err);
                self.pending_delete_user = None;
                self.pending_reset_user = None;
                self.delete_confirm_input.clear();
                self.delete_confirm_paste_blocked = false;
                self.delete_confirm_focus_pending = false;
            }
        }
    }

    pub fn request_delete_user(&mut self, user_id: &str) {
        self.pending_delete_user = Some(user_id.to_string());
        self.pending_reset_user = None;
        self.delete_confirm_input.clear();
        self.delete_confirm_paste_blocked = false;
        self.delete_confirm_focus_pending = true;
    }

    pub fn request_reset_password(&mut self, user_id: &str) {
        self.pending_reset_user = Some(user_id.to_string());
        self.pending_delete_user = None;
        self.delete_confirm_input.clear();
        self.delete_confirm_paste_blocked = false;
        self.delete_confirm_focus_pending = false;
    }

    pub fn cancel_user_action(&mut self) {
        self.pending_delete_user = None;
        self.pending_reset_user = None;
        self.delete_confirm_input.clear();
        self.delete_confirm_paste_blocked = false;
        self.delete_confirm_focus_pending = false;
    }

    pub fn dismiss_password_reset(&mut self) {
        self.last_password_reset = None;
    }

    pub fn confirm_delete_user(&mut self, user_id: &str) {
        let Some(target) = self.users_snapshot.iter().find(|u| u.user_id == user_id) else {
            self.notice_banner = Some(NoticeBanner {
                tone: NoticeTone::Error,
                text: "User account could not be found. Refresh and try again.".to_string(),
            });
            self.cancel_user_action();
            return;
        };
        let expected = account_delete_confirmation_phrase(&target.username);
        if !account_delete_confirmation_matches(&self.delete_confirm_input, &expected) {
            self.notice_banner = Some(NoticeBanner {
                tone: NoticeTone::Error,
                text: format!(
                    "Type `{expected}` manually to confirm moving this account to Trash."
                ),
            });
            return;
        }

        let res = match self.runtime.as_ref() {
            Some(rt) => rt.trash_user(user_id),
            None => papercraft_server_cli::trash_user_in_dir(
                &self.draft_settings.resolved_data_path(),
                user_id,
            ),
        };
        self.cancel_user_action();
        if self
            .last_password_reset
            .as_ref()
            .is_some_and(|r| r.user_id == user_id)
        {
            self.last_password_reset = None;
        }
        match res {
            Ok(trashed) => {
                self.notice_banner = Some(NoticeBanner {
                    tone: NoticeTone::Success,
                    text: format!(
                        "Moved `{}` to server Trash ({} synced notebook(s) kept for {} days). Restore anytime from Trash before it expires.",
                        trashed.account_label(),
                        trashed.notebook_count,
                        papercraft_server_cli::db::ACCOUNT_TRASH_RETENTION_DAYS
                    ),
                });
                self.refresh_users();
                if self.is_running() {
                    self.refresh_stats();
                }
            }
            Err(err) => {
                self.notice_banner = Some(NoticeBanner {
                    tone: NoticeTone::Error,
                    text: err,
                });
            }
        }
    }

    pub fn restore_trashed_user(&mut self, user_id: &str) {
        let res = match self.runtime.as_ref() {
            Some(rt) => rt.restore_trashed_user(user_id),
            None => papercraft_server_cli::restore_trashed_user_in_dir(
                &self.draft_settings.resolved_data_path(),
                user_id,
            ),
        };
        self.cancel_user_action();
        match res {
            Ok(restored) => {
                self.notice_banner = Some(NoticeBanner {
                    tone: NoticeTone::Success,
                    text: format!(
                        "Restored `{}` with {} synced notebook(s). They can sign in again now.",
                        restored.account_label(),
                        restored.restored_notebooks
                    ),
                });
                self.refresh_users();
                if self.is_running() {
                    self.refresh_stats();
                }
                if self.trashed_users_snapshot.is_empty() && self.active_tab == ControlTab::Trash {
                    self.active_tab = ControlTab::Users;
                }
            }
            Err(err) => {
                self.notice_banner = Some(NoticeBanner {
                    tone: NoticeTone::Error,
                    text: err,
                });
                self.refresh_users();
            }
        }
    }

    pub fn confirm_reset_password(&mut self, user_id: &str) {
        let res = match self.runtime.as_ref() {
            Some(rt) => rt.reset_user_password_temporary(user_id),
            None => papercraft_server_cli::reset_user_password_temporary_in_dir(
                &self.draft_settings.resolved_data_path(),
                user_id,
            ),
        };
        self.cancel_user_action();
        match res {
            Ok(outcome) => {
                let label = outcome.account_label();
                self.last_password_reset = Some(outcome);
                self.notice_banner = Some(NoticeBanner {
                    tone: NoticeTone::Success,
                    text: format!(
                        "Generated a temporary recovery code for `{label}` and signed out all active devices."
                    ),
                });
                self.refresh_users();
                if self.is_running() {
                    self.refresh_stats();
                }
            }
            Err(err) => {
                self.notice_banner = Some(NoticeBanner {
                    tone: NoticeTone::Error,
                    text: err,
                });
            }
        }
    }

    pub fn set_active_tab(&mut self, tab: ControlTab) {
        if self.active_tab != tab {
            self.active_tab = tab;
            self.cancel_user_action();
            if matches!(tab, ControlTab::Users | ControlTab::Trash) {
                self.refresh_users();
            }
        }
    }

    pub fn begin_folder_picker(&mut self, ctx: &egui::Context) {
        if self.pending_folder_rx.is_some() {
            return;
        }
        let initial = self.draft_settings.resolved_data_path();
        self.pending_folder_rx = Some(platform::pick_folder_async(Some(initial), ctx.clone()));
    }

    pub fn open_data_folder(&mut self) {
        let dir = self.draft_settings.resolved_data_path();
        if let Err(err) = platform::open_folder_in_file_manager(&dir) {
            self.notice_banner = Some(NoticeBanner {
                tone: NoticeTone::Error,
                text: err,
            });
        }
    }

    pub fn copy_url(&mut self, url: &str) {
        if platform::copy_text_to_clipboard(url) {
            self.copied_url_feedback = Some((url.to_string(), Instant::now()));
        } else {
            self.notice_banner = Some(NoticeBanner {
                tone: NoticeTone::Warning,
                text: "Could not copy to clipboard.".to_string(),
            });
        }
    }

    pub fn was_recently_copied(&self, url: &str) -> bool {
        self.copied_url_feedback
            .as_ref()
            .is_some_and(|(copied, _)| copied == url)
    }

    pub fn connection_urls(&self) -> ConnectionUrls {
        let source = self
            .active_settings
            .as_ref()
            .unwrap_or(&self.draft_settings);
        source.connection_urls(self.lan_ip)
    }

    pub fn tray_visual_state(&self) -> TrayVisualState {
        let urls = self.connection_urls();
        let (kind, status_line, tooltip) = match &self.status {
            ServerStatus::Running => (
                TrayStatusKind::Running,
                format!("Status: Running ({})", urls.primary_url),
                format!("PaperCraft Server — Running at {}", urls.primary_url),
            ),
            ServerStatus::Stopped => (
                TrayStatusKind::Stopped,
                "Status: Stopped".to_string(),
                "PaperCraft Server — Stopped".to_string(),
            ),
            ServerStatus::Error(_) => (
                TrayStatusKind::Error,
                "Status: Needs attention".to_string(),
                "PaperCraft Server — Error (click to open)".to_string(),
            ),
        };
        TrayVisualState {
            kind,
            status_line,
            tooltip,
            window_visible: self.window_visible,
        }
    }

    pub fn poll_background_events(&mut self) {
        while let Ok((run_gen, res)) = self.server_stopped_rx.try_recv() {
            if run_gen == self.run_generation && self.runtime.is_some() {
                self.runtime.take();
                self.active_settings = None;
                self.stats_snapshot = None;
                self.last_stats_refresh = None;
                self.status = match res {
                    Ok(()) => ServerStatus::Stopped,
                    Err(err) => ServerStatus::Error(err),
                };
            }
        }

        if let Some(rx) = &self.pending_folder_rx {
            match rx.try_recv() {
                Ok(maybe_path) => {
                    self.pending_folder_rx = None;
                    if let Some(chosen) = maybe_path {
                        self.draft_settings.data_dir = chosen.to_string_lossy().into_owned();
                        self.notice_banner = None;
                        self.auto_save_settings();
                        if !self.is_running() {
                            self.refresh_users();
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending_folder_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        if let Some((_, ts)) = &self.copied_url_feedback
            && ts.elapsed() >= COPY_FEEDBACK_DURATION
        {
            self.copied_url_feedback = None;
        }

        let _ = self.updater.poll();

        if self.window_visible
            && self.is_running()
            && self
                .last_stats_refresh
                .is_none_or(|t| t.elapsed() >= STATS_REFRESH_INTERVAL)
        {
            self.refresh_stats();
            self.refresh_users();
        }
    }

    /// Handles a tray menu or icon activation and reports `(visibility_changed, should_quit)`.
    pub fn handle_tray_action(&mut self, action: TrayAction) -> (Option<bool>, bool) {
        match action {
            TrayAction::ShowWindow => {
                self.window_visible = true;
                self.refresh_stats();
                self.refresh_users();
                (Some(true), false)
            }
            TrayAction::HideWindow => {
                self.window_visible = false;
                (Some(false), false)
            }
            TrayAction::ToggleServer => {
                self.toggle_server();
                (None, false)
            }
            TrayAction::RestartServer => {
                self.restart_server();
                (None, false)
            }
            TrayAction::CopyAddress => {
                let primary = self.connection_urls().primary_url;
                self.copy_url(&primary);
                (None, false)
            }
            TrayAction::OpenDataFolder => {
                self.open_data_folder();
                (None, false)
            }
            TrayAction::Quit => {
                self.allow_close = true;
                self.auto_save_settings();
                self.stop_server();
                (None, true)
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::settings::{CONFIG_FILE_NAME, NetworkScope};

    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(label: &str) -> TempDirGuard {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("bnc_ctrl_test_{label}_{nanos}"));
        let _ = std::fs::remove_dir_all(&p);
        TempDirGuard(p)
    }

    #[test]
    fn controller_starts_detects_pending_restart_reverts_and_stops_cleanly() {
        let dir = temp_dir("lifecycle");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let mut initial = ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            network_scope: NetworkScope::LocalOnly,
            port: 0,
            auto_start_server: false,
            ..ServerControlSettings::default()
        };
        initial.port = 18943;
        initial.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path, false);
        assert!(!ctrl.is_running());
        assert_eq!(ctrl.status, ServerStatus::Stopped);

        // Start server (if port 18943 happens to be busy on test box, try another ephemeral-like port).
        ctrl.start_server();
        if !ctrl.is_running() {
            ctrl.draft_settings.port = 28943;
            ctrl.start_server();
        }
        assert!(
            ctrl.is_running(),
            "expected server to start cleanly: {:?}",
            ctrl.status
        );
        assert_eq!(ctrl.status, ServerStatus::Running);
        assert!(!ctrl.has_pending_restart());
        assert!(ctrl.stats_snapshot.is_some());

        // Editing runtime setting flags pending restart; revert clears it.
        ctrl.draft_settings.allow_registration = false;
        assert!(ctrl.has_pending_restart());
        ctrl.revert_draft();
        assert!(!ctrl.has_pending_restart());
        assert!(ctrl.draft_settings.allow_registration);

        // Toggle UI-only setting should NOT flag pending restart.
        ctrl.draft_settings.theme = UiTheme::Dark;
        assert!(!ctrl.has_pending_restart());

        // Tray show/hide/quit actions update state cleanly.
        let (vis, quit) = ctrl.handle_tray_action(TrayAction::HideWindow);
        assert_eq!(vis, Some(false));
        assert!(!quit);
        assert!(!ctrl.window_visible);

        let (vis2, quit2) = ctrl.handle_tray_action(TrayAction::Quit);
        assert_eq!(vis2, None);
        assert!(quit2);
        assert!(ctrl.allow_close);
        assert!(!ctrl.is_running());
    }

    #[test]
    fn controller_loads_and_refreshes_registered_users_list() {
        use papercraft_server_cli::config::ensure_data_dir_and_meta;
        use papercraft_server_cli::db::{ServerDb, TokenTtls};

        let dir = temp_dir("users_list");
        let vault_dir = dir.0.join("vault");
        let _ = ensure_data_dir_and_meta(&vault_dir).unwrap();
        {
            let db = ServerDb::open(&vault_dir.join("papercraft.redb")).unwrap();
            let ttls = TokenTtls {
                access_ttl_ms: 3_600_000,
                refresh_ttl_ms: 86_400_000,
            };
            db.register_user(
                "alice",
                "strong-password-1",
                Some("Alice Smith"),
                Some("Desktop"),
                true,
                ttls,
            )
            .unwrap();
        }

        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
            data_dir: vault_dir.to_string_lossy().into_owned(),
            port: 18945,
            auto_start_server: false,
            ..ServerControlSettings::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let mut ctrl = ServerController::new(cfg_path, false);
        assert_eq!(ctrl.active_tab, ControlTab::Overview);
        assert_eq!(ctrl.users_snapshot.len(), 1);
        assert_eq!(ctrl.users_snapshot[0].username, "alice");
        assert_eq!(ctrl.users_snapshot[0].display_name, "Alice Smith");
        assert!(ctrl.users_error.is_none());

        ctrl.set_active_tab(ControlTab::Users);
        assert_eq!(ctrl.active_tab, ControlTab::Users);
        assert_eq!(ctrl.users_snapshot.len(), 1);

        ctrl.start_server();
        if !ctrl.is_running() {
            ctrl.draft_settings.port = 28945;
            ctrl.start_server();
        }
        assert!(ctrl.is_running());
        assert_eq!(ctrl.users_snapshot.len(), 1);
        ctrl.stop_server();
        assert_eq!(ctrl.users_snapshot.len(), 1);
    }

    fn setup_two_user_controller(
        label: &str,
        port: u16,
    ) -> (TempDirGuard, ServerController, String, String) {
        use papercraft_server_cli::config::ensure_data_dir_and_meta;
        use papercraft_server_cli::db::{ServerDb, TokenTtls};

        let dir = temp_dir(label);
        let vault_dir = dir.0.join("vault");
        let _ = ensure_data_dir_and_meta(&vault_dir).unwrap();
        let (alice_id, bob_id) = {
            let db = ServerDb::open(&vault_dir.join("papercraft.redb")).unwrap();
            let ttls = TokenTtls {
                access_ttl_ms: 3_600_000,
                refresh_ttl_ms: 86_400_000,
            };
            let a = db
                .register_user(
                    "alice",
                    "strong-password-1",
                    Some("Alice"),
                    Some("Desktop"),
                    true,
                    ttls,
                )
                .unwrap();
            let b = db
                .register_user(
                    "bob",
                    "strong-password-2",
                    Some("Bob"),
                    Some("Laptop"),
                    true,
                    ttls,
                )
                .unwrap();
            (a.user.user_id, b.user.user_id)
        };

        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
            data_dir: vault_dir.to_string_lossy().into_owned(),
            port,
            auto_start_server: false,
            ..ServerControlSettings::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let ctrl = ServerController::new(cfg_path, false);
        assert_eq!(ctrl.users_snapshot.len(), 2);
        (dir, ctrl, alice_id, bob_id)
    }

    fn start_controller_with_fallback_port(ctrl: &mut ServerController, fallback_port: u16) {
        ctrl.start_server();
        if !ctrl.is_running() {
            ctrl.draft_settings.port = fallback_port;
            ctrl.start_server();
        }
        assert!(ctrl.is_running());
    }

    #[test]
    fn controller_resets_password_and_deletes_user_offline_and_online() {
        let (_dir, mut ctrl, alice_id, bob_id) = setup_two_user_controller("users_actions", 18946);

        // Reset Alice while stopped
        ctrl.request_reset_password(&alice_id);
        assert_eq!(ctrl.pending_reset_user.as_deref(), Some(alice_id.as_str()));
        ctrl.confirm_reset_password(&alice_id);
        assert!(ctrl.pending_reset_user.is_none());
        let outcome = ctrl.last_password_reset.as_ref().unwrap();
        assert_eq!(outcome.user_id, alice_id);
        assert_eq!(outcome.temporary_password.len(), 14);
        assert!(
            ctrl.users_snapshot
                .iter()
                .find(|u| u.user_id == alice_id)
                .unwrap()
                .must_change_password
        );

        // Move Bob to 30-day Trash while server is running
        start_controller_with_fallback_port(&mut ctrl, 28946);
        ctrl.request_delete_user(&bob_id);
        ctrl.delete_confirm_input = "delete bob".to_string();
        ctrl.confirm_delete_user(&bob_id);
        assert!(ctrl.pending_delete_user.is_none());
        assert_eq!(ctrl.users_snapshot.len(), 1);
        assert_eq!(ctrl.users_snapshot[0].user_id, alice_id);
        assert_eq!(ctrl.trashed_users_snapshot.len(), 1);
        assert_eq!(ctrl.trashed_users_snapshot[0].user_id, bob_id);
        ctrl.stop_server();
    }

    #[test]
    fn controller_requires_manual_confirmation_for_trash_and_restores_user() {
        let (_dir, mut ctrl, _alice_id, bob_id) =
            setup_two_user_controller("trash_confirm_restore", 18947);
        start_controller_with_fallback_port(&mut ctrl, 28947);

        ctrl.request_delete_user(&bob_id);
        assert_eq!(ctrl.pending_delete_user.as_deref(), Some(bob_id.as_str()));
        assert!(ctrl.delete_confirm_focus_pending);

        // Empty or wrong confirmation is rejected without moving Bob to Trash.
        ctrl.confirm_delete_user(&bob_id);
        assert_eq!(ctrl.users_snapshot.len(), 2);
        assert!(ctrl.trashed_users_snapshot.is_empty());
        assert_eq!(
            ctrl.notice_banner.as_ref().map(|b| b.tone),
            Some(NoticeTone::Error)
        );

        ctrl.delete_confirm_input = "delete alice".to_string();
        ctrl.confirm_delete_user(&bob_id);
        assert_eq!(ctrl.users_snapshot.len(), 2);
        assert!(ctrl.trashed_users_snapshot.is_empty());

        // Background refresh while typing must preserve the in-progress confirmation draft.
        ctrl.delete_confirm_input = "delete bo".to_string();
        ctrl.refresh_users();
        assert_eq!(ctrl.pending_delete_user.as_deref(), Some(bob_id.as_str()));
        assert_eq!(ctrl.delete_confirm_input, "delete bo");

        // Exact manual phrase moves Bob to server Trash instead of erasing immediately.
        ctrl.delete_confirm_input = "delete bob".to_string();
        ctrl.confirm_delete_user(&bob_id);
        assert!(ctrl.pending_delete_user.is_none());
        assert!(ctrl.delete_confirm_input.is_empty());
        assert_eq!(ctrl.users_snapshot.len(), 1);
        assert_eq!(ctrl.trashed_users_snapshot.len(), 1);
        assert_eq!(ctrl.trashed_users_snapshot[0].user_id, bob_id);

        // Restore Bob from Trash back into active users.
        ctrl.set_active_tab(ControlTab::Trash);
        ctrl.restore_trashed_user(&bob_id);
        assert_eq!(ctrl.users_snapshot.len(), 2);
        assert!(ctrl.trashed_users_snapshot.is_empty());
        assert_eq!(ctrl.active_tab, ControlTab::Users);
        assert_eq!(
            ctrl.notice_banner.as_ref().map(|b| b.tone),
            Some(NoticeTone::Success)
        );
        ctrl.stop_server();
    }

    #[test]
    fn controller_toggles_desktop_shortcut_persists_and_surfaces_errors_only() {
        use std::sync::{Arc, Mutex};

        let dir = temp_dir("desktop_shortcut_ctrl");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let settings = ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            auto_start_server: false,
            ..ServerControlSettings::default()
        };
        settings.save_atomic(&cfg_path).unwrap();

        let calls = Arc::new(Mutex::new(Vec::<bool>::new()));
        let calls_hook = Arc::clone(&calls);

        let mut ctrl = ServerController::new(cfg_path.clone(), false);
        assert!(ctrl.draft_settings.auto_create_desktop_shortcut);
        ctrl.set_desktop_shortcut_sync(Box::new(move |enabled| {
            calls_hook
                .lock()
                .map_err(|err| err.to_string())?
                .push(enabled);
            Ok(if enabled {
                "Server shortcut created (mock).".to_string()
            } else {
                "Server shortcut removed (mock).".to_string()
            })
        }));

        ctrl.set_desktop_shortcut_enabled(false);
        assert!(!ctrl.draft_settings.auto_create_desktop_shortcut);
        assert!(ctrl.desktop_shortcut_feedback.is_none());

        let (reloaded, warning) = ServerControlSettings::load_or_init(&cfg_path);
        assert!(warning.is_none());
        assert!(!reloaded.auto_create_desktop_shortcut);

        ctrl.set_desktop_shortcut_enabled(true);
        assert!(ctrl.draft_settings.auto_create_desktop_shortcut);
        assert!(ctrl.desktop_shortcut_feedback.is_none());
        assert_eq!(*calls.lock().unwrap(), vec![false, true]);

        ctrl.set_desktop_shortcut_sync(Box::new(|_| Err("Permission denied".to_string())));
        ctrl.set_desktop_shortcut_enabled(true);
        assert_eq!(
            ctrl.desktop_shortcut_feedback.as_ref(),
            Some(&(true, "Permission denied".to_string()))
        );
    }

    #[test]
    fn controller_autostart_defaults_enabled_syncs_on_startup_and_toggles_cleanly() {
        use std::sync::{Arc, Mutex};

        let dir = temp_dir("autostart_ctrl");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);

        let calls = Arc::new(Mutex::new(Vec::<(bool, bool)>::new()));
        let calls_hook = Arc::clone(&calls);

        // Fresh launch without config file clears any stale OS override and persists defaults.
        let mut ctrl = ServerController::new(cfg_path.clone(), false);
        assert!(ctrl.draft_settings.launch_at_login);
        ctrl.set_autostart_sync(Box::new(move |enabled, clear_override| {
            calls_hook
                .lock()
                .map_err(|err| err.to_string())?
                .push((enabled, clear_override));
            Ok(if enabled {
                "Startup launch enabled (mock).".to_string()
            } else {
                "Startup launch disabled (mock).".to_string()
            })
        }));

        ctrl.sync_autostart_on_startup_if_enabled_inner();
        assert_eq!(*calls.lock().unwrap(), vec![(true, true)]);
        assert!(ctrl.autostart_feedback.is_none());

        ctrl.set_launch_at_login_enabled(false);
        assert!(!ctrl.draft_settings.launch_at_login);
        assert!(ctrl.autostart_feedback.is_none());

        let (reloaded, warning) = ServerControlSettings::load_or_init(&cfg_path);
        assert!(warning.is_none());
        assert!(!reloaded.launch_at_login);

        // Subsequent launch respects disabled preference.
        let mut ctrl2 = ServerController::new(cfg_path.clone(), false);
        let calls2 = Arc::clone(&calls);
        ctrl2.set_autostart_sync(Box::new(move |enabled, clear_override| {
            calls2
                .lock()
                .map_err(|err| err.to_string())?
                .push((enabled, clear_override));
            Ok(String::new())
        }));
        ctrl2.sync_autostart_on_startup_if_enabled_inner();
        assert_eq!(*calls.lock().unwrap(), vec![(true, true), (false, true)]);

        ctrl.set_launch_at_login_enabled(true);
        assert!(ctrl.draft_settings.launch_at_login);
        assert!(ctrl.autostart_feedback.is_none());
        assert_eq!(
            *calls.lock().unwrap(),
            vec![(true, true), (false, true), (true, true)]
        );

        // Subsequent launch when enabled preserves any OS Task Manager disable override.
        let mut ctrl3 = ServerController::new(cfg_path, false);
        let calls3 = Arc::clone(&calls);
        ctrl3.set_autostart_sync(Box::new(move |enabled, clear_override| {
            calls3
                .lock()
                .map_err(|err| err.to_string())?
                .push((enabled, clear_override));
            Ok(String::new())
        }));
        ctrl3.sync_autostart_on_startup_if_enabled_inner();
        assert_eq!(
            *calls.lock().unwrap(),
            vec![(true, true), (false, true), (true, true), (true, false)]
        );

        // Startup/toggle failures surface cleanly in error state.
        ctrl3.set_autostart_sync(Box::new(|_, _| Err("Registry write denied".to_string())));
        ctrl3.set_launch_at_login_enabled(true);
        assert_eq!(
            ctrl3.autostart_feedback.as_ref(),
            Some(&(true, "Registry write denied".to_string()))
        );
    }

    #[test]
    fn controller_launch_visibility_respects_autostart_and_open_window_setting() {
        let dir = temp_dir("autostart_visibility");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);

        // Default config: autostart launches stay hidden in tray; manual launches open visibly.
        let hidden_autostart = ServerController::new_for_launch(cfg_path.clone(), false, true);
        assert!(!hidden_autostart.window_visible);
        let manual_visible = ServerController::new_for_launch(cfg_path.clone(), false, false);
        assert!(manual_visible.window_visible);

        // Enabling `open_window_at_login` opens the window even on `--autostart`.
        let mut settings = ServerControlSettings {
            data_dir: dir.0.join("vault").to_string_lossy().into_owned(),
            auto_start_server: false,
            open_window_at_login: true,
            ..ServerControlSettings::default()
        };
        settings.save_atomic(&cfg_path).unwrap();
        let visible_autostart = ServerController::new_for_launch(cfg_path.clone(), false, true);
        assert!(visible_autostart.window_visible);

        // `start_in_tray` affects manual launches without blocking explicit `open_window_at_login`.
        settings.start_in_tray = true;
        settings.save_atomic(&cfg_path).unwrap();
        let manual_tray = ServerController::new_for_launch(cfg_path.clone(), false, false);
        assert!(!manual_tray.window_visible);
        let autostart_override = ServerController::new_for_launch(cfg_path, false, true);
        assert!(autostart_override.window_visible);
    }
}
