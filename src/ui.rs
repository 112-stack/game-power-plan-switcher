// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! GUI controller: immutable snapshots in, typed commands out. No power API waits.
use crate::{
    AppWindow, CoreRow, CoreSection, GameRow, HistoryRow, ipc,
    localization::{self, format as trf, tr},
    model::*,
    pro_view::{self, ProView},
    win,
};
use prost::Message;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{Color, ComponentHandle, Image, Model, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
thread_local! {static CONTROL:RefCell<Option<Rc<RefCell<Controller>>>>=const{RefCell::new(None)};}
thread_local! {static LANGUAGE_REFRESH_QUEUED:Cell<bool>=const{Cell::new(false)};}

fn schedule_system_language_refresh() {
    if LANGUAGE_REFRESH_QUEUED.with(|queued| queued.replace(true)) {
        return;
    }
    // Deferred, coalesced event work avoids a Controller borrow inside native
    // focus callbacks (which may arrive while a modal/file dialog is active).
    // This is not a periodic timer or a keyboard-layout observer.
    slint::Timer::single_shot(Duration::ZERO, || {
        LANGUAGE_REFRESH_QUEUED.with(|queued| queued.set(false));
        CONTROL.with(|cell| {
            if let Some(control) = cell.borrow().as_ref() {
                control.borrow_mut().refresh_system_language();
            }
        });
    });
}
// Slint's attributes hook runs when its WindowAdapter is created, before the
// native HWND exists. A construction guard identifies only our telemetry HUD;
// all main windows, dialogs and unrelated probe processes retain their defaults.
thread_local! {static CREATING_HUD:Cell<bool>=const{Cell::new(false)};}

fn new_telemetry_hud() -> Option<crate::TelemetryHud> {
    CREATING_HUD.with(|guard| {
        let previous = guard.replace(true);
        let result = crate::TelemetryHud::new();
        if let Ok(hud) = &result {
            // Force the lazy WindowAdapter under this guard. With winit 0.30 an
            // HWND may be deferred until the event loop; its attributes remain.
            let _ = hud.window().has_winit_window();
        }
        guard.set(previous);
        result.ok()
    })
}
type Writer = mpsc::SyncSender<Command>;
fn hud_setup_current(requested: u64, current: u64, enabled: bool) -> bool {
    enabled && requested == current
}
#[derive(Debug, PartialEq, Eq)]
struct ViewPolicy {
    main: bool,
    cpu: bool,
    sensors: bool,
}
fn view_policy(
    shown: bool,
    focused: bool,
    occluded: bool,
    page: i32,
    hud: bool,
    fullscreen: bool,
) -> ViewPolicy {
    // An enabled HUD is also suspended for a monitored full-screen game. This
    // preserves its preference while preventing GPU sensor updates from waking
    // the main window or hidden HUD if Windows does not report occlusion.
    if fullscreen {
        return ViewPolicy {
            main: false,
            cpu: false,
            sensors: false,
        };
    }
    ViewPolicy {
        main: shown && !occluded,
        cpu: (shown && focused && !occluded && page == 0) || hud,
        sensors: (shown && !occluded) || hud,
    }
}
fn cpu_sample_fresh(sampled_at: u64, now: u64) -> bool {
    sampled_at > 0 && sampled_at <= now && now.saturating_sub(sampled_at) < 3000
}
fn sensor_result_current(
    generation: u64,
    current: u64,
    visible: bool,
    completed: Instant,
    now: Instant,
) -> bool {
    generation == current
        && visible
        && now.saturating_duration_since(completed) < Duration::from_secs(3)
}
fn sensor_status(sample: &crate::system_integration::SensorSnapshot) -> String {
    let mut parts = vec![trf(
        "{0} · {1} readings · {2}",
        &[
            &sample.provider,
            &sample.readings.to_string(),
            &sample.gpu_label,
        ],
    )];
    if let Some(error) = sample.error.as_ref().filter(|s| !s.is_empty()) {
        parts.push(error.clone());
    }
    if !sample.freshness_note.is_empty() {
        parts.push(sample.freshness_note.clone());
    }
    parts.join("\n")
}
// Window focus can change inside a native file dialog while Controller::action
// is borrowed. Keep visibility independently borrowable to avoid RefCell
// reentrancy and to pause CPU reads immediately while that dialog is open.
struct TelemetryView {
    ui: slint::Weak<AppWindow>,
    writer: Writer,
    focused: Cell<bool>,
    occluded: Cell<bool>,
    last: Cell<Option<bool>>,
    sensor_visible: std::sync::Arc<std::sync::atomic::AtomicBool>,
    sensor_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    hud_visible: Cell<bool>,
    main_visible: Cell<bool>,
    fullscreen: Cell<bool>,
}
impl TelemetryView {
    fn publish(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let shown = ui
            .window()
            .with_winit_window(|w| {
                w.is_visible().unwrap_or(true) && !w.is_minimized().unwrap_or(false)
            })
            .unwrap_or(false);
        let policy = view_policy(
            shown,
            self.focused.get(),
            self.occluded.get(),
            ui.get_page(),
            self.hud_visible.get(),
            self.fullscreen.get(),
        );
        let visible = policy.cpu;
        let main_visible = policy.main;
        ui.set_render_active(main_visible);
        let sensors_changed = self
            .sensor_visible
            .swap(policy.sensors, std::sync::atomic::Ordering::SeqCst)
            != policy.sensors;
        if sensors_changed {
            // A late queued result may not cross hide/resume, even when both
            // transitions occur before the sensor callback reaches this thread.
            self.sensor_generation
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        let main_changed = self.main_visible.replace(main_visible) != main_visible;
        if main_changed || sensors_changed {
            // Defer Controller access: native focus/resize events can arrive
            // inside a file dialog or show/hide while it is already borrowed.
            slint::Timer::single_shot(Duration::ZERO, move || {
                CONTROL.with(|cell| {
                    if let Some(control) = cell.borrow().as_ref() {
                        let mut control = control.borrow_mut();
                        if control.sensor_generation
                            != control
                                .view
                                .sensor_generation
                                .load(std::sync::atomic::Ordering::SeqCst)
                        {
                            control.sensor_sample = None;
                            control.sensor_received = None;
                        }
                        if control.view.main_visible.get() {
                            control.ui_dirty = true;
                            control.cores_dirty = true;
                            control.render_snapshot(now_ms());
                        } else {
                            control.motion.stop();
                            if let Some(ui) = control.ui.upgrade() {
                                ui.set_reveal(1.);
                                ui.set_pulse(0.);
                            }
                        }
                    }
                });
            });
        }
        if !shown && !self.hud_visible.get() {
            ui.set_sensors_available(false);
        }
        if self.last.get() != Some(visible) {
            match self.writer.try_send(Command {
                kind: VIEW,
                flag: visible,
                ..Default::default()
            }) {
                Ok(()) => self.last.set(Some(visible)),
                Err(e) => {
                    ui.set_status(trf("CPU view notification: {0}", &[&e.to_string()]).into());
                    ui.set_error(true);
                }
            }
        }
    }
}
struct Controller {
    ui: slint::Weak<AppWindow>,
    writer: Writer,
    state: Snapshot,
    selected: String,
    page: usize,
    query: String,
    candidate_query: String,
    candidate_page: usize,
    plan_page: usize,
    plan_ids: Vec<String>,
    icons: HashMap<String, Image>,
    motion: slint::Timer,
    view: Rc<TelemetryView>,
    pro: ProView,
    session_started: Instant,
    preferences: crate::preferences::Preferences,
    integration: Option<crate::system_integration::SystemIntegration>,
    hotkey_presentation: RefCell<Option<HotkeyPresentation>>,
    sensor_request: mpsc::SyncSender<()>,
    sensor_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    running_candidates: Option<Vec<Game>>,
    discovered_once: bool,
    edit_plan: String,
    operation_busy: bool,
    transient: RefCell<Option<(String, bool, Instant)>>,
    last_tray: Cell<Option<bool>>,
    hud: Option<crate::TelemetryHud>,
    hud_setup: Option<slint::JoinHandle<()>>,
    hud_generation: u64,
    hud_ready: bool,
    elevated_retry: Option<crate::power_tools::Operation>,
    models: UiModels,
    history_dirty: bool,
    games_dirty: bool,
    plans_dirty: bool,
    cores_dirty: bool,
    core_sample_was_fresh: bool,
    ui_dirty: bool,
    connected: bool,
    connection_error: Option<String>,
    sensor_sample: Option<crate::system_integration::SensorSnapshot>,
    sensor_received: Option<Instant>,
    sensor_generation: u64,
    show_requested: Cell<bool>,
    topology: crate::cpu_topology::CpuTopology,
    core_keys: Vec<(u32, u32)>,
    core_order: Vec<usize>,
    requests: RefCell<Requests>,
}
include!("ui_pro.rs");
include!("ui_cpu.rs");
include!("ui_requests.rs");

#[derive(Default)]
struct UiModels {
    cores: Rc<VecModel<CoreRow>>,
    core_sections: Rc<VecModel<CoreSection>>,
    l3_groups: Rc<VecModel<i32>>,
    ccx_groups: Rc<VecModel<i32>>,
    history: Rc<VecModel<HistoryRow>>,
    games: Rc<VecModel<GameRow>>,
    candidates: Rc<VecModel<GameRow>>,
    running: Rc<VecModel<bool>>,
    plans: Rc<VecModel<SharedString>>,
}
impl UiModels {
    fn bind(&self, ui: &AppWindow) {
        ui.set_cores(self.cores.clone().into());
        ui.set_core_sections(self.core_sections.clone().into());
        ui.set_l3_groups(self.l3_groups.clone().into());
        ui.set_ccx_groups(self.ccx_groups.clone().into());
        ui.set_history(self.history.clone().into());
        ui.set_games(self.games.clone().into());
        ui.set_candidates(self.candidates.clone().into());
        ui.set_games_running(self.running.clone().into());
        ui.set_plan_options(self.plans.clone().into());
    }
}

/// Preserve model identity and notify Slint only about changed, added or removed
/// rows. Unchanged snapshots must not recreate repeaters, textures or selection.
fn sync_rows<T: Clone + PartialEq + 'static>(
    model: &VecModel<T>,
    rows: impl IntoIterator<Item = T>,
) -> usize {
    let mut count = 0;
    let mut changes = 0;
    for (index, row) in rows.into_iter().enumerate() {
        match model.row_data(index) {
            Some(previous) if previous == row => {}
            Some(_) => {
                model.set_row_data(index, row);
                changes += 1;
            }
            None => {
                model.push(row);
                changes += 1;
            }
        }
        count = index + 1;
    }
    while model.row_count() > count {
        model.remove(model.row_count() - 1);
        changes += 1;
    }
    changes
}

fn invalidate_cpu_display(ui: &AppWindow, rows: &VecModel<CoreRow>, reason: &str) {
    // Keep known hardware identities/tints, but never leave disconnected
    // percentages and a "Live" footer presented as fresh measurements.
    let invalid: Vec<_> = (0..rows.row_count())
        .filter_map(|i| rows.row_data(i))
        .map(|mut row| {
            let identity = row.label.split_whitespace().next().unwrap_or("CPU");
            row.label = format!("{identity} —").into();
            row.activity = -1.;
            row.detail = trf("Windows CPU activity unavailable · {0}", &[&tr(reason)]).into();
            row
        })
        .collect();
    sync_rows(rows, invalid);
    ui.set_core_info("".into());
    ui.set_telemetry(trf("CPU telemetry unavailable · {0}", &[&tr(reason)]).into());
}
fn hwnd(ui: &AppWindow) -> usize {
    match ui
        .window()
        .window_handle()
        .window_handle()
        .map(|h| h.as_raw())
    {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get() as usize,
        _ => 0,
    }
}
fn image(bytes: &[u8]) -> Image {
    if bytes.is_empty() {
        return Image::default();
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let Ok(mut reader) = decoder.read_info() else {
        return Image::default();
    };
    let Some(size) = reader.output_buffer_size() else {
        return Image::default();
    };
    if size > 4_000_000 {
        return Image::default();
    }
    let mut data = vec![0; size];
    let Ok(info) = reader.next_frame(&mut data) else {
        return Image::default();
    };
    let mut rgba = vec![];
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(&data[..info.buffer_size()]),
        png::ColorType::Rgb => {
            for p in data[..info.buffer_size()].chunks_exact(3) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        _ => return Image::default(),
    };
    Image::from_rgba8(
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            &rgba,
            info.width,
            info.height,
        ),
    )
}
impl Controller {
    fn language_changed(&mut self, index: i32) {
        let Some(requested) = localization::code(index) else {
            return;
        };
        self.apply_language(requested, index, true);
    }
    fn refresh_system_language(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(requested) = localization::code(ui.get_language_index()) else {
            return;
        };
        if let Some(locale) = localization::system_refresh_target(requested) {
            // A Windows change updates only the effective display locale. Keep
            // the System default preference and do not rewrite appearance.json.
            self.apply_language(locale, 0, false);
        }
    }
    fn apply_language(&mut self, requested: &str, index: i32, persist: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        if let Err(error) = localization::select(requested) {
            ui.set_language_index(1);
            ui.set_rtl(false);
            self.notice(&trf("Language selection failed: {0}", &[&error]), true);
            return;
        }
        if persist {
            self.preferences.language = requested.into();
        }
        ui.set_language_index(index);
        ui.set_language_names(slint::ModelRc::new(VecModel::from(localization::names())));
        ui.set_rtl(localization::is_rtl());
        ui.set_ui_font(
            localization::interface_font(&crate::system_integration::preferred_font()).into(),
        );
        let save_error = if persist {
            self.preferences
                .save()
                .err()
                .map(|error| trf("Preferences: {0}", &[&error]))
        } else {
            None
        };
        let tray_error = self
            .integration
            .as_ref()
            .and_then(|integration| integration.set_tray_language().err());
        // Reuse the existing models; only their translated presentation rows
        // change. No engine command, process identity or plan name is rewritten.
        self.core_keys.clear();
        self.cores_dirty = true;
        self.history_dirty = true;
        self.games_dirty = true;
        self.plans_dirty = true;
        self.ui_dirty = true;
        self.bind_cpu_topology(&ui);
        self.render_snapshot(now_ms());
        self.render_hotkey_status();
        self.update_hud();
        if let Some(error) = save_error.or(tray_error) {
            self.notice(&error, true);
        } else if persist {
            self.notice("Language changed.", false);
        }
    }
    fn render_pro(&self, now: u64) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.set_cpu_trend_path(self.pro.cpu.path(now).into());
        ui.set_cpu_p_trend_path(if self.topology.is_hybrid {
            self.pro.cpu.class_path(now, "P").into()
        } else {
            "".into()
        });
        ui.set_cpu_e_trend_path(if self.topology.is_hybrid {
            self.pro.cpu.class_path(now, "E").into()
        } else {
            "".into()
        });
        ui.set_cpu_switch_path(pro_view::switch_markers(&self.state.history, now).into());
        ui.set_cpu_trend_label(self.pro.cpu.label().into());
        ui.set_cpu_trend_available(self.pro.cpu.available(now));
        ui.set_session_gaming(self.pro.session.gaming().into());
        ui.set_session_default(self.pro.session.default_plan().into());
        ui.set_session_gaming_ratio(self.pro.session.ratio());
        ui.set_session_donut_path(pro_view::donut_path(self.pro.session.ratio()).into());
        ui.set_session_accounting_label(self.pro.session.label().into());
        ui.set_energy_savings(tr("Unavailable · no energy meter").into());
        ui.set_energy_savings_available(false);
    }
    fn send(&self, c: Command) {
        // A bounded queue keeps pipe backpressure and serialization off the UI.
        if let Err(e) = self.writer.try_send(c) {
            self.notice(&trf("Engine command queue: {0}", &[&e.to_string()]), true);
        }
    }
    fn simple(&self, kind: u32) {
        if kind == TOGGLE && self.requests.borrow().pending.is_some() {
            self.notice(
                "Wait for the pending engine change before toggling monitoring.",
                true,
            );
            return;
        }
        self.send(Command {
            kind,
            ..Default::default()
        })
    }
    fn notice(&self, s: &str, error: bool) {
        *self.transient.borrow_mut() = Some((s.into(), error, Instant::now()));
        if let Some(ui) = self.ui.upgrade() {
            ui.set_status(tr(s).into());
            ui.set_error(error);
        }
    }
    fn save(&self, c: Config) {
        self.try_save(c);
    }
    fn animate(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        if !self.view.main_visible.get() || ui.get_reduced_motion() || ui.get_suspended() {
            self.motion.stop();
            ui.set_reveal(1.);
            ui.set_pulse(0.);
            return;
        }
        let weak = self.ui.clone();
        let start = Instant::now();
        ui.set_reveal(0.);
        ui.set_pulse(1.);
        // A short opacity reveal matches the 120–150ms control transitions.
        // The 16ms timer stops after 140ms; no persistent animation polling.
        self.motion.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                if let Some(ui) = weak.upgrade() {
                    let progress = (start.elapsed().as_secs_f32() / 0.14).clamp(0., 1.);
                    ui.set_reveal(1. - (1. - progress).powi(3));
                    ui.set_pulse(1. - progress);
                    if progress >= 1. {
                        CONTROL.with(|s| {
                            if let Some(c) = s.borrow().as_ref() {
                                c.borrow().motion.stop();
                            }
                        });
                    }
                }
            },
        );
    }
    fn row(&mut self, g: &Game) -> GameRow {
        let icon = self
            .icons
            .entry(g.id.clone())
            .or_insert_with(|| image(&g.icon))
            .clone();
        GameRow {
            id: g.id.clone().into(),
            name: g.name.clone().into(),
            processes: g.processes.join(" · ").into(),
            icon,
            selected: g.id == self.selected,
        }
    }
    fn games(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let c = self.state.config.clone().unwrap_or_else(defaults);
        let matching: Vec<_> = c
            .games
            .iter()
            .filter(|g| {
                format!("{} {}", g.name, g.processes.join(" "))
                    .to_lowercase()
                    .contains(&self.query.to_lowercase())
            })
            .cloned()
            .collect();
        let pages = matching.len().div_ceil(3).max(1);
        self.page = self.page.min(pages - 1);
        if !matching
            .iter()
            .skip(self.page * 3)
            .take(3)
            .any(|g| g.id == self.selected)
        {
            self.selected.clear();
        }
        let shown: Vec<_> = matching
            .iter()
            .skip(self.page * 3)
            .take(3)
            .cloned()
            .collect();
        sync_rows(
            &self.models.running,
            pro_view::visible_running(&shown, &self.state),
        );
        let rows: Vec<_> = shown.iter().map(|g| self.row(g)).collect();
        sync_rows(&self.models.games, rows);
        ui.set_page_label(format!("{} / {pages}", self.page + 1).into());
        ui.set_can_back(self.page > 0);
        ui.set_can_next(self.page + 1 < pages);
        ui.set_has_selection(c.games.iter().any(|g| g.id == self.selected));
        let candidates: Vec<_> = self
            .running_candidates
            .as_ref()
            .unwrap_or(&self.state.discovered)
            .iter()
            .filter(|g| {
                format!("{} {}", g.name, g.processes.join(" "))
                    .to_lowercase()
                    .contains(&self.candidate_query.to_lowercase())
            })
            .cloned()
            .collect();
        let pages = candidates.len().div_ceil(6).max(1);
        self.candidate_page = self.candidate_page.min(pages - 1);
        ui.set_candidate_page_label(format!("{} / {pages}", self.candidate_page + 1).into());
        ui.set_candidate_back(self.candidate_page > 0);
        ui.set_candidate_next(self.candidate_page + 1 < pages);
        let rows: Vec<_> = candidates
            .iter()
            .skip(self.candidate_page * 6)
            .take(6)
            .map(|g| self.row(g))
            .collect();
        sync_rows(&self.models.candidates, rows);
        // Retain decoded icons only for the displayed workspace/picker pages.
        // The saved compressed icons stay in Config; paging decodes on demand.
        self.icons.retain(|id, _| {
            shown.iter().any(|g| &g.id == id)
                || candidates
                    .iter()
                    .skip(self.candidate_page * 6)
                    .take(6)
                    .any(|g| &g.id == id)
        });
        self.games_dirty = false;
    }
    fn plans(&mut self, query: &str, reveal_saved: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        let mut all_plans = self.state.plans.clone();
        if ui.get_plan_kind() == 2 {
            all_plans.insert(
                0,
                Plan {
                    guid: String::new(),
                    name: tr("Use global Gaming plan").into(),
                },
            );
        }
        let plans: Vec<_> = all_plans
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query.to_lowercase())
                    || p.guid.contains(&query.to_lowercase())
            })
            .collect();
        let saved = self
            .state
            .config
            .as_ref()
            .map(|c| {
                if ui.get_plan_kind() == 2 {
                    self.edit_plan.as_str()
                } else if ui.get_plan_kind() == 0 {
                    c.gaming.as_str()
                } else {
                    c.default_plan.as_str()
                }
            })
            .unwrap_or("");
        // Opening or clearing a selector reveals the saved plan even when it is
        // beyond the first page. Paging and literal filtering never change it.
        if reveal_saved && query.is_empty() {
            self.plan_page = plans
                .iter()
                .position(|p| p.guid.eq_ignore_ascii_case(saved))
                .map(|i| i / 4)
                .unwrap_or(0);
        }
        let pages = plans.len().div_ceil(4).max(1);
        self.plan_page = self.plan_page.min(pages - 1);
        let shown: Vec<_> = plans.into_iter().skip(self.plan_page * 4).take(4).collect();
        self.plan_ids = shown.iter().map(|p| p.guid.clone()).collect();
        sync_rows(
            &self.models.plans,
            shown.iter().map(|p| SharedString::from(&p.name)),
        );
        ui.set_plan_index(
            self.plan_ids
                .iter()
                .position(|p| p.eq_ignore_ascii_case(saved))
                .unwrap_or(0) as i32,
        );
        ui.set_plan_page_label(format!("{} / {pages}", self.plan_page + 1).into());
        ui.set_plan_back(self.plan_page > 0);
        ui.set_plan_next(self.plan_page + 1 < pages);
    }
    fn bulk(&self, s: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let names = self
            .state
            .config
            .as_ref()
            .map(|c| c.names())
            .unwrap_or_default();
        let parsed = parse_bulk(s, &names);
        ui.set_bulk_valid(!parsed.valid.is_empty());
        ui.set_bulk_error(!parsed.errors.is_empty());
        let message = if s.trim().is_empty() {
            tr("Separate .exe names with commas. Paths are reduced to file names.").into()
        } else {
            trf(
                "{0} valid · {1} duplicate · {2} invalid{3}",
                &[
                    &parsed.valid.len().to_string(),
                    &parsed.duplicates.to_string(),
                    &parsed.errors.len().to_string(),
                    &parsed
                        .errors
                        .first()
                        .map(|s| format!(" · {s}"))
                        .unwrap_or_default(),
                ],
            )
        };
        ui.set_validation(message.into());
    }
    fn update(&mut self, mut s: Snapshot) {
        let Some(ui) = self.ui.upgrade() else { return };
        if s.retired {
            let _ = ui.hide();
            let _ = slint::quit_event_loop();
            return;
        }
        if s.suspended {
            for core in &mut s.cores {
                core.load = -1.;
            }
            s.cpu_sampled_at_ms = 0;
            s.telemetry = "CPU sampling suspended for full-screen game".into();
        }
        let observed_at = now_ms();
        self.pro.observe(
            &s,
            self.session_started.elapsed().as_millis() as u64,
            observed_at,
            self.view.last.get() == Some(true),
        );
        let changed = self.state.config != s.config;
        let ownership_changed = self.state.observer != s.observer;
        self.history_dirty |= self.state.history != s.history;
        // Exact O(n) comparison allocates nothing and includes parked state,
        // identity and sample freshness. A load-only hash can silently retain
        // stale timestamps or miss a real change through a hash collision.
        self.cores_dirty |=
            self.state.cores != s.cores || self.state.cpu_sampled_at_ms != s.cpu_sampled_at_ms;
        self.cores_dirty |=
            cpu_sample_fresh(s.cpu_sampled_at_ms, observed_at) != self.core_sample_was_fresh;
        self.games_dirty |= changed
            || self.state.discovered != s.discovered
            || self.state.running != s.running
            || self.state.monitoring != s.monitoring
            || self.state.ready != s.ready;
        self.plans_dirty |= changed || self.state.plans != s.plans;
        if changed {
            self.icons.clear();
        }
        self.state = s;
        self.connected = true;
        self.connection_error = None;
        self.observe_request(&self.state);
        if self.view.fullscreen.replace(self.state.suspended) != self.state.suspended {
            // render_snapshot is intentionally gated while suspended; publish
            // the one state transition first so animations stop immediately.
            ui.set_suspended(self.state.suspended);
            if self.state.suspended {
                self.motion.stop();
                ui.set_reveal(1.);
                ui.set_pulse(0.);
                ui.set_cpu_warming(false);
            }
            self.view.publish();
        }
        self.ui_dirty = true;
        if self.last_tray.get() != Some(self.state.monitoring) {
            if let Some(integration) = &self.integration {
                if integration.set_tray(true, self.state.monitoring).is_ok() {
                    self.last_tray.set(Some(self.state.monitoring));
                    ui.set_tray_available(true);
                } else {
                    ui.set_tray_available(false);
                }
            }
        }
        self.update_hud();
        if ownership_changed {
            // Observers' VIEW commands do not control the engine. Re-announce
            // the actual foreground view when this window gains ownership.
            self.view.last.set(None);
            self.view.publish();
        }
        self.render_snapshot(observed_at);
    }
    fn render_snapshot(&mut self, observed_at: u64) {
        if !self.view.main_visible.get() || !self.ui_dirty {
            return;
        }
        let Some(ui) = self.ui.upgrade() else { return };
        self.ui_dirty = false;
        self.prepare_cpu_layout(&ui);
        let s = &self.state;
        if s.suspended {
            self.motion.stop();
            ui.set_reveal(1.);
            ui.set_pulse(0.);
        }
        if !s.backend.is_empty() {
            ui.set_backend(s.backend.clone().into());
        }
        ui.set_connected(self.connected);
        ui.set_monitoring(s.monitoring);
        ui.set_startup(s.startup);
        ui.set_error(s.error);
        ui.set_legacy(s.legacy);
        ui.set_observer(s.observer);
        ui.set_suspended(s.suspended);
        ui.set_state_label(
            tr(if s.observer {
                "Observer"
            } else if s.dry_run {
                "Preview"
            } else if s.monitoring && s.ready {
                "Monitoring"
            } else if s.monitoring {
                "Connecting"
            } else {
                "Paused"
            })
            .into(),
        );
        ui.set_active_name(s.active_name.clone().into());
        ui.set_active_guid(s.active_guid.clone().into());
        ui.set_status(tr(&s.status).into());
        if let Some((text, error, at)) = self
            .transient
            .borrow()
            .as_ref()
            .filter(|(_, _, at)| at.elapsed() < Duration::from_secs(8))
        {
            let _ = at;
            ui.set_status(tr(text).into());
            ui.set_error(*error);
        }
        ui.set_telemetry(tr(&s.telemetry).into());
        ui.set_cpu_warming(
            self.connected
                && !s.suspended
                && self.view.last.get() == Some(true)
                && s.cpu_sampled_at_ms == 0
                && s.telemetry.contains("warming up a fresh"),
        );
        ui.set_running(!s.running.is_empty());
        ui.set_process_status_available(self.connected && s.monitoring && s.ready);
        ui.set_detector_label(pro_view::detector_label(s).into());
        ui.set_process_summary(if !s.monitoring {
            tr("Monitoring paused").into()
        } else if s.running.is_empty() {
            tr("No watched games running").into()
        } else {
            s.running.join(", ").into()
        });
        if let Some(c) = &s.config {
            ui.set_auto_switch_battery(c.battery_guard.unwrap_or(false));
            ui.set_auto_apply_affinity(c.auto_affinity.unwrap_or(true));
            ui.set_foreground_only(c.foreground_only);
            ui.set_auto_time_rule(c.time_rule);
            ui.set_automation_start(c.time_start.clone().into());
            ui.set_automation_end(c.time_end.clone().into());
            ui.set_overlay(c.overlay);
            ui.set_reduced_motion(c.reduced_motion);
            if c.reduced_motion {
                self.motion.stop();
                ui.set_reveal(1.);
                ui.set_pulse(0.);
            }
            let name = |guid: &str| {
                s.plans
                    .iter()
                    .find(|p| p.guid.eq_ignore_ascii_case(guid))
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| tr("Missing plan — select another").into())
            };
            ui.set_gaming_name(name(&c.gaming).into());
            ui.set_default_name(name(&c.default_plan).into());
        }
        if self.cores_dirty {
            self.core_sample_was_fresh = cpu_sample_fresh(s.cpu_sampled_at_ms, observed_at);
            sync_rows(
                &self.models.cores,
                self.core_order.iter().map(|&i| {
                    topology_core_row(
                        &self.topology,
                        &s.cores[i],
                        self.core_sample_was_fresh,
                        s.cpu_sampled_at_ms,
                        observed_at,
                    )
                }),
            );
            self.cores_dirty = false;
        }
        if self.history_dirty {
            let max = s.history.iter().map(|e| e.duration_ms).fold(1., f64::max);
            let first = s.history.first().map(|e| e.time_ms).unwrap_or(0);
            let span = s
                .history
                .last()
                .map(|e| e.time_ms.saturating_sub(first))
                .unwrap_or(0)
                .max(1);
            let mut path = String::new();
            for (i, e) in s.history.iter().enumerate() {
                let x = 4. + (e.time_ms.saturating_sub(first) as f64 / span as f64) * 588.;
                let y = 26. - e.duration_ms / max * 22.;
                path.push_str(&format!(
                    "{} {x:.2} {y:.2} ",
                    if i == 0 { "M" } else { "L" }
                ));
            }
            for e in &s.history {
                let x = 4. + (e.time_ms.saturating_sub(first) as f64 / span as f64) * 588.;
                path.push_str(&format!("M {x:.2} 28 L {x:.2} 32 "));
            }
            ui.set_timeline_path(path.into());
            sync_rows(
                &self.models.history,
                pro_view::log_rows(&s.history)
                    .into_iter()
                    .map(|e| HistoryRow {
                        label: e.label.into(),
                        detail: e.detail.into(),
                        level: (e.duration_ms / max) as f32,
                        gaming: e.gaming,
                    }),
            );
            self.history_dirty = false;
        }
        self.render_pro(observed_at);
        self.render_sensors();
        if let Some(reason) = &self.connection_error {
            ui.set_cpu_switch_path("".into());
            ui.set_cpu_trend_label(tr("CPU graph unavailable · engine disconnected").into());
            ui.set_detector_label(tr("Windows process detector · engine disconnected").into());
            ui.set_error(true);
            ui.set_status(trf("Engine disconnected: {0}", &[reason]).into());
            invalidate_cpu_display(&ui, &self.models.cores, "engine disconnected");
        }
        if self.games_dirty {
            self.games();
            self.bulk(&ui.get_bulk_text());
        }
        if self.plans_dirty {
            if ui.get_plan_kind() >= 0 {
                self.plans(&ui.get_plan_search(), false);
            }
            self.plans_dirty = false;
        }
    }
    fn action(&mut self, action: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let mut c = self.state.config.clone().unwrap_or_else(defaults);
        match action {
            "toggle" => {
                self.simple(TOGGLE);
                self.animate()
            }
            "startup" => self.simple(STARTUP),
            "handover" | "claim" => {
                self.request(
                    RequestKind::Handover,
                    Command {
                        kind: if action == "handover" { LEGACY } else { CLAIM },
                        ..Default::default()
                    },
                );
            }
            "refresh" => self.simple(REFRESH),
            "browse" => {
                self.browse_exes();
            }
            "discover" => {
                self.running_candidates = None;
                ui.set_candidate_source(
                    tr("Installed game libraries · choose a reviewed executable").into(),
                );
                self.candidate_query.clear();
                self.candidate_page = 0;
                ui.set_modal(2);
                self.simple(SCAN)
            }
            "remove" => {
                if !c.games.iter().any(|g| g.id == self.selected) {
                    return;
                }
                c.games.retain(|g| g.id != self.selected);
                self.try_save(c);
            }
            "previous" => {
                self.page = self.page.saturating_sub(1);
                self.games()
            }
            "next" => {
                self.page += 1;
                self.games()
            }
            "candidate-previous" => {
                self.candidate_page = self.candidate_page.saturating_sub(1);
                self.games()
            }
            "candidate-next" => {
                self.candidate_page += 1;
                self.games()
            }
            "plan-previous" => {
                self.plan_page = self.plan_page.saturating_sub(1);
                self.plans(&ui.get_plan_search(), false)
            }
            "plan-next" => {
                self.plan_page += 1;
                self.plans(&ui.get_plan_search(), false)
            }
            "bulk" => {
                let p = parse_bulk(&ui.get_bulk_text(), &c.names());
                if !p.valid.is_empty() {
                    c.add_names(&p.valid);
                    if self.try_save(c) {
                        self.saved_draft(SavedDraft::Bulk {
                            submitted: ui.get_bulk_text().into(),
                            remaining: p.errors.join(", "),
                        });
                    }
                }
            }
            "overlay" => {
                c.overlay = !c.overlay;
                self.save(c)
            }
            "motion" => {
                c.reduced_motion = !c.reduced_motion;
                self.save(c)
            }
            "edit-game" => {
                if let Some(g) = c.games.iter().find(|g| g.id == self.selected) {
                    self.edit_plan = g.power_plan.clone();
                    ui.set_game_plan_name(self.plan_name(&self.edit_plan).into());
                    ui.set_edit_name(g.name.clone().into());
                    ui.set_selected_processes(g.processes.join(", ").into());
                    ui.set_affinity_text(g.affinity.clone().into());
                    ui.set_priority_index(
                        [0, 0x20, 0x8000, 0x80]
                            .iter()
                            .position(|p| *p == g.priority)
                            .unwrap_or(0) as i32,
                    );
                    ui.set_modal(3)
                }
            }
            "save-game" => {
                if let Some(g) = c.games.iter_mut().find(|g| g.id == self.selected) {
                    g.affinity = ui.get_affinity_text().trim().to_lowercase();
                    g.power_plan = self.edit_plan.clone();
                    g.priority =
                        [0, 0x20, 0x8000, 0x80][ui.get_priority_index().clamp(0, 3) as usize];
                    if let Err(e) = crate::engine::affinity(&g.affinity, &self.state.cores) {
                        self.notice(&e, true);
                        return;
                    }
                    if self.try_save(c) {
                        self.saved_draft(SavedDraft::Profile {
                            id: self.selected.clone(),
                            affinity: ui.get_affinity_text().into(),
                            priority: ui.get_priority_index(),
                            plan: self.edit_plan.clone(),
                        });
                    }
                }
            }
            "profile-reset" => {
                ui.set_affinity_text("".into());
                ui.set_priority_index(0);
                self.notice("Affinity and priority reset to Windows defaults in this draft. Save profile to apply.", false);
            }
            "copy-guid" => {
                if let Err(e) = win::copy(hwnd(&ui), &self.state.active_guid) {
                    self.notice(&e, true)
                } else {
                    self.notice("Active plan GUID copied.", false)
                }
            }
            "copy-discord" => {
                if let Err(e) = win::copy(hwnd(&ui), "https://discord.com/users/176078095957098497")
                {
                    self.notice(&e, true)
                } else {
                    self.notice("Developer profile link copied.", false)
                }
            }
            "discord" => {
                if let Err(e) = win::open_url("discord://-/users/176078095957098497")
                    .or_else(|_| win::open_url("https://discord.com/users/176078095957098497"))
                {
                    self.notice(&e, true)
                }
            }
            "licenses" => {
                let p = data_dir().join("THIRD-PARTY-NOTICES.txt");
                match std::fs::write(&p, include_str!("../THIRD-PARTY-NOTICES.txt")) {
                    Ok(()) => {
                        let _ = win::open_url(&p.to_string_lossy());
                    }
                    Err(e) => self.notice(&e.to_string(), true),
                }
            }
            "twitch" => {
                if let Err(e) = win::open_url("https://twitch.tv/mikimeows") {
                    self.notice(&e, true)
                }
            }
            "log" => {
                let p = data_dir().join("native.log");
                if !p.exists() {
                    let _ = std::fs::write(&p, "");
                }
                let _ = win::open_url(&p.to_string_lossy());
            }
            _ => {}
        }
    }
}
fn hydrate_telemetry(
    snapshot: &mut Snapshot,
    dry: bool,
    ring: &mut Option<win::TelemetryRing>,
    last_topology: &mut Vec<Core>,
) -> Result<(), String> {
    crate::runtime_scope::verify_peer(dry, snapshot.dry_run)?;
    if snapshot.telemetry_sequence > 0 {
        if ring.is_none() {
            *ring = win::TelemetryRing::open(false).ok();
        }
        let decoded = ring
            .as_ref()
            .ok_or_else(|| "Shared telemetry unavailable".to_owned())
            .and_then(|ring| ring.read(snapshot.telemetry_sequence))
            .and_then(|data| Snapshot::decode(data.as_slice()).map_err(|e| e.to_string()));
        match decoded {
            Ok(telemetry) => {
                // A mismatched ring is a session error, not a missing sensor.
                crate::runtime_scope::verify_peer(dry, telemetry.dry_run)?;
                snapshot.cores = telemetry.cores;
                snapshot.history = telemetry.history;
            }
            Err(_) => {
                *ring = None;
                if last_topology.is_empty() {
                    *last_topology = win::cores();
                }
                snapshot.cores.clone_from(last_topology);
                for core in &mut snapshot.cores {
                    core.load = -1.;
                }
                snapshot.cpu_sampled_at_ms = 0;
                snapshot.telemetry =
                    "CPU telemetry unavailable · reconnecting shared memory".into();
            }
        }
    }
    if !snapshot.cores.is_empty() {
        last_topology.clone_from(&snapshot.cores);
    }
    Ok(())
}

pub fn run(
    dry: bool,
    paused: bool,
    instance: &crate::gui_instance::GuiInstance,
) -> Result<(), String> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(|attributes| {
            use winit::platform::windows::WindowAttributesExtWindows;
            if CREATING_HUD.with(Cell::get) {
                // Cache non-activation in winit itself: adding NOACTIVATE only
                // after show() is too late and native-only flags are discarded
                // when winit reapplies its style during later hide/show cycles.
                attributes
                    .with_active(false)
                    .with_skip_taskbar(true)
                    .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
            } else {
                attributes
            }
        })
        .select()
        .map_err(|error| error.to_string())?;
    let ui = AppWindow::new().map_err(|e| e.to_string())?;
    let preferences = crate::preferences::Preferences::load();
    let preview_language = std::env::var("GPPS_PREVIEW_LANGUAGE").ok();
    let requested_language =
        localization::preview_request(&preferences.language, dry, preview_language.as_deref());
    let language_error = localization::select(requested_language).err();
    ui.set_language_index(if language_error.is_some() {
        1
    } else {
        localization::index(requested_language)
    });
    ui.set_language_names(slint::ModelRc::new(VecModel::from(localization::names())));
    ui.set_rtl(localization::is_rtl());
    ui.set_language_results(slint::ModelRc::new(VecModel::from(localization::filter(
        "",
    ))));
    let topology = crate::system_integration::cpu_topology();
    let mut pro = ProView::default();
    pro.cpu.set_topology(&topology);
    let (sensor_request, sensor_requests) = mpsc::sync_channel::<()>(1);
    let sensor_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (writer, commands) = mpsc::sync_channel::<Command>(256);
    let view = Rc::new(TelemetryView {
        ui: ui.as_weak(),
        writer: writer.clone(),
        focused: Cell::new(false),
        occluded: Cell::new(false),
        last: Cell::new(None),
        sensor_visible: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        sensor_generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        hud_visible: Cell::new(false),
        main_visible: Cell::new(false),
        fullscreen: Cell::new(false),
    });
    let control = Rc::new(RefCell::new(Controller {
        ui: ui.as_weak(),
        writer: writer.clone(),
        // Keep preview-only operation guards effective before the first IPC
        // snapshot and after a failed connection, not just after engine ready.
        state: Snapshot {
            dry_run: dry,
            ..Default::default()
        },
        selected: String::new(),
        page: 0,
        query: String::new(),
        candidate_query: String::new(),
        candidate_page: 0,
        plan_page: 0,
        plan_ids: vec![],
        icons: HashMap::new(),
        motion: slint::Timer::default(),
        view: view.clone(),
        pro,
        session_started: Instant::now(),
        preferences,
        integration: None,
        hotkey_presentation: RefCell::new(None),
        sensor_request,
        sensor_stop: sensor_stop.clone(),
        running_candidates: None,
        discovered_once: false,
        edit_plan: String::new(),
        operation_busy: false,
        transient: RefCell::new(None),
        last_tray: Cell::new(None),
        hud: None,
        hud_setup: None,
        hud_generation: 0,
        hud_ready: false,
        elevated_retry: None,
        models: UiModels::default(),
        history_dirty: true,
        games_dirty: true,
        plans_dirty: true,
        cores_dirty: true,
        core_sample_was_fresh: false,
        ui_dirty: true,
        connected: false,
        connection_error: None,
        sensor_sample: None,
        sensor_received: None,
        sensor_generation: 0,
        show_requested: Cell::new(false),
        topology,
        core_keys: Vec::new(),
        core_order: Vec::new(),
        requests: RefCell::new(Requests::default()),
    }));
    control.borrow().models.bind(&ui);
    control.borrow().bind_cpu_topology(&ui);
    CONTROL.with(|c| *c.borrow_mut() = Some(control.clone()));
    let weak = ui.as_weak();
    instance.set_show_handler(move || {
        weak.upgrade_in_event_loop(|_| {
            CONTROL.with(|cell| {
                if let Some(control) = cell.borrow().as_ref() {
                    let control = control.borrow();
                    control.show_window();
                    if let Some(integration) = &control.integration {
                        if let Err(error) = integration.notify_already_running() {
                            control.notice(
                                &trf("Already running; tray notice: {0}", &[&error.to_string()]),
                                true,
                            );
                        }
                    }
                }
            });
        })
        .map_err(|e| e.to_string())
    })?;
    install_pro(&ui, &control, sensor_requests, sensor_stop);
    if let Some(error) = language_error {
        control
            .borrow()
            .notice(&trf("Language selection failed: {0}", &[&error]), true);
    }
    let c = control.clone();
    ui.on_language_changed(move |index| c.borrow_mut().language_changed(index));
    let weak = ui.as_weak();
    ui.on_language_filter(move |query| {
        if let Some(ui) = weak.upgrade() {
            ui.set_language_results(slint::ModelRc::new(VecModel::from(localization::filter(
                &query,
            ))));
        }
    });
    let c = control.clone();
    ui.on_action(move |a| c.borrow_mut().action(&a));
    let c = control.clone();
    ui.on_select_game(move |id| {
        let mut c = c.borrow_mut();
        c.selected = id.into();
        c.games();
    });
    let c = control.clone();
    ui.global::<crate::Shortcuts>()
        .on_monitor(move || c.borrow_mut().action("toggle"));
    let c = control.clone();
    ui.on_search(move |s| {
        let mut c = c.borrow_mut();
        if c.ui.upgrade().is_some_and(|u| u.get_modal() == 2) {
            c.candidate_query = s.into();
            c.candidate_page = 0;
        } else {
            c.query = s.into();
            c.page = 0;
        }
        c.games();
        c.animate();
    });
    let c = control.clone();
    ui.on_bulk_edit(move |s| c.borrow().bulk(&s));
    let c = control.clone();
    ui.on_plan_filter(move |s| {
        let mut c = c.borrow_mut();
        c.plan_page = 0;
        c.plans(&s, true);
    });
    let c = control.clone();
    ui.on_choose_plan(move |i| {
        let mut c = c.borrow_mut();
        if i == -1 && c.ui.upgrade().is_some_and(|u| u.get_plan_kind() == 2) {
            c.edit_plan.clear();
            if let Some(ui) = c.ui.upgrade() {
                ui.set_game_plan_name(tr("Use global Gaming plan").into());
                ui.set_plan_kind(-1);
                ui.set_modal(3);
            }
            return;
        }
        // A cleared result set or a closed popup must never select row zero as
        // a side effect of a negative keyboard index or a delayed callback.
        let Ok(i) = usize::try_from(i) else { return };
        let Some(ui) = c.ui.upgrade() else { return };
        if !matches!(ui.get_plan_kind(), 0 | 1 | 2) {
            return;
        }
        if let Some(id) = c.plan_ids.get(i).cloned() {
            if ui.get_plan_kind() == 2 {
                c.edit_plan = id;
                ui.set_game_plan_name(c.plan_name(&c.edit_plan).into());
                ui.set_plan_kind(-1);
                ui.set_modal(3);
                return;
            }
            let mut config = c.state.config.clone().unwrap_or_else(defaults);
            if ui.get_plan_kind() == 0 {
                config.gaming = id.clone()
            } else {
                config.default_plan = id.clone()
            }
            if c.try_save(config) {
                ui.set_plan_kind(-1);
            }
        }
    });
    let c = control.clone();
    ui.on_add_candidate(move |id| {
        let c = c.borrow();
        if let Some(g) = c
            .running_candidates
            .as_ref()
            .unwrap_or(&c.state.discovered)
            .iter()
            .find(|g| g.id == id.as_str())
        {
            if let Some((_, path)) = g.source.split_once(" | ") {
                c.send(Command {
                    kind: IMPORT,
                    values: vec![path.into()],
                    ..Default::default()
                });
            } else {
                let mut config = c.state.config.clone().unwrap_or_else(defaults);
                config.add_names(&g.processes);
                c.save(config);
            }
        }
    });
    let c = control.clone();
    ui.on_navigate(move |page| {
        let mut c = c.borrow_mut();
        c.animate();
        if page == 1 && c.preferences.auto_discover && !c.discovered_once {
            c.discovered_once = true;
            c.simple(SCAN);
        }
    });
    let v = view.clone();
    ui.on_view(move |_| v.publish());
    let weak = ui.as_weak();
    ui.on_window_action(move |a| {
        if let Some(ui) = weak.upgrade() {
            if a == 0 {
                CONTROL.with(|cell| {
                    if let Some(c) = cell.borrow().as_ref() {
                        c.borrow().close_window();
                    }
                });
            } else if matches!(a, 3 | 4) {
                // Slint 1.18.1 exposes this public, version-specific backend
                // extension behind unstable-winit-030. Winit queues Windows'
                // native move/resize loop and reconciles the mouse release.
                // Calling SendMessageW from TouchArea's press callback instead
                // can leave Slint's internal pointer grab active after sizing.
                let operation = if a == 3 {
                    "Move window"
                } else {
                    "Resize window"
                };
                let result = ui.window().with_winit_window(|window| {
                    if a == 3 {
                        window.drag_window()
                    } else {
                        window.drag_resize_window(winit::window::ResizeDirection::SouthEast)
                    }
                });
                let error = match result {
                    Some(Ok(())) => None,
                    Some(Err(e)) => Some(format!("{operation}: {e}")),
                    None => Some(format!(
                        "{operation}: native window backend is unavailable."
                    )),
                };
                if let Some(error) = error {
                    ui.set_status(error.into());
                    ui.set_error(true);
                }
            } else {
                win::window(hwnd(&ui), a as u32);
            }
        }
    });
    let weak = ui.as_weak();
    thread::spawn(move || {
        let connection = (|| -> Result<win::Pipe, String> {
            if let Ok(f) = win::connect_pipe() {
                return Ok(f);
            }
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let mut cmd = win::hidden_command(&exe.to_string_lossy());
            cmd.arg("--daemon");
            if dry {
                cmd.arg("--dry-run");
            }
            cmd.spawn().map_err(|e| e.to_string())?;
            for _ in 0..150 {
                if let Ok(f) = win::connect_pipe() {
                    return Ok(f);
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(tr("The native engine did not start. See native-startup-error.txt.").into())
        })();
        let result = (|| -> Result<(), String> {
            let mut read = connection?;
            let mut ring = win::TelemetryRing::open(false).ok();
            let mut last_topology = Vec::<Core>::new();
            // HELLO may start monitoring: reject a mismatched peer and shared
            // ring before sending it or exposing the queued command writer.
            let initial = ipc::begin_session(&mut read, dry, paused, |snapshot| {
                hydrate_telemetry(snapshot, dry, &mut ring, &mut last_topology)
            })?;
            let mut first = Some(initial);
            let mut write = read.try_clone().map_err(|e| e.to_string())?;
            let active = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            struct SessionEnd(std::sync::Arc<std::sync::atomic::AtomicBool>);
            impl Drop for SessionEnd {
                fn drop(&mut self) {
                    self.0.store(false, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let _session_end = SessionEnd(active.clone());
            thread::spawn(move || {
                while let Ok(command) = commands.recv() {
                    if !active.load(std::sync::atomic::Ordering::SeqCst)
                        || ipc::write(&mut write, &command).is_err()
                    {
                        break;
                    }
                }
            });
            loop {
                let s = if let Some(initial) = first.take() {
                    initial
                } else {
                    let mut snapshot = ipc::read::<Snapshot>(&mut read)?;
                    hydrate_telemetry(&mut snapshot, dry, &mut ring, &mut last_topology)?;
                    snapshot
                };
                weak.upgrade_in_event_loop(move |_| {
                    CONTROL.with(|c| {
                        if let Some(c) = c.borrow().as_ref() {
                            c.borrow_mut().update(s);
                        }
                    })
                })
                .map_err(|e| e.to_string())?;
            }
        })();
        if let Err(e) = result {
            let _ = weak.upgrade_in_event_loop(move |_| {
                CONTROL.with(|cell| {
                    if let Some(control) = cell.borrow().as_ref() {
                        let mut control = control.borrow_mut();
                        let elapsed = control.session_started.elapsed().as_millis() as u64;
                        control.pro.disconnect(elapsed);
                        control.connected = false;
                        control.state.suspended = false;
                        control.view.fullscreen.set(false);
                        control.view.publish();
                        control.requests.borrow_mut().pending = None;
                        control.requests.borrow_mut().draft = None;
                        control.sync_request_state();
                        control.connection_error = Some(e);
                        for core in &mut control.state.cores {
                            core.load = -1.;
                        }
                        control.state.cpu_sampled_at_ms = 0;
                        control.cores_dirty = true;
                        control.ui_dirty = true;
                        control.update_hud();
                        control.render_snapshot(now_ms());
                    }
                });
            });
        }
    });
    crate::frame_probe::install(&ui);
    ui.show().map_err(|e| e.to_string())?;
    control.borrow().apply_appearance();
    let c = control.clone();
    ui.window().on_close_requested(move || {
        c.borrow().close_window();
        slint::CloseRequestResponse::KeepWindowShown
    });
    let v = view.clone();
    ui.window().on_winit_window_event(move |_, event| {
        match event {
            winit::event::WindowEvent::Focused(focused) => {
                v.focused.set(*focused);
                if *focused {
                    schedule_system_language_refresh();
                }
            }
            winit::event::WindowEvent::Occluded(occluded) => v.occluded.set(*occluded),
            winit::event::WindowEvent::Resized(_) => {}
            _ => return slint::winit_030::EventResult::Propagate,
        }
        v.publish();
        slint::winit_030::EventResult::Propagate
    });
    view.focused.set(
        ui.window()
            .with_winit_window(|w| w.has_focus())
            .unwrap_or(false),
    );
    view.publish();
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(50), move || {
        if let Some(ui) = weak.upgrade() {
            win::round_window(hwnd(&ui));
            CONTROL.with(|cell| {
                if let Some(c) = cell.borrow().as_ref() {
                    let c = c.borrow();
                    if c.preferences.start_minimized
                        && ui.get_tray_available()
                        && !c.show_requested.get()
                    {
                        let _ = ui.hide();
                        c.view.publish();
                    }
                }
            });
        }
    });
    slint::run_event_loop_until_quit().map_err(|e| e.to_string())?;
    control
        .borrow()
        .sensor_stop
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = control.borrow().sensor_request.try_send(());
    control.borrow().motion.stop();
    CONTROL.with(|c| *c.borrow_mut() = None);
    Ok(())
}

#[cfg(test)]
mod ui_model_tests {
    use super::*;

    #[test]
    fn deferred_hud_setup_cannot_revive_a_disabled_or_replaced_hud() {
        assert!(hud_setup_current(5, 5, true));
        assert!(!hud_setup_current(5, 5, false));
        assert!(!hud_setup_current(5, 6, false));
        assert!(!hud_setup_current(5, 7, true));
    }

    #[test]
    fn unchanged_cpu_timestamp_becomes_unavailable_at_freshness_boundary() {
        assert!(!cpu_sample_fresh(0, 1000));
        assert!(cpu_sample_fresh(1000, 3999));
        assert!(!cpu_sample_fresh(1000, 4000));
        assert!(!cpu_sample_fresh(1000, 9000));
        assert!(cpu_sample_fresh(9000, 9001));
        assert!(!cpu_sample_fresh(9001, 9000));
    }

    #[test]
    fn sensor_results_cannot_cross_hidden_generations_or_gain_queue_freshness() {
        let completed = Instant::now();
        assert!(sensor_result_current(4, 4, true, completed, completed));
        assert!(!sensor_result_current(4, 5, true, completed, completed));
        assert!(!sensor_result_current(4, 6, true, completed, completed));
        assert!(!sensor_result_current(4, 4, false, completed, completed));
        assert!(!sensor_result_current(
            4,
            4,
            true,
            completed,
            completed + Duration::from_secs(3)
        ));
    }

    #[test]
    fn unavailable_cpu_does_not_hide_gpu_identity_and_sample_caveats() {
        let sample = crate::system_integration::SensorSnapshot {
            provider: "NVIDIA NVML".into(),
            gpu_label: "GPU A [NVML GPU 0]".into(),
            error: Some("CPU temperature unavailable".into()),
            freshness_note: "WMI cache age 10 s; acquisition timestamp unavailable".into(),
            ..Default::default()
        };
        let status = sensor_status(&sample);
        for required in [
            "NVIDIA NVML",
            "GPU A",
            "CPU temperature unavailable",
            "cache age 10 s",
            "timestamp unavailable",
        ] {
            assert!(status.contains(required));
        }
    }

    #[test]
    fn model_sync_preserves_unchanged_rows_and_reports_only_real_changes() {
        let core = CoreRow {
            label: "P0 10%".into(),
            activity: 0.1,
            ..Default::default()
        };
        let rows = Rc::new(VecModel::from(vec![core.clone(), core.clone()]));
        let bound: slint::ModelRc<CoreRow> = rows.clone().into();
        assert_eq!(sync_rows(&rows, [core.clone(), core.clone()]), 0);
        let changed = CoreRow {
            activity: 0.8,
            label: "P0 80%".into(),
            ..core.clone()
        };
        assert_eq!(sync_rows(&rows, [changed.clone(), core.clone()]), 1);
        assert_eq!(bound.row_data(0), Some(changed));
        assert_eq!(bound.row_data(1), Some(core.clone()));
        assert_eq!(sync_rows(&rows, [core.clone()]), 2);
        assert_eq!(bound.row_count(), 1);
        assert_eq!(sync_rows(&rows, [core.clone(), core.clone(), core]), 2);
        assert_eq!(bound.row_count(), 3);
        assert_eq!(sync_rows(&rows, []), 3);
        assert_eq!(bound.row_count(), 0);
    }

    #[test]
    fn unchanged_history_retains_its_model_and_rows() {
        let event = HistoryRow {
            label: "Gaming → Balanced".into(),
            detail: "exit · 1.2 ms".into(),
            ..Default::default()
        };
        let rows = Rc::new(VecModel::from(vec![event.clone()]));
        let bound: slint::ModelRc<HistoryRow> = rows.clone().into();
        for _ in 0..100 {
            assert_eq!(sync_rows(&rows, [event.clone()]), 0);
        }
        assert_eq!(bound.row_data(0), Some(event));
    }

    #[test]
    fn monitored_fullscreen_suspends_main_and_hud_without_windows_occlusion() {
        for shown in [true, false] {
            for hud in [true, false] {
                assert_eq!(
                    view_policy(shown, true, false, 0, hud, true),
                    ViewPolicy {
                        main: false,
                        cpu: false,
                        sensors: false
                    }
                );
            }
        }
        assert_eq!(
            view_policy(true, true, false, 0, true, false),
            ViewPolicy {
                main: true,
                cpu: true,
                sensors: true
            }
        );
        assert_eq!(
            view_policy(false, false, false, 1, true, false),
            ViewPolicy {
                main: false,
                cpu: true,
                sensors: true
            }
        );
    }

    #[test]
    fn visibility_preserves_hud_sampling_without_hidden_main_rendering() {
        assert_eq!(
            view_policy(true, true, false, 0, false, false),
            ViewPolicy {
                main: true,
                cpu: true,
                sensors: true
            }
        );
        assert_eq!(
            view_policy(true, false, false, 0, false, false),
            ViewPolicy {
                main: true,
                cpu: false,
                sensors: true
            }
        );
        assert_eq!(
            view_policy(true, true, false, 1, false, false),
            ViewPolicy {
                main: true,
                cpu: false,
                sensors: true
            }
        );
        for shown in [false, true] {
            assert_eq!(
                view_policy(shown, true, true, 0, false, false),
                ViewPolicy {
                    main: false,
                    cpu: false,
                    sensors: false
                }
            );
            assert_eq!(
                view_policy(shown, false, true, 1, true, false),
                ViewPolicy {
                    main: false,
                    cpu: true,
                    sensors: true
                }
            );
        }
        assert_eq!(
            view_policy(false, true, false, 0, false, false),
            ViewPolicy {
                main: false,
                cpu: false,
                sensors: false
            }
        );
    }
}
