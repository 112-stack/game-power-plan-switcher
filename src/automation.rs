// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Pure policy decisions plus small read-only Windows observations.
//! All power writes remain in the engine's serial ownership/journal path.
use crate::model::Config;
use std::collections::HashMap;

pub const UNKNOWN: u32 = 0;
pub const AC: u32 = 1;
pub const BATTERY: u32 = 2;

pub fn parse_minute(value: &str) -> Option<u16> {
    let bytes = value.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || ![bytes[0], bytes[1], bytes[3], bytes[4]]
            .iter()
            .all(u8::is_ascii_digit)
    {
        return None;
    }
    let hour = u16::from(bytes[0] - b'0') * 10 + u16::from(bytes[1] - b'0');
    let minute = u16::from(bytes[3] - b'0') * 10 + u16::from(bytes[4] - b'0');
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}
fn in_window(now: u16, start: u16, end: u16) -> bool {
    if start < end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}
#[derive(Debug, PartialEq)]
pub struct Decision {
    pub target: String,
    pub gaming: bool,
    pub guard_active: bool,
    pub explanation: String,
}
pub fn decide(
    config: &Config,
    pids: &HashMap<u32, String>,
    foreground: u32,
    source: u32,
    minute: u16,
    manual: &str,
) -> Decision {
    let mut result = Decision {
        target: config.default_plan.clone(),
        gaming: false,
        guard_active: false,
        explanation: "No eligible watched process · Default".into(),
    };
    // Fail closed for missing power-status data. A successful AC read is the
    // only observation that permits an aggressive target with guard enabled.
    if config.battery_guard_enabled() && source != AC {
        result.guard_active = true;
        result.explanation = if source == BATTERY {
            "Battery guard · Default on DC"
        } else {
            "Battery guard · power source unknown, using Default"
        }
        .into();
        return result;
    }
    if manual == "default" {
        result.explanation = "Manual Default · until resume/pause or next game lifecycle".into();
        return result;
    }
    if manual == "gaming" {
        result.target = config.gaming.clone();
        result.gaming = true;
        result.explanation = "Manual Gaming · until resume/pause or next game lifecycle".into();
        return result;
    }
    if config.time_rule {
        let window = parse_minute(&config.time_start).zip(parse_minute(&config.time_end));
        if !window.is_some_and(|(start, end)| start != end && in_window(minute, start, end)) {
            result.explanation = "Outside configured local-time window · Default".into();
            return result;
        }
    }
    // Configured order is stable, unlike HashMap iteration. If several games
    // run at once, the first matching profile determines the power target.
    if let Some(game) = config.games.iter().find(|game| {
        pids.iter().any(|(pid, name)| {
            (!config.foreground_only || *pid == foreground)
                && game
                    .processes
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(name))
        })
    }) {
        result.target = if game.power_plan.is_empty() {
            config.gaming.clone()
        } else {
            game.power_plan.clone()
        };
        result.gaming = !result.target.eq_ignore_ascii_case(&config.default_plan);
        result.explanation = format!(
            "{} · {}{}",
            game.name,
            if config.foreground_only {
                "foreground process"
            } else {
                "running process"
            },
            if game.power_plan.is_empty() {
                ""
            } else {
                " · per-game plan"
            }
        );
    } else if config.foreground_only {
        result.explanation = "No watched foreground process · Default".into();
    }
    result
}

/// Validate manual intent before any lock acquisition, journaling or OS write.
pub fn manual_request(owner: bool, legacy: bool, dry: bool, role: &str) -> Result<(), String> {
    if !owner {
        return Err("This window is read-only until you take control.".into());
    }
    if !["gaming", "default", "auto"].contains(&role) {
        return Err("Manual target must be gaming, default or auto.".into());
    }
    if legacy && !dry {
        return Err("The previous monitor is active. Use the handover action first.".into());
    }
    Ok(())
}

#[repr(C)]
#[derive(Default)]
struct SystemPowerStatus {
    ac: u8,
    battery: u8,
    percent: u8,
    saver: u8,
    life: u32,
    full: u32,
}
#[repr(C)]
#[derive(Default)]
struct SystemTime {
    year: u16,
    month: u16,
    weekday: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    millis: u16,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
    fn GetLocalTime(time: *mut SystemTime);
}
#[link(name = "secur32")]
unsafe extern "system" {
    fn GetUserNameExW(format: u32, name: *mut u16, size: *mut u32) -> u8;
}
/// Win32 reports 255/failed status as unknown, never as battery or mains.
pub fn power_source() -> u32 {
    let mut status = SystemPowerStatus::default();
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return UNKNOWN;
    }
    match status.ac {
        1 => AC,
        0 => BATTERY,
        _ => UNKNOWN,
    }
}
pub fn local_minute() -> u16 {
    let mut time = SystemTime::default();
    unsafe {
        GetLocalTime(&mut time);
    }
    time.hour * 60 + time.minute
}
/// Account executing the per-user engine (not an inferred game-token owner).
pub fn account() -> String {
    let mut data = [0u16; 512];
    let mut size = data.len() as u32;
    if unsafe { GetUserNameExW(2, data.as_mut_ptr(), &mut size) } == 0 || size as usize > data.len()
    {
        return "Account unavailable".into();
    }
    String::from_utf16_lossy(&data[..size as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Game, defaults};
    #[test]
    fn battery_and_unknown_guard_manual_and_automatic_targets() {
        let mut c = defaults();
        let pids = HashMap::from([(7, "RainbowSix.exe".into())]);
        for source in [BATTERY, UNKNOWN] {
            for manual in ["", "gaming"] {
                let d = decide(&c, &pids, 7, source, 120, manual);
                assert_eq!(d.target, c.default_plan);
                assert!(d.guard_active);
                assert!(!d.gaming);
            }
        }
        assert_eq!(decide(&c, &pids, 7, AC, 120, "").target, c.gaming);
        c.battery_guard = Some(false);
        assert_eq!(decide(&c, &pids, 7, BATTERY, 120, "").target, c.gaming);
    }
    #[test]
    fn multi_profile_order_and_optional_foreground_are_deterministic() {
        let mut c = defaults();
        c.games.push(Game {
            id: "other".into(),
            name: "Other".into(),
            processes: vec!["Other.exe".into()],
            power_plan: "custom".into(),
            ..Default::default()
        });
        let pids = HashMap::from([(9, "Other.exe".into()), (8, "rainbowsix.EXE".into())]);
        assert_eq!(decide(&c, &pids, 9, AC, 0, "").target, c.gaming);
        c.foreground_only = true;
        assert_eq!(decide(&c, &pids, 9, AC, 0, "").target, "custom");
        assert_eq!(decide(&c, &pids, 1, AC, 0, "").target, c.default_plan);
        assert_eq!(decide(&c, &pids, 1, AC, 0, "gaming").target, c.gaming);
    }
    #[test]
    fn overnight_window_has_explicit_inclusive_start_exclusive_end() {
        let mut c = defaults();
        c.time_rule = true;
        c.time_start = "20:00".into();
        c.time_end = "02:00".into();
        let pids = HashMap::from([(1, "RainbowSix.exe".into())]);
        for minute in [1200, 1439, 0, 119] {
            assert!(decide(&c, &pids, 1, AC, minute, "").gaming);
        }
        for minute in [120, 1199] {
            assert!(!decide(&c, &pids, 1, AC, minute, "").gaming);
        }
        for invalid in ["2:00", "24:00", "12:60", "aa:00", "00:00 "] {
            assert!(parse_minute(invalid).is_none());
        }
    }
    #[test]
    fn rejected_manual_requests_never_reach_mock_power_path() {
        let mut writes = 0;
        for request in [
            (false, false, false, "gaming"),
            (true, true, false, "gaming"),
            (true, false, false, "evil"),
        ] {
            if manual_request(request.0, request.1, request.2, request.3).is_ok() {
                writes += 1;
            }
        }
        assert_eq!(writes, 0);
        assert!(manual_request(true, false, false, "gaming").is_ok());
        assert!(manual_request(true, true, true, "default").is_ok());
    }
}
