// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
// Included by ui.rs. Pure request bookkeeping prevents optimistic ownership /
// configuration claims and duplicate submissions while the daemon is working.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RequestKind {
    Handover,
    Configure,
}

#[derive(Debug, PartialEq, Eq)]
enum SavedDraft {
    Bulk {
        submitted: String,
        remaining: String,
    },
    Profile {
        id: String,
        affinity: String,
        priority: i32,
        plan: String,
    },
}

#[derive(Default)]
struct Requests {
    serial: u64,
    pending: Option<(u64, RequestKind)>,
    draft: Option<SavedDraft>,
}
impl Requests {
    fn begin(&mut self, kind: RequestKind) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        self.serial = self.serial.checked_add(1)?;
        self.pending = Some((self.serial, kind));
        self.draft = None;
        Some(self.serial)
    }
    fn finish(&mut self, receipt: u64) -> Option<(RequestKind, Option<SavedDraft>)> {
        if self.pending.is_some_and(|p| p.0 == receipt) {
            self.pending.take().map(|p| (p.1, self.draft.take()))
        } else {
            None
        }
    }
    fn is_pending(&self, kind: RequestKind) -> bool {
        self.pending.is_some_and(|p| p.1 == kind)
    }
}

impl Controller {
    fn finish_saved_draft(&self, draft: SavedDraft) {
        let Some(ui) = self.ui.upgrade() else { return };
        match draft {
            SavedDraft::Bulk {
                submitted,
                remaining,
            } if ui.get_bulk_text() == submitted => {
                ui.set_bulk_text(remaining.into());
                self.bulk(&ui.get_bulk_text());
            }
            SavedDraft::Profile {
                id,
                affinity,
                priority,
                plan,
            } if ui.get_modal() == 3
                && self.selected == id
                && ui.get_affinity_text() == affinity
                && ui.get_priority_index() == priority
                && self.edit_plan == plan =>
            {
                ui.set_modal(0)
            }
            _ => {} // Never discard a newer edit made while awaiting the receipt.
        }
    }

    fn saved_draft(&self, draft: SavedDraft) {
        let mut requests = self.requests.borrow_mut();
        if requests.is_pending(RequestKind::Configure) {
            requests.draft = Some(draft);
        } else {
            // An unchanged, already-saved configuration did not need a request.
            drop(requests);
            self.finish_saved_draft(draft);
        }
    }

    fn sync_request_state(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let requests = self.requests.borrow();
        ui.set_handover_pending(requests.is_pending(RequestKind::Handover));
        ui.set_configuration_pending(requests.is_pending(RequestKind::Configure));
    }

    fn request(&self, kind: RequestKind, mut command: Command) -> bool {
        if !self.connected {
            self.notice(
                "The engine is disconnected. Reopen the app before changing engine settings.",
                true,
            );
            return false;
        }
        let Some(id) = self.requests.borrow_mut().begin(kind) else {
            self.notice("An engine change is still pending. Wait for its result before applying another change.", true);
            return false;
        };
        command.request_id = id;
        if let Err(error) = self.writer.try_send(command) {
            self.requests.borrow_mut().finish(id);
            self.sync_request_state();
            self.notice(&format!("Engine command queue: {error}"), true);
            return false;
        }
        // Pending is a distinct UI state. Ownership, monitoring and saved plan
        // names remain the last authoritative daemon values until its receipt.
        self.sync_request_state();
        self.notice(
            match kind {
                RequestKind::Handover => "Taking control… waiting for the engine.",
                RequestKind::Configure => {
                    "Applying settings… the engine is validating the change and coordinating monitoring."
                }
            },
            false,
        );
        slint::Timer::single_shot(Duration::from_secs(30), move || {
            CONTROL.with(|cell| {
                if let Some(control) = cell.borrow().as_ref() {
                    let control = control.borrow();
                    let expired = control.requests.borrow_mut().finish(id).is_some();
                    if expired {
                        control.sync_request_state();
                        control.notice("No command receipt within 30 seconds; the outcome is unknown. Review the current state before retrying. An older engine may need a normal app restart.", true);
                    }
                }
            });
        });
        true
    }

    fn observe_request(&self, snapshot: &Snapshot) {
        let completed = self.requests.borrow_mut().finish(snapshot.command_id);
        let Some((kind, draft)) = completed else {
            return;
        };
        self.sync_request_state();
        if !snapshot.command_error.is_empty() {
            self.notice(&snapshot.command_error, true);
            return;
        }
        if let Some(draft) = draft {
            self.finish_saved_draft(draft);
        }
        let message = match kind {
            RequestKind::Handover if snapshot.observer => {
                "The engine acknowledged the request but this window is still read-only."
            }
            RequestKind::Handover if snapshot.legacy => {
                "Control is available; the previous monitor still owns power switching."
            }
            RequestKind::Handover if snapshot.monitoring => {
                "Control transferred. Native monitoring is starting."
            }
            RequestKind::Handover => "Control transferred. Monitoring is paused.",
            RequestKind::Configure if snapshot.monitoring && snapshot.ready => {
                "Settings applied. Monitoring resumed."
            }
            RequestKind::Configure if snapshot.monitoring => {
                "Settings applied. Monitoring is restarting."
            }
            RequestKind::Configure => "Settings applied. Monitoring remains paused.",
        };
        self.notice(message, false);
    }

    fn try_save(&self, config: Config) -> bool {
        if self.state.observer {
            self.notice(
                "Take control of this window before changing engine settings.",
                true,
            );
            return false;
        }
        if let Err(error) = config.validate() {
            self.notice(&error, true);
            return false;
        }
        if self.requests.borrow().pending.is_some() {
            self.notice(
                "An engine change is still pending. Wait before applying another change.",
                true,
            );
            return false;
        }
        if self.state.config.as_ref() == Some(&config) {
            self.notice("Settings unchanged.", false);
            return true;
        }
        // CONFIGURE is already one serialized daemon transaction: validate
        // installed plans, restore/pause, persist, resume or roll back. Sending
        // a separate PAUSE first would lose rollback and original run state.
        self.request(
            RequestKind::Configure,
            Command {
                kind: CONFIGURE,
                config: Some(config),
                ..Default::default()
            },
        )
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;

    #[test]
    fn pending_engine_request_rejects_duplicates_and_unrelated_receipts() {
        let mut requests = Requests::default();
        let id = requests.begin(RequestKind::Handover).unwrap();
        assert!(requests.begin(RequestKind::Handover).is_none());
        assert!(requests.begin(RequestKind::Configure).is_none());
        assert_eq!(requests.finish(0), None); // old engine cannot imply success
        assert_eq!(requests.finish(id + 1), None);
        assert!(requests.is_pending(RequestKind::Handover));
        assert_eq!(requests.finish(id), Some((RequestKind::Handover, None)));
        assert!(!requests.is_pending(RequestKind::Handover));
        assert_eq!(requests.finish(id), None);
    }

    #[test]
    fn late_timeout_or_receipt_cannot_clear_a_newer_transaction() {
        let mut requests = Requests::default();
        let old = requests.begin(RequestKind::Configure).unwrap();
        assert_eq!(requests.finish(old), Some((RequestKind::Configure, None)));
        let next = requests.begin(RequestKind::Handover).unwrap();
        assert_ne!(old, next);
        assert_eq!(requests.finish(old), None);
        assert!(requests.is_pending(RequestKind::Handover));
        assert_eq!(requests.finish(next), Some((RequestKind::Handover, None)));
    }

    #[test]
    fn draft_is_released_only_with_its_matching_receipt() {
        let mut requests = Requests::default();
        let id = requests.begin(RequestKind::Configure).unwrap();
        requests.draft = Some(SavedDraft::Bulk {
            submitted: "Game.exe".into(),
            remaining: "".into(),
        });
        assert_eq!(requests.finish(0), None);
        assert!(requests.draft.is_some());
        let (_, draft) = requests.finish(id).unwrap();
        assert_eq!(
            draft,
            Some(SavedDraft::Bulk {
                submitted: "Game.exe".into(),
                remaining: "".into()
            })
        );
        assert!(requests.draft.is_none());
        assert!(requests.begin(RequestKind::Handover).is_some());
        assert!(requests.draft.is_none());
    }
}
