// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Opt-in instrumented rendering test. Not active in ordinary app launches.
use crate::{AppWindow, FullscreenProbe, model, win};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub fn fullscreen() -> Result<(), String> {
    let ui = FullscreenProbe::new().map_err(|e| e.to_string())?;
    ui.show().map_err(|e| e.to_string())?;
    let begin = Instant::now();
    let ready = Arc::new(Mutex::new(None::<Result<f64, String>>));
    let ready_task = ready.clone();
    let weak = ui.as_weak();
    // An arbitrary delay does not prove an HWND exists with winit 0.30. Await
    // the same supported readiness accessor used by the production HUD.
    let setup = slint::spawn_local(async move {
        if let Some(ui) = weak.upgrade() {
            let result = match ui.window().winit_window().await {
                Ok(window) => match window.window_handle().map_err(|e| e.to_string()) {
                    Ok(handle) => match handle.as_raw() {
                        RawWindowHandle::Win32(handle) => {
                            win::window(handle.hwnd.get() as usize, 5);
                            Ok(begin.elapsed().as_secs_f64() * 1000.)
                        }
                        _ => Err("Fixture did not receive a Windows HWND".into()),
                    },
                    Err(error) => Err(error),
                },
                Err(error) => Err(error.to_string()),
            };
            if let Ok(mut ready) = ready_task.lock() {
                *ready = Some(result);
            }
            slint::Timer::single_shot(Duration::from_secs(4), || {
                let _ = slint::quit_event_loop();
            });
        }
    })
    .map_err(|error| error.to_string())?;
    // Bounded even if the backend can never create a native window.
    slint::Timer::single_shot(Duration::from_secs(8), || {
        let _ = slint::quit_event_loop();
    });
    let result = ui.run().map_err(|e| e.to_string());
    setup.abort();
    if let Some(path) = std::env::var_os("NN6_FRAME_FIXTURE_REPORT")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        let _ = model::atomic_json(
            &path,
            &serde_json::json!({
                "native_window_ready": *ready.lock().unwrap(),
                "lifetime_ms": begin.elapsed().as_secs_f64() * 1000.,
                "event_loop": result,
                "scope": "Readiness and focus request only; actual full-screen detection is recorded by the main app."
            }),
        );
    }
    result
}

fn preflight(
    report: &Path,
    isolated: Option<&Path>,
    dry: bool,
    config: &model::Config,
) -> Result<(), String> {
    if !dry || !isolated.is_some_and(Path::is_absolute) || !report.is_absolute() {
        return Err("Frame instrumentation requires --dry-run, an absolute NN6_TEST_DATA_DIR and absolute NN6_FRAME_REPORT.".into());
    }
    if !config
        .names()
        .iter()
        .any(|name| name.eq_ignore_ascii_case("NN6FullscreenProbe.exe"))
    {
        return Err("The isolated configuration must watch NN6FullscreenProbe.exe.".into());
    }
    Ok(())
}

pub fn install(ui: &AppWindow) {
    let Some(path) = std::env::var_os("NN6_FRAME_REPORT") else {
        return;
    };
    let path = PathBuf::from(path);
    let isolated = std::env::var_os("NN6_TEST_DATA_DIR").map(PathBuf::from);
    let dry = std::env::args().any(|arg| arg == "--dry-run");
    let setup = model::Config::load(&model::data_dir())
        .and_then(|config| preflight(&path, isolated.as_deref(), dry, &config));
    if let Err(error) = setup {
        ui.set_error(true);
        ui.set_status(error.clone().into());
        if path.is_absolute() && !path.exists() {
            let _ = model::atomic_json(
                &path,
                &serde_json::json!({"setup_error": error, "scope": "Probe did not run."}),
            );
        }
        return;
    }
    let fixture_report = path.with_extension("fixture.json");
    if path.exists() || fixture_report.exists() {
        ui.set_error(true);
        ui.set_status(
            "Use fresh frame and fixture report paths; previous evidence will not be overwritten."
                .into(),
        );
        return;
    }
    let frames = Arc::new(Mutex::new(Vec::<f64>::new()));
    let events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let begin = Instant::now();
    let f = frames.clone();
    let notifier = ui.window().set_rendering_notifier(move |state, _| {
        if matches!(state, slint::RenderingState::BeforeRendering) {
            if let Ok(mut f) = f.lock() {
                f.push(begin.elapsed().as_secs_f64() * 1000.);
            }
        }
    });
    let supported = notifier.is_ok();
    for (t, page) in [(4000, 1), (6000, 2), (8000, 0)] {
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(t), move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_page(page);
            }
        });
    }
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_secs(10), move || {
        if let Some(ui) = weak.upgrade() {
            if !ui.get_monitoring() {
                ui.invoke_action("toggle".into());
            }
        }
    });
    let e = events.clone();
    let fixture_path = fixture_report.clone();
    slint::Timer::single_shot(Duration::from_secs(12), move || {
        let outcome = (|| -> Result<(), String> {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let dest = model::data_dir().join("NN6FullscreenProbe.exe");
            let mut source = std::fs::File::open(exe).map_err(|e| e.to_string())?;
            let mut target = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&dest)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut source, &mut target).map_err(|e| e.to_string())?;
            drop(target);
            win::hidden_command(&dest.to_string_lossy())
                .arg("--fullscreen-probe")
                .env("NN6_FRAME_FIXTURE_REPORT", fixture_path)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(())
        })();
        e.lock()
            .unwrap()
            .push(serde_json::json!({"probe_launch":outcome, "time_ms": begin.elapsed().as_secs_f64() * 1000.}));
    });
    for t in [13500, 14500, 15500, 17500] {
        let weak = ui.as_weak();
        let e = events.clone();
        slint::Timer::single_shot(Duration::from_millis(t), move || {
            if let Some(ui) = weak.upgrade() {
                e.lock().unwrap().push(serde_json::json!({"scheduled_ms":t,"time_ms":begin.elapsed().as_secs_f64()*1000.,"suspended":ui.get_suspended(),"render_active":ui.get_render_active(),"cpu_warming":ui.get_cpu_warming(),"monitoring":ui.get_monitoring(),"status":ui.get_status().to_string(),"running":ui.get_process_summary().to_string()}));
            }
        });
    }
    slint::Timer::single_shot(Duration::from_secs(20), move || {
        let f = frames.lock().unwrap();
        let mut motion = vec![];
        for start in [4000., 6000., 8000.] {
            let times: Vec<_> = f
                .iter()
                .copied()
                .filter(|t| *t >= start + 100. && *t < start + 650.)
                .collect();
            let mut gaps: Vec<_> = times.windows(2).map(|w| w[1] - w[0]).collect();
            gaps.sort_by(f64::total_cmp);
            let percentile = |p: f64| {
                gaps.get(((gaps.len().saturating_sub(1)) as f64 * p) as usize)
                    .copied()
                    .unwrap_or(0.)
            };
            motion.push(serde_json::json!({"start_ms":start,"frames_in_550_ms":times.len(),"median_interval_ms":percentile(0.5),"p95_interval_ms":percentile(0.95)}));
        }
        let fixture = std::fs::read_to_string(&fixture_report)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        let result = serde_json::json!({"render_notifier_supported":supported,"motion":motion,"paused_frames_between_2s_and_3s":f.iter().filter(|t|**t>=2000.&&**t<3000.).count(),"frames_between_14s_and_15s":f.iter().filter(|t|**t>=14000.&&**t<15000.).count(),"events":*events.lock().unwrap(),"fixture":fixture,"frame_times_ms":*f,"scope":"Application render callbacks only; not GPU allocation, DirectX-exclusive mode or a DWM profiler measurement."});
        let _ = model::atomic_json(&path, &result);
        let _ = slint::quit_event_loop();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_probe_requires_explicit_disposable_dry_configuration() {
        let path = Path::new(r"C:\NN6Test\frames.json");
        let isolated = Some(Path::new(r"C:\NN6Test"));
        let mut config = model::defaults();
        assert!(preflight(path, isolated, true, &config).is_err());
        config.add_names(&["NN6FullscreenProbe.exe".into()]);
        assert!(preflight(path, isolated, true, &config).is_ok());
        assert!(preflight(path, isolated, false, &config).is_err());
        assert!(preflight(path, None, true, &config).is_err());
        assert!(preflight(Path::new("frames.json"), isolated, true, &config).is_err());
    }
}
