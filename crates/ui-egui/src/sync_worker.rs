use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use papercraft_sync::{ActiveAccountSession, SyncBatch, SyncCycleOutcome};
#[cfg(not(target_arch = "wasm32"))]
use papercraft_sync::{SyncCycleDriver, SyncError, SyncHttpClient, run_sync_cycle};
use papercraft_sync_protocol::{
    ChangeEmailRequest, ChangePasswordRequest, CompletePasswordResetRequest, LocalDiscoveryReport,
    LoginRequest, RegisterRequest, ServerInfoResponse, SyncPullResponse, UpdateProfileRequest,
    normalize_email, normalize_username, now_unix_ms, sanitize_display_name, validate_password,
};
#[cfg(not(target_arch = "wasm32"))]
use papercraft_sync_protocol::{
    ERR_CODE_PASSWORD_CHANGE_REQUIRED, ERR_CODE_PASSWORD_RESET_REQUIRED,
};

use crate::PaperCraftApp;

const AUTO_SYNC_DEBOUNCE_NORMAL_MS: f64 = 1_500.0;
#[cfg(not(target_arch = "wasm32"))]
const AUTO_SYNC_DEBOUNCE_MEDIUM_MS: f64 = 3_500.0;
#[cfg(target_arch = "wasm32")]
const AUTO_SYNC_DEBOUNCE_MEDIUM_MS: f64 = 6_000.0;
#[cfg(not(target_arch = "wasm32"))]
const AUTO_SYNC_DEBOUNCE_HEAVY_MS: f64 = 6_000.0;
#[cfg(target_arch = "wasm32")]
const AUTO_SYNC_DEBOUNCE_HEAVY_MS: f64 = 12_000.0;
const SYNC_MEDIUM_PAYLOAD_THRESHOLD_BYTES: usize = 4 * 1024 * 1024;
const SYNC_HEAVY_PAYLOAD_THRESHOLD_BYTES: usize = 32 * 1024 * 1024;
const AUTO_SYNC_IDLE_PULL_MS: f64 = 12_000.0;
const DISCOVERY_REPORT_INTERVAL_MS: f64 = 45_000.0;

pub(crate) fn adaptive_sync_debounce_ms(estimated_dirty_bytes: usize) -> f64 {
    if estimated_dirty_bytes >= SYNC_HEAVY_PAYLOAD_THRESHOLD_BYTES {
        AUTO_SYNC_DEBOUNCE_HEAVY_MS
    } else if estimated_dirty_bytes >= SYNC_MEDIUM_PAYLOAD_THRESHOLD_BYTES {
        AUTO_SYNC_DEBOUNCE_MEDIUM_MS
    } else {
        AUTO_SYNC_DEBOUNCE_NORMAL_MS
    }
}

pub enum SyncWorkerCommand {
    TestConnection {
        server_url: String,
    },
    ReportLocalDiscovery {
        server_url: String,
        report: LocalDiscoveryReport,
    },
    Register {
        session_generation: u64,
        server_url: String,
        email: String,
        username: Option<String>,
        password: String,
        display_name: Option<String>,
        device_name: String,
    },
    Login {
        session_generation: u64,
        server_url: String,
        identifier: String,
        password: String,
        device_name: String,
    },
    CompletePasswordReset {
        session_generation: u64,
        server_url: String,
        identifier: String,
        recovery_code: String,
        new_password: String,
        device_name: String,
    },
    ConvertLocalAccount {
        session_generation: u64,
        server_url: String,
        source_profile_id: String,
        source_nickname: String,
        email: String,
        password: String,
        device_name: String,
        upload_batch: Box<SyncBatch>,
    },
    UpdateProfile {
        session_generation: u64,
        session: ActiveAccountSession,
        username: Option<String>,
        display_name: Option<String>,
    },
    ChangePassword {
        session_generation: u64,
        session: ActiveAccountSession,
        current_password: String,
        new_password: String,
    },
    ChangeEmail {
        session_generation: u64,
        session: ActiveAccountSession,
        new_email: String,
        current_password: String,
    },
    SyncNow {
        session_generation: u64,
        session: ActiveAccountSession,
        batch: Box<SyncBatch>,
    },
    Logout {
        session: ActiveAccountSession,
    },
}

pub enum SyncWorkerEvent {
    HealthOk {
        server_url: String,
        info: ServerInfoResponse,
    },
    HealthErr {
        server_url: String,
        message: String,
    },
    AuthOk {
        session_generation: u64,
        auth: ActiveAccountSession,
        initial_pull: Box<SyncPullResponse>,
    },
    AuthErr {
        session_generation: u64,
        offline: bool,
        message: String,
    },
    ConvertLocalProgress {
        session_generation: u64,
        source_profile_id: String,
        stage: String,
        completed: usize,
        total: usize,
    },
    ConvertLocalOk {
        session_generation: u64,
        source_profile_id: String,
        auth: ActiveAccountSession,
        initial_pull: Box<SyncPullResponse>,
    },
    ConvertLocalErr {
        session_generation: u64,
        source_profile_id: String,
        offline: bool,
        message: String,
    },
    PasswordResetRequired {
        session_generation: u64,
        identifier: String,
        prefill_code: Option<String>,
        message: String,
    },
    ProfileUpdated {
        session_generation: u64,
        updated_session: ActiveAccountSession,
    },
    ProfileUpdateErr {
        session_generation: u64,
        offline: bool,
        unauthorized: bool,
        message: String,
    },
    PasswordChanged {
        session_generation: u64,
        updated_session: ActiveAccountSession,
    },
    PasswordChangeErr {
        session_generation: u64,
        offline: bool,
        unauthorized: bool,
        message: String,
    },
    EmailChanged {
        session_generation: u64,
        updated_session: ActiveAccountSession,
    },
    EmailChangeErr {
        session_generation: u64,
        offline: bool,
        unauthorized: bool,
        message: String,
    },
    SyncOk {
        session_generation: u64,
        outcome: Box<SyncCycleOutcome>,
    },
    SyncErr {
        session_generation: u64,
        offline: bool,
        unauthorized: bool,
        message: String,
    },
}

impl SyncWorkerEvent {
    fn session_generation(&self) -> Option<u64> {
        match self {
            Self::HealthOk { .. } | Self::HealthErr { .. } => None,
            Self::AuthOk {
                session_generation, ..
            }
            | Self::AuthErr {
                session_generation, ..
            }
            | Self::ConvertLocalProgress {
                session_generation, ..
            }
            | Self::ConvertLocalOk {
                session_generation, ..
            }
            | Self::ConvertLocalErr {
                session_generation, ..
            }
            | Self::PasswordResetRequired {
                session_generation, ..
            }
            | Self::ProfileUpdated {
                session_generation, ..
            }
            | Self::ProfileUpdateErr {
                session_generation, ..
            }
            | Self::PasswordChanged {
                session_generation, ..
            }
            | Self::PasswordChangeErr {
                session_generation, ..
            }
            | Self::EmailChanged {
                session_generation, ..
            }
            | Self::EmailChangeErr {
                session_generation, ..
            }
            | Self::SyncOk {
                session_generation, ..
            }
            | Self::SyncErr {
                session_generation, ..
            } => Some(*session_generation),
        }
    }
}

/// Builds a normalized registration payload from an email address, optional username, and password.
pub fn build_register_request(
    email_raw: &str,
    username_raw: Option<&str>,
    password: String,
    display_name: Option<String>,
    device_name: &str,
) -> Result<RegisterRequest, String> {
    let (email, _) = normalize_email(email_raw).map_err(|e| e.to_string())?;
    let username = match username_raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => normalize_username(raw).map_err(|e| e.to_string())?.0,
        None => String::new(),
    };
    Ok(RegisterRequest {
        email: Some(email),
        username,
        password,
        display_name,
        device_name: Some(device_name.to_string()),
    })
}

/// Builds an email-only login payload from an email address and password.
pub fn build_login_request(
    email_raw: &str,
    password: String,
    device_name: &str,
) -> Result<LoginRequest, String> {
    let (_, email) = normalize_email(email_raw)
        .map_err(|_| "Enter a valid email address to sign in.".to_string())?;
    Ok(LoginRequest {
        email: Some(email),
        username: String::new(),
        password,
        device_name: Some(device_name.to_string()),
    })
}

/// Builds a validated `CompletePasswordResetRequest` payload (email-only).
pub fn build_complete_password_reset_request(
    identifier: &str,
    recovery_code: &str,
    new_password: String,
    device_name: &str,
) -> Result<CompletePasswordResetRequest, String> {
    let code = recovery_code.trim();
    if code.is_empty() {
        return Err("Enter the temporary recovery code generated on the server.".to_string());
    }
    validate_password(&new_password).map_err(|e| e.to_string())?;
    if new_password == code {
        return Err("New password must be different from the temporary recovery code.".to_string());
    }
    let (_, email) = normalize_email(identifier)
        .map_err(|_| "Enter a valid email address to recover your account.".to_string())?;
    Ok(CompletePasswordResetRequest {
        email: Some(email),
        username: String::new(),
        recovery_code: code.to_string(),
        new_password,
        device_name: Some(device_name.to_string()),
    })
}

/// Builds a validated `UpdateProfileRequest` payload.
pub fn build_update_profile_request(
    username_raw: Option<&str>,
    display_name_raw: Option<&str>,
) -> Result<UpdateProfileRequest, String> {
    let username = match username_raw {
        Some(raw) => Some(normalize_username(raw).map_err(|e| e.to_string())?.0),
        None => None,
    };
    let display_name = display_name_raw.map(|raw| sanitize_display_name(raw, ""));
    if username.is_none() && display_name.is_none() {
        return Err("No profile changes to save.".to_string());
    }
    Ok(UpdateProfileRequest {
        username,
        display_name,
    })
}

/// Builds a validated `ChangePasswordRequest` payload.
pub fn build_change_password_request(
    current_password: String,
    new_password: String,
) -> Result<ChangePasswordRequest, String> {
    if current_password.is_empty() {
        return Err("Enter your current password to confirm.".to_string());
    }
    validate_password(&new_password).map_err(|e| e.to_string())?;
    if new_password == current_password {
        return Err("New password must be different from your current password.".to_string());
    }
    Ok(ChangePasswordRequest {
        current_password,
        new_password,
    })
}

/// Builds a validated `ChangeEmailRequest` payload.
pub fn build_change_email_request(
    new_email_raw: &str,
    current_password: String,
) -> Result<ChangeEmailRequest, String> {
    let (new_email, _) = normalize_email(new_email_raw).map_err(|e| e.to_string())?;
    Ok(ChangeEmailRequest {
        current_password,
        new_email,
    })
}

pub type WebSyncDispatchFn =
    Box<dyn FnMut(SyncWorkerCommand, mpsc::Sender<SyncWorkerEvent>, Option<egui::Context>)>;

pub(crate) struct BackgroundSyncWorker {
    cmd_tx: Option<mpsc::Sender<SyncWorkerCommand>>,
    pub(crate) evt_tx: mpsc::Sender<SyncWorkerEvent>,
    evt_rx: mpsc::Receiver<SyncWorkerEvent>,
    repaint_ctx: Arc<Mutex<Option<egui::Context>>>,
    busy: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    session_generation: u64,
    pending_outcome: Option<SyncCycleOutcome>,
    startup_sync_queued: bool,
    last_attempt_ui_ms: f64,
    next_retry_delay_ms: f64,
    last_discovery_sent: Option<(f64, LocalDiscoveryReport)>,
    last_discovery_check_ui_ms: Option<f64>,
    last_discovery_state_key: Option<(u64, bool, usize)>,
}

impl BackgroundSyncWorker {
    pub(crate) fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<SyncWorkerCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<SyncWorkerEvent>();
        let repaint_ctx: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
        let busy = Arc::new(AtomicBool::new(false));

        let ctx_slot = Arc::clone(&repaint_ctx);
        let busy_flag = Arc::clone(&busy);
        let thread_evt_tx = evt_tx.clone();
        #[cfg(not(target_arch = "wasm32"))]
        let handle = std::thread::Builder::new()
            .name("papercraft-sync".to_string())
            .spawn(move || worker_loop(cmd_rx, thread_evt_tx, ctx_slot, busy_flag))
            .ok();
        #[cfg(target_arch = "wasm32")]
        let handle = {
            let _ = (cmd_rx, thread_evt_tx, ctx_slot, busy_flag);
            None
        };

        Self {
            cmd_tx: Some(cmd_tx),
            evt_tx,
            evt_rx,
            repaint_ctx,
            busy,
            handle,
            session_generation: 0,
            pending_outcome: None,
            startup_sync_queued: false,
            last_attempt_ui_ms: -100_000.0,
            next_retry_delay_ms: 8_000.0,
            last_discovery_sent: None,
            last_discovery_check_ui_ms: None,
            last_discovery_state_key: None,
        }
    }

    pub(crate) fn send(&self, ctx: Option<&egui::Context>, cmd: SyncWorkerCommand) -> bool {
        if let Some(ctx) = ctx
            && let Ok(mut guard) = self.repaint_ctx.lock()
        {
            *guard = Some(ctx.clone());
        }
        self.cmd_tx.as_ref().is_some_and(|tx| tx.send(cmd).is_ok())
    }

    pub(crate) fn current_session_generation(&self) -> u64 {
        self.session_generation
    }

    pub(crate) fn bump_session_generation(&mut self) -> u64 {
        self.session_generation = self.session_generation.wrapping_add(1);
        self.session_generation
    }

    pub(crate) fn shutdown(&mut self) {
        self.cmd_tx = None;
        if !self.busy.load(Ordering::Relaxed)
            && let Some(handle) = self.handle.take()
        {
            let _ = handle.join();
        }
    }
}

pub(crate) fn dispatch_command(
    app: &mut PaperCraftApp,
    ctx: Option<&egui::Context>,
    cmd: SyncWorkerCommand,
) -> bool {
    if let Some(web_dispatch) = app.hooks.web_sync_dispatch.as_mut() {
        web_dispatch(cmd, app.sync_worker.evt_tx.clone(), ctx.cloned());
        return true;
    }
    app.sync_worker.send(ctx, cmd)
}

pub(crate) fn maybe_report_local_discovery(
    app: &mut PaperCraftApp,
    ctx: &egui::Context,
    now_ui_ms: f64,
) {
    if !now_ui_ms.is_finite()
        || !app.session.is_sync_persistence_enabled()
        || app.session.needs_initial_nickname()
    {
        return;
    }
    let state_key = (
        app.session.revision,
        app.session.is_viewing_cloud(),
        app.session.accessible_local_profiles().len(),
    );
    let periodic_due = app
        .sync_worker
        .last_discovery_sent
        .as_ref()
        .is_none_or(|(prev_ms, _)| now_ui_ms - *prev_ms >= DISCOVERY_REPORT_INTERVAL_MS);
    let change_due = app.sync_worker.last_discovery_state_key != Some(state_key)
        && app
            .sync_worker
            .last_discovery_check_ui_ms
            .is_none_or(|prev_check| now_ui_ms - prev_check >= 2_000.0);
    if !periodic_due && !change_due {
        return;
    }

    app.sync_worker.last_discovery_check_ui_ms = Some(now_ui_ms);
    app.sync_worker.last_discovery_state_key = Some(state_key);

    let report = app.session.local_discovery_report();
    if report.client_id.trim().is_empty() {
        return;
    }
    let should_send = match &app.sync_worker.last_discovery_sent {
        None => true,
        Some((prev_ms, prev_rep)) => {
            prev_rep != &report || (now_ui_ms - *prev_ms) >= DISCOVERY_REPORT_INTERVAL_MS
        }
    };
    if !should_send {
        return;
    }
    let server_url = app.session.server_url().to_string();
    app.sync_worker.last_discovery_sent = Some((now_ui_ms, report.clone()));
    let _ = dispatch_command(
        app,
        Some(ctx),
        SyncWorkerCommand::ReportLocalDiscovery { server_url, report },
    );
}

/// Flushes all pending edits for the outgoing workspace and bumps `session_generation` so stale
/// background sync/auth responses from the previous account cannot mutate the incoming workspace.
pub(crate) fn prepare_workspace_switch(app: &mut PaperCraftApp) {
    crate::canvas::commit_pending_text(app);
    app.session.cancel_live_stroke();
    app.session.cancel_live_eraser();
    app.view.text_insert_mode = false;
    app.view.text_editing = false;
    app.view.text_edit_index = None;
    app.view.shape_insert_mode = false;
    app.view.shape_drag_start = None;
    app.view.shape_drag_end = None;
    app.view.shape_drag_touch_id = None;
    app.view.ruler_active = false;
    app.view.ruler_drag_mode = None;
    app.view.ruler_drag_touch_id = None;
    app.view.ruler_drag_anchor = None;
    app.view.clear_selection();
    app.view.color_picker.close();
    app.flush_pending_autosaves();
    app.flush_web_storage(true);
    app.sync_worker.bump_session_generation();
    app.sync_worker.pending_outcome = None;
    app.sync_worker.startup_sync_queued = false;
}

/// Resets view caches for the newly loaded workspace and enqueues an urgent persistence barrier.
pub(crate) fn finish_workspace_switch(app: &mut PaperCraftApp, ctx: &egui::Context) {
    app.view.open_tabs.clear();
    app.view.inspector_open = false;
    app.view.library_search.clear();
    app.view.library_show_trash = false;
    app.view.library_folder_filter = None;
    app.view.library_favorites_only = false;
    app.view.library_tag_filter = None;
    app.mesh_cache.clear();
    app.image_textures.clear();
    crate::theme::apply_visuals(ctx, app.session.prefs.theme);
    crate::sync_view_to_active_notebook(app);
    app.flush_web_storage(true);
}

pub(crate) fn trigger_manual_sync(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let Some(acct) = app.session.active_account_session().cloned() else {
        return;
    };
    if app.session.is_sync_in_flight() {
        return;
    }
    let Some(batch) = app.session.prepare_sync_batch() else {
        return;
    };
    let session_gen = app.sync_worker.current_session_generation();
    app.session.set_sync_in_flight(true);
    app.sync_worker.last_attempt_ui_ms = ctx.input(|i| i.time * 1000.0);
    if !dispatch_command(
        app,
        Some(ctx),
        SyncWorkerCommand::SyncNow {
            session_generation: session_gen,
            session: acct,
            batch: Box::new(batch),
        },
    ) {
        app.view.account.clear_transient_auth_status();
        app.session
            .record_sync_failure(false, "Background sync worker unavailable.".to_string());
    }
}

/// Creates a new isolated local account and switches into it after flushing any active workspace.
pub(crate) fn create_local_account_and_switch(
    app: &mut PaperCraftApp,
    ctx: &egui::Context,
    nickname_raw: &str,
) -> Result<(), String> {
    let nick = nickname_raw.trim().to_string();
    if nick.is_empty() {
        return Err("Please enter a nickname for the local account.".to_string());
    }
    let server_url = app.session.preferred_server_url().to_string();
    prepare_workspace_switch(app);
    let res = if app.session.needs_initial_nickname() {
        app.session.execute(
            "profile.completeFirstAccess",
            &serde_json::json!({ "nickname": nick }),
        )
    } else {
        app.session.execute(
            "profile.create",
            &serde_json::json!({ "nickname": nick, "switch_now": true }),
        )
    };
    if let Err(err) = res {
        return Err(err.to_string());
    }
    app.view.account.select_local_new_name.clear();
    app.view.account.select_local_error = None;
    app.view.account.new_profile_name.clear();
    app.view.account.disconnect_notice = None;
    app.view.account.reset_after_sign_out(&server_url);
    app.view.account.status_message = None;
    app.view.screen = crate::AppScreen::Home;
    app.view.profile_return_screen = crate::AppScreen::Home;
    finish_workspace_switch(app, ctx);
    maybe_report_local_discovery(app, ctx, ctx.input(|i| i.time * 1000.0));
    Ok(())
}

/// Switches into a saved local account on this device without password or network access,
/// persisting any active Offline-First cloud workspace cache while keeping cloud sessions cached.
pub(crate) fn switch_to_local_account_clean(
    app: &mut PaperCraftApp,
    ctx: &egui::Context,
    profile_id: &str,
) -> Result<(), String> {
    let server_url = app.session.preferred_server_url().to_string();
    prepare_workspace_switch(app);
    app.session
        .execute(
            "profile.switch",
            &serde_json::json!({ "target": "local", "profile_id": profile_id }),
        )
        .map_err(|err| err.to_string())?;
    app.view.account.disconnect_notice = None;
    app.view.account.reset_after_sign_out(&server_url);
    app.view.account.status_message = Some("Switched to local account.".to_string());
    app.view.profile_return_screen = crate::AppScreen::Home;
    finish_workspace_switch(app, ctx);
    Ok(())
}

/// Forgets a remembered server account from this device, deleting only its local cache
/// while keeping server data intact, and returning gracefully to local state if it was active.
pub(crate) fn complete_forget_server_account(
    app: &mut PaperCraftApp,
    ctx: &egui::Context,
    server_url: &str,
    user_id: &str,
    label: &str,
) -> Result<(), String> {
    let cached_sess = app.session.cached_server_session_for(server_url, user_id);
    let was_active = app.session.active_account_session().is_some_and(|s| {
        s.user.user_id == user_id
            && (s.server_url.eq_ignore_ascii_case(server_url)
                || papercraft_sync_protocol::normalize_server_url(server_url)
                    .is_ok_and(|u| u.eq_ignore_ascii_case(&s.server_url)))
    });
    if was_active {
        prepare_workspace_switch(app);
    }
    app.session
        .execute(
            "profile.forgetServerAccount",
            &serde_json::json!({ "serverUrl": server_url, "userId": user_id }),
        )
        .map_err(|err| err.to_string())?;
    if let Some(sess) = cached_sess {
        let _ = dispatch_command(app, Some(ctx), SyncWorkerCommand::Logout { session: sess });
    }
    app.view.account.clear_pending_forget_server();
    if was_active {
        app.view.account.disconnect_notice = None;
        app.view.account.reset_after_sign_out(server_url);
        app.view.screen = crate::AppScreen::Home;
        app.view.profile_return_screen = crate::AppScreen::Home;
        finish_workspace_switch(app, ctx);
        let next_modal = if app.session.has_any_saved_accounts() {
            crate::modal::ActiveModal::SelectLocalAccount
        } else {
            crate::modal::ActiveModal::FirstAccessNickname
        };
        crate::modal::open(app, next_modal);
    } else {
        app.flush_web_storage(true);
        if app.session.needs_initial_nickname() && !app.session.has_any_saved_accounts() {
            crate::modal::open(app, crate::modal::ActiveModal::FirstAccessNickname);
        }
    }
    app.view.account.error_message = None;
    app.view.account.status_message = Some(format!(
        "Forgot {label} and cleared its local cache on this device. Your server data remains intact."
    ));
    Ok(())
}

pub(crate) fn poll_and_tick(app: &mut PaperCraftApp, ctx: &egui::Context, now_ui_ms: f64) {
    drain_worker_events(app, ctx);
    let can_mutate_docs = app.session.live_stroke.is_none() && app.session.live_eraser.is_none();
    if can_mutate_docs {
        maybe_report_local_discovery(app, ctx, now_ui_ms);
    }
    if can_mutate_docs && let Some(outcome) = app.sync_worker.pending_outcome.take() {
        let prev_theme = app.session.prefs.theme;
        let prev_active_title = app.session.doc.title.clone();
        let prev_active_cover = app.session.doc.cover;
        let prev_rev = app.session.revision;
        if let Err(err) = app.session.apply_sync_outcome(outcome) {
            app.session.record_sync_failure(false, err);
            app.view.account.clear_transient_auth_status();
        } else {
            app.sync_worker.next_retry_delay_ms = 8_000.0;
            app.view.account.clear_transient_auth_status();
            let active_url = app.session.server_url().to_string();
            app.view.account.clear_offline_after_sync(&active_url);
            if let Some(msg) = app.session.take_last_conflict_notice() {
                app.view.account.status_message = Some(msg);
            }
            if app.session.revision != prev_rev {
                app.mesh_cache.clear();
            }
            if app.session.prefs.theme != prev_theme {
                crate::theme::apply_visuals(ctx, app.session.prefs.theme);
            }
            if app.session.doc.title != prev_active_title
                || app.session.doc.cover != prev_active_cover
            {
                app.view.notebook_title_draft = app.session.doc.title.clone();
                app.view.notebook_cover_draft = app.session.doc.cover;
            }
        }
    }

    if !app.session.is_signed_in()
        || !app.session.is_viewing_cloud()
        || app.session.is_session_expired()
        || app.session.is_sync_in_flight()
        || app.sync_worker.pending_outcome.is_some()
        || !can_mutate_docs
        || !app.session.is_sync_persistence_enabled()
    {
        return;
    }

    let debounce_ms = adaptive_sync_debounce_ms(app.session.dirty_sync_estimated_payload_bytes());
    let elapsed_since_last = now_ui_ms - app.sync_worker.last_attempt_ui_ms;
    let is_disconnected = matches!(
        app.session.sync_status_summary().connection,
        papercraft_sync::SyncConnectionState::Offline | papercraft_sync::SyncConnectionState::Error
    );
    let dirty_gate_ms = if is_disconnected {
        app.sync_worker.next_retry_delay_ms
    } else {
        debounce_ms
    };
    let should_startup_sync = !app.sync_worker.startup_sync_queued;
    let has_dirty_ready = app.session.last_dirty_at_ms().is_some_and(|dirty_wall_ms| {
        now_unix_ms() - dirty_wall_ms >= debounce_ms as i64 && elapsed_since_last >= dirty_gate_ms
    });
    let periodic_pull =
        elapsed_since_last >= AUTO_SYNC_IDLE_PULL_MS.max(app.sync_worker.next_retry_delay_ms);

    if should_startup_sync || has_dirty_ready || periodic_pull {
        app.sync_worker.startup_sync_queued = true;
        trigger_manual_sync(app, ctx);
    } else if app.session.last_dirty_at_ms().is_some() {
        ctx.request_repaint_after(Duration::from_millis(500));
    } else {
        ctx.request_repaint_after(Duration::from_secs(6));
    }
}

fn drain_worker_events(app: &mut PaperCraftApp, ctx: &egui::Context) {
    while let Ok(evt) = app.sync_worker.evt_rx.try_recv() {
        apply_worker_event(app, ctx, evt);
    }
}

/// Applies an authenticated cloud session hydrated into the Offline-First cache from `initial_pull`.
pub(crate) fn complete_authenticated_switch(
    app: &mut PaperCraftApp,
    ctx: &egui::Context,
    auth: ActiveAccountSession,
    initial_pull: SyncPullResponse,
) {
    let label = auth.user.primary_label();
    let server_url = auth.server_url.clone();
    prepare_workspace_switch(app);
    app.view.account.apply_auth_ok(&server_url, &label);
    app.view.account.sync_identity_from_session(Some(&auth));
    app.view.account.disconnect_notice = None;
    app.view.profile_return_screen = crate::AppScreen::Home;
    app.session.enter_online_server_session(auth, initial_pull);
    if let Some(msg) = app.session.take_last_conflict_notice() {
        app.view.account.status_message = Some(msg);
    }
    app.sync_worker.next_retry_delay_ms = 8_000.0;
    app.sync_worker.startup_sync_queued = true;
    app.sync_worker.last_attempt_ui_ms = ctx.input(|i| i.time * 1000.0);
    finish_workspace_switch(app, ctx);
    if matches!(
        app.view.active_modal,
        Some(
            crate::modal::ActiveModal::SelectLocalAccount
                | crate::modal::ActiveModal::ConvertLocalAccount
                | crate::modal::ActiveModal::AccountSync
        )
    ) {
        crate::modal::close(app);
    }
    if app.session.cloud_pending_changes_count() > 0 && !app.session.is_sync_in_flight() {
        trigger_manual_sync(app, ctx);
    }
}

/// Signs out cleanly from a server session after flushing local changes, keeping Offline-First
/// server caches or local accounts accessible according to the session settings.
pub(crate) fn complete_sign_out(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let prev_url = app
        .session
        .active_account_session()
        .map(|s| s.server_url.clone())
        .unwrap_or_else(|| app.session.preferred_server_url().to_string());
    let outgoing_sess = app.session.active_account_session().cloned();
    prepare_workspace_switch(app);
    if let Some(sess) = outgoing_sess {
        let _ = dispatch_command(app, Some(ctx), SyncWorkerCommand::Logout { session: sess });
    }
    let _ = app.session.sign_out_from_server_session();
    app.view.screen = crate::AppScreen::Home;
    app.view.profile_return_screen = crate::AppScreen::Home;
    app.view.account.disconnect_notice = None;
    app.view.account.reset_after_sign_out(&prev_url);
    finish_workspace_switch(app, ctx);
    let next_modal = if app.session.has_any_saved_accounts() {
        crate::modal::ActiveModal::SelectLocalAccount
    } else {
        crate::modal::ActiveModal::FirstAccessNickname
    };
    crate::modal::open(app, next_modal);
}

fn handle_health_ok_reconnect(app: &mut PaperCraftApp, ctx: &egui::Context, server_url: &str) {
    let matches_active = app
        .session
        .active_account_session()
        .is_some_and(|s| crate::account_modal::urls_match(&s.server_url, server_url));
    if !app.session.is_viewing_cloud() || !matches_active {
        return;
    }
    app.sync_worker.next_retry_delay_ms = 5_000.0;
    let was_offline = app.session.sync_status_summary().connection
        == papercraft_sync::SyncConnectionState::Offline;
    if (was_offline || app.session.cloud_pending_changes_count() > 0)
        && !app.session.is_sync_in_flight()
    {
        trigger_manual_sync(app, ctx);
    }
}

pub(crate) fn apply_worker_event(
    app: &mut PaperCraftApp,
    ctx: &egui::Context,
    evt: SyncWorkerEvent,
) {
    if let Some(session_gen) = evt.session_generation()
        && session_gen != app.sync_worker.current_session_generation()
    {
        return;
    }
    match evt {
        SyncWorkerEvent::HealthOk { server_url, info } => {
            app.session.set_network_offline_state(false);
            app.view.account.apply_health_ok(server_url.clone(), info);
            handle_health_ok_reconnect(app, ctx, &server_url);
        }
        SyncWorkerEvent::HealthErr {
            server_url,
            message,
        } => {
            app.view.account.apply_health_err(server_url, message);
        }
        SyncWorkerEvent::AuthOk {
            auth, initial_pull, ..
        } => {
            app.session.set_network_offline_state(false);
            complete_authenticated_switch(app, ctx, auth, *initial_pull);
        }
        SyncWorkerEvent::AuthErr {
            offline, message, ..
        } => {
            if offline {
                let target_url = app.view.account.server_url.clone();
                app.view.account.mark_server_offline(&target_url, &message);
            }
            app.view.account.apply_auth_err(message);
        }
        SyncWorkerEvent::ConvertLocalProgress {
            source_profile_id,
            stage,
            completed,
            total,
            ..
        } => {
            if app.view.account.convert_source_profile_id == source_profile_id {
                app.view
                    .account
                    .apply_convert_local_progress(stage, completed, total);
            }
        }
        SyncWorkerEvent::ConvertLocalOk {
            source_profile_id,
            auth,
            initial_pull,
            ..
        } => {
            app.session.set_network_offline_state(false);
            let label = auth.user.primary_label();
            let server_url = auth.server_url.clone();
            prepare_workspace_switch(app);
            app.view.account.apply_convert_local_ok(&server_url, &label);
            app.view.account.sync_identity_from_session(Some(&auth));
            app.view.account.disconnect_notice = None;
            app.session
                .complete_local_account_conversion(&source_profile_id, auth, *initial_pull);
            app.sync_worker.next_retry_delay_ms = 8_000.0;
            app.sync_worker.startup_sync_queued = true;
            app.sync_worker.last_attempt_ui_ms = ctx.input(|i| i.time * 1000.0);
            app.view.profile_return_screen = crate::AppScreen::Home;
            finish_workspace_switch(app, ctx);
            if app.view.active_modal == Some(crate::modal::ActiveModal::ConvertLocalAccount) {
                crate::modal::close(app);
            }
        }
        SyncWorkerEvent::ConvertLocalErr {
            offline, message, ..
        } => {
            if offline {
                let target_url = app.view.account.server_url.clone();
                app.view.account.mark_server_offline(&target_url, &message);
            }
            app.view.account.apply_convert_local_err(message);
        }
        SyncWorkerEvent::PasswordResetRequired {
            identifier,
            prefill_code,
            message,
            ..
        } => {
            app.view
                .account
                .enter_password_reset_flow(identifier, prefill_code, message);
            if app.view.active_modal == Some(crate::modal::ActiveModal::SelectLocalAccount) {
                app.view.account.selector_other_server_open = true;
            }
        }
        SyncWorkerEvent::ProfileUpdated {
            updated_session, ..
        } => {
            let next_username = updated_session.user.username.clone();
            let next_display = updated_session.user.display_name.clone();
            app.session.update_active_session_tokens(updated_session);
            app.view
                .account
                .apply_profile_updated(&next_username, &next_display);
        }
        SyncWorkerEvent::ProfileUpdateErr {
            offline,
            unauthorized,
            message,
            ..
        } => {
            if offline {
                if let Some(url) = app
                    .session
                    .active_account_session()
                    .map(|s| s.server_url.clone())
                {
                    app.view.account.mark_server_offline(&url, &message);
                }
                app.view.account.apply_profile_update_err(
                    "Server is offline — updating your profile requires an online connection."
                        .to_string(),
                );
            } else if unauthorized {
                app.session.record_session_expired(
                    "Session expired — enter your password to resume syncing. Changes remain saved locally."
                        .to_string(),
                );
                app.view.account.apply_profile_update_err(
                    "Session expired — enter your password in the banner above to update your profile."
                        .to_string(),
                );
            } else {
                app.view.account.apply_profile_update_err(message);
            }
        }
        SyncWorkerEvent::PasswordChanged {
            updated_session, ..
        } => {
            app.session.update_active_session_tokens(updated_session);
            app.view.account.apply_password_changed();
        }
        SyncWorkerEvent::PasswordChangeErr {
            offline,
            unauthorized,
            message,
            ..
        } => {
            if offline {
                if let Some(url) = app
                    .session
                    .active_account_session()
                    .map(|s| s.server_url.clone())
                {
                    app.view.account.mark_server_offline(&url, &message);
                }
                app.view.account.apply_password_change_err(
                    "Server is offline — changing your password requires an online connection."
                        .to_string(),
                );
            } else if unauthorized {
                app.session.record_session_expired(
                    "Session expired — enter your password to resume syncing. Changes remain saved locally."
                        .to_string(),
                );
                app.view.account.apply_password_change_err(
                    "Session expired — enter your password in the banner above to change your password."
                        .to_string(),
                );
            } else {
                app.view.account.apply_password_change_err(message);
            }
        }
        SyncWorkerEvent::EmailChanged {
            updated_session, ..
        } => {
            let next_email = updated_session.user.email.clone();
            app.session.update_active_session_tokens(updated_session);
            app.view.account.apply_email_changed(next_email.as_deref());
        }
        SyncWorkerEvent::EmailChangeErr {
            offline,
            unauthorized,
            message,
            ..
        } => {
            if offline {
                if let Some(url) = app
                    .session
                    .active_account_session()
                    .map(|s| s.server_url.clone())
                {
                    app.view.account.mark_server_offline(&url, &message);
                }
                app.view.account.apply_email_change_err(
                    "Server is offline — updating your email requires an online connection."
                        .to_string(),
                );
            } else if unauthorized {
                app.session.record_session_expired(
                    "Session expired — enter your password to resume syncing. Changes remain saved locally."
                        .to_string(),
                );
                app.view.account.apply_email_change_err(
                    "Session expired — enter your password in the banner above to update your email."
                        .to_string(),
                );
            } else {
                app.view.account.apply_email_change_err(message);
            }
        }
        SyncWorkerEvent::SyncOk { outcome, .. } => {
            let matches_active = app
                .session
                .active_account_session()
                .is_some_and(|s| s.user.user_id == outcome.server_user.user_id);
            if matches_active {
                app.sync_worker.pending_outcome = Some(*outcome);
            }
        }
        SyncWorkerEvent::SyncErr {
            offline,
            unauthorized,
            message,
            ..
        } => {
            app.view.account.clear_transient_auth_status();
            let active_url = app
                .session
                .active_account_session()
                .map(|s| s.server_url.clone())
                .unwrap_or_else(|| app.session.server_url().to_string());
            if offline {
                app.view.account.mark_server_offline(&active_url, &message);
                app.session.record_sync_failure(true, message);
                app.sync_worker.next_retry_delay_ms =
                    (app.sync_worker.next_retry_delay_ms * 1.5).clamp(5_000.0, 20_000.0);
            } else if unauthorized {
                app.session.record_session_expired(
                    "Session expired — enter your password to resume syncing. Changes remain saved locally."
                        .to_string(),
                );
                app.sync_worker.next_retry_delay_ms = 60_000.0;
            } else {
                app.session.record_sync_failure(false, message);
                app.sync_worker.next_retry_delay_ms =
                    (app.sync_worker.next_retry_delay_ms * 2.0).clamp(15_000.0, 120_000.0);
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
const CONVERT_OFFLINE_ERR_MSG: &str = "Connection lost while syncing. Your local account and notebooks remain safely saved on this device — retry when the server is back online.";

#[cfg(not(target_arch = "wasm32"))]
fn emit_worker_progress(
    evt_tx: &mpsc::Sender<SyncWorkerEvent>,
    repaint_ctx: &Arc<Mutex<Option<egui::Context>>>,
    evt: SyncWorkerEvent,
) {
    let _ = evt_tx.send(evt);
    if let Ok(guard) = repaint_ctx.lock()
        && let Some(ref ctx) = *guard
    {
        ctx.request_repaint();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn convert_err_event(
    session_generation: u64,
    source_profile_id: String,
    err: &SyncError,
    is_auth: bool,
) -> SyncWorkerEvent {
    let offline = matches!(err, SyncError::Network(_));
    let message = if offline {
        CONVERT_OFFLINE_ERR_MSG.to_string()
    } else if is_auth {
        friendly_auth_error(err)
    } else {
        friendly_error(err)
    };
    SyncWorkerEvent::ConvertLocalErr {
        session_generation,
        source_profile_id,
        offline,
        message,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn worker_loop(
    cmd_rx: mpsc::Receiver<SyncWorkerCommand>,
    evt_tx: mpsc::Sender<SyncWorkerEvent>,
    repaint_ctx: Arc<Mutex<Option<egui::Context>>>,
    busy: Arc<AtomicBool>,
) {
    let client = SyncHttpClient::new();
    while let Ok(cmd) = cmd_rx.recv() {
        busy.store(true, Ordering::Relaxed);
        let maybe_event = execute_worker_command(&client, cmd, &evt_tx, &repaint_ctx);
        busy.store(false, Ordering::Relaxed);

        if let Some(evt) = maybe_event {
            if evt_tx.send(evt).is_err() {
                break;
            }
            if let Ok(guard) = repaint_ctx.lock()
                && let Some(ref ctx) = *guard
            {
                ctx.request_repaint();
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn exec_convert_local_account(
    client: &SyncHttpClient,
    evt_tx: &mpsc::Sender<SyncWorkerEvent>,
    repaint_ctx: &Arc<Mutex<Option<egui::Context>>>,
    cmd: SyncWorkerCommand,
) -> SyncWorkerEvent {
    let SyncWorkerCommand::ConvertLocalAccount {
        session_generation,
        server_url,
        source_profile_id,
        source_nickname,
        email,
        password,
        device_name,
        upload_batch,
    } = cmd
    else {
        return SyncWorkerEvent::ConvertLocalErr {
            session_generation: 0,
            source_profile_id: String::new(),
            offline: false,
            message: "Invalid conversion command.".to_string(),
        };
    };

    let req =
        match build_register_request(&email, None, password, Some(source_nickname), &device_name) {
            Ok(req) => req,
            Err(message) => {
                return SyncWorkerEvent::ConvertLocalErr {
                    session_generation,
                    source_profile_id,
                    offline: false,
                    message,
                };
            }
        };

    let total_docs = upload_batch.local_docs.len();
    let total_steps = total_docs.max(1) + 2;
    emit_worker_progress(
        evt_tx,
        repaint_ctx,
        SyncWorkerEvent::ConvertLocalProgress {
            session_generation,
            source_profile_id: source_profile_id.clone(),
            stage: "Creating cloud account...".to_string(),
            completed: 0,
            total: total_steps,
        },
    );

    let mut auth = match client.register(&server_url, &req, &device_name) {
        Ok(auth) => auth,
        Err(err) => return convert_err_event(session_generation, source_profile_id, &err, true),
    };

    emit_worker_progress(
        evt_tx,
        repaint_ctx,
        SyncWorkerEvent::ConvertLocalProgress {
            session_generation,
            source_profile_id: source_profile_id.clone(),
            stage: format!("Uploading notebooks (0/{total_docs})..."),
            completed: 1,
            total: total_steps,
        },
    );

    let mut driver = SyncCycleDriver::new(&upload_batch);
    let mut uploaded_docs = 0usize;
    while let Some(push_req) = driver.next_push_request(&upload_batch) {
        let chunk_docs = push_req.changes.len();
        let (resp, _) = match client.push_with_refresh(&mut auth, &push_req) {
            Ok(res) => res,
            Err(err) => {
                return convert_err_event(session_generation, source_profile_id, &err, false);
            }
        };
        driver.record_push_chunk_response(resp, &upload_batch);
        uploaded_docs = uploaded_docs.saturating_add(chunk_docs).min(total_docs);
        let done_step = if total_docs == 0 {
            2
        } else {
            1 + uploaded_docs
        };
        emit_worker_progress(
            evt_tx,
            repaint_ctx,
            SyncWorkerEvent::ConvertLocalProgress {
                session_generation,
                source_profile_id: source_profile_id.clone(),
                stage: format!("Uploading notebooks ({uploaded_docs}/{total_docs})..."),
                completed: done_step,
                total: total_steps,
            },
        );
    }

    emit_worker_progress(
        evt_tx,
        repaint_ctx,
        SyncWorkerEvent::ConvertLocalProgress {
            session_generation,
            source_profile_id: source_profile_id.clone(),
            stage: "Finalizing Offline-First cache...".to_string(),
            completed: total_steps.saturating_sub(1),
            total: total_steps,
        },
    );
    match client.pull_with_refresh(&mut auth, 0) {
        Ok((pull_resp, _)) => SyncWorkerEvent::ConvertLocalOk {
            session_generation,
            source_profile_id,
            auth,
            initial_pull: Box::new(pull_resp),
        },
        Err(err) => convert_err_event(session_generation, source_profile_id, &err, false),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pull_initial_auth_event(
    client: &SyncHttpClient,
    session_generation: u64,
    mut auth: ActiveAccountSession,
) -> SyncWorkerEvent {
    match client.pull_with_refresh(&mut auth, 0) {
        Ok((pull_resp, _)) => SyncWorkerEvent::AuthOk {
            session_generation,
            auth,
            initial_pull: Box::new(pull_resp),
        },
        Err(err) => SyncWorkerEvent::AuthErr {
            session_generation,
            offline: matches!(err, SyncError::Network(_)),
            message: friendly_error(&err),
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn exec_worker_register(client: &SyncHttpClient, cmd: SyncWorkerCommand) -> SyncWorkerEvent {
    let SyncWorkerCommand::Register {
        session_generation,
        server_url,
        email,
        username,
        password,
        display_name,
        device_name,
    } = cmd
    else {
        return SyncWorkerEvent::AuthErr {
            session_generation: 0,
            offline: false,
            message: "Invalid registration command.".to_string(),
        };
    };
    let req = match build_register_request(
        &email,
        username.as_deref(),
        password,
        display_name,
        &device_name,
    ) {
        Ok(req) => req,
        Err(message) => {
            return SyncWorkerEvent::AuthErr {
                session_generation,
                offline: false,
                message,
            };
        }
    };
    match client.register(&server_url, &req, &device_name) {
        Ok(auth) => pull_initial_auth_event(client, session_generation, auth),
        Err(err) => SyncWorkerEvent::AuthErr {
            session_generation,
            offline: matches!(err, SyncError::Network(_)),
            message: friendly_auth_error(&err),
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn exec_worker_login(
    client: &SyncHttpClient,
    session_generation: u64,
    server_url: &str,
    identifier: String,
    password: String,
    device_name: &str,
) -> SyncWorkerEvent {
    let submitted_password = password.clone();
    let req = match build_login_request(&identifier, password, device_name) {
        Ok(req) => req,
        Err(message) => {
            return SyncWorkerEvent::AuthErr {
                session_generation,
                offline: false,
                message,
            };
        }
    };
    match client.login(server_url, &req, device_name) {
        Ok(auth) => pull_initial_auth_event(client, session_generation, auth),
        Err(SyncError::Server { code, message, .. })
            if code == ERR_CODE_PASSWORD_CHANGE_REQUIRED =>
        {
            SyncWorkerEvent::PasswordResetRequired {
                session_generation,
                identifier,
                prefill_code: Some(submitted_password.trim().to_string()),
                message,
            }
        }
        Err(SyncError::Server { code, message, .. })
            if code == ERR_CODE_PASSWORD_RESET_REQUIRED =>
        {
            SyncWorkerEvent::PasswordResetRequired {
                session_generation,
                identifier,
                prefill_code: None,
                message,
            }
        }
        Err(err) => SyncWorkerEvent::AuthErr {
            session_generation,
            offline: matches!(err, SyncError::Network(_)),
            message: friendly_auth_error(&err),
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn exec_worker_complete_password_reset(
    client: &SyncHttpClient,
    session_generation: u64,
    server_url: &str,
    identifier: &str,
    recovery_code: &str,
    new_password: String,
    device_name: &str,
) -> SyncWorkerEvent {
    let req = match build_complete_password_reset_request(
        identifier,
        recovery_code,
        new_password,
        device_name,
    ) {
        Ok(req) => req,
        Err(message) => {
            return SyncWorkerEvent::AuthErr {
                session_generation,
                offline: false,
                message,
            };
        }
    };
    match client.complete_password_reset(server_url, &req, device_name) {
        Ok(auth) => pull_initial_auth_event(client, session_generation, auth),
        Err(err) => SyncWorkerEvent::AuthErr {
            session_generation,
            offline: matches!(err, SyncError::Network(_)),
            message: friendly_auth_error(&err),
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn execute_worker_command(
    client: &SyncHttpClient,
    cmd: SyncWorkerCommand,
    evt_tx: &mpsc::Sender<SyncWorkerEvent>,
    repaint_ctx: &Arc<Mutex<Option<egui::Context>>>,
) -> Option<SyncWorkerEvent> {
    match cmd {
        SyncWorkerCommand::TestConnection { server_url } => {
            let norm = papercraft_sync_protocol::normalize_server_url(&server_url)
                .unwrap_or_else(|_| server_url.clone());
            match client.health(&norm) {
                Ok(info) => Some(SyncWorkerEvent::HealthOk {
                    server_url: norm,
                    info,
                }),
                Err(err) => Some(SyncWorkerEvent::HealthErr {
                    server_url: norm,
                    message: friendly_error(&err),
                }),
            }
        }
        SyncWorkerCommand::ReportLocalDiscovery { server_url, report } => {
            let norm =
                papercraft_sync_protocol::normalize_server_url(&server_url).unwrap_or(server_url);
            let _ = client.report_local_discovery(&norm, &report);
            None
        }
        cmd @ SyncWorkerCommand::Register { .. } => Some(exec_worker_register(client, cmd)),
        SyncWorkerCommand::Login {
            session_generation,
            server_url,
            identifier,
            password,
            device_name,
        } => Some(exec_worker_login(
            client,
            session_generation,
            &server_url,
            identifier,
            password,
            &device_name,
        )),
        SyncWorkerCommand::CompletePasswordReset {
            session_generation,
            server_url,
            identifier,
            recovery_code,
            new_password,
            device_name,
        } => Some(exec_worker_complete_password_reset(
            client,
            session_generation,
            &server_url,
            &identifier,
            &recovery_code,
            new_password,
            &device_name,
        )),
        cmd @ SyncWorkerCommand::ConvertLocalAccount { .. } => {
            Some(exec_convert_local_account(client, evt_tx, repaint_ctx, cmd))
        }
        SyncWorkerCommand::UpdateProfile {
            session_generation,
            mut session,
            username,
            display_name,
        } => {
            let req =
                match build_update_profile_request(username.as_deref(), display_name.as_deref()) {
                    Ok(req) => req,
                    Err(message) => {
                        return Some(SyncWorkerEvent::ProfileUpdateErr {
                            session_generation,
                            offline: false,
                            unauthorized: false,
                            message,
                        });
                    }
                };
            match client.update_profile_with_refresh(&mut session, &req) {
                Ok((profile, _)) => {
                    session.user = profile;
                    Some(SyncWorkerEvent::ProfileUpdated {
                        session_generation,
                        updated_session: session,
                    })
                }
                Err(err) => Some(SyncWorkerEvent::ProfileUpdateErr {
                    session_generation,
                    offline: matches!(err, SyncError::Network(_)),
                    unauthorized: matches!(err, SyncError::Unauthorized(_)),
                    message: friendly_error(&err),
                }),
            }
        }
        SyncWorkerCommand::ChangePassword {
            session_generation,
            mut session,
            current_password,
            new_password,
        } => {
            let req = match build_change_password_request(current_password, new_password) {
                Ok(req) => req,
                Err(message) => {
                    return Some(SyncWorkerEvent::PasswordChangeErr {
                        session_generation,
                        offline: false,
                        unauthorized: false,
                        message,
                    });
                }
            };
            match client.change_password_with_refresh(&mut session, &req) {
                Ok((profile, _)) => {
                    session.user = profile;
                    Some(SyncWorkerEvent::PasswordChanged {
                        session_generation,
                        updated_session: session,
                    })
                }
                Err(err) => Some(SyncWorkerEvent::PasswordChangeErr {
                    session_generation,
                    offline: matches!(err, SyncError::Network(_)),
                    unauthorized: matches!(err, SyncError::Unauthorized(_)),
                    message: friendly_error(&err),
                }),
            }
        }
        SyncWorkerCommand::ChangeEmail {
            session_generation,
            mut session,
            new_email,
            current_password,
        } => {
            let req = match build_change_email_request(&new_email, current_password) {
                Ok(req) => req,
                Err(message) => {
                    return Some(SyncWorkerEvent::EmailChangeErr {
                        session_generation,
                        offline: false,
                        unauthorized: false,
                        message,
                    });
                }
            };
            match client.change_email_with_refresh(&mut session, &req) {
                Ok((profile, _)) => {
                    session.user = profile;
                    Some(SyncWorkerEvent::EmailChanged {
                        session_generation,
                        updated_session: session,
                    })
                }
                Err(err) => Some(SyncWorkerEvent::EmailChangeErr {
                    session_generation,
                    offline: matches!(err, SyncError::Network(_)),
                    unauthorized: matches!(err, SyncError::Unauthorized(_)),
                    message: friendly_error(&err),
                }),
            }
        }
        SyncWorkerCommand::SyncNow {
            session_generation,
            mut session,
            batch,
        } => match run_sync_cycle(client, &mut session, &batch) {
            Ok(outcome) => Some(SyncWorkerEvent::SyncOk {
                session_generation,
                outcome: Box::new(outcome),
            }),
            Err(err) => Some(sync_err_event(session_generation, &err)),
        },
        SyncWorkerCommand::Logout { session } => {
            let _ = client.logout(&session);
            None
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sync_err_event(session_generation: u64, err: &SyncError) -> SyncWorkerEvent {
    SyncWorkerEvent::SyncErr {
        session_generation,
        offline: matches!(err, SyncError::Network(_)),
        unauthorized: matches!(err, SyncError::Unauthorized(_)),
        message: friendly_error(err),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn friendly_auth_error(err: &SyncError) -> String {
    match err {
        SyncError::Unauthorized(msg) if !msg.trim().is_empty() => msg.clone(),
        _ => friendly_error(err),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn friendly_error(err: &SyncError) -> String {
    match err {
        SyncError::Network(_) => {
            "Server is currently unreachable. Verify it is running and reachable over your LAN or VPN."
                .to_string()
        }
        SyncError::Unauthorized(_) => {
            "Session expired or unauthorized. Please sign in again.".to_string()
        }
        SyncError::Server { message, .. } => message.clone(),
        SyncError::Protocol(p) => p.to_string(),
        SyncError::Io(msg) | SyncError::InvalidResponse(msg) => msg.clone(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn sync_error_clears_stale_syncing_status_banner() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.view.account.status_message =
            Some("Signed in as @alyx. Syncing notebooks...".to_string());

        let session_gen = app.sync_worker.current_session_generation();
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::SyncErr {
                session_generation: session_gen,
                offline: false,
                unauthorized: false,
                message: "Server error".to_string(),
            },
        );

        assert!(app.view.account.status_message.is_none());
        assert_eq!(
            app.session.sync_status_summary().last_error.as_deref(),
            Some("Server error")
        );
    }

    #[test]
    fn offline_sync_error_keeps_cloud_account_accessible_and_queues_changes() {
        let ctx = egui::Context::default();
        let auth = ActiveAccountSession {
            server_url: "http://127.0.0.1:8787".to_string(),
            user: papercraft_sync_protocol::UserProfile {
                user_id: "u-existing".to_string(),
                email: Some("alyx@example.com".to_string()),
                username: "alyx".to_string(),
                display_name: "Alyx".to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "acc".to_string(),
            refresh_token: "ref".to_string(),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "TestPC".to_string(),
            last_auth_ms: 1000,
        };

        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute("profile.completeFirstAccess", &json_nick("Personal"))
            .unwrap();
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = None;
        let empty_pull = SyncPullResponse {
            server_cursor: 0,
            user: auth.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(auth, empty_pull);

        // Going offline must NEVER log out or block the user's cached cloud notebooks.
        let session_gen = app.sync_worker.current_session_generation();
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::SyncErr {
                session_generation: session_gen,
                offline: true,
                unauthorized: false,
                message: "Connection refused".to_string(),
            },
        );
        assert!(app.session.is_signed_in());
        assert!(app.session.is_viewing_cloud());
        assert_eq!(
            app.session.sync_status_summary().connection,
            papercraft_sync::SyncConnectionState::Offline
        );
        assert_eq!(app.view.active_modal, None);

        // Editing offline queues changes locally to push automatically upon reconnect.
        app.session
            .execute(
                "notebook.create",
                &serde_json::json!({ "title": "Plane Notes" }),
            )
            .unwrap();
        assert!(app.session.cloud_pending_changes_count() >= 1);
    }

    #[test]
    fn convert_local_ok_preserves_local_cache_and_enters_cloud_session() {
        let ctx = egui::Context::default();
        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute("profile.completeFirstAccess", &json_nick("LocalUser"))
            .unwrap();
        session
            .execute(
                "notebook.create",
                &serde_json::json!({ "title": "Physics Notes" }),
            )
            .unwrap();
        let local_id = session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = Some(crate::modal::ActiveModal::ConvertLocalAccount);
        let auth = ActiveAccountSession {
            server_url: "http://127.0.0.1:8787".to_string(),
            user: papercraft_sync_protocol::UserProfile {
                user_id: "u-conv".to_string(),
                email: Some("conv@example.com".to_string()),
                username: "localuser".to_string(),
                display_name: "LocalUser".to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "acc".to_string(),
            refresh_token: "ref".to_string(),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "TestPC".to_string(),
            last_auth_ms: 1000,
        };
        let pull = SyncPullResponse {
            server_cursor: 1,
            user: auth.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };

        let session_gen = app.sync_worker.current_session_generation();
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::ConvertLocalOk {
                session_generation: session_gen,
                source_profile_id: local_id,
                auth,
                initial_pull: Box::new(pull),
            },
        );

        assert!(app.session.is_signed_in());
        assert!(app.session.is_viewing_cloud());
        assert!(!app.session.has_saved_local_accounts());
        assert!(
            app.session
                .notebooks
                .iter()
                .any(|d| d.title == "Physics Notes"),
            "converted local notebooks must stay cached locally for Offline-First access"
        );
        assert_eq!(app.view.active_modal, None);
    }

    #[test]
    fn local_discovery_skips_active_stroke_and_throttles_per_frame_polling() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let ctx = egui::Context::default();
        let dispatch_count = Arc::new(AtomicUsize::new(0));
        let dispatch_clone = Arc::clone(&dispatch_count);

        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute("profile.completeFirstAccess", &json_nick("Tester"))
            .unwrap();
        let hooks = crate::PlatformHooks {
            web_sync_dispatch: Some(Box::new(move |cmd, _tx, _ctx| {
                if matches!(cmd, SyncWorkerCommand::ReportLocalDiscovery { .. }) {
                    dispatch_clone.fetch_add(1, Ordering::Relaxed);
                }
            })),
            ..crate::PlatformHooks::default()
        };
        let mut app = PaperCraftApp::new(session, hooks);

        // Active drawing must completely defer discovery checks.
        app.session
            .begin_live_stroke(papercraft_doc::StrokeSample::from_xy_pressure(
                10.0, 20.0, 0.5,
            ));
        poll_and_tick(&mut app, &ctx, 100.0);
        assert_eq!(dispatch_count.load(Ordering::Relaxed), 0);
        assert_eq!(app.sync_worker.last_discovery_check_ui_ms, None);

        // First idle frame sends initial report.
        app.session.cancel_live_stroke();
        poll_and_tick(&mut app, &ctx, 100.0);
        assert_eq!(dispatch_count.load(Ordering::Relaxed), 1);
        assert_eq!(app.sync_worker.last_discovery_check_ui_ms, Some(100.0));

        // High-frequency idle frames within 2s must not re-poll discovery even on revision bump.
        app.session
            .execute("notebook.create", &serde_json::json!({ "title": "Second" }))
            .unwrap();
        for t in [116.0, 250.0, 1000.0, 1900.0] {
            poll_and_tick(&mut app, &ctx, t);
        }
        assert_eq!(app.sync_worker.last_discovery_check_ui_ms, Some(100.0));
        assert_eq!(dispatch_count.load(Ordering::Relaxed), 1);

        // After 2s debounce with changed state, discovery refreshes cleanly and dispatches updated count.
        poll_and_tick(&mut app, &ctx, 2200.0);
        assert_eq!(app.sync_worker.last_discovery_check_ui_ms, Some(2200.0));
        assert_eq!(dispatch_count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn adaptive_sync_debounce_scales_with_dirty_payload_size() {
        assert_eq!(adaptive_sync_debounce_ms(0), 1_500.0);
        assert_eq!(adaptive_sync_debounce_ms(1024), 1_500.0);
        assert_eq!(adaptive_sync_debounce_ms(4 * 1024 * 1024), 3_500.0);
        assert_eq!(adaptive_sync_debounce_ms(12 * 1024 * 1024), 3_500.0);
        assert_eq!(adaptive_sync_debounce_ms(32 * 1024 * 1024), 6_000.0);
        assert_eq!(adaptive_sync_debounce_ms(128 * 1024 * 1024), 6_000.0);
    }

    #[test]
    fn convert_progress_offline_interrupt_preserves_local_and_health_ok_triggers_reconnect_sync() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let ctx = egui::Context::default();
        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute("profile.completeFirstAccess", &json_nick("LocalSafe"))
            .unwrap();
        session
            .execute(
                "notebook.create",
                &serde_json::json!({ "title": "Important Sketch" }),
            )
            .unwrap();
        let local_id = session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();

        let sync_dispatches = Arc::new(AtomicUsize::new(0));
        let sync_clone = Arc::clone(&sync_dispatches);
        let hooks = crate::PlatformHooks {
            web_sync_dispatch: Some(Box::new(move |cmd, _tx, _ctx| {
                if matches!(cmd, SyncWorkerCommand::SyncNow { .. }) {
                    sync_clone.fetch_add(1, Ordering::Relaxed);
                }
            })),
            ..crate::PlatformHooks::default()
        };
        let mut app = PaperCraftApp::new(session, hooks);
        app.view.active_modal = Some(crate::modal::ActiveModal::ConvertLocalAccount);
        app.view.account.convert_source_profile_id = local_id.clone();
        app.view.account.convert_busy = true;

        // Progress updates reflect stage and counts accurately.
        let session_gen = app.sync_worker.current_session_generation();
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::ConvertLocalProgress {
                session_generation: session_gen,
                source_profile_id: local_id.clone(),
                stage: "Uploading notebooks (1/2)...".to_string(),
                completed: 2,
                total: 4,
            },
        );
        assert_eq!(
            app.view.account.convert_progress_stage,
            "Uploading notebooks (1/2)..."
        );
        assert_eq!(app.view.account.convert_progress_completed, 2);
        assert_eq!(app.view.account.convert_progress_total, 4);

        // Mid-sync network drop keeps the modal open, marks server offline, and keeps all local notebooks intact.
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::ConvertLocalErr {
                session_generation: session_gen,
                source_profile_id: local_id,
                offline: true,
                message: "Connection lost while syncing. Your local account and notebooks remain safely saved on this device — retry when the server is back online.".to_string(),
            },
        );
        assert!(!app.view.account.convert_busy);
        assert!(!app.session.is_viewing_cloud());
        assert!(
            app.session
                .notebooks
                .iter()
                .any(|d| d.title == "Important Sketch")
        );
        assert!(matches!(
            app.view.account.probe_for("http://127.0.0.1:8787"),
            crate::account_modal::ServerProbeState::Failed { .. }
        ));

        // Now simulate viewing an offline cloud session whose connection probe recovers via HealthOk.
        let auth = ActiveAccountSession {
            server_url: "http://127.0.0.1:8787".to_string(),
            user: papercraft_sync_protocol::UserProfile {
                user_id: "u-rec".to_string(),
                email: Some("rec@example.com".to_string()),
                username: "rec".to_string(),
                display_name: "Rec".to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "acc".to_string(),
            refresh_token: "ref".to_string(),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "PC".to_string(),
            last_auth_ms: 1000,
        };
        let pull = SyncPullResponse {
            server_cursor: 1,
            user: auth.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(auth, pull);
        app.session
            .record_sync_failure(true, "Connection refused".to_string());
        app.sync_worker.next_retry_delay_ms = 20_000.0;
        let before = sync_dispatches.load(Ordering::Relaxed);

        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::HealthOk {
                server_url: "http://127.0.0.1:8787".to_string(),
                info: papercraft_sync_protocol::ServerInfoResponse {
                    status: "ok".to_string(),
                    service: "papercraft-server".to_string(),
                    version: "0.1.0".to_string(),
                    protocol_version: 1,
                    registration_open: true,
                    storage_engine: "redb".to_string(),
                    max_notebook_bytes: 1_048_576,
                    local_discovery: papercraft_sync_protocol::LocalDiscoverySummary::default(),
                },
            },
        );
        assert_eq!(sync_dispatches.load(Ordering::Relaxed), before + 1);
        assert_eq!(app.sync_worker.next_retry_delay_ms, 5_000.0);
    }

    #[test]
    fn expired_session_pauses_polling_and_reauth_immediately_uploads_queued_edits() {
        use papercraft_sync_protocol::{
            RemoteNotebookRecord, canonical_document_body_hash, canonical_document_hash,
            canonical_document_meta_hash,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};

        let ctx = egui::Context::default();
        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute("profile.completeFirstAccess", &json_nick("ReauthUser"))
            .unwrap();

        let sync_dispatches = Arc::new(AtomicUsize::new(0));
        let sync_clone = Arc::clone(&sync_dispatches);
        let hooks = crate::PlatformHooks {
            web_sync_dispatch: Some(Box::new(move |cmd, _tx, _ctx| {
                if matches!(cmd, SyncWorkerCommand::SyncNow { .. }) {
                    sync_clone.fetch_add(1, Ordering::Relaxed);
                }
            })),
            ..crate::PlatformHooks::default()
        };
        let mut app = PaperCraftApp::new(session, hooks);
        let auth = ActiveAccountSession {
            server_url: "http://127.0.0.1:8787".to_string(),
            user: papercraft_sync_protocol::UserProfile {
                user_id: "u-exp-ui".to_string(),
                email: Some("reauth@example.com".to_string()),
                username: "reauthuser".to_string(),
                display_name: "Reauth".to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "old_tok".to_string(),
            refresh_token: "old_ref".to_string(),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "PC".to_string(),
            last_auth_ms: 1000,
        };
        let synced_doc = papercraft_doc::Document {
            id: "nb-reauth".to_string(),
            title: "Original Title".to_string(),
            ..Default::default()
        };
        let record = RemoteNotebookRecord {
            notebook_id: "nb-reauth".to_string(),
            version: 1,
            cursor: 1,
            deleted: false,
            updated_at_ms: 1000,
            deleted_at_ms: None,
            title: "Original Title".to_string(),
            content_hash: canonical_document_hash(&synced_doc),
            body_hash: canonical_document_body_hash(&synced_doc),
            meta_hash: canonical_document_meta_hash(&synced_doc),
            meta_updated_at_ms: 1000,
            document: Some(synced_doc),
        };
        let pull = SyncPullResponse {
            server_cursor: 1,
            user: auth.user.clone(),
            notebooks: vec![record.clone()],
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(auth.clone(), pull);

        // Token expires online: keep viewing cloud, do NOT log out, mark expired, pause polling.
        let session_gen = app.sync_worker.current_session_generation();
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::SyncErr {
                session_generation: session_gen,
                offline: false,
                unauthorized: true,
                message: "expired".to_string(),
            },
        );
        assert!(app.session.is_signed_in());
        assert!(app.session.is_viewing_cloud());
        assert!(app.session.is_session_expired());

        app.session
            .execute(
                "notebook.rename",
                &serde_json::json!({ "title": "Queued While Expired" }),
            )
            .unwrap();
        assert_eq!(app.session.cloud_pending_changes_count(), 1);

        let before = sync_dispatches.load(Ordering::Relaxed);
        poll_and_tick(&mut app, &ctx, 200_000.0);
        assert_eq!(
            sync_dispatches.load(Ordering::Relaxed),
            before,
            "background polling must pause while waiting for password re-entry"
        );

        // Re-authentication succeeds: clears expired flag and immediately dispatches SyncNow for queued edits!
        let mut renewed_auth = auth;
        renewed_auth.access_token = "new_tok".to_string();
        let renewed_pull = SyncPullResponse {
            server_cursor: 1,
            user: renewed_auth.user.clone(),
            notebooks: vec![record],
            folders: Vec::new(),
            studio_prefs: None,
        };
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::AuthOk {
                session_generation: session_gen,
                auth: renewed_auth,
                initial_pull: Box::new(renewed_pull),
            },
        );
        assert!(!app.session.is_session_expired());
        assert_eq!(app.session.notebooks.len(), 1);
        assert_eq!(app.session.notebooks[0].title, "Queued While Expired");
        assert_eq!(
            sync_dispatches.load(Ordering::Relaxed),
            before + 1,
            "re-authenticating with queued offline changes must immediately trigger sync"
        );
    }

    fn json_nick(nick: &str) -> serde_json::Value {
        serde_json::json!({ "nickname": nick })
    }

    #[test]
    fn build_login_request_requires_email_and_rejects_username_only() {
        let err =
            build_login_request("alex_nick", "SecretPass1!".to_string(), "TestPC").unwrap_err();
        assert!(err.to_ascii_lowercase().contains("email"), "{err}");

        let req = build_login_request("  Alex@Example.COM ", "SecretPass1!".to_string(), "TestPC")
            .expect("normalized email login request");
        assert_eq!(req.email.as_deref(), Some("alex@example.com"));
        assert!(req.username.is_empty());
    }

    #[test]
    fn build_complete_password_reset_request_requires_email() {
        let err = build_complete_password_reset_request(
            "alex_nick",
            "REC-123",
            "NewSecretPass1!".to_string(),
            "TestPC",
        )
        .unwrap_err();
        assert!(err.to_ascii_lowercase().contains("email"), "{err}");

        let req = build_complete_password_reset_request(
            "Alyx@Example.COM",
            "REC-123",
            "NewSecretPass1!".to_string(),
            "TestPC",
        )
        .expect("valid email reset request");
        assert_eq!(req.email.as_deref(), Some("alyx@example.com"));
        assert!(req.username.is_empty());
    }

    #[test]
    fn sign_out_opens_account_selector_when_remembered_server_account_exists_and_first_access_when_none()
     {
        let ctx = egui::Context::default();
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        assert!(app.session.needs_initial_nickname());
        assert!(!app.session.has_any_saved_accounts());

        let auth = ActiveAccountSession {
            server_url: "http://127.0.0.1:8787".to_string(),
            user: papercraft_sync_protocol::UserProfile {
                user_id: "u-only-srv".to_string(),
                email: Some("only@example.com".to_string()),
                username: "onlyuser".to_string(),
                display_name: "Only User".to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "acc".to_string(),
            refresh_token: "ref".to_string(),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "TestPC".to_string(),
            last_auth_ms: 1000,
        };
        let empty_pull = SyncPullResponse {
            server_cursor: 0,
            user: auth.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(auth, empty_pull);

        // Zero local profiles, 1 remembered server account -> Sign Out opens Choose an Account selector.
        complete_sign_out(&mut app, &ctx);
        assert!(app.session.needs_initial_nickname());
        assert!(app.session.has_any_saved_accounts());
        assert_eq!(
            app.view.active_modal,
            Some(crate::modal::ActiveModal::SelectLocalAccount)
        );

        // Forgetting that last server account drops saved accounts to 0 -> opens FirstAccessNickname.
        complete_forget_server_account(
            &mut app,
            &ctx,
            "http://127.0.0.1:8787",
            "u-only-srv",
            "Only User",
        )
        .expect("forget remembered server account");
        assert!(!app.session.has_any_saved_accounts());
        assert_eq!(
            app.view.active_modal,
            Some(crate::modal::ActiveModal::FirstAccessNickname)
        );
    }

    fn make_test_auth(
        user_id: &str,
        email: &str,
        username: &str,
        display_name: &str,
    ) -> ActiveAccountSession {
        ActiveAccountSession {
            server_url: "http://127.0.0.1:8787".to_string(),
            user: papercraft_sync_protocol::UserProfile {
                user_id: user_id.to_string(),
                email: Some(email.to_string()),
                username: username.to_string(),
                display_name: display_name.to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: format!("acc-{user_id}"),
            refresh_token: format!("ref-{user_id}"),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "TestPC".to_string(),
            last_auth_ms: 1000,
        }
    }

    fn make_remote_notebook(
        notebook_id: &str,
        title: &str,
    ) -> papercraft_sync_protocol::RemoteNotebookRecord {
        let doc = papercraft_doc::Document {
            id: notebook_id.to_string(),
            title: title.to_string(),
            ..Default::default()
        };
        papercraft_sync_protocol::RemoteNotebookRecord {
            notebook_id: notebook_id.to_string(),
            version: 1,
            cursor: 1,
            deleted: false,
            updated_at_ms: 1000,
            deleted_at_ms: None,
            title: title.to_string(),
            content_hash: papercraft_sync_protocol::canonical_document_hash(&doc),
            body_hash: papercraft_sync_protocol::canonical_document_body_hash(&doc),
            meta_hash: papercraft_sync_protocol::canonical_document_meta_hash(&doc),
            meta_updated_at_ms: 1000,
            document: Some(doc),
        }
    }

    #[test]
    fn logout_then_second_server_account_login_isolates_notebooks_and_preserves_first_cache() {
        let ctx = egui::Context::default();
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());

        let auth_a = make_test_auth("u-a", "alice@example.com", "alice", "Alice");
        let pull_a = SyncPullResponse {
            server_cursor: 1,
            user: auth_a.user.clone(),
            notebooks: vec![make_remote_notebook("nb-alice", "Alice Notebook")],
            folders: Vec::new(),
            studio_prefs: None,
        };
        complete_authenticated_switch(&mut app, &ctx, auth_a, pull_a);
        assert!(app.session.notebooks.iter().any(|d| d.id == "nb-alice"));

        complete_sign_out(&mut app, &ctx);
        assert!(!app.session.is_signed_in());
        assert_eq!(
            app.view.active_modal,
            Some(crate::modal::ActiveModal::SelectLocalAccount)
        );
        assert!(
            !app.session.notebooks.iter().any(|d| d.id == "nb-alice"),
            "signing out must clear Account A's active notebook list from memory"
        );
        assert!(
            app.session
                .remembered_server_accounts()
                .iter()
                .any(|a| a.user_id == "u-a"),
            "Account A must stay remembered for quick offline/online re-entry"
        );

        let session_gen = app.sync_worker.current_session_generation();
        let auth_b = make_test_auth("u-b", "bob@example.com", "bob", "Bob");
        let pull_b = SyncPullResponse {
            server_cursor: 2,
            user: auth_b.user.clone(),
            notebooks: vec![make_remote_notebook("nb-bob", "Bob Notebook")],
            folders: Vec::new(),
            studio_prefs: None,
        };
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::AuthOk {
                session_generation: session_gen,
                auth: auth_b,
                initial_pull: Box::new(pull_b),
            },
        );

        assert!(app.session.is_signed_in());
        assert_eq!(
            app.session
                .active_account_session()
                .map(|s| s.user.user_id.as_str()),
            Some("u-b")
        );
        assert_eq!(app.session.notebooks.len(), 1);
        assert_eq!(app.session.notebooks[0].id, "nb-bob");
        assert_eq!(
            app.session
                .server_account_notebook_count("http://127.0.0.1:8787", "u-a"),
            1,
            "Account A's offline notebook cache must remain preserved"
        );
    }

    #[test]
    fn stale_session_generation_events_are_dropped_after_account_switch() {
        let ctx = egui::Context::default();
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());

        let auth_a = make_test_auth("u-a", "alice@example.com", "alice", "Alice");
        let pull_a = SyncPullResponse {
            server_cursor: 1,
            user: auth_a.user.clone(),
            notebooks: vec![make_remote_notebook("nb-alice", "Alice Notebook")],
            folders: Vec::new(),
            studio_prefs: None,
        };
        complete_authenticated_switch(&mut app, &ctx, auth_a, pull_a);
        let stale_gen = app.sync_worker.current_session_generation();

        // Switch directly into a fresh local account -> generation bumps.
        create_local_account_and_switch(&mut app, &ctx, "OfflineUser").unwrap();
        assert!(!app.session.is_signed_in());
        assert!(app.sync_worker.current_session_generation() > stale_gen);

        // Delayed error or SyncOk from Account A must be silently ignored.
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::SyncErr {
                session_generation: stale_gen,
                offline: false,
                unauthorized: true,
                message: "stale expired".to_string(),
            },
        );
        assert!(!app.session.is_session_expired());
        assert!(app.session.sync_status_summary().last_error.is_none());

        let auth_b = make_test_auth("u-b", "bob@example.com", "bob", "Bob");
        let pull_b = SyncPullResponse {
            server_cursor: 2,
            user: auth_b.user.clone(),
            notebooks: vec![make_remote_notebook("nb-bob", "Bob Notebook")],
            folders: Vec::new(),
            studio_prefs: None,
        };
        apply_worker_event(
            &mut app,
            &ctx,
            SyncWorkerEvent::AuthOk {
                session_generation: stale_gen,
                auth: auth_b,
                initial_pull: Box::new(pull_b),
            },
        );
        assert!(
            !app.session.is_signed_in(),
            "stale AuthOk from previous generation must not hijack the new workspace"
        );
    }

    #[test]
    fn direct_cloud_to_cloud_switch_preserves_first_account_shortcut_and_pending_edits() {
        let ctx = egui::Context::default();
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());

        let auth_a = make_test_auth("u-a", "alice@example.com", "alice", "Alice");
        let pull_a = SyncPullResponse {
            server_cursor: 1,
            user: auth_a.user.clone(),
            notebooks: vec![make_remote_notebook("nb-alice", "Alice Notebook")],
            folders: Vec::new(),
            studio_prefs: None,
        };
        complete_authenticated_switch(&mut app, &ctx, auth_a.clone(), pull_a.clone());

        // Make an unsynced local edit while signed in as Account A.
        app.session
            .execute(
                "notebook.create",
                &serde_json::json!({ "title": "Alice Offline Draft" }),
            )
            .unwrap();
        assert!(app.session.cloud_pending_changes_count() >= 1);

        // Directly switch to Account B without signing out first.
        let auth_b = make_test_auth("u-b", "bob@example.com", "bob", "Bob");
        let pull_b = SyncPullResponse {
            server_cursor: 3,
            user: auth_b.user.clone(),
            notebooks: vec![make_remote_notebook("nb-bob", "Bob Notebook")],
            folders: Vec::new(),
            studio_prefs: None,
        };
        complete_authenticated_switch(&mut app, &ctx, auth_b, pull_b);

        assert_eq!(
            app.session
                .active_account_session()
                .map(|s| s.user.user_id.as_str()),
            Some("u-b")
        );
        assert!(
            app.session
                .notebooks
                .iter()
                .all(|d| d.title == "Bob Notebook")
        );
        assert_eq!(
            app.session
                .server_account_notebook_count("http://127.0.0.1:8787", "u-a"),
            2,
            "Account A's unsynced draft and synced notebook must both stay isolated in Account A's cache"
        );
        assert_eq!(
            app.session
                .server_account_pending_changes_count("http://127.0.0.1:8787", "u-a"),
            1,
            "Account A's cached pending-change count must remain 1"
        );
    }
}
