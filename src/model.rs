// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
use prost::Message;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const GAMING: &str = "319f9863-5b07-4924-b71a-3a687a155782";
pub const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
pub const BEST_OVERLAY: &str = "ded574b5-45a0-4f42-8737-46345c09c238";
#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct Game {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, repeated, tag = "3")]
    pub processes: Vec<String>,
    #[prost(bytes = "vec", tag = "4")]
    #[serde(default)]
    pub icon: Vec<u8>,
    #[prost(uint32, tag = "5")]
    #[serde(default)]
    pub priority: u32,
    #[prost(string, tag = "6")]
    #[serde(default)]
    pub affinity: String,
    #[prost(string, tag = "7")]
    #[serde(default)]
    pub source: String,
    /// Empty uses the global Gaming plan; never edits a Windows plan definition.
    #[prost(string, tag = "8")]
    #[serde(default)]
    pub power_plan: String,
}
#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct Config {
    #[prost(string, tag = "1")]
    pub gaming: String,
    #[prost(string, tag = "2")]
    pub default_plan: String,
    #[prost(message, repeated, tag = "3")]
    pub games: Vec<Game>,
    #[prost(bool, tag = "4")]
    #[serde(default)]
    pub overlay: bool,
    #[prost(bool, tag = "5")]
    #[serde(default)]
    pub reduced_motion: bool,
    // Optional booleans let older JSON/protobuf inherit compatible defaults.
    #[prost(bool, optional, tag = "6")]
    #[serde(default)]
    pub battery_guard: Option<bool>,
    #[prost(bool, optional, tag = "7")]
    #[serde(default)]
    pub auto_affinity: Option<bool>,
    #[prost(bool, tag = "8")]
    #[serde(default)]
    pub foreground_only: bool,
    #[prost(bool, tag = "9")]
    #[serde(default)]
    pub time_rule: bool,
    #[prost(string, tag = "10")]
    #[serde(default)]
    pub time_start: String,
    #[prost(string, tag = "11")]
    #[serde(default)]
    pub time_end: String,
}
#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct Plan {
    #[prost(string, tag = "1")]
    pub guid: String,
    #[prost(string, tag = "2")]
    pub name: String,
}
#[derive(Clone, PartialEq, Message, Serialize, Deserialize)]
pub struct Transition {
    #[prost(uint64, tag = "1")]
    pub time_ms: u64,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(double, tag = "3")]
    pub duration_ms: f64,
    #[prost(string, tag = "4")]
    pub cause: String,
    #[prost(bool, tag = "5")]
    pub gaming: bool,
    // Additive metadata: old engine messages decode these as empty strings.
    #[prost(string, tag = "6")]
    #[serde(default)]
    pub from_guid: String,
    #[prost(string, tag = "7")]
    #[serde(default)]
    pub from_name: String,
    #[prost(string, tag = "8")]
    #[serde(default)]
    pub to_guid: String,
    #[prost(string, tag = "9")]
    #[serde(default)]
    pub process: String,
    #[prost(uint32, tag = "10")]
    #[serde(default)]
    pub pid: u32,
    #[prost(string, tag = "11")]
    #[serde(default)]
    pub account: String,
}
#[derive(Clone, PartialEq, Message)]
pub struct Core {
    #[prost(uint32, tag = "1")]
    pub logical: u32,
    #[prost(uint32, tag = "2")]
    pub physical: u32,
    #[prost(uint32, tag = "3")]
    pub group: u32,
    #[prost(uint32, tag = "4")]
    pub efficiency: u32,
    #[prost(bool, tag = "5")]
    pub parked: bool,
    #[prost(double, tag = "6")]
    pub load: f64,
}
#[derive(Clone, PartialEq, Message)]
pub struct Snapshot {
    #[prost(message, optional, tag = "1")]
    pub config: Option<Config>,
    #[prost(message, repeated, tag = "2")]
    pub plans: Vec<Plan>,
    #[prost(bool, tag = "3")]
    pub monitoring: bool,
    #[prost(string, tag = "4")]
    pub active_guid: String,
    #[prost(string, tag = "5")]
    pub active_name: String,
    #[prost(string, repeated, tag = "6")]
    pub running: Vec<String>,
    #[prost(string, tag = "7")]
    pub status: String,
    #[prost(bool, tag = "8")]
    pub error: bool,
    #[prost(bool, tag = "9")]
    pub legacy: bool,
    #[prost(bool, tag = "10")]
    pub startup: bool,
    #[prost(bool, tag = "11")]
    pub suspended: bool,
    #[prost(message, repeated, tag = "12")]
    pub cores: Vec<Core>,
    #[prost(message, repeated, tag = "13")]
    pub history: Vec<Transition>,
    #[prost(message, repeated, tag = "14")]
    pub discovered: Vec<Game>,
    #[prost(bool, tag = "15")]
    pub observer: bool,
    #[prost(bool, tag = "16")]
    pub retired: bool,
    #[prost(bool, tag = "17")]
    pub ready: bool,
    #[prost(bool, tag = "18")]
    pub dry_run: bool,
    #[prost(string, tag = "19")]
    pub telemetry: String,
    #[prost(string, tag = "20")]
    pub backend: String,
    #[prost(uint64, tag = "21")]
    pub telemetry_sequence: u64,
    #[prost(uint64, tag = "22")]
    pub cpu_sampled_at_ms: u64,
    /// 0 unknown, 1 AC, 2 battery. Unknown is never reported as AC.
    #[prost(uint32, tag = "23")]
    pub power_source: u32,
    #[prost(bool, tag = "24")]
    pub battery_guard_active: bool,
    #[prost(string, tag = "25")]
    pub manual_plan: String,
    #[prost(string, tag = "26")]
    pub automation_status: String,
    /// Receipt for the last tracked command from THIS pipe client. Zero means
    /// no receipt (including an older engine), never implicit success.
    #[prost(uint64, tag = "27")]
    pub command_id: u64,
    #[prost(string, tag = "28")]
    pub command_error: String,
}
// Stable Protocol Buffers envelope; all configuration/snapshot records are binary.
#[derive(Clone, PartialEq, Message)]
pub struct Command {
    #[prost(uint32, tag = "1")]
    pub kind: u32,
    #[prost(string, tag = "2")]
    pub text: String,
    #[prost(string, repeated, tag = "3")]
    pub values: Vec<String>,
    #[prost(message, optional, tag = "4")]
    pub config: Option<Config>,
    #[prost(bool, tag = "5")]
    pub flag: bool,
    /// Optional correlation for synchronous engine transactions. Existing
    /// clients omit this field and retain their original wire behavior.
    #[prost(uint64, tag = "6")]
    pub request_id: u64,
}
pub const HELLO: u32 = 0;
pub const TOGGLE: u32 = 1;
pub const CONFIGURE: u32 = 2;
pub const IMPORT: u32 = 3;
pub const SCAN: u32 = 4;
pub const STARTUP: u32 = 5;
pub const LEGACY: u32 = 6;
pub const CLAIM: u32 = 7;
pub const VIEW: u32 = 8;
pub const REFRESH: u32 = 9;
pub const SHUTDOWN: u32 = 10;
/// Owner-only: text is "gaming", "default", or "auto".
pub const MANUAL_PLAN: u32 = 11;
/// Owner-only, idempotent: flag is the desired monitoring state.
pub const ENSURE_MONITORING: u32 = 12;
pub fn data_dir() -> PathBuf {
    crate::runtime_scope::current().data_dir()
}
pub fn defaults() -> Config {
    Config {
        gaming: GAMING.into(),
        default_plan: BALANCED.into(),
        games: vec![Game {
            id: "siege".into(),
            name: "Rainbow Six Siege".into(),
            processes: vec![
                "RainbowSix.exe".into(),
                "RainbowSixSiege.exe".into(),
                "RainbowSix_DX11.exe".into(),
            ],
            icon: include_bytes!("../assets/siege.png").to_vec(),
            ..Default::default()
        }],
        time_start: "20:00".into(),
        time_end: "00:00".into(),
        battery_guard: Some(true),
        ..Default::default()
    }
}
pub fn valid_guid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
pub fn valid_process(s: &str) -> bool {
    s.len() > 4
        && s.len() <= 132
        && s.to_ascii_lowercase().ends_with(".exe")
        && s[..s.len() - 4]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub struct Parsed {
    pub valid: Vec<String>,
    pub errors: Vec<String>,
    pub duplicates: usize,
    pub paths: usize,
}
pub fn parse_bulk(input: &str, existing: &[String]) -> Parsed {
    let mut seen: HashSet<String> = existing.iter().map(|v| v.to_lowercase()).collect();
    let mut r = Parsed {
        valid: vec![],
        errors: vec![],
        duplicates: 0,
        paths: 0,
    };
    for piece in input.split([',', ';', '\r', '\n']) {
        let s = piece.trim().trim_matches('"');
        if s.is_empty() {
            continue;
        }
        let name = s.rsplit(['\\', '/']).next().unwrap_or(s);
        if name != s {
            r.paths += 1
        }
        if !valid_process(name) {
            r.errors.push(s.chars().take(80).collect());
            continue;
        }
        if !seen.insert(name.to_lowercase()) {
            r.duplicates += 1;
            continue;
        }
        if seen.len() > 500 {
            r.errors.push("Maximum 500 process names".into());
            continue;
        }
        r.valid.push(name.to_owned());
    }
    r
}
impl Config {
    pub fn battery_guard_enabled(&self) -> bool {
        // Old saved settings preserve their behavior; new installs opt in in defaults().
        self.battery_guard.unwrap_or(false)
    }
    pub fn auto_affinity_enabled(&self) -> bool {
        self.auto_affinity.unwrap_or(true)
    }
    pub fn names(&self) -> Vec<String> {
        self.games
            .iter()
            .flat_map(|g| g.processes.clone())
            .collect()
    }
    pub fn validate(&self) -> Result<(), String> {
        if !valid_guid(&self.gaming)
            || !valid_guid(&self.default_plan)
            || self.gaming.eq_ignore_ascii_case(&self.default_plan)
        {
            return Err("Choose two different valid power plans.".into());
        }
        if self.time_rule {
            let start = crate::automation::parse_minute(&self.time_start);
            let end = crate::automation::parse_minute(&self.time_end);
            if start.is_none() || end.is_none() || start == end {
                return Err("Time rule needs different start/end times in HH:MM format.".into());
            }
        }
        let names = self.names();
        if names.len() > 500 {
            return Err("Maximum 500 watched processes.".into());
        }
        let mut seen = HashSet::new();
        let mut ids = HashSet::new();
        for g in &self.games {
            if g.id.is_empty()
                || !ids.insert(g.id.to_lowercase())
                || g.name.len() > 160
                || g.processes.is_empty()
                || g.icon.len() > 200000
            {
                return Err("Invalid or oversized game profile.".into());
            }
            if ![0, 0x20, 0x8000, 0x80].contains(&g.priority) {
                return Err("Unsupported process priority.".into());
            }
            if !g.power_plan.is_empty() && !valid_guid(&g.power_plan) {
                return Err("Per-game power plan must be an installed plan GUID or blank.".into());
            }
            if !g.affinity.is_empty()
                && g.affinity != "performance"
                && g.affinity != "efficiency"
                && g.affinity != "physical"
                && g.affinity != "performance-physical"
                && u64::from_str_radix(g.affinity.trim_start_matches("0x"), 16)
                    .ok()
                    .filter(|v| *v > 0)
                    .is_none()
            {
                return Err("Affinity must be performance, efficiency, physical, performance-physical, or a nonzero hexadecimal mask.".into());
            }
        }
        for n in names {
            if !valid_process(&n) || !seen.insert(n.to_lowercase()) {
                return Err(format!("Invalid or duplicate process: {n}"));
            }
        }
        Ok(())
    }
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        self.validate()?;
        atomic_json(&dir.join("native-settings.json"), self)
    }
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join("native-settings.json");
        if path.exists() {
            let data = fs::read(&path).map_err(|e| e.to_string())?;
            if data.len() > 16 * 1024 * 1024 {
                return Err("Settings exceed 16 MB.".into());
            }
            let c: Self = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
            c.validate()?;
            return Ok(c);
        }
        let mut c = defaults();
        let old = dir.join("settings.json");
        if old.exists() {
            let v: serde_json::Value =
                serde_json::from_slice(&fs::read(old).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if let Some(s) = v["GamingPlanGuid"].as_str() {
                c.gaming = s.into()
            }
            if let Some(s) = v["DefaultPlanGuid"].as_str() {
                c.default_plan = s.into()
            }
            if let Some(names) = v["GameProcessNames"].as_array() {
                c.games.clear();
                for n in names.iter().filter_map(|v| v.as_str()) {
                    let exe = if n.to_lowercase().ends_with(".exe") {
                        n.to_owned()
                    } else {
                        format!("{n}.exe")
                    };
                    if !valid_process(&exe) {
                        return Err(format!("Legacy name needs review: {n}"));
                    }
                    c.add_names(&[exe]);
                }
            }
            if let Some(details) = v["GameDetails"].as_array() {
                use base64::Engine;
                for detail in details {
                    if let Some(name) = detail["Name"].as_str() {
                        let exe = if name.to_lowercase().ends_with(".exe") {
                            name.to_owned()
                        } else {
                            format!("{name}.exe")
                        };
                        if let Some(game) = c.games.iter_mut().find(|g| {
                            g.id != "siege"
                                && g.processes.iter().any(|p| p.eq_ignore_ascii_case(&exe))
                        }) {
                            if let Some(title) = detail["DisplayName"].as_str() {
                                game.name = title.chars().take(160).collect();
                            }
                            if let Some(encoded) = detail["IconBase64"].as_str() {
                                if encoded.len() < 270000 {
                                    if let Ok(icon) =
                                        base64::engine::general_purpose::STANDARD.decode(encoded)
                                    {
                                        if icon.starts_with(b"\x89PNG") {
                                            game.icon = icon;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        c.validate()?;
        Ok(c)
    }
    pub fn add_names(&mut self, names: &[String]) {
        let mut seen: HashSet<String> = self.names().iter().map(|s| s.to_lowercase()).collect();
        for n in names {
            if !seen.insert(n.to_lowercase()) {
                continue;
            }
            let siege = [
                "rainbowsix.exe",
                "rainbowsixsiege.exe",
                "rainbowsix_dx11.exe",
            ]
            .contains(&n.to_lowercase().as_str());
            if siege {
                if let Some(g) = self.games.iter_mut().find(|g| g.id == "siege") {
                    g.processes.push(n.clone())
                } else {
                    let mut g = defaults().games.remove(0);
                    g.processes = vec![n.clone()];
                    self.games.push(g)
                }
            } else {
                self.games.push(Game {
                    id: n.to_lowercase(),
                    name: n[..n.len() - 4].into(),
                    processes: vec![n.clone()],
                    ..Default::default()
                })
            }
        }
    }
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    // ReplaceFileW preserves atomic replacement; MoveFileEx handles the first save.
    crate::win::replace_file(&tmp, path)
}
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bulk_mixed() {
        let p = parse_bulk(
            "RainbowSix.exe,C:\\Games\\Game.exe,invalid_name,Duplicate.exe,duplicate.EXE",
            &[],
        );
        assert_eq!(p.valid.len(), 3);
        assert_eq!(p.errors, vec!["invalid_name"]);
        assert_eq!(p.paths, 1);
        assert_eq!(p.duplicates, 1);
    }
    #[test]
    fn names_reject_commands() {
        for n in [
            "../x.exe",
            "x*.exe",
            "a b.exe",
            "x.exe;calc.exe",
            "a:evil.exe",
            "x.exe\n",
        ] {
            assert!(!valid_process(n), "{n}")
        }
    }
    #[test]
    fn unicode_safe() {
        assert!(!valid_process("🎮.exe"));
        assert!(!valid_guid("é"));
    }
    #[test]
    fn guid_rules() {
        assert!(valid_guid(GAMING));
        assert!(!valid_guid("../x"));
        let mut c = defaults();
        c.default_plan = c.gaming.clone();
        assert!(c.validate().is_err());
    }
    #[test]
    fn profile_grouping() {
        let mut c = defaults();
        c.add_names(&["rainbowsix.exe".into(), "Other.exe".into()]);
        assert_eq!(c.games.len(), 2);
        assert_eq!(c.names().len(), 4);
        assert!(c.validate().is_ok());
    }
    #[test]
    fn protobuf_roundtrip() {
        let c = defaults();
        assert_eq!(Config::decode(c.encode_to_vec().as_slice()).unwrap(), c);
    }
    #[test]
    fn transition_metadata_keeps_old_wire_and_json_compatible() {
        // Literal legacy wire bytes: tag1 time=1, tag2 name="Plan", tag5 gaming=true.
        let old = Transition::decode(&b"\x08\x01\x12\x04Plan\x28\x01"[..]).unwrap();
        assert_eq!(old.name, "Plan");
        assert!(old.from_guid.is_empty() && old.from_name.is_empty());
        assert!(old.to_guid.is_empty() && old.process.is_empty());
        assert_eq!(old.pid, 0);
        assert!(old.account.is_empty());
        let old_json = r#"{"time_ms":1,"name":"Plan","duration_ms":2.0,"cause":"Process started","gaming":true}"#;
        let old_json: Transition = serde_json::from_str(old_json).unwrap();
        assert!(old_json.from_guid.is_empty() && old_json.process.is_empty());
        let rich = Transition {
            from_guid: BALANCED.into(),
            from_name: "Balanced".into(),
            to_guid: GAMING.into(),
            process: "RainbowSix.exe".into(),
            pid: 42,
            account: "PC\\User".into(),
            ..old
        };
        assert_eq!(
            Transition::decode(rich.encode_to_vec().as_slice()).unwrap(),
            rich
        );
        assert_eq!(
            serde_json::from_slice::<Transition>(&serde_json::to_vec(&rich).unwrap()).unwrap(),
            rich
        );
    }
    #[test]
    fn settings_json_roundtrip_and_missing_optional_fields() {
        let config = defaults();
        let bytes = serde_json::to_vec(&config).unwrap();
        let decoded: Config = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, config);
        let minimal = format!(
            r#"{{"gaming":"{GAMING}","default_plan":"{BALANCED}","games":[{{"id":"a","name":"A","processes":["A.exe"]}}]}}"#
        );
        let c: Config = serde_json::from_str(&minimal).unwrap();
        assert!(c.validate().is_ok());
        assert!(!c.overlay);
        assert_eq!(c.games[0].priority, 0);
        assert!(c.auto_affinity_enabled());
        assert!(!c.battery_guard_enabled());
        assert!(!c.foreground_only && !c.time_rule);
        assert!(c.games[0].power_plan.is_empty());
        assert!(serde_json::from_str::<Config>("{broken").is_err());
    }
    #[test]
    fn automation_settings_keep_old_wire_defaults_and_roundtrip_new_rules() {
        // A genuine old-format two-field protobuf (without new settings tags).
        let mut old = vec![0x0a, 36];
        old.extend_from_slice(GAMING.as_bytes());
        old.extend_from_slice(&[0x12, 36]);
        old.extend_from_slice(BALANCED.as_bytes());
        let old = Config::decode(old.as_slice()).unwrap();
        assert!(old.auto_affinity_enabled());
        assert!(!old.battery_guard_enabled());
        let mut c = defaults();
        c.auto_affinity = Some(false);
        c.foreground_only = true;
        c.time_rule = true;
        c.games[0].power_plan = GAMING.into();
        assert!(c.validate().is_ok());
        assert!(c.battery_guard_enabled());
        assert_eq!(Config::decode(c.encode_to_vec().as_slice()).unwrap(), c);
        assert_eq!(
            serde_json::from_slice::<Config>(&serde_json::to_vec(&c).unwrap()).unwrap(),
            c
        );
        c.time_end = c.time_start.clone();
        assert!(c.validate().is_err());
        c.time_end = "25:00".into();
        assert!(c.validate().is_err());
        c.time_rule = false;
        c.games[0].power_plan = "bad-guid".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn bounds() {
        let p = parse_bulk(
            &(0..501)
                .map(|i| format!("Game{i}.exe"))
                .collect::<Vec<_>>()
                .join(","),
            &[],
        );
        assert_eq!(p.valid.len(), 500);
        assert!(!p.errors.is_empty());
    }
}
