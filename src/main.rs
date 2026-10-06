// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
slint::include_modules!();
mod automation;
mod cpu_topology;
mod discovery;
mod engine;
mod exercise;
mod frame_probe;
mod gui_instance;
mod ipc;
mod model;
mod power;
mod power_tools;
mod preferences;
mod pro_probe;
mod pro_view;
mod protection;
mod runtime_scope;
mod sensors;
mod session_export;
mod system_integration;
mod ui;
mod win;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if let Err(error) = runtime_scope::initialize(&args) {
        gui_instance::show_startup_error(&error);
        std::process::exit(1);
    }
    std::panic::set_hook(Box::new(|info| {
        let dir = model::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("native-crash.txt"),
            format!("{} {info}", model::now_ms()),
        );
    }));
    // Verification is an explicit offline diagnostic. Never block guardian recovery
    // or alter telemetry because a debugger, VM, or changed build is present.
    let outcome = if args.get(1).is_some_and(|s| s == "--write-release-manifest") {
        args.get(2)
            .ok_or_else(|| "Expected a manifest output path".to_string())
            .and_then(|p| protection::write_manifest(std::path::Path::new(p)))
    } else if args.get(1).is_some_and(|s| s == "--verify-release") {
        args.get(2)
            .ok_or_else(|| "Expected a trusted manifest path".to_string())
            .and_then(|p| protection::verify_manifest(std::path::Path::new(p)))
    } else if args.get(1).is_some_and(|s| s == "--gui-instance-probe") {
        args.get(2)
            .ok_or_else(|| "Expected an absolute singleton probe report path".to_string())
            .and_then(|path| {
                let hold = args
                    .get(3)
                    .map(|s| s.parse::<u64>())
                    .transpose()
                    .map_err(|_| "Invalid singleton probe hold_ms".to_string())?
                    .unwrap_or(5000);
                gui_instance::probe(std::path::Path::new(path), hold)
            })
    } else if args.get(1).is_some_and(|s| s == "--pro-probe") {
        pro_probe::run(
            args.get(2).map(String::as_str).unwrap_or("pro-probe.json"),
            args.iter().any(|s| s == "--live"),
        )
    } else if args.get(1).is_some_and(|s| s == "--topology-probe") {
        args.get(2)
            .filter(|p| std::path::Path::new(p).is_absolute())
            .ok_or_else(|| "Expected an absolute topology report path".to_string())
            .and_then(|p| {
                model::atomic_json(std::path::Path::new(p), &system_integration::cpu_topology())
            })
    } else if args.get(1).is_some_and(|s| s == "--system-probe") {
        model::atomic_json(
            &std::path::PathBuf::from(
                args.get(2)
                    .map(String::as_str)
                    .unwrap_or("system-probe.json"),
            ),
            &system_integration::probe(),
        )
    } else if args.iter().any(|a| a == "--fullscreen-probe") {
        frame_probe::fullscreen()
    } else if args.iter().any(|a| a == "--sleeper") {
        std::thread::sleep(std::time::Duration::from_secs(90));
        Ok(())
    } else if args.get(1).is_some_and(|s| s == "--exercise") {
        exercise::run(
            args.get(2)
                .map(String::as_str)
                .unwrap_or("native-exercise.json"),
            args.iter().any(|a| a == "--live"),
        )
    } else if args.iter().any(|a| a == "--daemon") {
        engine::daemon(model::data_dir(), runtime_scope::current().dry)
    } else if args.get(1).is_some_and(|s| s == "--guardian") {
        if let Some(pid) = args.get(2).and_then(|s| s.parse().ok()) {
            engine::guardian(pid);
        }
        Ok(())
    } else if args.get(1).is_some_and(|s| s == "--diagnose") {
        let v = serde_json::json!({"active":win::active(),"plans":win::plans(),"process_count":win::processes().map(|p|p.len()),"core_count":win::cores().len(),"sid":win::sid()});
        let path = args
            .get(2)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| model::data_dir().join("diagnostics.json"));
        model::atomic_json(&path, &v)
    } else {
        let dry = runtime_scope::current().dry;
        match gui_instance::acquire(dry) {
            Ok(gui_instance::Launch::Primary(instance)) => {
                ui::run(dry, args.iter().any(|a| a == "--start-paused"), &instance)
            }
            Ok(gui_instance::Launch::Forwarded) => Ok(()),
            Err(error) => {
                gui_instance::show_startup_error(&error);
                Err(error)
            }
        }
    };
    if let Err(e) = outcome {
        let dir = model::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("native-startup-error.txt"), &e);
        eprintln!("{e}");
        std::process::exit(1);
    }
}
