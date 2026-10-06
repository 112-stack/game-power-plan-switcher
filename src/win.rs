// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Small checked wrappers around the C++ bridge. No game executable is launched.
use crate::model::{Core, Plan};
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    os::windows::{fs::OpenOptionsExt, io::FromRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
};
pub type EventFn = extern "C" fn(u32, u32, *const u16, u64);
#[repr(C)]
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct Schedule {
    pub pid: u32,
    pub priority: u32,
    pub affinity: u64,
    pub created: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RawCore {
    logical: u32,
    core: u32,
    group: u32,
    efficiency: u32,
    parked: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct CpuLoad {
    pub group: u32,
    pub logical: u32,
    pub load: f64,
}
#[derive(Debug, Serialize)]
pub struct CpuSample {
    pub values: Vec<CpuLoad>,
    pub interval_ms: u64,
}
unsafe extern "C" {
    fn nn6_ring_open(name: *const u16, writer: bool) -> *mut c_void;
    fn nn6_ring_close(context: *mut c_void);
    fn nn6_ring_write(context: *mut c_void, data: *const u8, size: u32, sequence: *mut u64) -> u32;
    fn nn6_ring_read(
        context: *mut c_void,
        sequence: u64,
        out: *mut u8,
        cap: u32,
        length: *mut u32,
    ) -> u32;
    fn nn6_watch_exit(pid: u32) -> *mut c_void;
    fn nn6_unwatch_exit(context: *mut c_void);
    fn nn6_sid(out: *mut u16, cap: u32) -> u32;
    fn nn6_active(out: *mut u16, cap: u32) -> u32;
    fn nn6_set_plan(s: *const u16) -> u32;
    fn nn6_plans(out: *mut u16, cap: u32) -> u32;
    fn nn6_overlay(out: *mut u16, cap: u32) -> u32;
    fn nn6_set_overlay(s: *const u16) -> u32;
    fn nn6_processes(out: *mut u16, cap: u32) -> u32;
    fn nn6_schedule(pid: u32, p: u32, a: u64, expected: u64, out: *mut Schedule) -> u32;
    fn nn6_restore_schedule(s: *const Schedule) -> u32;
    fn nn6_cores(out: *mut RawCore, cap: u32) -> u32;
    fn nn6_cpu_load(out: *mut CpuLoad, cap: u32, count: *mut u32, interval_ms: *mut u64) -> u32;
    fn nn6_cpu_reset();
    fn nn6_pick_exes(owner: *mut c_void, out: *mut u16, cap: u32) -> u32;
    fn nn6_icon(path: *const u16, out: *mut u8, side: u32) -> u32;
    fn nn6_open_url(url: *const u16) -> u32;
    fn nn6_clipboard(owner: *mut c_void, text: *const u16) -> u32;
    fn nn6_window(owner: *mut c_void, action: u32);
    fn nn6_round_window(owner: *mut c_void);
    fn nn6_startup(command: *const u16, change: i32, out: *mut u16, cap: u32) -> u32;
    fn nn6_pipe(name: *const u16) -> *mut c_void;
    fn nn6_connect_pipe(pipe: *mut c_void) -> u32;
    fn nn6_pipe_io(
        pipe: *mut c_void,
        data: *mut u8,
        size: u32,
        count: *mut u32,
        writing: bool,
    ) -> u32;
    pub fn nn6_events(callback: EventFn, generation: u64) -> u32;
    pub fn nn6_prepare_events();
    pub fn nn6_stop_events();
    fn nn6_wait_process(pid: u32);
}
pub struct ProcessWait(*mut c_void);
pub struct TelemetryRing(*mut c_void);
impl Drop for TelemetryRing {
    fn drop(&mut self) {
        unsafe { nn6_ring_close(self.0) }
    }
}
impl TelemetryRing {
    pub fn open(writer: bool) -> Result<Self, String> {
        let name = crate::runtime_scope::current().ring_name(&sid()?);
        let h = unsafe { nn6_ring_open(wide(&name).as_ptr(), writer) };
        if h.is_null() {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(Self(h))
        }
    }
    pub fn write(&self, data: &[u8]) -> Result<u64, String> {
        let mut seq = 0;
        result(
            unsafe { nn6_ring_write(self.0, data.as_ptr(), data.len() as u32, &mut seq) },
            "Write telemetry ring",
        )?;
        Ok(seq)
    }
    pub fn read(&self, seq: u64) -> Result<Vec<u8>, String> {
        let mut bytes = vec![0; 65536];
        let mut len = 0;
        result(
            unsafe {
                nn6_ring_read(
                    self.0,
                    seq,
                    bytes.as_mut_ptr(),
                    bytes.len() as u32,
                    &mut len,
                )
            },
            "Read telemetry ring",
        )?;
        bytes.truncate(len as usize);
        Ok(bytes)
    }
}
impl Drop for ProcessWait {
    fn drop(&mut self) {
        unsafe { nn6_unwatch_exit(self.0) }
    }
}
pub fn watch_exit(pid: u32) -> Option<ProcessWait> {
    let p = unsafe { nn6_watch_exit(pid) };
    if p.is_null() {
        None
    } else {
        Some(ProcessWait(p))
    }
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
    fn ReplaceFileW(
        a: *const u16,
        b: *const u16,
        c: *const u16,
        flags: u32,
        x: *mut c_void,
        y: *mut c_void,
    ) -> i32;
}
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub fn string(v: &[u16]) -> String {
    String::from_utf16_lossy(&v[..v.iter().position(|c| *c == 0).unwrap_or(v.len())])
}
pub fn result(code: u32, operation: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("{operation}: Windows error {code} (0x{code:08X})"))
    }
}
fn read(
    f: unsafe extern "C" fn(*mut u16, u32) -> u32,
    n: usize,
    label: &str,
) -> Result<String, String> {
    let mut b = vec![0; n];
    result(unsafe { f(b.as_mut_ptr(), n as u32) }, label)?;
    Ok(string(&b))
}
pub fn sid() -> Result<String, String> {
    read(nn6_sid, 256, "Read user SID")
}
pub fn active() -> Result<String, String> {
    read(nn6_active, 64, "Read active plan").map(|s| s.to_lowercase())
}
pub fn set_plan(s: &str) -> Result<(), String> {
    result(
        unsafe { nn6_set_plan(wide(&format!("{{{s}}}")).as_ptr()) },
        "Switch power plan",
    )
}
pub fn plans() -> Result<Vec<Plan>, String> {
    Ok(read(nn6_plans, 32768, "List power plans")?
        .lines()
        .filter_map(|s| s.split_once('\t'))
        .map(|(guid, name)| Plan {
            guid: guid.to_lowercase(),
            name: name.into(),
        })
        .collect())
}
pub fn overlay() -> Result<String, String> {
    read(nn6_overlay, 64, "Read power overlay")
}
pub fn set_overlay(s: &str) -> Result<(), String> {
    result(
        unsafe { nn6_set_overlay(wide(&format!("{{{s}}}")).as_ptr()) },
        "Set power overlay",
    )
}
pub fn processes() -> Result<Vec<(u32, String)>, String> {
    Ok(read(nn6_processes, 262144, "Read processes")?
        .lines()
        .filter_map(|s| s.split_once('\t'))
        .filter_map(|(p, n)| Some((p.parse().ok()?, n.into())))
        .collect())
}
pub fn schedule(pid: u32, p: u32, a: u64) -> Result<Schedule, String> {
    let mut out = Schedule::default();
    result(
        unsafe { nn6_schedule(pid, p, a, 0, &mut out) },
        "Read game scheduling",
    )?;
    Ok(out)
}
pub fn apply_schedule(before: &Schedule, p: u32, a: u64) -> Result<(), String> {
    let mut out = Schedule::default();
    result(
        unsafe { nn6_schedule(before.pid, p, a, before.created, &mut out) },
        "Apply game scheduling",
    )
}
pub fn restore_schedule(s: &Schedule) -> Result<(), String> {
    result(unsafe { nn6_restore_schedule(s) }, "Restore scheduling")
}
pub fn cores() -> Vec<Core> {
    let mut b = vec![RawCore::default(); 4096];
    let n = unsafe { nn6_cores(b.as_mut_ptr(), b.len() as u32) } as usize;
    b[..n.min(b.len())]
        .iter()
        .map(|v| Core {
            logical: v.logical,
            physical: v.core,
            group: v.group,
            efficiency: v.efficiency,
            parked: v.parked != 0,
            load: -1.0,
        })
        .collect()
}
pub fn reset_cpu_load() {
    unsafe { nn6_cpu_reset() }
}
pub fn cpu_load() -> Result<Option<CpuSample>, String> {
    let mut values = vec![CpuLoad::default(); 4096];
    let mut count = 0;
    let mut interval_ms = 0;
    let status = unsafe {
        nn6_cpu_load(
            values.as_mut_ptr(),
            values.len() as u32,
            &mut count,
            &mut interval_ms,
        )
    };
    if status == 997 {
        return Ok(None); // ERROR_IO_PENDING: first PDH rate-counter observation.
    }
    if status != 0 {
        return Err(format!("Windows CPU counters unavailable (0x{status:08X})"));
    }
    values.truncate((count as usize).min(values.len()));
    Ok(Some(CpuSample {
        values,
        interval_ms,
    }))
}
/// Join by Windows processor group AND logical index. Missing/unavailable is
/// -1, which the UI displays as an em dash, never as an invented 0% measurement.
pub fn apply_cpu_sample(cores: &mut [Core], values: &[CpuLoad]) {
    for core in cores {
        core.load = values
            .iter()
            .find(|v| v.group == core.group && v.logical == core.logical)
            .filter(|v| v.load.is_finite() && v.load >= 0.)
            .map(|v| v.load.min(100.))
            .unwrap_or(-1.);
    }
}
pub fn icon(path: &Path) -> Result<Vec<u8>, String> {
    let mut rgba = vec![0; 32 * 32 * 4];
    result(
        unsafe {
            nn6_icon(
                wide(&path.to_string_lossy()).as_ptr(),
                rgba.as_mut_ptr(),
                32,
            )
        },
        "Read executable icon",
    )?;
    let mut data = vec![];
    {
        let mut encoder = png::Encoder::new(&mut data, 32, 32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok(data)
}
pub fn pick_exes(hwnd: usize) -> Vec<String> {
    let mut out = vec![0u16; 65536];
    if unsafe { nn6_pick_exes(hwnd as _, out.as_mut_ptr(), out.len() as u32) } == 0 {
        return vec![];
    }
    let pieces: Vec<_> = out
        .split(|c| *c == 0)
        .take_while(|v| !v.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    if pieces.len() < 2 {
        return pieces;
    }
    pieces[1..]
        .iter()
        .map(|n| Path::new(&pieces[0]).join(n).to_string_lossy().into())
        .collect()
}
pub fn open_url(url: &str) -> Result<(), String> {
    result(unsafe { nn6_open_url(wide(url).as_ptr()) }, "Open link")
}
pub fn copy(hwnd: usize, s: &str) -> Result<(), String> {
    result(
        unsafe { nn6_clipboard(hwnd as _, wide(s).as_ptr()) },
        "Copy to clipboard",
    )
}
pub fn window(hwnd: usize, action: u32) {
    unsafe { nn6_window(hwnd as _, action) }
}
pub fn round_window(hwnd: usize) {
    unsafe { nn6_round_window(hwnd as _) }
}
pub fn lock(path: &Path) -> Result<File, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
        .map_err(|e| format!("Another monitor owns {}: {e}", path.display()))
}
pub fn monitor_path() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
        .join("GamePowerPlan/monitor.lock")
}
pub fn replace_file(tmp: &Path, dest: &Path) -> Result<(), String> {
    let a = wide(&tmp.to_string_lossy());
    let b = wide(&dest.to_string_lossy());
    let ok = unsafe {
        if dest.exists() {
            let backup = wide(&dest.with_extension("bak").to_string_lossy());
            ReplaceFileW(
                b.as_ptr(),
                a.as_ptr(),
                backup.as_ptr(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } else {
            MoveFileExW(a.as_ptr(), b.as_ptr(), 8)
        }
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
pub fn hidden_command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.creation_flags(0x08000000);
    c
}
pub fn startup(change: Option<bool>) -> Result<bool, String> {
    let mut command = String::new();
    if change == Some(true) {
        let dir = crate::model::data_dir().join("NativeApp");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let dest = dir.join("NN6-PowerPlan-Native.exe");
        if exe != dest {
            std::fs::copy(&exe, &dest).map_err(|e| e.to_string())?;
        }
        command = format!("\"{}\"", dest.display());
    }
    let mut out = vec![0; 32768];
    let op = match change {
        Some(true) => 1,
        Some(false) => -1,
        None => 0,
    };
    result(
        unsafe {
            nn6_startup(
                wide(&command).as_ptr(),
                op,
                out.as_mut_ptr(),
                out.len() as u32,
            )
        },
        "Startup registration",
    )?;
    Ok(change.unwrap_or_else(|| !string(&out).is_empty()))
}
pub fn legacy(disable: bool) -> Result<bool, String> {
    let sid = sid()?;
    let task = format!("GamePowerPlan-RainbowSix-{sid}");
    let script = if disable {
        format!(
            "$ErrorActionPreference='Stop';$t=Get-ScheduledTask -TaskName '{task}' -ErrorAction SilentlyContinue;if($t){{$t|Disable-ScheduledTask|Out-Null;$t|Stop-ScheduledTask;Write-Output 'false'}}else{{Write-Output 'false'}}"
        )
    } else {
        format!(
            "$t=Get-ScheduledTask -TaskName '{task}' -ErrorAction SilentlyContinue;if($t -and $t.Settings.Enabled){{'true'}}else{{'false'}}"
        )
    };
    let out = hidden_command(&format!(
        "{}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
        std::env::var("SystemRoot").unwrap_or("C:\\Windows".into())
    ))
    .args(["-NoProfile", "-NonInteractive", "-Command", &script])
    .output()
    .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim() == "true")
}
pub fn pipe_name() -> Result<String, String> {
    Ok(crate::runtime_scope::current().pipe_name(&sid()?))
}
// Overlapped I/O is essential: simultaneous synchronous reads/writes on cloned
// Windows handles serialize and can deadlock a duplex connection.
pub struct Pipe(File);
impl Pipe {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        self.0.try_clone().map(Self)
    }
    fn transfer(&self, data: *mut u8, size: usize, writing: bool) -> std::io::Result<usize> {
        use std::os::windows::io::AsRawHandle;
        let mut count = 0;
        let code = unsafe {
            nn6_pipe_io(
                self.0.as_raw_handle(),
                data,
                size.min(u32::MAX as usize) as u32,
                &mut count,
                writing,
            )
        };
        if code == 0 {
            Ok(count as usize)
        } else {
            Err(std::io::Error::from_raw_os_error(code as i32))
        }
    }
}
impl std::io::Read for Pipe {
    fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
        self.transfer(data.as_mut_ptr(), data.len(), false)
    }
}
impl std::io::Write for Pipe {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.transfer(data.as_ptr() as _, data.len(), true)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn create_pipe() -> Result<Pipe, String> {
    let h = unsafe { nn6_pipe(wide(&pipe_name()?).as_ptr()) };
    if h.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(Pipe(unsafe { File::from_raw_handle(h) }))
}
pub fn accept_pipe(file: &Pipe) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    result(
        unsafe { nn6_connect_pipe(file.0.as_raw_handle()) },
        "Connect local IPC",
    )
}
pub fn connect_pipe() -> Result<Pipe, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(0x40000000)
        .open(pipe_name()?)
        .map(Pipe)
        .map_err(|e| e.to_string())
}
pub fn wait_process(pid: u32) {
    unsafe { nn6_wait_process(pid) }
}

#[cfg(test)]
mod cpu_tests {
    use super::*;

    #[test]
    fn cpu_samples_join_processor_group_and_logical_index() {
        let mut cores = vec![
            Core {
                group: 0,
                logical: 0,
                ..Default::default()
            },
            Core {
                group: 1,
                logical: 0,
                ..Default::default()
            },
            Core {
                group: 1,
                logical: 7,
                ..Default::default()
            },
        ];
        apply_cpu_sample(
            &mut cores,
            &[
                CpuLoad {
                    group: 1,
                    logical: 0,
                    load: 72.5,
                },
                CpuLoad {
                    group: 0,
                    logical: 0,
                    load: 0.0,
                },
            ],
        );
        assert_eq!(cores[0].load, 0.0); // A real measured zero is valid.
        assert_eq!(cores[1].load, 72.5); // Never overwrite group zero.
        assert_eq!(cores[2].load, -1.0); // Missing data is not a measured zero.
    }

    #[test]
    fn cpu_sample_errors_never_become_zero_usage() {
        let mut cores: Vec<_> = (0..4)
            .map(|logical| Core {
                logical,
                load: 66.,
                ..Default::default()
            })
            .collect();
        apply_cpu_sample(
            &mut cores,
            &[
                CpuLoad {
                    logical: 0,
                    load: f64::NAN,
                    ..Default::default()
                },
                CpuLoad {
                    logical: 1,
                    load: -1.,
                    ..Default::default()
                },
                CpuLoad {
                    logical: 2,
                    load: 110.,
                    ..Default::default()
                },
            ],
        );
        assert_eq!(
            cores.iter().map(|c| c.load).collect::<Vec<_>>(),
            [-1., -1., 100., -1.]
        );
        apply_cpu_sample(&mut cores, &[]);
        assert!(cores.iter().all(|c| c.load == -1.));
    }

    /// Explicit opt-in read-only Windows verification. Creates a short local CPU
    /// load, reads actual PDH observations, and never calls a power mutation API.
    #[test]
    #[ignore = "Live Windows PDH probe; briefly loads four CPU threads without switching plans"]
    fn live_windows_cpu_counter_probe() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::Duration;
        let original = active().expect("Read active plan");
        let topology = cores();
        assert!(!topology.is_empty());
        reset_cpu_load();
        assert!(cpu_load().expect("Initialize Windows counters").is_none());
        std::thread::sleep(Duration::from_millis(1100));
        let idle = cpu_load()
            .expect("Read initial Windows sample")
            .expect("Counter warmed up");
        let running = Arc::new(AtomicBool::new(true));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let running = running.clone();
                std::thread::spawn(move || {
                    let mut n = 1.0f64;
                    while running.load(Ordering::Relaxed) {
                        n = std::hint::black_box((n + 1.0).sqrt() * 1.001);
                    }
                    std::hint::black_box(n);
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(1100));
        let loaded_result = cpu_load();
        running.store(false, Ordering::Relaxed);
        for worker in workers {
            worker.join().expect("Disposable load thread");
        }
        let loaded = loaded_result
            .expect("Read loaded Windows sample")
            .expect("Counter data");
        std::thread::sleep(Duration::from_millis(1100));
        let after = cpu_load()
            .expect("Read recovered Windows sample")
            .expect("Counter data");
        reset_cpu_load();
        let changed = idle.values.iter().any(|first| {
            loaded.values.iter().any(|next| {
                first.group == next.group
                    && first.logical == next.logical
                    && (first.load - next.load).abs() > 0.1
            })
        });
        let final_plan = active().expect("Read final active plan");
        let report = serde_json::json!({
            "read_only_power": true,
            "topology_count": topology.len(),
            "topology": topology.iter().map(|c| serde_json::json!({"group":c.group,"logical":c.logical,"physical":c.physical,"efficiency":c.efficiency})).collect::<Vec<_>>(),
            "active_before": original, "active_after": final_plan,
            "load_changed": changed, "samples": [idle, loaded, after],
            "measurement": "Windows PDH Processor Information % Processor Time; sampled busy time, not process ownership"
        });
        let report_path = std::env::var_os("NN6_CPU_REPORT")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("NN6-CPU-live-counter-test.json"));
        crate::model::atomic_json(&report_path, &report).expect("Write live telemetry report");
        assert!(
            changed,
            "Expected actual changing CPU counter values; inspect {}",
            report_path.display()
        );
        assert_eq!(
            original, final_plan,
            "The active plan changed during this read-only probe"
        );
        println!("Live Windows CPU counter report: {}", report_path.display());
    }
}
