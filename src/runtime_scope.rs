// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Immutable process scope for engine IPC, shared telemetry and local data.
//! A preview must never discover the production engine or save its preferences.
//! The real monitor.lock remains shared separately in win::monitor_path().
use sha2::{Digest, Sha256};
use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::OnceLock,
};

static SCOPE: OnceLock<RuntimeScope> = OnceLock::new();

#[derive(Debug)]
pub struct RuntimeScope {
    pub dry: bool,
    data: PathBuf,
    suffix: String,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn CharLowerBuffW(text: *mut u16, length: u32) -> u32;
}

fn is_dry(args: &[String]) -> Result<bool, String> {
    let explicit = args.iter().any(|arg| arg == "--dry-run");
    let live = args.iter().any(|arg| arg == "--live");
    if explicit && live {
        return Err("--dry-run and --live cannot be combined.".into());
    }
    if explicit && args.get(1).is_some_and(|arg| arg == "--guardian") {
        return Err("A preview never starts a recovery guardian.".into());
    }
    // The exercise parent has no --dry-run flag by default, but its child does.
    // Resolve both to the same namespace before either connects to a daemon.
    Ok(explicit
        || (!live
            && args
                .get(1)
                .is_some_and(|arg| matches!(arg.as_str(), "--exercise" | "--pro-probe"))))
}

fn production_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("NN6PowerPlan")
}

/// Canonicalize the existing ancestor too, so a future test directory under a
/// junction gets the same identity before and after a child daemon creates it.
pub(crate) fn normalized_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("NN6_TEST_DATA_DIR must not be empty.".into());
    }
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    let mut ancestor = absolute.as_path();
    let mut tail = Vec::new();
    loop {
        if let Ok(mut canonical) = std::fs::canonicalize(ancestor) {
            for component in tail.iter().rev() {
                canonical.push(component);
            }
            return Ok(canonical);
        }
        match (ancestor.file_name(), ancestor.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                ancestor = parent;
            }
            _ => return Ok(absolute),
        }
    }
}

fn path_marker(path: &Path) -> Vec<u16> {
    let mut units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let unc: Vec<_> = r"\\?\UNC\".encode_utf16().collect();
    let extended: Vec<_> = r"\\?\".encode_utf16().collect();
    if units.starts_with(&unc) {
        units.splice(..unc.len(), [92, 92]);
    } else if units.starts_with(&extended) {
        units.drain(..extended.len());
    }
    for unit in &mut units {
        if *unit == 47 {
            *unit = 92;
        }
    }
    // Windows filesystem scope comparison is case-insensitive. Preserve UTF-16
    // rather than hashing a lossy UTF-8 display of an arbitrary local path.
    unsafe { CharLowerBuffW(units.as_mut_ptr(), units.len() as u32) };
    while units.last() == Some(&92) {
        units.pop();
    }
    units
}

impl RuntimeScope {
    fn from_parts(dry: bool, test: Option<&Path>, production: &Path) -> Result<Self, String> {
        if let Some(test) = test {
            let data = normalized_path(test)?;
            let marker = path_marker(&data);
            // An explicit "test" environment must not alias real saved settings,
            // including case variants, dot segments and existing junctions.
            for protected in [
                production.to_owned(),
                production.with_file_name("GamePowerPlan"),
            ] {
                let protected_marker = path_marker(&normalized_path(&protected)?);
                if marker == protected_marker
                    || (marker.starts_with(&protected_marker)
                        && marker.get(protected_marker.len()) == Some(&92))
                {
                    return Err("NN6_TEST_DATA_DIR must be outside the production NN6PowerPlan and GamePowerPlan data directories.".into());
                }
            }
            let mut digest = Sha256::new();
            digest.update(b"NN6.Engine.Test.v1\0");
            digest.update([u8::from(dry)]);
            for unit in marker {
                digest.update(unit.to_le_bytes());
            }
            return Ok(Self {
                dry,
                data,
                suffix: format!("-test-{}", hex::encode(digest.finalize())),
            });
        }
        let data = if dry {
            let preview = production.with_file_name("NN6PowerPlanPreview");
            let preview_marker = path_marker(&normalized_path(&preview)?);
            for protected in [
                production.to_owned(),
                production.with_file_name("GamePowerPlan"),
            ] {
                let protected_marker = path_marker(&normalized_path(&protected)?);
                if preview_marker == protected_marker
                    || (preview_marker.starts_with(&protected_marker)
                        && preview_marker.get(protected_marker.len()) == Some(&92))
                {
                    return Err("The preview data directory aliases production data. Choose a separate NN6_TEST_DATA_DIR.".into());
                }
            }
            preview
        } else {
            production.to_owned()
        };
        Ok(Self {
            dry,
            data,
            suffix: if dry {
                "-preview-v1".into()
            } else {
                String::new()
            },
        })
    }

    fn from_process(args: &[String]) -> Result<Self, String> {
        let test = std::env::var_os("NN6_TEST_DATA_DIR").map(PathBuf::from);
        Self::from_parts(is_dry(args)?, test.as_deref(), &production_dir())
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data.clone()
    }

    pub fn pipe_name(&self, sid: &str) -> String {
        format!(r"\\.\pipe\NN6-PowerPlan-v2-{sid}{}", self.suffix)
    }

    pub fn ring_name(&self, sid: &str) -> String {
        format!(r"Local\NN6Telemetry-{sid}{}", self.suffix)
    }
}

pub fn initialize(args: &[String]) -> Result<(), String> {
    SCOPE
        .set(RuntimeScope::from_process(args)?)
        .map_err(|_| "Runtime scope was initialized twice.".into())
}

pub fn current() -> &'static RuntimeScope {
    // Unit-test executables do not call main(); production initializes explicitly
    // before registering a panic hook, loading preferences or creating a window.
    SCOPE.get_or_init(|| {
        RuntimeScope::from_process(&std::env::args().collect::<Vec<_>>())
            .expect("Invalid runtime scope")
    })
}

pub fn verify_peer(expected_dry: bool, actual_dry: bool) -> Result<(), String> {
    if expected_dry == actual_dry {
        Ok(())
    } else {
        Err("Engine mode mismatch. Disconnected before sending commands; preview and live sessions cannot share an engine.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        std::iter::once("nn6.exe")
            .chain(values.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn production_names_remain_compatible_but_preview_is_separate() {
        let production = Path::new(r"C:\NN6ScopeTest\NN6PowerPlan");
        let live = RuntimeScope::from_parts(false, None, production).unwrap();
        let preview = RuntimeScope::from_parts(true, None, production).unwrap();
        assert_eq!(
            live.pipe_name("S-1-5-test"),
            r"\\.\pipe\NN6-PowerPlan-v2-S-1-5-test"
        );
        assert_eq!(
            live.ring_name("S-1-5-test"),
            r"Local\NN6Telemetry-S-1-5-test"
        );
        assert_eq!(live.data_dir(), production);
        assert_ne!(preview.pipe_name("sid"), live.pipe_name("sid"));
        assert_ne!(preview.ring_name("sid"), live.ring_name("sid"));
        assert_eq!(
            preview.data_dir(),
            Path::new(r"C:\NN6ScopeTest\NN6PowerPlanPreview")
        );
    }

    #[test]
    fn exercise_parent_and_daemon_child_resolve_identical_modes() {
        assert!(is_dry(&args(&["--exercise", "report.json"])).unwrap());
        assert!(is_dry(&args(&["--daemon", "--dry-run"])).unwrap());
        assert!(!is_dry(&args(&["--exercise", "report.json", "--live"])).unwrap());
        assert!(!is_dry(&args(&["--guardian", "123"])).unwrap());
        assert!(is_dry(&args(&["--exercise", "--live", "--dry-run"])).is_err());
    }

    #[test]
    fn test_names_separate_directory_and_mode_but_normalize_aliases() {
        let production = Path::new(r"C:\NN6ScopeTest\NN6PowerPlan");
        let first = RuntimeScope::from_parts(
            true,
            Some(Path::new(r"C:\NN6ScopeTest\a\..\fixture")),
            production,
        )
        .unwrap();
        let alias = RuntimeScope::from_parts(
            true,
            Some(Path::new(r"c:\nn6scopetest\FIXTURE\")),
            production,
        )
        .unwrap();
        let second =
            RuntimeScope::from_parts(true, Some(Path::new(r"C:\NN6ScopeTest\other")), production)
                .unwrap();
        let live = RuntimeScope::from_parts(
            false,
            Some(Path::new(r"C:\NN6ScopeTest\fixture")),
            production,
        )
        .unwrap();
        assert_eq!(first.pipe_name("sid"), alias.pipe_name("sid"));
        assert_eq!(first.ring_name("sid"), alias.ring_name("sid"));
        assert_ne!(first.pipe_name("sid"), second.pipe_name("sid"));
        assert_ne!(first.ring_name("sid"), live.ring_name("sid"));
        assert_eq!(
            path_marker(&first.data_dir()),
            path_marker(&live.data_dir())
        );
    }

    #[test]
    fn test_data_cannot_alias_production_settings() {
        let production = Path::new(r"C:\NN6ScopeTest\NN6PowerPlan");
        for test in [
            r"c:\nn6scopetest\NN6POWERPLAN",
            r"C:\NN6ScopeTest\NN6PowerPlan\child",
            r"C:\NN6ScopeTest\GamePowerPlan",
            "",
        ] {
            assert!(RuntimeScope::from_parts(true, Some(Path::new(test)), production).is_err());
        }
    }

    #[test]
    fn mismatched_mode_is_always_rejected() {
        assert!(verify_peer(true, false).is_err());
        assert!(verify_peer(false, true).is_err());
        assert!(verify_peer(true, true).is_ok());
        assert!(verify_peer(false, false).is_ok());
    }
}
