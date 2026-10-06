// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Disposable integration driver, opt-in through --exercise [report] [--live].
//! It never disables another monitor or changes installed power-plan definitions.
use crate::{ipc, model::*, win};
use std::{
    fs,
    process::Child,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
struct Client {
    file: win::Pipe,
    rx: mpsc::Receiver<Snapshot>,
}
impl Client {
    fn new() -> Result<Self, String> {
        let mut file = win::connect_pipe()?;
        let dry = crate::runtime_scope::current().dry;
        let initial = ipc::begin_session(&mut file, dry, true, |_| Ok(()))?;
        let mut read = file.try_clone().map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        tx.send(initial).map_err(|e| e.to_string())?;
        thread::spawn(move || {
            while let Ok(s) = ipc::read::<Snapshot>(&mut read) {
                if crate::runtime_scope::verify_peer(dry, s.dry_run).is_err() || tx.send(s).is_err()
                {
                    break;
                }
            }
        });
        Ok(Self { file, rx })
    }
    fn manual(&mut self, role: &str) -> Result<(), String> {
        ipc::write(
            &mut self.file,
            &Command {
                kind: MANUAL_PLAN,
                text: role.into(),
                ..Default::default()
            },
        )
    }
    fn configure(&mut self, config: Config) -> Result<(), String> {
        ipc::write(
            &mut self.file,
            &Command {
                kind: CONFIGURE,
                config: Some(config),
                ..Default::default()
            },
        )
    }
    fn send(&mut self, kind: u32) -> Result<(), String> {
        ipc::write(
            &mut self.file,
            &Command {
                kind,
                ..Default::default()
            },
        )
    }
    fn wait(&self, label: &str, p: impl Fn(&Snapshot) -> bool) -> Result<Snapshot, String> {
        let start = Instant::now();
        let mut last = String::new();
        while start.elapsed() < Duration::from_secs(20) {
            if let Ok(s) = self.rx.recv_timeout(Duration::from_millis(100)) {
                last = s.status.clone();
                if p(&s) {
                    return Ok(s);
                }
            }
        }
        Err(format!("Timeout: {label}. Last state: {last}"))
    }
}
struct Cleanup {
    children: Vec<Child>,
    daemon: Option<Child>,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(d) = self.daemon.as_mut() {
            let _ = d.kill();
            let _ = d.wait();
        }
    }
}
pub fn run(report: &str, live: bool) -> Result<(), String> {
    let Some(dir) = std::env::var_os("NN6_TEST_DATA_DIR").map(std::path::PathBuf::from) else {
        return Err("NN6_TEST_DATA_DIR must name a disposable directory.".into());
    };
    if dir.exists() {
        return Err("Use a fresh test directory to preserve earlier results.".into());
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if win::processes()?
        .iter()
        .any(|(_, n)| defaults().names().iter().any(|v| v.eq_ignore_ascii_case(n)))
    {
        return Err("Close all real Siege processes before integration testing.".into());
    }
    if live && win::legacy(false)? {
        return Err(
            "The original monitor must be explicitly handed over before live testing.".into(),
        );
    }
    let mut config = defaults();
    // Plan switching is tested independently of the machine's current AC/DC
    // state. This setting belongs only to the fresh diagnostic configuration;
    // the user's real configuration and physical power source are untouched.
    config.battery_guard = Some(false);
    if !live {
        config.games.clear();
        config.add_names(&[
            "NN6_TestAlpha.exe".into(),
            "NN6_TestBeta.exe".into(),
            "NN6_TestGamma.exe".into(),
        ]);
    }
    let names = config.names();
    config.save(&dir)?;
    let original = win::active()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cleanup = Cleanup {
        children: vec![],
        daemon: None,
    };
    let mut results = vec![];
    let outcome = (|| -> Result<(), String> {
        let mut cmd = win::hidden_command(&exe.to_string_lossy());
        cmd.arg("--daemon");
        if !live {
            cmd.arg("--dry-run");
        }
        cleanup.daemon = Some(cmd.spawn().map_err(|e| e.to_string())?);
        let mut client = None;
        for _ in 0..150 {
            if let Ok(c) = Client::new() {
                client = Some(c);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let mut a = client.ok_or("Engine IPC unavailable")?;
        let s = a.wait("initial paused", |s| !s.monitoring && !s.observer)?;
        results.push(
            serde_json::json!({"test":"initial paused","passed":true,"active":s.active_guid}),
        );
        if live {
            for (role, target) in [
                ("gaming", GAMING),
                ("default", BALANCED),
                ("auto", BALANCED),
            ] {
                a.manual(role)?;
                let state = a.wait("paused manual profile outcome", |s| {
                    !s.monitoring
                        && !s.error
                        && s.manual_plan == if role == "auto" { "" } else { role }
                        && s.active_guid.eq_ignore_ascii_case(target)
                })?;
                let actual = win::active()?;
                if !actual.eq_ignore_ascii_case(target) {
                    return Err(format!(
                        "Paused manual {role} reported {target}, but Windows has {actual}"
                    ));
                }
                results.push(serde_json::json!({
                    "test":format!("paused MANUAL_PLAN {role}"), "passed":true,
                    "active":actual, "monitoring":state.monitoring,
                    "manual_intent":state.manual_plan
                }));
            }
        }
        a.send(TOGGLE)?;
        a.wait("event listeners ready", |s| s.monitoring && s.ready)?;
        results.push(serde_json::json!({"test":"WMI start/stop subscriptions","passed":true}));
        for name in names.clone() {
            let path = dir.join(&name);
            fs::copy(&exe, &path).map_err(|e| e.to_string())?;
            let t = Instant::now();
            cleanup.children.push(
                win::hidden_command(&path.to_string_lossy())
                    .arg("--sleeper")
                    .spawn()
                    .map_err(|e| e.to_string())?,
            );
            a.wait("game start", |s| {
                s.running.iter().any(|n| n.eq_ignore_ascii_case(&name))
                    && (!live || s.active_guid == GAMING)
            })?;
            let start_ms = t.elapsed().as_secs_f64() * 1000.;
            let t = Instant::now();
            let child = cleanup.children.last_mut().unwrap();
            child.kill().map_err(|e| e.to_string())?;
            child.wait().map_err(|e| e.to_string())?;
            a.wait("game exit", |s| {
                s.running.is_empty() && (!live || s.active_guid == BALANCED)
            })?;
            results.push(serde_json::json!({"test":name,"passed":true,"start_to_observed_ms":start_ms,"kill_to_observed_ms":t.elapsed().as_secs_f64()*1000.}));
        }
        // Overlapping games: only the last exit returns to the default plan.
        for name in [&names[0], &names[1]] {
            cleanup.children.push(
                win::hidden_command(&dir.join(name).to_string_lossy())
                    .arg("--sleeper")
                    .spawn()
                    .map_err(|e| e.to_string())?,
            );
        }
        a.wait("two games", |s| s.running.len() == 2)?;
        let n = cleanup.children.len();
        cleanup.children[n - 2].kill().map_err(|e| e.to_string())?;
        a.wait("one remains", |s| {
            s.running.len() == 1 && (!live || s.active_guid == GAMING)
        })?;
        cleanup.children[n - 1].kill().map_err(|e| e.to_string())?;
        a.wait("both closed", |s| {
            s.running.is_empty() && (!live || s.active_guid == BALANCED)
        })?;
        results.push(
            serde_json::json!({"test":"overlap retains gaming until last exit","passed":true}),
        );
        if live {
            // A real per-game override must win over global Gaming. The
            // disposable process stays alive while settings return to normal,
            // proving the configured override is removed rather than merely
            // observing the Default plan after the process has exited.
            let mut overridden = config.clone();
            overridden.games[0].power_plan = BALANCED.into();
            a.configure(overridden.clone())?;
            a.wait("per-game override configured", |s| {
                s.monitoring && s.ready && !s.error && s.config.as_ref() == Some(&overridden)
            })?;
            cleanup.children.push(
                win::hidden_command(&dir.join(&names[0]).to_string_lossy())
                    .arg("--sleeper")
                    .spawn()
                    .map_err(|e| e.to_string())?,
            );
            a.wait("running fixture uses Balanced override", |s| {
                s.running.iter().any(|n| n.eq_ignore_ascii_case(&names[0]))
                    && s.active_guid.eq_ignore_ascii_case(BALANCED)
                    && !s.error
            })?;
            if !win::active()?.eq_ignore_ascii_case(BALANCED) {
                return Err("Windows did not retain the per-game Balanced override".into());
            }
            a.configure(config.clone())?;
            a.wait("live fixture resumes global Gaming", |s| {
                s.monitoring
                    && s.ready
                    && !s.error
                    && s.config.as_ref() == Some(&config)
                    && s.running.iter().any(|n| n.eq_ignore_ascii_case(&names[0]))
                    && s.active_guid.eq_ignore_ascii_case(GAMING)
            })?;
            if !win::active()?.eq_ignore_ascii_case(GAMING) || Config::load(&dir)? != config {
                return Err("Global Gaming or saved fixture settings were not restored".into());
            }
            let child = cleanup.children.last_mut().unwrap();
            child.kill().map_err(|e| e.to_string())?;
            child.wait().map_err(|e| e.to_string())?;
            a.wait("override fixture exited", |s| {
                s.running.is_empty() && s.active_guid.eq_ignore_ascii_case(BALANCED)
            })?;
            if !win::active()?.eq_ignore_ascii_case(BALANCED) {
                return Err("Windows did not return to Default after the override fixture".into());
            }
            results.push(serde_json::json!({
                "test":"per-game Balanced override and restored global Gaming while process alive",
                "passed":true, "fixture":names[0], "saved_config_restored":true
            }));
        }
        let mut b = Client::new()?;
        b.wait("observer", |s| s.observer)?;
        b.send(CLAIM)?;
        a.wait("old window retires", |s| s.retired)?;
        b.wait("new owner", |s| !s.observer && s.monitoring && s.ready)?;
        results.push(serde_json::json!({"test":"named pipe ownership transfer retains listener","passed":true}));
        // A missing GUID is simulated in app settings. Never delete an installed plan.
        let mut missing = config.clone();
        missing.gaming = "00000000-1111-2222-3333-444444444444".into();
        b.configure(missing)?;
        b.wait("missing plan prompt", |s| {
            s.error
                && s.monitoring
                && s.ready
                && s.status.contains("missing")
                && s.config.as_ref() == Some(&config)
        })?;
        if Config::load(&dir)? != config {
            return Err("Rejected missing-plan configuration replaced valid saved settings".into());
        }
        b.configure(config.clone())?;
        b.wait("settings recovered", |s| {
            !s.error && s.monitoring && s.ready && s.config.as_ref() == Some(&config)
        })?;
        results.push(serde_json::json!({"test":"missing plan rejected while valid monitoring and saved configuration remain intact","passed":true}));
        if !live {
            let mut edited = config.clone();
            edited.gaming = BALANCED.into();
            edited.default_plan = GAMING.into(); // Both roles must stay distinct.
            ipc::write(
                &mut b.file,
                &Command {
                    kind: CONFIGURE,
                    request_id: 801,
                    config: Some(edited.clone()),
                    ..Default::default()
                },
            )?;
            b.wait("correlated plan edit resumed", |s| {
                s.command_id == 801
                    && s.command_error.is_empty()
                    && s.monitoring
                    && s.ready
                    && s.config.as_ref() == Some(&edited)
            })?;
            if Config::load(&dir)? != edited {
                return Err("Plan edit was not saved".into());
            }
            ipc::write(
                &mut b.file,
                &Command {
                    kind: CONFIGURE,
                    request_id: 802,
                    config: Some(config.clone()),
                    ..Default::default()
                },
            )?;
            b.wait("correlated original config resumed", |s| {
                s.command_id == 802
                    && s.command_error.is_empty()
                    && s.monitoring
                    && s.ready
                    && s.config.as_ref() == Some(&config)
            })?;
            // Handover deliberately denies in preview; it must return the
            // matching error receipt without disabling the installed task.
            ipc::write(
                &mut b.file,
                &Command {
                    kind: LEGACY,
                    request_id: 803,
                    ..Default::default()
                },
            )?;
            b.wait("correlated preview handover denied", |s| {
                s.command_id == 803
                    && s.command_error.contains("disabled in preview")
                    && s.monitoring
                    && s.config.as_ref() == Some(&config)
            })?;
            // The observer must not inherit this owner's receipt.
            a.send(VIEW)?;
            a.wait("receipt isolated from observer", |s| {
                s.observer && s.command_id == 0
            })?;
            results.push(serde_json::json!({"test":"correlated live-config transaction, preview handover denial and receipt isolation", "passed":true, "actual_power_writes":false}));
        }
        // Correlated receipts distinguish command completion from incidental
        // readiness/telemetry broadcasts. These requests only target this
        // disposable engine; the production GUI/monitor remains untouched.
        let t = Instant::now();
        for request_id in 1001..=1050 {
            ipc::write(
                &mut b.file,
                &Command {
                    kind: TOGGLE,
                    request_id,
                    ..Default::default()
                },
            )?;
        }
        let queued_ms = t.elapsed().as_secs_f64() * 1000.;
        if queued_ms >= 5000. {
            return Err(format!("50 commands took {queued_ms:.1} ms to queue"));
        }
        let mut receipts = std::collections::BTreeSet::new();
        let start = Instant::now();
        while receipts.len() < 50 && start.elapsed() < Duration::from_secs(60) {
            if let Ok(s) = b.rx.recv_timeout(Duration::from_millis(100)) {
                if (1001..=1050).contains(&s.command_id) {
                    if !s.command_error.is_empty() {
                        return Err(format!(
                            "Toggle {} failed: {}",
                            s.command_id, s.command_error
                        ));
                    }
                    receipts.insert(s.command_id);
                }
            }
        }
        if receipts.len() != 50 {
            return Err(format!(
                "Only {}/50 correlated toggle receipts",
                receipts.len()
            ));
        }
        // Explicit status command guarantees a post-burst snapshot even if the
        // final Ready broadcast was already consumed together with receipt 1050.
        b.send(ENSURE_MONITORING)?;
        b.wait("rapid toggles paused", |s| !s.monitoring)?;
        b.send(TOGGLE)?;
        b.wait("rapid toggles settled", |s| s.monitoring && s.ready)?;
        results.push(serde_json::json!({"test":"50 rapid correlated toggles and clean listener restart","passed":true,"queue_ms":queued_ms,"settle_ms":t.elapsed().as_secs_f64()*1000.,"receipts":receipts.len()}));
        if live {
            if !win::startup(None)? {
                b.send(STARTUP)?;
                b.wait("startup installed", |s| s.startup)?;
                let registered = win::startup(None)?;
                b.send(STARTUP)?;
                b.wait("startup removed", |s| !s.startup)?;
                if !registered || win::startup(None)? {
                    return Err("Startup registry state did not match UI".into());
                }
                results.push(serde_json::json!({"test":"startup copy and registry synchronization","passed":true}));
            }
            let mut scheduled = config.clone();
            scheduled.games[0].priority = 0x8000;
            scheduled.games[0].affinity = "performance".into();
            ipc::write(
                &mut b.file,
                &Command {
                    kind: CONFIGURE,
                    config: Some(scheduled),
                    ..Default::default()
                },
            )?;
            b.wait("inline edit resumed", |s| {
                s.monitoring
                    && s.ready
                    && s.config
                        .as_ref()
                        .is_some_and(|c| c.games[0].priority == 0x8000)
            })?;
            let p = win::hidden_command(&dir.join("RainbowSix.exe").to_string_lossy())
                .arg("--sleeper")
                .spawn()
                .map_err(|e| e.to_string())?;
            let pid = p.id();
            let before = win::schedule(pid, 0, 0)?;
            cleanup.children.push(p);
            b.wait("gaming before pause", |s| {
                s.active_guid == GAMING && !s.running.is_empty()
            })?;
            let applied = win::schedule(pid, 0, 0)?;
            if applied.priority != 0x8000 {
                return Err("Disposable process priority did not change".into());
            }
            let previous_overlay = win::overlay().ok();
            if let Some(previous) = &previous_overlay {
                let mut overlay_config = config.clone();
                overlay_config.games[0].priority = 0x8000;
                overlay_config.games[0].affinity = "performance".into();
                overlay_config.overlay = true;
                ipc::write(
                    &mut b.file,
                    &Command {
                        kind: CONFIGURE,
                        config: Some(overlay_config),
                        ..Default::default()
                    },
                )?;
                let state = b.wait("optional overlay outcome", |s| {
                    s.ready && s.config.as_ref().is_some_and(|c| c.overlay)
                })?;
                results.push(serde_json::json!({"test":"optional overlay request","previous":previous,"actual":win::overlay().ok(),"activated":!state.error&&win::overlay().is_ok_and(|v|v.eq_ignore_ascii_case(BEST_OVERLAY)),"status":state.status}));
            }
            b.send(SHUTDOWN)?;
            b.wait("scheduling restored", |s| {
                !s.monitoring && s.active_guid == BALANCED
            })?;
            let after = win::schedule(pid, 0, 0)?;
            if before.priority != after.priority || before.affinity != after.affinity {
                return Err("Original process scheduling was not restored".into());
            }
            if let Some(previous) = previous_overlay {
                if win::overlay()? != previous {
                    return Err("Original power overlay was not restored".into());
                }
                results.push(
                    serde_json::json!({"test":"previous power overlay restored","passed":true}),
                );
            }
            results.push(serde_json::json!({"test":"priority and affinity applied and restored","passed":true}));
            b.send(TOGGLE)?;
            b.wait("gaming before engine crash", |s| {
                s.active_guid == GAMING && s.ready
            })?;
            let t = Instant::now();
            cleanup
                .daemon
                .as_mut()
                .unwrap()
                .kill()
                .map_err(|e| e.to_string())?;
            while t.elapsed() < Duration::from_secs(15) && win::active()? != BALANCED {
                thread::sleep(Duration::from_millis(20));
            }
            if win::active()? != BALANCED {
                return Err("Supervisor did not restore Balanced".into());
            }
            results.push(serde_json::json!({"test":"supervisor after forced engine exit","passed":true,"restore_ms":t.elapsed().as_secs_f64()*1000.}));
        } else {
            b.send(SHUTDOWN)?;
            b.wait("paused at end", |s| !s.monitoring)?;
        }
        Ok(())
    })();
    drop(cleanup);
    if live {
        let _ = crate::power::select(&mut crate::power::Windows, &original);
    }
    let error = outcome.as_ref().err().cloned();
    atomic_json(
        std::path::Path::new(report),
        &serde_json::json!({"live":live,"results":results,"error":error,"original_plan_restored":win::active()?==original,"battery_test_scope":"This fresh isolated switching fixture disables its battery guard. AC/DC/unknown policy decisions are covered by pure automation unit tests; no physical power-source change or live battery transition is claimed."}),
    )?;
    outcome
}
