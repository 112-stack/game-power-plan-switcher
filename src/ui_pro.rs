// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
// Included by ui.rs: operational controllers for the additive Pro bindings.
// Native callbacks are always marshalled to Slint's event loop. File/plan and
// sensor work runs in bounded background jobs; no hardware DLL is injected.
impl Controller {
    fn plan_name(&self, guid: &str) -> String {
        if guid.is_empty() {
            "Use global Gaming plan".into()
        } else {
            self.state
                .plans
                .iter()
                .find(|p| p.guid.eq_ignore_ascii_case(guid))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| format!("Unavailable: {guid}"))
        }
    }
    fn apply_appearance(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let p = &self.preferences;
        ui.set_compact_mode(p.compact);
        ui.set_dark_mode(if p.sync_theme {
            crate::system_integration::system_theme_dark()
        } else {
            p.dark
        });
        ui.set_sync_system_theme(p.sync_theme);
        ui.set_native_frame(p.native_frame);
        ui.set_heatmap_view(p.heatmap);
        ui.set_auto_discover(p.auto_discover);
        ui.set_minimize_to_tray(p.close_to_tray);
        ui.set_start_minimized(p.start_minimized);
        ui.set_global_hotkey(p.hotkey.clone().into());
        ui.set_visual_overlay_enabled(p.hud_enabled);
        ui.set_overlay_position(p.hud_position);
        ui.set_overlay_opacity(p.hud_opacity);
        ui.set_overlay_hotkey(p.hud_hotkey.clone().into());
        ui.set_ui_font(crate::system_integration::preferred_font().into());
        ui.set_mono_font("Consolas".into());
        ui.window()
            .with_winit_window(|w| w.set_decorations(p.native_frame));
        ui.window().set_size(slint::LogicalSize::new(
            if p.compact { 584. } else { 800. },
            if p.compact { 526. } else { 600. },
        ));
    }
    fn save_preferences(&self) {
        if let Err(e) = self.preferences.save() {
            self.notice(&format!("Preferences: {e}"), true);
        }
    }
    fn close_window(&self) {
        if let Some(ui) = self.ui.upgrade() {
            if self.preferences.close_to_tray && ui.get_tray_available() {
                let _ = ui.hide();
                self.view.focused.set(false);
                self.view.publish();
            } else {
                let _ = slint::quit_event_loop();
            }
        }
    }
    fn show_window(&self) {
        // An explicit tray/singleton SHOW wins over the one deferred initial
        // start-minimized action, without changing the saved preference.
        self.show_requested.set(true);
        if let Some(ui) = self.ui.upgrade() {
            let _ = ui.show();
            ui.window().with_winit_window(|w| {
                w.set_minimized(false);
                w.focus_window();
            });
            self.view.publish();
        }
    }
    fn manual(&self, role: &str) {
        if self.state.observer {
            self.notice(
                "Use this app instead before controlling the shared engine.",
                true,
            );
            return;
        }
        self.send(Command {
            kind: MANUAL_PLAN,
            text: role.into(),
            ..Default::default()
        });
    }
    fn system_event(&mut self, event: crate::system_integration::SystemEvent) {
        use crate::system_integration::SystemEvent;
        match event {
            SystemEvent::Show => self.show_window(),
            SystemEvent::QuickGaming => self.manual("gaming"),
            SystemEvent::QuickDefault => self.manual("default"),
            SystemEvent::ToggleMonitoring => self.simple(TOGGLE),
            SystemEvent::TogglePlan => {
                let gaming = self.state.config.as_ref().is_some_and(|c| {
                    self.state.active_guid.eq_ignore_ascii_case(&c.gaming)
                        || c.games.iter().any(|g| {
                            !g.power_plan.is_empty()
                                && g.power_plan.eq_ignore_ascii_case(&self.state.active_guid)
                        })
                });
                self.manual(if gaming { "default" } else { "gaming" });
            }
            SystemEvent::Exit => {
                let _ = slint::quit_event_loop();
            }
            SystemEvent::ToggleOverlay => {
                if let Some(ui) = self.ui.upgrade() {
                    ui.set_visual_overlay_enabled(!ui.get_visual_overlay_enabled());
                    self.pro_setting("visual-overlay");
                }
            }
            SystemEvent::ThemeChanged(dark) => {
                if self.preferences.sync_theme {
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_dark_mode(dark);
                    }
                }
            }
            SystemEvent::Error(e) => {
                if let Some(ui) = self.ui.upgrade() {
                    ui.set_tray_available(false);
                    if !ui.window().is_visible() {
                        self.show_window();
                    }
                }
                self.last_tray.set(None);
                self.notice(&e, true);
            }
        }
    }
    fn pro_setting(&mut self, key: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let mut config = self.state.config.clone().unwrap_or_else(defaults);
        match key {
            "battery" => {
                config.battery_guard = Some(ui.get_auto_switch_battery());
                self.save(config);
            }
            "affinity" => {
                config.auto_affinity = Some(ui.get_auto_apply_affinity());
                self.save(config);
            }
            "time" => {
                config.time_rule = ui.get_auto_time_rule();
                config.time_start = ui.get_automation_start().trim().into();
                config.time_end = ui.get_automation_end().trim().into();
                self.save(config);
            }
            "foreground-only" => {
                config.foreground_only = ui.get_foreground_only();
                self.save(config);
            }
            "theme" => {
                self.preferences.dark = ui.get_dark_mode();
                self.preferences.sync_theme = false;
                ui.set_sync_system_theme(false);
                self.save_preferences();
            }
            "sync-theme" => {
                self.preferences.sync_theme = ui.get_sync_system_theme();
                if self.preferences.sync_theme {
                    ui.set_dark_mode(crate::system_integration::system_theme_dark());
                }
                self.save_preferences();
            }
            "native-frame" => {
                self.preferences.native_frame = ui.get_native_frame();
                self.apply_appearance();
                self.save_preferences();
            }
            "heatmap" => {
                self.preferences.heatmap = ui.get_heatmap_view();
                self.save_preferences();
            }
            "auto-discover" => {
                self.preferences.auto_discover = ui.get_auto_discover();
                self.save_preferences();
                if self.preferences.auto_discover {
                    self.simple(SCAN);
                    self.discovered_once = true;
                }
            }
            "close-to-tray" => {
                self.preferences.close_to_tray = ui.get_minimize_to_tray();
                self.save_preferences();
            }
            "start-minimized" => {
                self.preferences.start_minimized = ui.get_start_minimized();
                self.save_preferences();
            }
            "global-hotkey" => {
                let previous = self.preferences.hotkey.clone();
                self.preferences.hotkey = ui.get_global_hotkey().trim().into();
                if self.register_hotkeys() {
                    self.save_preferences();
                } else {
                    self.preferences.hotkey = previous;
                    ui.set_global_hotkey(self.preferences.hotkey.clone().into());
                    if let Some(integration) = &self.integration {
                        let _ = integration
                            .set_hotkeys(&self.preferences.hotkey, &self.preferences.hud_hotkey);
                    }
                }
            }
            "visual-overlay" | "overlay-layout" => {
                let previous = self.preferences.hud_hotkey.clone();
                let was_enabled = self.preferences.hud_enabled;
                self.preferences.hud_enabled = ui.get_visual_overlay_enabled();
                self.preferences.hud_position = ui.get_overlay_position().clamp(0, 3);
                self.preferences.hud_opacity = ui.get_overlay_opacity().clamp(0., 100.);
                self.preferences.hud_hotkey = ui.get_overlay_hotkey().trim().into();
                if !self.register_hotkeys() {
                    self.preferences.hud_hotkey = previous;
                    ui.set_overlay_hotkey(self.preferences.hud_hotkey.clone().into());
                    if let Some(integration) = &self.integration {
                        let _ = integration
                            .set_hotkeys(&self.preferences.hotkey, &self.preferences.hud_hotkey);
                    }
                }
                if self.preferences.hud_enabled && !was_enabled {
                    let _ = self.sensor_request.try_send(());
                }
                self.save_preferences();
                self.update_hud();
            }
            _ => self.notice(&format!("Unknown setting: {key}"), true),
        }
    }
    fn register_hotkeys(&self) -> bool {
        if let Some(integration) = &self.integration {
            match integration.set_hotkeys(&self.preferences.hotkey, &self.preferences.hud_hotkey) {
                Ok(status) => {
                    let errors: Vec<_> = [status.monitor_error, status.overlay_error]
                        .into_iter()
                        .flatten()
                        .collect();
                    if errors.is_empty() {
                        let message = format!(
                            "Plan toggle: {} · HUD: {}",
                            if status.monitor_registered {
                                self.preferences.hotkey.as_str()
                            } else {
                                "disabled"
                            },
                            if status.overlay_registered {
                                self.preferences.hud_hotkey.as_str()
                            } else {
                                "disabled"
                            }
                        );
                        if let Some(ui) = self.ui.upgrade() {
                            ui.set_hotkey_status(message.clone().into());
                        }
                        self.notice(&message, false);
                        return true;
                    } else {
                        let message = errors.join(" · ");
                        if let Some(ui) = self.ui.upgrade() {
                            ui.set_hotkey_status(message.clone().into());
                        }
                        self.notice(&message, true);
                    }
                }
                Err(e) => {
                    if let Some(ui) = self.ui.upgrade() {
                        ui.set_hotkey_status(e.clone().into());
                    }
                    self.notice(&e, true);
                }
            }
        }
        false
    }
    fn begin_job(&mut self, operation: impl FnOnce() -> Result<String, String> + Send + 'static) {
        if self.operation_busy {
            self.notice("Another file operation is still running.", true);
            return;
        }
        self.operation_busy = true;
        self.notice("Working…", false);
        let weak = self.ui.clone();
        thread::spawn(move || {
            let result = operation();
            let _ = weak.upgrade_in_event_loop(move |_| {
                CONTROL.with(|cell| {
                    if let Some(c) = cell.borrow().as_ref() {
                        let mut c = c.borrow_mut();
                        c.operation_busy = false;
                        match result {
                            Ok(s) => {
                                c.simple(REFRESH);
                                c.notice(&s, false);
                            }
                            Err(e) => c.notice(&e, true),
                        }
                    }
                })
            });
        });
    }
    fn power_tool(&mut self, role: i32, action: &str) {
        if self.state.dry_run && action == "duplicate" {
            self.notice("Preview cannot create Windows power plans. Open the normal app to duplicate a plan.",true);
            return;
        }
        if self.operation_busy {
            self.notice("Another file operation is still running.", true);
            return;
        }
        let config = self.state.config.clone().unwrap_or_else(defaults);
        let guid = if role == 0 {
            config.gaming
        } else {
            config.default_plan
        };
        match action {
            "export" => self.pick_for("export-plan", Some(guid)),
            "duplicate" => {
                self.begin_plan_job(crate::power_tools::Operation::Duplicate(guid), false)
            }
            "advanced" => {
                let result = win::hidden_command("control.exe")
                    .args(["powercfg.cpl", ",,3"])
                    .spawn();
                if let Err(e) = result {
                    self.notice(&e.to_string(), true);
                }
            }
            _ => self.notice("Unknown power-plan tool", true),
        }
    }
    fn pro_command(&mut self, command: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.state.dry_run && matches!(command, "import-plan" | "retry-elevated") {
            self.notice(
                "Preview does not import power plans or request administrator approval.",
                true,
            );
            return;
        }
        match command {
            "retry-elevated" => {
                if let Some(operation) = self.elevated_retry.clone() {
                    self.begin_plan_job(operation, true);
                }
            }
            "tray-gaming" => self.manual("gaming"),
            "tray-default" => self.manual("default"),
            "tray-pause" => self.simple(TOGGLE),
            "tray-exit" => {
                let _ = slint::quit_event_loop();
            }
            "connect-sensors" => {
                match self.sensor_request.try_send(()) {
                    Ok(()) => ui.set_provider_status("Checking installed GPU drivers and local LHM/OHM sensor providers. This does not install drivers or enable energy-savings estimates.".into()),
                    Err(mpsc::TrySendError::Full(_)) => ui.set_provider_status("A sensor refresh is already queued. Missing readings stay unavailable.".into()),
                    Err(mpsc::TrySendError::Disconnected(_)) => self.notice("The sensor worker is unavailable. Reopen the app to reconnect.", true),
                }
            }
            "running-processes" | "foreground-process" => {
                self.candidate_query.clear();
                self.candidate_page = 0;
                self.running_candidates = Some(vec![]);
                ui.set_candidate_source("Running processes · read-only Win32 snapshot".into());
                ui.set_modal(2);
                self.games();
                let weak = self.ui.clone();
                let only_foreground = command == "foreground-process";
                let foreground = crate::system_integration::last_external_foreground_pid();
                thread::spawn(move || {
                    let result = win::processes().map(|processes| {
                        let mut seen = std::collections::HashSet::new();
                        let mut games: Vec<_> = processes
                            .into_iter()
                            .filter(|(pid, name)| {
                                (!only_foreground || *pid == foreground)
                                    && *pid != 0
                                    && valid_process(name)
                                    && seen.insert(name.to_lowercase())
                            })
                            .take(1000)
                            .map(|(pid, name)| {
                                let path = crate::system_integration::process_path(pid);
                                Game {
                                    id: format!("running-{pid}"),
                                    name: name.clone(),
                                    processes: vec![name],
                                    source: path
                                        .map(|p| format!("Running PID {pid} | {}", p.display()))
                                        .unwrap_or_else(|| format!("Running PID {pid}")),
                                    ..Default::default()
                                }
                            })
                            .collect();
                        games.sort_by_key(|g| g.name.to_lowercase());
                        games
                    });
                    let _ = weak.upgrade_in_event_loop(move |_| {
                        CONTROL.with(|cell| {
                            if let Some(c) = cell.borrow().as_ref() {
                                let mut c = c.borrow_mut();
                                match result {
                                    Ok(games) => {
                                        c.running_candidates = Some(games);
                                        c.games();
                                    }
                                    Err(e) => c.notice(&e, true),
                                }
                            }
                        })
                    });
                });
            }
            "import-plan" | "export-csv" | "export-json" | "export-diagnostics" => {
                self.pick_for(command, None)
            }
            _ => self.notice(&format!("Unknown command: {command}"), true),
        }
    }
    fn browse_exes(&mut self) {
        if self.operation_busy {
            self.notice("Close the current file operation first.", true);
            return;
        }
        let Some(ui) = self.ui.upgrade() else { return };
        let owner = hwnd(&ui);
        let weak = self.ui.clone();
        self.operation_busy = true;
        thread::spawn(move || {
            let paths = win::pick_exes(owner);
            let _ = weak.upgrade_in_event_loop(move |_| {
                CONTROL.with(|cell| {
                    if let Some(c) = cell.borrow().as_ref() {
                        let mut c = c.borrow_mut();
                        c.operation_busy = false;
                        if !paths.is_empty() {
                            c.send(Command {
                                kind: IMPORT,
                                values: paths,
                                ..Default::default()
                            });
                        }
                    }
                })
            });
        });
    }
    /// The native picker runs outside a borrowed Controller; Windows pumps
    /// events while a dialog is open, including background IPC and tray events.
    fn pick_for(&mut self, purpose: &str, guid: Option<String>) {
        if self.operation_busy {
            self.notice("Close the current file operation first.", true);
            return;
        }
        let Some(ui) = self.ui.upgrade() else { return };
        let owner = hwnd(&ui);
        let weak = self.ui.clone();
        let purpose = purpose.to_string();
        self.operation_busy = true;
        let kind = match purpose.as_str() {
            "export-csv" => crate::system_integration::FileKind::Csv,
            "export-json" => crate::system_integration::FileKind::Json,
            "export-diagnostics" => crate::system_integration::FileKind::Json,
            _ => crate::system_integration::FileKind::PowerPlan,
        };
        thread::spawn(move || {
            let result =
                crate::system_integration::pick_file(owner, purpose != "import-plan", kind);
            let _ = weak.upgrade_in_event_loop(move |_| {
                CONTROL.with(|cell| {
                    if let Some(c) = cell.borrow().as_ref() {
                        let mut c = c.borrow_mut();
                        c.operation_busy = false;
                        match result {
                            Ok(Some(path)) => c.finish_pick(&purpose, guid, path),
                            Ok(None) => {}
                            Err(e) => c.notice(&e, true),
                        }
                    }
                })
            });
        });
    }
    fn finish_pick(&mut self, purpose: &str, guid: Option<String>, path: std::path::PathBuf) {
        if purpose == "export-plan" {
            if let Some(guid) = guid {
                self.begin_plan_job(crate::power_tools::Operation::Export(guid, path), false);
            }
            return;
        }
        if purpose == "import-plan" {
            self.begin_plan_job(crate::power_tools::Operation::Import(path), false);
            return;
        }
        self.pro.advance_export(
            &self.state,
            self.session_started.elapsed().as_millis() as u64,
            self.connected,
        );
        let export = crate::session_export::SessionExport::new(&self.state, &self.pro.session);
        let purpose = purpose.to_owned();
        let details = self
            .ui
            .upgrade()
            .map(|u| u.get_event_detail().to_string())
            .unwrap_or_default();
        let directory = data_dir();
        // File reads, serialization and disk writes stay off the UI thread.
        self.begin_job(move || {
            let data = match purpose.as_str() {
                "export-csv" => export.csv(),
                "export-json" => export.json()?,
                _ => export.diagnostic_json(&directory, &details)?,
            };
            std::fs::write(&path, data).map_err(|e| e.to_string())?;
            Ok(format!("Saved {}", path.display()))
        });
    }
    fn begin_plan_job(&mut self, operation: crate::power_tools::Operation, elevated: bool) {
        if self.operation_busy {
            self.notice("Another file operation is still running.", true);
            return;
        }
        self.operation_busy = true;
        self.elevated_retry = None;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_elevated_plan_retry(false);
        }
        self.notice(
            if elevated {
                "Waiting for Windows administrator approval…"
            } else {
                "Working on the selected power plan…"
            },
            false,
        );
        let weak = self.ui.clone();
        thread::spawn(move || {
            let result = operation.run(elevated);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                CONTROL.with(|cell| {
                    if let Some(control) = cell.borrow().as_ref() {
                        let mut control = control.borrow_mut();
                        control.operation_busy = false;
                        match result {
                            Ok(message) => {
                                control.simple(REFRESH);
                                control.notice(&message, false);
                            }
                            Err(error) => {
                                if !elevated && crate::power_tools::needs_elevation(&error) {
                                    control.elevated_retry = Some(operation);
                                    ui.set_elevated_plan_retry(true);
                                    control.notice("Windows denied this .pow operation (administrator approval required). Settings → Power → Retry with Windows approval opens a Windows UAC prompt for powercfg; approve or cancel there. The app itself stays unelevated.", true);
                                } else {
                                    control.notice(&error, true);
                                }
                            }
                        }
                    }
                });
            });
        });
    }
    fn fresh_sensor_sample(&self) -> Option<&crate::system_integration::SensorSnapshot> {
        self.sensor_received
            .filter(|at| {
                at.elapsed() < Duration::from_secs(12)
                    && self.sensor_generation
                        == self
                            .view
                            .sensor_generation
                            .load(std::sync::atomic::Ordering::SeqCst)
                    && self
                        .view
                        .sensor_visible
                        .load(std::sync::atomic::Ordering::SeqCst)
            })
            .and(self.sensor_sample.as_ref())
    }
    fn render_sensors(&self) {
        if !self.view.main_visible.get() {
            return;
        }
        let Some(ui) = self.ui.upgrade() else { return };
        let sample = self.fresh_sensor_sample();
        ui.set_cpu_temp(sample.and_then(|s| s.cpu_c).unwrap_or(-1.) as f32);
        ui.set_gpu_temp(sample.and_then(|s| s.gpu_c).unwrap_or(-1.) as f32);
        ui.set_cpu_package_watts(sample.and_then(|s| s.package_w).unwrap_or(-1.) as f32);
        ui.set_fan_rpm(sample.and_then(|s| s.fan_rpm).unwrap_or(-1.) as f32);
        ui.set_gpu_utilization(sample.and_then(|s| s.gpu_utilization).unwrap_or(-1.) as f32);
        ui.set_gpu_fan_percent(sample.and_then(|s| s.gpu_fan_percent).unwrap_or(-1.) as f32);
        ui.set_gpu_fan_rpm(sample.and_then(|s| s.gpu_fan_rpm).unwrap_or(-1.) as f32);
        ui.set_sensors_available(sample.is_some_and(|s| {
            s.cpu_c.is_some()
                || s.gpu_c.is_some()
                || s.package_w.is_some()
                || s.fan_rpm.is_some()
                || s.gpu_utilization.is_some()
                || s.gpu_fan_percent.is_some()
                || s.gpu_fan_rpm.is_some()
        }));
        if let Some(s) = sample {
            ui.set_cpu_sensor_provider(format!("{} · {}", s.cpu_provider, s.cpu_label).into());
            ui.set_gpu_sensor_provider(
                format!(
                    "{} · {} · utilization: {}",
                    s.gpu_provider, s.gpu_label, s.gpu_utilization_provider
                )
                .into(),
            );
            ui.set_power_sensor_provider(
                format!("{} · {}", s.power_provider, s.power_label).into(),
            );
            ui.set_fan_sensor_provider(
                if s.gpu_fan_rpm.is_some() || s.gpu_fan_percent.is_some() {
                    format!(
                        "{} · {} · reported fan speed",
                        s.gpu_fan_provider, s.gpu_label
                    )
                } else {
                    format!("{} · {}", s.fan_provider, s.fan_label)
                }
                .into(),
            );
            ui.set_provider_status(sensor_status(s).into());
        } else {
            ui.set_cpu_sensor_provider("Unavailable · awaiting fresh sample".into());
            ui.set_gpu_sensor_provider("Unavailable · awaiting fresh sample".into());
            ui.set_power_sensor_provider("Unavailable · awaiting fresh sample".into());
            ui.set_fan_sensor_provider("Unavailable · awaiting fresh sample".into());
            ui.set_provider_status("Waiting for a fresh hardware sensor sample…".into());
        }
    }
    fn update_hud(&mut self) {
        let Some(_ui) = self.ui.upgrade() else { return };
        let enabled = self.preferences.hud_enabled;
        if !enabled || self.state.suspended {
            self.hud_generation = self.hud_generation.wrapping_add(1);
            self.hud_ready = false;
            if let Some(setup) = self.hud_setup.take() {
                setup.abort();
            }
            self.view.hud_visible.set(false);
            // Drop the component and its native adapter/resources, rather than
            // keeping a second graphics context alive behind a hidden window.
            // Full-screen suspension preserves the saved HUD preference; the
            // next unsuspended engine snapshot recreates it automatically.
            if let Some(hud) = self.hud.take() {
                let _ = hud.hide();
                drop(hud);
            }
            self.view.publish();
            return;
        }
        if self.hud.is_none() {
            self.hud_generation = self.hud_generation.wrapping_add(1);
            self.hud_ready = false;
            self.hud = new_telemetry_hud();
        }
        let Some(hud) = &self.hud else {
            self.view.hud_visible.set(false);
            self.notice("The telemetry HUD could not be created.", true);
            self.view.publish();
            return;
        };
        hud.set_plan_name(self.state.active_name.clone().into());
        let values: Vec<_> = self
            .state
            .cores
            .iter()
            .filter(|c| c.load.is_finite() && (0.0..=100.0).contains(&c.load))
            .map(|c| c.load)
            .collect();
        let recent = self.state.cpu_sampled_at_ms > 0
            && now_ms().saturating_sub(self.state.cpu_sampled_at_ms) < 3000
            && !self.state.suspended;
        hud.set_cpu_text(
            if recent && !values.is_empty() {
                format!(
                    "CPU {:5.1}% · {} threads",
                    values.iter().sum::<f64>() / values.len() as f64,
                    values.len()
                )
            } else {
                "CPU — · awaiting sample".into()
            }
            .into(),
        );
        hud.set_sensor_text(
            format!(
                "CPU {} · GPU {}",
                if let Some(value) = self.fresh_sensor_sample().and_then(|s| s.cpu_c) {
                    format!("{value:.0}°C")
                } else {
                    "—".into()
                },
                if let Some(value) = self.fresh_sensor_sample().and_then(|s| s.gpu_c) {
                    format!("{value:.0}°C")
                } else {
                    "—".into()
                }
            )
            .into(),
        );
        hud.set_alpha(1.);
        if !hud.window().is_visible() {
            if let Err(e) = hud.show() {
                self.notice(&e.to_string(), true);
                return;
            }
        }
        if self.hud_ready {
            self.configure_hud();
            return;
        }
        if self.hud_setup.is_some() {
            return;
        }
        // Slint 1.18.1 show() can return before its Winit HWND is created.
        // Await the public readiness accessor rather than passing handle 0 to
        // Windows or polling. The original attribute hook already prevents
        // activation while this first native window is being constructed.
        let weak = hud.as_weak();
        let generation = self.hud_generation;
        match slint::spawn_local(async move {
            let Some(hud) = weak.upgrade() else { return };
            let ready = hud.window().winit_window().await;
            drop(hud);
            CONTROL.with(|cell| {
                if let Some(control) = cell.borrow().as_ref() {
                    let mut control = control.borrow_mut();
                    if !hud_setup_current(
                        generation,
                        control.hud_generation,
                        control.preferences.hud_enabled && !control.state.suspended,
                    ) {
                        return;
                    }
                    control.hud_setup.take();
                    match ready {
                        Ok(_) => {
                            control.hud_ready = true;
                            control.configure_hud();
                        }
                        Err(error) => {
                            control.notice(&format!("HUD window creation: {error}"), true)
                        }
                    }
                }
            });
        }) {
            Ok(setup) => self.hud_setup = Some(setup),
            Err(error) => self.notice(&format!("HUD initialization: {error}"), true),
        }
    }

    fn configure_hud(&mut self) {
        let Some(hud) = &self.hud else { return };
        let Some(ui) = self.ui.upgrade() else { return };
        // Keep these in winit's cached flags as well as the native helper. A
        // later show/hide or Slint property update must not turn hit testing or
        // a taskbar entry back on. Non-activation is set before HWND creation.
        let handle = hud
            .window()
            .with_winit_window(|window| -> Result<usize, String> {
                use winit::platform::windows::WindowExtWindows;
                window.set_skip_taskbar(true);
                window.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
                window
                    .set_cursor_hittest(false)
                    .map_err(|e| e.to_string())?;
                match window.window_handle().map_err(|e| e.to_string())?.as_raw() {
                    RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get() as usize),
                    _ => Err("HUD has no Windows native handle".into()),
                }
            });
        let handle = match handle {
            Some(Ok(handle)) => handle,
            Some(Err(error)) => {
                self.notice(&format!("HUD native setup: {error}"), true);
                return;
            }
            None => {
                self.hud_ready = false;
                return;
            }
        };
        if let Err(e) = crate::system_integration::configure_overlay(
            handle,
            hwnd(&ui),
            self.preferences.hud_position,
            (self.preferences.hud_opacity * 2.55).round() as u8,
        ) {
            self.notice(&e, true);
            return;
        }
        self.view.hud_visible.set(true);
        self.view.publish();
    }
}

fn install_pro(
    ui: &AppWindow,
    control: &Rc<RefCell<Controller>>,
    sensor_requests: mpsc::Receiver<()>,
    sensor_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    ui.set_power_tools_available(true);
    ui.set_automation_available(true);
    // The provider is compiled in; construction is deferred until enabled.
    ui.set_visual_overlay_available(true);
    ui.set_sensors_available(false);
    ui.set_provider_status("Checking local hardware sensor provider…".into());
    let c = control.clone();
    ui.on_pro_setting(move |key| c.borrow_mut().pro_setting(&key));
    let c = control.clone();
    ui.on_pro_command(move |key| c.borrow_mut().pro_command(&key));
    let c = control.clone();
    ui.on_power_tool(move |role, key| c.borrow_mut().power_tool(role, &key));
    let c = control.clone();
    ui.on_layout_mode(move |compact| {
        let mut c = c.borrow_mut();
        c.preferences.compact = compact;
        c.apply_appearance();
        c.save_preferences();
    });
    let c = control.clone();
    ui.on_graph_hover(move |fraction| {
        let c = c.borrow();
        if let Some(ui) = c.ui.upgrade() {
            ui.set_graph_tooltip(
                c.pro
                    .cpu
                    .topology_tooltip(now_ms(), fraction, c.topology.is_hybrid)
                    .into(),
            );
        }
    });
    let c = control.clone();
    ui.on_event_selected(move|index|{
        let c=c.borrow();if let Some(event)=usize::try_from(index).ok().and_then(|i|c.state.history.iter().rev().nth(i)){if let Some(ui)=c.ui.upgrade(){
            ui.set_event_detail(format!("{} · {} → {}\nCause: {}\nProcess: {}\nPID: {}\nEngine Windows user: {}\nPower API duration: {:.3} ms\nFrom GUID: {}\nTo GUID: {}",pro_view::local_clock(event.time_ms),if event.from_name.is_empty(){"Not captured"}else{&event.from_name},event.name,event.cause,if event.process.is_empty(){"Not captured"}else{&event.process},if event.pid==0{"Not captured".into()}else{event.pid.to_string()},if event.account.is_empty(){"Not captured"}else{&event.account},event.duration_ms,event.from_guid,event.to_guid).into());
        }}
    });
    let weak = ui.as_weak();
    match crate::system_integration::SystemIntegration::start(move |event| {
        let _ = weak.upgrade_in_event_loop(move |_| {
            CONTROL.with(|cell| {
                if let Some(c) = cell.borrow().as_ref() {
                    c.borrow_mut().system_event(event)
                }
            })
        });
    }) {
        Ok(integration) => {
            let result = integration.set_tray(true, false);
            ui.set_tray_available(result.is_ok());
            ui.set_hotkeys_available(true);
            control.borrow_mut().integration = Some(integration);
            control.borrow().register_hotkeys();
            if let Err(e) = result {
                control.borrow().notice(&e, true);
            }
        }
        Err(e) => {
            ui.set_tray_available(false);
            ui.set_hotkeys_available(false);
            control.borrow().notice(&e, true);
        }
    }
    let weak = ui.as_weak();
    let sensor_visible = control.borrow().view.sensor_visible.clone();
    let sensor_generation = control.borrow().view.sensor_generation.clone();
    thread::spawn(move || {
        let mut explicit = false;
        let mut reader = crate::sensors::SensorReader::new();
        while !sensor_stop.load(std::sync::atomic::Ordering::Relaxed) {
            if explicit || sensor_visible.load(std::sync::atomic::Ordering::Relaxed) {
                let generation = sensor_generation.load(std::sync::atomic::Ordering::SeqCst);
                let sample = reader.sample(explicit);
                let completed = Instant::now();
                let _ = weak.upgrade_in_event_loop(move |_| {
                    CONTROL.with(|cell| {
                        if let Some(c) = cell.borrow().as_ref() {
                            let mut c = c.borrow_mut();
                            // Sampling can finish after minimize/close. Retain
                            // no stale result and do not redraw a hidden main UI.
                            if sensor_result_current(
                                generation,
                                c.view
                                    .sensor_generation
                                    .load(std::sync::atomic::Ordering::SeqCst),
                                c.view
                                    .sensor_visible
                                    .load(std::sync::atomic::Ordering::SeqCst),
                                completed,
                                Instant::now(),
                            ) {
                                c.sensor_generation = generation;
                                c.sensor_received = Some(completed);
                                c.sensor_sample = Some(sample);
                                c.render_sensors();
                                c.update_hud();
                            }
                        }
                    });
                });
            } else {
                reader.suspend();
            }
            explicit = sensor_requests.recv_timeout(Duration::from_secs(2)).is_ok();
        }
    });
}
