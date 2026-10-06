// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Tray, global shortcuts, Windows presentation preferences and optional sensors.
//! The tray/hotkey message loop is separate from Slint and never switches plans.
//! Callbacks must queue work onto Slint's event loop, not touch GUI objects here.
//! Hardware telemetry combines direct GPU APIs and existing LHM/OHM WMI providers.
//! No sensor driver is installed; the persistent sampler lives in sensors.rs.
use serde::Serialize;
use std::{
    ffi::{OsString, c_void},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub enum SystemEvent {
    Show,
    QuickGaming,
    QuickDefault,
    ToggleMonitoring,
    TogglePlan,
    Exit,
    ToggleOverlay,
    ThemeChanged(bool),
    Error(String),
}
type EventCallback = Box<dyn Fn(SystemEvent) + Send + Sync + 'static>;

/// Read Windows processor/cache relationships once for UI presentation. This
/// does not change affinity, priority, power policy, or the telemetry sampler.
pub fn cpu_topology() -> crate::cpu_topology::CpuTopology {
    crate::cpu_topology::detect()
}

#[repr(C)]
#[derive(Default)]
struct RawHotkeyStatus {
    monitor_error: u32,
    overlay_error: u32,
    monitor_registered: u32,
    overlay_registered: u32,
}
#[repr(C)]
struct RawSensors {
    cpu_c: f64,
    gpu_c: f64,
    package_w: f64,
    fan_rpm: f64,
    readings: u32,
    provider_connected: u32,
    provider: [u16; 96],
    cpu_label: [u16; 128],
    gpu_label: [u16; 128],
    power_label: [u16; 128],
    fan_label: [u16; 128],
}
unsafe extern "C" {
    fn nn6_pro_start(
        callback: extern "C" fn(*mut c_void, u32, u32),
        user: *mut c_void,
        error: *mut u32,
    ) -> *mut c_void;
    fn nn6_pro_close(context: *mut c_void);
    fn nn6_pro_tray(context: *mut c_void, enabled: bool, monitoring: bool) -> u32;
    fn nn6_pro_notify_already_running(context: *mut c_void) -> u32;
    fn nn6_pro_hotkeys(
        context: *mut c_void,
        monitor_mods: u32,
        monitor_key: u32,
        overlay_mods: u32,
        overlay_key: u32,
        result: *mut RawHotkeyStatus,
    ) -> u32;
    fn nn6_pro_dark() -> bool;
    fn nn6_pro_font(out: *mut u16, cap: u32) -> u32;
    fn nn6_pro_sensors(out: *mut RawSensors) -> u32;
    fn nn6_pro_file_dialog(
        owner: *mut c_void,
        save: bool,
        kind: u32,
        out: *mut u16,
        cap: u32,
    ) -> u32;
    fn nn6_pro_process_path(pid: u32, out: *mut u16, cap: u32) -> u32;
    fn nn6_pro_last_external_foreground_pid() -> u32;
    fn nn6_pro_elevated_powercfg(parameters: *const u16, exit_code: *mut u32) -> u32;
    fn nn6_pro_configure_overlay(
        owner: *mut c_void,
        main: *mut c_void,
        corner: i32,
        opacity: u8,
    ) -> u32;
}
fn wide_text(value: &[u16]) -> String {
    String::from_utf16_lossy(&value[..value.iter().position(|&c| c == 0).unwrap_or(value.len())])
}
fn error(code: u32, operation: &str) -> String {
    if code == 0x8004100e {
        return format!("{operation}: WMI namespace is not installed (0x{code:08X})");
    }
    if code == 0x80041003 {
        return format!("{operation}: WMI access was denied (0x{code:08X})");
    }
    format!(
        "{operation}: {} (0x{code:08X})",
        std::io::Error::from_raw_os_error(code as i32)
    )
}
fn checked(code: u32, operation: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(error(code, operation))
    }
}
extern "C" fn system_callback(context: *mut c_void, event: u32, value: u32) {
    // Box stays alive until the native thread has joined in SystemIntegration::drop.
    let callback = unsafe { &*(context as *const EventCallback) };
    let event = match event {
        1 => SystemEvent::Show,
        2 => SystemEvent::QuickGaming,
        3 => SystemEvent::QuickDefault,
        4 => SystemEvent::ToggleMonitoring,
        5 => SystemEvent::Exit,
        6 => SystemEvent::ToggleOverlay,
        7 => SystemEvent::ThemeChanged(value != 0),
        8 => SystemEvent::Error(error(value, "Windows tray")),
        9 => SystemEvent::TogglePlan,
        _ => return,
    };
    // Never unwind through a C callback. Release builds also use panic=abort.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(event)));
}

/// RAII ownership of this window's tray/hotkeys. Keep it alive for the GUI lifetime.
/// No tray icon or shortcut is enabled until explicitly configured by the caller.
pub struct SystemIntegration {
    context: *mut c_void,
    _callback: Box<EventCallback>,
}
impl SystemIntegration {
    pub fn start(callback: impl Fn(SystemEvent) + Send + Sync + 'static) -> Result<Self, String> {
        let mut callback: Box<EventCallback> = Box::new(Box::new(callback));
        let mut code = 0;
        let context = unsafe {
            nn6_pro_start(
                system_callback,
                (&mut *callback as *mut EventCallback).cast(),
                &mut code,
            )
        };
        if context.is_null() {
            return Err(error(
                if code == 0 { 31 } else { code },
                "Start Windows integrations",
            ));
        }
        Ok(Self {
            context,
            _callback: callback,
        })
    }
    /// Tray state and menu caption are applied on the native message thread.
    /// TaskbarCreated broadcasts automatically re-add a requested tray icon.
    pub fn set_tray(&self, enabled: bool, monitoring: bool) -> Result<(), String> {
        checked(
            unsafe { nn6_pro_tray(self.context, enabled, monitoring) },
            "Configure tray",
        )
    }
    /// Request a quiet, real-time notice after a duplicate GUI launch. This
    /// never creates a tray icon and is throttled to at most once per 5 seconds
    /// on its native message thread. Ok means queued, not displayed: Windows
    /// may suppress notifications, and shell failures use SystemEvent::Error.
    pub fn notify_already_running(&self) -> Result<(), String> {
        checked(
            unsafe { nn6_pro_notify_already_running(self.context) },
            "Queue existing-window notification",
        )
    }
    /// Blank disables a shortcut. Conflicts return per-shortcut errors instead of
    /// claiming success. Replacing a shortcut unregisters the prior registration.
    pub fn set_hotkeys(&self, monitor: &str, overlay: &str) -> Result<HotkeyStatus, String> {
        let monitor = parse_hotkey(monitor)?;
        let overlay = parse_hotkey(overlay)?;
        if monitor.is_some() && monitor == overlay {
            return Err("Power-plan and overlay shortcuts must be different.".into());
        }
        let monitor = monitor.unwrap_or_default();
        let overlay = overlay.unwrap_or_default();
        let mut raw = RawHotkeyStatus::default();
        checked(
            unsafe {
                nn6_pro_hotkeys(
                    self.context,
                    monitor.modifiers,
                    monitor.key,
                    overlay.modifiers,
                    overlay.key,
                    &mut raw,
                )
            },
            "Configure global hotkeys",
        )?;
        Ok(HotkeyStatus {
            monitor_registered: raw.monitor_registered != 0,
            overlay_registered: raw.overlay_registered != 0,
            monitor_error: (raw.monitor_error != 0)
                .then(|| error(raw.monitor_error, "Power-plan hotkey")),
            overlay_error: (raw.overlay_error != 0)
                .then(|| error(raw.overlay_error, "Overlay hotkey")),
        })
    }
}
impl Drop for SystemIntegration {
    fn drop(&mut self) {
        // Deletes the shell icon, unregisters both shortcuts, closes the window,
        // joins its native thread, and only then releases the callback allocation.
        unsafe { nn6_pro_close(self.context) };
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct HotkeyStatus {
    pub monitor_registered: bool,
    pub overlay_registered: bool,
    pub monitor_error: Option<String>,
    pub overlay_error: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Hotkey {
    pub modifiers: u32,
    pub key: u32,
}
/// Validate names rather than guessing keyboard layouts or invoking a shell.
pub fn parse_hotkey(input: &str) -> Result<Option<Hotkey>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    if input.len() > 64 || !input.is_ascii() {
        return Err("Use a shortcut such as Ctrl+Alt+P (maximum 64 ASCII characters).".into());
    }
    let mut result = Hotkey::default();
    for part in input.split('+') {
        let part = part.trim().to_ascii_uppercase();
        let modifier = match part.as_str() {
            "ALT" => 1,
            "CTRL" | "CONTROL" => 2,
            "SHIFT" => 4,
            "WIN" | "WINDOWS" => 8,
            _ => 0,
        };
        if modifier != 0 {
            if result.modifiers & modifier != 0 {
                return Err("Do not repeat a shortcut modifier.".into());
            }
            result.modifiers |= modifier;
            continue;
        }
        if result.key != 0 {
            return Err("A shortcut must contain exactly one non-modifier key.".into());
        }
        result.key = if part.len() == 1 && part.as_bytes()[0].is_ascii_alphanumeric() {
            part.as_bytes()[0] as u32
        } else if let Some(number) = part.strip_prefix('F').and_then(|v| v.parse::<u32>().ok()) {
            if !(1..=24).contains(&number) || number == 12 {
                return Err("Use F1–F24 except F12, which Windows reserves for debuggers.".into());
            }
            0x70 + number - 1
        } else {
            match part.as_str() {
                "SPACE" => 0x20,
                "ENTER" => 0x0d,
                "HOME" => 0x24,
                "END" => 0x23,
                "PAGEUP" | "PGUP" => 0x21,
                "PAGEDOWN" | "PGDN" => 0x22,
                _ => {
                    return Err(
                        "Use A–Z, 0–9, F1–F24 (except F12), Space, Enter, Home, End, PgUp or PgDn."
                            .into(),
                    );
                }
            }
        };
    }
    if result.modifiers == 0 || result.key == 0 {
        return Err("Include Ctrl, Alt, Shift or Win and one key, for example Ctrl+Alt+P.".into());
    }
    Ok(Some(result))
}
pub fn system_theme_dark() -> bool {
    unsafe { nn6_pro_dark() }
}
pub fn preferred_font() -> String {
    let mut family = [0; 64];
    if unsafe { nn6_pro_font(family.as_mut_ptr(), family.len() as u32) } == 0 {
        wide_text(&family)
    } else {
        "Segoe UI".into()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SensorSnapshot {
    pub connected: bool,
    pub provider: String,
    pub cpu_c: Option<f64>,
    pub gpu_c: Option<f64>,
    pub package_w: Option<f64>,
    pub fan_rpm: Option<f64>,
    pub gpu_utilization: Option<f64>,
    pub gpu_fan_percent: Option<f64>,
    pub gpu_fan_rpm: Option<f64>,
    pub cpu_provider: String,
    pub gpu_provider: String,
    pub power_provider: String,
    pub fan_provider: String,
    pub gpu_utilization_provider: String,
    pub gpu_fan_provider: String,
    pub cpu_label: String,
    pub gpu_label: String,
    pub power_label: String,
    pub fan_label: String,
    pub readings: u32,
    pub error: Option<String>,
    /// Poll completion time, not the hardware provider's acquisition timestamp.
    pub polled_at_ms: u64,
    /// Separate fallback completion time; never relabeled as a new direct read.
    pub wmi_polled_at_ms: Option<u64>,
    pub freshness_verified: bool,
    pub freshness_note: String,
}
/// Single blocking diagnostic sample. GUI workers must retain SensorReader instead
/// of repeatedly initializing the GPU libraries. No settings or power mutations.
pub fn sensor_snapshot() -> SensorSnapshot {
    crate::sensors::SensorReader::new().sample_for_probe()
}
/// Blocking WMI read: call on a worker, never Slint's UI thread. Only existing
/// ROOT\LibreHardwareMonitor / ROOT\OpenHardwareMonitor Sensor classes are read.
/// No administrator request, driver load, software install or synthetic fallback.
pub(crate) fn wmi_sensor_snapshot() -> SensorSnapshot {
    let mut raw = RawSensors {
        cpu_c: -1.,
        gpu_c: -1.,
        package_w: -1.,
        fan_rpm: -1.,
        readings: 0,
        provider_connected: 0,
        provider: [0; 96],
        cpu_label: [0; 128],
        gpu_label: [0; 128],
        power_label: [0; 128],
        fan_label: [0; 128],
    };
    let code = unsafe { nn6_pro_sensors(&mut raw) };
    let available = |value: f64| (value.is_finite() && value >= 0.).then_some(value);
    let provider = wide_text(&raw.provider);
    SensorSnapshot {
        connected: raw.provider_connected != 0,
        provider: provider.clone(),
        cpu_c: available(raw.cpu_c), gpu_c: available(raw.gpu_c),
        package_w: available(raw.package_w), fan_rpm: available(raw.fan_rpm),
        cpu_label: wide_text(&raw.cpu_label), gpu_label: wide_text(&raw.gpu_label),
        power_label: wide_text(&raw.power_label), fan_label: wide_text(&raw.fan_label),
        readings: raw.readings,
        error: if code != 0 { Some(format!("{}; run an existing LibreHardwareMonitor/OpenHardwareMonitor provider with WMI enabled, then Retry.", error(code, "Sensor provider unavailable"))) }
            else if raw.readings == 0 { Some("Provider connected but no readings are exposed. Check the external monitor, then Retry.".into()) }
            else if raw.cpu_c < 0. && raw.gpu_c < 0. && raw.package_w < 0. && raw.fan_rpm < 0. {
                Some("Provider connected but no supported temperature, CPU package-power or fan readings are exposed.".into())
            } else { None },
        polled_at_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        freshness_verified: false,
        freshness_note: "Provider sample timestamps are not exposed by this WMI Sensor schema; values may be cached. A stable value does not prove staleness or freshness.".into(),
        cpu_provider: available(raw.cpu_c).map(|_| provider.clone()).unwrap_or_default(),
        gpu_provider: available(raw.gpu_c).map(|_| provider.clone()).unwrap_or_default(),
        power_provider: available(raw.package_w).map(|_| provider.clone()).unwrap_or_default(),
        fan_provider: available(raw.fan_rpm).map(|_| provider.clone()).unwrap_or_default(),
        ..SensorSnapshot::default()
    }
}
#[derive(Clone, Copy, Debug)]
pub enum FileKind {
    PowerPlan = 0,
    Csv = 1,
    Json = 2,
}
/// Native user-initiated file picker. Does not import/export a plan by itself.
/// Call on a worker; the native dialog initializes STA and pumps a modal loop.
pub fn pick_file(owner: usize, save: bool, kind: FileKind) -> Result<Option<PathBuf>, String> {
    let mut path = vec![0; 32768];
    let code = unsafe {
        nn6_pro_file_dialog(
            owner as *mut c_void,
            save,
            kind as u32,
            path.as_mut_ptr(),
            path.len() as u32,
        )
    };
    if code == 1223 {
        return Ok(None);
    }
    checked(code, "Choose file")?;
    Ok(Some(wide_text(&path).into()))
}
pub fn process_path(pid: u32) -> Option<PathBuf> {
    let mut path = vec![0; 32768];
    (unsafe { nn6_pro_process_path(pid, path.as_mut_ptr(), path.len() as u32) } == 0)
        .then(|| wide_text(&path).into())
}

// Quote each UTF-16 argument using Windows' backslash-before-quote rules. No
// shell, interpolation, environment substitution or lossy path conversion is
// involved. In particular, trailing backslashes must double before the closing
// quote or they could consume it and change argument boundaries.
fn quote_windows_arguments(args: &[OsString]) -> Result<Vec<u16>, String> {
    let mut command = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if index != 0 {
            command.push(b' ' as u16);
        }
        command.push(b'"' as u16);
        let mut slashes = 0;
        for character in arg.encode_wide() {
            if character == 0 {
                return Err("Power-plan arguments cannot contain a NUL character.".into());
            }
            if character == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            if character == b'"' as u16 {
                command.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            } else {
                command.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            }
            slashes = 0;
            command.push(character);
        }
        command.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        command.push(b'"' as u16);
        if command.len() > 30000 {
            return Err("Power-plan arguments exceed the Windows command-line limit.".into());
        }
    }
    command.push(0);
    Ok(command)
}

fn validate_elevated_powercfg(args: &[OsString]) -> Result<(), String> {
    // This broker deliberately cannot activate/delete/reset plans or execute an
    // arbitrary executable. The controller separately validates file ownership,
    // overwrite approval and generated destination GUIDs before calling it.
    if args.len() != 3 {
        return Err(
            "Elevated plan tools require one supported operation and two arguments.".into(),
        );
    }
    let operation = args[0].to_str().unwrap_or("").to_ascii_lowercase();
    let valid_guid = |argument: &OsString| {
        argument.to_str().is_some_and(|s| {
            s.len() == 36
                && s.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                })
        })
    };
    match operation.as_str() {
        "/export" | "/import" if Path::new(&args[1]).is_absolute() && valid_guid(&args[2]) => Ok(()),
        "/duplicatescheme" if valid_guid(&args[1]) && valid_guid(&args[2]) => Ok(()),
        _ => Err("Windows approval is limited to /export, /import or /duplicatescheme with absolute file paths and explicit valid GUIDs.".into()),
    }
}

/// Explicit, user-approved power-plan tool only. Call on a background worker
/// after the user enables Windows approval or presses the approval retry button.
/// The ordinary GUI/engine stay unelevated. This launches only the OS-resolved
/// System32 powercfg.exe using the `runas` verb; Windows controls the UAC prompt.
/// Waits at most 30 seconds AFTER process launch. Approval itself is user-paced.
/// On timeout the child is not killed and the outcome must be checked before any
/// retry. This helper is intentionally absent from all probes and live tests.
pub fn elevated_powercfg(args: &[OsString]) -> Result<(), String> {
    validate_elevated_powercfg(args)?;
    let parameters = quote_windows_arguments(args)?;
    let mut exit_code = 259;
    let code = unsafe { nn6_pro_elevated_powercfg(parameters.as_ptr(), &mut exit_code) };
    match code {
        1223 => return Err("Windows approval was cancelled; elevated powercfg was not started.".into()),
        1460 => return Err("Elevated powercfg did not finish within 30 seconds. It was left running; the result is unconfirmed. Check the destination file or plan before retrying.".into()),
        _ => checked(code, "Windows-approved powercfg")?,
    }
    if exit_code != 0 {
        return Err(format!(
            "Windows-approved powercfg exited with code {exit_code} (0x{exit_code:08X}). No console output is captured through the Windows approval launcher."
        ));
    }
    Ok(())
}
/// Last external foreground application's PID, captured before the main UI is
/// shown and refreshed by EVENT_SYSTEM_FOREGROUND on the native message thread.
/// Excludes this process and desktop/taskbar surfaces. Zero means none observed.
pub fn last_external_foreground_pid() -> u32 {
    unsafe { nn6_pro_last_external_foreground_pid() }
}
/// Size-aware convenience wrapper: place the existing HUD near a main-monitor
/// work-area corner (0TL,1TR,2BL,3BR), keeping its present logical dimensions.
pub fn configure_overlay(
    owner: usize,
    main: usize,
    corner: i32,
    opacity: u8,
) -> Result<(), String> {
    checked(
        unsafe {
            nn6_pro_configure_overlay(owner as *mut c_void, main as *mut c_void, corner, opacity)
        },
        "Configure overlay",
    )
}

/// Diagnostic only: transiently adds/removes our tray icon and tries/relinquishes
/// Ctrl+Alt+P plus Ctrl+Alt+O. It never invokes the emitted commands or changes a
/// power plan, startup task, settings, or another application's registrations.
/// sensor_snapshot blocks here: expose this as a CLI/worker probe, not a UI call.
pub fn probe() -> serde_json::Value {
    let mut result = serde_json::json!({
        "font": preferred_font(), "system_dark": system_theme_dark(),
        "sensors": sensor_snapshot(),
        "scope": "Transient tray/hotkey registration probe; no commands dispatched or power changes.",
    });
    match SystemIntegration::start(|_| {}) {
        Ok(integration) => {
            result["tray"] = serde_json::json!(integration.set_tray(true, false));
            result["hotkeys"] =
                serde_json::json!(integration.set_hotkeys("Ctrl+Alt+P", "Ctrl+Alt+O"));
            result["unregister"] = serde_json::json!(integration.set_hotkeys("", ""));
            result["remove_tray"] = serde_json::json!(integration.set_tray(false, false));
            drop(integration);
            result["shutdown"] =
                serde_json::json!("Native window/thread closed; owned registrations released.");
        }
        Err(error) => {
            result["integration_error"] = error.into();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn elevated_broker_rejects_unrelated_commands_without_launching() {
        let guid = "381b4222-f694-41f0-9685-ff5bb260df2e";
        assert!(
            validate_elevated_powercfg(&[
                "/export".into(),
                r"C:\Power plans\plan.pow".into(),
                guid.into()
            ])
            .is_ok()
        );
        assert!(
            validate_elevated_powercfg(&["/duplicatescheme".into(), guid.into(), guid.into()])
                .is_ok()
        );
        for args in [
            vec!["/delete".into(), guid.into(), guid.into()],
            vec!["/setactive".into(), guid.into(), guid.into()],
            vec!["/export".into(), "relative.pow".into(), guid.into()],
            vec!["/export".into(), r"C:\plan.pow".into(), "/setactive".into()],
            vec!["/import".into(), r"C:\plan.pow".into()],
        ] {
            assert!(validate_elevated_powercfg(&args).is_err());
        }
        assert!(quote_windows_arguments(&[OsString::from("bad\0path")]).is_err());
        assert!(quote_windows_arguments(&[OsString::from("x".repeat(30001))]).is_err());
    }
    #[test]
    fn windows_quoting_round_trips_paths_and_metacharacters() {
        use std::os::windows::ffi::OsStringExt;
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn CommandLineToArgvW(command: *const u16, count: *mut i32) -> *mut *mut u16;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn LocalFree(memory: *mut c_void) -> *mut c_void;
        }
        let arguments = vec![
            OsString::from("/export"),
            OsString::from(r"C:\Users\AB F\Power plans\plan.pow"),
            OsString::from(r"C:\folder with space\"),
            OsString::from("embedded\"quote\\\"and\\"),
            OsString::from("& $(calc) | %SYSTEMROOT% ; \n"),
            OsString::from(""),
            OsString::from("ملف 日本語.pow"),
            OsString::from_wide(&[0xd800, b'x' as u16]),
        ];
        let mut command: Vec<_> = "powercfg.exe ".encode_utf16().collect();
        command.extend(quote_windows_arguments(&arguments).unwrap());
        let mut count = 0;
        let parsed = unsafe { CommandLineToArgvW(command.as_ptr(), &mut count) };
        assert!(!parsed.is_null());
        let mut observed = Vec::new();
        for index in 1..count as usize {
            let argument = unsafe { *parsed.add(index) };
            let mut length = 0;
            while unsafe { *argument.add(length) } != 0 {
                length += 1;
            }
            observed.push(OsString::from_wide(unsafe {
                std::slice::from_raw_parts(argument, length)
            }));
        }
        unsafe {
            LocalFree(parsed.cast());
        }
        assert_eq!(observed, arguments);
    }
    #[test]
    fn hotkey_parser_preserves_default_and_disabling() {
        assert_eq!(
            parse_hotkey("ctrl + alt + p").unwrap(),
            Some(Hotkey {
                modifiers: 3,
                key: 80
            })
        );
        assert_eq!(parse_hotkey(" ").unwrap(), None);
        assert_eq!(parse_hotkey("Shift+F24").unwrap().unwrap().key, 0x87);
        assert_eq!(parse_hotkey("Win+Ctrl+9").unwrap().unwrap().modifiers, 10);
    }
    #[test]
    fn invalid_and_reserved_hotkeys_are_rejected() {
        for input in [
            "P",
            "Ctrl",
            "Ctrl++P",
            "Ctrl+Control+P",
            "Alt+P+Q",
            "Alt+F0",
            "Ctrl+F12",
            "Ctrl+F25",
            "Ctrl+é",
            "Ctrl+$(calc)",
        ] {
            assert!(parse_hotkey(input).is_err(), "{input}");
        }
    }
}
