// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Explicit 2.3 diagnostics. Default mode only reads Windows and writes fixtures
//! under a fresh NN6_TEST_DATA_DIR. --live additionally creates/cleans two owned
//! disposable power plans; it never selects a power plan or disables a monitor.
//! A Windows privilege gate leaves the optional file roundtrip unverified. This
//! diagnostic never elevates or opens UAC; cleanup errors still fail the run.
use crate::{
    automation, model::*, preferences::Preferences, pro_view::SessionTimes,
    session_export::SessionExport, win,
};
use std::{collections::HashSet, fs, path::PathBuf};

#[repr(C)]
struct Guid {
    a: u32,
    b: u16,
    c: u16,
    d: [u8; 8],
}
#[link(name = "ole32")]
unsafe extern "system" {
    fn CLSIDFromString(text: *const u16, guid: *mut Guid) -> i32;
}
#[link(name = "powrprof")]
unsafe extern "system" {
    fn PowerDeleteScheme(root: *mut std::ffi::c_void, guid: *const Guid) -> u32;
}
struct OwnedPlans {
    before: HashSet<String>,
    created: Vec<String>,
}
impl OwnedPlans {
    fn record(&mut self, result: &str) -> Result<String, String> {
        let id = result
            .split_whitespace()
            .map(|s| s.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != '-'))
            .find(|s| valid_guid(s) && !self.before.contains(&s.to_lowercase()))
            .ok_or("Creation returned no new disposable plan GUID; no deletion was attempted")?
            .to_lowercase();
        self.created.push(id.clone());
        if !win::plans()?
            .iter()
            .any(|p| p.guid.eq_ignore_ascii_case(&id))
        {
            return Err("Created probe plan was not found".into());
        }
        Ok(id)
    }
    fn cleanup(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        for id in std::mem::take(&mut self.created) {
            if self.before.contains(&id) {
                errors.push("Refused to delete a pre-existing plan".into());
                continue;
            }
            match win::active() {
                Ok(active) if active.eq_ignore_ascii_case(&id) => {
                    errors.push(format!("Probe plan {id} became active; left installed"));
                    continue;
                }
                Err(e) => {
                    errors.push(e);
                    continue;
                }
                _ => {}
            }
            let text: Vec<u16> = format!("{{{id}}}").encode_utf16().chain(Some(0)).collect();
            let mut guid = Guid {
                a: 0,
                b: 0,
                c: 0,
                d: [0; 8],
            };
            if unsafe { CLSIDFromString(text.as_ptr(), &mut guid) } < 0 {
                errors.push(format!("Invalid owned GUID {id}"));
                continue;
            }
            let result = unsafe { PowerDeleteScheme(std::ptr::null_mut(), &guid) };
            if result != 0 {
                errors.push(format!("Cleanup {id}: Windows error {result}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
impl Drop for OwnedPlans {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

// Return true only for a complete file roundtrip. A known privilege error may
// leave read-only checks successful, but may never hide an owned-plan leak.
fn file_probe_result(
    operation: Result<(), String>,
    cleanup: Result<(), String>,
) -> Result<bool, String> {
    if let Err(cleanup) = cleanup {
        return Err(match operation {
            Err(operation) => format!("{operation}; cleanup: {cleanup}"),
            Ok(()) => format!("cleanup: {cleanup}"),
        });
    }
    match operation {
        Ok(()) => Ok(true),
        Err(error) if crate::power_tools::needs_elevation(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn run(report: &str, live: bool) -> Result<(), String> {
    let dir = std::env::var_os("NN6_TEST_DATA_DIR")
        .map(PathBuf::from)
        .ok_or("Set NN6_TEST_DATA_DIR to a fresh disposable directory")?;
    if dir.exists() {
        return Err("Use a fresh diagnostic directory to preserve previous/user data".into());
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let original = win::active()?;
    let mut results = Vec::new();
    let mut file_roundtrip_verified = false;
    let mut file_test_status = "not_requested";
    let outcome = (|| -> Result<(), String> {
        let source = automation::power_source();
        results.push(serde_json::json!({"test":"read-only Windows power source","source":source,"account":automation::account(),"passed":source<=2,"note":"0 is unavailable, never inferred as AC"}));
        let mut config = defaults();
        config.auto_affinity = Some(false);
        config.foreground_only = true;
        config.save(&dir)?;
        if Config::load(&dir)? != config {
            return Err("Isolated engine settings failed roundtrip".into());
        }
        let mut prefs = Preferences::default();
        prefs.heatmap = true;
        prefs.compact = true;
        prefs.save()?;
        if serde_json::to_value(&Preferences::load()).map_err(|e| e.to_string())?
            != serde_json::to_value(&prefs).map_err(|e| e.to_string())?
        {
            return Err("Isolated appearance settings failed roundtrip".into());
        }
        results.push(
            serde_json::json!({"test":"isolated config/preferences roundtrip","passed":true}),
        );
        // Exercise the exact pre-mutation authorization gate with simulated
        // ownership inputs. No daemon/manual command is sent to the user's PC.
        if automation::manual_request(true, true, false, "gaming").is_ok()
            || automation::manual_request(false, false, false, "gaming").is_ok()
        {
            return Err("Manual ownership/legacy guard accepted a denied request".into());
        }
        results.push(serde_json::json!({"test":"manual pre-mutation gate fixtures","passed":true,"note":"simulated legacy/observer inputs; no IPC or power change"}));
        let session = SessionTimes::default();
        let snapshot = Snapshot {
            history: vec![Transition {
                name: "=Fixture, \"target\"".into(),
                from_name: "Before".into(),
                process: "Probe.exe".into(),
                pid: 42,
                account: "Fixture\\User".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let export = SessionExport::new(&snapshot, &session);
        let json = export.json()?;
        let csv = export.csv();
        let decoded: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        if decoded["history"][0]["pid"] != 42 || !csv.contains("\"'=Fixture, \"\"target\"\"\"") {
            return Err("Diagnostic export lost metadata or CSV safety quoting".into());
        }
        fs::write(dir.join("fixture-session.json"), json).map_err(|e| e.to_string())?;
        fs::write(dir.join("fixture-session.csv"), csv).map_err(|e| e.to_string())?;
        results.push(serde_json::json!({"test":"JSON/CSV synthetic fixture roundtrip and escaping","passed":true}));
        if live {
            file_test_status = "failed";
            let before = win::plans()?;
            let ids = before.iter().map(|p| p.guid.to_lowercase()).collect();
            let source = before
                .iter()
                .find(|p| p.guid.eq_ignore_ascii_case(&original))
                .ok_or("Current source plan was not enumerated")?;
            let mut owned = OwnedPlans {
                before: ids,
                created: vec![],
            };
            let mut stage = "duplicate";
            let operation = (|| -> Result<(), String> {
                let copy = crate::power_tools::duplicate(&source.guid)?;
                let id = owned.record(&copy)?;
                results.push(serde_json::json!({"test":"opt-in disposable duplicate","passed":true,"created":id}));
                let file = dir.join("disposable-plan.pow");
                stage = "export";
                crate::power_tools::export(&id, &file)?;
                stage = "import";
                let imported = crate::power_tools::import(&file)?;
                let imported_id = owned.record(&imported)?;
                stage = "verify";
                if id == imported_id {
                    return Err("Import reused the duplicate GUID".into());
                }
                if !win::active()?.eq_ignore_ascii_case(&original) {
                    return Err("An external actor changed the active plan during the probe".into());
                }
                results.push(serde_json::json!({"test":"opt-in disposable duplicate/export/import","passed":true,"created":[id,imported_id]}));
                Ok(())
            })();
            let created = owned.created.clone();
            let cleanup = owned.cleanup();
            results.push(serde_json::json!({"test":"owned disposable plan cleanup","created":created,"passed":cleanup.is_ok(),"error":cleanup.as_ref().err()}));
            let operation_error = operation.as_ref().err().cloned();
            file_roundtrip_verified = file_probe_result(operation, cleanup)?;
            file_test_status = if file_roundtrip_verified {
                "verified"
            } else {
                "requires_manual_approval"
            };
            if !file_roundtrip_verified {
                results.push(serde_json::json!({
                    "test":"opt-in disposable duplicate/export/import",
                    "status":"requires_manual_approval", "passed":null,
                    "file_roundtrip_verified":false, "stage":stage,
                    "error":operation_error,
                    "note":"Windows denied a required privilege. No elevation or UAC was attempted; optional file operations remain unverified."
                }));
            }
        }
        if !win::active()?.eq_ignore_ascii_case(&original) {
            return Err("Active plan changed externally while read-only diagnostics ran; no restoration was attempted".into());
        }
        Ok(())
    })();
    let report = PathBuf::from(report);
    let report = if report.is_absolute() {
        report
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(report)
    };
    atomic_json(
        &report,
        &serde_json::json!({"kind":"Game Power Plan Switcher diagnostic","live_plan_file_tests":live,"file_test_status":file_test_status,"file_roundtrip_verified":file_roundtrip_verified,"requires_manual_approval":file_test_status=="requires_manual_approval","original_plan":original,"active_after":win::active(),"checks":results,"passed":outcome.is_ok(),"error":outcome.as_ref().err(),"scope":"No game launch, plan selection, startup mutation, hotkey registration, monitor handover or elevation. Overall passed does not imply the optional file roundtrip was verified."}),
    )?;
    outcome
}

#[cfg(test)]
mod tests {
    use super::file_probe_result;

    #[test]
    fn privilege_gate_is_incomplete_not_verified() {
        assert_eq!(file_probe_result(Ok(()), Ok(())), Ok(true));
        assert_eq!(
            file_probe_result(
                Err("powercfg exit code: 0x522: privilege required".into()),
                Ok(())
            ),
            Ok(false)
        );
        assert!(file_probe_result(Err("Unexpected invalid plan".into()), Ok(())).is_err());
    }

    #[test]
    fn permission_gate_never_hides_cleanup_failure() {
        let result = file_probe_result(
            Err("powercfg exit code: 0x522".into()),
            Err("owned plan was not deleted".into()),
        );
        let error = result.unwrap_err();
        assert!(error.contains("0x522"));
        assert!(error.contains("owned plan was not deleted"));
        assert!(file_probe_result(Ok(()), Err("cleanup failed".into())).is_err());
    }
}
