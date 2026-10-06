// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Persistent, read-only hardware sensor worker. No WMI/driver work on Slint.
//! NVML: https://docs.nvidia.com/deploy/nvml-api/latest/index.html
//! ADL: https://gpuopen-librariesandsdks.github.io/adl/group__OVERDRIVE8API.html
//! LHM/OHM expose WMI Sensor classes; no generic shared-memory ABI is assumed.
use crate::system_integration::{SensorSnapshot, wmi_sensor_snapshot};
use nvml_wrapper::{Nvml, enum_wrappers::device::TemperatureSensor};
use std::{
    ffi::{OsString, c_void},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const WMI_INTERVAL: Duration = Duration::from_secs(10);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
}
unsafe extern "C" {
    fn nn6_adl_open(error: *mut u32) -> *mut c_void;
    fn nn6_adl_sample(handle: *mut c_void, out: *mut RawGpu) -> u32;
    fn nn6_adl_suspend(handle: *mut c_void);
    fn nn6_adl_close(handle: *mut c_void);
}
#[repr(C)]
struct RawGpu {
    temperature: f64,
    utilization: f64,
    fan_percent: f64,
    fan_rpm: f64,
    name: [u16; 128],
    connected: u32,
}

/// Preload an absolute System32 driver DLL and its dependencies using restricted
/// search flags. The wrapper's subsequent load uses this same absolute module;
/// no current-directory/PATH/default-name NVML search is permitted.
struct SystemLibrary(*mut c_void);
impl SystemLibrary {
    fn open(filename: &str) -> Result<(Self, PathBuf), String> {
        let mut buffer = [0u16; 32768];
        let count = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if count == 0 || count as usize >= buffer.len() {
            return Err("Windows system directory could not be resolved".into());
        }
        let path = PathBuf::from(OsString::from_wide(&buffer[..count as usize])).join(filename);
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let module = unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), 0x100 | 0x800) };
        if module.is_null() {
            return Err(format!(
                "System32/{filename}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok((Self(module), path))
    }
}
impl Drop for SystemLibrary {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.0);
        }
    }
}

struct Nvidia {
    // Rust drops fields in declaration order: shut down NVML before releasing
    // our secure pre-load reference to its driver DLL.
    nvml: Nvml,
    _library: SystemLibrary,
    index: u32,
    name: String,
}
impl Nvidia {
    fn new() -> Result<Self, String> {
        let (library, path) = SystemLibrary::open("nvml.dll")?;
        let nvml = Nvml::builder()
            .lib_path(path.as_os_str())
            .init()
            .map_err(|e| e.to_string())?;
        let total = nvml.device_count().map_err(|e| e.to_string())?.min(64);
        let (index, name) = (0..total)
            .find_map(|index| {
                let device = nvml.device_by_index(index).ok()?;
                Some((index, device.name().unwrap_or_else(|_| "NVIDIA GPU".into())))
            })
            .ok_or("NVML has no accessible GPU")?;
        Ok(Self {
            nvml,
            _library: library,
            index,
            name,
        })
    }
    fn sample(&self) -> GpuSample {
        let mut sample = GpuSample {
            provider: "NVIDIA NVML".into(),
            name: format!("{} [NVML GPU {}]", self.name, self.index),
            ..GpuSample::default()
        };
        match self.nvml.device_by_index(self.index) {
            Ok(device) => {
                sample.connected = true;
                sample.temperature = device
                    .temperature(TemperatureSensor::Gpu)
                    .ok()
                    .and_then(|v| valid(v as f64, 150.));
                sample.utilization = device
                    .utilization_rates()
                    .ok()
                    .and_then(|v| valid(v.gpu as f64, 100.));
                // These are distinct NVML calls. RPM is never derived from %.
                // NVML describes the intended fan speed, not a blocked-fan alarm.
                sample.fan_percent = device.fan_speed(0).ok().and_then(|v| valid(v as f64, 200.));
                sample.fan_rpm = device
                    .fan_speed_rpm(0)
                    .ok()
                    .and_then(|v| valid(v as f64, 50000.));
                if !sample.has_values() {
                    sample.error = Some(
                        "NVML GPU is present, but these sensor calls are unsupported or failed."
                            .into(),
                    );
                }
            }
            Err(e) => sample.error = Some(format!("NVML GPU unavailable: {e}")),
        }
        sample
    }
}

struct Amd(*mut c_void);
impl Amd {
    fn new() -> Result<Self, String> {
        let mut error = 0;
        let handle = unsafe { nn6_adl_open(&mut error) };
        if handle.is_null() {
            Err(format!(
                "AMD ADL8 unavailable or unsupported (Windows {error})"
            ))
        } else {
            Ok(Self(handle))
        }
    }
    fn suspend(&mut self) {
        unsafe { nn6_adl_suspend(self.0) }
    }
    fn sample(&mut self) -> GpuSample {
        let mut raw = RawGpu {
            temperature: -1.,
            utilization: -1.,
            fan_percent: -1.,
            fan_rpm: -1.,
            name: [0; 128],
            connected: 0,
        };
        let code = unsafe { nn6_adl_sample(self.0, &mut raw) };
        let length = raw
            .name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(raw.name.len());
        let available = |v, max| if code == 0 { valid(v, max) } else { None };
        GpuSample {
            connected: raw.connected != 0,
            provider: "AMD ADL8 PMLog".into(),
            name: String::from_utf16_lossy(&raw.name[..length]),
            temperature: available(raw.temperature, 150.),
            utilization: available(raw.utilization, 100.),
            fan_percent: available(raw.fan_percent, 100.),
            fan_rpm: available(raw.fan_rpm, 50000.),
            error: if code == 1237 {
                Some(
                    "AMD sensor session warming up; next sample will read supported metrics."
                        .into(),
                )
            } else if code != 0 {
                Some(format!("AMD sensor query unavailable (Windows {code})."))
            } else {
                None
            },
        }
    }
}
impl Drop for Amd {
    fn drop(&mut self) {
        unsafe { nn6_adl_close(self.0) }
    }
}

#[derive(Default)]
struct GpuSample {
    connected: bool,
    provider: String,
    name: String,
    temperature: Option<f64>,
    utilization: Option<f64>,
    fan_percent: Option<f64>,
    fan_rpm: Option<f64>,
    error: Option<String>,
}
impl GpuSample {
    fn has_values(&self) -> bool {
        [
            self.temperature,
            self.utilization,
            self.fan_percent,
            self.fan_rpm,
        ]
        .iter()
        .any(Option::is_some)
    }
}
fn valid(value: f64, maximum: f64) -> Option<f64> {
    (value.is_finite() && (0.0..=maximum).contains(&value)).then_some(value)
}
fn wmi_due(previous: Option<Instant>, now: Instant, explicit: bool) -> bool {
    explicit || previous.is_none_or(|at| now.saturating_duration_since(at) >= WMI_INTERVAL)
}

#[derive(Default)]
struct WmiCache {
    snapshot: SensorSnapshot,
    at: Option<Instant>,
    requested_at: Option<Instant>,
    generation: u64,
    pending: bool,
    suspended: bool,
}
impl WmiCache {
    fn accept(&mut self, generation: u64, snapshot: SensorSnapshot, at: Instant) {
        if self.suspended || generation != self.generation {
            return;
        }
        self.snapshot = snapshot;
        self.at = Some(at);
        self.pending = false;
    }
    fn suspend(&mut self) {
        if !self.suspended {
            self.generation = self.generation.wrapping_add(1);
        }
        self.suspended = true;
        self.snapshot = SensorSnapshot::default();
        self.at = None;
        self.requested_at = None;
        self.pending = false;
    }
    fn current(&self, now: Instant) -> SensorSnapshot {
        // Never display an indefinitely stale fallback while its next COM call
        // is blocked. This is local cache age, not provider acquisition age.
        if self
            .at
            .is_some_and(|at| now.saturating_duration_since(at) > Duration::from_secs(30))
        {
            SensorSnapshot {
                error: Some(
                    "WMI fallback poll expired; CPU/provider values are unavailable.".into(),
                ),
                ..Default::default()
            }
        } else {
            self.snapshot.clone()
        }
    }
}
struct WmiWorker {
    requests: mpsc::SyncSender<u64>,
    results: mpsc::Receiver<(u64, SensorSnapshot, Instant)>,
    stop: Arc<AtomicBool>,
}
impl WmiWorker {
    fn new() -> Self {
        let (requests, receive) = mpsc::sync_channel(1);
        let (send, results) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        std::thread::spawn(move || {
            while let Ok(generation) = receive.recv() {
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                let result = wmi_sensor_snapshot();
                if stopping.load(Ordering::Relaxed)
                    || send.send((generation, result, Instant::now())).is_err()
                {
                    break;
                }
            }
        });
        Self {
            requests,
            results,
            stop,
        }
    }
}
impl Drop for WmiWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Dropping the sender wakes an idle worker. An in-flight COM operation
        // gets its existing best-effort timeout; never join it on the UI path.
    }
}

/// Construct inside the long-lived worker, not before moving it to a thread.
/// NVML/ADL setup is cached for that worker. No thread per direct sensor poll.
pub struct SensorReader {
    nvidia: Option<Nvidia>,
    amd: Option<Amd>,
    direct_error: String,
    wmi: WmiCache,
    worker: WmiWorker,
}
impl SensorReader {
    pub fn new() -> Self {
        let (nvidia, nvidia_error) = match Nvidia::new() {
            Ok(provider) => (Some(provider), String::new()),
            Err(error) => (None, error),
        };
        // One deterministic GPU, named in every snapshot. Never combine metrics
        // from an NVIDIA GPU and a different AMD/WMI GPU into a fictional device.
        let (amd, amd_error) = if nvidia.is_none() {
            match Amd::new() {
                Ok(provider) => (Some(provider), String::new()),
                Err(error) => (None, error),
            }
        } else {
            (None, String::new())
        };
        Self {
            nvidia,
            amd,
            direct_error: [nvidia_error, amd_error]
                .into_iter()
                .filter(|v| !v.is_empty())
                .collect::<Vec<_>>()
                .join("; "),
            wmi: WmiCache::default(),
            worker: WmiWorker::new(),
        }
    }
    /// Stop AMD's owned logging session while hidden. NVML initialization remains
    /// cached, but no query is performed. Invalidate WMI cache on resume.
    pub fn suspend(&mut self) {
        if let Some(amd) = self.amd.as_mut() {
            amd.suspend();
        }
        self.wmi.suspend();
    }
    /// Called nominally every two seconds when visible/HUD; WMI at most every
    /// ten seconds unless the user explicitly requests Retry. There is no claim
    /// that an external provider refreshed its own sample during this interval.
    pub fn sample(&mut self, explicit: bool) -> SensorSnapshot {
        while let Ok((generation, snapshot, at)) = self.worker.results.try_recv() {
            self.wmi.accept(generation, snapshot, at);
        }
        self.wmi.suspended = false;
        if !self.wmi.pending && wmi_due(self.wmi.requested_at, Instant::now(), explicit) {
            if self.worker.requests.try_send(self.wmi.generation).is_ok() {
                self.wmi.pending = true;
                self.wmi.requested_at = Some(Instant::now());
            }
        }
        let gpu = if let Some(nvidia) = &self.nvidia {
            Some(nvidia.sample())
        } else {
            self.amd.as_mut().map(Amd::sample)
        };
        combine(
            self.wmi.current(Instant::now()),
            gpu,
            self.wmi.at.map(|v| v.elapsed()),
            &self.direct_error,
        )
    }
    /// CLI-only initial observation, bounded even if a WMI provider ignores COM
    /// cancellation. The UI never calls this method or waits for WMI completion.
    pub fn sample_for_probe(&mut self) -> SensorSnapshot {
        let _ = self.sample(true);
        if let Ok((generation, snapshot, at)) =
            self.worker.results.recv_timeout(Duration::from_secs(7))
        {
            self.wmi.accept(generation, snapshot, at);
        }
        self.sample(false)
    }
}

fn combine(
    mut out: SensorSnapshot,
    gpu: Option<GpuSample>,
    wmi_age: Option<Duration>,
    direct_error: &str,
) -> SensorSnapshot {
    out.wmi_polled_at_ms = (out.polled_at_ms != 0).then_some(out.polled_at_ms);
    let wmi_error = out.error.take();
    let mut gpu_error = None;
    if let Some(gpu) = gpu {
        out.connected |= gpu.connected;
        // A wholly unavailable direct snapshot may fall back as a whole to the
        // WMI GPU. Partial direct data remains coherent and never borrows another
        // adapter's temperature or fan values to fill unsupported direct metrics.
        if gpu.has_values() {
            out.gpu_c = gpu.temperature;
            out.gpu_utilization = gpu.utilization;
            out.gpu_fan_percent = gpu.fan_percent;
            out.gpu_fan_rpm = gpu.fan_rpm;
            out.gpu_label = gpu.name;
            out.gpu_provider = gpu
                .temperature
                .map(|_| gpu.provider.clone())
                .unwrap_or_default();
            out.gpu_utilization_provider = gpu
                .utilization
                .map(|_| gpu.provider.clone())
                .unwrap_or_default();
            out.gpu_fan_provider = (gpu.fan_percent.is_some() || gpu.fan_rpm.is_some())
                .then(|| gpu.provider.clone())
                .unwrap_or_default();
        }
        gpu_error = gpu.error;
    }
    let sources = [
        &out.cpu_provider,
        &out.gpu_provider,
        &out.power_provider,
        &out.fan_provider,
        &out.gpu_utilization_provider,
        &out.gpu_fan_provider,
    ];
    let mut providers = Vec::new();
    for source in sources {
        if !source.is_empty() && !providers.contains(source) {
            providers.push(source.clone());
        }
    }
    out.provider = if providers.is_empty() {
        "No supported readings".into()
    } else {
        providers.join(" + ")
    };
    out.readings = [
        out.cpu_c,
        out.gpu_c,
        out.package_w,
        out.fan_rpm,
        out.gpu_utilization,
        out.gpu_fan_percent,
        out.gpu_fan_rpm,
    ]
    .iter()
    .filter(|v| v.is_some())
    .count() as u32;
    out.polled_at_ms = crate::model::now_ms();
    out.freshness_verified = false;
    let age = wmi_age
        .map(|v| format!("last poll {} s ago", v.as_secs()))
        .unwrap_or_else(|| "initial query pending; no completed fallback sample".into());
    out.freshness_note = format!(
        "Direct GPU queries: 2 s while visible/HUD. WMI: 10 s fallback, {age}; provider acquisition timestamps unavailable. NVML fan values are intended speed; WMI values may be externally cached."
    );
    out.error = if out.readings == 0 {
        Some(
            [
                gpu_error,
                wmi_error,
                (!direct_error.is_empty()).then(|| direct_error.to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("; "),
        )
    } else if out.cpu_c.is_none() {
        Some("CPU temperature unavailable: an existing supported LHM/OHM WMI provider must expose it. GPU readings remain independent.".into())
    } else {
        gpu_error
    };
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_invalid_and_measured_zero_remain_distinct() {
        assert_eq!(valid(0., 100.), Some(0.));
        for v in [-1., f64::NAN, f64::INFINITY, 101.] {
            assert_eq!(valid(v, 100.), None);
        }
    }
    #[test]
    fn wmi_is_throttled_and_explicit_retry_is_immediate() {
        let now = Instant::now();
        assert!(wmi_due(None, now, false));
        assert!(!wmi_due(Some(now), now + Duration::from_secs(9), false));
        assert!(wmi_due(Some(now), now + Duration::from_secs(10), false));
        assert!(wmi_due(Some(now), now, true));
    }
    #[test]
    fn suspended_generation_rejects_old_wmi_results_and_cache_expires() {
        let now = Instant::now();
        let sample = SensorSnapshot {
            cpu_c: Some(44.),
            ..Default::default()
        };
        let mut cache = WmiCache::default();
        cache.accept(0, sample.clone(), now);
        assert_eq!(cache.current(now).cpu_c, Some(44.));
        assert_eq!(cache.current(now + Duration::from_secs(31)).cpu_c, None);
        cache.suspend();
        cache.suspended = false;
        cache.accept(0, sample.clone(), now);
        assert_eq!(cache.current(now).cpu_c, None);
        cache.accept(cache.generation, sample, now);
        assert_eq!(cache.current(now).cpu_c, Some(44.));
    }
    #[test]
    fn nvml_percent_never_becomes_rpm_or_cpu_temperature() {
        let out = combine(
            SensorSnapshot::default(),
            Some(GpuSample {
                connected: true,
                provider: "NVML".into(),
                temperature: Some(51.),
                fan_percent: Some(35.),
                utilization: Some(0.),
                ..Default::default()
            }),
            Some(Duration::ZERO),
            "",
        );
        assert_eq!(out.cpu_c, None);
        assert_eq!(out.fan_rpm, None);
        assert_eq!(out.gpu_fan_rpm, None);
        assert_eq!(out.gpu_fan_percent, Some(35.));
        assert_eq!(out.gpu_utilization, Some(0.));
        assert_eq!(out.gpu_provider, "NVML");
        assert_eq!(out.cpu_provider, "");
    }
    #[test]
    fn selected_direct_gpu_never_inherits_another_gpu_wmi_temperature() {
        let wmi = SensorSnapshot {
            cpu_c: Some(41.),
            gpu_c: Some(65.),
            cpu_provider: "LHM".into(),
            gpu_provider: "LHM".into(),
            ..Default::default()
        };
        let out = combine(
            wmi,
            Some(GpuSample {
                provider: "NVML".into(),
                utilization: Some(10.),
                ..Default::default()
            }),
            Some(Duration::from_secs(6)),
            "",
        );
        assert_eq!(out.cpu_c, Some(41.));
        assert_eq!(out.gpu_c, None);
        assert_eq!(out.gpu_provider, "");
        assert_eq!(out.gpu_utilization_provider, "NVML");
        assert!(out.freshness_note.contains("6 s ago"));
        assert!(!out.freshness_verified);
    }
    #[test]
    fn wmi_fallback_retains_its_metric_provenance_when_direct_gpu_is_absent() {
        let wmi = SensorSnapshot {
            gpu_c: Some(58.),
            gpu_provider: "OHM".into(),
            ..Default::default()
        };
        let out = combine(wmi, None, Some(Duration::ZERO), "No NVML DLL");
        assert_eq!(out.gpu_c, Some(58.));
        assert_eq!(out.gpu_provider, "OHM");
        assert_eq!(out.gpu_utilization, None);
        assert_eq!(out.readings, 1);
    }
    #[test]
    fn wholly_unavailable_direct_gpu_falls_back_as_one_coherent_wmi_device() {
        let wmi = SensorSnapshot {
            gpu_c: Some(58.),
            gpu_provider: "OHM".into(),
            gpu_label: "GPU B".into(),
            ..Default::default()
        };
        let out = combine(
            wmi,
            Some(GpuSample {
                connected: true,
                provider: "NVML".into(),
                name: "GPU A".into(),
                error: Some("Unsupported".into()),
                ..Default::default()
            }),
            Some(Duration::ZERO),
            "",
        );
        assert_eq!(out.gpu_c, Some(58.));
        assert_eq!(out.gpu_provider, "OHM");
        assert_eq!(out.gpu_label, "GPU B");
        assert_eq!(out.gpu_utilization, None);
    }
    #[test]
    fn pending_wmi_is_never_reported_as_just_polled() {
        let out = combine(SensorSnapshot::default(), None, None, "");
        assert!(!out.freshness_note.contains("0 s ago"));
        assert!(out.freshness_note.contains("no completed fallback sample"));
        assert_eq!(out.wmi_polled_at_ms, None);
    }
}
