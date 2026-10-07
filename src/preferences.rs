// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Per-user appearance and shell preferences, separate from engine policy.
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Interface choice only. Engine policy and native Windows plan names are
    /// independent of this per-user appearance setting.
    #[serde(deserialize_with = "deserialize_language")]
    pub language: String,
    pub compact: bool,
    pub dark: bool,
    pub sync_theme: bool,
    pub native_frame: bool,
    pub heatmap: bool,
    pub auto_discover: bool,
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub hotkey: String,
    pub hud_enabled: bool,
    pub hud_position: i32,
    pub hud_opacity: f32,
    pub hud_hotkey: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: "system".into(),
            compact: true,
            dark: true,
            sync_theme: true,
            native_frame: false,
            heatmap: false,
            auto_discover: false,
            close_to_tray: true,
            start_minimized: false,
            hotkey: "Ctrl + Alt + P".into(),
            hud_enabled: false,
            hud_position: 1,
            hud_opacity: 80.,
            hud_hotkey: String::new(),
        }
    }
}

fn deserialize_language<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    Ok(crate::localization::normalize(&value).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_missing_preferences_use_compact_without_overriding_saved_expanded() {
        assert!(Preferences::default().compact);
        assert!(serde_json::from_str::<Preferences>("{}").unwrap().compact);
        let saved: Preferences = serde_json::from_str(r#"{"compact":false,"dark":false}"#).unwrap();
        assert!(!saved.compact);
        assert!(!saved.dark);
        let saved: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(!saved.compact);
    }

    #[test]
    fn language_migrates_old_preferences_and_round_trips_the_override() {
        let old: Preferences =
            serde_json::from_str(r#"{"compact":false,"hotkey":"Ctrl + Alt + P"}"#).unwrap();
        assert_eq!(old.language, "system");
        assert!(!old.compact);
        let selected: Preferences =
            serde_json::from_str(r#"{"language":"pt_BR","compact":false}"#).unwrap();
        assert_eq!(selected.language, "pt-BR");
        let roundtrip: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&selected).unwrap()).unwrap();
        assert_eq!(roundtrip.language, "pt-BR");
        assert!(!roundtrip.compact);
        let unknown: Preferences = serde_json::from_str(r#"{"language":"unavailable"}"#).unwrap();
        assert_eq!(unknown.language, "en");
    }
}
impl Preferences {
    pub fn load() -> Self {
        std::fs::read(crate::model::data_dir().join("appearance.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<(), String> {
        crate::model::atomic_json(&crate::model::data_dir().join("appearance.json"), self)
    }
}
