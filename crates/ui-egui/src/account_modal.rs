//! Account sign-in, Offline-First server caching, local profiles, and local-to-cloud conversion.

use std::collections::{BTreeSet, HashMap};

use egui::{Color32, CornerRadius, Margin, Rect, Sense, Stroke, StrokeKind, Vec2};
use papercraft_sync::{
    EntitySyncStatus, RememberedServerAccount, SyncConnectionState, SyncStatusSummary,
};
use papercraft_sync_protocol::{
    DEFAULT_SERVER_URL, MIN_PASSWORD_LEN, ServerInfoResponse, normalize_email,
    normalize_server_url, normalize_username, now_unix_ms, sanitize_display_name,
    validate_password,
};
use serde_json::json;

use crate::PaperCraftApp;
use crate::icons::{Icon, SwitchLayout, SwitchOption, paint_icon, segmented_switch};
use crate::modal::{danger_action_button, primary_action_button, secondary_action_button};
use crate::sync_worker::{
    SyncWorkerCommand, complete_forget_server_account, complete_sign_out,
    create_local_account_and_switch, finish_workspace_switch, prepare_workspace_switch,
    switch_to_local_account_clean, trigger_manual_sync,
};
use crate::theme::Tokens;

const INPUT_HEIGHT: f32 = 30.0;
const CARD_RADIUS: f32 = 10.0;
const CONTROL_RADIUS: f32 = 7.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthTab {
    #[default]
    SignIn,
    CreateAccount,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AddAccountMode {
    #[default]
    Server,
    Local,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProfileSection {
    #[default]
    Accounts,
    Profile,
    Sync,
    Appearance,
    System,
}

impl ProfileSection {
    pub(crate) const fn flow_order(self) -> isize {
        match self {
            Self::Accounts => 0,
            Self::Profile => 1,
            Self::Sync => 2,
            Self::Appearance => 3,
            Self::System => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingUnsyncedSwitchTarget {
    AuthForm,
    QuickLogin { server_url: String, user_id: String },
    PasswordReset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingUnsyncedSwitch {
    pub outgoing_label: String,
    pub pending_count: usize,
    pub target_label: String,
    pub target: PendingUnsyncedSwitchTarget,
}

#[derive(Clone, Debug)]
pub struct PasswordResetFlowState {
    pub active: bool,
    pub identifier: String,
    pub recovery_code: String,
    pub show_recovery_code: bool,
    pub new_password: String,
    pub confirm_password: String,
    pub show_new_password: bool,
    pub busy: bool,
    pub banner_message: Option<String>,
    pub code_preverified: bool,
}

impl Default for PasswordResetFlowState {
    fn default() -> Self {
        Self {
            active: false,
            identifier: String::new(),
            recovery_code: String::new(),
            show_recovery_code: true,
            new_password: String::new(),
            confirm_password: String::new(),
            show_new_password: false,
            busy: false,
            banner_message: None,
            code_preverified: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ServerProbeState {
    #[default]
    Untested,
    Checking {
        server_url: String,
    },
    Connected {
        server_url: String,
        version: String,
        protocol_version: u32,
        registration_open: bool,
        storage_engine: String,
        max_notebook_bytes: usize,
    },
    Failed {
        server_url: String,
        message: String,
    },
}

impl ServerProbeState {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    fn target_url(&self) -> Option<&str> {
        match self {
            Self::Untested => None,
            Self::Checking { server_url }
            | Self::Connected { server_url, .. }
            | Self::Failed { server_url, .. } => Some(server_url.as_str()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AccountDraftState {
    pub tab: AuthTab,
    pub add_account_mode: AddAccountMode,
    pub profile_section: ProfileSection,
    pub profile_probe_pending: bool,
    pub server_url: String,
    pub email_or_username: String,
    pub signup_username: String,
    pub display_name: String,
    pub password: String,
    pub show_password: bool,
    pub auth_busy: bool,
    pub password_reset: PasswordResetFlowState,
    pub probe: ServerProbeState,
    pub server_probes: HashMap<String, ServerProbeState>,
    pub quick_login_key: Option<(String, String)>,
    pub quick_login_password: String,
    pub quick_login_show_password: bool,
    pub quick_login_error: Option<String>,
    pub quick_login_focus_pending: bool,
    pub reauth_password: String,
    pub reauth_show_password: bool,
    pub reauth_error: Option<String>,
    pub reauth_pending: bool,
    pub disconnect_notice: Option<String>,
    pub select_local_new_name: String,
    pub select_local_error: Option<String>,
    pub select_local_focus_pending: bool,
    pub selector_other_server_open: bool,
    pub server_switch_form_open: bool,
    pub pending_unsynced_switch: Option<PendingUnsyncedSwitch>,
    pub local_profile_rename_draft: String,
    pub convert_source_profile_id: String,
    pub convert_source_nickname: String,
    pub convert_source_notebook_count: usize,
    pub convert_email: String,
    pub convert_password: String,
    pub convert_show_password: bool,
    pub convert_busy: bool,
    pub convert_progress_stage: String,
    pub convert_progress_completed: usize,
    pub convert_progress_total: usize,
    pub convert_error: Option<String>,
    pub convert_focus_pending: bool,
    pub profiles_panel_open: bool,
    pub new_profile_name: String,
    pub renaming_profile_id: Option<String>,
    pub rename_profile_draft: String,
    pub pending_delete_profile_id: Option<String>,
    pub delete_profile_confirm_input: String,
    pub pending_delete_profile_scroll_pending: bool,
    pub delete_profile_paste_blocked: bool,
    pub pending_forget_server_key: Option<(String, String)>,
    pub edit_display_name: String,
    pub edit_username: String,
    pub profile_save_busy: bool,
    pub change_email_open: bool,
    pub new_email_draft: String,
    pub change_email_password: String,
    pub show_change_email_password: bool,
    pub change_email_busy: bool,
    pub current_password_draft: String,
    pub new_password_draft: String,
    pub confirm_new_password_draft: String,
    pub show_change_pw_current: bool,
    pub show_change_pw_new: bool,
    pub change_password_busy: bool,
    pub status_message: Option<String>,
    pub error_message: Option<String>,
}

impl Default for AccountDraftState {
    fn default() -> Self {
        Self {
            tab: AuthTab::SignIn,
            add_account_mode: AddAccountMode::Server,
            profile_section: ProfileSection::Accounts,
            profile_probe_pending: false,
            server_url: DEFAULT_SERVER_URL.to_string(),
            email_or_username: String::new(),
            signup_username: String::new(),
            display_name: String::new(),
            password: String::new(),
            show_password: false,
            auth_busy: false,
            password_reset: PasswordResetFlowState::default(),
            probe: ServerProbeState::Untested,
            server_probes: HashMap::new(),
            quick_login_key: None,
            quick_login_password: String::new(),
            quick_login_show_password: false,
            quick_login_error: None,
            quick_login_focus_pending: false,
            reauth_password: String::new(),
            reauth_show_password: false,
            reauth_error: None,
            reauth_pending: false,
            disconnect_notice: None,
            select_local_new_name: String::new(),
            select_local_error: None,
            select_local_focus_pending: false,
            selector_other_server_open: false,
            server_switch_form_open: false,
            pending_unsynced_switch: None,
            local_profile_rename_draft: String::new(),
            convert_source_profile_id: String::new(),
            convert_source_nickname: String::new(),
            convert_source_notebook_count: 0,
            convert_email: String::new(),
            convert_password: String::new(),
            convert_show_password: false,
            convert_busy: false,
            convert_progress_stage: String::new(),
            convert_progress_completed: 0,
            convert_progress_total: 0,
            convert_error: None,
            convert_focus_pending: false,
            profiles_panel_open: false,
            new_profile_name: String::new(),
            renaming_profile_id: None,
            rename_profile_draft: String::new(),
            pending_delete_profile_id: None,
            delete_profile_confirm_input: String::new(),
            pending_delete_profile_scroll_pending: false,
            delete_profile_paste_blocked: false,
            pending_forget_server_key: None,
            edit_display_name: String::new(),
            edit_username: String::new(),
            profile_save_busy: false,
            change_email_open: false,
            new_email_draft: String::new(),
            change_email_password: String::new(),
            show_change_email_password: false,
            change_email_busy: false,
            current_password_draft: String::new(),
            new_password_draft: String::new(),
            confirm_new_password_draft: String::new(),
            show_change_pw_current: false,
            show_change_pw_new: false,
            change_password_busy: false,
            status_message: None,
            error_message: None,
        }
    }
}

pub(crate) fn norm_probe_url_key(server_url: &str) -> String {
    normalize_server_url(server_url)
        .unwrap_or_else(|_| server_url.trim().trim_end_matches('/').to_string())
}

impl AccountDraftState {
    /// Returns true while the active server URL probe is actively running.
    pub fn is_testing(&self) -> bool {
        matches!(self.probe, ServerProbeState::Checking { .. })
    }

    /// Returns the most recently observed server status for `server_url`.
    pub fn probe_for(&self, server_url: &str) -> ServerProbeState {
        let key = norm_probe_url_key(server_url);
        if let Some(found) = self.server_probes.get(&key) {
            return found.clone();
        }
        if urls_match(&self.server_url, server_url) {
            return self.probe.clone();
        }
        ServerProbeState::Untested
    }

    /// Refreshes `self.probe` when the draft URL changes to a different target server.
    pub fn invalidate_probe_if_url_changed(&mut self) {
        if let Some(probed) = self.probe.target_url()
            && !urls_match(&self.server_url, probed)
        {
            let key = norm_probe_url_key(&self.server_url);
            self.probe = self
                .server_probes
                .get(&key)
                .cloned()
                .unwrap_or(ServerProbeState::Untested);
        }
    }

    /// Returns true when the active server URL was verified and explicitly closed registration.
    pub fn is_registration_closed(&self) -> bool {
        matches!(
            &self.probe,
            ServerProbeState::Connected {
                server_url,
                registration_open: false,
                ..
            } if urls_match(&self.server_url, server_url)
        )
    }

    /// Records a successful server health check for any probed server URL.
    pub fn apply_health_ok(&mut self, server_url: String, info: ServerInfoResponse) {
        let norm = norm_probe_url_key(&server_url);
        let state = ServerProbeState::Connected {
            server_url: norm.clone(),
            version: info.version,
            protocol_version: info.protocol_version,
            registration_open: info.registration_open,
            storage_engine: info.storage_engine,
            max_notebook_bytes: info.max_notebook_bytes,
        };
        self.server_probes.insert(norm.clone(), state.clone());
        if urls_match(&self.server_url, &norm) {
            self.server_url = norm;
            self.probe = state;
            self.profile_probe_pending = false;
            self.error_message = None;
        }
    }

    /// Records a failed server health check for any probed server URL.
    pub fn apply_health_err(&mut self, server_url: String, message: String) {
        let norm = norm_probe_url_key(&server_url);
        let state = ServerProbeState::Failed {
            server_url: norm.clone(),
            message,
        };
        self.server_probes.insert(norm.clone(), state.clone());
        if urls_match(&self.server_url, &norm) {
            self.probe = state;
            self.profile_probe_pending = false;
        }
    }

    /// Marks `server_url` offline immediately after a dropped server connection.
    pub(crate) fn mark_server_offline(&mut self, server_url: &str, message: &str) {
        let norm = norm_probe_url_key(server_url);
        let state = ServerProbeState::Failed {
            server_url: norm.clone(),
            message: message.to_string(),
        };
        self.server_probes.insert(norm.clone(), state.clone());
        if urls_match(&self.server_url, &norm) {
            self.probe = state;
            self.profile_probe_pending = false;
        }
    }

    /// Clears a stale offline probe failure once a real sync cycle succeeds.
    pub(crate) fn clear_offline_after_sync(&mut self, server_url: &str) {
        let norm = norm_probe_url_key(server_url);
        if matches!(
            self.server_probes.get(&norm),
            Some(ServerProbeState::Failed { .. })
        ) {
            self.server_probes.remove(&norm);
        }
        if urls_match(&self.server_url, &norm)
            && matches!(self.probe, ServerProbeState::Failed { .. })
        {
            self.probe = ServerProbeState::Untested;
        }
    }

    /// Clears any pending Forget Account confirmation prompt.
    pub(crate) fn clear_pending_forget_server(&mut self) {
        self.pending_forget_server_key = None;
    }

    /// Clears any pending local profile deletion confirmation draft and focus/clipboard flags.
    pub fn clear_pending_delete_profile(&mut self) {
        self.pending_delete_profile_id = None;
        self.delete_profile_confirm_input.clear();
        self.pending_delete_profile_scroll_pending = false;
        self.delete_profile_paste_blocked = false;
    }

    pub(crate) fn clear_quick_login_draft(&mut self) {
        self.quick_login_key = None;
        self.quick_login_password.clear();
        self.quick_login_show_password = false;
        self.quick_login_error = None;
        self.quick_login_focus_pending = false;
    }

    pub fn clear_reauth_draft(&mut self) {
        self.reauth_password.clear();
        self.reauth_show_password = false;
        self.reauth_error = None;
        self.reauth_pending = false;
    }

    fn clear_convert_draft(&mut self) {
        self.convert_source_profile_id.clear();
        self.convert_source_nickname.clear();
        self.convert_source_notebook_count = 0;
        self.convert_email.clear();
        self.convert_password.clear();
        self.convert_show_password = false;
        self.convert_busy = false;
        self.convert_progress_stage.clear();
        self.convert_progress_completed = 0;
        self.convert_progress_total = 0;
        self.convert_error = None;
        self.convert_focus_pending = false;
    }

    pub fn clear_pending_unsynced_switch(&mut self) {
        self.pending_unsynced_switch = None;
    }

    pub fn clear_auth_form_credentials(&mut self) {
        self.email_or_username.clear();
        self.signup_username.clear();
        self.display_name.clear();
        self.password.clear();
        self.show_password = false;
        self.error_message = None;
        self.status_message = None;
        self.clear_pending_unsynced_switch();
        self.exit_password_reset_flow();
    }

    /// Applies successful login/registration feedback to the draft state.
    pub fn apply_auth_ok(&mut self, server_url: &str, label: &str) {
        let norm = norm_probe_url_key(server_url);
        if !self.probe_for(&norm).is_connected() {
            let connected = ServerProbeState::Connected {
                server_url: norm.clone(),
                version: String::new(),
                protocol_version: 1,
                registration_open: true,
                storage_engine: String::new(),
                max_notebook_bytes: 0,
            };
            self.server_probes.insert(norm.clone(), connected.clone());
            if urls_match(&self.server_url, &norm) {
                self.probe = connected;
            }
        }
        self.auth_busy = false;
        self.email_or_username.clear();
        self.display_name.clear();
        self.password.clear();
        self.show_password = false;
        self.signup_username.clear();
        self.password_reset = PasswordResetFlowState::default();
        self.clear_quick_login_draft();
        self.clear_reauth_draft();
        self.clear_convert_draft();
        self.clear_pending_delete_profile();
        self.clear_pending_forget_server();
        self.clear_pending_unsynced_switch();
        self.selector_other_server_open = false;
        self.server_switch_form_open = false;
        self.profile_save_busy = false;
        self.change_password_busy = false;
        self.current_password_draft.clear();
        self.new_password_draft.clear();
        self.confirm_new_password_draft.clear();
        self.error_message = None;
        self.server_url = norm;
        self.profile_section = ProfileSection::Accounts;
        self.status_message = Some(format!(
            "Signed in as {label}. Offline-First cloud sync active."
        ));
    }

    /// Applies successful local-to-server conversion feedback.
    pub fn apply_convert_local_ok(&mut self, server_url: &str, label: &str) {
        self.apply_auth_ok(server_url, label);
        self.status_message = Some(format!(
            "Local account synced to {label}! Offline-First copy retained on this device."
        ));
    }

    /// Updates real upload progress while converting a local account to a cloud account.
    pub fn apply_convert_local_progress(&mut self, stage: String, completed: usize, total: usize) {
        self.convert_progress_stage = stage;
        self.convert_progress_completed = completed;
        self.convert_progress_total = total;
    }

    /// Applies a conversion error without dismissing the conversion modal.
    pub fn apply_convert_local_err(&mut self, message: String) {
        self.convert_busy = false;
        self.convert_progress_stage.clear();
        self.convert_progress_completed = 0;
        self.convert_progress_total = 0;
        self.convert_error = Some(message);
    }

    /// Applies an authentication error message to the draft state.
    pub fn apply_auth_err(&mut self, message: String) {
        self.auth_busy = false;
        self.password_reset.busy = false;
        self.status_message = None;
        if self.reauth_pending {
            self.reauth_pending = false;
            self.reauth_error = Some(message.clone());
        }
        if self.quick_login_key.is_some() {
            self.quick_login_error = Some(message.clone());
        }
        self.error_message = Some(message);
    }

    /// Transitions into the forced recovery-code password reset flow.
    pub fn enter_password_reset_flow(
        &mut self,
        identifier: String,
        prefill_code: Option<String>,
        message: String,
    ) {
        self.auth_busy = false;
        self.password.clear();
        self.clear_quick_login_draft();
        self.clear_reauth_draft();
        self.error_message = None;
        self.status_message = None;
        self.add_account_mode = AddAccountMode::Server;
        self.server_switch_form_open = true;
        self.clear_pending_unsynced_switch();
        self.password_reset.active = true;
        self.password_reset.identifier = identifier;
        self.password_reset.code_preverified = prefill_code.is_some();
        self.password_reset.recovery_code = prefill_code.unwrap_or_default();
        self.password_reset.show_recovery_code = true;
        self.password_reset.new_password.clear();
        self.password_reset.confirm_password.clear();
        self.password_reset.busy = false;
        self.password_reset.banner_message = Some(message);
    }

    /// Exits the recovery-code password reset flow back to normal sign-in.
    pub fn exit_password_reset_flow(&mut self) {
        self.password_reset = PasswordResetFlowState::default();
        self.error_message = None;
    }

    /// Syncs editable identity fields from an active cloud session.
    pub fn sync_identity_from_session(
        &mut self,
        session: Option<&papercraft_sync::ActiveAccountSession>,
    ) {
        if let Some(sess) = session {
            self.edit_display_name = sess.user.display_name.clone();
            self.edit_username = sess.user.username.clone();
        } else {
            self.edit_display_name.clear();
            self.edit_username.clear();
        }
    }

    /// Applies successful profile update confirmation.
    pub fn apply_profile_updated(&mut self, username: &str, display_name: &str) {
        self.profile_save_busy = false;
        self.edit_username = username.to_string();
        self.edit_display_name = display_name.to_string();
        self.error_message = None;
        self.status_message = Some(format!("Profile updated (@{username})."));
        if self.profile_section == ProfileSection::Profile {
            self.profile_section = ProfileSection::Accounts;
        }
    }

    /// Applies a profile update error message.
    pub fn apply_profile_update_err(&mut self, message: String) {
        self.profile_save_busy = false;
        self.status_message = None;
        self.error_message = Some(message);
    }

    /// Applies successful voluntary password change confirmation.
    pub fn apply_password_changed(&mut self) {
        self.change_password_busy = false;
        self.current_password_draft.clear();
        self.new_password_draft.clear();
        self.confirm_new_password_draft.clear();
        self.error_message = None;
        self.status_message = Some("Account password updated.".to_string());
        if self.profile_section == ProfileSection::Profile {
            self.profile_section = ProfileSection::Accounts;
        }
    }

    /// Applies a password change error message.
    pub fn apply_password_change_err(&mut self, message: String) {
        self.change_password_busy = false;
        self.status_message = None;
        self.error_message = Some(message);
    }

    /// Applies a successful email change confirmation.
    pub fn apply_email_changed(&mut self, new_email: Option<&str>) {
        self.change_email_busy = false;
        self.change_email_open = false;
        self.new_email_draft.clear();
        self.change_email_password.clear();
        self.error_message = None;
        let label = new_email.unwrap_or("updated email");
        self.status_message = Some(format!("Account email updated to {label}."));
        if self.profile_section == ProfileSection::Profile {
            self.profile_section = ProfileSection::Accounts;
        }
    }

    /// Applies an email change error message.
    pub fn apply_email_change_err(&mut self, message: String) {
        self.change_email_busy = false;
        self.status_message = None;
        self.error_message = Some(message);
    }

    /// Clears transient post-login sync banners once the initial sync settles.
    pub fn clear_transient_auth_status(&mut self) {
        if self.status_message.as_deref().is_some_and(|msg| {
            msg.starts_with("Signed in as ") || msg.starts_with("Local account synced to ")
        }) {
            self.status_message = None;
        }
    }

    /// Resets transient authentication/confirmation flags after exiting a server session.
    pub fn reset_after_sign_out(&mut self, server_url: &str) {
        let norm = norm_probe_url_key(server_url);
        self.server_url = norm.clone();
        self.probe = self.probe_for(&norm);
        self.auth_busy = false;
        self.clear_quick_login_draft();
        self.clear_reauth_draft();
        self.clear_convert_draft();
        self.clear_pending_delete_profile();
        self.clear_pending_forget_server();
        self.clear_pending_unsynced_switch();
        self.selector_other_server_open = false;
        self.server_switch_form_open = false;
        self.password_reset = PasswordResetFlowState::default();
        self.email_or_username.clear();
        self.display_name.clear();
        self.password.clear();
        self.show_password = false;
        self.signup_username.clear();
        self.edit_display_name.clear();
        self.edit_username.clear();
        self.profile_save_busy = false;
        self.change_email_open = false;
        self.change_email_password.clear();
        self.new_email_draft.clear();
        self.change_password_busy = false;
        self.current_password_draft.clear();
        self.new_password_draft.clear();
        self.confirm_new_password_draft.clear();
        self.error_message = None;
        self.status_message = Some("Signed out. Choose an account to continue.".to_string());
    }
}

pub(crate) fn sync_draft_from_session(app: &mut PaperCraftApp) {
    app.view.account.server_url = app.session.preferred_server_url().to_string();
    app.view
        .account
        .sync_identity_from_session(app.session.active_account_session());
    app.view.account.local_profile_rename_draft = app
        .session
        .active_local_profile()
        .map(|p| p.nickname.clone())
        .unwrap_or_default();
    app.view.account.error_message = None;
    app.view.account.status_message = None;
    app.view.account.select_local_error = None;
    app.view.account.selector_other_server_open = false;
    app.view.account.server_switch_form_open = false;
    app.view.account.clear_pending_unsynced_switch();
    app.view.account.clear_quick_login_draft();
    app.view.account.clear_reauth_draft();
    app.view.account.clear_pending_delete_profile();
    app.view.account.clear_pending_forget_server();
    app.view.account.profile_save_busy = false;
    app.view.account.change_email_open = false;
    app.view.account.change_email_password.clear();
    app.view.account.change_password_busy = false;
    app.view.account.current_password_draft.clear();
    app.view.account.new_password_draft.clear();
    app.view.account.confirm_new_password_draft.clear();
    app.view.account.invalidate_probe_if_url_changed();
}

/// Probes configured or previously remembered server URLs in the background.
pub(crate) fn probe_all_known_servers(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let mut targets = BTreeSet::new();
    if app.session.is_server_configured() {
        targets.insert(norm_probe_url_key(app.session.server_url()));
        targets.insert(norm_probe_url_key(app.session.preferred_server_url()));
        if !app.view.account.server_url.trim().is_empty() {
            targets.insert(norm_probe_url_key(&app.view.account.server_url));
        }
    }
    for entry in app.session.remembered_server_accounts() {
        targets.insert(norm_probe_url_key(&entry.server_url));
    }
    for raw_url in targets {
        match normalize_server_url(&raw_url) {
            Ok(norm) => {
                let checking = ServerProbeState::Checking {
                    server_url: norm.clone(),
                };
                app.view
                    .account
                    .server_probes
                    .insert(norm.clone(), checking.clone());
                if urls_match(&app.view.account.server_url, &norm) {
                    app.view.account.probe = checking;
                }
                if !crate::sync_worker::dispatch_command(
                    app,
                    Some(ctx),
                    SyncWorkerCommand::TestConnection {
                        server_url: norm.clone(),
                    },
                ) {
                    app.view
                        .account
                        .mark_server_offline(&norm, "Background network worker is unavailable.");
                }
            }
            Err(err) => {
                app.view
                    .account
                    .mark_server_offline(&raw_url, &err.to_string());
            }
        }
    }
}

/// Renders an inline setup reminder when no sync server URL has been configured yet.
/// Returns `true` when the user clicked `Open Sync Settings` and the caller should close its modal.
pub(crate) fn render_unconfigured_server_notice(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    allow_open_sync: bool,
) -> bool {
    if app.session.is_server_configured() {
        return false;
    }
    let mut opened_sync = false;
    egui::Frame::new()
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::AlertCircle, t.warning_fg);
                ui.label(
                    egui::RichText::new("Sync Server Not Configured")
                        .size(12.5)
                        .strong()
                        .color(t.text),
                );
            });
            ui.add_space(4.0);
            let body_msg = if allow_open_sync {
                "Configure your PaperCraft Server URL in Profile → Sync before signing in or creating a server account."
            } else {
                "Open a local account first, then configure your PaperCraft Server URL in Profile → Sync."
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(body_msg)
                        .size(11.0)
                        .color(t.text_muted),
                )
                .wrap(),
            );
            if allow_open_sync {
                ui.add_space(8.0);
                if secondary_action_button(ui, "Open Sync Settings", true, t).clicked() {
                    crate::open_profile_sync_drawer(app);
                    opened_sync = true;
                }
            }
        });
    opened_sync
}

pub(crate) fn render_add_account_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    if std::mem::take(&mut app.view.account.profile_probe_pending) {
        probe_all_known_servers(app, ui.ctx());
    }

    let mut should_close = false;
    ui.spacing_mut().item_spacing.y = 0.0;

    let mode_options = [
        SwitchOption::new(
            Some(Icon::Server),
            Some("Server Account"),
            "Sign in or create an account on a PaperCraft Server",
            true,
        ),
        SwitchOption::new(
            Some(Icon::Account),
            Some("Local Account"),
            "Create an offline account stored only on this device",
            true,
        ),
    ];
    let active_idx = match app.view.account.add_account_mode {
        AddAccountMode::Server => 0,
        AddAccountMode::Local => 1,
    };
    let (clicked_idx, _) = segmented_switch(
        ui,
        "modal_add_account_mode_switch",
        &mode_options,
        Some(active_idx),
        SwitchLayout::EqualWidth,
        t,
    );
    if let Some(idx) = clicked_idx {
        let next = if idx == 0 {
            AddAccountMode::Server
        } else {
            AddAccountMode::Local
        };
        if app.view.account.add_account_mode != next {
            app.view.account.add_account_mode = next;
            app.view.account.error_message = None;
            app.view.account.status_message = None;
            app.view.account.select_local_error = None;
            if next == AddAccountMode::Local {
                app.view.account.select_local_focus_pending = true;
            } else {
                app.view.account.profile_probe_pending = true;
            }
        }
    }
    ui.add_space(10.0);

    let subflow_id: u8 = match app.view.account.add_account_mode {
        AddAccountMode::Server => 0,
        AddAccountMode::Local => 1,
    };
    crate::modal::apply_subflow_motion(
        app,
        ui,
        crate::modal::ActiveModal::AddAccount,
        subflow_id,
        isize::from(subflow_id),
        |app, ui| match app.view.account.add_account_mode {
            AddAccountMode::Server => {
                if render_unconfigured_server_notice(app, ui, t, true) {
                    should_close = true;
                    return;
                }
                if !app.session.is_server_configured() {
                    ui.add_space(9.0);
                }
                let mut submit_on_enter = false;
                draw_section_card(ui, t, |ui| {
                    submit_on_enter = render_auth_card_contents(app, ui, t, false);
                });
                render_unsynced_switch_confirmation(app, ui, t);
                render_auth_banners(app, ui, t);
                if submit_on_enter {
                    handle_auth_submit_on_enter(app, ui.ctx());
                }
            }
            AddAccountMode::Local => {
                let mut submitted = false;
                let (acct_icon, icon_fg, _, _) = account_icon_and_colors(false, false, t);
                draw_section_card(ui, t, |ui| {
                    ui.horizontal(|ui| {
                        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        paint_icon(ui.painter(), ir, acct_icon, icon_fg);
                        ui.label(
                            egui::RichText::new("New Local Account")
                                .size(13.0)
                                .strong()
                                .color(t.text),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            draw_account_type_badge(ui, false, false, t);
                        });
                    });
                    ui.add_space(3.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                "Stored only on this device. Switch anytime without a password.",
                            )
                            .size(11.0)
                            .color(t.text_muted),
                        )
                        .wrap(),
                    );
                    ui.add_space(8.0);
                    draw_field_label_row(
                        ui,
                        "Nickname",
                        FieldHint::Muted("Used to identify this account"),
                        t,
                    );
                    ui.add_space(4.0);
                    let resp = draw_text_input(
                        ui,
                        InputFieldSpec {
                            id_salt: "add_account_local_nickname_input",
                            width: ui.available_width(),
                            hint: "e.g. Work or Study",
                            border_color: None,
                            trailing: InputTrailing::None,
                        },
                        &mut app.view.account.new_profile_name,
                        t,
                    );
                    if std::mem::take(&mut app.view.account.select_local_focus_pending) {
                        ui.memory_mut(|m| {
                            m.request_focus(
                                ui.make_persistent_id("add_account_local_nickname_input"),
                            )
                        });
                    }
                    if resp.changed {
                        app.view.account.select_local_error = None;
                        app.view.account.error_message = None;
                    }
                    if resp.enter_pressed {
                        submitted = true;
                    }
                });

                if let Some(ref err) = app.view.account.select_local_error {
                    ui.add_space(8.0);
                    draw_status_banner(ui, BannerTone::Error, err, t);
                } else {
                    render_auth_banners(app, ui, t);
                }

                if submitted && submit_add_local_account(app, ui.ctx()) {
                    should_close = true;
                }
            }
        },
    );

    should_close
}

pub(crate) fn render_add_account_footer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    match app.view.account.add_account_mode {
        AddAccountMode::Server => render_server_auth_footer(app, ui, t),
        AddAccountMode::Local => {
            let mut should_close = false;
            ui.horizontal(|ui| {
                if secondary_action_button(ui, "Cancel", true, t).clicked() {
                    app.view.account.select_local_error = None;
                    should_close = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let valid = !app.view.account.new_profile_name.trim().is_empty();
                    if primary_action_button(ui, "Create Local Account", valid, t).clicked()
                        && submit_add_local_account(app, ui.ctx())
                    {
                        should_close = true;
                    }
                });
            });
            should_close
        }
    }
}

fn submit_add_local_account(app: &mut PaperCraftApp, ctx: &egui::Context) -> bool {
    let nick = app.view.account.new_profile_name.trim().to_string();
    if nick.is_empty() {
        app.view.account.select_local_error =
            Some("Please enter a nickname for the local account.".to_string());
        return false;
    }
    let res = if !app.session.is_signed_in() {
        create_local_account_and_switch(app, ctx, &nick)
            .map(|()| format!("Switched to local account '{nick}'."))
    } else {
        crate::canvas::commit_pending_text(app);
        app.session
            .execute(
                "profile.create",
                &json!({ "nickname": nick, "switch_now": false }),
            )
            .map(|_| {
                app.flush_web_storage(true);
                format!("Created local account '{nick}'.")
            })
            .map_err(|e| e.to_string())
    };
    match res {
        Ok(status) => {
            app.view.account.new_profile_name.clear();
            app.view.account.select_local_error = None;
            app.view.account.error_message = None;
            app.view.account.status_message = Some(status);
            true
        }
        Err(err) => {
            app.view.account.select_local_error = Some(err);
            false
        }
    }
}

pub(crate) fn render_body(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    if std::mem::take(&mut app.view.account.profile_probe_pending) {
        probe_all_known_servers(app, ui.ctx());
    }
    if app.session.is_signed_in() {
        let mut should_close = false;
        crate::modal::apply_subflow_motion(
            app,
            ui,
            crate::modal::ActiveModal::AccountSync,
            3,
            3,
            |app, ui| {
                should_close = render_signed_in_body(app, ui, t);
            },
        );
        should_close
    } else {
        render_signed_out_body(app, ui, t)
    }
}

pub(crate) fn render_footer(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    if app.session.is_signed_in() {
        render_signed_in_footer(app, ui, t)
    } else {
        render_signed_out_footer(app, ui, t)
    }
}

pub(crate) fn render_first_access_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let mut submitted = false;
    ui.spacing_mut().item_spacing.y = 0.0;

    if let Some(ref notice) = app.view.account.disconnect_notice {
        draw_status_banner(ui, BannerTone::Error, notice, t);
        ui.add_space(10.0);
    }

    draw_section_card(ui, t, |ui| {
        draw_field_label_row(
            ui,
            "Nickname",
            FieldHint::Muted("Used only to identify this local workspace"),
            t,
        );
        ui.add_space(6.0);
        let resp = draw_text_input(
            ui,
            InputFieldSpec {
                id_salt: "first_access_nickname_input",
                width: ui.available_width(),
                hint: "e.g. Jordan or Alyx",
                border_color: None,
                trailing: InputTrailing::None,
            },
            &mut app.view.account.select_local_new_name,
            t,
        );
        if std::mem::take(&mut app.view.account.select_local_focus_pending) {
            ui.memory_mut(|m| {
                m.request_focus(ui.make_persistent_id("first_access_nickname_input"))
            });
        }
        if resp.changed {
            app.view.account.select_local_error = None;
        }
        if resp.enter_pressed {
            submitted = true;
        }
    });

    if let Some(ref err) = app.view.account.select_local_error {
        ui.add_space(8.0);
        draw_status_banner(ui, BannerTone::Error, err, t);
    }

    if submitted {
        return submit_first_access_nickname(app, ui.ctx());
    }
    false
}

pub(crate) fn render_first_access_footer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let mut should_close = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Required to begin a local session")
                .size(11.0)
                .color(t.text_muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let valid = !app.view.account.select_local_new_name.trim().is_empty();
            if primary_action_button(ui, "Start Session", valid, t).clicked() {
                should_close = submit_first_access_nickname(app, ui.ctx());
            }
        });
    });
    should_close
}

fn submit_first_access_nickname(app: &mut PaperCraftApp, ctx: &egui::Context) -> bool {
    let nick = app.view.account.select_local_new_name.trim().to_string();
    match create_local_account_and_switch(app, ctx, &nick) {
        Ok(()) => {
            app.view.account.select_local_new_name.clear();
            app.view.account.select_local_error = None;
            true
        }
        Err(err) => {
            app.view.account.select_local_error = Some(err);
            false
        }
    }
}

pub(crate) fn render_startup_picker_switch(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    show_settings_hint: bool,
) {
    if !app.session.is_sync_persistence_enabled() {
        return;
    }
    let mut skip = app.session.skip_account_picker_on_startup();
    let changed = crate::icons::toggle_switch_row(
        ui,
        ("startup_picker_switch", show_settings_hint),
        "Remember last account and skip on startup",
        "Automatically open your most recent account when PaperCraft launches.",
        &mut skip,
        t,
    )
    .changed();
    if changed {
        let _ = app
            .session
            .execute("profile.setStartupPicker", &json!({ "skip": skip }));
        app.flush_web_storage(true);
    }
    ui.add_space(4.0);
    let hint = if show_settings_hint {
        "Open your most recent account immediately on launch."
    } else {
        "Change anytime in Profile → Accounts."
    };
    ui.add(egui::Label::new(egui::RichText::new(hint).size(10.5).color(t.text_muted)).wrap());
}

/// Returns `(icon, fg, soft_bg, border)` for local vs online/offline server infrastructure.
pub(crate) fn account_icon_and_colors(
    is_server: bool,
    server_online: bool,
    t: &Tokens,
) -> (Icon, Color32, Color32, Color32) {
    if !is_server {
        (
            Icon::Account,
            t.accent,
            t.accent_soft,
            t.accent.gamma_multiply(0.45),
        )
    } else if server_online {
        (Icon::Server, t.success_fg, t.success_bg, t.success_border)
    } else {
        let disabled_green =
            crate::motion::lerp_color(t.text_muted, t.success_fg, 0.22).gamma_multiply(0.68);
        (Icon::Server, disabled_green, t.toolbar_bg, t.border)
    }
}

/// Returns `(Icon::Account, fg, soft_bg, border)` for account avatars and account rows.
pub(crate) fn account_avatar_icon_and_colors(
    is_server: bool,
    server_online: bool,
    t: &Tokens,
) -> (Icon, Color32, Color32, Color32) {
    let (_, fg, fill, border) = account_icon_and_colors(is_server, server_online, t);
    (Icon::Account, fg, fill, border)
}

pub(crate) fn is_server_url_online(app: &PaperCraftApp, server_url: &str) -> bool {
    !matches!(
        app.view.account.probe_for(server_url),
        ServerProbeState::Failed { .. }
    )
}

pub(crate) fn draw_account_type_badge(
    ui: &mut egui::Ui,
    connected_to_server: bool,
    server_online: bool,
    t: &Tokens,
) {
    let (icon, fg, fill, border) = account_icon_and_colors(connected_to_server, server_online, t);
    let label = if connected_to_server {
        "Server"
    } else {
        "Local"
    };
    draw_pill_badge(
        ui,
        PillStyle {
            fill,
            border,
            fg,
            icon: PillIcon::Vector(icon),
        },
        label,
    );
}

pub(crate) fn render_select_local_account_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    if std::mem::take(&mut app.view.account.profile_probe_pending) {
        probe_all_known_servers(app, ui.ctx());
    }

    let mut should_close = false;
    ui.spacing_mut().item_spacing.y = 0.0;
    let subflow_id: u8 = u8::from(app.view.account.selector_other_server_open);

    crate::modal::apply_subflow_motion(
        app,
        ui,
        crate::modal::ActiveModal::SelectLocalAccount,
        subflow_id,
        isize::from(subflow_id),
        |app, ui| {
            if app.view.account.selector_other_server_open {
                let can_open_sync = crate::modal::is_account_picker_dismissible(app);
                if render_unconfigured_server_notice(app, ui, t, can_open_sync) {
                    should_close = true;
                    return;
                }
                if !app.session.is_server_configured() {
                    ui.add_space(9.0);
                }
                let mut submit_on_enter = false;
                draw_section_card(ui, t, |ui| {
                    submit_on_enter = render_auth_card_contents(app, ui, t, true);
                });
                render_unsynced_switch_confirmation(app, ui, t);
                render_auth_banners(app, ui, t);
                if submit_on_enter {
                    handle_auth_submit_on_enter(app, ui.ctx());
                }
                return;
            }

            if let Some(ref notice) = app.view.account.disconnect_notice {
                draw_status_banner(ui, BannerTone::Error, notice, t);
                ui.add_space(10.0);
            }

            let profiles = app.session.saved_local_profiles().to_vec();
            let remembered = app.session.remembered_server_accounts().to_vec();
            let any_checking = app
                .view
                .account
                .server_probes
                .values()
                .any(|p| matches!(p, ServerProbeState::Checking { .. }));
            let viewing_cloud = app.session.is_viewing_cloud();
            let active_local_id = app
                .session
                .active_local_profile()
                .map(|p| p.local_profile_id.clone());
            let active_cloud_key = app
                .session
                .active_account_session()
                .map(|s| (norm_probe_url_key(&s.server_url), s.user.user_id.clone()));

            let mut split_state = (app, &mut should_close);
            draw_local_vs_server_split(
                ui,
                &mut split_state,
                640.0,
                20.0,
                t,
                |(app, should_close), ui| {
                    render_selector_local_column(
                        app,
                        ui,
                        &profiles,
                        remembered.is_empty(),
                        viewing_cloud,
                        active_local_id.as_deref(),
                        should_close,
                        t,
                    );
                },
                |(app, should_close), ui| {
                    render_selector_server_column(
                        app,
                        ui,
                        &remembered,
                        any_checking,
                        viewing_cloud,
                        active_cloud_key.as_ref(),
                        should_close,
                        t,
                    );
                },
            );
            let app = split_state.0;

            render_unsynced_switch_confirmation(app, ui, t);

            if app.session.is_sync_persistence_enabled() {
                ui.add_space(10.0);
                draw_section_card(ui, t, |ui| {
                    render_startup_picker_switch(app, ui, t, false);
                });
            }

            if let Some(ref err) = app.view.account.select_local_error {
                ui.add_space(8.0);
                draw_status_banner(ui, BannerTone::Error, err, t);
            } else if app.view.account.quick_login_error.is_none() {
                render_auth_banners(app, ui, t);
            }
        },
    );

    should_close
}

#[allow(clippy::too_many_arguments)]
fn render_selector_local_column(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    profiles: &[papercraft_sync::LocalProfile],
    no_server_accounts: bool,
    viewing_cloud: bool,
    active_local_id: Option<&str>,
    should_close: &mut bool,
    t: &Tokens,
) {
    draw_account_column_card(ui, AccountColumnTone::Local, t, |ui| {
        ui.horizontal(|ui| {
            draw_column_header_badge(ui, AccountColumnTone::Local, Icon::Account, t);
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("Local Accounts ({})", profiles.len()))
                    .size(13.0)
                    .strong()
                    .color(t.text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                draw_account_type_badge(ui, false, false, t);
            });
        });
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new("Stored on this device.")
                    .size(11.0)
                    .color(t.text_muted),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        draw_hairline(ui, t);
        ui.add_space(8.0);

        if profiles.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("No local accounts on this device yet.")
                        .size(11.5)
                        .color(t.text_muted),
                )
                .wrap(),
            );
            ui.add_space(8.0);
        } else {
            let mut open_id: Option<String> = None;
            for prof in profiles {
                let is_active =
                    !viewing_cloud && active_local_id == Some(prof.local_profile_id.as_str());
                if render_selector_local_account_row(app, ui, prof, is_active, t) {
                    if is_active {
                        *should_close = true;
                    } else {
                        open_id = Some(prof.local_profile_id.clone());
                    }
                }
                ui.add_space(6.0);
            }
            if let Some(pid) = open_id {
                if let Err(err) = switch_to_local_account_clean(app, ui.ctx(), &pid) {
                    app.view.account.select_local_error = Some(err);
                } else {
                    *should_close = true;
                }
            }
        }

        draw_hairline(ui, t);
        ui.add_space(8.0);
        if render_selector_create_local_inline(
            app,
            ui,
            profiles.is_empty() && no_server_accounts,
            t,
        ) {
            *should_close = true;
        }
    });
}

fn render_selector_local_account_row(
    app: &PaperCraftApp,
    ui: &mut egui::Ui,
    prof: &papercraft_sync::LocalProfile,
    is_active: bool,
    t: &Tokens,
) -> bool {
    let mut clicked = false;
    let nb_count = app
        .session
        .local_profile_notebook_count(&prof.local_profile_id);
    let count_label = format_notebook_count(nb_count);
    draw_account_block_frame(
        ui,
        ("select_local_prof", &prof.local_profile_id),
        is_active,
        false,
        t,
        |ui| {
            let (acct_icon, icon_fg, _, _) = account_icon_and_colors(false, false, t);
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(ui.painter(), ir, acct_icon, icon_fg);
                draw_truncated_block_title(
                    ui,
                    &prof.nickname,
                    (ui.available_width() - 108.0).max(52.0),
                    t,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let cta = if is_active { "Continue" } else { "Open" };
                    if primary_action_button(ui, cta, true, t).clicked() {
                        clicked = true;
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if is_active {
                    draw_active_pill(ui, t);
                }
                draw_account_type_badge(ui, false, false, t);
                ui.label(
                    egui::RichText::new(count_label)
                        .size(11.0)
                        .color(t.text_muted),
                )
                .on_hover_text("Offline on this device");
            });
        },
    );
    clicked
}

fn render_selector_create_local_inline(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    is_first_launch: bool,
    t: &Tokens,
) -> bool {
    let mut should_close = false;
    let heading = if is_first_launch {
        "Start a Local Account"
    } else {
        "New Local Account"
    };
    let cta_label = if is_first_launch {
        "Start Local Session"
    } else {
        "Create & Open"
    };
    draw_field_label_row(ui, heading, FieldHint::Muted("Offline only"), t);
    ui.add_space(6.0);
    let btn_w = if is_first_launch {
        136.0_f32
    } else {
        112.0_f32
    };
    let gap = 8.0_f32;
    let input_w = (ui.available_width() - btn_w - gap).max(110.0);
    let mut create_submitted = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let resp = draw_text_input(
            ui,
            InputFieldSpec {
                id_salt: "select_local_new_nickname_input",
                width: input_w,
                hint: "Local nickname...",
                border_color: None,
                trailing: InputTrailing::None,
            },
            &mut app.view.account.select_local_new_name,
            t,
        );
        if std::mem::take(&mut app.view.account.select_local_focus_pending) {
            ui.memory_mut(|m| {
                m.request_focus(ui.make_persistent_id("select_local_new_nickname_input"));
            });
        }
        if resp.changed {
            app.view.account.select_local_error = None;
        }
        if resp.enter_pressed {
            create_submitted = true;
        }
        let has_name = !app.view.account.select_local_new_name.trim().is_empty();
        if primary_action_button(ui, cta_label, has_name, t).clicked()
            || (create_submitted && has_name)
        {
            let nick = app.view.account.select_local_new_name.trim().to_string();
            match create_local_account_and_switch(app, ui.ctx(), &nick) {
                Ok(()) => {
                    app.view.account.select_local_new_name.clear();
                    app.view.account.select_local_error = None;
                    should_close = true;
                }
                Err(err) => {
                    app.view.account.select_local_error = Some(err);
                }
            }
        }
    });
    should_close
}

#[allow(clippy::too_many_arguments)]
fn render_selector_server_column(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    remembered: &[RememberedServerAccount],
    any_checking: bool,
    viewing_cloud: bool,
    active_cloud_key: Option<&(String, String)>,
    should_close: &mut bool,
    t: &Tokens,
) {
    let default_url = app.session.preferred_server_url().to_string();
    let default_online = is_server_url_online(app, &default_url);
    let col_tone = if default_online {
        AccountColumnTone::Server
    } else {
        AccountColumnTone::ServerOffline
    };
    draw_account_column_card(ui, col_tone, t, |ui| {
        ui.horizontal(|ui| {
            draw_column_header_badge(ui, col_tone, Icon::Server, t);
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("Server Accounts ({})", remembered.len()))
                    .size(13.0)
                    .strong()
                    .color(t.text),
            );
            if !remembered.is_empty() || app.session.is_server_configured() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let check_lbl = if any_checking { "..." } else { "Check" };
                    if fixed_secondary_button(ui, check_lbl, 56.0, !any_checking, t)
                        .on_hover_text("Check whether your configured sync servers are reachable")
                        .clicked()
                    {
                        probe_all_known_servers(app, ui.ctx());
                    }
                });
            }
        });
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new("Synced via PaperCraft Server.")
                    .size(11.0)
                    .color(t.text_muted),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        draw_hairline(ui, t);
        ui.add_space(8.0);

        if remembered.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("No server accounts saved on this device yet.")
                        .size(11.5)
                        .color(t.text_muted),
                )
                .wrap(),
            );
            ui.add_space(8.0);
        } else {
            for entry in remembered {
                let entry_key = (norm_probe_url_key(&entry.server_url), entry.user_id.clone());
                let is_active = viewing_cloud && active_cloud_key == Some(&entry_key);
                if render_selector_server_account_block(app, ui, entry, is_active, t) {
                    *should_close = true;
                }
                ui.add_space(6.0);
            }
        }

        draw_hairline(ui, t);
        ui.add_space(8.0);
        let btn_w = ui.available_width();
        if fixed_secondary_button(ui, "Sign In / Register", btn_w, true, t)
            .on_hover_text("Sign in or create an account on your configured sync server")
            .clicked()
        {
            app.view.account.server_url = default_url.clone();
            app.view.account.invalidate_probe_if_url_changed();
            app.view.account.selector_other_server_open = true;
            app.view.account.tab = AuthTab::SignIn;
            app.view.account.clear_auth_form_credentials();
            app.view.account.clear_quick_login_draft();
            app.view.account.profile_probe_pending = true;
        }
    });
}

fn render_selector_server_account_block(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    entry: &RememberedServerAccount,
    is_active: bool,
    t: &Tokens,
) -> bool {
    let mut should_close = false;
    let norm_url = norm_probe_url_key(&entry.server_url);
    let probe = app.view.account.probe_for(&entry.server_url);
    let server_offline = matches!(probe, ServerProbeState::Failed { .. });
    let cached_notebooks = app
        .session
        .server_account_notebook_count(&entry.server_url, &entry.user_id);
    let is_cached = cached_notebooks > 0
        || app
            .session
            .has_cached_server_account(&entry.server_url, &entry.user_id);
    let pending = if is_active {
        app.session.cloud_pending_changes_count()
    } else {
        app.session
            .server_account_pending_changes_count(&entry.server_url, &entry.user_id)
    };
    let entry_key = (norm_url, entry.user_id.clone());
    let quick_open = !is_active && app.view.account.quick_login_key.as_ref() == Some(&entry_key);
    let primary = entry.primary_label();
    let subtitle_id = entry
        .login_email()
        .map(str::to_string)
        .unwrap_or_else(|| format!("@{}", entry.username));

    draw_account_block_frame(
        ui,
        ("selector_srv_block", &entry.server_url, &entry.user_id),
        is_active || quick_open,
        false,
        t,
        |ui| {
            let (acct_icon, icon_fg, _, _) =
                account_avatar_icon_and_colors(true, !server_offline, t);
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(ui.painter(), ir, acct_icon, icon_fg);
                draw_truncated_block_title(
                    ui,
                    &primary,
                    (ui.available_width() - 108.0).max(48.0),
                    t,
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_active {
                        if primary_action_button(ui, "Continue", true, t).clicked() {
                            should_close = true;
                        }
                    } else {
                        let can_open_login = !app.view.account.auth_busy;
                        let tip = if server_offline {
                            "Last check failed — click Sign In to retry authenticating"
                        } else {
                            "Enter your password to verify your identity and switch"
                        };
                        if primary_action_button(ui, "Sign In", can_open_login, t)
                            .on_hover_text(tip)
                            .clicked()
                        {
                            if quick_open {
                                app.view.account.clear_quick_login_draft();
                            } else {
                                app.view.account.quick_login_key = Some(entry_key.clone());
                                app.view.account.quick_login_password.clear();
                                app.view.account.quick_login_error = None;
                                app.view.account.quick_login_focus_pending = true;
                                app.view.account.server_url = entry.server_url.clone();
                                app.view.account.clear_pending_unsynced_switch();
                            }
                        }
                    }
                });
            });

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                draw_account_type_badge(ui, true, !server_offline, t);
                if is_active {
                    draw_active_pill(ui, t);
                } else {
                    draw_probe_status_pill(ui, &probe, t);
                }
            });

            let cache_note = if is_cached {
                format!(" · {}", format_notebook_count(cached_notebooks))
            } else {
                String::new()
            };
            let pending_note = if pending > 0 {
                format!(" · {pending} unsynced")
            } else {
                String::new()
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{subtitle_id} · {}{cache_note}{pending_note}",
                        entry.server_url
                    ))
                    .size(11.0)
                    .color(t.text_muted),
                )
                .wrap(),
            );
            if !is_active && server_offline {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Last check could not reach server — click Sign In to retry or Check above.",
                        )
                        .size(10.5)
                        .color(t.warning_fg),
                    )
                    .wrap(),
                );
            }

            let drawer_id = ui.make_persistent_id((
                "selector_quick_login_collapse",
                &entry.server_url,
                &entry.user_id,
            ));
            crate::motion::animated_collapse_with_duration(
                ui,
                drawer_id,
                quick_open,
                92.0,
                crate::motion::MICRO_DURATION_S,
                |ui| {
                    render_quick_login_drawer(app, ui, entry, is_cached, "selector_quick_pw", t);
                },
            );
        },
    );
    should_close
}

fn render_quick_login_drawer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    entry: &RememberedServerAccount,
    is_cached: bool,
    pw_prefix: &str,
    t: &Tokens,
) {
    let entry_key = (norm_probe_url_key(&entry.server_url), entry.user_id.clone());
    let quick_open = app.view.account.quick_login_key.as_ref() == Some(&entry_key);

    ui.add_space(6.0);
    let hint_text = if is_cached {
        "Enter your password to verify and switch accounts."
    } else {
        "Enter your password to sign in."
    };
    ui.add(
        egui::Label::new(
            egui::RichText::new(hint_text)
                .size(11.0)
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(4.0);

    let busy = app.view.account.auth_busy;
    let mut submit_quick = false;
    let pw_id_salt = format!("{pw_prefix}_{}", entry.user_id);
    let pw_edit_id = ui.make_persistent_id(&pw_id_salt);
    if ui.is_enabled()
        && quick_open
        && std::mem::take(&mut app.view.account.quick_login_focus_pending)
    {
        ui.memory_mut(|m| m.request_focus(pw_edit_id));
    }

    let can_connect = !busy && !app.view.account.quick_login_password.is_empty();
    let cta = if busy {
        "Signing In..."
    } else if is_cached {
        "Verify"
    } else {
        "Sign In"
    };
    let avail_w = ui.available_width();
    let cancel_w = 60.0_f32;
    if avail_w < 285.0 {
        let pw_resp = draw_password_input(
            ui,
            &pw_id_salt,
            avail_w,
            &mut app.view.account.quick_login_password,
            &mut app.view.account.quick_login_show_password,
            t,
        );
        if pw_resp.changed {
            app.view.account.quick_login_error = None;
        }
        if ui.is_enabled() && pw_resp.enter_pressed && !busy {
            submit_quick = true;
        }
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if primary_action_button(ui, cta, can_connect, t).clicked() && ui.is_enabled() {
                submit_quick = true;
            }
            if fixed_secondary_button(ui, "Cancel", cancel_w, !busy, t).clicked() && ui.is_enabled()
            {
                app.view.account.clear_quick_login_draft();
                app.view.account.clear_pending_unsynced_switch();
            }
        });
    } else {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let btn_w = 78.0_f32;
            let input_w = (avail_w - btn_w - cancel_w - 12.0).max(110.0);
            let pw_resp = draw_password_input(
                ui,
                &pw_id_salt,
                input_w,
                &mut app.view.account.quick_login_password,
                &mut app.view.account.quick_login_show_password,
                t,
            );
            if pw_resp.changed {
                app.view.account.quick_login_error = None;
            }
            if ui.is_enabled() && pw_resp.enter_pressed && !busy {
                submit_quick = true;
            }
            if primary_action_button(ui, cta, can_connect, t).clicked() && ui.is_enabled() {
                submit_quick = true;
            }
            if fixed_secondary_button(ui, "Cancel", cancel_w, !busy, t).clicked() && ui.is_enabled()
            {
                app.view.account.clear_quick_login_draft();
                app.view.account.clear_pending_unsynced_switch();
            }
        });
    }
    if let Some(ref err) = app.view.account.quick_login_error {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(err).size(11.0).color(t.danger_fg));
    }
    if ui.is_enabled() && submit_quick && !busy {
        submit_quick_server_login(app, ui.ctx(), entry);
    }
}

pub(crate) fn render_select_local_account_footer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let mut should_close = false;
    let dismissible = crate::modal::is_account_picker_dismissible(app);
    ui.horizontal(|ui| {
        if app.view.account.selector_other_server_open {
            let busy = app.view.account.auth_busy || app.view.account.password_reset.busy;
            if secondary_action_button(ui, "Back to Accounts", !busy, t).clicked() {
                app.view.account.selector_other_server_open = false;
                app.view.account.exit_password_reset_flow();
                app.view.account.clear_pending_unsynced_switch();
                app.view.account.error_message = None;
                app.view.account.status_message = None;
            }
            if dismissible {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_action_button(ui, "Cancel", !busy, t).clicked() {
                        should_close = true;
                    }
                });
            }
        } else {
            let hint = if dismissible {
                "Pick an account above, or cancel to stay on your current workspace."
            } else {
                "Pick a saved account, create a local one, or sign in to a server."
            };
            ui.label(egui::RichText::new(hint).size(11.0).color(t.text_muted));
            if dismissible {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if secondary_action_button(ui, "Cancel", true, t).clicked() {
                        should_close = true;
                    }
                });
            }
        }
    });
    should_close
}

pub(crate) fn render_convert_local_account_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    if std::mem::take(&mut app.view.account.profile_probe_pending) {
        probe_all_known_servers(app, ui.ctx());
    }
    let mut submit_now = false;
    let busy = app.view.account.convert_busy;
    let server_configured = app.session.is_server_configured();
    let nick = if app.view.account.convert_source_nickname.trim().is_empty() {
        "Local Account".to_string()
    } else {
        app.view.account.convert_source_nickname.clone()
    };
    let count_label = format_notebook_count(app.view.account.convert_source_notebook_count);
    let target_url = app.view.account.server_url.trim().to_string();
    let target_desc = if server_configured && !target_url.is_empty() {
        format!("on {target_url}")
    } else {
        "on your configured PaperCraft Server".to_string()
    };

    ui.spacing_mut().item_spacing.y = 0.0;
    draw_section_card(ui, t, |ui| {
        ui.horizontal(|ui| {
            let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_icon(ui.painter(), ir, Icon::Sync, t.accent);
            ui.label(
                egui::RichText::new(format!("Convert '{nick}' to Cloud Account?"))
                    .size(13.0)
                    .strong()
                    .color(t.text),
            );
        });
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!(
                    "All {count_label}, folders, and settings stored locally in '{nick}' will be uploaded to your new cloud account {target_desc}. Your local copy stays right here on this device as the active Offline-First cache so you can continue working offline anytime."
                ))
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
    });

    ui.add_space(10.0);

    if !server_configured {
        if render_unconfigured_server_notice(app, ui, t, !busy) {
            return true;
        }
        ui.add_space(10.0);
    }

    draw_section_card(ui, t, |ui| {
        if app.view.account.is_registration_closed() {
            draw_status_banner(
                ui,
                BannerTone::Warning,
                "Registration is closed on this server. Only an existing account or an open server can accept new registrations.",
                t,
            );
            ui.add_space(8.0);
        }
        draw_field_label_row(
            ui,
            "Email Address",
            FieldHint::Muted("Used to sign in to your new cloud account"),
            t,
        );
        ui.add_space(4.0);
        ui.add_enabled_ui(!busy, |ui| {
            let email_resp = draw_text_input(
                ui,
                InputFieldSpec {
                    id_salt: "convert_local_email_input",
                    width: ui.available_width(),
                    hint: "you@example.com",
                    border_color: None,
                    trailing: InputTrailing::None,
                },
                &mut app.view.account.convert_email,
                t,
            );
            if app.view.account.convert_focus_pending {
                ui.memory_mut(|m| {
                    m.request_focus(ui.make_persistent_id("convert_local_email_input"))
                });
                app.view.account.convert_focus_pending = false;
            }
            if email_resp.changed {
                app.view.account.convert_error = None;
            }
            if email_resp.enter_pressed {
                submit_now = true;
            }

            ui.add_space(10.0);
            draw_field_label_row(ui, "Password", FieldHint::Muted("At least 8 characters"), t);
            ui.add_space(4.0);
            let pw_resp = draw_password_input(
                ui,
                "convert_local_password_input",
                ui.available_width(),
                &mut app.view.account.convert_password,
                &mut app.view.account.convert_show_password,
                t,
            );
            if pw_resp.changed {
                app.view.account.convert_error = None;
            }
            if pw_resp.enter_pressed {
                submit_now = true;
            }
        });
    });

    if let Some(ref err) = app.view.account.convert_error {
        ui.add_space(8.0);
        draw_status_banner(ui, BannerTone::Error, err, t);
    }
    if busy {
        ui.add_space(8.0);
        render_convert_progress_card(app, ui, t);
    }

    if submit_now && !busy && !app.view.account.is_registration_closed() {
        submit_convert_local_account(app, ui.ctx());
    }
    false
}

fn render_convert_progress_card(app: &PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let stage = if app.view.account.convert_progress_stage.trim().is_empty() {
        "Creating cloud account and uploading local notebooks...".to_string()
    } else {
        app.view.account.convert_progress_stage.clone()
    };
    let total = app.view.account.convert_progress_total;
    let completed = app.view.account.convert_progress_completed.min(total);
    let fraction = if total > 0 {
        (completed as f32 / total as f32).clamp(0.05, 1.0)
    } else {
        0.15
    };
    egui::Frame::new()
        .fill(t.accent_soft)
        .stroke(Stroke::new(1.0, t.accent))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new(stage)
                        .size(11.5)
                        .strong()
                        .color(t.accent),
                );
            });
            ui.add_space(5.0);
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(ui.available_width())
                    .desired_height(6.0)
                    .fill(t.accent)
                    .animate(true),
            );
        });
}

pub(crate) fn render_convert_local_account_footer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let mut should_close = false;
    let busy = app.view.account.convert_busy;
    let blocked_closed = app.view.account.is_registration_closed();
    ui.horizontal(|ui| {
        if secondary_action_button(ui, "Cancel", !busy, t).clicked() {
            app.view.account.convert_error = None;
            should_close = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let cta = if busy {
                "Syncing to Server..."
            } else {
                "Create Cloud Account & Sync"
            };
            if primary_action_button(ui, cta, !busy && !blocked_closed, t).clicked() {
                submit_convert_local_account(app, ui.ctx());
            }
        });
    });
    should_close
}

fn submit_convert_local_account(app: &mut PaperCraftApp, ctx: &egui::Context) {
    if !app.session.is_server_configured() {
        app.view.account.convert_error = Some(
            "No sync server configured yet. Configure your server URL in Profile → Sync first."
                .to_string(),
        );
        return;
    }
    if app.view.account.is_registration_closed() {
        app.view.account.convert_error = Some("Registration is closed on this server.".to_string());
        return;
    }
    let url = match normalize_server_url(&app.view.account.server_url) {
        Ok(norm) => norm,
        Err(err) => {
            app.view.account.convert_error = Some(err.to_string());
            return;
        }
    };
    let (email, _) = match normalize_email(&app.view.account.convert_email) {
        Ok(pair) => pair,
        Err(err) => {
            app.view.account.convert_error = Some(err.to_string());
            return;
        }
    };
    if let Err(err) = validate_password(&app.view.account.convert_password) {
        app.view.account.convert_error = Some(err.to_string());
        return;
    }

    let source_profile_id = app.view.account.convert_source_profile_id.clone();
    let (source_nickname, batch) = match app
        .session
        .prepare_local_account_conversion_batch(&source_profile_id)
    {
        Ok(res) => res,
        Err(err) => {
            app.view.account.convert_error = Some(err);
            return;
        }
    };

    let device_name = app.session.device_name().to_string();
    let password = app.view.account.convert_password.clone();
    app.view.account.server_url = url.clone();
    app.view.account.convert_busy = true;
    app.view.account.convert_progress_stage = "Creating cloud account...".to_string();
    app.view.account.convert_progress_completed = 0;
    app.view.account.convert_progress_total = batch.local_docs.len().max(1) + 2;
    app.view.account.convert_error = None;
    let _ = app.session.set_server_url_preference(&url);

    let cmd = SyncWorkerCommand::ConvertLocalAccount {
        session_generation: app.sync_worker.current_session_generation(),
        server_url: url,
        source_profile_id,
        source_nickname,
        email,
        password,
        device_name,
        upload_batch: Box::new(batch),
    };
    if !crate::sync_worker::dispatch_command(app, Some(ctx), cmd) {
        app.view.account.convert_busy = false;
        app.view.account.convert_progress_stage.clear();
        app.view.account.convert_progress_completed = 0;
        app.view.account.convert_progress_total = 0;
        app.view.account.convert_error =
            Some("Background network worker is unavailable.".to_string());
    }
}

pub(crate) fn summary_account_label(summary: &SyncStatusSummary) -> String {
    summary
        .email
        .clone()
        .or_else(|| summary.username.as_ref().map(|u| format!("@{u}")))
        .unwrap_or_else(|| "your account".to_string())
}

fn render_signed_out_body(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut submit_on_enter = false;

    draw_section_card(ui, t, |ui| {
        render_signed_out_local_workspace_card(app, ui, t);
    });

    ui.add_space(9.0);

    if render_unconfigured_server_notice(app, ui, t, true) {
        return true;
    }
    if !app.session.is_server_configured() {
        ui.add_space(9.0);
    }

    draw_section_card(ui, t, |ui| {
        if render_auth_card_contents(app, ui, t, false) {
            submit_on_enter = true;
        }
    });

    render_auth_banners(app, ui, t);

    if submit_on_enter {
        handle_auth_submit_on_enter(app, ui.ctx());
    }
    false
}

fn is_active_server_known_offline(app: &PaperCraftApp) -> bool {
    matches!(
        app.view.account.probe_for(&app.view.account.server_url),
        ServerProbeState::Failed { .. }
    )
}

pub(crate) fn handle_auth_submit_on_enter(app: &mut PaperCraftApp, ctx: &egui::Context) {
    if app.view.account.password_reset.active {
        if !app.view.account.password_reset.busy {
            submit_complete_password_reset(app, ctx);
        }
    } else if !app.view.account.auth_busy
        && !(app.view.account.tab == AuthTab::CreateAccount
            && app.view.account.is_registration_closed())
    {
        submit_auth_request(app, ctx);
    }
}

pub(crate) fn render_auth_card_contents(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    show_inline_submit: bool,
) -> bool {
    let auth_flow_key: u8 = if app.view.account.password_reset.active {
        2
    } else if app.view.account.tab == AuthTab::CreateAccount {
        1
    } else {
        0
    };
    let dt = ui.ctx().input(|i| i.stable_dt);
    if app
        .view
        .motion
        .auth_flow
        .advance(auth_flow_key, isize::from(auth_flow_key), dt)
    {
        ui.ctx().request_repaint();
    }
    let flow_opacity = app.view.motion.auth_flow.opacity();
    let flow_offset_x = app.view.motion.auth_flow.offset_x();

    if app.view.account.password_reset.active {
        return crate::motion::show_flow_content(ui, flow_opacity, flow_offset_x, |ui| {
            render_password_reset_flow_card(app, ui, t, show_inline_submit)
        });
    }

    let mut submit_on_enter = false;
    draw_auth_mode_tabs(app, ui, t);
    ui.add_space(9.0);

    let tab = app.view.account.tab;
    crate::motion::show_flow_content(ui, flow_opacity, flow_offset_x, |ui| {
        render_auth_tab_explanation(app, ui, t);
        ui.add_space(8.0);

        if app.session.is_server_configured() && is_active_server_known_offline(app) {
            draw_status_banner(
                ui,
                BannerTone::Warning,
                "Last check could not reach your sync server — you can still retry signing in or check settings in Profile → Sync.",
                t,
            );
            ui.add_space(8.0);
        } else if matches!(tab, AuthTab::CreateAccount) && app.view.account.is_registration_closed()
        {
            draw_status_banner(
                ui,
                BannerTone::Warning,
                "Registration is closed on this server. Switch to Sign In to use an existing account.",
                t,
            );
            ui.add_space(8.0);
        }

        if render_account_credentials_fields(app, ui, t) {
            submit_on_enter = true;
        }
    });

    if tab == AuthTab::SignIn {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let link = ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("Have a recovery code from the server?")
                            .size(11.0)
                            .color(t.accent),
                    )
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::ZERO),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if link.clicked() {
                let id = app.view.account.email_or_username.trim().to_string();
                app.view.account.enter_password_reset_flow(
                    id,
                    None,
                    "Enter the recovery code generated on the server and choose a new password."
                        .to_string(),
                );
            }
        });
    }

    if show_inline_submit {
        ui.add_space(10.0);
        let busy = app.view.account.auth_busy;
        let blocked_closed =
            tab == AuthTab::CreateAccount && app.view.account.is_registration_closed();
        let cta = match (tab, busy) {
            (AuthTab::SignIn, false) => "Sign In",
            (AuthTab::SignIn, true) => "Signing In...",
            (AuthTab::CreateAccount, false) => "Create Account",
            (AuthTab::CreateAccount, true) => "Creating...",
        };
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if primary_action_button(ui, cta, !busy && !blocked_closed, t).clicked() {
                    submit_auth_request(app, ui.ctx());
                }
            });
        });
    }

    submit_on_enter
}

fn render_password_reset_flow_card(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    show_inline_submit: bool,
) -> bool {
    let mut submit_on_enter = false;
    ui.horizontal(|ui| {
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::Reset, t.accent);
        ui.label(
            egui::RichText::new("Complete Password Reset")
                .size(13.5)
                .strong()
                .color(t.text),
        );
    });
    ui.add_space(4.0);
    let banner = app
        .view
        .account
        .password_reset
        .banner_message
        .clone()
        .unwrap_or_else(|| {
            "Enter the temporary recovery code generated on your server and choose a new password."
                .to_string()
        });
    draw_status_banner(
        ui,
        if app.view.account.password_reset.code_preverified {
            BannerTone::Info
        } else {
            BannerTone::Warning
        },
        &banner,
        t,
    );

    ui.add_space(8.0);
    draw_field_label_row(ui, "Email", FieldHint::Muted("Account being recovered"), t);
    ui.add_space(3.0);
    if draw_text_input(
        ui,
        InputFieldSpec {
            id_salt: "acct_reset_identifier",
            width: ui.available_width(),
            hint: "you@example.com",
            border_color: None,
            trailing: InputTrailing::None,
        },
        &mut app.view.account.password_reset.identifier,
        t,
    )
    .enter_pressed
    {
        submit_on_enter = true;
    }

    ui.add_space(7.0);
    let code_hint = if app.view.account.password_reset.code_preverified {
        FieldHint::Success("Recovery code verified")
    } else {
        FieldHint::Muted("Code generated on server")
    };
    draw_field_label_row(ui, "Temporary Recovery Code", code_hint, t);
    ui.add_space(3.0);
    if draw_password_input(
        ui,
        "acct_reset_code",
        ui.available_width(),
        &mut app.view.account.password_reset.recovery_code,
        &mut app.view.account.password_reset.show_recovery_code,
        t,
    )
    .enter_pressed
    {
        submit_on_enter = true;
    }

    ui.add_space(7.0);
    let pw_chars = app.view.account.password_reset.new_password.chars().count();
    let pw_hint = if pw_chars == 0 {
        FieldHint::Muted("Minimum 8 characters")
    } else if pw_chars >= MIN_PASSWORD_LEN {
        FieldHint::Success("Ready (8+ characters)")
    } else {
        FieldHint::Warning(format!("{pw_chars}/{MIN_PASSWORD_LEN} characters"))
    };
    draw_field_label_row(ui, "New Password", pw_hint, t);
    ui.add_space(3.0);
    if draw_password_input(
        ui,
        "acct_reset_new_pw",
        ui.available_width(),
        &mut app.view.account.password_reset.new_password,
        &mut app.view.account.password_reset.show_new_password,
        t,
    )
    .enter_pressed
    {
        submit_on_enter = true;
    }

    ui.add_space(7.0);
    let confirm_hint = if app.view.account.password_reset.confirm_password.is_empty() {
        FieldHint::Muted("Re-enter new password")
    } else if app.view.account.password_reset.new_password
        == app.view.account.password_reset.confirm_password
    {
        FieldHint::Success("Passwords match")
    } else {
        FieldHint::Warning("Passwords do not match".to_string())
    };
    draw_field_label_row(ui, "Confirm New Password", confirm_hint, t);
    ui.add_space(3.0);
    if draw_password_input(
        ui,
        "acct_reset_confirm_pw",
        ui.available_width(),
        &mut app.view.account.password_reset.confirm_password,
        &mut app.view.account.password_reset.show_new_password,
        t,
    )
    .enter_pressed
    {
        submit_on_enter = true;
    }

    if app.session.is_server_configured() && is_active_server_known_offline(app) {
        ui.add_space(6.0);
        draw_status_banner(
            ui,
            BannerTone::Warning,
            "Last check could not reach your sync server — you can still retry resetting your password or check settings in Profile → Sync.",
            t,
        );
    }

    if show_inline_submit {
        ui.add_space(10.0);
        let busy = app.view.account.password_reset.busy;
        ui.horizontal(|ui| {
            if secondary_action_button(ui, "Back to Sign In", !busy, t).clicked() {
                app.view.account.exit_password_reset_flow();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let cta = if busy {
                    "Updating Password..."
                } else {
                    "Set New Password & Sign In"
                };
                if primary_action_button(ui, cta, !busy, t).clicked() {
                    submit_complete_password_reset(app, ui.ctx());
                }
            });
        });
    }

    submit_on_enter
}

fn render_signed_out_local_workspace_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let active_name = app
        .session
        .active_local_profile()
        .map_or_else(|| "Personal".to_string(), |p| p.nickname.clone());
    let nb_count = app.session.notebooks.len();
    let profile_count = app.session.accessible_local_profiles().len();

    ui.horizontal(|ui| {
        let (acct_icon, icon_fg, _, _) = account_icon_and_colors(false, false, t);
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, acct_icon, icon_fg);
        ui.label(
            egui::RichText::new(format!("On This Device: {active_name}"))
                .size(13.0)
                .strong()
                .color(t.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let toggle_lbl = if app.view.account.profiles_panel_open {
                "Hide Profiles"
            } else {
                "Profiles"
            };
            if fixed_secondary_button(ui, toggle_lbl, 86.0, true, t)
                .on_hover_text(format!("Manage {profile_count} offline profile(s)"))
                .clicked()
            {
                app.view.account.profiles_panel_open = !app.view.account.profiles_panel_open;
            }
            draw_account_type_badge(ui, false, false, t);
        });
    });
    ui.add_space(3.0);
    ui.label(
        egui::RichText::new(format!(
            "{nb_count} notebook(s) in this local workspace. Sync options become available after signing in to an account."
        ))
        .size(11.0)
        .color(t.text_muted),
    );

    let collapse_id = ui.make_persistent_id("signed_out_local_profiles_collapse");
    crate::motion::animated_collapse_with_duration(
        ui,
        collapse_id,
        app.view.account.profiles_panel_open,
        148.0,
        crate::motion::MICRO_DURATION_S,
        |ui| {
            ui.add_space(8.0);
            draw_hairline(ui, t);
            ui.add_space(8.0);
            render_local_profiles_manager(app, ui, true, false, t);
        },
    );
}

fn render_auth_tab_explanation(app: &PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let msg = match app.view.account.tab {
        AuthTab::SignIn => "Sign in with your account email and password.",
        AuthTab::CreateAccount => "Create an account on this server with a unique email.",
    };
    ui.add(egui::Label::new(egui::RichText::new(msg).size(11.0).color(t.text_muted)).wrap());
}

fn render_signup_username_column(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    let uname_trimmed = app.view.account.signup_username.trim();
    let uname_hint = if uname_trimmed.is_empty() {
        FieldHint::Muted("Optional")
    } else {
        match normalize_username(uname_trimmed) {
            Ok(_) => FieldHint::Success("Valid @username"),
            Err(err) => FieldHint::Warning(err.to_string()),
        }
    };
    draw_field_label_row(ui, "Username", uname_hint, t);
    ui.add_space(3.0);
    draw_text_input(
        ui,
        InputFieldSpec {
            id_salt: "acct_signup_username",
            width: ui.available_width(),
            hint: "alex_notes (auto if empty)",
            border_color: None,
            trailing: InputTrailing::None,
        },
        &mut app.view.account.signup_username,
        t,
    )
    .enter_pressed
}

fn render_signup_display_name_column(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    draw_field_label_row(ui, "Display Name", FieldHint::Muted("Optional"), t);
    ui.add_space(3.0);
    draw_text_input(
        ui,
        InputFieldSpec {
            id_salt: "acct_display_name",
            width: ui.available_width(),
            hint: "Shown across devices",
            border_color: None,
            trailing: InputTrailing::None,
        },
        &mut app.view.account.display_name,
        t,
    )
    .enter_pressed
}

fn render_account_credentials_fields(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let mut submit_on_enter = false;
    let tab = app.view.account.tab;
    let is_create = matches!(tab, AuthTab::CreateAccount);

    let id_hint = if app.view.account.email_or_username.trim().is_empty() {
        if is_create {
            FieldHint::Muted("Unique account email")
        } else {
            FieldHint::Muted("Account email address")
        }
    } else if normalize_email(&app.view.account.email_or_username).is_ok() {
        FieldHint::Success("Valid email")
    } else {
        FieldHint::Warning("Enter a valid email address".to_string())
    };

    draw_field_label_row(ui, "Email", id_hint, t);
    ui.add_space(3.0);

    let user_trailing = if normalize_email(&app.view.account.email_or_username).is_ok() {
        InputTrailing::GreenCheck("Valid email format")
    } else {
        InputTrailing::None
    };
    let resp = draw_text_input(
        ui,
        InputFieldSpec {
            id_salt: "acct_email_or_username",
            width: ui.available_width(),
            hint: "you@example.com",
            border_color: None,
            trailing: user_trailing,
        },
        &mut app.view.account.email_or_username,
        t,
    );
    if resp.enter_pressed {
        submit_on_enter = true;
    }

    if is_create {
        ui.add_space(7.0);
        let avail_w = ui.available_width();
        if avail_w >= 360.0 {
            let col_gap = 10.0_f32;
            let col_w = ((avail_w - col_gap) * 0.5).max(120.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = col_gap;
                ui.allocate_ui_with_layout(
                    Vec2::new(col_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(col_w);
                        if render_signup_username_column(app, ui, t) {
                            submit_on_enter = true;
                        }
                    },
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(col_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(col_w);
                        if render_signup_display_name_column(app, ui, t) {
                            submit_on_enter = true;
                        }
                    },
                );
            });
        } else {
            if render_signup_username_column(app, ui, t) {
                submit_on_enter = true;
            }
            ui.add_space(7.0);
            if render_signup_display_name_column(app, ui, t) {
                submit_on_enter = true;
            }
        }
    }

    ui.add_space(7.0);
    let pw_chars = app.view.account.password.chars().count();
    let pw_hint = if tab == AuthTab::CreateAccount && pw_chars > 0 {
        if pw_chars >= MIN_PASSWORD_LEN {
            FieldHint::Success("Ready (8+ characters)")
        } else {
            FieldHint::Warning(format!("{pw_chars}/{MIN_PASSWORD_LEN} characters"))
        }
    } else {
        FieldHint::Muted("Minimum 8 characters")
    };
    draw_field_label_row(ui, "Password", pw_hint, t);
    ui.add_space(3.0);
    let pw_resp = draw_password_input(
        ui,
        "acct_password",
        ui.available_width(),
        &mut app.view.account.password,
        &mut app.view.account.show_password,
        t,
    );
    if pw_resp.enter_pressed {
        submit_on_enter = true;
    }

    submit_on_enter
}

fn save_server_url_preference(app: &mut PaperCraftApp) {
    match app
        .session
        .set_server_url_preference(&app.view.account.server_url)
    {
        Ok(norm) => {
            app.view.account.server_url = norm.clone();
            app.view.account.invalidate_probe_if_url_changed();
            app.view.account.error_message = None;
            app.view.account.status_message = Some(format!("Sync server saved ({norm})."));
            app.flush_web_storage(true);
        }
        Err(err) => {
            app.view.account.status_message = None;
            app.view.account.error_message = Some(err);
        }
    }
}

fn draw_server_probe_url_input(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    id_salt: &str,
    width: f32,
    border_override: Option<Color32>,
    trailing: InputTrailing,
    t: &Tokens,
) -> bool {
    let url_resp = draw_text_input(
        ui,
        InputFieldSpec {
            id_salt,
            width,
            hint: DEFAULT_SERVER_URL,
            border_color: border_override,
            trailing,
        },
        &mut app.view.account.server_url,
        t,
    );
    if url_resp.changed {
        app.view.account.invalidate_probe_if_url_changed();
        app.view.account.error_message = None;
    }
    url_resp.enter_pressed
}

pub(crate) fn render_server_probe_section(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    show_examples: bool,
) -> bool {
    let mut test_on_enter = false;
    let probe = app.view.account.probe.clone();
    let configured = app.session.is_server_configured();

    ui.horizontal(|ui| {
        let server_online = !matches!(probe, ServerProbeState::Failed { .. });
        let (acct_icon, icon_color, _, _) = account_icon_and_colors(true, server_online, t);
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, acct_icon, icon_color);
        ui.label(
            egui::RichText::new("Self-Hosted Server")
                .size(13.0)
                .strong()
                .color(t.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            draw_probe_status_pill_with_config(ui, &probe, configured, t);
        });
    });
    ui.add_space(2.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("Enter your local or VPN PaperCraft Server URL.")
                .size(11.0)
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(8.0);

    let (border_override, trailing) = match &probe {
        ServerProbeState::Connected { .. } => (
            Some(t.success_border),
            InputTrailing::GreenCheck("Server connection verified"),
        ),
        ServerProbeState::Failed { .. } => (
            Some(t.danger_border),
            InputTrailing::Icon(Icon::AlertCircle, t.danger_fg, "Server connection failed"),
        ),
        ServerProbeState::Checking { .. } => (
            None,
            InputTrailing::Icon(Icon::Sync, t.accent, "Checking connection..."),
        ),
        ServerProbeState::Untested => (None, InputTrailing::None),
    };

    let testing = app.view.account.is_testing();
    let test_w = 72.0_f32;
    let save_w = 64.0_f32;
    let gap = 6.0_f32;
    let avail_w = ui.available_width();
    let test_lbl = if testing { "..." } else { "Test" };
    let can_save = !app.view.account.server_url.trim().is_empty()
        && (!configured
            || !urls_match(
                &app.view.account.server_url,
                app.session.preferred_server_url(),
            ));
    let mut clicked_test = false;
    let mut clicked_save = false;

    if avail_w < 330.0 {
        test_on_enter = draw_server_probe_url_input(
            app,
            ui,
            "acct_server_url_signed_out",
            avail_w,
            border_override,
            trailing,
            t,
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, 6.0);
            if fixed_secondary_button(ui, test_lbl, test_w, !testing, t)
                .on_hover_text("Verify server reachability (Enter)")
                .clicked()
            {
                clicked_test = true;
            }
            if fixed_secondary_button(ui, "Save", save_w, can_save, t)
                .on_hover_text("Save this sync server URL and enable server sign-in")
                .clicked()
            {
                clicked_save = true;
            }
        });
    } else {
        let input_w = (avail_w - test_w - save_w - gap * 2.0).max(110.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            test_on_enter = draw_server_probe_url_input(
                app,
                ui,
                "acct_server_url_signed_out",
                input_w,
                border_override,
                trailing,
                t,
            );
            if fixed_secondary_button(ui, test_lbl, test_w, !testing, t)
                .on_hover_text("Verify server reachability (Enter)")
                .clicked()
            {
                clicked_test = true;
            }
            if fixed_secondary_button(ui, "Save", save_w, can_save, t)
                .on_hover_text("Save this sync server URL and enable server sign-in")
                .clicked()
            {
                clicked_save = true;
            }
        });
    }
    if clicked_test {
        trigger_test_connection(app, ui.ctx());
    }
    if clicked_save {
        save_server_url_preference(app);
    }

    ui.add_space(7.0);
    render_probe_feedback_row(app, ui, &probe, show_examples, configured, t);
    ui.add_space(7.0);
    draw_server_download_link_row(app, ui, t);

    test_on_enter
}

fn draw_server_download_link_row(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut open_site = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 3.0);
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::Link, t.text_muted);
        ui.label(
            egui::RichText::new("Need the server?")
                .size(11.0)
                .color(t.text_muted),
        );
        let link_resp = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(
                        "Download PaperCraft Server for your platform (papercraft.ink)",
                    )
                    .size(11.0)
                    .color(t.accent),
                )
                .wrap()
                .sense(Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(
                "Open papercraft.ink to download PaperCraft Server for Windows, macOS, or Linux",
            );
        if link_resp.hovered() {
            ui.painter().hline(
                link_resp.rect.x_range(),
                link_resp.rect.bottom() - 1.0,
                Stroke::new(1.0, t.accent),
            );
        }
        if link_resp.clicked() {
            open_site = true;
        }
    });
    if open_site {
        let _ = crate::open_external_url(app, ui.ctx(), crate::WEBSITE_URL);
    }
}

fn render_probe_feedback_row(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    probe: &ServerProbeState,
    show_examples: bool,
    configured: bool,
    t: &Tokens,
) {
    match probe {
        ServerProbeState::Untested => {
            if !configured {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Enter your PaperCraft Server URL and click Save or Test to enable sign-in.",
                        )
                        .size(10.5)
                        .color(t.text_muted),
                    )
                    .wrap(),
                );
                ui.add_space(3.0);
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 3.0);
                let help = if show_examples {
                    "Local: http://127.0.0.1:8787  •  VPN: http://100.x.y.z:8787"
                } else {
                    "Press Enter or click Test to check server reachability."
                };
                ui.add(
                    egui::Label::new(egui::RichText::new(help).size(10.5).color(t.text_muted))
                        .wrap(),
                );
                if app.view.account.server_url.trim() != DEFAULT_SERVER_URL {
                    let link = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Use localhost")
                                    .size(10.5)
                                    .color(t.accent),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .min_size(Vec2::ZERO),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if link.clicked() {
                        app.view.account.server_url = DEFAULT_SERVER_URL.to_string();
                        app.view.account.invalidate_probe_if_url_changed();
                        app.view.account.error_message = None;
                    }
                }
            });
        }
        ServerProbeState::Checking { .. } => {
            draw_status_banner(
                ui,
                BannerTone::Info,
                "Checking connection to PaperCraft Server...",
                t,
            );
        }
        ServerProbeState::Connected {
            version,
            protocol_version,
            registration_open,
            storage_engine,
            max_notebook_bytes,
            ..
        } => {
            draw_connected_server_banner(
                ui,
                version,
                *protocol_version,
                *registration_open,
                storage_engine,
                *max_notebook_bytes,
                t,
            );
        }
        ServerProbeState::Failed { message, .. } => {
            draw_status_banner(ui, BannerTone::Error, message, t);
        }
    }
}

fn draw_connected_server_banner(
    ui: &mut egui::Ui,
    version: &str,
    protocol_version: u32,
    registration_open: bool,
    storage_engine: &str,
    max_notebook_bytes: usize,
    t: &Tokens,
) {
    let mut capability_parts = vec![format!("Protocol v{protocol_version}")];
    if !storage_engine.trim().is_empty() {
        capability_parts.push(format!("Storage: {storage_engine}"));
    }
    if max_notebook_bytes > 0 {
        capability_parts.push(format!(
            "Max notebook: {}",
            crate::inspector::format_storage_bytes(max_notebook_bytes as u64)
        ));
    }

    egui::Frame::new()
        .fill(t.success_bg)
        .stroke(Stroke::new(1.0, t.success_border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (badge_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_green_check_badge(ui.painter(), badge_rect, t);
                let title = if version.trim().is_empty() {
                    "Connected to PaperCraft Server".to_string()
                } else {
                    format!("Connected to PaperCraft Server v{version}")
                };
                ui.label(
                    egui::RichText::new(title)
                        .size(11.5)
                        .strong()
                        .color(t.success_fg),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (reg_lbl, reg_fg) = if registration_open {
                        ("Registration open", t.success_fg)
                    } else {
                        ("Registration closed", t.warning_fg)
                    };
                    ui.label(
                        egui::RichText::new(reg_lbl)
                            .size(10.5)
                            .strong()
                            .color(reg_fg),
                    );
                });
            });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(capability_parts.join(" · "))
                    .size(10.5)
                    .color(t.text_muted),
            );
        });
}

fn draw_probe_status_pill(ui: &mut egui::Ui, probe: &ServerProbeState, t: &Tokens) {
    draw_probe_status_pill_with_config(ui, probe, true, t);
}

fn draw_probe_status_pill_with_config(
    ui: &mut egui::Ui,
    probe: &ServerProbeState,
    configured: bool,
    t: &Tokens,
) {
    match probe {
        ServerProbeState::Untested => {
            draw_pill_badge(
                ui,
                PillStyle {
                    fill: t.toolbar_bg,
                    border: t.border,
                    fg: t.text_muted,
                    icon: PillIcon::None,
                },
                if configured {
                    "Not tested"
                } else {
                    "Not configured"
                },
            );
        }
        ServerProbeState::Checking { .. } => {
            draw_pill_badge(
                ui,
                PillStyle {
                    fill: t.accent_soft,
                    border: t.accent,
                    fg: t.accent,
                    icon: PillIcon::Vector(Icon::Sync),
                },
                "Checking...",
            );
        }
        ServerProbeState::Connected { .. } => {
            draw_pill_badge(
                ui,
                PillStyle {
                    fill: t.success_bg,
                    border: t.success_border,
                    fg: t.success_fg,
                    icon: PillIcon::GreenCheck(t),
                },
                "Connected",
            );
        }
        ServerProbeState::Failed { .. } => {
            draw_pill_badge(
                ui,
                PillStyle {
                    fill: t.danger_bg,
                    border: t.danger_border,
                    fg: t.danger_fg,
                    icon: PillIcon::Vector(Icon::AlertCircle),
                },
                "Unreachable",
            );
        }
    }
}

fn render_signed_out_footer(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    render_server_auth_footer(app, ui, t)
}

fn render_server_auth_footer(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    let mut should_close = false;
    ui.horizontal(|ui| {
        if app.view.account.password_reset.active {
            let busy = app.view.account.password_reset.busy;
            if secondary_action_button(ui, "Back to Sign In", !busy, t).clicked() {
                app.view.account.exit_password_reset_flow();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let cta = if busy {
                    "Updating..."
                } else {
                    "Set New Password & Sign In"
                };
                if primary_action_button(ui, cta, !busy, t).clicked() {
                    submit_complete_password_reset(app, ui.ctx());
                }
            });
            return;
        }

        if secondary_action_button(ui, "Cancel", true, t).clicked() {
            should_close = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let busy = app.view.account.auth_busy;
            let blocked_closed = app.view.account.tab == AuthTab::CreateAccount
                && app.view.account.is_registration_closed();
            let cta = match (app.view.account.tab, busy) {
                (AuthTab::SignIn, false) => "Sign In",
                (AuthTab::SignIn, true) => "Signing In...",
                (AuthTab::CreateAccount, false) => "Create Account",
                (AuthTab::CreateAccount, true) => "Creating...",
            };
            if primary_action_button(ui, cta, !busy && !blocked_closed, t).clicked() {
                submit_auth_request(app, ui.ctx());
            }
        });
    });
    should_close
}

fn render_signed_in_body(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    ui.spacing_mut().item_spacing.y = 0.0;
    let summary = app.session.sync_status_summary();
    let cloud_pending = app.session.cloud_pending_changes_count();

    draw_section_card(ui, t, |ui| {
        render_signed_in_account_card(app, ui, &summary, cloud_pending, t);
    });

    if app.session.is_session_expired()
        && let Some(acct) = app.session.active_account_session().cloned()
    {
        ui.add_space(6.0);
        render_expired_session_reauth_box(app, ui, &acct, cloud_pending, "modal_reauth_pw", t);
    }

    if let Some(ref err) = summary.last_error
        && app.view.account.error_message.is_none()
    {
        ui.add_space(8.0);
        draw_status_banner(ui, BannerTone::Error, err, t);
    }
    if summary.last_error.is_none() || app.view.account.error_message.is_some() {
        render_auth_banners(app, ui, t);
    }
    false
}

pub(crate) fn render_signed_in_account_card(
    _app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    cloud_pending: usize,
    t: &Tokens,
) {
    let display = summary
        .display_name
        .clone()
        .unwrap_or_else(|| "Account".to_string());
    let primary_id = if summary.signed_in {
        summary_account_label(summary)
    } else {
        "Cloud Account".to_string()
    };

    ui.horizontal(|ui| {
        let (avatar_rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
        ui.painter()
            .rect_filled(avatar_rect, CornerRadius::same(9), t.accent_soft);
        ui.painter().rect_stroke(
            avatar_rect,
            CornerRadius::same(9),
            Stroke::new(1.0, t.accent.gamma_multiply(0.35)),
            StrokeKind::Inside,
        );
        paint_icon(
            ui.painter(),
            avatar_rect.shrink(7.0),
            Icon::Account,
            t.accent,
        );

        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&display)
                        .size(14.0)
                        .strong()
                        .color(t.text),
                );
                ui.label(
                    egui::RichText::new(&primary_id)
                        .size(11.5)
                        .color(t.text_muted),
                );
            });
            ui.label(
                egui::RichText::new(format!(
                    "Device: {} · Offline-First Local Cache",
                    summary.device_name
                ))
                .size(11.0)
                .color(t.text_muted),
            );
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            draw_sync_connection_pill(ui, summary.connection, t);
        });
    });

    ui.add_space(9.0);
    draw_hairline(ui, t);
    ui.add_space(8.0);

    let queue_value = if summary.connection == SyncConnectionState::Offline {
        match cloud_pending {
            0 => "Saved locally".to_string(),
            1 => "1 queued offline".to_string(),
            n => format!("{n} queued offline"),
        }
    } else {
        match cloud_pending {
            0 => "Synced to cloud".to_string(),
            1 => "1 pending sync".to_string(),
            n => format!("{n} pending syncs"),
        }
    };
    let last_sync_value = format_last_sync(summary.last_sync_at_ms);
    let mode_value = if summary.connection == SyncConnectionState::Offline {
        "Offline • Cached locally".to_string()
    } else {
        "Offline-First • Synced".to_string()
    };

    ui.columns(3, |cols| {
        draw_metric_cell(&mut cols[0], "SYNC QUEUE", &queue_value, t);
        draw_metric_cell(&mut cols[1], "LAST SYNC", &last_sync_value, t);
        draw_metric_cell(&mut cols[2], "STORAGE", &mode_value, t);
    });
}

fn render_account_email_row(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    let email_text = match summary.email.as_deref() {
        Some(email) if !email.trim().is_empty() => format!("Email: {email}"),
        _ => "No email set".to_string(),
    };
    let toggle_lbl = if app.view.account.change_email_open {
        "Close"
    } else if summary.email.is_some() {
        "Change Email"
    } else {
        "Add Email"
    };
    if ui.available_width() < 340.0 {
        ui.add(
            egui::Label::new(
                egui::RichText::new(email_text)
                    .size(11.0)
                    .color(t.text_muted),
            )
            .wrap(),
        );
        ui.add_space(6.0);
        if fixed_secondary_button(ui, toggle_lbl, 96.0, true, t).clicked() {
            app.view.account.change_email_open = !app.view.account.change_email_open;
            app.view.account.error_message = None;
        }
    } else {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(email_text)
                    .size(11.0)
                    .color(t.text_muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if fixed_secondary_button(ui, toggle_lbl, 96.0, true, t).clicked() {
                    app.view.account.change_email_open = !app.view.account.change_email_open;
                    app.view.account.error_message = None;
                }
            });
        });
    }

    crate::motion::animated_collapse_with_duration(
        ui,
        ui.make_persistent_id("acct_change_email_drawer"),
        app.view.account.change_email_open,
        156.0,
        crate::motion::MICRO_DURATION_S,
        |ui| {
            ui.add_space(8.0);
            draw_field_label_row(ui, "New Email", FieldHint::Muted("Must be unique"), t);
            ui.add_space(3.0);
            let mut submit_email = false;
            let email_resp = draw_text_input(
                ui,
                InputFieldSpec {
                    id_salt: "acct_change_email_new",
                    width: ui.available_width(),
                    hint: "new@example.com",
                    border_color: None,
                    trailing: InputTrailing::None,
                },
                &mut app.view.account.new_email_draft,
                t,
            );
            if email_resp.enter_pressed {
                submit_email = true;
            }

            ui.add_space(6.0);
            draw_field_label_row(
                ui,
                "Current Password",
                FieldHint::Muted("Verify identity"),
                t,
            );
            ui.add_space(3.0);
            let pw_resp = draw_password_input(
                ui,
                "acct_change_email_pw",
                ui.available_width(),
                &mut app.view.account.change_email_password,
                &mut app.view.account.show_change_email_password,
                t,
            );
            if pw_resp.enter_pressed {
                submit_email = true;
            }

            ui.add_space(7.0);
            let busy = app.view.account.change_email_busy;
            ui.horizontal(|ui| {
                let save_lbl = if busy { "Saving..." } else { "Save Email" };
                if (fixed_secondary_button(ui, save_lbl, 96.0, !busy, t).clicked()
                    || (ui.is_enabled() && submit_email))
                    && !busy
                {
                    submit_change_email(app, ui.ctx());
                }
            });
        },
    );
}

pub(crate) fn render_local_profile_identity_card(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    let nb_count = app.session.notebooks.len();
    let count_label = format_notebook_count(nb_count);
    let active = app.session.active_local_profile().cloned();
    let (acct_icon, icon_fg, _, _) = account_icon_and_colors(false, false, t);

    ui.horizontal(|ui| {
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, acct_icon, icon_fg);
        ui.label(
            egui::RichText::new("Local Account Profile")
                .size(13.5)
                .strong()
                .color(t.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            draw_account_type_badge(ui, false, false, t);
        });
    });
    ui.add_space(3.0);

    let Some(active) = active else {
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{count_label} in temporary local workspace."))
                    .size(11.0)
                    .color(t.text_muted),
            )
            .wrap(),
        );
        return;
    };

    ui.add(
        egui::Label::new(
            egui::RichText::new(format!("{count_label} stored on this device."))
                .size(11.0)
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(10.0);

    draw_field_label_row(
        ui,
        "Local Nickname",
        FieldHint::Muted("Displayed in Accounts"),
        t,
    );
    ui.add_space(4.0);
    let mut submit_rename = false;
    let resp = draw_text_input(
        ui,
        InputFieldSpec {
            id_salt: "profile_local_nickname_edit",
            width: ui.available_width(),
            hint: "Local nickname...",
            border_color: None,
            trailing: InputTrailing::None,
        },
        &mut app.view.account.local_profile_rename_draft,
        t,
    );
    if resp.enter_pressed {
        submit_rename = true;
    }

    let clean_draft = app
        .view
        .account
        .local_profile_rename_draft
        .trim()
        .to_string();
    let dirty = !clean_draft.is_empty() && clean_draft != active.nickname;

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if secondary_action_button(ui, "Reset", dirty, t).clicked() {
            app.view.account.local_profile_rename_draft = active.nickname.clone();
            app.view.account.error_message = None;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if primary_action_button(ui, "Save Nickname", dirty, t).clicked()
                || (submit_rename && dirty)
            {
                match app.session.execute(
                    "profile.rename",
                    &json!({
                        "profile_id": active.local_profile_id,
                        "nickname": clean_draft,
                    }),
                ) {
                    Ok(_) => {
                        app.view.account.error_message = None;
                        app.view.account.status_message =
                            Some(format!("Local account renamed to '{clean_draft}'."));
                        app.view.account.profile_section = ProfileSection::Accounts;
                        app.flush_web_storage(true);
                    }
                    Err(err) => {
                        app.view.account.status_message = None;
                        app.view.account.error_message = Some(err.to_string());
                    }
                }
            }
        });
    });
}

pub(crate) fn render_profile_identity_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let Some(sess) = app.session.active_account_session().cloned() else {
        return;
    };
    ui.horizontal(|ui| {
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::Account, t.accent);
        ui.label(
            egui::RichText::new("Profile Details")
                .size(13.5)
                .strong()
                .color(t.text),
        );
    });
    ui.add_space(3.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("How your name and @username appear on synced devices.")
                .size(11.0)
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(10.0);

    let mut submit_on_enter = false;
    draw_field_label_row(
        ui,
        "Display Name",
        FieldHint::Muted("Shown on synced devices"),
        t,
    );
    ui.add_space(3.0);
    if draw_text_input(
        ui,
        InputFieldSpec {
            id_salt: "profile_edit_display_name",
            width: ui.available_width(),
            hint: "Your name",
            border_color: None,
            trailing: InputTrailing::None,
        },
        &mut app.view.account.edit_display_name,
        t,
    )
    .enter_pressed
    {
        submit_on_enter = true;
    }

    ui.add_space(8.0);
    let username_validation = normalize_username(&app.view.account.edit_username);
    let username_hint = match &username_validation {
        Ok((clean, _)) => {
            if *clean == sess.user.username {
                FieldHint::Muted("Current @username")
            } else {
                FieldHint::Success("Valid @username")
            }
        }
        Err(err) => FieldHint::Warning(err.to_string()),
    };
    draw_field_label_row(ui, "Username", username_hint, t);
    ui.add_space(3.0);
    let trailing = match &username_validation {
        Ok((clean, _)) if *clean != sess.user.username => {
            InputTrailing::GreenCheck("Valid username format")
        }
        _ => InputTrailing::None,
    };
    if draw_text_input(
        ui,
        InputFieldSpec {
            id_salt: "profile_edit_username",
            width: ui.available_width(),
            hint: "alex_notes",
            border_color: None,
            trailing,
        },
        &mut app.view.account.edit_username,
        t,
    )
    .enter_pressed
    {
        submit_on_enter = true;
    }

    let clean_display = sanitize_display_name(
        &app.view.account.edit_display_name,
        username_validation
            .as_ref()
            .map_or(sess.user.username.as_str(), |(u, _)| u.as_str()),
    );
    let username_changed = username_validation
        .as_ref()
        .is_ok_and(|(u, _)| *u != sess.user.username);
    let display_changed = clean_display != sess.user.display_name;
    let dirty = username_changed || display_changed;
    let valid = username_validation.is_ok();
    let busy = app.view.account.profile_save_busy;

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if secondary_action_button(ui, "Reset", dirty && !busy, t).clicked() {
            app.view.account.sync_identity_from_session(Some(&sess));
            app.view.account.error_message = None;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let save_lbl = if busy { "Saving..." } else { "Save Profile" };
            if primary_action_button(ui, save_lbl, dirty && valid && !busy, t).clicked()
                || (submit_on_enter && dirty && valid && !busy)
            {
                submit_update_profile(app, ui.ctx());
            }
        });
    });
}

pub(crate) fn render_account_email_card(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    ui.horizontal(|ui| {
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::Link, t.accent);
        ui.label(
            egui::RichText::new("Email Address")
                .size(13.5)
                .strong()
                .color(t.text),
        );
    });
    ui.add_space(3.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("Email used to sign in across devices.")
                .size(11.0)
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(8.0);
    render_account_email_row(app, ui, summary, t);
}

pub(crate) fn render_change_password_card(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.horizontal(|ui| {
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, Icon::Reset, t.accent);
        ui.label(
            egui::RichText::new("Change Password")
                .size(13.5)
                .strong()
                .color(t.text),
        );
    });
    ui.add_space(3.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("Update the password used for this server account.")
                .size(11.0)
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(10.0);

    let mut submit_on_enter = false;
    draw_field_label_row(
        ui,
        "Current Password",
        FieldHint::Muted("Verify your identity"),
        t,
    );
    ui.add_space(3.0);
    if draw_password_input(
        ui,
        "profile_change_pw_current",
        ui.available_width(),
        &mut app.view.account.current_password_draft,
        &mut app.view.account.show_change_pw_current,
        t,
    )
    .enter_pressed
    {
        submit_on_enter = true;
    }

    ui.add_space(8.0);
    let new_chars = app.view.account.new_password_draft.chars().count();
    let new_hint = if new_chars == 0 {
        FieldHint::Muted("Minimum 8 characters")
    } else if new_chars >= MIN_PASSWORD_LEN {
        FieldHint::Success("Ready (8+ characters)")
    } else {
        FieldHint::Warning(format!("{new_chars}/{MIN_PASSWORD_LEN} characters"))
    };
    draw_field_label_row(ui, "New Password", new_hint, t);
    ui.add_space(3.0);
    if draw_password_input(
        ui,
        "profile_change_pw_new",
        ui.available_width(),
        &mut app.view.account.new_password_draft,
        &mut app.view.account.show_change_pw_new,
        t,
    )
    .enter_pressed
    {
        submit_on_enter = true;
    }

    ui.add_space(8.0);
    let confirm_hint = if app.view.account.confirm_new_password_draft.is_empty() {
        FieldHint::Muted("Re-enter new password")
    } else if app.view.account.new_password_draft == app.view.account.confirm_new_password_draft {
        FieldHint::Success("Passwords match")
    } else {
        FieldHint::Warning("Passwords do not match".to_string())
    };
    draw_field_label_row(ui, "Confirm New Password", confirm_hint, t);
    ui.add_space(3.0);
    if draw_password_input(
        ui,
        "profile_change_pw_confirm",
        ui.available_width(),
        &mut app.view.account.confirm_new_password_draft,
        &mut app.view.account.show_change_pw_new,
        t,
    )
    .enter_pressed
    {
        submit_on_enter = true;
    }

    let busy = app.view.account.change_password_busy;
    let ready = !app.view.account.current_password_draft.is_empty()
        && validate_password(&app.view.account.new_password_draft).is_ok()
        && app.view.account.new_password_draft == app.view.account.confirm_new_password_draft;

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let lbl = if busy {
                "Updating..."
            } else {
                "Update Password"
            };
            if primary_action_button(ui, lbl, ready && !busy, t).clicked()
                || (submit_on_enter && ready && !busy)
            {
                submit_change_password(app, ui.ctx());
            }
        });
    });
}

pub(crate) fn format_notebook_count(count: usize) -> String {
    if count == 1 {
        "1 notebook".to_string()
    } else {
        format!("{count} notebooks")
    }
}

pub(crate) fn draw_active_pill(ui: &mut egui::Ui, t: &Tokens) {
    draw_pill_badge(
        ui,
        PillStyle {
            fill: t.success_bg,
            border: t.success_border,
            fg: t.success_fg,
            icon: PillIcon::GreenCheck(t),
        },
        "Active",
    );
}

fn draw_truncated_block_title(ui: &mut egui::Ui, title: &str, max_w: f32, t: &Tokens) {
    ui.scope(|ui| {
        ui.set_max_width(max_w.max(48.0));
        ui.add(
            egui::Label::new(egui::RichText::new(title).size(13.0).strong().color(t.text))
                .truncate(),
        );
    });
}

fn draw_account_block_frame(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    active: bool,
    danger: bool,
    t: &Tokens,
    body: impl FnOnce(&mut egui::Ui),
) {
    let block_id = ui.make_persistent_id(id_salt);
    let base_fill = if danger {
        t.danger_bg
    } else if active {
        t.accent_soft
    } else {
        t.toolbar_bg
    };
    let hover_fill = if danger {
        t.danger_bg
    } else if active {
        t.accent_soft
    } else {
        t.field_bg
    };
    let base_stroke = if danger {
        t.danger_border
    } else if active {
        t.accent
    } else {
        t.border
    };
    let hover_stroke = if danger { t.danger_border } else { t.accent };

    let prev_hovered = ui.ctx().data(|d| {
        d.get_temp::<bool>(block_id.with("hovered"))
            .unwrap_or(false)
    });
    let hover_t = crate::motion::hover_factor(ui.ctx(), block_id.with("hov"), prev_hovered);
    let fill = crate::motion::lerp_color(base_fill, hover_fill, hover_t * 0.65);
    let stroke_color = crate::motion::lerp_color(base_stroke, hover_stroke, hover_t * 0.45);

    let resp = egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke_color))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            body(ui);
        });
    let now_hovered = ui.rect_contains_pointer(resp.response.rect);
    if now_hovered != prev_hovered {
        ui.ctx()
            .data_mut(|d| d.insert_temp(block_id.with("hovered"), now_hovered));
        ui.ctx().request_repaint();
    }
}

/// Builds the exact manual confirmation phrase (`I agree <local_profile>`) for local deletion.
pub(crate) fn local_profile_delete_confirmation_phrase(local_nickname: &str) -> String {
    format!("I agree {}", local_nickname.trim())
}

fn normalize_confirmation_words(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn confirmation_phrase_matches(input: &str, expected: &str) -> bool {
    normalize_confirmation_words(input).to_lowercase()
        == normalize_confirmation_words(expected).to_lowercase()
}

struct ManualAgreeInputOutcome {
    edit_id: egui::Id,
    matched: bool,
    enter_pressed: bool,
}

fn draw_manual_agree_confirmation_input(
    ui: &mut egui::Ui,
    id_salt: &str,
    expected: &str,
    input: &mut String,
    paste_blocked: &mut bool,
    t: &Tokens,
) -> ManualAgreeInputOutcome {
    ui.label(
        egui::RichText::new("Example — type this manually:")
            .size(10.5)
            .color(t.text_muted),
    );
    ui.add_space(3.0);
    egui::Frame::new()
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(expected)
                        .size(11.5)
                        .strong()
                        .monospace()
                        .color(t.text),
                )
                .selectable(false)
                .wrap(),
            );
        });
    ui.add_space(6.0);
    let matched = confirmation_phrase_matches(input, expected);
    let hint = if *paste_blocked && !matched {
        FieldHint::Warning("Copy/paste is disabled — type it manually".to_string())
    } else if input.trim().is_empty() {
        FieldHint::Muted("Type manually (no copy/paste)")
    } else if matched {
        FieldHint::Success("Confirmation matched")
    } else {
        FieldHint::Warning(format!("Must match: {expected}"))
    };
    draw_field_label_row(ui, "Manual Confirmation", hint, t);
    ui.add_space(3.0);
    let input_resp = draw_manual_confirm_input(
        ui,
        InputFieldSpec {
            id_salt,
            width: ui.available_width(),
            hint: "I agree ...",
            border_color: Some(if matched {
                t.success_border
            } else {
                t.danger_border
            }),
            trailing: if matched {
                InputTrailing::GreenCheck("Confirmation matches")
            } else {
                InputTrailing::None
            },
        },
        input,
        t,
    );
    if input_resp.paste_blocked {
        *paste_blocked = true;
        ui.ctx().request_repaint();
    } else if input_resp.outcome.changed {
        *paste_blocked = false;
    }
    ManualAgreeInputOutcome {
        edit_id: input_resp.edit_id,
        matched,
        enter_pressed: input_resp.outcome.enter_pressed,
    }
}

pub(crate) fn render_accounts_switcher_section(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) {
    let viewing_cloud = app.session.is_viewing_cloud();
    let active_local_id = app
        .session
        .active_local_profile()
        .map(|p| p.local_profile_id.as_str());
    let local_count = app
        .session
        .saved_local_profiles()
        .iter()
        .filter(|p| viewing_cloud || active_local_id != Some(p.local_profile_id.as_str()))
        .count();
    let active_server_url = if summary.signed_in {
        summary.server_url.clone()
    } else {
        app.session.preferred_server_url().to_string()
    };
    let active_user_id = app
        .session
        .active_account_session()
        .map(|s| (norm_probe_url_key(&s.server_url), s.user.user_id.clone()));
    let shortcuts: Vec<RememberedServerAccount> = app
        .session
        .remembered_server_accounts()
        .iter()
        .filter(|entry| {
            active_user_id.as_ref().is_none_or(|(au, aid)| {
                !(aid == &entry.user_id && norm_probe_url_key(&entry.server_url) == *au)
            })
        })
        .cloned()
        .collect();
    let any_server_online = if !shortcuts.is_empty() {
        shortcuts
            .iter()
            .any(|entry| is_server_url_online(app, &entry.server_url))
    } else if summary.signed_in {
        summary.connection != SyncConnectionState::Offline
            && is_server_url_online(app, &summary.server_url)
    } else {
        is_server_url_online(app, &active_server_url)
    };
    let server_col_tone = if any_server_online {
        AccountColumnTone::Server
    } else {
        AccountColumnTone::ServerOffline
    };
    let server_count = shortcuts.len();

    draw_local_vs_server_split(
        ui,
        app,
        680.0,
        24.0,
        t,
        |app, ui| {
            draw_account_column_card(ui, AccountColumnTone::Local, t, |ui| {
                ui.horizontal(|ui| {
                    draw_column_header_badge(ui, AccountColumnTone::Local, Icon::Account, t);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("Local Accounts ({local_count})"))
                            .size(13.0)
                            .strong()
                            .color(t.text),
                    );
                });
                ui.add_space(6.0);
                draw_hairline(ui, t);
                ui.add_space(8.0);
                render_local_profiles_manager(app, ui, false, true, t);
            });
        },
        |app, ui| {
            draw_account_column_card(ui, server_col_tone, t, |ui| {
                ui.horizontal(|ui| {
                    draw_column_header_badge(ui, server_col_tone, Icon::Server, t);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("Server Accounts ({server_count})"))
                            .size(13.0)
                            .strong()
                            .color(t.text),
                    );
                });
                ui.add_space(6.0);
                draw_hairline(ui, t);
                ui.add_space(8.0);

                if shortcuts.is_empty() {
                    let empty_msg = if summary.signed_in {
                        "No other server accounts saved on this device."
                    } else {
                        "No server accounts saved yet."
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(empty_msg)
                                .size(11.5)
                                .color(t.text_muted),
                        )
                        .wrap(),
                    );
                } else {
                    for (idx, entry) in shortcuts.iter().enumerate() {
                        if idx > 0 {
                            ui.add_space(8.0);
                        }
                        render_remembered_server_account_block(app, ui, entry, t);
                    }
                }
            });
        },
    );

    render_unsynced_switch_confirmation(app, ui, t);

    if app.session.is_sync_persistence_enabled() {
        ui.add_space(10.0);
        draw_section_card(ui, t, |ui| {
            render_startup_picker_switch(app, ui, t, true);
        });
    }
}

pub(crate) fn render_expired_session_reauth_box(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    acct: &papercraft_sync::ActiveAccountSession,
    cloud_pending: usize,
    pw_id_salt: &str,
    t: &Tokens,
) {
    let queue_note = match cloud_pending {
        0 => "Your session expired. Enter your password to resume cloud sync.".to_string(),
        1 => "Your session expired. 1 offline change is queued locally and will sync as soon as you sign in.".to_string(),
        n => format!("Your session expired. {n} offline changes are queued locally and will sync as soon as you sign in."),
    };
    let server_offline = matches!(
        app.view.account.probe_for(&acct.server_url),
        ServerProbeState::Failed { .. }
    );
    let busy = app.view.account.auth_busy;

    egui::Frame::new()
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, t.danger_border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::AlertCircle, t.danger_fg);
                ui.label(
                    egui::RichText::new("Session Expired — Sign In Again")
                        .size(12.0)
                        .strong()
                        .color(t.danger_fg),
                );
            });
            ui.add_space(3.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(queue_note)
                        .size(11.0)
                        .color(t.text_muted),
                )
                .wrap(),
            );
            if server_offline {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Last check could not reach server — click Sign In & Sync to retry. Your edits stay safely saved locally.",
                    )
                    .size(10.5)
                    .color(t.warning_fg),
                );
            }
            ui.add_space(6.0);
            let mut submit_now = false;
            let avail_w = ui.available_width();
            let can_submit = !busy && !app.view.account.reauth_password.is_empty();
            let cta = if busy { "Signing In..." } else { "Sign In & Sync" };
            if avail_w < 275.0 {
                let pw_resp = draw_password_input(
                    ui,
                    pw_id_salt,
                    avail_w,
                    &mut app.view.account.reauth_password,
                    &mut app.view.account.reauth_show_password,
                    t,
                );
                if pw_resp.changed {
                    app.view.account.reauth_error = None;
                }
                if ui.is_enabled() && pw_resp.enter_pressed && !busy {
                    submit_now = true;
                }
                ui.add_space(5.0);
                if primary_action_button(ui, cta, can_submit, t).clicked() && ui.is_enabled() {
                    submit_now = true;
                }
            } else {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let btn_w = 108.0_f32;
                    let input_w = (avail_w - btn_w - 6.0).max(110.0);
                    let pw_resp = draw_password_input(
                        ui,
                        pw_id_salt,
                        input_w,
                        &mut app.view.account.reauth_password,
                        &mut app.view.account.reauth_show_password,
                        t,
                    );
                    if pw_resp.changed {
                        app.view.account.reauth_error = None;
                    }
                    if ui.is_enabled() && pw_resp.enter_pressed && !busy {
                        submit_now = true;
                    }
                    if primary_action_button(ui, cta, can_submit, t).clicked() && ui.is_enabled() {
                        submit_now = true;
                    }
                });
            }
            if let Some(ref err) = app.view.account.reauth_error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(err).size(11.0).color(t.danger_fg));
            }
            if ui.is_enabled() && submit_now && !busy {
                submit_active_session_reauth(app, ui.ctx(), acct);
            }
        });
}

fn submit_active_session_reauth(
    app: &mut PaperCraftApp,
    ctx: &egui::Context,
    acct: &papercraft_sync::ActiveAccountSession,
) {
    let password = app.view.account.reauth_password.clone();
    if password.is_empty() {
        app.view.account.reauth_error = Some("Enter your password to resume sync.".to_string());
        return;
    }
    let Some(identifier) = acct
        .user
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        app.view.account.reauth_error =
            Some("Account session is missing a sign-in email address.".to_string());
        return;
    };
    let norm_url = norm_probe_url_key(&acct.server_url);
    app.view.account.server_url = norm_url.clone();
    let _ = app.session.set_server_url_preference(&norm_url);
    app.view.account.auth_busy = true;
    app.view.account.reauth_pending = true;
    app.view.account.reauth_error = None;
    app.view.account.error_message = None;
    app.view.account.status_message = Some(format!(
        "Renewing session for {}...",
        acct.user.primary_label()
    ));

    let cmd = SyncWorkerCommand::Login {
        session_generation: app.sync_worker.current_session_generation(),
        server_url: norm_url,
        identifier,
        password,
        device_name: app.session.device_name().to_string(),
    };
    if !crate::sync_worker::dispatch_command(app, Some(ctx), cmd) {
        app.view.account.auth_busy = false;
        app.view.account.reauth_pending = false;
        app.view.account.status_message = None;
        app.view.account.reauth_error =
            Some("Background network worker is unavailable.".to_string());
    }
}

pub(crate) fn render_forget_server_account_confirmation(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    server_url: &str,
    user_id: &str,
    label: &str,
    pending_count: usize,
    t: &Tokens,
) {
    egui::Frame::new()
        .fill(t.field_bg)
        .stroke(Stroke::new(
            1.0,
            if pending_count > 0 {
                t.danger_border
            } else {
                t.warning_fg
            },
        ))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(
                    ui.painter(),
                    ir,
                    Icon::AlertCircle,
                    if pending_count > 0 {
                        t.danger_fg
                    } else {
                        t.warning_fg
                    },
                );
                ui.label(
                    egui::RichText::new(format!("Forget '{label}' on This Device?"))
                        .size(12.0)
                        .strong()
                        .color(if pending_count > 0 {
                            t.danger_fg
                        } else {
                            t.text
                        }),
                );
            });
            ui.add_space(4.0);
            if pending_count > 0 {
                let change_word = if pending_count == 1 {
                    "1 pending local change has"
                } else {
                    "Pending local changes have"
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!(
                            "Warning: {change_word} not synced to the server yet ({pending_count} pending). Forgetting this account removes the shortcut and permanently deletes its local offline cache on this device for privacy. Data already synced to the server stays safe."
                        ))
                        .size(11.0)
                        .color(t.danger_fg),
                    )
                    .wrap(),
                );
            } else {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Removes this shortcut and deletes ONLY the local offline cache stored for this account on this device. All data already saved on the server remains intact.",
                        )
                        .size(11.0)
                        .color(t.text_muted),
                    )
                    .wrap(),
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if secondary_action_button(ui, "Cancel", true, t).clicked() {
                    app.view.account.clear_pending_forget_server();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if danger_action_button(ui, "Confirm Forget", true, t).clicked()
                        && let Err(err) = complete_forget_server_account(
                            app,
                            ui.ctx(),
                            server_url,
                            user_id,
                            label,
                        )
                    {
                        app.view.account.error_message = Some(err);
                    }
                });
            });
        });
}

struct InactiveAccountRowSpec<'a> {
    icon: Icon,
    icon_color: Color32,
    name: &'a str,
    hover_tip: Option<&'a str>,
    primary_label: &'a str,
    primary_enabled: bool,
    primary_tip: &'a str,
    remove_label: Option<&'a str>,
    remove_enabled: bool,
    remove_tip: &'a str,
    show_actions: bool,
}

#[derive(Default)]
struct InactiveAccountRowOutcome {
    primary_clicked: bool,
    remove_clicked: bool,
}

fn estimate_action_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let text_w = ui
        .painter()
        .layout_no_wrap(
            label.to_string(),
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        )
        .size()
        .x;
    text_w + ui.spacing().button_padding.x * 2.0 + 4.0
}

fn draw_inactive_account_actions(
    ui: &mut egui::Ui,
    spec: &InactiveAccountRowSpec<'_>,
    outcome: &mut InactiveAccountRowOutcome,
    t: &Tokens,
) {
    if primary_action_button(ui, spec.primary_label, spec.primary_enabled, t)
        .on_hover_text(spec.primary_tip)
        .clicked()
    {
        outcome.primary_clicked = true;
    }
    if let Some(rm_lbl) = spec.remove_label
        && hollow_secondary_button(ui, rm_lbl, spec.remove_enabled, t)
            .on_hover_text(spec.remove_tip)
            .clicked()
    {
        outcome.remove_clicked = true;
    }
}

fn draw_inactive_account_row(
    ui: &mut egui::Ui,
    spec: InactiveAccountRowSpec<'_>,
    t: &Tokens,
) -> InactiveAccountRowOutcome {
    let mut outcome = InactiveAccountRowOutcome::default();
    let avail_w = ui.available_width();
    let primary_w = estimate_action_button_width(ui, spec.primary_label);
    let remove_w = spec
        .remove_label
        .map_or(0.0, |lbl| estimate_action_button_width(ui, lbl) + 6.0);
    let buttons_w = if spec.show_actions {
        primary_w + remove_w + 8.0
    } else {
        0.0
    };
    let stacked = spec.show_actions && avail_w < 210.0;
    let name_max_w = if stacked || !spec.show_actions {
        (avail_w - 24.0).max(48.0)
    } else {
        (avail_w - buttons_w - 24.0).max(48.0)
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (ir, icon_resp) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
        paint_icon(ui.painter(), ir, spec.icon, spec.icon_color);
        if let Some(tip) = spec.hover_tip {
            let _ = icon_resp.on_hover_text(tip);
        }
        draw_truncated_block_title(ui, spec.name, name_max_w, t);

        if spec.show_actions && !stacked {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                draw_inactive_account_actions(ui, &spec, &mut outcome, t);
            });
        }
    });

    if stacked {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            draw_inactive_account_actions(ui, &spec, &mut outcome, t);
        });
    }

    outcome
}

fn render_remembered_server_account_block(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    entry: &RememberedServerAccount,
    t: &Tokens,
) {
    let norm_url = norm_probe_url_key(&entry.server_url);
    let probe = app.view.account.probe_for(&entry.server_url);
    let server_offline = matches!(probe, ServerProbeState::Failed { .. });
    let cached_notebooks = app
        .session
        .server_account_notebook_count(&entry.server_url, &entry.user_id);
    let is_cached = cached_notebooks > 0
        || app
            .session
            .has_cached_server_account(&entry.server_url, &entry.user_id);
    let pending = app
        .session
        .server_account_pending_changes_count(&entry.server_url, &entry.user_id);
    let entry_key = (norm_url, entry.user_id.clone());
    let quick_open = app.view.account.quick_login_key.as_ref() == Some(&entry_key);
    let confirming_forget = app.view.account.pending_forget_server_key.as_ref() == Some(&entry_key);
    let primary = entry.primary_label();
    let subtitle_id = entry
        .login_email()
        .map(|e| {
            if entry.username.is_empty() {
                e.to_string()
            } else {
                format!("{e} (@{})", entry.username)
            }
        })
        .unwrap_or_else(|| format!("@{}", entry.username));
    let hover_tip = format!("{subtitle_id} · {}", entry.server_url);

    draw_account_block_frame(
        ui,
        ("rem_srv_block", &entry.server_url, &entry.user_id),
        quick_open,
        confirming_forget,
        t,
        |ui| {
            let (acct_icon, icon_fg, _, _) =
                account_avatar_icon_and_colors(true, !server_offline, t);
            let switch_tip = if server_offline {
                "Server unreachable — click to enter password and retry"
            } else if is_cached {
                "Enter your password to switch to this account"
            } else {
                "Sign in and switch to this server account"
            };
            let outcome = draw_inactive_account_row(
                ui,
                InactiveAccountRowSpec {
                    icon: acct_icon,
                    icon_color: icon_fg,
                    name: &primary,
                    hover_tip: Some(&hover_tip),
                    primary_label: "Switch",
                    primary_enabled: !confirming_forget && !app.view.account.auth_busy,
                    primary_tip: switch_tip,
                    remove_label: Some("Forget"),
                    remove_enabled: true,
                    remove_tip: "Remove this shortcut and delete ONLY its local cache on this device",
                    show_actions: !confirming_forget,
                },
                t,
            );
            if outcome.primary_clicked {
                app.view.account.clear_pending_forget_server();
                app.view.account.clear_pending_unsynced_switch();
                if quick_open {
                    app.view.account.clear_quick_login_draft();
                } else {
                    app.view.account.quick_login_key = Some(entry_key.clone());
                    app.view.account.quick_login_password.clear();
                    app.view.account.quick_login_error = None;
                    app.view.account.quick_login_focus_pending = true;
                    app.view.account.server_url = entry.server_url.clone();
                }
            }
            if outcome.remove_clicked {
                app.view.account.pending_forget_server_key = Some(entry_key.clone());
                app.view.account.clear_quick_login_draft();
                app.view.account.clear_pending_delete_profile();
            }

            crate::motion::animated_collapse_with_duration(
                ui,
                ui.make_persistent_id(("rem_forget_confirm", &entry.server_url, &entry.user_id)),
                confirming_forget,
                152.0,
                crate::motion::MICRO_DURATION_S,
                |ui| {
                    ui.add_space(4.0);
                    render_forget_server_account_confirmation(
                        app,
                        ui,
                        &entry.server_url,
                        &entry.user_id,
                        &primary,
                        pending,
                        t,
                    );
                },
            );
            crate::motion::animated_collapse_with_duration(
                ui,
                ui.make_persistent_id(("rem_quick_login", &entry.server_url, &entry.user_id)),
                quick_open && !confirming_forget,
                if server_offline { 116.0 } else { 92.0 },
                crate::motion::MICRO_DURATION_S,
                |ui| {
                    if server_offline {
                        ui.add_space(4.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "Last check could not reach server — enter your password to retry.",
                                )
                                .size(10.5)
                                .color(t.warning_fg),
                            )
                            .wrap(),
                        );
                    }
                    render_quick_login_drawer(app, ui, entry, is_cached, "quick_login_pw", t);
                },
            );
        },
    );
}

fn submit_quick_server_login(
    app: &mut PaperCraftApp,
    ctx: &egui::Context,
    entry: &RememberedServerAccount,
) {
    if app.view.account.quick_login_password.is_empty() {
        app.view.account.quick_login_error = Some("Enter your password.".to_string());
        return;
    }
    if entry.login_email().is_none() {
        app.view.account.quick_login_error = Some(
            "Account shortcut is missing a sign-in email address. Use Sign In with Another Account instead."
                .to_string(),
        );
        return;
    }
    if let Some((outgoing_label, pending_count)) = should_warn_unsynced_cloud_switch(
        app,
        &entry.server_url,
        Some(&entry.user_id),
        entry.login_email(),
    ) {
        app.view.account.pending_unsynced_switch = Some(PendingUnsyncedSwitch {
            outgoing_label,
            pending_count,
            target_label: entry.primary_label(),
            target: PendingUnsyncedSwitchTarget::QuickLogin {
                server_url: entry.server_url.clone(),
                user_id: entry.user_id.clone(),
            },
        });
        return;
    }
    dispatch_quick_login_now(app, ctx, &entry.server_url, &entry.user_id);
}

fn dispatch_quick_login_now(
    app: &mut PaperCraftApp,
    ctx: &egui::Context,
    server_url: &str,
    user_id: &str,
) {
    let Some(entry) = app
        .session
        .remembered_server_accounts()
        .iter()
        .find(|e| e.user_id == user_id && urls_match(&e.server_url, server_url))
        .cloned()
    else {
        app.view.account.quick_login_error =
            Some("Saved server account shortcut was not found.".to_string());
        return;
    };
    let password = app.view.account.quick_login_password.clone();
    if password.is_empty() {
        app.view.account.quick_login_error = Some("Enter your password.".to_string());
        return;
    }
    let Some(identifier) = entry.login_email().map(str::to_string) else {
        app.view.account.quick_login_error = Some(
            "Account shortcut is missing a sign-in email address. Use Sign In with Another Account instead."
                .to_string(),
        );
        return;
    };
    let norm_url = norm_probe_url_key(&entry.server_url);
    app.view.account.server_url = norm_url.clone();
    let _ = app.session.set_server_url_preference(&norm_url);
    app.view.account.auth_busy = true;
    app.view.account.clear_pending_unsynced_switch();
    app.view.account.quick_login_error = None;
    app.view.account.error_message = None;
    app.view.account.status_message = Some(format!("Signing in as {}...", entry.primary_label()));

    let cmd = SyncWorkerCommand::Login {
        session_generation: app.sync_worker.current_session_generation(),
        server_url: norm_url,
        identifier,
        password,
        device_name: app.session.device_name().to_string(),
    };
    if !crate::sync_worker::dispatch_command(app, Some(ctx), cmd) {
        app.view.account.auth_busy = false;
        app.view.account.status_message = None;
        app.view.account.quick_login_error =
            Some("Background network worker is unavailable.".to_string());
    }
}

pub(crate) fn render_local_profiles_manager(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    show_inline_create: bool,
    exclude_active: bool,
    t: &Tokens,
) {
    let all_profiles = app.session.saved_local_profiles().to_vec();
    let active_local_id = app
        .session
        .active_local_profile()
        .map(|p| p.local_profile_id.clone())
        .unwrap_or_default();
    let viewing_cloud = app.session.is_viewing_cloud();
    let can_delete = viewing_cloud || all_profiles.len() > 1;
    let visible_profiles: Vec<_> = all_profiles
        .iter()
        .filter(|p| !(exclude_active && !viewing_cloud && p.local_profile_id == active_local_id))
        .collect();
    if let Some(ref pending_del) = app.view.account.pending_delete_profile_id
        && (!can_delete
            || !visible_profiles
                .iter()
                .any(|p| p.local_profile_id == *pending_del))
    {
        app.view.account.clear_pending_delete_profile();
    }

    if visible_profiles.is_empty() {
        let empty_msg = if exclude_active && !viewing_cloud && !all_profiles.is_empty() {
            "No other local accounts on this device."
        } else {
            "No local accounts saved yet."
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(empty_msg)
                    .size(11.5)
                    .color(t.text_muted),
            )
            .wrap(),
        );
    } else {
        for (idx, profile) in visible_profiles.into_iter().enumerate() {
            if idx > 0 {
                ui.add_space(8.0);
            }
            let is_active = !viewing_cloud && profile.local_profile_id == active_local_id;
            render_local_profile_block(app, ui, profile, is_active, can_delete, t);
        }
    }

    if !show_inline_create {
        return;
    }

    ui.add_space(8.0);
    draw_hairline(ui, t);
    ui.add_space(8.0);
    let create_w = 88.0_f32;
    let gap = 8.0_f32;
    let input_w = (ui.available_width() - create_w - gap).max(120.0);
    let mut create_submitted = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let resp = draw_text_input(
            ui,
            InputFieldSpec {
                id_salt: "acct_new_local_profile_name",
                width: input_w,
                hint: "New local profile nickname...",
                border_color: None,
                trailing: InputTrailing::None,
            },
            &mut app.view.account.new_profile_name,
            t,
        );
        if resp.enter_pressed {
            create_submitted = true;
        }
        let has_name = !app.view.account.new_profile_name.trim().is_empty();
        if fixed_secondary_button(ui, "Add Profile", create_w, has_name, t).clicked()
            || (create_submitted && has_name)
        {
            let name = app.view.account.new_profile_name.trim().to_string();
            let switch_now = !app.session.is_signed_in();
            let res = if switch_now {
                create_local_account_and_switch(app, ui.ctx(), &name)
            } else {
                crate::canvas::commit_pending_text(app);
                app.session
                    .execute(
                        "profile.create",
                        &json!({ "nickname": name, "switch_now": false }),
                    )
                    .map(|_| {
                        app.flush_web_storage(true);
                    })
                    .map_err(|e| e.to_string())
            };
            if let Err(err) = res {
                app.view.account.error_message = Some(err);
            } else {
                app.view.account.new_profile_name.clear();
                app.view.account.error_message = None;
            }
        }
    });
}

fn execute_local_profile_delete(
    app: &mut PaperCraftApp,
    ui: &egui::Ui,
    profile: &papercraft_sync::LocalProfile,
) {
    crate::canvas::commit_pending_text(app);
    let was_active = !app.session.is_viewing_cloud()
        && app
            .session
            .active_local_profile()
            .is_some_and(|p| p.local_profile_id == profile.local_profile_id);
    if was_active {
        prepare_workspace_switch(app);
    }
    let deleted_name = profile.nickname.clone();
    match app.session.execute(
        "profile.delete",
        &json!({ "profile_id": profile.local_profile_id }),
    ) {
        Ok(_) => {
            app.view.account.clear_pending_delete_profile();
            app.view.account.error_message = None;
            app.view.account.status_message =
                Some(format!("Deleted local profile '{deleted_name}'."));
            if was_active {
                finish_workspace_switch(app, ui.ctx());
            } else {
                app.flush_web_storage(true);
            }
            crate::sync_worker::maybe_report_local_discovery(
                app,
                ui.ctx(),
                ui.ctx().input(|i| i.time * 1000.0),
            );
        }
        Err(err) => {
            app.view.account.status_message = None;
            app.view.account.error_message = Some(err.to_string());
        }
    }
}

fn render_local_profile_delete_confirmation(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    profile: &papercraft_sync::LocalProfile,
    is_active: bool,
    nb_count: usize,
    t: &Tokens,
) {
    let expected = local_profile_delete_confirmation_phrase(&profile.nickname);
    let count_label = format_notebook_count(nb_count);
    let mut confirm_delete = false;
    let mut cancel_delete = false;
    let active_note = if is_active {
        " PaperCraft will switch to another local profile."
    } else {
        ""
    };

    let mut matched = false;
    let confirm_resp = egui::Frame::new()
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, t.danger_border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::AlertCircle, t.danger_fg);
                ui.label(
                    egui::RichText::new("Delete Local Profile?")
                        .size(12.0)
                        .strong()
                        .color(t.danger_fg),
                );
            });
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "Irreversible action: permanently deletes local profile '{}' ({count_label}) and all offline data on this device.{active_note} Copy and paste is disabled.",
                        profile.nickname
                    ))
                    .size(11.0)
                    .color(t.text_muted),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            let manual = draw_manual_agree_confirmation_input(
                ui,
                "acct_delete_profile_confirm_input",
                &expected,
                &mut app.view.account.delete_profile_confirm_input,
                &mut app.view.account.delete_profile_paste_blocked,
                t,
            );
            matched = manual.matched;
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if secondary_action_button(ui, "Cancel", true, t).clicked() {
                    cancel_delete = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if danger_action_button(ui, "Delete Profile", matched, t).clicked()
                        || (manual.enter_pressed && matched)
                    {
                        confirm_delete = true;
                    }
                });
            });
            manual.edit_id
        });

    if ui.is_enabled()
        && std::mem::take(&mut app.view.account.pending_delete_profile_scroll_pending)
    {
        ui.scroll_to_rect(confirm_resp.response.rect, Some(egui::Align::Center));
        ui.memory_mut(|m| m.request_focus(confirm_resp.inner));
    }

    if ui.is_enabled() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel_delete = true;
    }

    if ui.is_enabled() && confirm_delete && matched {
        execute_local_profile_delete(app, ui, profile);
    } else if ui.is_enabled() && cancel_delete {
        app.view.account.clear_pending_delete_profile();
    }
}

fn render_local_profile_rename_row(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    profile_id: &str,
    t: &Tokens,
) {
    let save_w = 56.0_f32;
    let gap = 6.0_f32;
    let avail_w = ui.available_width();
    let commit_rename = |app: &mut PaperCraftApp| {
        let name = app.view.account.rename_profile_draft.trim().to_string();
        if let Err(err) = app.session.execute(
            "profile.rename",
            &json!({ "profile_id": profile_id, "nickname": name }),
        ) {
            app.view.account.error_message = Some(err.to_string());
        } else {
            app.view.account.renaming_profile_id = None;
            app.view.account.error_message = None;
        }
    };
    if avail_w < 245.0 {
        let resp = draw_text_input(
            ui,
            InputFieldSpec {
                id_salt: "acct_rename_profile_input",
                width: avail_w,
                hint: "Profile nickname",
                border_color: None,
                trailing: InputTrailing::None,
            },
            &mut app.view.account.rename_profile_draft,
            t,
        );
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            if fixed_secondary_button(ui, "Save", save_w, true, t).clicked() || resp.enter_pressed {
                commit_rename(app);
            }
            if fixed_secondary_button(ui, "Cancel", save_w, true, t).clicked() {
                app.view.account.renaming_profile_id = None;
            }
        });
    } else {
        ui.horizontal(|ui| {
            let input_w = (avail_w - save_w * 2.0 - gap * 2.0).max(90.0);
            let resp = draw_text_input(
                ui,
                InputFieldSpec {
                    id_salt: "acct_rename_profile_input",
                    width: input_w,
                    hint: "Profile nickname",
                    border_color: None,
                    trailing: InputTrailing::None,
                },
                &mut app.view.account.rename_profile_draft,
                t,
            );
            if fixed_secondary_button(ui, "Save", save_w, true, t).clicked() || resp.enter_pressed {
                commit_rename(app);
            }
            if fixed_secondary_button(ui, "Cancel", save_w, true, t).clicked() {
                app.view.account.renaming_profile_id = None;
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn render_active_local_profile_contents(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    profile: &papercraft_sync::LocalProfile,
    nb_count: usize,
    count_label: &str,
    server_online: bool,
    is_confirming_delete: bool,
    can_delete: bool,
    t: &Tokens,
) {
    let (acct_icon, icon_fg, _, _) = account_icon_and_colors(false, false, t);
    ui.horizontal(|ui| {
        let name_max_w = (ui.available_width() - 108.0).max(48.0);
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
        paint_icon(ui.painter(), ir, acct_icon, icon_fg);
        draw_truncated_block_title(ui, &profile.nickname, name_max_w, t);

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            draw_active_pill(ui, t);
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
        draw_account_type_badge(ui, false, false, t);
        let count_tip = if server_online {
            "Offline on this device · Ready to Sync"
        } else {
            "Offline on this device"
        };
        ui.label(
            egui::RichText::new(count_label)
                .size(11.0)
                .color(t.text_muted),
        )
        .on_hover_text(count_tip);
    });

    if !is_confirming_delete {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
            let sync_tip = if server_online {
                "Upload this local account to the server while keeping an Offline-First local copy"
            } else {
                "Upload this local account to a self-hosted server"
            };
            if fixed_secondary_button(ui, "Sync", 52.0, true, t)
                .on_hover_text(sync_tip)
                .clicked()
            {
                app.view.account.convert_source_profile_id = profile.local_profile_id.clone();
                app.view.account.convert_source_nickname = profile.nickname.clone();
                app.view.account.convert_source_notebook_count = nb_count;
                app.view.account.convert_email.clear();
                app.view.account.convert_password.clear();
                app.view.account.convert_show_password = false;
                crate::modal::open(app, crate::modal::ActiveModal::ConvertLocalAccount);
            }
            if fixed_secondary_button(ui, "Rename", 60.0, true, t).clicked() {
                app.view.account.renaming_profile_id = Some(profile.local_profile_id.clone());
                app.view.account.rename_profile_draft = profile.nickname.clone();
                app.view.account.clear_pending_delete_profile();
            }
            if can_delete && fixed_secondary_button(ui, "Delete", 56.0, true, t).clicked() {
                app.view.account.pending_delete_profile_id = Some(profile.local_profile_id.clone());
                app.view.account.delete_profile_confirm_input.clear();
                app.view.account.pending_delete_profile_scroll_pending = true;
                app.view.account.delete_profile_paste_blocked = false;
                app.view.account.renaming_profile_id = None;
                app.view.account.error_message = None;
            }
        });
    }
}

fn render_local_profile_block(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    profile: &papercraft_sync::LocalProfile,
    is_active: bool,
    can_delete: bool,
    t: &Tokens,
) {
    let is_renaming =
        app.view.account.renaming_profile_id.as_deref() == Some(&profile.local_profile_id);
    let is_confirming_delete = !is_renaming
        && can_delete
        && app.view.account.pending_delete_profile_id.as_deref() == Some(&profile.local_profile_id);
    let nb_count = app
        .session
        .local_profile_notebook_count(&profile.local_profile_id);
    let count_label = format_notebook_count(nb_count);
    let server_url = app.session.preferred_server_url().to_string();
    let server_online = app.view.account.probe_for(&server_url).is_connected();

    draw_account_block_frame(
        ui,
        ("local_prof_block", &profile.local_profile_id),
        is_active,
        is_confirming_delete,
        t,
        |ui| {
            if is_renaming {
                render_local_profile_rename_row(app, ui, &profile.local_profile_id, t);
                return;
            }

            if !is_active {
                let (acct_icon, icon_fg, _, _) = account_icon_and_colors(false, false, t);
                let switch_tip = format!("Switch to '{}'", profile.nickname);
                let outcome = draw_inactive_account_row(
                    ui,
                    InactiveAccountRowSpec {
                        icon: acct_icon,
                        icon_color: icon_fg,
                        name: &profile.nickname,
                        hover_tip: None,
                        primary_label: "Switch",
                        primary_enabled: true,
                        primary_tip: &switch_tip,
                        remove_label: can_delete.then_some("Delete"),
                        remove_enabled: true,
                        remove_tip: "Delete this local account from this device",
                        show_actions: !is_confirming_delete,
                    },
                    t,
                );
                if outcome.primary_clicked {
                    app.view.account.clear_pending_delete_profile();
                    crate::canvas::commit_pending_text(app);
                    if let Err(err) =
                        switch_to_local_account_clean(app, ui.ctx(), &profile.local_profile_id)
                    {
                        app.view.account.error_message = Some(err);
                    } else {
                        app.view.account.error_message = None;
                    }
                }
                if outcome.remove_clicked {
                    app.view.account.pending_delete_profile_id =
                        Some(profile.local_profile_id.clone());
                    app.view.account.delete_profile_confirm_input.clear();
                    app.view.account.pending_delete_profile_scroll_pending = true;
                    app.view.account.delete_profile_paste_blocked = false;
                    app.view.account.renaming_profile_id = None;
                    app.view.account.error_message = None;
                }
            } else {
                render_active_local_profile_contents(
                    app,
                    ui,
                    profile,
                    nb_count,
                    &count_label,
                    server_online,
                    is_confirming_delete,
                    can_delete,
                    t,
                );
            }
            crate::motion::animated_collapse_with_duration(
                ui,
                ui.make_persistent_id(("acct_local_delete_confirm", &profile.local_profile_id)),
                is_confirming_delete,
                224.0,
                crate::motion::MICRO_DURATION_S,
                |ui| {
                    ui.add_space(4.0);
                    render_local_profile_delete_confirmation(
                        app, ui, profile, is_active, nb_count, t,
                    );
                },
            );
        },
    );
}

pub(crate) fn render_signed_in_server_card(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    summary: &SyncStatusSummary,
    t: &Tokens,
) -> bool {
    let mut test_on_enter = false;
    let probe = app.view.account.probe.clone();
    let preferred_url = app.session.preferred_server_url().to_string();
    let url_matches_saved = urls_match(&app.view.account.server_url, &preferred_url);
    let url_matches_active = urls_match(&app.view.account.server_url, &summary.server_url);
    let synced_active = url_matches_active && summary.connection == SyncConnectionState::Synced;

    let server_online = synced_active
        || (summary.connection != SyncConnectionState::Offline
            && !matches!(probe, ServerProbeState::Failed { .. }));
    let (acct_icon, icon_color, _, _) = account_icon_and_colors(true, server_online, t);

    ui.horizontal(|ui| {
        let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
        paint_icon(ui.painter(), ir, acct_icon, icon_color);
        ui.label(
            egui::RichText::new("Server Address & Connection")
                .size(13.0)
                .strong()
                .color(t.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if matches!(probe, ServerProbeState::Untested) && synced_active {
                draw_pill_badge(
                    ui,
                    PillStyle {
                        fill: t.success_bg,
                        border: t.success_border,
                        fg: t.success_fg,
                        icon: PillIcon::GreenCheck(t),
                    },
                    "Connected",
                );
            } else if !matches!(probe, ServerProbeState::Untested) {
                draw_probe_status_pill(ui, &probe, t);
            }
        });
    });
    ui.add_space(2.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(format!("Active server: {}", summary.server_url))
                .size(11.0)
                .monospace()
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(2.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new("Default server URL for connection tests and new logins.")
                .size(11.0)
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(8.0);

    let (border_override, trailing) = match &probe {
        ServerProbeState::Connected { .. } => (
            Some(t.success_border),
            InputTrailing::GreenCheck("Server connection verified"),
        ),
        ServerProbeState::Failed { .. } => (
            Some(t.danger_border),
            InputTrailing::Icon(Icon::AlertCircle, t.danger_fg, "Server connection failed"),
        ),
        ServerProbeState::Checking { .. } => (
            None,
            InputTrailing::Icon(Icon::Sync, t.accent, "Checking connection..."),
        ),
        ServerProbeState::Untested if synced_active => (
            Some(t.success_border),
            InputTrailing::GreenCheck("Active sync server"),
        ),
        ServerProbeState::Untested => (None, InputTrailing::None),
    };

    let testing = app.view.account.is_testing();
    let test_w = 72.0_f32;
    let apply_w = 104.0_f32;
    let gap = 6.0_f32;
    let avail_w = ui.available_width();
    let test_lbl = if testing { "..." } else { "Test" };
    let can_apply = !app.view.account.server_url.trim().is_empty() && !url_matches_saved;
    let mut clicked_test = false;
    let mut clicked_save = false;

    if avail_w < 345.0 {
        let url_resp = draw_text_input(
            ui,
            InputFieldSpec {
                id_salt: "acct_server_url_signed_in",
                width: avail_w,
                hint: DEFAULT_SERVER_URL,
                border_color: border_override,
                trailing,
            },
            &mut app.view.account.server_url,
            t,
        );
        if url_resp.changed {
            app.view.account.invalidate_probe_if_url_changed();
            app.view.account.error_message = None;
        }
        if url_resp.enter_pressed {
            test_on_enter = true;
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, 6.0);
            if fixed_secondary_button(ui, test_lbl, test_w, !testing, t)
                .on_hover_text("Test server connection (Enter)")
                .clicked()
            {
                clicked_test = true;
            }
            if fixed_secondary_button(ui, "Save Default", apply_w, can_apply, t)
                .on_hover_text("Save this default server address for new logins")
                .clicked()
            {
                clicked_save = true;
            }
        });
    } else {
        let input_w = (avail_w - test_w - apply_w - gap * 2.0).max(120.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let url_resp = draw_text_input(
                ui,
                InputFieldSpec {
                    id_salt: "acct_server_url_signed_in",
                    width: input_w,
                    hint: DEFAULT_SERVER_URL,
                    border_color: border_override,
                    trailing,
                },
                &mut app.view.account.server_url,
                t,
            );
            if url_resp.changed {
                app.view.account.invalidate_probe_if_url_changed();
                app.view.account.error_message = None;
            }
            if url_resp.enter_pressed {
                test_on_enter = true;
            }

            if fixed_secondary_button(ui, test_lbl, test_w, !testing, t)
                .on_hover_text("Test server connection (Enter)")
                .clicked()
            {
                clicked_test = true;
            }
            if fixed_secondary_button(ui, "Save Default", apply_w, can_apply, t)
                .on_hover_text("Save this default server address for new logins")
                .clicked()
            {
                clicked_save = true;
            }
        });
    }
    if clicked_test {
        trigger_test_connection(app, ui.ctx());
    }
    if clicked_save {
        match app
            .session
            .set_server_url_preference(&app.view.account.server_url)
        {
            Ok(norm) => {
                app.view.account.server_url = norm.clone();
                app.view.account.invalidate_probe_if_url_changed();
                app.view.account.error_message = None;
                app.view.account.status_message = Some(format!("Server URL updated to {norm}."));
                app.flush_web_storage(true);
            }
            Err(err) => {
                app.view.account.status_message = None;
                app.view.account.error_message = Some(err);
            }
        }
    }

    if !matches!(probe, ServerProbeState::Untested) {
        ui.add_space(7.0);
        render_probe_feedback_row(app, ui, &probe, false, true, t);
    }
    ui.add_space(7.0);
    draw_server_download_link_row(app, ui, t);

    test_on_enter
}

fn render_signed_in_footer(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    let mut should_close = false;
    ui.horizontal(|ui| {
        if danger_action_button(ui, "Sign Out", true, t).clicked() {
            complete_sign_out(app, ui.ctx());
        }
        if secondary_action_button(ui, "Switch Account", true, t).clicked() {
            crate::modal::open(app, crate::modal::ActiveModal::SelectLocalAccount);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let in_flight = app.session.is_sync_in_flight();
            let sync_lbl = if in_flight { "Syncing..." } else { "Sync Now" };
            let can_sync = !in_flight && app.session.is_viewing_cloud();
            if primary_action_button(ui, sync_lbl, can_sync, t).clicked() {
                app.view.account.error_message = None;
                app.view.account.status_message = None;
                trigger_manual_sync(app, ui.ctx());
            }
            if secondary_action_button(ui, "Close", true, t).clicked() {
                should_close = true;
            }
        });
    });
    should_close
}

fn draw_auth_mode_tabs(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let options = [
        SwitchOption::new(
            None,
            Some("Sign In"),
            "Sign in to an existing synced account",
            true,
        ),
        SwitchOption::new(
            None,
            Some("Create Account"),
            "Register a new synced account with a unique email",
            true,
        ),
    ];
    let active = match app.view.account.tab {
        AuthTab::SignIn => 0,
        AuthTab::CreateAccount => 1,
    };
    let (clicked, _) = segmented_switch(
        ui,
        "account_auth_mode_switch",
        &options,
        Some(active),
        SwitchLayout::EqualWidth,
        t,
    );
    if let Some(idx) = clicked {
        let tab = if idx == 0 {
            AuthTab::SignIn
        } else {
            AuthTab::CreateAccount
        };
        if app.view.account.tab != tab {
            app.view.account.tab = tab;
            app.view.account.error_message = None;
            app.view.account.status_message = None;
        }
    }
}

pub(crate) fn trigger_test_connection(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let raw_url = app.view.account.server_url.trim().to_string();
    match normalize_server_url(&raw_url) {
        Ok(norm) => {
            app.view.account.server_url = norm.clone();
            let _ = app.session.set_server_url_preference(&norm);
            app.flush_web_storage(true);
            app.view.account.error_message = None;
            let checking = ServerProbeState::Checking {
                server_url: norm.clone(),
            };
            app.view
                .account
                .server_probes
                .insert(norm.clone(), checking.clone());
            app.view.account.probe = checking;
            if !crate::sync_worker::dispatch_command(
                app,
                Some(ctx),
                SyncWorkerCommand::TestConnection {
                    server_url: norm.clone(),
                },
            ) {
                app.view
                    .account
                    .mark_server_offline(&norm, "Background network worker is unavailable.");
            }
        }
        Err(err) => {
            app.view
                .account
                .mark_server_offline(&raw_url, &err.to_string());
        }
    }
}

fn should_warn_unsynced_cloud_switch(
    app: &PaperCraftApp,
    target_server_url: &str,
    target_user_id: Option<&str>,
    target_email: Option<&str>,
) -> Option<(String, usize)> {
    let active = app.session.active_account_session()?;
    if !app.session.is_viewing_cloud() {
        return None;
    }
    let pending = app.session.cloud_pending_changes_count();
    if pending == 0 {
        return None;
    }
    let same_server = urls_match(&active.server_url, target_server_url);
    let same_user = target_user_id.is_some_and(|uid| uid == active.user.user_id)
        || target_email.is_some_and(|email| {
            active
                .user
                .email
                .as_deref()
                .is_some_and(|ae| ae.eq_ignore_ascii_case(email.trim()))
        });
    if same_server && same_user {
        return None;
    }
    Some((active.user.primary_label(), pending))
}

pub(crate) fn render_unsynced_switch_confirmation(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) {
    let Some(pending) = app.view.account.pending_unsynced_switch.clone() else {
        return;
    };
    let busy = app.view.account.auth_busy || app.view.account.password_reset.busy;
    ui.add_space(8.0);
    egui::Frame::new()
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, t.danger_border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), ir, Icon::AlertCircle, t.warning_fg);
                ui.label(
                    egui::RichText::new("Unsynced Changes on Current Account")
                        .size(12.5)
                        .strong()
                        .color(t.text),
                );
            });
            ui.add_space(4.0);
            let change_word = if pending.pending_count == 1 {
                "1 unsynced change"
            } else {
                "unsynced changes"
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "'{}' has {} ({change_word}) not uploaded yet. Switching to '{}' keeps a local cache so you can sync later.",
                        pending.outgoing_label,
                        pending.pending_count,
                        pending.target_label,
                    ))
                    .size(11.0)
                    .color(t.text_muted),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                if secondary_action_button(ui, "Cancel", !busy, t).clicked() {
                    app.view.account.clear_pending_unsynced_switch();
                }
                let sync_busy = app.session.is_sync_in_flight();
                if fixed_secondary_button(
                    ui,
                    if sync_busy { "Syncing..." } else { "Sync Now First" },
                    116.0,
                    !busy && !sync_busy,
                    t,
                )
                .clicked()
                {
                    app.view.account.clear_pending_unsynced_switch();
                    trigger_manual_sync(app, ui.ctx());
                }
                if primary_action_button(ui, "Switch Anyway", !busy, t)
                    .on_hover_text("Switch now and keep the unsynced offline cache on this device")
                    .clicked()
                {
                    let target = pending.target.clone();
                    app.view.account.clear_pending_unsynced_switch();
                    match target {
                        PendingUnsyncedSwitchTarget::AuthForm => {
                            dispatch_auth_form_now(app, ui.ctx());
                        }
                        PendingUnsyncedSwitchTarget::QuickLogin {
                            server_url,
                            user_id,
                        } => {
                            dispatch_quick_login_now(app, ui.ctx(), &server_url, &user_id);
                        }
                        PendingUnsyncedSwitchTarget::PasswordReset => {
                            dispatch_password_reset_now(app, ui.ctx());
                        }
                    }
                }
            });
        });
}

fn ensure_server_configured_url(app: &PaperCraftApp) -> Result<String, String> {
    if !app.session.is_server_configured() {
        return Err(
            "No sync server configured yet. Configure your server URL in Profile → Sync first."
                .to_string(),
        );
    }
    normalize_server_url(&app.view.account.server_url).map_err(|e| e.to_string())
}

fn validate_and_build_auth_form_command(
    app: &PaperCraftApp,
) -> Result<(String, String, SyncWorkerCommand), String> {
    let url = ensure_server_configured_url(app)?;
    let device_name = app.session.device_name().to_string();
    let password = app.view.account.password.clone();
    let disp_raw = app.view.account.display_name.trim().to_string();
    let display_name = (!disp_raw.is_empty()).then_some(disp_raw);
    let session_gen = app.sync_worker.current_session_generation();

    match app.view.account.tab {
        AuthTab::SignIn => {
            if password.is_empty() {
                return Err("Enter your password or recovery code.".to_string());
            }
            let cmd = build_sign_in_command(
                session_gen,
                &url,
                &app.view.account.email_or_username,
                password,
                device_name,
            )?;
            let target_email = normalize_email(&app.view.account.email_or_username)
                .map(|(_, e)| e)
                .unwrap_or_else(|_| app.view.account.email_or_username.trim().to_string());
            Ok((url, target_email, cmd))
        }
        AuthTab::CreateAccount => {
            if app.view.account.is_registration_closed() {
                return Err("Registration is closed on this server.".to_string());
            }
            validate_password(&app.view.account.password).map_err(|e| e.to_string())?;
            let (email, _) =
                normalize_email(&app.view.account.email_or_username).map_err(|e| e.to_string())?;
            let username = if app.view.account.signup_username.trim().is_empty() {
                None
            } else {
                let (clean, _) = normalize_username(&app.view.account.signup_username)
                    .map_err(|e| e.to_string())?;
                Some(clean)
            };
            let cmd = SyncWorkerCommand::Register {
                session_generation: session_gen,
                server_url: url.clone(),
                email: email.clone(),
                username,
                password,
                display_name,
                device_name,
            };
            Ok((url, email, cmd))
        }
    }
}

fn submit_auth_request(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let (url, target_email, _) = match validate_and_build_auth_form_command(app) {
        Ok(parts) => parts,
        Err(msg) => {
            app.view.account.status_message = None;
            app.view.account.error_message = Some(msg);
            return;
        }
    };
    let compare_email = (app.view.account.tab == AuthTab::SignIn).then_some(target_email.as_str());
    if let Some((outgoing_label, pending_count)) =
        should_warn_unsynced_cloud_switch(app, &url, None, compare_email)
    {
        app.view.account.error_message = None;
        app.view.account.pending_unsynced_switch = Some(PendingUnsyncedSwitch {
            outgoing_label,
            pending_count,
            target_label: target_email,
            target: PendingUnsyncedSwitchTarget::AuthForm,
        });
        return;
    }
    dispatch_auth_form_now(app, ctx);
}

fn dispatch_auth_form_now(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let (url, _, cmd) = match validate_and_build_auth_form_command(app) {
        Ok(parts) => parts,
        Err(msg) => {
            app.view.account.status_message = None;
            app.view.account.error_message = Some(msg);
            return;
        }
    };
    app.view.account.server_url = url.clone();
    let _ = app.session.set_server_url_preference(&url);
    app.view.account.clear_pending_unsynced_switch();
    app.view.account.auth_busy = true;
    app.view.account.error_message = None;
    app.view.account.status_message = Some("Contacting PaperCraft Server...".to_string());

    if !crate::sync_worker::dispatch_command(app, Some(ctx), cmd) {
        app.view.account.auth_busy = false;
        app.view.account.status_message = None;
        app.view.account.error_message =
            Some("Background network worker is unavailable.".to_string());
    }
}

fn validate_and_build_password_reset(
    app: &PaperCraftApp,
) -> Result<(String, String, SyncWorkerCommand), String> {
    let url = ensure_server_configured_url(app)?;
    let identifier = app
        .view
        .account
        .password_reset
        .identifier
        .trim()
        .to_string();
    let (clean_id, _) = normalize_email(&identifier)
        .map_err(|_| "Enter a valid email address to recover your account.".to_string())?;
    let code = app
        .view
        .account
        .password_reset
        .recovery_code
        .trim()
        .to_string();
    if code.is_empty() {
        return Err("Enter the temporary recovery code generated on the server.".to_string());
    }
    validate_password(&app.view.account.password_reset.new_password).map_err(|e| e.to_string())?;
    if app.view.account.password_reset.new_password
        != app.view.account.password_reset.confirm_password
    {
        return Err("New password and confirmation do not match.".to_string());
    }
    if app.view.account.password_reset.new_password == code {
        return Err("New password must be different from the temporary recovery code.".to_string());
    }
    let cmd = SyncWorkerCommand::CompletePasswordReset {
        session_generation: app.sync_worker.current_session_generation(),
        server_url: url.clone(),
        identifier: clean_id.clone(),
        recovery_code: code,
        new_password: app.view.account.password_reset.new_password.clone(),
        device_name: app.session.device_name().to_string(),
    };
    Ok((url, clean_id, cmd))
}

fn submit_complete_password_reset(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let (url, clean_id, _) = match validate_and_build_password_reset(app) {
        Ok(parts) => parts,
        Err(msg) => {
            app.view.account.status_message = None;
            app.view.account.error_message = Some(msg);
            return;
        }
    };
    if let Some((outgoing_label, pending_count)) =
        should_warn_unsynced_cloud_switch(app, &url, None, Some(&clean_id))
    {
        app.view.account.error_message = None;
        app.view.account.pending_unsynced_switch = Some(PendingUnsyncedSwitch {
            outgoing_label,
            pending_count,
            target_label: clean_id,
            target: PendingUnsyncedSwitchTarget::PasswordReset,
        });
        return;
    }
    dispatch_password_reset_now(app, ctx);
}

fn dispatch_password_reset_now(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let (url, _, cmd) = match validate_and_build_password_reset(app) {
        Ok(parts) => parts,
        Err(msg) => {
            app.view.account.status_message = None;
            app.view.account.error_message = Some(msg);
            return;
        }
    };
    app.view.account.server_url = url.clone();
    let _ = app.session.set_server_url_preference(&url);
    app.view.account.clear_pending_unsynced_switch();
    app.view.account.password_reset.busy = true;
    app.view.account.error_message = None;
    app.view.account.status_message =
        Some("Verifying recovery code and saving your new password...".to_string());

    if !crate::sync_worker::dispatch_command(app, Some(ctx), cmd) {
        app.view.account.password_reset.busy = false;
        app.view.account.status_message = None;
        app.view.account.error_message =
            Some("Background network worker is unavailable.".to_string());
    }
}

fn submit_update_profile(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let Some(session) = app.session.active_account_session().cloned() else {
        app.view.account.error_message = Some("Sign in to a cloud account first.".to_string());
        return;
    };
    let (clean_username, _) = match normalize_username(&app.view.account.edit_username) {
        Ok(pair) => pair,
        Err(err) => {
            app.view.account.status_message = None;
            app.view.account.error_message = Some(err.to_string());
            return;
        }
    };
    let clean_display = sanitize_display_name(&app.view.account.edit_display_name, &clean_username);
    let username = (clean_username != session.user.username).then_some(clean_username);
    let display_name = (clean_display != session.user.display_name).then_some(clean_display);
    if username.is_none() && display_name.is_none() {
        app.view.account.status_message = Some("No profile changes to save.".to_string());
        return;
    }

    app.view.account.profile_save_busy = true;
    app.view.account.error_message = None;
    let cmd = SyncWorkerCommand::UpdateProfile {
        session_generation: app.sync_worker.current_session_generation(),
        session,
        username,
        display_name,
    };
    if !crate::sync_worker::dispatch_command(app, Some(ctx), cmd) {
        app.view.account.profile_save_busy = false;
        app.view.account.error_message =
            Some("Background network worker is unavailable.".to_string());
    }
}

fn submit_change_password(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let Some(session) = app.session.active_account_session().cloned() else {
        app.view.account.error_message = Some("Sign in to a cloud account first.".to_string());
        return;
    };
    if app.view.account.current_password_draft.is_empty() {
        app.view.account.status_message = None;
        app.view.account.error_message =
            Some("Enter your current password to confirm.".to_string());
        return;
    }
    if let Err(err) = validate_password(&app.view.account.new_password_draft) {
        app.view.account.status_message = None;
        app.view.account.error_message = Some(err.to_string());
        return;
    }
    if app.view.account.new_password_draft != app.view.account.confirm_new_password_draft {
        app.view.account.status_message = None;
        app.view.account.error_message =
            Some("New password and confirmation do not match.".to_string());
        return;
    }
    if app.view.account.new_password_draft == app.view.account.current_password_draft {
        app.view.account.status_message = None;
        app.view.account.error_message =
            Some("New password must be different from your current password.".to_string());
        return;
    }

    app.view.account.change_password_busy = true;
    app.view.account.error_message = None;
    let cmd = SyncWorkerCommand::ChangePassword {
        session_generation: app.sync_worker.current_session_generation(),
        session,
        current_password: app.view.account.current_password_draft.clone(),
        new_password: app.view.account.new_password_draft.clone(),
    };
    if !crate::sync_worker::dispatch_command(app, Some(ctx), cmd) {
        app.view.account.change_password_busy = false;
        app.view.account.error_message =
            Some("Background network worker is unavailable.".to_string());
    }
}

fn build_sign_in_command(
    session_generation: u64,
    server_url: &str,
    identifier_raw: &str,
    password: String,
    device_name: String,
) -> Result<SyncWorkerCommand, String> {
    let trimmed = identifier_raw.trim();
    if !trimmed.contains('@') {
        return Err("Sign in with your email address (usernames are not accepted).".to_string());
    }
    let (_, email) = normalize_email(trimmed).map_err(|e| e.to_string())?;
    Ok(SyncWorkerCommand::Login {
        session_generation,
        server_url: server_url.to_string(),
        identifier: email,
        password,
        device_name,
    })
}

fn submit_change_email(app: &mut PaperCraftApp, ctx: &egui::Context) {
    let Some(session) = app.session.active_account_session().cloned() else {
        app.view.account.error_message = Some("Sign in to a cloud account first.".to_string());
        return;
    };
    let (new_email, _) = match normalize_email(&app.view.account.new_email_draft) {
        Ok(pair) => pair,
        Err(err) => {
            app.view.account.error_message = Some(err.to_string());
            return;
        }
    };
    if app.view.account.change_email_password.is_empty() {
        app.view.account.error_message =
            Some("Enter your current password to confirm.".to_string());
        return;
    }

    app.view.account.change_email_busy = true;
    app.view.account.error_message = None;
    let cmd = SyncWorkerCommand::ChangeEmail {
        session_generation: app.sync_worker.current_session_generation(),
        session,
        new_email,
        current_password: app.view.account.change_email_password.clone(),
    };
    if !crate::sync_worker::dispatch_command(app, Some(ctx), cmd) {
        app.view.account.change_email_busy = false;
        app.view.account.error_message =
            Some("Background network worker is unavailable.".to_string());
    }
}

enum FieldHint {
    Muted(&'static str),
    Success(&'static str),
    Warning(String),
}

fn draw_field_label_row(ui: &mut egui::Ui, label: &str, hint: FieldHint, t: &Tokens) {
    ui.horizontal_top(|ui| {
        ui.label(egui::RichText::new(label).size(11.5).strong().color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let (text, color) = match hint {
                FieldHint::Muted(s) => (s.to_string(), t.text_muted),
                FieldHint::Success(s) => (s.to_string(), t.success_fg),
                FieldHint::Warning(s) => (s, t.warning_fg),
            };
            ui.label(egui::RichText::new(text).size(10.5).color(color));
        });
    });
}

enum InputTrailing {
    None,
    GreenCheck(&'static str),
    Icon(Icon, Color32, &'static str),
}

struct InputFieldSpec<'a> {
    id_salt: &'a str,
    width: f32,
    hint: &'a str,
    border_color: Option<Color32>,
    trailing: InputTrailing,
}

struct InputBoxOutcome {
    changed: bool,
    enter_pressed: bool,
}

fn draw_input_shell(
    ui: &mut egui::Ui,
    id_salt: &str,
    width: f32,
    border_override: Option<Color32>,
    t: &Tokens,
) -> (Rect, egui::Id) {
    let input_width = width.max(0.0).min(ui.available_width().max(0.0));
    let (box_rect, _) =
        ui.allocate_exact_size(Vec2::new(input_width, INPUT_HEIGHT), Sense::hover());
    let edit_id = ui.make_persistent_id(id_salt);
    let has_focus = ui.ctx().memory(|m| m.has_focus(edit_id));
    let is_hovered = !has_focus && ui.is_enabled() && ui.rect_contains_pointer(box_rect);
    let focus_t = crate::motion::hover_factor(ui.ctx(), edit_id.with("focus_t"), has_focus);
    let hover_t = crate::motion::hover_factor(ui.ctx(), edit_id.with("hover_t"), is_hovered);
    let rest_border = border_override.unwrap_or(t.border);
    let hover_border = border_override.unwrap_or_else(|| t.text_muted.gamma_multiply(0.55));
    let active_border = crate::motion::lerp_color(rest_border, hover_border, hover_t);
    let border_color = crate::motion::lerp_color(active_border, t.accent, focus_t);
    let rest_width = if border_override.is_some() { 1.2 } else { 1.0 };
    let stroke_width = crate::motion::lerp_f32(rest_width, 1.5, focus_t);
    let p = ui.painter();
    p.rect_filled(
        box_rect,
        CornerRadius::same(CONTROL_RADIUS as u8),
        t.field_bg,
    );
    p.rect_stroke(
        box_rect,
        CornerRadius::same(CONTROL_RADIUS as u8),
        Stroke::new(stroke_width, border_color),
        StrokeKind::Inside,
    );
    (box_rect, edit_id)
}

struct ManualInputOutcome {
    edit_id: egui::Id,
    outcome: InputBoxOutcome,
    paste_blocked: bool,
}

fn paint_input_trailing(ui: &mut egui::Ui, trailing: InputTrailing, t: &Tokens) {
    match trailing {
        InputTrailing::None => {}
        InputTrailing::GreenCheck(tip) => {
            let (ir, icon_resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_green_check_badge(ui.painter(), ir, t);
            let _ = icon_resp.on_hover_text(tip);
        }
        InputTrailing::Icon(icon, color, tip) => {
            let (ir, icon_resp) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
            paint_icon(ui.painter(), ir, icon, color);
            let _ = icon_resp.on_hover_text(tip);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipboardEventAction {
    Keep,
    Drop,
    BlockPaste,
}

fn classify_clipboard_event(
    event: &egui::Event,
    has_focus: bool,
    pointer_inside: bool,
    box_rect: Rect,
    target_accesskit_id: egui::accesskit::NodeId,
) -> ClipboardEventAction {
    match event {
        egui::Event::Paste(_) if has_focus || pointer_inside => ClipboardEventAction::BlockPaste,
        egui::Event::MiddleClickPaste { pos, .. } if has_focus || box_rect.contains(*pos) => {
            ClipboardEventAction::BlockPaste
        }
        egui::Event::Copy | egui::Event::Cut if has_focus => ClipboardEventAction::Drop,
        egui::Event::AccessKitActionRequest(req)
            if req.target_node == target_accesskit_id
                && matches!(
                    req.action,
                    egui::accesskit::Action::ReplaceSelectedText
                        | egui::accesskit::Action::SetValue
                ) =>
        {
            ClipboardEventAction::BlockPaste
        }
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } if has_focus => {
            let cmd = modifiers.command || modifiers.ctrl;
            if (cmd && *key == egui::Key::V) || (modifiers.shift && *key == egui::Key::Insert) {
                ClipboardEventAction::BlockPaste
            } else if cmd && matches!(*key, egui::Key::C | egui::Key::X | egui::Key::Insert) {
                ClipboardEventAction::Drop
            } else {
                ClipboardEventAction::Keep
            }
        }
        _ => ClipboardEventAction::Keep,
    }
}

fn suppress_clipboard_events(ui: &egui::Ui, edit_id: egui::Id, box_rect: Rect) -> bool {
    let has_focus = ui.ctx().memory(|m| m.has_focus(edit_id));
    let pointer_inside = ui.rect_contains_pointer(box_rect);
    if !has_focus && !pointer_inside {
        return false;
    }
    let target_accesskit_id = edit_id.accesskit_id();
    let mut paste_blocked = false;
    ui.ctx().input_mut(|i| {
        i.events.retain(|event| {
            match classify_clipboard_event(
                event,
                has_focus,
                pointer_inside,
                box_rect,
                target_accesskit_id,
            ) {
                ClipboardEventAction::Keep => true,
                ClipboardEventAction::Drop => false,
                ClipboardEventAction::BlockPaste => {
                    paste_blocked = true;
                    false
                }
            }
        });
    });
    paste_blocked
}

fn draw_text_input_inner(
    ui: &mut egui::Ui,
    spec: InputFieldSpec<'_>,
    value: &mut String,
    allow_clipboard: bool,
    t: &Tokens,
) -> ManualInputOutcome {
    let (box_rect, edit_id) = draw_input_shell(ui, spec.id_salt, spec.width, spec.border_color, t);
    let paste_blocked = if allow_clipboard {
        false
    } else {
        suppress_clipboard_events(ui, edit_id, box_rect)
    };
    let inner_rect = box_rect.shrink2(Vec2::new(10.0, 2.0));
    let has_trailing = !matches!(spec.trailing, InputTrailing::None);
    let trailing_w = if has_trailing { 22.0 } else { 0.0 };
    let text_w = (inner_rect.width() - trailing_w).max(40.0);

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 6.0;
    child.set_width(inner_rect.width());

    let resp = child.add_sized(
        Vec2::new(text_w, inner_rect.height()),
        egui::TextEdit::singleline(value)
            .id(edit_id)
            .desired_width(text_w)
            .font(egui::FontId::proportional(12.5))
            .margin(egui::Margin::ZERO)
            .align(egui::Align2::LEFT_CENTER)
            .frame(egui::Frame::NONE)
            .hint_text(spec.hint),
    );
    let enter_pressed = resp.lost_focus() && child.input(|i| i.key_pressed(egui::Key::Enter));
    paint_input_trailing(&mut child, spec.trailing, t);

    ManualInputOutcome {
        edit_id,
        outcome: InputBoxOutcome {
            changed: resp.changed(),
            enter_pressed,
        },
        paste_blocked,
    }
}

fn draw_text_input(
    ui: &mut egui::Ui,
    spec: InputFieldSpec<'_>,
    value: &mut String,
    t: &Tokens,
) -> InputBoxOutcome {
    draw_text_input_inner(ui, spec, value, true, t).outcome
}

fn draw_manual_confirm_input(
    ui: &mut egui::Ui,
    spec: InputFieldSpec<'_>,
    value: &mut String,
    t: &Tokens,
) -> ManualInputOutcome {
    draw_text_input_inner(ui, spec, value, false, t)
}

fn draw_password_input(
    ui: &mut egui::Ui,
    id_salt: &str,
    width: f32,
    password: &mut String,
    show_password: &mut bool,
    t: &Tokens,
) -> InputBoxOutcome {
    let (box_rect, edit_id) = draw_input_shell(ui, id_salt, width, None, t);
    let inner_rect = box_rect.shrink2(Vec2::new(10.0, 2.0));
    let toggle_w = 24.0_f32;
    let text_w = (inner_rect.width() - toggle_w - 4.0).max(40.0);

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 4.0;
    child.set_width(inner_rect.width());

    let resp = child.add_sized(
        Vec2::new(text_w, inner_rect.height()),
        egui::TextEdit::singleline(password)
            .id(edit_id)
            .password(!*show_password)
            .desired_width(text_w)
            .font(egui::FontId::proportional(12.5))
            .margin(egui::Margin::ZERO)
            .align(egui::Align2::LEFT_CENTER)
            .frame(egui::Frame::NONE)
            .hint_text("Minimum 8 characters"),
    );
    let enter_pressed = resp.lost_focus() && child.input(|i| i.key_pressed(egui::Key::Enter));

    let (btn_rect, btn_resp) = child.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    if child.is_rect_visible(btn_rect) {
        let hover_t =
            crate::motion::hover_factor(child.ctx(), btn_resp.id.with("hov"), btn_resp.hovered());
        let bg = crate::motion::lerp_color(Color32::TRANSPARENT, t.toolbar_bg, hover_t);
        let fg = crate::motion::lerp_color(t.text_muted, t.text, hover_t);
        child
            .painter()
            .rect_filled(btn_rect, CornerRadius::same(5), bg);
        let icon = if *show_password {
            Icon::EyeOff
        } else {
            Icon::Eye
        };
        paint_icon(child.painter(), btn_rect.shrink(3.5), icon, fg);
    }
    let tip = if *show_password {
        "Hide password"
    } else {
        "Show password"
    };
    if btn_resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
        .clicked()
    {
        *show_password = !*show_password;
    }

    InputBoxOutcome {
        changed: resp.changed(),
        enter_pressed,
    }
}

pub(crate) fn hollow_secondary_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let text_color = if enabled { t.text } else { t.text_muted };
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(12.0),
        text_color,
    );
    let size = Vec2::new((galley.size().x + 20.0).max(52.0), INPUT_HEIGHT);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let hov_t = crate::motion::hover_factor(
            ui.ctx(),
            resp.id.with("hov"),
            enabled && ui.is_enabled() && resp.hovered(),
        );
        let fill = if enabled {
            crate::motion::lerp_color(Color32::TRANSPARENT, t.border.gamma_multiply(0.45), hov_t)
        } else {
            Color32::TRANSPARENT
        };
        let border = if enabled {
            crate::motion::lerp_color(t.border, t.text_muted.gamma_multiply(0.55), hov_t)
        } else {
            t.border.gamma_multiply(0.6)
        };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(CONTROL_RADIUS as u8), fill);
        painter.rect_stroke(
            rect,
            CornerRadius::same(CONTROL_RADIUS as u8),
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        let text_pos = egui::pos2(
            rect.center().x - galley.size().x * 0.5,
            rect.center().y - galley.size().y * 0.5,
        );
        painter.galley(text_pos, galley, text_color);
    }
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

pub(crate) fn fixed_secondary_button(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let text_color = if enabled { t.text } else { t.text_muted };
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(12.0),
        text_color,
    );
    let size = Vec2::new(width.max(galley.size().x + 16.0), INPUT_HEIGHT);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hov_t = crate::motion::hover_factor(
        ui.ctx(),
        resp.id.with("hov"),
        enabled && ui.is_enabled() && resp.hovered(),
    );
    let fill = if enabled {
        crate::motion::lerp_color(t.toolbar_bg, t.field_bg, hov_t)
    } else {
        t.toolbar_bg
    };
    let border = if enabled {
        crate::motion::lerp_color(t.border, t.accent.gamma_multiply(0.75), hov_t * 0.65)
    } else {
        t.border
    };
    let fg = if enabled {
        crate::motion::lerp_color(t.text, t.accent, hov_t * 0.25)
    } else {
        t.text_muted
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(CONTROL_RADIUS as u8), fill);
    painter.rect_stroke(
        rect,
        CornerRadius::same(CONTROL_RADIUS as u8),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let text_pos = egui::pos2(
        rect.center().x - galley.size().x * 0.5,
        rect.center().y - galley.size().y * 0.5,
    );
    painter.galley(text_pos, galley, fg);
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

pub(crate) fn draw_section_card(ui: &mut egui::Ui, t: &Tokens, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui);
        });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountColumnTone {
    Local,
    Server,
    ServerOffline,
}

pub(crate) fn draw_account_column_card(
    ui: &mut egui::Ui,
    tone: AccountColumnTone,
    t: &Tokens,
    body: impl FnOnce(&mut egui::Ui),
) {
    let border = match tone {
        AccountColumnTone::Local => t.accent.gamma_multiply(0.35),
        AccountColumnTone::Server => t.success_border,
        AccountColumnTone::ServerOffline => t.border,
    };
    egui::Frame::new()
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            body(ui);
        });
}

pub(crate) fn draw_column_header_badge(
    ui: &mut egui::Ui,
    tone: AccountColumnTone,
    _icon: Icon,
    t: &Tokens,
) {
    let (resolved_icon, fg, fill, stroke) = match tone {
        AccountColumnTone::Local => account_icon_and_colors(false, false, t),
        AccountColumnTone::Server => account_icon_and_colors(true, true, t),
        AccountColumnTone::ServerOffline => account_icon_and_colors(true, false, t),
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(7),
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    paint_icon(ui.painter(), rect.shrink(5.5), resolved_icon, fg);
}

pub(crate) fn draw_local_vs_server_split<S>(
    ui: &mut egui::Ui,
    state: &mut S,
    min_split_width: f32,
    gutter: f32,
    t: &Tokens,
    render_local: impl FnOnce(&mut S, &mut egui::Ui),
    render_server: impl FnOnce(&mut S, &mut egui::Ui),
) {
    let avail_w = ui.available_width();
    if avail_w >= min_split_width {
        let col_w = ((avail_w - gutter) * 0.5).floor().max(220.0);
        let row_resp = ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let left_resp = ui.allocate_ui_with_layout(
                Vec2::new(col_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(col_w);
                    ui.set_max_width(col_w);
                    render_local(state, ui);
                },
            );
            let (div_rect, _) = ui.allocate_exact_size(Vec2::new(gutter, 1.0), Sense::hover());
            let right_resp = ui.allocate_ui_with_layout(
                Vec2::new(col_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(col_w);
                    ui.set_max_width(col_w);
                    render_server(state, ui);
                },
            );
            (left_resp.response.rect, div_rect, right_resp.response.rect)
        });
        let (left_rect, div_rect, right_rect) = row_resp.inner;
        let top_y = left_rect.top().min(right_rect.top()) + 8.0;
        let bottom_y = left_rect.bottom().max(right_rect.bottom()) - 8.0;
        if bottom_y > top_y + 8.0 {
            ui.painter().vline(
                div_rect.center().x,
                top_y..=bottom_y,
                Stroke::new(1.0, t.border),
            );
        }
    } else {
        render_local(state, ui);
        ui.add_space(10.0);
        render_server(state, ui);
    }
}

pub(crate) fn draw_hairline(ui: &mut egui::Ui, t: &Tokens) {
    let w = ui.available_width();
    let (sep_rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter().hline(
        sep_rect.x_range(),
        sep_rect.center().y,
        Stroke::new(1.0, t.border),
    );
}

fn draw_metric_cell(ui: &mut egui::Ui, label: &str, value: &str, t: &Tokens) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(egui::RichText::new(label).size(10.0).color(t.text_muted));
        ui.label(egui::RichText::new(value).size(12.0).strong().color(t.text));
    });
}

pub(crate) fn render_auth_banners(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let active_key = if let Some(ref err) = app.view.account.error_message {
        format!("err:{err}")
    } else if let Some(ref msg) = app.view.account.status_message {
        format!("msg:{msg}")
    } else {
        String::new()
    };
    let dt = ui.input(|i| i.unstable_dt).min(0.1);
    if app.view.motion.account_banner.advance(active_key, dt) {
        ui.ctx().request_repaint();
    }
    let banner_opacity = app.view.motion.account_banner.opacity();

    if let Some(ref err) = app.view.account.error_message {
        ui.add_space(8.0);
        ui.scope(|ui| {
            ui.multiply_opacity(banner_opacity);
            draw_status_banner(ui, BannerTone::Error, err, t);
        });
    } else if let Some(ref msg) = app.view.account.status_message {
        ui.add_space(8.0);
        let tone = if msg.starts_with("Server URL updated")
            || msg.starts_with("Account email updated")
            || msg.starts_with("Profile updated")
            || msg.starts_with("Local account renamed")
            || msg.starts_with("Account password updated")
            || msg.starts_with("Moved ")
            || msg.starts_with("Deleted local profile ")
            || msg.starts_with("Copied ")
        {
            BannerTone::Success
        } else {
            BannerTone::Info
        };
        ui.scope(|ui| {
            ui.multiply_opacity(banner_opacity);
            draw_status_banner(ui, tone, msg, t);
        });
    }
}

#[derive(Clone, Copy)]
pub(crate) enum BannerTone {
    Info,
    Success,
    Warning,
    Error,
}

pub(crate) fn draw_status_banner(ui: &mut egui::Ui, tone: BannerTone, text: &str, t: &Tokens) {
    let (bg, border, fg) = match tone {
        BannerTone::Info => (t.accent_soft, t.accent, t.text),
        BannerTone::Success => (t.success_bg, t.success_border, t.success_fg),
        BannerTone::Warning => (t.toolbar_bg, t.border, t.warning_fg),
        BannerTone::Error => (t.danger_bg, t.danger_border, t.danger_fg),
    };
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
                match tone {
                    BannerTone::Success => paint_green_check_badge(ui.painter(), icon_rect, t),
                    BannerTone::Info => paint_icon(ui.painter(), icon_rect, Icon::Info, t.accent),
                    BannerTone::Warning => {
                        paint_icon(ui.painter(), icon_rect, Icon::AlertCircle, t.warning_fg);
                    }
                    BannerTone::Error => {
                        paint_icon(ui.painter(), icon_rect, Icon::AlertCircle, t.danger_fg);
                    }
                }
                ui.add(egui::Label::new(egui::RichText::new(text).size(11.5).color(fg)).wrap());
            });
        });
}

pub(crate) enum PillIcon<'a> {
    None,
    Vector(Icon),
    GreenCheck(&'a Tokens),
    Dot,
}

pub(crate) struct PillStyle<'a> {
    pub(crate) fill: Color32,
    pub(crate) border: Color32,
    pub(crate) fg: Color32,
    pub(crate) icon: PillIcon<'a>,
}

pub(crate) fn draw_pill_badge(ui: &mut egui::Ui, style: PillStyle<'_>, label: &str) {
    egui::Frame::new()
        .fill(style.fill)
        .stroke(Stroke::new(1.0, style.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                match style.icon {
                    PillIcon::None => {}
                    PillIcon::Vector(icon) => {
                        let (ir, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                        paint_icon(ui.painter(), ir, icon, style.fg);
                    }
                    PillIcon::GreenCheck(t) => {
                        let (ir, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                        paint_green_check_badge(ui.painter(), ir, t);
                    }
                    PillIcon::Dot => {
                        let (dr, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        ui.painter().circle_filled(dr.center(), 3.5, style.fg);
                    }
                }
                ui.label(
                    egui::RichText::new(label)
                        .size(11.0)
                        .strong()
                        .color(style.fg),
                );
            });
        });
}

pub(crate) fn draw_sync_connection_pill(ui: &mut egui::Ui, state: SyncConnectionState, t: &Tokens) {
    let (label, fg) = status_badge_style(state, t);
    let style = match state {
        SyncConnectionState::Synced => PillStyle {
            fill: t.success_bg,
            border: t.success_border,
            fg,
            icon: PillIcon::GreenCheck(t),
        },
        SyncConnectionState::Syncing => PillStyle {
            fill: t.accent_soft,
            border: t.accent,
            fg,
            icon: PillIcon::Vector(Icon::Sync),
        },
        SyncConnectionState::Error => PillStyle {
            fill: t.danger_bg,
            border: t.danger_border,
            fg,
            icon: PillIcon::Vector(Icon::AlertCircle),
        },
        SyncConnectionState::Offline | SyncConnectionState::SignedOut => PillStyle {
            fill: t.toolbar_bg,
            border: t.border,
            fg,
            icon: PillIcon::Dot,
        },
    };
    draw_pill_badge(ui, style, label);
}

/// Paints a solid green circular status badge containing a crisp white checkmark.
pub(crate) fn paint_green_check_badge(painter: &egui::Painter, rect: Rect, t: &Tokens) {
    let radius = rect.width().min(rect.height()) * 0.5;
    painter.circle_filled(rect.center(), radius, t.success_fg);
    paint_icon(
        painter,
        rect.shrink(radius * 0.28),
        Icon::Check,
        Color32::WHITE,
    );
}

pub(crate) fn status_badge_style(
    state: SyncConnectionState,
    t: &Tokens,
) -> (&'static str, Color32) {
    match state {
        SyncConnectionState::SignedOut => ("Local Only", t.text_muted),
        SyncConnectionState::Offline => ("Offline", t.warning_fg),
        SyncConnectionState::Syncing => ("Syncing", t.accent),
        SyncConnectionState::Synced => ("Synced", t.success_fg),
        SyncConnectionState::Error => ("Sync Issue", t.danger_fg),
    }
}

/// High-contrast sync status dot color intended for top application headers.
pub(crate) fn header_status_dot_color(state: SyncConnectionState, t: &Tokens) -> Color32 {
    if t.is_dark {
        status_badge_style(state, t).1
    } else {
        let base = match state {
            SyncConnectionState::SignedOut => return t.header_fg_muted,
            SyncConnectionState::Offline => Color32::from_rgb(0xFB, 0xBF, 0x24),
            SyncConnectionState::Syncing => Color32::from_rgb(0x82, 0xCA, 0xF2),
            SyncConnectionState::Synced => Color32::from_rgb(0x4A, 0xDE, 0x80),
            SyncConnectionState::Error => Color32::from_rgb(0xF8, 0x71, 0x71),
        };
        crate::theme::ensure_contrast_against(base, &[t.header_bg], 3.0, None)
    }
}

/// Returns badge text and semantic color tokens for per-notebook sync indicators.
pub(crate) fn notebook_status_badge_style(
    status: EntitySyncStatus,
    t: &Tokens,
) -> (&'static str, Color32, Color32, Color32) {
    match status {
        EntitySyncStatus::LocalOnly => ("On device", t.text_muted, t.toolbar_bg, t.border),
        EntitySyncStatus::PendingPush => ("Pending sync", t.warning_fg, t.toolbar_bg, t.border),
        EntitySyncStatus::Synced => ("Cloud", t.success_fg, t.success_bg, t.success_border),
        EntitySyncStatus::SyncError => ("Sync issue", t.danger_fg, t.danger_bg, t.danger_border),
    }
}

pub(crate) fn urls_match(a: &str, b: &str) -> bool {
    match (normalize_server_url(a), normalize_server_url(b)) {
        (Ok(na), Ok(nb)) => na == nb,
        _ => a.trim().trim_end_matches('/') == b.trim().trim_end_matches('/'),
    }
}

fn format_last_sync(last_ms: Option<i64>) -> String {
    let Some(ts) = last_ms else {
        return "Not synced yet".to_string();
    };
    let elapsed_s = (now_unix_ms() - ts).max(0) / 1000;
    if elapsed_s < 10 {
        "Just now".to_string()
    } else if elapsed_s < 60 {
        format!("{elapsed_s}s ago")
    } else if elapsed_s < 3600 {
        format!("{}m ago", elapsed_s / 60)
    } else {
        format!("{}h ago", elapsed_s / 3600)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{
        AuthTab, ProfileSection, ServerProbeState, local_profile_delete_confirmation_phrase,
    };
    use crate::PaperCraftApp;
    use crate::modal;
    use egui::{Shape, epaint::ClippedShape};
    use papercraft_sync::ActiveAccountSession;
    use papercraft_sync_protocol::{
        DEFAULT_SERVER_URL, LocalDiscoverySummary, ServerInfoResponse, SyncPullResponse,
        UserProfile,
    };

    fn collect_text(shapes: &[ClippedShape]) -> String {
        let mut out = String::new();
        fn visit(shape: &Shape, buf: &mut String) {
            match shape {
                Shape::Text(text) => {
                    buf.push_str(text.galley.text());
                    buf.push('\n');
                }
                Shape::Vec(items) => {
                    for item in items {
                        visit(item, buf);
                    }
                }
                _ => {}
            }
        }
        for clipped in shapes {
            visit(&clipped.shape, &mut out);
        }
        out
    }

    fn count_filled_circles_with_color(shapes: &[ClippedShape], target: egui::Color32) -> usize {
        let mut count = 0usize;
        fn visit(shape: &Shape, target: egui::Color32, count: &mut usize) {
            match shape {
                Shape::Circle(c) if c.fill == target => {
                    *count += 1;
                }
                Shape::Vec(items) => {
                    for item in items {
                        visit(item, target, count);
                    }
                }
                _ => {}
            }
        }
        for clipped in shapes {
            visit(&clipped.shape, target, &mut count);
        }
        count
    }

    fn test_account(user_id: &str, email: &str) -> ActiveAccountSession {
        ActiveAccountSession {
            server_url: DEFAULT_SERVER_URL.to_string(),
            user: UserProfile {
                user_id: user_id.to_string(),
                email: Some(email.to_string()),
                username: "alex".to_string(),
                display_name: "Alex".to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "a".to_string(),
            refresh_token: "r".to_string(),
            access_expires_at_ms: 99_999,
            refresh_expires_at_ms: 99_999,
            device_name: "WinPC".to_string(),
            last_auth_ms: 1000,
        }
    }

    fn render_modal_settled(ctx: &egui::Context, app: &mut PaperCraftApp) -> String {
        let mut text = String::new();
        for step in 0..6 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f64::from(step) * 0.08),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                modal::show(app, ui);
            });
            text = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        text
    }

    #[test]
    fn account_sync_modal_renders_both_signed_out_and_signed_in_states() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.session.prefs.theme = papercraft_doc::UiTheme::Light;
        modal::open(&mut app, modal::ActiveModal::AccountSync);

        let signed_out_text = render_modal_settled(&ctx, &mut app);
        assert_eq!(app.view.active_modal, Some(modal::ActiveModal::AccountSync));
        assert!(
            signed_out_text.contains("On This Device:"),
            "signed-out text:\n{signed_out_text}"
        );
        assert!(!signed_out_text.contains("Link Profile"));
        assert!(!signed_out_text.contains("Do you want to sync it"));

        let _ = app.session.execute(
            "profile.create",
            &serde_json::json!({ "nickname": "Bruno" }),
        );
        let acct = test_account("u-1", "alex@example.com");
        let pull = SyncPullResponse {
            server_cursor: 0,
            user: acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(acct, pull);
        let signed_in_text = render_modal_settled(&ctx, &mut app);
        assert!(app.session.is_signed_in());
        assert!(
            signed_in_text.contains("alex@example.com"),
            "signed-in text:\n{signed_in_text}"
        );
        assert!(signed_in_text.contains("SYNC QUEUE"));
        assert!(signed_in_text.contains("Offline-First • Synced"));
        assert!(!signed_in_text.contains("Detected Local Accounts on This Device"));
        assert!(!signed_in_text.contains("Merge into Cloud"));
    }

    #[test]
    fn first_access_select_local_and_convert_modals_render_cleanly() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();

        app.view.account.disconnect_notice =
            Some("Connection lost — automatically signed out.".to_string());
        modal::open(&mut app, modal::ActiveModal::FirstAccessNickname);
        let first_text = render_modal_settled(&ctx, &mut app);
        assert!(first_text.contains("Connection lost — automatically signed out."));
        assert!(first_text.contains("Nickname"));
        assert!(first_text.contains("Start Session"));
        assert!(!first_text.contains("Sign In / Register"));
        assert!(!first_text.contains("Remember last account and skip on startup"));

        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Bruno" }),
            )
            .unwrap();
        let bruno_id = app
            .session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();
        modal::open(&mut app, modal::ActiveModal::SelectLocalAccount);
        let select_text = render_modal_settled(&ctx, &mut app);
        assert!(select_text.contains("Local Accounts (1)"));
        assert!(select_text.contains("Server Accounts (0)"));
        assert!(select_text.contains("Bruno"));
        assert!(select_text.contains("Local"));
        assert!(select_text.contains("Open"));
        assert!(select_text.contains("Create & Open"));

        app.view.account.convert_source_profile_id = bruno_id;
        app.view.account.convert_source_nickname = "Bruno".to_string();
        app.view.account.convert_source_notebook_count = 2;
        modal::open(&mut app, modal::ActiveModal::ConvertLocalAccount);
        let convert_text = render_modal_settled(&ctx, &mut app);
        assert!(convert_text.contains("Convert 'Bruno' to Cloud Account?"));
        assert!(!convert_text.contains("Self-Hosted Server"));
        assert!(convert_text.contains("Sync Server Not Configured"));
        assert!(convert_text.contains("Open Sync Settings"));
        assert!(convert_text.contains("Offline-First cache"));
        assert!(convert_text.contains("Email Address"));
        assert!(convert_text.contains("Password"));
        assert!(convert_text.contains("Create Cloud Account & Sync"));
    }

    fn render_profile_frames(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        start_time: f64,
        frames: u32,
    ) -> (String, Vec<ClippedShape>) {
        let mut text = String::new();
        let mut shapes = Vec::new();
        let total = frames.max(8);
        for step in 0..total {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(start_time + f64::from(step) * 0.025),
                    predicted_dt: 0.04,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    crate::inspector::show(app, ui);
                },
            );
            text = collect_text(&out.shapes);
            shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
        }
        (text, shapes)
    }

    fn find_profile_button_center(shapes: &[ClippedShape], label: &str) -> egui::Pos2 {
        fn visit(shape: &Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                Shape::Text(t) if t.galley.text() == label => {
                    Some(t.visual_bounding_rect().center())
                }
                Shape::Vec(items) => items.iter().find_map(|i| visit(i, label)),
                _ => None,
            }
        }
        shapes
            .iter()
            .find_map(|c| visit(&c.shape, label))
            .expect("expected button label")
    }

    fn click_profile_button(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        shapes: &[ClippedShape],
        label: &str,
        press_time: f64,
    ) {
        let pos = find_profile_button_center(shapes, label);
        for (pressed, offset) in [(true, 0.0), (false, 0.02)] {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(press_time + offset),
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    crate::inspector::show(app, ui);
                },
            );
            out.drop_without_applying_deltas();
        }
    }

    #[test]
    fn accounts_switcher_handles_offline_blocking_forget_and_local_sync() {
        let ctx = egui::Context::default();
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = None;
        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Personal" }),
            )
            .unwrap();
        let acct = test_account("u-shortcut", "cloud@example.com");
        let pull = SyncPullResponse {
            server_cursor: 0,
            user: acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(acct, pull);
        assert!(app.session.sign_out_from_server_session());
        assert_eq!(app.session.remembered_server_accounts().len(), 1);

        // When server probe failed and shortcut has no cached session, keep the inactive card minimal (name + Switch + Forget) and confirm Forget.
        crate::open_profile_drawer(&mut app);
        app.view
            .account
            .mark_server_offline(DEFAULT_SERVER_URL, "timeout");
        assert_eq!(app.view.account.profile_section, ProfileSection::Accounts);

        let (text, shapes) = render_profile_frames(&ctx, &mut app, 0.0, 4);
        assert!(
            text.contains("Alex"),
            "missing server account primary label:\n{text}"
        );
        assert!(
            !text.contains("cloud@example.com"),
            "collapsed inactive server card should omit secondary email/URL clutter:\n{text}"
        );
        assert!(text.contains("Switch"));
        assert!(text.contains("Forget"));
        assert!(
            !text.contains("Last check could not reach server"),
            "collapsed inactive server card should stay minimal:\n{text}"
        );
        assert!(text.contains("Sync"));
        assert!(text.contains("Add Account"));

        click_profile_button(&ctx, &mut app, &shapes, "Forget", 0.30);
        // First click opens inline confirmation rather than deleting immediately.
        assert_eq!(app.session.remembered_server_accounts().len(), 1);
        let (_, confirm_shapes) = render_profile_frames(&ctx, &mut app, 0.36, 10);
        click_profile_button(&ctx, &mut app, &confirm_shapes, "Confirm Forget", 1.20);
        assert!(app.session.remembered_server_accounts().is_empty());

        verify_cached_server_account_switch_and_forget(&ctx, &mut app);
    }

    fn verify_cached_server_account_switch_and_forget(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
    ) {
        // Cached server accounts always require password verification instead of passwordless access.
        let cached_acct = test_account("u-cached", "cached@example.com");
        let cached_pull = SyncPullResponse {
            server_cursor: 1,
            user: cached_acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session
            .enter_online_server_session(cached_acct, cached_pull);
        app.session
            .execute(
                "notebook.create",
                &serde_json::json!({ "title": "Pending Cloud Note" }),
            )
            .unwrap();
        let pid = app
            .session
            .saved_local_profiles()
            .first()
            .unwrap()
            .local_profile_id
            .clone();
        app.session.switch_to_local_profile(&pid).unwrap();
        app.view
            .account
            .mark_server_offline(DEFAULT_SERVER_URL, "offline");
        let (cached_text, cached_shapes) = render_profile_frames(ctx, app, 1.30, 10);
        assert!(
            !cached_text.contains("pending sync"),
            "inactive server card should omit extra sync metadata:\n{cached_text}"
        );
        assert!(
            !cached_text.contains("Last check could not reach server"),
            "collapsed inactive server card should stay minimal:\n{cached_text}"
        );
        assert!(
            !cached_text.contains("Open Offline"),
            "must not allow passwordless Open Offline bypass:\n{cached_text}"
        );
        // Clicking Switch when prior probe failed still opens the password verification drawer so user can retry!
        click_profile_button(ctx, app, &cached_shapes, "Switch", 2.15);
        assert!(!app.session.is_viewing_cloud());
        assert!(
            app.view.account.quick_login_key.is_some(),
            "stale offline probe must never hard-disable retrying sign-in"
        );
        let (offline_pw_text, _) = render_profile_frames(ctx, app, 2.20, 10);
        assert!(
            offline_pw_text.contains("Last check could not reach server"),
            "expanded quick-login drawer should show offline retry hint:\n{offline_pw_text}"
        );
        app.view.account.clear_quick_login_draft();

        // Clicking Forget on the cached account shows the pending local change warning.
        click_profile_button(ctx, app, &cached_shapes, "Forget", 2.85);
        let (warn_text, _) = render_profile_frames(ctx, app, 2.95, 10);
        assert!(
            warn_text.contains("not synced to the server yet"),
            "missing pending changes warning on Forget:\n{warn_text}"
        );
        app.view.account.clear_pending_forget_server();

        // When server comes back online, clicking Switch on the cached server account opens the password verification drawer instead of switching without password.
        app.view.account.apply_health_ok(
            DEFAULT_SERVER_URL.to_string(),
            ServerInfoResponse {
                status: "ok".to_string(),
                service: "papercraft-server".to_string(),
                version: "0.1.0".to_string(),
                protocol_version: 1,
                registration_open: true,
                storage_engine: "redb".to_string(),
                max_notebook_bytes: 1_048_576,
                local_discovery: LocalDiscoverySummary::default(),
            },
        );
        let (_, online_shapes) = render_profile_frames(ctx, app, 3.25, 10);
        click_profile_button(ctx, app, &online_shapes, "Switch", 4.10);
        assert!(
            !app.session.is_viewing_cloud(),
            "switching to a server-linked account must require password verification"
        );
        assert!(
            app.view.account.quick_login_key.is_some(),
            "clicking Switch on a server-linked account must open password prompt"
        );
        let (pw_text, _) = render_profile_frames(ctx, app, 4.20, 10);
        assert!(
            pw_text.contains("Verify"),
            "missing Verify button in quick-login drawer:\n{pw_text}"
        );
        assert!(
            pw_text.contains("Enter your password to verify"),
            "missing password verification hint:\n{pw_text}"
        );
        app.view.account.clear_quick_login_draft();

        // Active Personal local card omits Upload to Server (moved to top-bar Sync button) and Add Account sits in header above it.
        let (clean_text, clean_shapes) = render_profile_frames(ctx, app, 5.10, 10);
        assert!(
            !clean_text.contains("Upload to Server"),
            "active local card should no longer show Upload to Server:\n{clean_text}"
        );
        let personal_y = find_profile_button_center(&clean_shapes, "Personal").y;
        let add_acct_y = find_profile_button_center(&clean_shapes, "Add Account").y;
        assert!(
            add_acct_y < personal_y,
            "expected Add Account in top header above active Personal card: add={add_acct_y}, personal={personal_y}"
        );
    }

    #[test]
    fn select_local_account_modal_shows_server_accounts_password_drawer_and_other_email_subflow() {
        fn run_modal_frames(
            ctx: &egui::Context,
            app: &mut PaperCraftApp,
            start_time: f64,
            frames: usize,
        ) -> (String, Vec<ClippedShape>) {
            let mut text = String::new();
            let mut shapes = Vec::new();
            for step in 0..frames {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(start_time + f64::from(step as u32) * 0.05),
                    predicted_dt: 0.05,
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(app, ui);
                });
                text = collect_text(&out.shapes);
                shapes = out.shapes.clone();
                out.drop_without_applying_deltas();
            }
            (text, shapes)
        }

        let ctx = egui::Context::default();
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = None;
        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Work" }),
            )
            .unwrap();
        let acct = test_account("u-selector", "selector@example.com");
        let pull = SyncPullResponse {
            server_cursor: 0,
            user: acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(acct, pull);
        crate::sync_worker::complete_sign_out(&mut app, &ctx);
        assert_eq!(
            app.view.active_modal,
            Some(modal::ActiveModal::SelectLocalAccount)
        );
        app.view.account.apply_health_ok(
            DEFAULT_SERVER_URL.to_string(),
            ServerInfoResponse {
                status: "ok".to_string(),
                service: "papercraft-server".to_string(),
                version: "0.1.0".to_string(),
                protocol_version: 1,
                registration_open: true,
                storage_engine: "redb".to_string(),
                max_notebook_bytes: 1_048_576,
                local_discovery: LocalDiscoverySummary::default(),
            },
        );

        let (text, shapes) = run_modal_frames(&ctx, &mut app, 0.0, 8);
        assert!(text.contains("Choose an Account"), "\n{text}");
        assert!(text.contains("Local Accounts (1)"), "\n{text}");
        assert!(text.contains("Server Accounts (1)"), "\n{text}");
        assert!(text.contains("selector@example.com"), "\n{text}");
        assert!(text.contains("Server"), "\n{text}");
        assert!(text.contains("Work"), "\n{text}");
        assert!(text.contains("Local"), "\n{text}");
        assert!(text.contains("Sign In / Register"), "\n{text}");

        // Clicking Sign In on the server card opens the password drawer instead of bypassing auth.
        let sign_in_pos = find_profile_button_center(&shapes, "Sign In");
        for (pressed, t) in [(true, 0.45), (false, 0.47)] {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(t),
                    events: vec![
                        egui::Event::PointerMoved(sign_in_pos),
                        egui::Event::PointerButton {
                            pos: sign_in_pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(&mut app, ui);
                },
            );
            out.drop_without_applying_deltas();
        }
        assert!(app.view.account.quick_login_key.is_some());
        assert!(!app.session.is_viewing_cloud());
        let (pw_text, pw_shapes) = run_modal_frames(&ctx, &mut app, 0.55, 8);
        assert!(
            pw_text.contains("Enter your password"),
            "missing inline password prompt:\n{pw_text}"
        );
        assert!(
            pw_text.contains("Cancel"),
            "missing drawer Cancel button:\n{pw_text}"
        );

        // Clicking Sign In / Register transitions into the server URL & email login form.
        let other_pos = find_profile_button_center(&pw_shapes, "Sign In / Register");
        for (pressed, t) in [(true, 1.00), (false, 1.02)] {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(t),
                    events: vec![
                        egui::Event::PointerMoved(other_pos),
                        egui::Event::PointerButton {
                            pos: other_pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(&mut app, ui);
                },
            );
            out.drop_without_applying_deltas();
        }
        assert!(app.view.account.selector_other_server_open);
        let (other_text, _) = run_modal_frames(&ctx, &mut app, 1.10, 8);
        assert!(other_text.contains("Back to Accounts"), "\n{other_text}");
        assert!(other_text.contains("Email"), "\n{other_text}");
        assert!(
            !other_text.contains("Username or Email"),
            "must only allow email login:\n{other_text}"
        );
    }

    #[test]
    fn sign_in_rejects_username_only_and_accepts_email_only() {
        use crate::sync_worker::SyncWorkerCommand;

        let Err(err) = super::build_sign_in_command(
            0,
            "http://127.0.0.1:8787",
            "alex_nick",
            "SecretPass1!".to_string(),
            "Laptop".to_string(),
        ) else {
            panic!("username-only login must be rejected");
        };
        assert!(
            err.to_ascii_lowercase().contains("email"),
            "expected email-only validation error, got: {err}"
        );

        let Ok(cmd) = super::build_sign_in_command(
            0,
            "http://127.0.0.1:8787",
            "  Alex@Example.COM ",
            "SecretPass1!".to_string(),
            "Laptop".to_string(),
        ) else {
            panic!("valid email must produce login command");
        };
        match cmd {
            SyncWorkerCommand::Login { identifier, .. } => {
                assert_eq!(identifier, "alex@example.com");
            }
            _ => panic!("unexpected command variant"),
        }
    }

    #[test]
    fn successful_server_probe_shows_green_icon_and_resets_when_url_changes() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.session.prefs.theme = papercraft_doc::UiTheme::Light;
        crate::open_profile_sync_drawer(&mut app);

        app.view.account.apply_health_ok(
            DEFAULT_SERVER_URL.to_string(),
            ServerInfoResponse {
                status: "ok".to_string(),
                service: "papercraft-server".to_string(),
                version: "0.1.0".to_string(),
                protocol_version: 1,
                registration_open: true,
                storage_engine: "redb".to_string(),
                max_notebook_bytes: 1_048_576,
                local_discovery: LocalDiscoverySummary::default(),
            },
        );
        assert!(matches!(
            app.view.account.probe,
            ServerProbeState::Connected { .. }
        ));

        let mut text_out = String::new();
        let mut green_badges = 0usize;
        for step in 0..8 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f64::from(step) * 0.08),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                crate::inspector::show(&mut app, ui);
            });
            text_out = collect_text(&out.shapes);
            let light = crate::theme::Tokens::light();
            green_badges = count_filled_circles_with_color(&out.shapes, light.success_fg);
            out.drop_without_applying_deltas();
        }
        assert!(text_out.contains("Connected to PaperCraft Server v0.1.0"));
        assert!(text_out.contains("Registration open"));
        assert!(text_out.contains("Protocol v1"), "{text_out}");
        assert!(text_out.contains("Storage: redb"), "{text_out}");
        assert!(
            green_badges >= 2,
            "expected green connection icons in header/input/banner, got {green_badges}"
        );

        app.view.account.server_url = "http://100.64.0.10:8787".to_string();
        app.view.account.invalidate_probe_if_url_changed();
        assert_eq!(app.view.account.probe, ServerProbeState::Untested);
    }

    #[test]
    fn closed_registration_probe_flags_create_account_tab() {
        let mut app = PaperCraftApp::default();
        app.view.account.tab = AuthTab::CreateAccount;
        app.view.account.apply_health_ok(
            DEFAULT_SERVER_URL.to_string(),
            ServerInfoResponse {
                status: "ok".to_string(),
                service: "papercraft-server".to_string(),
                version: "0.1.0".to_string(),
                protocol_version: 1,
                registration_open: false,
                storage_engine: "redb".to_string(),
                max_notebook_bytes: 1_048_576,
                local_discovery: LocalDiscoverySummary::default(),
            },
        );
        assert!(app.view.account.is_registration_closed());

        app.view.account.server_url = "http://100.64.0.22:8787".to_string();
        app.view.account.invalidate_probe_if_url_changed();
        app.view
            .account
            .apply_health_err(DEFAULT_SERVER_URL.to_string(), "timeout".to_string());
        assert_eq!(app.view.account.probe, ServerProbeState::Untested);
    }

    #[test]
    fn dark_theme_and_create_account_with_banners_fit_without_clipping() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        let _ = app
            .session
            .execute("theme.set", &serde_json::json!({ "theme": "Dark" }));
        app.session
            .set_server_url_preference(DEFAULT_SERVER_URL)
            .unwrap();
        modal::open(&mut app, modal::ActiveModal::AccountSync);
        app.view.account.tab = AuthTab::CreateAccount;
        app.view.account.apply_health_ok(
            DEFAULT_SERVER_URL.to_string(),
            ServerInfoResponse {
                status: "ok".to_string(),
                service: "papercraft-server".to_string(),
                version: "0.1.0".to_string(),
                protocol_version: 1,
                registration_open: true,
                storage_engine: "redb".to_string(),
                max_notebook_bytes: 1_048_576,
                local_discovery: LocalDiscoverySummary::default(),
            },
        );
        app.view.account.error_message = Some("Email already taken".to_string());

        let mut text_out = String::new();
        for step in 0..6 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f64::from(step) * 0.08),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_visuals(ui.ctx(), papercraft_doc::UiTheme::Dark);
                modal::show(&mut app, ui);
            });
            text_out = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        assert!(
            text_out.contains("Display Name"),
            "missing Display Name in:\n{text_out}"
        );
        assert!(
            text_out.contains("Email already taken"),
            "missing error banner in:\n{text_out}"
        );
        assert!(
            !text_out.contains("Self-Hosted Server"),
            "server setup card must stay out of auth modal:\n{text_out}"
        );

        crate::open_profile_sync_drawer(&mut app);
        app.view.account.apply_health_ok(
            DEFAULT_SERVER_URL.to_string(),
            ServerInfoResponse {
                status: "ok".to_string(),
                service: "papercraft-server".to_string(),
                version: "0.1.0".to_string(),
                protocol_version: 1,
                registration_open: true,
                storage_engine: "redb".to_string(),
                max_notebook_bytes: 1_048_576,
                local_discovery: LocalDiscoverySummary::default(),
            },
        );
        let mut sync_text = String::new();
        let mut green_badges = 0usize;
        for step in 0..8 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(1.0 + f64::from(step) * 0.08),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_visuals(ui.ctx(), papercraft_doc::UiTheme::Dark);
                crate::inspector::show(&mut app, ui);
            });
            sync_text = collect_text(&out.shapes);
            let dark = crate::theme::Tokens::dark();
            green_badges = count_filled_circles_with_color(&out.shapes, dark.success_fg);
            out.drop_without_applying_deltas();
        }
        assert!(
            sync_text.contains("Connected to PaperCraft Server v0.1.0"),
            "missing Connected banner in Profile -> Sync:\n{sync_text}"
        );
        assert!(
            green_badges >= 2,
            "expected dark-theme green connection badges, got {green_badges}"
        );
    }

    #[test]
    fn account_modal_local_profile_create_switch_rename_and_delete_succeed() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Primary" }),
            )
            .unwrap();
        let primary_id = app
            .session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();

        let created = app
            .session
            .execute(
                "profile.create",
                &serde_json::json!({ "nickname": "test", "switch_now": true }),
            )
            .unwrap();
        let test_id = created["localProfileId"].as_str().unwrap().to_string();
        assert_eq!(app.session.active_local_profile().unwrap().nickname, "test");

        app.session
            .execute(
                "profile.switch",
                &serde_json::json!({ "target": "local", "profile_id": primary_id }),
            )
            .unwrap();
        assert_eq!(
            app.session.active_local_profile().unwrap().nickname,
            "Primary"
        );

        app.session
            .execute(
                "profile.rename",
                &serde_json::json!({ "profile_id": test_id, "nickname": "test2" }),
            )
            .unwrap();

        modal::open(&mut app, modal::ActiveModal::AccountSync);
        app.view.account.profiles_panel_open = true;
        let text = render_modal_settled(&ctx, &mut app);
        assert!(text.contains("On This Device: Primary"), "text:\n{text}");
        assert!(text.contains("test2"), "text:\n{text}");

        app.session
            .execute(
                "profile.delete",
                &serde_json::json!({ "profile_id": test_id }),
            )
            .unwrap();
        assert_eq!(app.session.local_profiles().len(), 1);
    }

    #[test]
    fn account_blocks_truncate_long_names_without_clipping_buttons() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Primary" }),
            )
            .unwrap();
        let long_name = "VeryLongOfflineProfileNameThatExceedsStandardBlockWidth";
        let _ = app
            .session
            .execute(
                "profile.create",
                &serde_json::json!({ "nickname": long_name, "switch_now": false }),
            )
            .unwrap();
        let acct = test_account(
            "u-long",
            "very.long.email.address.for.testing.truncation@example.com",
        );
        let pull = SyncPullResponse {
            server_cursor: 0,
            user: acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(acct, pull);
        crate::open_profile_drawer(&mut app);

        let mut text = String::new();
        for step in 0..8 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(f64::from(step) * 0.05),
                    predicted_dt: 0.05,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    crate::inspector::show(&mut app, ui);
                },
            );
            text = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        assert!(text.contains("Sync"), "missing Sync button:\n{text}");
        assert!(
            text.contains("Sign Out"),
            "missing Sign Out button:\n{text}"
        );
        assert!(text.contains("Switch"), "missing Switch button:\n{text}");
        assert!(text.contains("Delete"), "missing Delete button:\n{text}");
        assert!(
            !text.contains("Rename"),
            "inactive local card must omit Rename button:\n{text}"
        );
        assert!(
            text.contains("Add Account"),
            "missing Add Account button:\n{text}"
        );
    }

    #[test]
    fn inactive_local_account_card_shows_only_basic_info_and_actions() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Primary" }),
            )
            .unwrap();
        let _ = app
            .session
            .execute(
                "profile.create",
                &serde_json::json!({ "nickname": "Secondary", "switch_now": false }),
            )
            .unwrap();
        crate::open_profile_drawer(&mut app);

        let (text, shapes) = render_profile_frames(&ctx, &mut app, 0.0, 8);
        assert!(text.contains("Local Accounts (1)"), "\n{text}");
        assert!(text.contains("Secondary"), "\n{text}");
        assert!(text.contains("Switch"), "\n{text}");
        assert!(text.contains("Delete"), "\n{text}");
        assert!(
            !text.contains("Rename"),
            "inactive local card must not show Rename:\n{text}"
        );
        // Active hero card has "0 notebooks · Saved on this device", inactive card should not add a separate "0 notebooks" line.
        assert_eq!(
            text.lines()
                .filter(|line| line.trim() == "0 notebooks")
                .count(),
            0,
            "inactive local card must not show standalone notebook count:\n{text}"
        );

        click_profile_button(&ctx, &mut app, &shapes, "Switch", 0.50);
        assert_eq!(
            app.session
                .active_local_profile()
                .map(|p| p.nickname.as_str()),
            Some("Secondary")
        );
    }

    #[test]
    fn local_profile_delete_requires_explicit_confirmation_before_removing_profile() {
        fn find_all_text_centers(shapes: &[ClippedShape], target: &str) -> Vec<egui::Pos2> {
            fn visit(shape: &Shape, target: &str, out: &mut Vec<egui::Pos2>) {
                match shape {
                    Shape::Text(text) if text.galley.text() == target => {
                        out.push(text.visual_bounding_rect().center());
                    }
                    Shape::Vec(items) => {
                        for item in items {
                            visit(item, target, out);
                        }
                    }
                    _ => {}
                }
            }
            let mut out = Vec::new();
            for clipped in shapes {
                let mut found = Vec::new();
                visit(&clipped.shape, target, &mut found);
                for pos in found {
                    if clipped.clip_rect.contains(pos) {
                        out.push(pos);
                    }
                }
            }
            out
        }

        fn run_profile_screen(
            ctx: &egui::Context,
            app: &mut PaperCraftApp,
            events: Vec<egui::Event>,
            t0: &mut f64,
        ) -> (String, Vec<ClippedShape>) {
            *t0 += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(*t0),
                predicted_dt: 0.05,
                events,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                crate::inspector::show(app, ui);
            });
            let text = collect_text(&out.shapes);
            let shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
            (text, shapes)
        }

        fn click_pos(ctx: &egui::Context, app: &mut PaperCraftApp, pos: egui::Pos2, t0: &mut f64) {
            for pressed in [true, false] {
                let _ = run_profile_screen(
                    ctx,
                    app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    t0,
                );
            }
        }

        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Primary" }),
            )
            .unwrap();
        let created = app
            .session
            .execute(
                "profile.create",
                &serde_json::json!({ "nickname": "SideNotes", "switch_now": false }),
            )
            .unwrap();
        let side_id = created["localProfileId"].as_str().unwrap().to_string();
        assert_eq!(app.session.accessible_local_profiles().len(), 2);

        crate::open_profile_drawer(&mut app);
        app.view.account.profile_section = super::ProfileSection::Accounts;

        fn find_delete_btn_near(shapes: &[ClippedShape], label: &str) -> egui::Pos2 {
            let target_y = find_all_text_centers(shapes, label)
                .into_iter()
                .next()
                .expect("target profile label")
                .y;
            find_all_text_centers(shapes, "Delete")
                .into_iter()
                .filter(|p| p.y >= target_y)
                .min_by(|a, b| {
                    (a.y - target_y)
                        .partial_cmp(&(b.y - target_y))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("Delete button for target profile")
        }

        let mut t = 0.0_f64;
        let mut shapes = Vec::new();
        for _ in 0..8 {
            shapes = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t).1;
        }

        // 1. Clicking Delete opens typed `I agree <name>` confirmation and keeps both profiles intact.
        click_pos(
            &ctx,
            &mut app,
            find_delete_btn_near(&shapes, "SideNotes"),
            &mut t,
        );
        assert_eq!(
            app.view.account.pending_delete_profile_id.as_deref(),
            Some(side_id.as_str())
        );
        assert_eq!(app.session.accessible_local_profiles().len(), 2);

        let _ = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        let (confirm_text, _) = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        assert!(
            confirm_text.contains("Delete Local Profile?"),
            "\n{confirm_text}"
        );
        assert!(
            confirm_text.contains("I agree SideNotes"),
            "\n{confirm_text}"
        );
        assert!(confirm_text.contains("Delete Profile"), "\n{confirm_text}");

        // 2. Paste attempts (Event::Paste and Ctrl+V) must be blocked with a clear hint.
        let _ = run_profile_screen(
            &ctx,
            &mut app,
            vec![
                egui::Event::Paste("I agree SideNotes".to_string()),
                egui::Event::Key {
                    key: egui::Key::V,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::CTRL,
                },
            ],
            &mut t,
        );
        let (paste_text, _) = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        assert!(app.view.account.delete_profile_confirm_input.is_empty());
        assert!(app.view.account.delete_profile_paste_blocked);
        assert!(
            paste_text.contains("Copy/paste is disabled — type it manually"),
            "missing paste-blocked warning:\n{paste_text}"
        );

        // 3. Wrong manual phrase keeps Delete Profile disabled and preserves SideNotes.
        let _ = run_profile_screen(
            &ctx,
            &mut app,
            vec![egui::Event::Text("yes delete SideNotes".to_string())],
            &mut t,
        );
        let (_, wrong_shapes) = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        let disabled_btn = find_all_text_centers(&wrong_shapes, "Delete Profile")
            .into_iter()
            .next()
            .expect("Delete Profile button");
        click_pos(&ctx, &mut app, disabled_btn, &mut t);
        assert_eq!(app.session.accessible_local_profiles().len(), 2);
        assert_eq!(
            app.view.account.pending_delete_profile_id.as_deref(),
            Some(side_id.as_str())
        );

        // 4. Clicking Cancel clears confirmation input/flags and preserves SideNotes.
        let (_, cancel_shapes) = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        let cancel_btn = find_all_text_centers(&cancel_shapes, "Cancel")
            .into_iter()
            .next()
            .expect("Cancel button");
        click_pos(&ctx, &mut app, cancel_btn, &mut t);
        assert!(app.view.account.pending_delete_profile_id.is_none());
        assert!(app.view.account.delete_profile_confirm_input.is_empty());
        assert!(!app.view.account.delete_profile_paste_blocked);
        assert_eq!(app.session.accessible_local_profiles().len(), 2);

        // 5. Re-opening confirmation, typing exact `I agree SideNotes`, and confirming deletes SideNotes.
        let (_, shapes_reopen) = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        click_pos(
            &ctx,
            &mut app,
            find_delete_btn_near(&shapes_reopen, "SideNotes"),
            &mut t,
        );
        let _ = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        let _ = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        let _ = run_profile_screen(
            &ctx,
            &mut app,
            vec![egui::Event::Text(local_profile_delete_confirmation_phrase(
                "SideNotes",
            ))],
            &mut t,
        );
        let _ = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        let (_, shapes_confirm2) = run_profile_screen(&ctx, &mut app, Vec::new(), &mut t);
        assert_eq!(
            app.view.account.delete_profile_confirm_input,
            local_profile_delete_confirmation_phrase("SideNotes")
        );
        assert!(!app.view.account.delete_profile_paste_blocked);
        let confirm_btn = find_all_text_centers(&shapes_confirm2, "Delete Profile")
            .into_iter()
            .next()
            .expect("enabled Delete Profile button");
        click_pos(&ctx, &mut app, confirm_btn, &mut t);

        assert!(app.view.account.pending_delete_profile_id.is_none());
        assert!(app.view.account.delete_profile_confirm_input.is_empty());
        assert_eq!(app.session.accessible_local_profiles().len(), 1);
        assert_eq!(
            app.view.account.status_message.as_deref(),
            Some("Deleted local profile 'SideNotes'.")
        );
    }

    #[test]
    fn expired_session_renders_inline_reauth_form_in_profile() {
        let ctx = egui::Context::default();
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = None;
        let acct = test_account("u-reauth-ui", "reauth@example.com");
        let pull = SyncPullResponse {
            server_cursor: 1,
            user: acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(acct, pull);
        app.session
            .execute(
                "notebook.create",
                &serde_json::json!({ "title": "Offline Before Expire" }),
            )
            .unwrap();
        app.session
            .record_session_expired("Session expired".to_string());
        crate::open_profile_drawer(&mut app);

        let (text, _) = render_profile_frames(&ctx, &mut app, 0.0, 8);
        assert!(
            text.contains("Session Expired — Sign In Again"),
            "missing inline expired-session header:\n{text}"
        );
        assert!(
            text.contains("1 offline change is queued locally"),
            "missing pending offline count in re-auth box:\n{text}"
        );
        assert!(
            text.contains("Sign In & Sync"),
            "missing Sign In & Sync button:\n{text}"
        );
    }

    #[test]
    fn startup_account_picker_respects_option_a_dismissibility_and_toggle() {
        let ctx = egui::Context::default();
        let mut session = papercraft_engine::Session::open_web_cache(None);
        session.dismiss_welcome_banner();
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());

        assert_eq!(
            app.view.active_modal,
            Some(modal::ActiveModal::FirstAccessNickname)
        );
        assert!(app.session.needs_initial_nickname());
        assert!(!app.session.has_any_saved_accounts());
        assert!(!app.session.should_show_startup_account_picker());

        let initial_text = render_modal_settled(&ctx, &mut app);
        assert!(
            initial_text.contains("Welcome to PaperCraft"),
            "{initial_text}"
        );
        assert!(initial_text.contains("Start Session"), "{initial_text}");

        // Create the first local account -> Option A now shows the account picker on subsequent launches.
        crate::sync_worker::create_local_account_and_switch(&mut app, &ctx, "LocalOne").unwrap();
        assert!(!app.session.needs_initial_nickname());
        assert!(app.session.has_any_saved_accounts());
        assert!(app.session.has_loaded_workspace());
        assert!(app.session.should_show_startup_account_picker());

        // Re-open picker as Switch Account -> now dismissible, shows Local badge, Active pill, and Option A switch.
        modal::open(&mut app, modal::ActiveModal::SelectLocalAccount);
        assert!(modal::is_account_picker_dismissible(&app));
        let switch_text = render_modal_settled(&ctx, &mut app);
        assert!(switch_text.contains("Switch Account"), "{switch_text}");
        assert!(switch_text.contains("LocalOne"), "{switch_text}");
        assert!(switch_text.contains("Active"), "{switch_text}");
        assert!(switch_text.contains("Continue"), "{switch_text}");
        assert!(switch_text.contains("Cancel"), "{switch_text}");
        assert!(
            switch_text.contains("Remember last account and skip on startup"),
            "{switch_text}"
        );

        // Toggling startup preference off updates the engine session immediately.
        let mut shapes = Vec::new();
        for step in 0..4 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(0.5 + f64::from(step) * 0.04),
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(&mut app, ui);
                },
            );
            shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
        }
        let check_pos =
            find_profile_button_center(&shapes, "Remember last account and skip on startup");
        for (pressed, t) in [(true, 0.7), (false, 0.74)] {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(t),
                    events: vec![
                        egui::Event::PointerMoved(check_pos),
                        egui::Event::PointerButton {
                            pos: check_pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(&mut app, ui);
                },
            );
            out.drop_without_applying_deltas();
        }
        assert!(app.session.skip_account_picker_on_startup());
        assert!(!app.session.should_show_startup_account_picker());
    }

    #[test]
    fn direct_cloud_switch_with_pending_edits_warns_and_preserves_cache() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let ctx = egui::Context::default();
        let login_dispatches = Arc::new(AtomicUsize::new(0));
        let login_clone = Arc::clone(&login_dispatches);
        let hooks = crate::PlatformHooks {
            web_sync_dispatch: Some(Box::new(move |cmd, _tx, _ctx| {
                if matches!(cmd, crate::sync_worker::SyncWorkerCommand::Login { .. }) {
                    login_clone.fetch_add(1, Ordering::Relaxed);
                }
            })),
            ..crate::PlatformHooks::default()
        };
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, hooks);
        app.view.active_modal = None;

        let acct_a = test_account("u-cloud-a", "alice@example.com");
        let pull_a = SyncPullResponse {
            server_cursor: 1,
            user: acct_a.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(acct_a, pull_a);
        app.session
            .execute(
                "notebook.create",
                &serde_json::json!({ "title": "Unsynced Sketch" }),
            )
            .unwrap();
        assert_eq!(app.session.cloud_pending_changes_count(), 1);

        modal::open(&mut app, modal::ActiveModal::AddAccount);
        app.view.account.add_account_mode = super::AddAccountMode::Server;
        app.view.account.email_or_username = "bob@example.com".to_string();
        app.view.account.password = "SecretPass1!".to_string();

        // Trying to switch to Bob sets pending_unsynced_switch instead of dispatching Login immediately.
        super::submit_auth_request(&mut app, &ctx);
        assert_eq!(login_dispatches.load(Ordering::Relaxed), 0);
        assert!(app.view.account.pending_unsynced_switch.is_some());

        let mut warn_text = String::new();
        let mut warn_shapes = Vec::new();
        for step in 0..8 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(f64::from(step) * 0.04),
                    predicted_dt: 0.04,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(&mut app, ui);
                },
            );
            warn_text = collect_text(&out.shapes);
            warn_shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
        }
        assert!(
            warn_text.contains("Unsynced Changes on Current Account"),
            "{warn_text}"
        );
        assert!(warn_text.contains("Sync Now First"), "{warn_text}");
        assert!(warn_text.contains("Switch Anyway"), "{warn_text}");

        let btn_pos = find_profile_button_center(&warn_shapes, "Switch Anyway");
        for (pressed, offset) in [(true, 0.40), (false, 0.42)] {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(offset),
                    events: vec![
                        egui::Event::PointerMoved(btn_pos),
                        egui::Event::PointerButton {
                            pos: btn_pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(&mut app, ui);
                },
            );
            out.drop_without_applying_deltas();
        }
        assert!(app.view.account.pending_unsynced_switch.is_none());
        assert_eq!(login_dispatches.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn self_hosted_server_card_renders_website_download_link_and_opens_papercraft_ink() {
        use std::sync::{Arc, Mutex};

        let opened: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let opened_hook = Arc::clone(&opened);
        let hooks = crate::PlatformHooks {
            open_url: Some(Box::new(move |url| {
                opened_hook.lock().unwrap().push(url.to_string());
            })),
            ..crate::PlatformHooks::default()
        };
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::new(papercraft_engine::Session::default(), hooks);
        crate::open_profile_drawer(&mut app);
        app.view.account.profile_section = ProfileSection::Sync;

        let (text, shapes) = render_profile_frames(&ctx, &mut app, 0.0, 12);
        assert!(
            text.contains("Download PaperCraft Server for your platform (papercraft.ink)"),
            "{text}"
        );
        click_profile_button(
            &ctx,
            &mut app,
            &shapes,
            "Download PaperCraft Server for your platform (papercraft.ink)",
            0.35,
        );
        assert_eq!(
            *opened.lock().unwrap(),
            vec![crate::WEBSITE_URL.to_string()]
        );
    }

    #[test]
    fn accounts_tab_stacks_cleanly_in_drawer_and_picker_splits_responsively() {
        let ctx = egui::Context::default();
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.view.active_modal = None;
        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "LocalOne" }),
            )
            .unwrap();
        let acct = test_account("u-split-srv", "cloud.one@example.com");
        let pull = SyncPullResponse {
            server_cursor: 0,
            user: acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(acct, pull);
        app.session.sign_out_from_server_session();
        crate::open_profile_drawer(&mut app);

        for (idx, (theme, w)) in [
            (papercraft_doc::UiTheme::Light, 1280.0_f32),
            (papercraft_doc::UiTheme::Dark, 1280.0_f32),
            (papercraft_doc::UiTheme::Light, 360.0_f32),
        ]
        .into_iter()
        .enumerate()
        {
            let _ = app.session.execute(
                "theme.set",
                &serde_json::json!({ "theme": if theme == papercraft_doc::UiTheme::Light { "Light" } else { "Dark" } }),
            );
            let mut shapes = Vec::new();
            let mut text = String::new();
            for step in 0..8 {
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(w, 800.0),
                        )),
                        time: Some(idx as f64 + f64::from(step) * 0.05),
                        predicted_dt: 0.05,
                        ..Default::default()
                    },
                    |ui| {
                        crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                        crate::inspector::show(&mut app, ui);
                    },
                );
                shapes = out.shapes.clone();
                text = collect_text(&shapes);
                out.drop_without_applying_deltas();
            }
            let expected_add_lbl = if w >= 480.0 { "Add Account" } else { "Add" };
            assert!(text.contains(expected_add_lbl), "{text}");
            let local_one = find_profile_button_center(&shapes, "LocalOne");
            let add_btn = find_profile_button_center(&shapes, expected_add_lbl);
            let local = find_profile_button_center(&shapes, "Local Accounts (0)");
            let server = find_profile_button_center(&shapes, "Server Accounts (1)");
            assert!(
                add_btn.y < local_one.y && local_one.y < local.y,
                "expected Add Account in header above active card above inactive lists ({theme:?}, w={w}): add={add_btn:?}, active={local_one:?}, local={local:?}\n{text}"
            );
            assert!(
                local.y + 40.0 < server.y,
                "expected drawer to stack Local above Server ({theme:?}, w={w}): local={local:?}, server={server:?}\n{text}"
            );
            assert!(add_btn.x <= w, "Add Account overflowed {w}: {add_btn:?}");
        }

        // Choose/Switch Account modal also splits Local (left) and Server (right) at 1280x800.
        modal::open(&mut app, modal::ActiveModal::SelectLocalAccount);
        let mut modal_shapes = Vec::new();
        for step in 0..8 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(4.0 + f64::from(step) * 0.05),
                    predicted_dt: 0.05,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                    modal::show(&mut app, ui);
                },
            );
            modal_shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
        }
        let local_m = find_profile_button_center(&modal_shapes, "Local Accounts (1)");
        let server_m = find_profile_button_center(&modal_shapes, "Server Accounts (1)");
        assert!(
            local_m.x + 120.0 < server_m.x,
            "expected modal Local column left of Server column: local={local_m:?}, server={server_m:?}"
        );
        assert!(
            (local_m.y - server_m.y).abs() < 24.0,
            "expected modal split column headers aligned horizontally: local={local_m:?}, server={server_m:?}"
        );
    }

    #[test]
    fn add_account_modal_switches_cleanly_and_creates_local_account() {
        let ctx = egui::Context::default();
        let session = papercraft_engine::Session::open_web_cache(None);
        let mut app = PaperCraftApp::new(session, crate::PlatformHooks::default());
        app.session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Personal" }),
            )
            .unwrap();
        crate::open_profile_drawer(&mut app);

        let (_, drawer_shapes) = render_profile_frames(&ctx, &mut app, 0.0, 8);
        click_profile_button(&ctx, &mut app, &drawer_shapes, "Add Account", 0.35);
        assert_eq!(app.view.active_modal, Some(modal::ActiveModal::AddAccount));
        assert_eq!(
            app.view.account.add_account_mode,
            super::AddAccountMode::Server
        );

        fn render_modal_at(ctx: &egui::Context, app: &mut PaperCraftApp, start_t: f64) -> String {
            let mut text = String::new();
            for step in 0..6 {
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 800.0),
                        )),
                        time: Some(start_t + f64::from(step) * 0.05),
                        predicted_dt: 0.05,
                        ..Default::default()
                    },
                    |ui| {
                        crate::theme::apply_light_visuals(ui.ctx());
                        modal::show(app, ui);
                    },
                );
                text = collect_text(&out.shapes);
                out.drop_without_applying_deltas();
            }
            text
        }

        let server_text = render_modal_at(&ctx, &mut app, 0.50);
        assert!(server_text.contains("Server Account"), "{server_text}");
        assert!(server_text.contains("Local Account"), "{server_text}");
        assert!(!server_text.contains("Self-Hosted Server"), "{server_text}");
        assert!(
            server_text.contains("Sync Server Not Configured"),
            "{server_text}"
        );
        assert!(server_text.contains("Open Sync Settings"), "{server_text}");

        app.view.account.add_account_mode = super::AddAccountMode::Local;
        let local_text = render_modal_at(&ctx, &mut app, 1.00);
        assert!(local_text.contains("New Local Account"), "{local_text}");
        assert!(local_text.contains("Create Local Account"), "{local_text}");

        app.view.account.new_profile_name = "StudioWork".to_string();
        assert!(super::submit_add_local_account(&mut app, &ctx));
        assert_eq!(
            app.session
                .active_local_profile()
                .map(|p| p.nickname.as_str()),
            Some("StudioWork")
        );
    }

    #[test]
    fn account_icon_and_colors_distinguish_local_online_and_offline_server() {
        for t in [crate::theme::Tokens::light(), crate::theme::Tokens::dark()] {
            let (local_icon, local_fg, _, _) = super::account_icon_and_colors(false, false, &t);
            assert_eq!(local_icon, crate::icons::Icon::Account);
            assert_eq!(local_fg, t.accent);

            let (srv_on_icon, srv_on_fg, _, _) = super::account_icon_and_colors(true, true, &t);
            assert_eq!(srv_on_icon, crate::icons::Icon::Server);
            assert_eq!(srv_on_fg, t.success_fg);

            let (srv_off_icon, srv_off_fg, _, _) = super::account_icon_and_colors(true, false, &t);
            assert_eq!(srv_off_icon, crate::icons::Icon::Server);
            assert_ne!(srv_off_fg, srv_on_fg);
            assert!(srv_off_fg.a() < 255);

            let (avatar_on_icon, avatar_on_fg, _, _) =
                super::account_avatar_icon_and_colors(true, true, &t);
            assert_eq!(avatar_on_icon, crate::icons::Icon::Account);
            assert_eq!(avatar_on_fg, t.success_fg);

            let (avatar_off_icon, avatar_off_fg, _, _) =
                super::account_avatar_icon_and_colors(true, false, &t);
            assert_eq!(avatar_off_icon, crate::icons::Icon::Account);
            assert_eq!(avatar_off_fg, srv_off_fg);
        }
    }

    #[test]
    fn unconfigured_server_blocks_login_and_links_to_profile_sync_tab() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let ctx = egui::Context::default();
        let login_dispatches = Arc::new(AtomicUsize::new(0));
        let login_clone = Arc::clone(&login_dispatches);
        let hooks = crate::PlatformHooks {
            web_sync_dispatch: Some(Box::new(move |cmd, _tx, _ctx| {
                if matches!(cmd, crate::sync_worker::SyncWorkerCommand::Login { .. }) {
                    login_clone.fetch_add(1, Ordering::Relaxed);
                }
            })),
            ..crate::PlatformHooks::default()
        };
        let mut session = papercraft_engine::Session::open_web_cache(None);
        session
            .execute(
                "profile.completeFirstAccess",
                &serde_json::json!({ "nickname": "Solo" }),
            )
            .unwrap();
        let prof_id = session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();
        let mut app = PaperCraftApp::new(session, hooks);
        assert!(!app.session.is_server_configured());

        // Attempting Sign In before configuring Sync must fail gracefully with a Sync guidance message.
        modal::open(&mut app, modal::ActiveModal::AddAccount);
        app.view.account.add_account_mode = super::AddAccountMode::Server;
        app.view.account.email_or_username = "alice@example.com".to_string();
        app.view.account.password = "SecretPass1!".to_string();
        super::submit_auth_request(&mut app, &ctx);
        assert_eq!(login_dispatches.load(Ordering::Relaxed), 0);
        let err_msg = app.view.account.error_message.clone().unwrap_or_default();
        assert!(err_msg.contains("Profile → Sync"), "{err_msg}");

        // ConvertLocalAccount must also block with the same Sync guidance when unconfigured.
        app.view.account.convert_source_profile_id = prof_id;
        app.view.account.convert_source_nickname = "Solo".to_string();
        app.view.account.convert_email = "alice@example.com".to_string();
        app.view.account.convert_password = "SecretPass1!".to_string();
        super::submit_convert_local_account(&mut app, &ctx);
        let conv_err = app.view.account.convert_error.clone().unwrap_or_default();
        assert!(conv_err.contains("Profile → Sync"), "{conv_err}");

        // Rendering AddAccount shows the Sync reminder without Self-Hosted Server inputs,
        // and clicking Open Sync Settings switches straight into Profile -> Sync.
        let mut shapes = Vec::new();
        let mut modal_text = String::new();
        for step in 0..8 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(f64::from(step) * 0.05),
                    predicted_dt: 0.05,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(&mut app, ui);
                },
            );
            shapes = out.shapes.clone();
            modal_text = collect_text(&shapes);
            out.drop_without_applying_deltas();
        }
        assert!(!modal_text.contains("Self-Hosted Server"), "{modal_text}");
        assert!(
            modal_text.contains("Sync Server Not Configured"),
            "{modal_text}"
        );
        let btn_pos = find_profile_button_center(&shapes, "Open Sync Settings");
        for (pressed, offset) in [(true, 0.50), (false, 0.52)] {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(offset),
                    events: vec![
                        egui::Event::PointerMoved(btn_pos),
                        egui::Event::PointerButton {
                            pos: btn_pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(&mut app, ui);
                },
            );
            out.drop_without_applying_deltas();
        }
        assert!(app.view.active_modal.is_none());
        assert!(app.view.inspector_open);
        assert_eq!(app.view.screen, crate::AppScreen::Home);
        assert_eq!(app.view.account.profile_section, ProfileSection::Sync);

        // Once server URL is configured in Sync, AddAccount hides the notice and Login dispatches cleanly.
        app.session
            .set_server_url_preference(DEFAULT_SERVER_URL)
            .unwrap();
        assert!(app.session.is_server_configured());
        modal::open(&mut app, modal::ActiveModal::AddAccount);
        app.view.account.add_account_mode = super::AddAccountMode::Server;
        let mut configured_text = String::new();
        for step in 0..6 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(1.0 + f64::from(step) * 0.08),
                    predicted_dt: 0.08,
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    modal::show(&mut app, ui);
                },
            );
            configured_text = collect_text(&out.shapes);
            out.drop_without_applying_deltas();
        }
        assert!(
            !configured_text.contains("Sync Server Not Configured"),
            "{configured_text}"
        );
        assert!(
            !configured_text.contains("Self-Hosted Server"),
            "{configured_text}"
        );
        app.view.account.email_or_username = "alice@example.com".to_string();
        app.view.account.password = "SecretPass1!".to_string();
        super::submit_auth_request(&mut app, &ctx);
        assert_eq!(login_dispatches.load(Ordering::Relaxed), 1);
    }
}
