//! Non-blocking background update worker for GUI shells.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use crate::UpdateError;
use crate::client::{
    AvailableUpdate, CheckOutcome, UpdateConfig, check_for_update, download_and_verify_update,
};
use crate::install::{StagedUpdate, apply_staged_update_and_relaunch_with_shortcut};
use crate::manifest::{AppId, DEFAULT_RELEASES_PAGE_URL};
use crate::version::SemVer;

/// UI-observable state machine for update discovery, download, and installation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking {
        silent: bool,
    },
    UpToDate {
        latest_version: SemVer,
    },
    Available(Box<AvailableUpdate>),
    Downloading {
        update: Box<AvailableUpdate>,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    ReadyToInstall {
        update: Box<AvailableUpdate>,
        staged: StagedUpdate,
    },
    NoPlatformAsset {
        latest_version: SemVer,
        release_url: String,
        platform: String,
    },
    Error {
        message: String,
        release_url: String,
        was_silent: bool,
    },
}

enum WorkerEvent {
    CheckDone {
        generation: u64,
        silent: bool,
        result: Result<CheckOutcome, UpdateError>,
    },
    DownloadProgress {
        generation: u64,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    DownloadDone {
        generation: u64,
        update: Box<AvailableUpdate>,
        result: Result<StagedUpdate, UpdateError>,
    },
}

/// Manages background manifest checks and verified binary downloads for one desktop app.
pub struct UpdateWorker {
    config: Result<UpdateConfig, String>,
    status: UpdateStatus,
    tx: mpsc::Sender<WorkerEvent>,
    rx: mpsc::Receiver<WorkerEvent>,
    cancel_flag: Arc<AtomicBool>,
    generation: u64,
}

impl UpdateWorker {
    #[must_use]
    pub fn new(app: AppId, compiled_version: &str) -> Self {
        let (tx, rx) = mpsc::channel();
        let config = UpdateConfig::from_env(app, compiled_version).map_err(|e| e.to_string());
        Self {
            config,
            status: UpdateStatus::Idle,
            tx,
            rx,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            generation: 0,
        }
    }

    #[must_use]
    pub fn config(&self) -> Option<&UpdateConfig> {
        self.config.as_ref().ok()
    }

    #[must_use]
    pub fn config_error(&self) -> Option<&str> {
        self.config.as_ref().err().map(String::as_str)
    }

    #[must_use]
    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(
            self.status,
            UpdateStatus::Checking { .. } | UpdateStatus::Downloading { .. }
        )
    }

    /// True when an update is available, actively downloading, or staged and ready to restart.
    #[must_use]
    pub fn has_actionable_update(&self) -> bool {
        matches!(
            self.status,
            UpdateStatus::Available(_)
                | UpdateStatus::Downloading { .. }
                | UpdateStatus::ReadyToInstall { .. }
        )
    }

    #[must_use]
    pub fn available_version(&self) -> Option<&SemVer> {
        match &self.status {
            UpdateStatus::Available(u) | UpdateStatus::Downloading { update: u, .. } => {
                Some(&u.version)
            }
            UpdateStatus::ReadyToInstall { update, .. } => Some(&update.version),
            _ => None,
        }
    }

    /// Starts a background manifest check, notifying the UI via `notify` when complete.
    pub fn check_now(&mut self, silent: bool, notify: Arc<dyn Fn() + Send + Sync>) {
        if self.is_busy() {
            return;
        }
        let cfg = match &self.config {
            Ok(cfg) => cfg.clone(),
            Err(err) => {
                self.status = UpdateStatus::Error {
                    message: err.clone(),
                    release_url: DEFAULT_RELEASES_PAGE_URL.to_string(),
                    was_silent: silent,
                };
                return;
            }
        };

        self.generation = self.generation.saturating_add(1);
        let run_gen = self.generation;
        self.status = UpdateStatus::Checking { silent };
        let tx = self.tx.clone();

        let spawn_res = std::thread::Builder::new()
            .name("papercraft-update-check".to_string())
            .spawn(move || {
                let result = check_for_update(&cfg);
                let _ = tx.send(WorkerEvent::CheckDone {
                    generation: run_gen,
                    silent,
                    result,
                });
                notify();
            });
        if let Err(err) = spawn_res {
            self.status = UpdateStatus::Error {
                message: format!("Could not start update check thread: {err}"),
                release_url: DEFAULT_RELEASES_PAGE_URL.to_string(),
                was_silent: silent,
            };
        }
    }

    /// Starts downloading the currently available update in the background.
    pub fn start_download(&mut self, notify: Arc<dyn Fn() + Send + Sync>) {
        let update = match &self.status {
            UpdateStatus::Available(u) => u.clone(),
            _ => return,
        };
        let cfg = match &self.config {
            Ok(cfg) => cfg.clone(),
            Err(err) => {
                self.status = UpdateStatus::Error {
                    message: err.clone(),
                    release_url: update.release_url,
                    was_silent: false,
                };
                return;
            }
        };

        self.cancel_flag.store(false, Ordering::Relaxed);
        let cancel = Arc::clone(&self.cancel_flag);
        self.generation = self.generation.saturating_add(1);
        let run_gen = self.generation;
        let total_bytes = update.asset.size_bytes;
        self.status = UpdateStatus::Downloading {
            update: update.clone(),
            downloaded_bytes: 0,
            total_bytes,
        };

        let tx = self.tx.clone();
        let spawn_res = std::thread::Builder::new()
            .name("papercraft-update-download".to_string())
            .spawn(move || {
                let mut last_notified = 0_u64;
                let tx_progress = tx.clone();
                let notify_progress = Arc::clone(&notify);
                let result = download_and_verify_update(&update, &cfg, &cancel, |done, total| {
                    if done == 0 || done == total || done.saturating_sub(last_notified) >= 262_144 {
                        last_notified = done;
                        let _ = tx_progress.send(WorkerEvent::DownloadProgress {
                            generation: run_gen,
                            downloaded_bytes: done,
                            total_bytes: total,
                        });
                        notify_progress();
                    }
                });
                let _ = tx.send(WorkerEvent::DownloadDone {
                    generation: run_gen,
                    update,
                    result,
                });
                notify();
            });
        if let Err(err) = spawn_res {
            self.status = UpdateStatus::Error {
                message: format!("Could not start update download thread: {err}"),
                release_url: DEFAULT_RELEASES_PAGE_URL.to_string(),
                was_silent: false,
            };
        }
    }

    /// Cancels an active download and restores the `Available` state when possible.
    pub fn cancel_download(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
        if let UpdateStatus::Downloading { update, .. } = &self.status {
            self.generation = self.generation.saturating_add(1);
            self.status = UpdateStatus::Available(update.clone());
        }
    }

    /// Drains pending background worker events and returns `true` if status changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                WorkerEvent::CheckDone {
                    generation,
                    silent,
                    result,
                } if generation == self.generation => {
                    changed = true;
                    self.apply_check_result(silent, result);
                }
                WorkerEvent::DownloadProgress {
                    generation,
                    downloaded_bytes,
                    total_bytes,
                } if generation == self.generation => {
                    if let UpdateStatus::Downloading {
                        downloaded_bytes: ref mut cur_done,
                        total_bytes: ref mut cur_total,
                        ..
                    } = self.status
                    {
                        *cur_done = downloaded_bytes;
                        if total_bytes > 0 {
                            *cur_total = total_bytes;
                        }
                        changed = true;
                    }
                }
                WorkerEvent::DownloadDone {
                    generation,
                    update,
                    result,
                } if generation == self.generation => {
                    changed = true;
                    match result {
                        Ok(staged) => {
                            self.status = UpdateStatus::ReadyToInstall { update, staged };
                        }
                        Err(UpdateError::Cancelled) => {
                            self.status = UpdateStatus::Available(update);
                        }
                        Err(err) => {
                            self.status = UpdateStatus::Error {
                                message: err.to_string(),
                                release_url: update.release_url,
                                was_silent: false,
                            };
                        }
                    }
                }
                _ => {}
            }
        }
        changed
    }

    fn apply_check_result(&mut self, silent: bool, result: Result<CheckOutcome, UpdateError>) {
        match result {
            Ok(CheckOutcome::UpToDate { latest_version }) => {
                self.status = UpdateStatus::UpToDate { latest_version };
            }
            Ok(CheckOutcome::UpdateAvailable(update)) => {
                self.status = UpdateStatus::Available(update);
            }
            Ok(CheckOutcome::NoPlatformAsset {
                latest_version,
                release_url,
                platform,
            }) => {
                self.status = UpdateStatus::NoPlatformAsset {
                    latest_version,
                    release_url,
                    platform,
                };
            }
            Err(UpdateError::FeedNotPublished) if silent => {
                log::debug!("Update manifest has not been published on GitHub Releases yet");
                self.status = UpdateStatus::Idle;
            }
            Err(err) => {
                if silent {
                    log::debug!("Background update check skipped/failed: {err}");
                }
                self.status = UpdateStatus::Error {
                    message: err.to_string(),
                    release_url: DEFAULT_RELEASES_PAGE_URL.to_string(),
                    was_silent: silent,
                };
            }
        }
    }

    /// Replaces the running binary with the verified staged update and relaunches the app.
    pub fn apply_ready_update_and_relaunch(&mut self) -> Result<(), String> {
        self.apply_ready_update_and_relaunch_with_shortcut(false)
    }

    /// Replaces the running binary with the verified staged update, optionally refreshes the OS
    /// Desktop shortcut, and relaunches the app.
    pub fn apply_ready_update_and_relaunch_with_shortcut(
        &mut self,
        sync_desktop_shortcut: bool,
    ) -> Result<(), String> {
        let staged = match &self.status {
            UpdateStatus::ReadyToInstall { staged, .. } => staged.clone(),
            _ => return Err("No verified update is currently staged to install.".to_string()),
        };
        apply_staged_update_and_relaunch_with_shortcut(&staged, sync_desktop_shortcut).map_err(
            |err| {
                let msg = err.to_string();
                self.status = UpdateStatus::Error {
                    message: msg.clone(),
                    release_url: DEFAULT_RELEASES_PAGE_URL.to_string(),
                    was_silent: false,
                };
                msg
            },
        )
    }
}

impl Drop for UpdateWorker {
    fn drop(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
}
