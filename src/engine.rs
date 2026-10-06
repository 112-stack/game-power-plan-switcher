// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! One serial state machine owns all power writes. WMI callbacks only enqueue.
use crate::{automation, discovery, ipc, model::*, win};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};
static EVENTS: OnceLock<Mutex<Sender<Event>>> = OnceLock::new();
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[derive(Clone)]
pub enum Event {
    Connect(u64, Sender<Snapshot>),
    Client(u64, Command),
    Gone(u64),
    Native(u32, u32, String, u64),
    Imported(Result<Vec<Game>, String>),
    Scanned(Result<Vec<Game>, String>),
}
extern "C" fn native_callback(kind: u32, pid: u32, name: *const u16, time: u64) {
    if name.is_null() {
        return;
    }
    let mut count = 0;
    unsafe {
        while count < 1024 && *name.add(count) != 0 {
            count += 1
        }
    }
    let _ = time;
    let name = unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(name, count)) };
    if let Some(tx) = EVENTS.get() {
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(Event::Native(kind, pid, name, time));
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Journal {
    pid: u32,
    default_plan: String,
    overlay: Option<String>,
    schedules: Vec<win::Schedule>,
    dirty: bool,
}
#[derive(Default)]
struct PolicyRetry {
    pending: bool,
}
impl PolicyRetry {
    fn needed(&self, changed: bool) -> bool {
        changed || self.pending
    }
    fn completed(&mut self, enabled: bool, result: &Result<(), String>) {
        self.pending = enabled && result.is_err();
    }
}
fn require_restored(dirty: bool, dry: bool) -> Result<(), String> {
    if dirty && !dry {
        Err(
            "Settings were not changed: previous power or scheduling restoration is still pending."
                .into(),
        )
    } else {
        Ok(())
    }
}
/// Per-client receipts survive unrelated telemetry broadcasts. A receipt must
/// never acknowledge another GUI's command, including after a control claim.
#[derive(Default)]
struct CommandReceipts(HashMap<u64, (u64, String)>);
impl CommandReceipts {
    fn complete(&mut self, client: u64, request: u64, result: &Result<(), String>) {
        if request != 0 {
            self.0.insert(
                client,
                (request, result.as_ref().err().cloned().unwrap_or_default()),
            );
        }
    }
    fn apply(&self, client: u64, snapshot: &mut Snapshot) {
        let receipt = self.0.get(&client);
        snapshot.command_id = receipt.map_or(0, |r| r.0);
        snapshot.command_error = receipt.map_or_else(String::new, |r| r.1.clone());
    }
    fn remove(&mut self, client: u64) {
        self.0.remove(&client);
    }
}
pub struct Engine {
    pub state: Snapshot,
    dir: PathBuf,
    pids: HashMap<u32, String>,
    clients: HashMap<u64, Sender<Snapshot>>,
    receipts: CommandReceipts,
    owner: u64,
    lock: Option<File>,
    listener: Option<thread::JoinHandle<u32>>,
    journal: Journal,
    overview: bool,
    foreground: u32,
    telemetry_at: Instant,
    tx: Sender<Event>,
    started: bool,
    gui_pids: HashMap<u64, u32>,
    generation: u64,
    foreground_full: bool,
    waits: HashMap<u32, win::ProcessWait>,
    ring: Option<win::TelemetryRing>,
    policy_at: Instant,
    policy_minute: u16,
    policy_retry: PolicyRetry,
    account: String,
}
fn log(dir: &Path, text: &str) {
    let _ = fs::create_dir_all(dir);
    let path = dir.join("native.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > 2_000_000) {
        let _ = fs::rename(&path, dir.join("native.log.1"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} {text}", now_ms());
    }
}
fn telemetry_snapshot(state: &Snapshot) -> Snapshot {
    Snapshot {
        cores: state.cores.clone(),
        history: state.history.clone(),
        // The ring is independently mode-validated before the GUI sends HELLO.
        dry_run: state.dry_run,
        ..Default::default()
    }
}
fn validate_config_plans(config: &Config, plans: &[Plan]) -> Result<(), String> {
    config.validate()?;
    for guid in std::iter::once(&config.gaming)
        .chain(std::iter::once(&config.default_plan))
        .chain(
            config
                .games
                .iter()
                .filter_map(|g| (!g.power_plan.is_empty()).then_some(&g.power_plan)),
        )
    {
        if !plans.iter().any(|p| p.guid.eq_ignore_ascii_case(guid)) {
            return Err(format!(
                "Configured power plan is missing: {guid}. Select an installed plan in Settings."
            ));
        }
    }
    Ok(())
}
impl Engine {
    fn new(dir: PathBuf, dry: bool, tx: Sender<Event>) -> Self {
        let loaded = Config::load(&dir);
        let (config, error) = match loaded {
            Ok(c) => (c, String::new()),
            Err(e) => (defaults(), e),
        };
        let plans = win::plans().unwrap_or_default();
        let active = win::active().unwrap_or_default();
        let name = plans
            .iter()
            .find(|p| p.guid == active)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Unavailable".into());
        let legacy = win::legacy(false).unwrap_or(true);
        let startup = win::startup(None).unwrap_or(false);
        Self {
            state: Snapshot {
                config: Some(config),
                plans,
                active_guid: active,
                active_name: name,
                legacy,
                startup,
                dry_run: dry,
                cores: win::cores(),
                status: if error.is_empty() {
                    "Ready. Monitoring is paused.".into()
                } else {
                    error.clone()
                },
                error: !error.is_empty(),
                telemetry: "Telemetry paused".into(),
                power_source: automation::power_source(),
                ..Default::default()
            },
            dir,
            pids: HashMap::new(),
            clients: HashMap::new(),
            receipts: CommandReceipts::default(),
            owner: 0,
            lock: None,
            listener: None,
            journal: Journal::default(),
            overview: false,
            foreground: 0,
            telemetry_at: Instant::now(),
            tx,
            started: false,
            gui_pids: HashMap::new(),
            generation: 0,
            foreground_full: false,
            waits: HashMap::new(),
            ring: win::TelemetryRing::open(true).ok(),
            policy_at: Instant::now(),
            policy_minute: automation::local_minute(),
            policy_retry: PolicyRetry::default(),
            account: automation::account(),
        }
    }
    fn config(&self) -> &Config {
        self.state.config.as_ref().unwrap()
    }
    fn status(&mut self, s: impl Into<String>, error: bool) {
        self.state.status = s.into();
        self.state.error = error;
        if error {
            log(&self.dir, &self.state.status);
        }
    }
    fn telemetry_enabled(&self) -> bool {
        // VIEW describes the foreground, visible Overview, not power-monitor
        // state. Reading PDH never acquires power ownership or changes a plan.
        self.overview && !self.state.suspended && !self.clients.is_empty()
    }
    fn reset_telemetry(&mut self, visible: bool) {
        win::reset_cpu_load();
        for core in &mut self.state.cores {
            core.load = -1.;
        }
        self.state.cpu_sampled_at_ms = 0;
        self.state.telemetry = if visible {
            "Windows CPU · warming up a fresh 1-second sample…"
        } else {
            "CPU sampling paused · return to the visible Overview"
        }
        .into();
        // Seed immediately on resume; the second observation arrives in 1s.
        self.telemetry_at = Instant::now() - Duration::from_secs(1);
    }
    fn sample_telemetry(&mut self) {
        let mut cores = win::cores();
        self.state.cpu_sampled_at_ms = 0;
        match win::cpu_load() {
            Ok(Some(sample)) => {
                win::apply_cpu_sample(&mut cores, &sample.values);
                let valid: Vec<_> = cores.iter().filter(|c| c.load >= 0.).collect();
                if valid.is_empty() {
                    self.state.telemetry =
                        "Windows CPU · no matching per-core data available".into();
                } else {
                    let avg = valid.iter().map(|c| c.load).sum::<f64>() / valid.len() as f64;
                    let active = valid.iter().filter(|c| c.load >= 1.).count();
                    self.state.cpu_sampled_at_ms = now_ms();
                    self.state.telemetry = format!(
                        "Live Windows · {:.1}s sample · {avg:.0}% avg · {active}/{} ≥1%{}",
                        sample.interval_ms as f64 / 1000.,
                        cores.len(),
                        if valid.len() < cores.len() {
                            " · partial data"
                        } else {
                            ""
                        }
                    );
                }
            }
            Ok(None) => {
                self.state.telemetry = "Windows CPU · warming up a fresh 1-second sample…".into()
            }
            Err(e) => self.state.telemetry = e,
        }
        self.state.cores = cores;
        self.telemetry_at = Instant::now();
    }
    fn broadcast(&self) {
        let seq = self.ring.as_ref().and_then(|ring| {
            ring.write(&telemetry_snapshot(&self.state).encode_to_vec())
                .ok()
        });
        for (id, tx) in &self.clients {
            let mut s = self.state.clone();
            if let Some(seq) = seq {
                s.telemetry_sequence = seq;
                s.cores.clear();
                s.history.clear();
            }
            s.observer = *id != self.owner;
            self.receipts.apply(*id, &mut s);
            let _ = tx.send(s);
        }
    }
    fn journal(&self) -> Result<(), String> {
        atomic_json(&self.dir.join("recovery.json"), &self.journal)
    }
    fn update_running(&mut self) {
        let mut names: Vec<_> = self.pids.values().cloned().collect();
        names.sort();
        names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        self.state.running = names;
        self.waits.retain(|pid, _| self.pids.contains_key(pid));
        if self.state.monitoring {
            for pid in self.pids.keys() {
                if !self.waits.contains_key(pid) {
                    if let Some(w) = win::watch_exit(*pid) {
                        self.waits.insert(*pid, w);
                    }
                }
            }
        }
        self.state.suspended = self.foreground_full && self.pids.contains_key(&self.foreground);
    }
    fn snapshot_processes(&mut self) -> Result<(), String> {
        let watched: HashSet<_> = self
            .config()
            .names()
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect();
        self.pids = win::processes()?
            .into_iter()
            .filter(|(_, n)| watched.contains(&n.to_lowercase()))
            .collect();
        self.update_running();
        Ok(())
    }
    fn validate_installed_plans(&mut self) -> Result<(), String> {
        self.state.plans = win::plans()?;
        validate_config_plans(self.config(), &self.state.plans)
    }
    /// The same monitor lock and recovery journal protect automation and manual
    /// tray/hotkey selections. A paused manual selection still has a guardian.
    fn acquire_ownership(&mut self) -> Result<(), String> {
        if self.state.dry_run || self.lock.is_some() {
            return Ok(());
        }
        self.state.legacy = win::legacy(false)?;
        automation::manual_request(true, self.state.legacy, false, "default")?;
        self.lock = Some(win::lock(&win::monitor_path())?);
        if let Ok(bytes) = fs::read(self.dir.join("recovery.json")) {
            if let Ok(previous) = serde_json::from_slice::<Journal>(&bytes) {
                if previous.dirty {
                    self.journal = previous;
                    self.restore();
                    if self.journal.dirty {
                        self.lock = None;
                        return Err("Previous state restoration is incomplete. See the activity log before restarting monitoring.".into());
                    }
                }
            }
        }
        self.journal = Journal {
            pid: std::process::id(),
            default_plan: self.config().default_plan.clone(),
            ..Default::default()
        };
        Ok(())
    }
    fn begin(&mut self) -> Result<(), String> {
        if self.state.monitoring {
            return Ok(());
        }
        if self.state.legacy && !self.state.dry_run {
            return Err("The previous monitor is active. Use the handover action first.".into());
        }
        self.validate_installed_plans()?;
        if self.config().games.is_empty() {
            return Err("Add a game before monitoring.".into());
        }
        self.acquire_ownership()?;
        self.state.manual_plan.clear();
        self.generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let generation = self.generation;
        let tx = self.tx.clone();
        self.state.monitoring = true;
        self.state.ready = false;
        unsafe { win::nn6_prepare_events() };
        let handle = thread::spawn(move || {
            let result = unsafe { win::nn6_events(native_callback, generation) };
            if result != 0 {
                let _ = tx.send(Event::Native(5, result, String::new(), generation));
            }
            result
        });
        self.listener = Some(handle);
        self.status("Connecting to Windows process events...", false);
        Ok(())
    }
    fn restore(&mut self) {
        if self.state.dry_run || self.lock.is_none() {
            return;
        }
        let mut remaining = Vec::new();
        for s in std::mem::take(&mut self.journal.schedules) {
            if let Err(e) = win::restore_schedule(&s) {
                self.status(format!("PID {}: {e}", s.pid), true);
                remaining.push(s);
            }
        }
        self.journal.schedules = remaining;
        if let Some(overlay) = self.journal.overlay.clone() {
            if let Err(e) = self.overlay_to(&overlay, "Restore previous overlay") {
                self.status(e, true);
            } else {
                self.journal.overlay = None;
            }
        }
        if self.journal.dirty {
            let default = self.journal.default_plan.clone();
            if let Err(e) = self.switch_to(&default, "Restore default") {
                self.status(e, true);
                let _ = self.journal();
                return;
            }
        }
        self.journal.dirty = self.journal.overlay.is_some() || !self.journal.schedules.is_empty();
        if let Err(e) = self.journal() {
            self.status(e, true);
        }
    }
    fn pause(&mut self) {
        self.policy_retry.pending = false;
        self.state.manual_plan.clear();
        self.state.battery_guard_active = false;
        self.state.automation_status = "Automatic rules paused".into();
        self.waits.clear();
        self.state.monitoring = false;
        self.state.ready = false;
        if let Some(h) = self.listener.take() {
            unsafe { win::nn6_stop_events() };
            let _ = h.join();
        }
        self.generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.restore();
        self.lock = None;
        self.state.suspended = false;
        if !self.state.error {
            self.status(
                "Paused. Default plan restored; process listeners stopped.",
                false,
            );
        }
    }
    fn switch_to(&mut self, target: &str, cause: &str) -> Result<(), String> {
        self.switch_to_context(target, cause, "", 0)
    }
    fn switch_to_context(
        &mut self,
        target: &str,
        cause: &str,
        process: &str,
        pid: u32,
    ) -> Result<(), String> {
        let current = win::active()?;
        if current.eq_ignore_ascii_case(target) {
            self.state.active_name = self
                .state
                .plans
                .iter()
                .find(|p| p.guid.eq_ignore_ascii_case(&current))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| current.clone());
            self.state.active_guid = current;
            return Ok(());
        }
        if self.state.dry_run {
            self.status(format!("Preview: would select {target}"), false);
            return Ok(());
        }
        let timer = Instant::now();
        if !crate::power::select(&mut crate::power::Windows, target)? {
            return Ok(());
        }
        let active = win::active()?;
        self.state.active_guid = active.clone();
        self.state.active_name = self
            .state
            .plans
            .iter()
            .find(|p| p.guid.eq_ignore_ascii_case(&active))
            .map(|p| p.name.clone())
            .unwrap_or(active);
        let event = Transition {
            time_ms: now_ms(),
            name: self.state.active_name.clone(),
            duration_ms: timer.elapsed().as_secs_f64() * 1000.,
            cause: cause.into(),
            gaming: !target.eq_ignore_ascii_case(&self.config().default_plan),
            from_name: self
                .state
                .plans
                .iter()
                .find(|p| p.guid.eq_ignore_ascii_case(&current))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| current.clone()),
            from_guid: current.clone(),
            to_guid: self.state.active_guid.clone(),
            process: process.into(),
            pid,
            account: self.account.clone(),
        };
        log(
            &self.dir,
            &format!(
                "SWITCH {} -> {} | {:.3} ms | {}{} | PID {} | Account {}",
                event.from_name,
                event.name,
                event.duration_ms,
                cause,
                if process.is_empty() {
                    String::new()
                } else {
                    format!(" via {process}")
                },
                event.pid,
                event.account
            ),
        );
        let logdir = self.dir.parent().unwrap().join("GamePowerPlan");
        let _ = fs::create_dir_all(&logdir);
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(logdir.join("switch.log"))
        {
            let _ = writeln!(
                f,
                "{} [native] {} -> {} ({:.3} ms)",
                event.time_ms, current, target, event.duration_ms
            );
        }
        self.state.history.push(event);
        if self.state.history.len() > 64 {
            self.state.history.remove(0);
        }
        Ok(())
    }
    fn apply(&mut self, cause: &str) -> Result<(), String> {
        self.apply_context(cause, "", 0)
    }
    fn apply_context(&mut self, cause: &str, process: &str, pid: u32) -> Result<(), String> {
        let result = self.apply_policy_context(cause, process, pid);
        // AC/DC notifications and the periodic check both enter this path.
        // Remember a failure after observing the new source, so an unchanged
        // source still retries on the next bounded policy tick. A successful
        // apply clears the retry and avoids needless five-second plan work.
        let enabled = self.policy_enabled();
        self.policy_retry.completed(enabled, &result);
        result
    }
    fn apply_policy_context(&mut self, cause: &str, process: &str, pid: u32) -> Result<(), String> {
        if !(self.state.monitoring && self.state.ready) && self.state.manual_plan.is_empty() {
            return Ok(());
        }
        let config = self.config().clone();
        self.state.power_source = automation::power_source();
        self.policy_minute = automation::local_minute();
        let decision = automation::decide(
            &config,
            &self.pids,
            self.foreground,
            self.state.power_source,
            self.policy_minute,
            &self.state.manual_plan,
        );
        self.state.battery_guard_active = decision.guard_active;
        self.state.automation_status = decision.explanation.clone();
        let target = &decision.target;
        if !self.state.dry_run {
            self.journal.dirty = true;
            self.journal()?;
        }
        self.switch_to_context(target, cause, process, pid)?;
        if !self.state.dry_run {
            if self.state.monitoring
                && self.state.ready
                && decision.gaming
                && !self.pids.is_empty()
                && config.overlay
                && self.journal.overlay.is_none()
            {
                let previous = win::overlay()?;
                self.journal.overlay = Some(previous);
                self.journal()?;
                self.overlay_to(BEST_OVERLAY, "Best performance overlay")?;
            }
            if !decision.gaming || self.pids.is_empty() {
                if let Some(previous) = self.journal.overlay.clone() {
                    self.overlay_to(&previous, "Restore previous overlay")?;
                    self.journal.overlay = None;
                }
            }
            self.journal
                .schedules
                .retain(|s| self.pids.contains_key(&s.pid));
            for (pid, name) in self.pids.clone() {
                if !self.state.monitoring || !self.state.ready || !config.auto_affinity_enabled() {
                    break;
                }
                if self.journal.schedules.iter().any(|s| s.pid == pid) {
                    continue;
                }
                if let Some(game) = config
                    .games
                    .iter()
                    .find(|g| g.processes.iter().any(|p| p.eq_ignore_ascii_case(&name)))
                {
                    let mask = affinity(&game.affinity, &self.state.cores)?;
                    if game.priority != 0 || mask != 0 {
                        let before = win::schedule(pid, 0, 0)?;
                        self.journal.schedules.push(before);
                        self.journal()?;
                        win::apply_schedule(&before, game.priority, mask)?;
                    }
                }
            }
            self.journal()?;
        }
        self.status(
            if self.state.dry_run {
                format!("Preview · {} · no power changes", self.state.backend)
            } else {
                format!("{} · {}", self.state.backend, self.state.automation_status)
            },
            false,
        );
        Ok(())
    }
    fn overlay_to(&mut self, target: &str, cause: &str) -> Result<(), String> {
        if win::overlay()?.eq_ignore_ascii_case(target) {
            return Ok(());
        }
        let start = Instant::now();
        win::set_overlay(target)?;
        if !win::overlay()?.eq_ignore_ascii_case(target) {
            return Err("Windows did not activate the requested overlay; policy or hardware may override it.".into());
        }
        let duration_ms = start.elapsed().as_secs_f64() * 1000.;
        log(
            &self.dir,
            &format!("OVERLAY {target} | {duration_ms:.3} ms | {cause}"),
        );
        self.state.history.push(Transition {
            time_ms: now_ms(),
            name: cause.into(),
            duration_ms,
            cause: "Overlay API".into(),
            gaming: target == BEST_OVERLAY,
            account: self.account.clone(),
            ..Default::default()
        });
        if self.state.history.len() > 64 {
            self.state.history.remove(0);
        }
        Ok(())
    }
    fn manual_select(&mut self, role: &str) -> Result<(), String> {
        if !self.state.dry_run && self.lock.is_none() {
            self.state.legacy = win::legacy(false)?;
        }
        automation::manual_request(true, self.state.legacy, self.state.dry_run, role)?;
        if role == "auto" {
            self.state.manual_plan.clear();
            if self.state.monitoring {
                self.apply("Automatic policy resumed")?;
            } else {
                self.pause();
            }
            return Ok(());
        }
        self.validate_installed_plans()?;
        self.acquire_ownership()?;
        self.state.manual_plan = role.into();
        self.apply("Manual profile selection")
    }
    fn policy_enabled(&self) -> bool {
        (self.state.monitoring || !self.state.manual_plan.is_empty())
            && (self.config().battery_guard_enabled() || self.config().time_rule)
    }
    fn policy_tick(&mut self) -> Result<bool, String> {
        self.policy_at = Instant::now();
        let source = automation::power_source();
        let minute = automation::local_minute();
        let source_changed = source != self.state.power_source;
        let time_changed = self.config().time_rule && minute != self.policy_minute;
        self.state.power_source = source;
        self.policy_minute = minute;
        let apply = self.policy_retry.needed(source_changed || time_changed);
        if apply {
            self.apply(if source_changed {
                "Power source changed"
            } else if time_changed {
                "Local time rule evaluated"
            } else {
                "Retry previous power policy failure"
            })?;
        }
        Ok(apply)
    }
    /// One transaction path for both Settings and asynchronously imported
    /// profiles. Neither may commit over a failed restoration journal.
    fn configure(&mut self, c: Config) -> Result<(), String> {
        validate_config_plans(&c, &win::plans()?)?;
        let on = self.state.monitoring;
        let manual = self.state.manual_plan.clone();
        let old = self.config().clone();
        self.pause();
        require_restored(self.journal.dirty, self.state.dry_run)?;
        self.state.config = Some(c);
        if let Err(e) = self.config().save(&self.dir) {
            self.state.config = Some(old);
            if on {
                let _ = self.begin();
            }
            if !manual.is_empty() {
                let _ = self.manual_select(&manual);
            }
            return Err(e);
        }
        self.status("Settings saved.", false);
        if on {
            if let Err(e) = self.begin() {
                self.state.config = Some(old);
                let rollback = self.config().save(&self.dir);
                let _ = self.begin();
                if !manual.is_empty() {
                    let _ = self.manual_select(&manual);
                }
                return Err(format!(
                    "New settings could not restart monitoring: {e}. {}",
                    if rollback.is_ok() {
                        "Previous settings restored.".into()
                    } else {
                        format!(
                            "Saving previous settings also failed: {}",
                            rollback.unwrap_err()
                        )
                    }
                ));
            }
        }
        Ok(())
    }
    fn command(&mut self, id: u64, command: Command) -> Result<(), String> {
        if command.kind == HELLO {
            if let Ok(pid) = command.text.parse() {
                self.gui_pids.insert(id, pid);
            }
        }
        if command.kind == CLAIM {
            if self.owner != id {
                if let Some(old) = self.clients.get(&self.owner) {
                    let mut s = self.state.clone();
                    s.retired = true;
                    let _ = old.send(s);
                }
                self.owner = id;
                self.status(
                    "Control transferred to this window; the same event listeners remain active.",
                    false,
                );
            }
            return Ok(());
        }
        if id != self.owner {
            if [HELLO, VIEW].contains(&command.kind) {
                return Ok(());
            }
            return Err("This window is read-only until you take control.".into());
        }
        match command.kind {
            HELLO => {
                if !self.started {
                    self.started = true;
                    if !self.state.legacy && !command.flag && !self.state.error {
                        self.begin()?;
                    }
                }
            }
            TOGGLE => {
                if self.state.monitoring {
                    self.pause()
                } else {
                    self.begin()?
                }
            }
            ENSURE_MONITORING => {
                if command.flag {
                    self.begin()?;
                } else {
                    self.pause();
                }
            }
            MANUAL_PLAN => self.manual_select(&command.text)?,
            CONFIGURE => {
                let c = command.config.ok_or("Missing settings")?;
                self.configure(c)?;
            }
            IMPORT => {
                let tx = self.tx.clone();
                self.status("Reading executable icons...", false);
                thread::spawn(move || {
                    let _ = tx.send(Event::Imported(discovery::import(&command.values)));
                });
            }
            SCAN => {
                let tx = self.tx.clone();
                self.status(
                    "Scanning Steam, Epic, Ubisoft Connect and Xbox libraries...",
                    false,
                );
                thread::spawn(move || {
                    let _ = tx.send(Event::Scanned(discovery::scan()));
                });
            }
            STARTUP => {
                if self.state.dry_run {
                    return Err("Startup changes are disabled in preview mode.".into());
                }
                self.state.startup = win::startup(Some(!self.state.startup))?;
                self.status(
                    if self.state.startup {
                        "Startup installed for your Windows account."
                    } else {
                        "Native startup removed."
                    },
                    false,
                );
            }
            LEGACY => {
                if self.state.dry_run {
                    return Err("Handover is disabled in preview mode.".into());
                }
                win::legacy(true)?;
                self.state.legacy = false;
                self.status(
                    "Previous monitor stopped. Starting the native engine.",
                    false,
                );
                self.begin()?;
            }
            VIEW => {
                if self.overview != command.flag {
                    self.overview = command.flag;
                    self.reset_telemetry(command.flag);
                }
            }
            REFRESH => {
                self.state.plans = win::plans()?;
                self.state.active_guid = win::active()?;
                self.state.active_name = self
                    .state
                    .plans
                    .iter()
                    .find(|p| p.guid == self.state.active_guid)
                    .map(|p| p.name.clone())
                    .unwrap_or("Unavailable".into());
                if !self.state.monitoring {
                    self.snapshot_processes()?;
                }
                self.status("Windows state refreshed.", false);
            }
            SHUTDOWN => self.pause(),
            _ => {}
        }
        Ok(())
    }
    fn native(&mut self, kind: u32, pid: u32, name: String) -> Result<(), String> {
        if !self.state.monitoring {
            return Ok(());
        }
        match kind {
            1 => {
                if self
                    .config()
                    .names()
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(&name))
                {
                    self.pids.insert(pid, name.clone());
                    self.update_running();
                    self.state.manual_plan.clear();
                    self.apply_context("Process started", &name, pid)?;
                }
            }
            2 => {
                if let Some(name) = self.pids.remove(&pid) {
                    self.update_running();
                    self.state.manual_plan.clear();
                    self.apply_context("Process exited", &name, pid)?;
                }
            }
            3 => {
                let foreground_changed = self.foreground != pid;
                self.foreground = pid;
                self.foreground_full = name == "fullscreen";
                self.state.suspended = self.foreground_full && self.pids.contains_key(&pid);
                if self.state.suspended {
                    self.reset_telemetry(false);
                    self.state.telemetry =
                        "Rendering and telemetry suspended for full-screen game".into();
                }
                if self.config().foreground_only && foreground_changed {
                    let process = self.pids.get(&pid).cloned().unwrap_or_default();
                    self.apply_context("Foreground window changed", &process, pid)?;
                }
            }
            4 => self.apply("Windows power event")?,
            5 => {
                return Err(format!(
                    "Process event subscription failed: 0x{pid:08X}. Monitoring stopped."
                ));
            }
            6 => {
                self.state.ready = true;
                self.snapshot_processes()?;
                self.apply("Monitoring enabled")?;
            }
            7 => self.state.backend = name,
            _ => {}
        }
        Ok(())
    }
}
pub fn affinity(choice: &str, cores: &[Core]) -> Result<u64, String> {
    if choice.is_empty() {
        return Ok(0);
    }
    if ![
        "performance",
        "efficiency",
        "physical",
        "performance-physical",
    ]
    .contains(&choice)
    {
        return u64::from_str_radix(choice.trim_start_matches("0x"), 16)
            .ok()
            .filter(|v| *v != 0)
            .ok_or("Enter a nonzero hexadecimal affinity mask.".into());
    }
    if cores.iter().any(|c| c.group != 0 || c.logical >= 64) {
        return Err(
            "Affinity presets are unavailable on multi-group systems; use Windows scheduling."
                .into(),
        );
    }
    let min = cores.iter().map(|c| c.efficiency).min().unwrap_or(0);
    let max = cores.iter().map(|c| c.efficiency).max().unwrap_or(0);
    if min == max && choice != "physical" {
        return Err("Windows did not report distinct performance/efficiency classes.".into());
    }
    let target = if choice == "efficiency" { min } else { max };
    if ["physical", "performance-physical"].contains(&choice) {
        // Select one real logical processor per physical core. This is process
        // affinity, not a firmware change or global Hyper-Threading disable.
        let mut physical = HashMap::<u32, u32>::new();
        for c in cores
            .iter()
            .filter(|c| choice == "physical" || c.efficiency == target)
        {
            physical
                .entry(c.physical)
                .and_modify(|logical| *logical = (*logical).min(c.logical))
                .or_insert(c.logical);
        }
        let mask = physical
            .values()
            .fold(0, |mask, logical| mask | (1u64 << logical));
        return if mask == 0 {
            Err("No matching physical cores.".into())
        } else {
            Ok(mask)
        };
    }
    let mask = cores
        .iter()
        .filter(|c| c.efficiency == target && c.logical < 64)
        .fold(0, |mask, c| mask | (1u64 << c.logical));
    if mask == 0 {
        Err("No matching CPU cores.".into())
    } else {
        Ok(mask)
    }
}

pub fn daemon(dir: PathBuf, dry: bool) -> Result<(), String> {
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _daemon_lock = win::lock(&dir.join("native-engine.lock"))?;
    let (tx, rx) = mpsc::channel();
    let _ = EVENTS.set(Mutex::new(tx.clone()));
    let mut engine = Engine::new(dir.clone(), dry, tx.clone());
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if !dry {
        win::hidden_command(&exe.to_string_lossy())
            .args(["--guardian", &std::process::id().to_string()])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    thread::spawn(move || {
        let mut id = 0;
        loop {
            let Ok(mut read) = win::create_pipe() else {
                break;
            };
            if win::accept_pipe(&read).is_err() {
                continue;
            }
            id += 1;
            let client = id;
            let Ok(mut write) = read.try_clone() else {
                continue;
            };
            let (out_tx, out_rx) = mpsc::channel();
            if tx.send(Event::Connect(client, out_tx)).is_err() {
                break;
            }
            thread::spawn(move || {
                for state in out_rx {
                    if ipc::write(&mut write, &state).is_err() {
                        break;
                    }
                }
            });
            let tx = tx.clone();
            thread::spawn(move || {
                while let Ok(c) = ipc::read::<Command>(&mut read) {
                    if tx.send(Event::Client(client, c)).is_err() {
                        break;
                    }
                }
                let _ = tx.send(Event::Gone(client));
            });
        }
    });
    let mut ever_connected = false;
    loop {
        let sample = engine.telemetry_enabled();
        let policy = engine.policy_enabled();
        let event = if sample || policy {
            let telemetry_wait = if sample {
                Duration::from_secs(1).saturating_sub(engine.telemetry_at.elapsed())
            } else {
                Duration::MAX
            };
            let policy_wait = if policy {
                Duration::from_secs(5).saturating_sub(engine.policy_at.elapsed())
            } else {
                Duration::MAX
            };
            rx.recv_timeout(telemetry_wait.min(policy_wait)).ok()
        } else {
            match rx.recv() {
                Ok(e) => Some(e),
                Err(_) => break,
            }
        };
        if let Some(event) = event {
            let mut result = Ok(());
            match event {
                Event::Connect(id, tx) => {
                    ever_connected = true;
                    engine.clients.insert(id, tx);
                    if engine.owner == 0 {
                        engine.owner = id;
                    }
                }
                Event::Gone(id) => {
                    engine.clients.remove(&id);
                    engine.receipts.remove(id);
                    if engine.owner == id {
                        engine.owner = engine.clients.keys().copied().next().unwrap_or(0);
                    }
                    if ever_connected && engine.clients.is_empty() {
                        engine.pause();
                        break;
                    }
                }
                Event::Client(id, c) => {
                    let request = c.request_id;
                    result = engine.command(id, c);
                    engine.receipts.complete(id, request, &result);
                }
                Event::Native(kind, pid, name, generation) => {
                    if generation != engine.generation {
                        continue;
                    }
                    if kind == 1
                        && !engine
                            .config()
                            .names()
                            .iter()
                            .any(|n| n.eq_ignore_ascii_case(&name))
                    {
                        continue;
                    }
                    if kind == 2 && !engine.pids.contains_key(&pid) {
                        continue;
                    }
                    result = engine.native(kind, pid, name);
                    if kind == 5 {
                        engine.pause();
                    }
                }
                Event::Imported(import) => {
                    result = import.and_then(|games| {
                        let mut c = engine.config().clone();
                        for g in games {
                            c.add_names(&g.processes);
                            if let Some(existing) = c.games.iter_mut().find(|v| {
                                v.processes
                                    .iter()
                                    .any(|p| g.processes.iter().any(|n| n.eq_ignore_ascii_case(p)))
                            }) {
                                if existing.id != "siege" {
                                    existing.name = g.name;
                                    existing.icon = g.icon;
                                    existing.source = g.source;
                                }
                            }
                        }
                        engine.configure(c)?;
                        if !engine.state.monitoring {
                            engine.status("Game profiles imported.", false);
                        }
                        Ok(())
                    });
                }
                Event::Scanned(r) => {
                    result = r.map(|games| {
                        let n = games.len();
                        engine.state.discovered = games;
                        engine.status(
                            format!("Found {n} executable candidates. Review names before adding."),
                            false,
                        );
                    });
                }
            }
            if let Err(e) = result {
                engine.status(e, true);
            }
            engine.broadcast();
        }
        if engine.telemetry_enabled() && engine.telemetry_at.elapsed() >= Duration::from_secs(1) {
            engine.sample_telemetry();
            engine.broadcast();
        }
        // Cheap AC/DC/time observations supplement Win32 notifications; no
        // process polling is added and paused/no-rule views remain event-driven.
        if engine.policy_enabled() && engine.policy_at.elapsed() >= Duration::from_secs(5) {
            match engine.policy_tick() {
                Ok(true) => engine.broadcast(),
                Ok(false) => {}
                Err(e) => {
                    engine.status(e, true);
                    engine.broadcast();
                }
            }
        }
    }
    Ok(())
}
pub fn guardian(pid: u32) {
    win::wait_process(pid);
    let dir = data_dir();
    let path = dir.join("recovery.json");
    let Ok(bytes) = fs::read(&path) else { return };
    let Ok(mut journal) = serde_json::from_slice::<Journal>(&bytes) else {
        return;
    };
    if journal.pid != pid || !journal.dirty {
        return;
    }
    let Ok(_lock) = win::lock(&win::monitor_path()) else {
        return;
    };
    journal
        .schedules
        .retain(|s| match win::restore_schedule(s) {
            Ok(()) => false,
            Err(e) => {
                log(&dir, &format!("Recovery PID {}: {e}", s.pid));
                true
            }
        });
    if let Some(overlay) = journal.overlay.clone() {
        match win::set_overlay(&overlay) {
            Ok(()) => journal.overlay = None,
            Err(e) => log(&dir, &format!("Recovery overlay: {e}")),
        }
    }
    if valid_guid(&journal.default_plan)
        && crate::power::select(&mut crate::power::Windows, &journal.default_plan).is_ok()
    {
        journal.dirty = journal.overlay.is_some() || !journal.schedules.is_empty();
        let _ = atomic_json(&path, &journal);
        log(
            &dir,
            "Supervisor restored default power plan after engine exit.",
        );
    } else {
        log(
            &dir,
            "Supervisor could not restore the default plan. The recovery journal remains pending.",
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_ring_payload_preserves_preview_mode_with_real_telemetry() {
        for dry in [false, true] {
            let state = Snapshot {
                dry_run: dry,
                cores: vec![Core {
                    logical: 3,
                    load: 42.,
                    ..Default::default()
                }],
                history: vec![Transition {
                    process: "Game.exe".into(),
                    ..Default::default()
                }],
                ..Default::default()
            };
            let bytes = telemetry_snapshot(&state).encode_to_vec();
            let decoded = Snapshot::decode(bytes.as_slice()).unwrap();
            crate::runtime_scope::verify_peer(dry, decoded.dry_run).unwrap();
            assert_eq!(decoded.cores, state.cores);
            assert_eq!(decoded.history, state.history);
            assert!(crate::runtime_scope::verify_peer(!dry, decoded.dry_run).is_err());
        }
    }
    #[test]
    fn tracked_command_receipts_are_isolated_per_pipe_client() {
        let mut receipts = CommandReceipts::default();
        receipts.complete(1, 7, &Ok(()));
        receipts.complete(2, 7, &Err("Handover is disabled in preview mode.".into()));
        let mut snapshot = Snapshot::default();
        receipts.apply(1, &mut snapshot);
        assert_eq!(snapshot.command_id, 7);
        assert!(snapshot.command_error.is_empty());
        receipts.apply(2, &mut snapshot);
        assert_eq!(snapshot.command_id, 7);
        assert!(snapshot.command_error.contains("preview mode"));
        receipts.apply(3, &mut snapshot);
        assert_eq!(snapshot.command_id, 0);
        assert!(snapshot.command_error.is_empty());
    }
    #[test]
    fn receipts_survive_untracked_events_but_not_client_disconnect() {
        let mut receipts = CommandReceipts::default();
        receipts.complete(1, 44, &Err("Settings could not be saved".into()));
        receipts.complete(1, 0, &Ok(())); // a VIEW command is not an ACK
        let mut snapshot = Snapshot::default();
        receipts.apply(1, &mut snapshot);
        assert_eq!(snapshot.command_id, 44);
        assert!(!snapshot.command_error.is_empty());
        receipts.complete(1, 45, &Ok(()));
        receipts.apply(1, &mut snapshot);
        assert_eq!(snapshot.command_id, 45);
        assert!(snapshot.command_error.is_empty());
        receipts.remove(1);
        receipts.apply(1, &mut snapshot);
        assert_eq!(snapshot.command_id, 0);
        assert!(snapshot.command_error.is_empty());
    }
    #[test]
    fn failed_policy_retries_unchanged_source_then_stops_after_success() {
        let mut retry = PolicyRetry::default();
        let mut applies = 0;
        // The first change fails after its new AC/DC source has been observed.
        for (changed, outcome) in [
            (true, Err("transient Default switch failure".into())),
            (false, Ok(())),
            (false, Ok(())),
        ] {
            if retry.needed(changed) {
                applies += 1;
                retry.completed(true, &outcome);
            }
        }
        assert_eq!(
            applies, 2,
            "an unchanged successful policy must not keep applying"
        );
        assert!(!retry.needed(false));
        // A native power event can fail before the periodic observer sees any
        // source change; that same pending state must also request a retry.
        retry.completed(true, &Err("native power event failed".into()));
        assert!(retry.needed(false));
        retry.completed(false, &Err("policy no longer enabled".into()));
        assert!(!retry.needed(false));
    }
    #[test]
    fn dirty_restoration_blocks_both_config_commit_entry_points() {
        // Settings and imported profiles now share configure(), including this
        // guard before assignment/save. A failed restore retains the old data.
        for origin in ["settings", "imported profiles"] {
            let old = defaults();
            let mut candidate = old.clone();
            candidate.foreground_only = true;
            let mut saved = old.clone();
            let result = require_restored(true, false).map(|()| saved = candidate);
            assert!(
                result.is_err(),
                "{origin} must reject a pending restoration"
            );
            assert_eq!(saved, old);
        }
        assert!(require_restored(false, false).is_ok());
        assert!(require_restored(true, true).is_ok());
    }
    #[test]
    fn affinity_classes() {
        let c = vec![
            Core {
                logical: 0,
                efficiency: 1,
                ..Default::default()
            },
            Core {
                logical: 1,
                efficiency: 0,
                ..Default::default()
            },
        ];
        assert_eq!(affinity("performance", &c).unwrap(), 1);
        assert_eq!(affinity("efficiency", &c).unwrap(), 2);
        assert!(affinity("0", &c).is_err());
    }
    #[test]
    fn homogeneous_not_guessed() {
        assert!(affinity("performance", &[Core::default()]).is_err());
    }
    #[test]
    fn physical_affinity_uses_one_real_thread_per_core_and_refuses_groups() {
        let mut cores: Vec<_> = [(3, 1, 1), (0, 0, 1), (1, 0, 1), (2, 1, 1), (4, 2, 0)]
            .into_iter()
            .map(|(logical, physical, efficiency)| Core {
                logical,
                physical,
                efficiency,
                ..Default::default()
            })
            .collect();
        assert_eq!(affinity("physical", &cores).unwrap(), 0x15);
        assert_eq!(affinity("performance-physical", &cores).unwrap(), 0x05);
        assert_eq!(affinity("performance", &cores).unwrap(), 0x0f);
        assert_eq!(affinity("physical", &[Core::default()]).unwrap(), 1);
        assert!(affinity("physical", &[]).is_err());
        cores[0].group = 1;
        for preset in [
            "physical",
            "performance-physical",
            "performance",
            "efficiency",
        ] {
            assert!(affinity(preset, &cores).is_err());
        }
        cores[0].group = 0;
        cores[0].logical = 64;
        assert!(affinity("physical", &cores).is_err());
    }
    #[test]
    fn configuration_preflight_rejects_missing_override_before_old_state_changes() {
        let original = defaults();
        let mut proposed = original.clone();
        let plans = vec![
            Plan {
                guid: GAMING.into(),
                name: "Game".into(),
            },
            Plan {
                guid: BALANCED.into(),
                name: "Default".into(),
            },
        ];
        assert!(validate_config_plans(&original, &plans).is_ok());
        proposed.games[0].power_plan = "00000000-0000-0000-0000-000000000001".into();
        assert!(validate_config_plans(&proposed, &plans).is_err());
        assert!(original.games[0].power_plan.is_empty());
        proposed.games[0].power_plan = GAMING.to_uppercase();
        assert!(validate_config_plans(&proposed, &plans).is_ok());
    }
}
