// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Read-only projections for the Pro frontend. No power or scheduling writes.
//! CPU charts contain measured observations; plan time is observed session time.
use crate::model::{Config, Core, Game, Snapshot, Transition};
use std::collections::{BTreeMap, VecDeque};

const WINDOW_MS: u64 = 30_000;
const FRESH_MS: u64 = 2_500;

#[derive(Clone, Debug)]
struct CpuPoint {
    at: u64,
    load: f64,
    segment: u64,
    p_load: Option<f64>,
    e_load: Option<f64>,
    plan: String,
}

#[derive(Default)]
pub struct CpuTrend {
    points: VecDeque<CpuPoint>,
    last_sample: Option<u64>,
    segment: u64,
    gap: bool,
    fresh: bool,
    classes: Option<BTreeMap<(u32, u32), String>>,
}
impl CpuTrend {
    pub fn set_topology(&mut self, topology: &crate::cpu_topology::CpuTopology) {
        self.classes = Some(
            topology
                .logicals
                .iter()
                .map(|c| ((c.group, c.logical), c.class.clone()))
                .collect(),
        );
    }
    #[cfg(test)]
    pub fn observe(&mut self, now: u64, sample_at: u64, cores: &[Core], visible: bool) {
        self.observe_context(now, sample_at, cores, visible, "Plan not captured");
    }
    pub fn observe_context(
        &mut self,
        now: u64,
        sample_at: u64,
        cores: &[Core],
        visible: bool,
        plan: &str,
    ) {
        self.trim(now);
        let values: Vec<_> = cores
            .iter()
            .filter_map(|c| {
                (c.load.is_finite() && (0.0..=100.0).contains(&c.load)).then_some(c.load)
            })
            .collect();
        let available = visible
            && sample_at > 0
            && sample_at <= now
            && now.saturating_sub(sample_at) <= FRESH_MS
            && !values.is_empty();
        self.fresh = available;
        if !available {
            self.gap = true;
            return;
        }
        if self.last_sample == Some(sample_at) {
            return; // Repeated power/status snapshots must not invent CPU samples.
        }
        if self
            .last_sample
            .is_some_and(|previous| sample_at < previous)
        {
            // Wall clock correction: do not draw backward or join unrelated time.
            self.points.clear();
            self.gap = true;
        }
        if self.gap
            || self
                .last_sample
                .is_some_and(|previous| sample_at.saturating_sub(previous) > FRESH_MS)
        {
            self.segment = self.segment.wrapping_add(1);
        }
        self.gap = false;
        self.last_sample = Some(sample_at);
        self.points.push_back(CpuPoint {
            at: sample_at,
            load: values.iter().sum::<f64>() / values.len() as f64,
            segment: self.segment,
            p_load: class_average(cores, "P", self.classes.as_ref()),
            e_load: class_average(cores, "E", self.classes.as_ref()),
            plan: plan.into(),
        });
        self.trim(now);
    }
    pub fn clear(&mut self) {
        let classes = self.classes.take();
        *self = Self::default();
        self.classes = classes;
    }
    fn trim(&mut self, now: u64) {
        while self
            .points
            .front()
            .is_some_and(|p| p.at < now.saturating_sub(WINDOW_MS) || p.at > now)
        {
            self.points.pop_front();
        }
        // Also bound memory if a faulty producer emits faster than the 1s contract.
        while self.points.len() > 256 {
            self.points.pop_front();
        }
    }
    pub fn path(&self, now: u64) -> String {
        self.class_path(now, "all")
    }
    pub fn class_path(&self, now: u64, class: &str) -> String {
        let mut path = String::new();
        let mut segment = None;
        for p in self
            .points
            .iter()
            .filter(|p| p.at <= now && now - p.at <= WINDOW_MS)
        {
            let x = 600. * (1. - (now - p.at) as f64 / WINDOW_MS as f64);
            let load = match class {
                "P" => p.p_load,
                "E" => p.e_load,
                _ => Some(p.load),
            };
            let Some(load) = load else {
                segment = None;
                continue;
            };
            let y = 100. - load;
            let command = if segment == Some(p.segment) { "L" } else { "M" };
            path.push_str(&format!("{command} {x:.2} {y:.2} "));
            // A first point remains visible without pretending there is a trend.
            if command == "M" {
                path.push_str(&format!("L {:.2} {y:.2} ", (x + 0.1).min(600.)));
            }
            segment = Some(p.segment);
        }
        path
    }
    /// Select only an actual observation; do not interpolate missing readings.
    #[cfg(test)]
    pub fn tooltip(&self, now: u64, fraction: f32) -> String {
        self.topology_tooltip(now, fraction, true)
    }
    pub fn topology_tooltip(&self, now: u64, fraction: f32, hybrid: bool) -> String {
        if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
            return String::new();
        }
        let target = now.saturating_sub(((1. - fraction) * WINDOW_MS as f32) as u64);
        let point = self
            .points
            .iter()
            .filter(|p| p.at <= now && now - p.at <= WINDOW_MS)
            .min_by_key(|p| p.at.abs_diff(target));
        match point.filter(|p| p.at.abs_diff(target) <= 1500) {
            Some(p) => {
                // Class values come from this exact recorded sample, never the
                // latest snapshot. Homogeneous or missing classes remain —.
                let class_load = |value: Option<f64>| {
                    value
                        .map(|load| format!("{load:.1}%"))
                        .unwrap_or_else(|| "—".into())
                };
                if !hybrid {
                    return format!("{} · CPU {:5.1}%\n{}", local_clock(p.at), p.load, p.plan);
                }
                format!(
                    "{} · CPU {:5.1}% · P {} · E {}\n{}",
                    local_clock(p.at),
                    p.load,
                    class_load(p.p_load),
                    class_load(p.e_load),
                    p.plan
                )
            }
            None => "No measured sample at this time".into(),
        }
    }
    pub fn available(&self, now: u64) -> bool {
        self.points
            .iter()
            .any(|p| p.at <= now && now - p.at <= WINDOW_MS)
    }
    pub fn label(&self) -> String {
        if self.fresh {
            match self.points.back() {
                Some(p) => format!("30 s measured CPU load · {:3.0}% latest", p.load),
                None => "Waiting for a real CPU sample".into(),
            }
        } else {
            "30 s history · awaiting a fresh sample".into()
        }
    }
}
fn class_average(
    cores: &[Core],
    class: &str,
    classes: Option<&BTreeMap<(u32, u32), String>>,
) -> Option<f64> {
    if let Some(classes) = classes {
        let (sum, count) = cores
            .iter()
            .filter(|c| {
                classes
                    .get(&(c.group, c.logical))
                    .is_some_and(|name| name == class)
                    && c.load.is_finite()
                    && (0.0..=100.0).contains(&c.load)
            })
            .fold((0.0, 0usize), |(sum, count), c| (sum + c.load, count + 1));
        return (count > 0).then(|| sum / count as f64);
    }
    let high = cores.iter().map(|c| c.efficiency).max()?;
    let low = cores.iter().map(|c| c.efficiency).min()?;
    if high == low {
        return None;
    }
    let values: Vec<_> = cores
        .iter()
        .filter(|c| {
            (if class == "P" {
                c.efficiency == high
            } else {
                c.efficiency != high
            }) && c.load.is_finite()
                && (0.0..=100.0).contains(&c.load)
        })
        .map(|c| c.load)
        .collect();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

#[cfg(test)]
#[test]
fn topology_classes_join_by_group_and_survive_history_reset() {
    let classes = BTreeMap::from([((0, 0), "P".into()), ((1, 0), "E".into())]);
    let cores = vec![
        Core {
            group: 1,
            logical: 0,
            efficiency: 255,
            load: 12.,
            ..Default::default()
        },
        Core {
            group: 0,
            logical: 0,
            efficiency: 0,
            load: 78.,
            ..Default::default()
        },
        Core {
            group: 2,
            logical: 0,
            efficiency: 255,
            load: 99.,
            ..Default::default()
        },
    ];
    assert_eq!(class_average(&cores, "P", Some(&classes)), Some(78.));
    assert_eq!(class_average(&cores, "E", Some(&classes)), Some(12.));
    assert_eq!(class_average(&cores, "P", Some(&BTreeMap::new())), None);
    let mut trend = CpuTrend {
        classes: Some(classes.clone()),
        ..Default::default()
    };
    trend.clear();
    assert_eq!(trend.classes, Some(classes));
}

pub fn switch_markers(history: &[Transition], now: u64) -> String {
    let mut path = String::new();
    let mut seen = Vec::new();
    for event in history {
        if event.cause == "Overlay API"
            || event.time_ms > now
            || now - event.time_ms > WINDOW_MS
            || seen.contains(&event.time_ms)
        {
            continue;
        }
        seen.push(event.time_ms);
        let x = 600. * (1. - (now - event.time_ms) as f64 / WINDOW_MS as f64);
        path.push_str(&format!("M {x:.2} 0 L {x:.2} 100 "));
    }
    path
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PlanClass {
    Gaming,
    Default,
    #[default]
    Unknown,
}
fn classify(guid: &str, config: Option<&Config>) -> PlanClass {
    let Some(config) = config else {
        return PlanClass::Unknown;
    };
    // Identical roles are ambiguous; never double-count the same milliseconds.
    if config.gaming.eq_ignore_ascii_case(&config.default_plan) || guid.is_empty() {
        PlanClass::Unknown
    } else if guid.eq_ignore_ascii_case(&config.gaming)
        || (!guid.eq_ignore_ascii_case(&config.default_plan)
            && config
                .games
                .iter()
                .any(|g| !g.power_plan.is_empty() && guid.eq_ignore_ascii_case(&g.power_plan)))
    {
        PlanClass::Gaming
    } else if guid.eq_ignore_ascii_case(&config.default_plan) {
        PlanClass::Default
    } else {
        PlanClass::Unknown
    }
}
#[derive(Default)]
pub struct SessionTimes {
    cursor_ms: u64,
    class: PlanClass,
    gaming_ms: u64,
    default_ms: u64,
    unknown_ms: u64,
    ever_known: bool,
}
impl SessionTimes {
    pub fn milliseconds(&self) -> (u64, u64, u64) {
        (self.gaming_ms, self.default_ms, self.unknown_ms)
    }
    pub fn observe(&mut self, elapsed_ms: u64, guid: &str, config: Option<&Config>) {
        // elapsed_ms is monotonic time since this GUI opened, not calendar time.
        let delta = elapsed_ms.saturating_sub(self.cursor_ms);
        match self.class {
            PlanClass::Gaming => self.gaming_ms += delta,
            PlanClass::Default => self.default_ms += delta,
            PlanClass::Unknown => self.unknown_ms += delta,
        }
        self.cursor_ms = elapsed_ms.max(self.cursor_ms);
        self.class = classify(guid, config);
        self.ever_known |= self.class != PlanClass::Unknown;
    }
    pub fn gaming(&self) -> String {
        if self.ever_known {
            duration(self.gaming_ms)
        } else {
            "—".into()
        }
    }
    pub fn default_plan(&self) -> String {
        if self.ever_known {
            duration(self.default_ms)
        } else {
            "—".into()
        }
    }
    pub fn ratio(&self) -> f32 {
        let known = self.gaming_ms + self.default_ms;
        if known == 0 {
            0.
        } else {
            self.gaming_ms as f32 / known as f32
        }
    }
    pub fn label(&self) -> String {
        format!(
            "This window · observed plan time · {} unknown excluded",
            duration(self.unknown_ms)
        )
    }
}
fn duration(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds >= 3600 {
        format!("{}h {:02}m", seconds / 3600, seconds / 60 % 60)
    } else if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}
pub fn donut_path(ratio: f32) -> String {
    let ratio = if ratio.is_finite() {
        ratio.clamp(0., 1.)
    } else {
        0.
    };
    if ratio <= 0. {
        return String::new();
    }
    if ratio >= 1. {
        return "M 16 4 A 12 12 0 0 1 16 28 A 12 12 0 0 1 16 4".into();
    }
    let angle = std::f64::consts::TAU * ratio as f64;
    format!(
        "M 16 4 A 12 12 0 {} 1 {:.3} {:.3}",
        u8::from(ratio > 0.5),
        16. + 12. * angle.sin(),
        16. - 12. * angle.cos()
    )
}

pub fn visible_running(games: &[Game], snapshot: &Snapshot) -> Vec<bool> {
    let available = snapshot.monitoring && snapshot.ready;
    games
        .iter()
        .map(|game| {
            available
                && game.processes.iter().any(|name| {
                    snapshot
                        .running
                        .iter()
                        .any(|running| name.eq_ignore_ascii_case(running))
                })
        })
        .collect()
}
/// Keep unavailable distinct from a genuine measured zero in Slint bars/colors.
pub fn core_activity(load: f64) -> f32 {
    if load.is_finite() && (0.0..=100.0).contains(&load) {
        (load / 100.) as f32
    } else {
        -1.
    }
}
pub fn detector_label(snapshot: &Snapshot) -> String {
    if !snapshot.monitoring {
        return "Windows process detector · paused".into();
    }
    if !snapshot.ready {
        return "Connecting to Windows process detector".into();
    }
    if snapshot.backend.contains("Compatibility") {
        "WMI compatibility · 1 s start checks + process exit waits".into()
    } else if snapshot.backend.is_empty() {
        "Windows process detector · backend not reported".into()
    } else {
        snapshot.backend.clone()
    }
}

pub struct LogRow {
    pub label: String,
    pub detail: String,
    pub gaming: bool,
    pub duration_ms: f64,
}
pub fn log_rows(history: &[Transition]) -> Vec<LogRow> {
    history
        .iter()
        .rev()
        .take(64)
        .map(|event| {
            let overlay = event.cause == "Overlay API";
            LogRow {
                label: if overlay {
                    format!("{} · Overlay changed", local_clock(event.time_ms))
                } else {
                    format!(
                        "{} · {} → {}",
                        local_clock(event.time_ms),
                        if !event.from_name.is_empty() {
                            &event.from_name
                        } else if !event.from_guid.is_empty() {
                            &event.from_guid
                        } else {
                            "Source not captured"
                        },
                        event.name
                    )
                },
                detail: format!(
                    "{}{} · {:.2} ms{}",
                    if overlay { &event.name } else { &event.cause },
                    if event.process.is_empty() {
                        String::new()
                    } else {
                        format!(" via {}", event.process)
                    },
                    event.duration_ms,
                    if overlay {
                        " · overlay GUIDs not captured"
                    } else {
                        ""
                    }
                ),
                gaming: event.gaming,
                duration_ms: event.duration_ms,
            }
        })
        .collect()
}

// The product uses the PC's local clock. No extra runtime/date library required.
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
#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn FileTimeToSystemTime(file: *const FileTime, system: *mut SystemTime) -> i32;
    fn SystemTimeToTzSpecificLocalTime(
        zone: *const std::ffi::c_void,
        utc: *const SystemTime,
        local: *mut SystemTime,
    ) -> i32;
}
pub fn local_clock(unix_ms: u64) -> String {
    let Some(ticks) = unix_ms
        .checked_mul(10_000)
        .and_then(|v| v.checked_add(116_444_736_000_000_000))
    else {
        return "Time unavailable".into();
    };
    let file = FileTime {
        low: ticks as u32,
        high: (ticks >> 32) as u32,
    };
    let mut utc = SystemTime::default();
    let mut local = SystemTime::default();
    if unsafe { FileTimeToSystemTime(&file, &mut utc) } == 0
        || unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) } == 0
    {
        return "Time unavailable".into();
    }
    format!("{:02}:{:02}:{:02}", local.hour, local.minute, local.second)
}

#[derive(Default)]
pub struct ProView {
    pub cpu: CpuTrend,
    pub session: SessionTimes,
}
impl ProView {
    /// Finalize an export at the current time without treating a cached snapshot
    /// as a live observation after the engine connection has closed.
    pub fn advance_export(&mut self, snapshot: &Snapshot, elapsed_ms: u64, connected: bool) {
        if connected {
            self.session
                .observe(elapsed_ms, &snapshot.active_guid, snapshot.config.as_ref());
        } else {
            self.session.observe(elapsed_ms, "", None);
        }
    }
    pub fn observe(&mut self, snapshot: &Snapshot, elapsed_ms: u64, now: u64, visible: bool) {
        self.cpu.observe_context(
            now,
            snapshot.cpu_sampled_at_ms,
            &snapshot.cores,
            visible && !snapshot.suspended,
            &snapshot.active_name,
        );
        self.session
            .observe(elapsed_ms, &snapshot.active_guid, snapshot.config.as_ref());
    }
    pub fn disconnect(&mut self, elapsed_ms: u64) {
        // No further engine snapshots will advance a disconnected chart's
        // rolling window. Clear it rather than leaving a frozen "30 s" graph.
        self.cpu.clear();
        self.session.observe(elapsed_ms, "", None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn core(load: f64) -> Core {
        Core {
            load,
            ..Default::default()
        }
    }
    #[test]
    fn graph_hover_keeps_sample_plan_and_separates_hybrid_averages() {
        let mut trend = CpuTrend::default();
        trend.observe_context(
            30_000,
            30_000,
            &[
                Core {
                    efficiency: 8,
                    load: 80.,
                    ..Default::default()
                },
                Core {
                    efficiency: 0,
                    load: 20.,
                    ..Default::default()
                },
            ],
            true,
            "Gaming",
        );
        assert_eq!(trend.points[0].p_load, Some(80.));
        assert_eq!(trend.points[0].e_load, Some(20.));
        let tooltip = trend.tooltip(30_000, 1.);
        assert!(tooltip.contains("CPU  50.0%"));
        assert!(tooltip.contains("P 80.0% · E 20.0%"));
        assert!(tooltip.ends_with("\nGaming"));
        let standard = trend.topology_tooltip(30_000, 1., false);
        assert!(standard.contains("CPU  50.0%") && standard.ends_with("\nGaming"));
        assert!(!standard.contains("P 80.0%"));
        assert_eq!(trend.tooltip(30_000, 0.), "No measured sample at this time");
        assert!(trend.tooltip(30_000, -1.).is_empty());
        trend.observe_context(31_000, 31_000, &[core(10.)], true, "Balanced");
        assert_eq!(trend.points[1].p_load, None);
        assert!(trend.tooltip(31_000, 1.).contains("P — · E —"));
        assert!(trend.tooltip(31_000, 1. - 1. / 30.).contains("Gaming"));
    }
    fn config() -> Config {
        Config {
            gaming: "GAME".into(),
            default_plan: "BASE".into(),
            ..Default::default()
        }
    }
    #[test]
    fn cpu_rolls_thirty_seconds_and_rejects_repeated_stale_samples() {
        let mut trend = CpuTrend::default();
        for second in 1..=40 {
            trend.observe(second * 1000, second * 1000, &[core(40.)], true);
        }
        assert_eq!(trend.points.len(), 31);
        assert_eq!(trend.points.front().unwrap().at, 10_000);
        trend.observe(40_000, 40_000, &[core(99.)], true);
        assert_eq!(trend.points.back().unwrap().load, 40.);
        trend.observe(44_000, 40_000, &[core(99.)], true);
        assert!(!trend.fresh);
        assert_eq!(trend.points.back().unwrap().load, 40.);
    }
    #[test]
    fn cpu_gaps_never_join_hidden_or_missing_values_as_zero() {
        let mut trend = CpuTrend::default();
        trend.observe(1000, 1000, &[core(50.)], true);
        trend.observe(2000, 2000, &[core(0.)], false);
        trend.observe(3000, 0, &[core(-1.)], true);
        trend.observe(4000, 4000, &[core(20.), core(-1.)], true);
        assert_eq!(trend.points.len(), 2);
        assert_eq!(trend.path(4000).matches('M').count(), 2);
        assert_eq!(trend.points.back().unwrap().load, 20.);
    }
    #[test]
    fn initial_active_plan_is_counted_before_any_transition_and_unknown_excluded() {
        let mut session = SessionTimes::default();
        let config = config();
        session.observe(1000, "base", Some(&config));
        session.observe(6000, "GAME", Some(&config));
        session.observe(9000, "third-plan", Some(&config));
        session.observe(12_000, "BASE", Some(&config));
        assert_eq!(session.default_ms, 5000);
        assert_eq!(session.gaming_ms, 3000);
        assert_eq!(session.unknown_ms, 4000);
        assert_eq!(session.ratio(), 0.375);
    }
    #[test]
    fn session_disconnect_stops_known_accounting_and_ambiguous_roles_are_unknown() {
        let mut session = SessionTimes::default();
        let config = config();
        session.observe(0, "GAME", Some(&config));
        session.observe(3000, "", None);
        session.observe(10_000, "", None);
        assert_eq!(session.gaming_ms, 3000);
        assert_eq!(session.unknown_ms, 7000);
        let same = Config {
            gaming: "same".into(),
            default_plan: "SAME".into(),
            ..Default::default()
        };
        assert_eq!(classify("same", Some(&same)), PlanClass::Unknown);
    }
    #[test]
    fn repeated_exports_do_not_reclassify_disconnected_time_from_cached_plan() {
        let mut view = ProView::default();
        let snapshot = Snapshot {
            active_guid: "GAME".into(),
            config: Some(config()),
            ..Default::default()
        };
        view.advance_export(&snapshot, 0, true);
        view.disconnect(3000);
        view.advance_export(&snapshot, 6000, false);
        view.advance_export(&snapshot, 10_000, false);
        assert_eq!(view.session.gaming_ms, 3000);
        assert_eq!(view.session.default_ms, 0);
        assert_eq!(view.session.unknown_ms, 7000);
        // A real reconnect starts accounting from that observation, never backfills.
        view.advance_export(&snapshot, 12_000, true);
        view.advance_export(&snapshot, 14_000, true);
        assert_eq!(view.session.gaming_ms, 5000);
        assert_eq!(view.session.unknown_ms, 9000);
    }
    #[test]
    fn game_running_alignment_matches_any_alias_only_with_live_detector() {
        let games = vec![
            Game {
                processes: vec!["one.exe".into(), "Two.exe".into()],
                ..Default::default()
            },
            Game {
                processes: vec!["three.exe".into()],
                ..Default::default()
            },
        ];
        let mut snapshot = Snapshot {
            monitoring: true,
            ready: true,
            running: vec!["TWO.EXE".into()],
            ..Default::default()
        };
        assert_eq!(visible_running(&games, &snapshot), [true, false]);
        snapshot.ready = false;
        assert_eq!(visible_running(&games, &snapshot), [false, false]);
    }
    #[test]
    fn markers_only_show_recent_plan_switches_and_donut_handles_bounds() {
        let history = vec![
            Transition {
                time_ms: 1000,
                ..Default::default()
            },
            Transition {
                time_ms: 39_000,
                cause: "Overlay API".into(),
                ..Default::default()
            },
            Transition {
                time_ms: 40_000,
                ..Default::default()
            },
        ];
        assert_eq!(switch_markers(&history, 40_000).matches('M').count(), 1);
        assert!(donut_path(0.).is_empty());
        assert!(donut_path(f32::NAN).is_empty());
        assert!(donut_path(0.25).ends_with("28.000 16.000"));
        assert_eq!(donut_path(1.).matches('A').count(), 2);
        assert_eq!(donut_path(2.), donut_path(1.));
    }
    #[test]
    fn new_history_uses_recorded_source_and_process_without_guessing_old_events() {
        let rows = log_rows(&[
            Transition {
                name: "Gaming".into(),
                cause: "Process started".into(),
                ..Default::default()
            },
            Transition {
                name: "Balanced".into(),
                from_name: "Gaming".into(),
                cause: "Process exited".into(),
                process: "RainbowSix.exe".into(),
                ..Default::default()
            },
        ]);
        assert!(rows[0].label.ends_with("Gaming → Balanced"));
        assert!(
            rows[0]
                .detail
                .starts_with("Process exited via RainbowSix.exe")
        );
        assert!(rows[1].label.ends_with("Source not captured → Gaming"));
        assert!(!rows[1].detail.contains(" via "));
    }
    #[test]
    fn unavailable_activity_and_disconnect_never_leave_a_zero_or_frozen_chart() {
        assert_eq!(core_activity(0.), 0.);
        assert_eq!(core_activity(100.), 1.);
        for unavailable in [-1., 101., f64::NAN, f64::INFINITY] {
            assert_eq!(core_activity(unavailable), -1.);
        }
        let mut view = ProView::default();
        view.cpu.observe(1000, 1000, &[core(42.)], true);
        assert!(view.cpu.available(1000));
        view.disconnect(1500);
        assert!(!view.cpu.available(1500));
        assert!(view.cpu.path(1500).is_empty());
    }
}
