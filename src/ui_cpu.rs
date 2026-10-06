// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
// Included by ui.rs. Topology is read once, separate from live utilization.
// Section models/ordering change only when processor membership changes.
impl Controller {
    fn bind_cpu_topology(&self, ui: &AppWindow) {
        let presentation = self.topology.presentation();
        ui.set_cpu_vendor(self.topology.vendor.clone().into());
        ui.set_cpu_model(self.topology.model.clone().into());
        ui.set_topology_mode(presentation.mode);
        ui.set_p_core_count(presentation.p_core_count as i32);
        ui.set_e_core_count(presentation.e_core_count as i32);
        ui.set_is_hybrid(presentation.is_hybrid);
        ui.set_has_l3_groups(presentation.has_l3_groups);
        // L3 sharing is observed; actual AMD CCD/CCX identifiers are not exposed
        // by this Windows API. Retain the requested compatibility bindings without
        // misrepresenting cache-domain IDs as manufacturer die/complex IDs.
        ui.set_has_ccx(false);
        ui.set_topology_summary(self.topology.summary.clone().into());
        ui.set_topology_detail(format!("{}\n\n{}\n\n{}", self.topology.summary, self.topology.status,
            "P/E classes use Windows core EfficiencyClass. L3 groups describe shared cache, not guaranteed CCD boundaries. Larger L3 is reported as cache asymmetry, not a confirmed 3D V-Cache product feature.").into());
    }
    fn prepare_cpu_layout(&mut self, ui: &AppWindow) {
        let cores = &self.state.cores;
        if self.core_keys.len() == cores.len()
            && self
                .core_keys
                .iter()
                .zip(cores)
                .all(|(key, c)| *key == (c.group, c.logical))
        {
            return;
        }
        self.core_keys = cores.iter().map(|c| (c.group, c.logical)).collect();
        let layout = cpu_layout(&self.topology, cores);
        self.core_order = layout.order;
        sync_rows(&self.models.core_sections, layout.sections);
        sync_rows(&self.models.l3_groups, layout.l3_groups);
        sync_rows(&self.models.ccx_groups, self.core_order.iter().map(|_| -1));
        ui.set_core_rows_8(layout.rows[0] as i32);
        ui.set_core_rows_12(layout.rows[1] as i32);
        ui.set_core_rows_16(layout.rows[2] as i32);
        self.cores_dirty = true;
        ui.set_core_info("".into());
    }
}

struct CpuLayout {
    order: Vec<usize>,
    sections: Vec<CoreSection>,
    rows: [usize; 3],
    l3_groups: Vec<i32>,
}

/// Calculate offsets from applicable, nonempty sections only. Unknown live
/// processor addresses get a neutral final section, even on a hybrid CPU.
/// Physical and SMT counts always come from measured masks, never 2 * P-count.
fn cpu_layout(topology: &crate::cpu_topology::CpuTopology, cores: &[Core]) -> CpuLayout {
    let presentation = topology.presentation();
    let applicable_group = |id| {
        topology.groups.iter().find(|g| {
            g.id == id
                && match g.class.as_str() {
                    "P" | "E" => presentation.is_hybrid,
                    _ => true,
                }
        })
    };
    let group_key = |c: &Core| {
        topology
            .logical(c.group, c.logical)
            .and_then(|c| applicable_group(c.group_id))
            .map(|g| g.id)
            .unwrap_or(u32::MAX)
    };
    let mut order: Vec<_> = (0..cores.len()).collect();
    order.sort_by_key(|&i| {
        let c = &cores[i];
        (group_key(c), c.group, c.logical)
    });
    let mut sections = Vec::new();
    let mut rows = [0usize; 3];
    let mut start = 0;
    while start < order.len() {
        let key = group_key(&cores[order[start]]);
        let count = order[start..]
            .iter()
            .take_while(|&&i| group_key(&cores[i]) == key)
            .count();
        let group = applicable_group(key);
        let physical: std::collections::BTreeSet<_> = order[start..start + count]
            .iter()
            .map(|&i| {
                let c = &cores[i];
                topology
                    .logical(c.group, c.logical)
                    .map(|c| (0, c.physical))
                    .unwrap_or((c.group, c.physical))
            })
            .collect();
        let cache = group
            .and_then(|g| g.l3_bytes)
            .map(|bytes| format!(" · {} MiB L3", bytes / 1_048_576))
            .unwrap_or_default();
        sections.push(CoreSection {
            title: group
                .map(|g| g.label.as_str())
                .unwrap_or("LOGICAL PROCESSORS")
                .into(),
            detail: group
                .filter(|g| {
                    g.physical_count == physical.len() as u32 && g.logical_count == count as u32
                })
                .map(|g| g.detail.clone())
                .unwrap_or_else(|| format!("{} cores · {} threads{}", physical.len(), count, cache))
                .into(),
            class: group.map(|g| g.class.as_str()).unwrap_or("C").into(),
            start: start as i32,
            count: count as i32,
            rows_8: rows[0] as i32,
            rows_12: rows[1] as i32,
            rows_16: rows[2] as i32,
        });
        for (total, columns) in rows.iter_mut().zip([8, 12, 16]) {
            *total += count.div_ceil(columns);
        }
        start += count;
    }
    // A false capability has an empty mapping, not a vector of -1 values
    // which could accidentally look like a populated cache-group model.
    let l3_groups = if presentation.has_l3_groups {
        order
            .iter()
            .map(|&i| {
                let c = &cores[i];
                topology
                    .logical(c.group, c.logical)
                    .and_then(|c| c.cache_domain)
                    .map(|id| id as i32)
                    .unwrap_or(-1)
            })
            .collect()
    } else {
        Vec::new()
    };
    CpuLayout {
        order,
        sections,
        rows,
        l3_groups,
    }
}

fn topology_core_row(
    topology: &crate::cpu_topology::CpuTopology,
    c: &Core,
    fresh: bool,
    sampled_at: u64,
    observed_at: u64,
) -> CoreRow {
    let info = topology.logical(c.group, c.logical);
    let class = info.map(|c| c.class.as_str()).unwrap_or("C");
    let load = if fresh { c.load } else { -1. };
    let activity = pro_view::core_activity(load);
    let cache = info
        .and_then(|c| c.cache_domain.zip(c.l3_bytes))
        .map(|(id, bytes)| format!(" · L3 group {id} · {} MiB shared", bytes / 1_048_576))
        .unwrap_or_default();
    CoreRow {
        class: class.into(),
        label: format!(
            "{}{:<3} {:>4}",
            class,
            c.logical,
            if activity < 0. {
                "—".into()
            } else {
                format!("{:3.0}%", load)
            }
        )
        .into(),
        fill: if c.parked {
            Color::from_rgb_u8(86, 93, 107)
        } else if class == "P" {
            Color::from_rgb_u8(16, 163, 127)
        } else if class == "E" {
            Color::from_rgb_u8(0, 229, 255)
        } else if topology.vendor == "AMD" && info.and_then(|c| c.cache_domain).is_some() {
            if info.and_then(|c| c.cache_domain).unwrap_or(0) % 2 == 0 {
                Color::from_rgb_u8(90, 171, 150)
            } else {
                Color::from_rgb_u8(93, 168, 193)
            }
        } else {
            Color::from_rgb_u8(125, 137, 156)
        },
        activity,
        detail: format!(
            "Group {} · logical {} · physical {} · {}{}{}",
            c.group,
            c.logical,
            info.map(|c| c.physical).unwrap_or(c.physical),
            if activity < 0. {
                "Activity unavailable".into()
            } else {
                format!("{load:.1}% Windows activity")
            },
            cache,
            if activity >= 0. && sampled_at > 0 {
                format!(
                    " · age {:.1}s · active ≥1%",
                    observed_at.saturating_sub(sampled_at) as f64 / 1000.
                )
            } else {
                String::new()
            }
        )
        .into(),
    }
}

#[cfg(test)]
mod topology_row_tests {
    use super::*;
    use crate::cpu_topology::{CpuTopology, LogicalCpu};

    fn fixture() -> CpuTopology {
        CpuTopology {
            vendor: "Intel".into(),
            model: "Fixture CPU".into(),
            summary: String::new(),
            status: String::new(),
            physical_count: 2,
            logical_count: 2,
            p_core_count: 1,
            e_core_count: 1,
            is_hybrid: true,
            has_l3_groups: false,
            asymmetric_l3: false,
            groups: Vec::new(),
            logicals: vec![
                LogicalCpu {
                    group: 0,
                    logical: 0,
                    physical: 3,
                    efficiency: 8,
                    class: "P".into(),
                    group_id: 0,
                    cache_domain: None,
                    l3_bytes: None,
                },
                LogicalCpu {
                    group: 1,
                    logical: 0,
                    physical: 27,
                    efficiency: 0,
                    class: "E".into(),
                    group_id: 1,
                    cache_domain: None,
                    l3_bytes: None,
                },
            ],
        }
    }

    fn section_fixture() -> CpuTopology {
        let mut t = fixture();
        t.groups = ["P", "E"]
            .iter()
            .enumerate()
            .map(|(id, class)| crate::cpu_topology::TopologyGroup {
                id: id as u32,
                label: format!("{class}-CORES"),
                detail: "1 cores · 1 threads".into(),
                class: (*class).into(),
                physical_count: 1,
                logical_count: 1,
                l3_bytes: None,
            })
            .collect();
        t
    }

    #[test]
    fn cpu_layout_has_no_empty_sections_and_joins_unknown_addresses_neutrally() {
        let t = section_fixture();
        let cores = [
            Core {
                group: 2,
                logical: 7,
                physical: 4,
                ..Default::default()
            },
            Core {
                group: 1,
                logical: 0,
                ..Default::default()
            },
            Core {
                group: 0,
                logical: 0,
                ..Default::default()
            },
        ];
        let layout = cpu_layout(&t, &cores);
        assert_eq!(layout.order, [2, 1, 0]);
        assert!(layout.l3_groups.is_empty());
        assert_eq!(layout.rows, [3, 3, 3]);
        assert_eq!(layout.sections.len(), 3);
        for (index, section) in layout.sections.iter().enumerate() {
            assert_eq!((section.start, section.count), (index as i32, 1));
            assert_eq!(
                (section.rows_8, section.rows_12, section.rows_16),
                (index as i32, index as i32, index as i32)
            );
        }
        assert_eq!(layout.sections[2].title.as_str(), "LOGICAL PROCESSORS");
        assert_eq!(layout.sections[2].class.as_str(), "C");
        // Missing live E rows do not reserve a header or grid row.
        let only_p = cpu_layout(&t, &cores[2..]);
        assert_eq!(only_p.sections.len(), 1);
        assert_eq!(only_p.sections[0].class.as_str(), "P");
        assert_eq!(only_p.rows, [1, 1, 1]);
        let empty = cpu_layout(&t, &[]);
        assert!(empty.sections.is_empty() && empty.order.is_empty());
        assert_eq!(empty.rows, [0, 0, 0]);
    }

    #[test]
    fn homogeneous_layout_cannot_expose_stale_hybrid_headings_or_cache_mapping() {
        let mut t = section_fixture();
        t.is_hybrid = false;
        // Even an inconsistent snapshot/fixture cannot make stale P/E metadata
        // produce a reserved hidden section or nonzero public hybrid counts.
        let cores = [
            Core {
                group: 0,
                ..Default::default()
            },
            Core {
                group: 1,
                ..Default::default()
            },
        ];
        let contract = t.presentation();
        assert_eq!(
            (contract.mode, contract.p_core_count, contract.e_core_count),
            (0, 0, 0)
        );
        let layout = cpu_layout(&t, &cores);
        assert_eq!(layout.sections.len(), 1);
        assert_eq!(layout.sections[0].title.as_str(), "LOGICAL PROCESSORS");
        assert_eq!(layout.sections[0].count, 2);
        assert_eq!(layout.rows, [1, 1, 1]);
        assert!(layout.l3_groups.is_empty());
    }

    #[test]
    fn one_observed_amd_cache_remains_a_visible_unified_cache_section() {
        let mut t = section_fixture();
        t.vendor = "AMD".into();
        t.is_hybrid = false;
        t.has_l3_groups = true;
        for c in &mut t.logicals {
            c.class = "C".into();
            c.group_id = 0;
            c.cache_domain = Some(0);
            c.l3_bytes = Some(96 * 1_048_576);
        }
        t.groups.truncate(1);
        let group = &mut t.groups[0];
        group.class = "C".into();
        group.label = "CACHE GROUP 0".into();
        group.physical_count = 2;
        group.logical_count = 2;
        group.l3_bytes = Some(96 * 1_048_576);
        group.detail = "2 cores · 2 threads · 96 MiB shared L3".into();
        let cores = [
            Core {
                group: 1,
                ..Default::default()
            },
            Core {
                group: 0,
                ..Default::default()
            },
        ];
        let layout = cpu_layout(&t, &cores);
        assert_eq!(t.presentation().mode, 2);
        assert_eq!(layout.order, [1, 0]);
        assert_eq!(layout.l3_groups, [0, 0]);
        assert_eq!(layout.sections.len(), 1);
        assert_eq!(layout.sections[0].title.as_str(), "CACHE GROUP 0");
        assert_eq!(
            layout.sections[0].detail.as_str(),
            "2 cores · 2 threads · 96 MiB shared L3"
        );
        assert_eq!(layout.rows, [1, 1, 1]);
    }

    #[test]
    fn row_identity_joins_both_windows_group_and_logical_index() {
        let t = fixture();
        let original = Core {
            group: 1,
            logical: 0,
            physical: 999,
            efficiency: 8,
            load: 42.5,
            ..Default::default()
        };
        let row = topology_core_row(&t, &original, true, 1000, 1250);
        assert_eq!(row.class.as_str(), "E");
        assert!(row.detail.contains("Group 1 · logical 0 · physical 27"));
        assert!(row.detail.contains("42.5% Windows activity"));
        assert!((row.activity - 0.425).abs() < 0.0001);
        // Presentation IDs never rewrite the Core used by engine scheduling.
        assert_eq!((original.physical, original.efficiency), (999, 8));
        let same_local_index = Core {
            group: 0,
            ..original.clone()
        };
        assert_eq!(
            topology_core_row(&t, &same_local_index, true, 1000, 1250)
                .class
                .as_str(),
            "P"
        );
    }

    #[test]
    fn stale_rows_retain_hardware_class_without_old_percent_or_freshness() {
        let t = fixture();
        let core = Core {
            load: 98.5,
            ..Default::default()
        };
        let live = topology_core_row(&t, &core, true, 1000, 1001);
        let stale = topology_core_row(&t, &core, false, 1000, 5000);
        assert_eq!(stale.class, live.class);
        assert_eq!(stale.fill, live.fill);
        assert_eq!(stale.activity, -1.);
        assert!(stale.label.contains('—'));
        assert!(!stale.label.contains('%'));
        assert!(stale.detail.contains("Activity unavailable"));
        assert!(!stale.detail.contains("age "));
    }

    #[test]
    fn unmapped_cpu_is_neutral_instead_of_reusing_a_matching_ordinal() {
        let t = fixture();
        let core = Core {
            group: 2,
            logical: 0,
            physical: 4,
            efficiency: 8,
            load: 0.,
            ..Default::default()
        };
        let row = topology_core_row(&t, &core, true, 1000, 1000);
        assert_eq!(row.class.as_str(), "C");
        assert_eq!(row.fill, Color::from_rgb_u8(125, 137, 156));
        assert!(row.detail.contains("Group 2 · logical 0 · physical 4"));
        assert!(!row.detail.contains("L3"));
        assert_eq!(row.activity, 0.); // A genuine zero is not unavailable.
    }

    #[test]
    fn amd_cache_tooltip_reports_domain_size_without_ccd_or_vcache_claims() {
        let mut t = fixture();
        t.vendor = "AMD".into();
        t.is_hybrid = false;
        t.has_l3_groups = true;
        t.logicals[0].class = "C".into();
        t.logicals[0].cache_domain = Some(4);
        t.logicals[0].l3_bytes = Some(96 * 1_048_576);
        let row = topology_core_row(
            &t,
            &Core {
                load: 15.,
                ..Default::default()
            },
            true,
            1000,
            1100,
        );
        assert_eq!(row.class.as_str(), "C");
        assert!(row.detail.contains("L3 group 4 · 96 MiB shared"));
        assert!(!row.detail.contains("CCD") && !row.detail.contains("V-Cache"));
        assert_eq!(row.fill, Color::from_rgb_u8(90, 171, 150));
    }

    #[test]
    fn bad_live_values_never_become_valid_percentages() {
        let t = fixture();
        for load in [-1., 101., f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let row = topology_core_row(
                &t,
                &Core {
                    load,
                    ..Default::default()
                },
                true,
                1000,
                1000,
            );
            assert_eq!(row.activity, -1.);
            assert!(!row.label.contains('%'));
            assert!(row.detail.contains("Activity unavailable"));
        }
    }
}
