// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Exports only observed session counters and recorded transitions.
use crate::{model::Snapshot, pro_view::SessionTimes};
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const LOG_BYTES: u64 = 64 * 1024;

/// App-owned observations only: no environment dump, game memory, configuration
/// icons, or arbitrary directory scan is included in diagnostic exports.
#[derive(Serialize)]
struct CurrentState {
    status: String,
    error: bool,
    backend: String,
    automation_status: String,
    active_guid: String,
    active_name: String,
    monitoring: bool,
    ready: bool,
    observer: bool,
    legacy_monitor: bool,
    dry_run: bool,
    power_source: &'static str,
    battery_guard_active: bool,
    manual_plan: String,
    running_process_names: Vec<String>,
    telemetry: String,
    cpu_sampled_at_unix_ms: u64,
}
#[derive(Serialize)]
struct LogExcerpt {
    file: &'static str,
    byte_limit: u64,
    truncated: bool,
    text: String,
    error: Option<String>,
}
fn app_log(directory: &Path) -> LogExcerpt {
    let mut excerpt = LogExcerpt {
        file: "native.log",
        byte_limit: LOG_BYTES,
        truncated: false,
        text: String::new(),
        error: None,
    };
    let read = (|| -> Result<(), std::io::Error> {
        // Read only the bounded tail, even when an old log escaped rotation.
        let mut file = File::open(directory.join("native.log"))?;
        let size = file.metadata()?.len();
        excerpt.truncated = size > LOG_BYTES;
        file.seek(SeekFrom::Start(size.saturating_sub(LOG_BYTES)))?;
        let mut bytes = Vec::new();
        file.take(LOG_BYTES).read_to_end(&mut bytes)?;
        let start = if excerpt.truncated {
            // The seek can land inside a UTF-8 code point or a log record.
            // Omit that first partial line rather than fabricate its prefix.
            bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |i| i + 1)
        } else {
            0
        };
        excerpt.text = String::from_utf8_lossy(&bytes[start..]).into_owned();
        Ok(())
    })();
    if let Err(error) = read {
        excerpt.error = Some(error.to_string());
    }
    excerpt
}
#[derive(Serialize)]
pub struct SessionExport {
    schema_version: u32,
    exported_at_unix_ms: u64,
    gaming_ms: u64,
    default_ms: u64,
    unknown_ms: u64,
    accounting: &'static str,
    history: Vec<crate::model::Transition>,
    current_state: CurrentState,
}
impl SessionExport {
    pub fn new(snapshot: &Snapshot, session: &SessionTimes) -> Self {
        let (gaming_ms, default_ms, unknown_ms) = session.milliseconds();
        Self {
            schema_version: 2,
            exported_at_unix_ms: crate::model::now_ms(),
            gaming_ms,
            default_ms,
            unknown_ms,
            accounting: "Observed while this GUI was connected; unknown time excluded; no energy estimate",
            history: snapshot.history.clone(),
            current_state: CurrentState {
                status: snapshot.status.clone(),
                error: snapshot.error,
                backend: snapshot.backend.clone(),
                automation_status: snapshot.automation_status.clone(),
                active_guid: snapshot.active_guid.clone(),
                active_name: snapshot.active_name.clone(),
                monitoring: snapshot.monitoring,
                ready: snapshot.ready,
                observer: snapshot.observer,
                legacy_monitor: snapshot.legacy,
                dry_run: snapshot.dry_run,
                power_source: match snapshot.power_source {
                    1 => "AC",
                    2 => "DC",
                    _ => "unknown",
                },
                battery_guard_active: snapshot.battery_guard_active,
                manual_plan: snapshot.manual_plan.clone(),
                running_process_names: snapshot.running.clone(),
                telemetry: snapshot.telemetry.clone(),
                cpu_sampled_at_unix_ms: snapshot.cpu_sampled_at_ms,
            },
        }
    }
    pub fn json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }
    /// Invoke from the controller's worker, not its Slint event-loop thread.
    /// Missing logs are an explicit diagnostic state, not an export failure.
    pub fn diagnostic_json(
        &self,
        directory: &Path,
        selected_event: &str,
    ) -> Result<String, String> {
        let detail: String = selected_event.chars().take(8192).collect();
        serde_json::to_string_pretty(&serde_json::json!({
            "kind": "Game Power Plan Switcher diagnostic",
            "diagnostic_schema_version": 1,
            "session": self,
            "selected_event_detail": detail,
            "activity_log": app_log(directory),
            "scope": "Current app state, recorded transitions, observed session counters and the last 64 KiB of native.log. May contain account/process/plan names. No credential stores, environment variables, game memory or unrelated files are read. Older failures may be outside the retained log tail."
        })).map_err(|e| e.to_string())
    }
    pub fn csv(&self) -> String {
        let mut out = "record,time_unix_ms,from_plan,to_plan,process,pid,account,duration_ms,gaming_ms,default_ms,unknown_ms,status,error,backend,automation_status,active_guid,active_name\r\n".to_string();
        let mut row = vec![String::new(); 17];
        row[0] = "session".into();
        row[1] = self.exported_at_unix_ms.to_string();
        row[8] = self.gaming_ms.to_string();
        row[9] = self.default_ms.to_string();
        row[10] = self.unknown_ms.to_string();
        out.push_str(&row.join(","));
        out.push_str("\r\n");
        let mut row = vec![String::new(); 17];
        row[0] = "state".into();
        row[1] = self.exported_at_unix_ms.to_string();
        row[11] = csv_cell(&self.current_state.status);
        row[12] = self.current_state.error.to_string();
        row[13] = csv_cell(&self.current_state.backend);
        row[14] = csv_cell(&self.current_state.automation_status);
        row[15] = csv_cell(&self.current_state.active_guid);
        row[16] = csv_cell(&self.current_state.active_name);
        out.push_str(&row.join(","));
        out.push_str("\r\n");
        for e in &self.history {
            let mut row = vec![String::new(); 17];
            row[0] = "transition".into();
            row[1] = e.time_ms.to_string();
            row[2] = csv_cell(&e.from_name);
            row[3] = csv_cell(&e.name);
            row[4] = csv_cell(&e.process);
            row[5] = e.pid.to_string();
            row[6] = csv_cell(&e.account);
            row[7] = format!("{:.3}", e.duration_ms);
            out.push_str(&row.join(","));
            out.push_str("\r\n");
        }
        out
    }
}
fn csv_cell(value: &str) -> String {
    // Spreadsheet consumers must not interpret a plan/process name as a formula.
    let prefix = if value.trim_start().starts_with(['=', '+', '-', '@']) {
        "'"
    } else {
        ""
    };
    format!("\"{}{}\"", prefix, value.replace('"', "\"\""))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_quotes_and_neutralizes_formulas() {
        assert_eq!(csv_cell("a,\"b\""), "\"a,\"\"b\"\"\"");
        assert_eq!(csv_cell(" =1+2"), "\"' =1+2\"");
    }
    #[test]
    fn exports_unknown_without_inventing_energy() {
        let export = SessionExport::new(&Snapshot::default(), &SessionTimes::default());
        let json = export.json().unwrap();
        assert!(json.contains("unknown_ms"));
        assert!(!json.contains("energy_saved"));
        assert_eq!(export.csv().lines().count(), 3);
    }
    #[test]
    fn failed_state_is_exported_without_inventing_a_successful_transition() {
        let snapshot = Snapshot {
            error: true,
            status: "Default switch failed: Windows error 5".into(),
            backend: "Windows process events".into(),
            automation_status: "Battery guard · Default on DC".into(),
            power_source: 2,
            battery_guard_active: true,
            ..Default::default()
        };
        let export = SessionExport::new(&snapshot, &SessionTimes::default());
        let data: serde_json::Value = serde_json::from_str(&export.json().unwrap()).unwrap();
        assert_eq!(data["current_state"]["error"], true);
        assert_eq!(data["current_state"]["status"], snapshot.status);
        assert_eq!(data["current_state"]["power_source"], "DC");
        assert!(data["history"].as_array().unwrap().is_empty());
        assert!(
            export
                .csv()
                .contains("Default switch failed: Windows error 5")
        );
        assert!(export.csv().lines().all(|row| row.split(',').count() == 17));
    }
    #[test]
    fn diagnostic_log_is_bounded_and_missing_log_is_explicit() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("nn6-export-{}-{unique}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let export = SessionExport::new(&Snapshot::default(), &SessionTimes::default());
        let absent: serde_json::Value =
            serde_json::from_str(&export.diagnostic_json(&directory, "").unwrap()).unwrap();
        assert!(absent["activity_log"]["error"].is_string());
        std::fs::write(
            directory.join("unrelated.txt"),
            "unrelated fixture excluded",
        )
        .unwrap();
        let log = format!(
            "Old omitted record\n{}\nLatest failure: Windows error 5\n",
            "sample\n".repeat(12000)
        );
        std::fs::write(directory.join("native.log"), log).unwrap();
        let json = export
            .diagnostic_json(&directory, "Selected recorded event")
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(&json).unwrap();
        let tail = data["activity_log"]["text"].as_str().unwrap();
        assert_eq!(data["activity_log"]["truncated"], true);
        assert!(data["activity_log"]["error"].is_null());
        assert!(tail.len() <= LOG_BYTES as usize);
        assert!(tail.contains("Latest failure"));
        assert!(!tail.contains("Old omitted record"));
        assert!(!json.contains("unrelated fixture excluded"));
        std::fs::remove_file(directory.join("native.log")).unwrap();
        std::fs::remove_file(directory.join("unrelated.txt")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
