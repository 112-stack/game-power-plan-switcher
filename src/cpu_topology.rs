// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Read-only, group-aware Windows topology. Enumerate once when the GUI starts;
//! utilization still comes from PDH and is joined by (group, logical index).
//!
//! GetLogicalProcessorInformationEx supplies the active physical cores,
//! EfficiencyClass and actual shared L3 masks. CPUID supplies identity only.
//! AMD L3 sharing domains are deliberately not asserted to be physical CCDs:
//! Zen 2 can have two CCXs in a CCD, and cache asymmetry alone does not prove
//! 3D V-Cache. Likewise, Windows efficiency classes do not identify LP E-cores.
//!
//! ABI references (x64, including pre-GroupCount CACHE_RELATIONSHIP layout):
//! https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-processor_relationship
//! https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-cache_relationship
//! https://learn.microsoft.com/windows/win32/api/sysinfoapi/nf-sysinfoapi-getlogicalprocessorinformationex
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

type CpuKey = (u32, u32);

#[derive(Clone, Debug, Serialize)]
pub struct LogicalCpu {
    pub group: u32,
    pub logical: u32,
    /// A GUI topology identifier, not a CPU-set CoreIndex or APIC ID.
    pub physical: u32,
    pub efficiency: u8,
    /// Intel P/E only when Windows reports distinct classes; otherwise C.
    pub class: String,
    /// Dense index into CpuTopology.groups; not a Windows processor group.
    pub group_id: u32,
    /// Shared L3 domain index, never implicitly a CCD or CPUID CCX ID.
    pub cache_domain: Option<u32>,
    pub l3_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TopologyGroup {
    pub id: u32,
    pub label: String,
    pub detail: String,
    pub class: String,
    pub physical_count: u32,
    pub logical_count: u32,
    /// The cache shared by this domain; never the package cache total.
    pub l3_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CpuTopology {
    pub vendor: String,
    pub model: String,
    pub summary: String,
    pub status: String,
    /// Physical core counts (SMT siblings are counted only once).
    pub physical_count: u32,
    pub logical_count: u32,
    pub p_core_count: u32,
    pub e_core_count: u32,
    /// An Intel P/E classification justified by distinct Windows classes.
    pub is_hybrid: bool,
    /// AMD grouping by verified shared L3 masks is available.
    pub has_l3_groups: bool,
    pub asymmetric_l3: bool,
    pub groups: Vec<TopologyGroup>,
    pub logicals: Vec<LogicalCpu>,
}

/// A conservative presentation contract, separate from native records. A single
/// observed AMD cache domain is still a valid grouped view; the number of
/// logical-to-cache mapping entries is never treated as a cache-domain count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TopologyPresentation {
    /// 0 = standard, 1 = Intel hybrid, 2 = observed AMD shared-L3 domains.
    pub mode: i32,
    pub p_core_count: u32,
    pub e_core_count: u32,
    pub is_hybrid: bool,
    pub has_l3_groups: bool,
}

impl CpuTopology {
    pub fn logical(&self, group: u32, logical: u32) -> Option<&LogicalCpu> {
        self.logicals
            .binary_search_by_key(&(group, logical), |c| (c.group, c.logical))
            .ok()
            .map(|i| &self.logicals[i])
    }

    pub fn presentation(&self) -> TopologyPresentation {
        let hybrid = self.vendor == "Intel"
            && self.is_hybrid
            && self.p_core_count > 0
            && self.e_core_count > 0;
        let cache_groups = !hybrid
            && self.vendor == "AMD"
            && self.has_l3_groups
            && self.logicals.iter().any(|c| c.cache_domain.is_some());
        TopologyPresentation {
            mode: if hybrid {
                1
            } else if cache_groups {
                2
            } else {
                0
            },
            p_core_count: if hybrid { self.p_core_count } else { 0 },
            e_core_count: if hybrid { self.e_core_count } else { 0 },
            is_hybrid: hybrid,
            has_l3_groups: cache_groups,
        }
    }
}

/// Never changes thread/process affinity, power policy, or sensor state.
/// Failure preserves CPUID identity but leaves all topology claims unavailable.
pub fn detect() -> CpuTopology {
    let (vendor, model) = identity();
    from_windows(vendor, model, query_windows())
}

fn from_windows(vendor: String, model: String, result: Result<Vec<u8>, String>) -> CpuTopology {
    match result.and_then(|bytes| parse_windows(&bytes)) {
        Ok(raw) => assemble(vendor, model, raw),
        Err(error) => CpuTopology {
            summary: "Topology unavailable; logical processors remain unclassified".into(),
            status: format!("Windows topology unavailable: {error}"),
            vendor,
            model,
            physical_count: 0,
            logical_count: 0,
            p_core_count: 0,
            e_core_count: 0,
            is_hybrid: false,
            has_l3_groups: false,
            asymmetric_l3: false,
            groups: Vec::new(),
            logicals: Vec::new(),
        },
    }
}

#[cfg(target_arch = "x86_64")]
fn identity() -> (String, String) {
    use std::arch::x86_64::__cpuid;
    // CPUID is guaranteed on x86-64. Brand leaves are queried only when present.
    let base = __cpuid(0);
    let mut id = Vec::with_capacity(12);
    id.extend_from_slice(&base.ebx.to_le_bytes());
    id.extend_from_slice(&base.edx.to_le_bytes());
    id.extend_from_slice(&base.ecx.to_le_bytes());
    let raw_vendor = String::from_utf8_lossy(&id).trim_matches('\0').to_owned();
    let vendor = match raw_vendor.as_str() {
        "GenuineIntel" => "Intel".into(),
        "AuthenticAMD" => "AMD".into(),
        "" => "Unknown".into(),
        _ => raw_vendor,
    };
    let mut brand = Vec::with_capacity(48);
    if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
        for leaf in 0x8000_0002..=0x8000_0004 {
            let r = __cpuid(leaf);
            for part in [r.eax, r.ebx, r.ecx, r.edx] {
                brand.extend_from_slice(&part.to_le_bytes());
            }
        }
    }
    let model = String::from_utf8_lossy(&brand)
        .trim_matches('\0')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let model = if model.is_empty() {
        format!("{vendor} processor (brand unavailable)")
    } else {
        model
    };
    (vendor, model)
}

#[cfg(not(target_arch = "x86_64"))]
fn identity() -> (String, String) {
    ("Unknown".into(), "Processor identity unavailable".into())
}

#[cfg(all(windows, target_pointer_width = "64"))]
fn query_windows() -> Result<Vec<u8>, String> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLogicalProcessorInformationEx(
            relationship: i32,
            buffer: *mut c_void,
            length: *mut u32,
        ) -> i32;
    }
    const RELATION_ALL: i32 = 0xffff;
    const MAX_BYTES: u32 = 16 * 1024 * 1024;
    const INSUFFICIENT_BUFFER: i32 = 122;
    let mut length = 0;
    let ok = unsafe {
        GetLogicalProcessorInformationEx(RELATION_ALL, std::ptr::null_mut(), &mut length)
    };
    if ok != 0 || std::io::Error::last_os_error().raw_os_error() != Some(INSUFFICIENT_BUFFER) {
        return Err(format!("size query: {}", std::io::Error::last_os_error()));
    }
    // Hot-add can enlarge the buffer between calls. Retry without an unbounded
    // allocation/loop. u64 storage gives the native buffer its required alignment.
    for _ in 0..4 {
        if !(8..=MAX_BYTES).contains(&length) {
            return Err("invalid or excessive Windows topology buffer size".into());
        }
        let capacity = length;
        let mut aligned = vec![0u64; (capacity as usize).div_ceil(8)];
        let result = unsafe {
            GetLogicalProcessorInformationEx(RELATION_ALL, aligned.as_mut_ptr().cast(), &mut length)
        };
        if result != 0 {
            if length > capacity || length < 8 {
                return Err("Windows topology returned an invalid length".into());
            }
            // The successful API call initialized the returned byte count.
            return Ok(unsafe {
                std::slice::from_raw_parts(aligned.as_ptr().cast::<u8>(), length as usize).to_vec()
            });
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(INSUFFICIENT_BUFFER) {
            return Err(error.to_string());
        }
    }
    Err("Windows topology changed repeatedly during enumeration".into())
}

#[cfg(not(all(windows, target_pointer_width = "64")))]
fn query_windows() -> Result<Vec<u8>, String> {
    Err("64-bit Windows is required".into())
}

#[derive(Clone, Debug)]
struct PhysicalCore {
    efficiency: u8,
    members: Vec<CpuKey>,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CacheDomain {
    members: Vec<CpuKey>,
    bytes: u64,
}
#[derive(Default, Debug)]
struct RawTopology {
    cores: Vec<PhysicalCore>,
    l3: Vec<CacheDomain>,
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    bytes
        .get(offset..offset + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| "truncated topology WORD".into())
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "truncated topology DWORD".into())
}

/// Checked byte reads avoid unaligned union references and trusting native
/// variable-size trailing arrays. Offsets are the documented Windows x64 ABI.
fn masks(bytes: &[u8], offset: usize, count: u16) -> Result<Vec<CpuKey>, String> {
    if count == 0 || offset + usize::from(count) * 16 > bytes.len() {
        return Err("invalid GROUP_AFFINITY count".into());
    }
    let mut members = BTreeSet::new();
    for index in 0..usize::from(count) {
        let start = offset + index * 16;
        let b = &bytes[start..start + 8];
        let mask = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        let group = u32::from(u16_at(bytes, start + 8)?);
        for logical in 0..64 {
            if mask & (1u64 << logical) != 0 && !members.insert((group, logical)) {
                return Err("duplicate processor in affinity masks".into());
            }
        }
    }
    if members.is_empty() {
        return Err("empty processor affinity mask".into());
    }
    Ok(members.into_iter().collect())
}

fn parse_windows(bytes: &[u8]) -> Result<RawTopology, String> {
    let mut raw = RawTopology::default();
    let mut position = 0usize;
    while position < bytes.len() {
        let rest = &bytes[position..];
        let relationship = u32_at(rest, 0)?;
        let size = u32_at(rest, 4)? as usize;
        if size < 8 || size > rest.len() {
            return Err("invalid Windows topology record length".into());
        }
        let record = &rest[..size];
        match relationship {
            0 => {
                // 8-byte header + PROCESSOR_RELATIONSHIP fixed 24 bytes.
                let count = u16_at(record, 30)?;
                let members = masks(record, 32, count)?;
                raw.cores.push(PhysicalCore {
                    efficiency: record[9],
                    members,
                });
            }
            2 => {
                // 8-byte header + CACHE_RELATIONSHIP fixed 32 bytes.
                if size < 56 {
                    return Err("truncated CACHE_RELATIONSHIP".into());
                }
                // L3 unified/data caches, excluding instruction/trace caches.
                if record[8] == 3 && matches!(u32_at(record, 16)?, 0 | 2) {
                    let count = u16_at(record, 38)?;
                    // Older Windows uses a single GroupMask and zeroed reserved
                    // bytes in the position later reused for GroupCount.
                    let count = if count == 0 && size == 56 { 1 } else { count };
                    let members = masks(record, 40, count)?;
                    let cache_size = u64::from(u32_at(record, 12)?);
                    if cache_size != 0 {
                        raw.l3.push(CacheDomain {
                            members,
                            bytes: cache_size,
                        });
                    }
                }
            }
            _ => {} // Package/NUMA/die/new relationship types are not guessed.
        }
        position += size;
    }
    if raw.cores.is_empty() {
        return Err("Windows returned no active physical core relationships".into());
    }
    raw.cores.sort_by(|a, b| a.members.cmp(&b.members));
    let mut seen = BTreeSet::new();
    for core in &raw.cores {
        for key in &core.members {
            if !seen.insert(*key) {
                return Err("logical processor belongs to more than one physical core".into());
            }
        }
    }
    raw.l3.sort();
    raw.l3.dedup();
    Ok(raw)
}

fn cache_membership(raw: &RawTopology) -> Result<BTreeMap<CpuKey, usize>, String> {
    let active: BTreeSet<_> = raw
        .cores
        .iter()
        .flat_map(|c| c.members.iter().copied())
        .collect();
    let mut membership = BTreeMap::new();
    for (domain, cache) in raw.l3.iter().enumerate() {
        for key in &cache.members {
            if !active.contains(key) {
                return Err("L3 masks include a processor absent from the core snapshot".into());
            }
            if membership.insert(*key, domain).is_some() {
                return Err("overlapping L3 domains cannot be classified reliably".into());
            }
        }
    }
    for core in &raw.cores {
        let first = membership.get(&core.members[0]);
        if core.members.iter().any(|key| membership.get(key) != first) {
            return Err("SMT siblings disagree on their shared L3 domain".into());
        }
    }
    Ok(membership)
}

fn assemble(vendor: String, model: String, raw: RawTopology) -> CpuTopology {
    let classes: BTreeSet<_> = raw.cores.iter().map(|c| c.efficiency).collect();
    let high = classes.last().copied().unwrap_or(0);
    let is_hybrid = vendor == "Intel" && classes.len() > 1;
    let (cache_map, cache_warning) = match cache_membership(&raw) {
        Ok(mapping) => (mapping, String::new()),
        Err(error) => (
            BTreeMap::new(),
            format!("; L3 grouping unavailable: {error}"),
        ),
    };
    let has_l3_groups = vendor == "AMD" && !cache_map.is_empty();
    let asymmetric_l3 = !cache_map.is_empty()
        && raw
            .l3
            .iter()
            .map(|c| c.bytes)
            .collect::<BTreeSet<_>>()
            .len()
            > 1;
    let min_cache = raw.l3.iter().map(|c| c.bytes).min().unwrap_or(0);
    let mut topology = CpuTopology {
        vendor,
        model,
        summary: String::new(),
        status: format!("GetLogicalProcessorInformationEx; active Windows topology{cache_warning}"),
        physical_count: raw.cores.len() as u32,
        logical_count: raw.cores.iter().map(|c| c.members.len() as u32).sum(),
        p_core_count: 0,
        e_core_count: 0,
        is_hybrid,
        has_l3_groups,
        asymmetric_l3,
        groups: Vec::new(),
        logicals: Vec::new(),
    };
    // Section keys are separated from dense public IDs so missing cache masks
    // yield a final neutral section instead of silently assigning a guessed CCD.
    let mut section_ids = BTreeMap::new();
    let mut section_members: BTreeMap<u32, (BTreeSet<u32>, u32)> = BTreeMap::new();
    for (physical, core) in raw.cores.iter().enumerate() {
        let class = if is_hybrid {
            if core.efficiency == high {
                topology.p_core_count += 1;
                "P"
            } else {
                topology.e_core_count += 1;
                "E"
            }
        } else {
            "C"
        };
        let domain = cache_map.get(&core.members[0]).copied();
        let section = if is_hybrid {
            if class == "P" { 0 } else { 1 }
        } else if has_l3_groups {
            domain.map(|d| d as u32).unwrap_or(u32::MAX)
        } else {
            0
        };
        section_ids.entry(section).or_insert(0u32);
        let counts = section_members.entry(section).or_default();
        counts.0.insert(physical as u32);
        counts.1 += core.members.len() as u32;
        for &(group, logical) in &core.members {
            topology.logicals.push(LogicalCpu {
                group,
                logical,
                physical: physical as u32,
                efficiency: core.efficiency,
                class: class.into(),
                group_id: section,
                cache_domain: domain.map(|d| d as u32),
                l3_bytes: domain.map(|d| raw.l3[d].bytes),
            });
        }
    }
    for (id, (section, dense_id)) in section_ids.iter_mut().enumerate() {
        *dense_id = id as u32;
        let (physical, logical) = &section_members[section];
        let cache = if has_l3_groups && *section != u32::MAX {
            Some(raw.l3[*section as usize].bytes)
        } else {
            None
        };
        let (label, class) = if is_hybrid {
            if *section == 0 {
                ("P-CORES".into(), "P")
            } else {
                ("E-CORES".into(), "E")
            }
        } else if has_l3_groups && *section != u32::MAX {
            (format!("CACHE GROUP {section}"), "C")
        } else {
            ("LOGICAL PROCESSORS".into(), "C")
        };
        let mut detail = format!("{} cores · {logical} threads", physical.len());
        if let Some(bytes) = cache {
            detail.push_str(&format!(" · {} MiB shared L3", bytes / (1024 * 1024)));
            if asymmetric_l3 && bytes > min_cache {
                detail.push_str(" · larger L3");
            }
        } else if has_l3_groups {
            detail.push_str(" · L3 unavailable");
        }
        topology.groups.push(TopologyGroup {
            id: id as u32,
            label,
            detail,
            class: class.into(),
            physical_count: physical.len() as u32,
            logical_count: *logical,
            l3_bytes: cache,
        });
    }
    for cpu in &mut topology.logicals {
        cpu.group_id = section_ids[&cpu.group_id];
    }
    topology.logicals.sort_by_key(|c| (c.group, c.logical));
    topology.summary = if is_hybrid {
        format!(
            "{} P + {} E cores · {} logical processors",
            topology.p_core_count, topology.e_core_count, topology.logical_count
        )
    } else if has_l3_groups {
        format!(
            "{} cores · {} logical processors · {} shared L3 domains{}",
            topology.physical_count,
            topology.logical_count,
            raw.l3.len(),
            if asymmetric_l3 {
                " · asymmetric L3"
            } else {
                ""
            }
        )
    } else {
        format!(
            "{} cores · {} logical processors · standard topology",
            topology.physical_count, topology.logical_count
        )
    };
    if is_hybrid && classes.len() > 2 {
        topology
            .status
            .push_str("; multiple lower efficiency classes grouped as E; LP-E not inferred");
    }
    if has_l3_groups {
        topology
            .status
            .push_str("; L3 sharing is not proof of physical CCD count or 3D V-Cache");
    }
    topology
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core(class: u8, affinities: &[(u16, u64)]) -> Vec<u8> {
        let mut bytes = vec![0; 32 + affinities.len() * 16];
        let size = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        bytes[9] = class;
        bytes[30..32].copy_from_slice(&(affinities.len() as u16).to_le_bytes());
        put_masks(&mut bytes[32..], affinities);
        bytes
    }
    fn cache(mib: u32, affinities: &[(u16, u64)], legacy: bool) -> Vec<u8> {
        let mut bytes = vec![0; 40 + affinities.len() * 16];
        let size = bytes.len() as u32;
        bytes[..4].copy_from_slice(&2u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        bytes[8] = 3;
        bytes[12..16].copy_from_slice(&(mib * 1024 * 1024).to_le_bytes());
        if !legacy {
            bytes[38..40].copy_from_slice(&(affinities.len() as u16).to_le_bytes());
        }
        put_masks(&mut bytes[40..], affinities);
        bytes
    }
    fn put_masks(bytes: &mut [u8], masks: &[(u16, u64)]) {
        for (index, (group, mask)) in masks.iter().enumerate() {
            let at = index * 16;
            bytes[at..at + 8].copy_from_slice(&mask.to_le_bytes());
            bytes[at + 8..at + 10].copy_from_slice(&group.to_le_bytes());
        }
    }
    fn topology(vendor: &str, records: Vec<Vec<u8>>) -> CpuTopology {
        let bytes: Vec<u8> = records.into_iter().flatten().collect();
        assemble(
            vendor.into(),
            "Fixture CPU".into(),
            parse_windows(&bytes).unwrap(),
        )
    }

    #[test]
    fn intel_hybrid_counts_physical_cores_not_smt_siblings() {
        let t = topology(
            "Intel",
            vec![
                core(8, &[(0, 0b11)]),
                core(8, &[(0, 0b1100)]),
                core(0, &[(0, 0b10000)]),
                core(0, &[(0, 0b100000)]),
            ],
        );
        assert!(t.is_hybrid);
        assert_eq!((t.p_core_count, t.e_core_count, t.logical_count), (2, 2, 6));
        assert_eq!(t.groups[0].label, "P-CORES");
        assert_eq!(t.groups[1].label, "E-CORES");
        assert_eq!(
            t.logical(0, 1).unwrap().physical,
            t.logical(0, 0).unwrap().physical
        );
        assert_eq!(t.logical(0, 4).unwrap().class, "E");
    }
    #[test]
    fn homogeneous_intel_is_not_mislabeled_all_p_or_e() {
        for efficiency in [0, 8] {
            let t = topology(
                "Intel",
                vec![core(efficiency, &[(0, 1)]), core(efficiency, &[(0, 2)])],
            );
            assert!(!t.is_hybrid);
            assert_eq!((t.p_core_count, t.e_core_count), (0, 0));
            assert_eq!(t.groups[0].label, "LOGICAL PROCESSORS");
            assert!(t.logicals.iter().all(|c| c.class == "C"));
        }
    }
    #[test]
    fn ryzen_7800x3d_like_single_cache_has_one_section_and_no_hybrid_claim() {
        let mut records = (0..8)
            .map(|i| core(0, &[(0, 3 << (i * 2))]))
            .collect::<Vec<_>>();
        records.push(cache(96, &[(0, 0xffff)], false));
        let t = topology("AMD", records);
        let view = t.presentation();
        assert_eq!((view.mode, view.p_core_count, view.e_core_count), (2, 0, 0));
        assert!(!view.is_hybrid && view.has_l3_groups);
        assert_eq!(t.groups.len(), 1);
        assert_eq!(t.groups[0].label, "CACHE GROUP 0");
        assert_eq!(
            (t.groups[0].physical_count, t.groups[0].logical_count),
            (8, 16)
        );
        assert_eq!(t.groups[0].l3_bytes, Some(96 * 1_048_576));
        assert!(t.logicals.iter().all(|c| c.class == "C"));
        assert!(!t.asymmetric_l3 && !t.summary.contains("V-Cache"));
    }
    #[test]
    fn ryzen_5900x_like_two_caches_preserve_six_cores_per_domain() {
        let mut records = (0..12)
            .map(|i| core(0, &[(0, 3 << (i * 2))]))
            .collect::<Vec<_>>();
        records.extend([
            cache(32, &[(0, 0xfff)], false),
            cache(32, &[(0, 0xfff000)], false),
        ]);
        let t = topology("AMD", records);
        assert_eq!(t.presentation().mode, 2);
        assert_eq!((t.p_core_count, t.e_core_count), (0, 0));
        assert_eq!(t.groups.len(), 2);
        assert!(
            t.groups
                .iter()
                .all(|g| g.class == "C" && g.physical_count == 6 && g.logical_count == 12)
        );
        assert_eq!(t.groups[1].label, "CACHE GROUP 1");
    }
    #[test]
    fn intel_11900k_like_homogeneous_cache_stays_standard() {
        let mut records = (0..8)
            .map(|i| core(8, &[(0, 3 << (i * 2))]))
            .collect::<Vec<_>>();
        records.push(cache(16, &[(0, 0xffff)], false));
        let t = topology("Intel", records);
        assert_eq!(
            t.presentation(),
            TopologyPresentation {
                mode: 0,
                p_core_count: 0,
                e_core_count: 0,
                is_hybrid: false,
                has_l3_groups: false,
            }
        );
        assert_eq!(t.groups.len(), 1);
        assert_eq!(t.groups[0].label, "LOGICAL PROCESSORS");
        assert_eq!(t.groups[0].logical_count, 16);
    }
    #[test]
    fn intel_14900k_like_topology_has_exactly_thirty_two_logical_members() {
        let mut records = (0..8)
            .map(|i| core(8, &[(0, 3 << (i * 2))]))
            .collect::<Vec<_>>();
        records.extend((16..32).map(|i| core(0, &[(0, 1 << i)])));
        let t = topology("Intel", records);
        assert_eq!(t.presentation().mode, 1);
        assert_eq!(
            (t.p_core_count, t.e_core_count, t.logical_count),
            (8, 16, 32)
        );
        assert_eq!(t.groups.len(), 2);
        assert_eq!(
            (t.groups[0].logical_count, t.groups[1].logical_count),
            (16, 16)
        );
    }
    #[test]
    fn hybrid_thread_counts_use_masks_even_without_smt_or_across_groups() {
        let t = topology(
            "Intel",
            vec![
                core(8, &[(0, 1)]),         // A P-core with SMT disabled.
                core(8, &[(0, 2), (1, 1)]), // Native multi-group relationship.
                core(0, &[(1, 2)]),
            ],
        );
        assert_eq!((t.p_core_count, t.e_core_count), (2, 1));
        assert_eq!(t.groups[0].detail, "2 cores · 3 threads");
        assert_eq!(t.groups[1].detail, "1 cores · 1 threads");
        assert_eq!(
            t.logical(0, 1).unwrap().physical,
            t.logical(1, 0).unwrap().physical
        );
    }
    #[test]
    fn lower_intel_classes_do_not_invent_lp_e_architecture() {
        let t = topology(
            "Intel",
            vec![core(8, &[(0, 1)]), core(2, &[(0, 2)]), core(0, &[(0, 4)])],
        );
        assert_eq!((t.p_core_count, t.e_core_count), (1, 2));
        assert!(t.status.contains("LP-E not inferred"));
        assert!(t.logicals.iter().all(|c| c.class != "LP-E"));
    }
    #[test]
    fn zen2_shared_l3_domains_are_not_fabricated_ccd_counts() {
        let mut records = (0..8)
            .map(|i| core(0, &[(0, 3 << (i * 2))]))
            .collect::<Vec<_>>();
        records.extend([
            cache(16, &[(0, 0xff)], true),
            cache(16, &[(0, 0xff00)], true),
        ]);
        let t = topology("AMD", records);
        assert!(t.has_l3_groups);
        assert_eq!(t.groups.len(), 2);
        assert_eq!(t.groups[0].physical_count, 4);
        assert_eq!(t.groups[1].l3_bytes, Some(16 * 1024 * 1024));
        assert_eq!(t.logical(0, 8).unwrap().cache_domain, Some(1));
        assert!(!t.summary.contains("CCD"));
        assert!(!t.asymmetric_l3);
    }
    #[test]
    fn asymmetric_ryzen_l3_shows_per_domain_sizes_without_vcache_claim() {
        let mut records = (0..16)
            .map(|i| core(0, &[(0, 3 << (i * 2))]))
            .collect::<Vec<_>>();
        records.extend([
            cache(96, &[(0, 0xffff)], false),
            cache(32, &[(0, 0xffff0000)], false),
        ]);
        let t = topology("AMD", records);
        assert!(t.asymmetric_l3);
        assert_eq!(t.groups[0].physical_count, 8);
        assert!(t.groups[0].detail.contains("96 MiB shared L3 · larger L3"));
        assert_eq!(t.logical(0, 31).unwrap().l3_bytes, Some(32 * 1024 * 1024));
        assert!(!t.summary.contains("V-Cache"));
    }
    #[test]
    fn group_local_indices_remain_distinct_above_sixty_four_processors() {
        let mut records = Vec::new();
        for group in 0..2 {
            for logical in 0..64 {
                records.push(core(0, &[(group, 1u64 << logical)]));
            }
        }
        records.push(cache(128, &[(0, u64::MAX), (1, u64::MAX)], false));
        let t = topology("AMD", records);
        assert_eq!(t.logical_count, 128);
        assert_ne!(
            t.logical(0, 0).unwrap().physical,
            t.logical(1, 0).unwrap().physical
        );
        assert_eq!(t.logical(1, 63).unwrap().cache_domain, Some(0));
        assert_eq!(t.groups[0].logical_count, 128);
        assert!(t.logical(2, 0).is_none());
    }
    #[test]
    fn missing_l3_on_amd_stays_neutral() {
        let t = topology("AMD", vec![core(0, &[(0, 3)])]);
        assert!(!t.has_l3_groups);
        assert!(t.logicals[0].cache_domain.is_none());
        assert_eq!(t.groups[0].label, "LOGICAL PROCESSORS");
    }
    #[test]
    fn unknown_vendor_efficiency_classes_do_not_invent_intel_architecture() {
        let t = topology("Unknown", vec![core(8, &[(0, 1)]), core(0, &[(0, 2)])]);
        assert!(!t.is_hybrid);
        assert!(t.logicals.iter().all(|c| c.class == "C"));
    }
    #[test]
    fn partial_cache_coverage_keeps_unknown_cores_in_neutral_section() {
        let t = topology(
            "AMD",
            vec![
                core(0, &[(0, 3)]),
                core(0, &[(0, 12)]),
                cache(32, &[(0, 3)], false),
            ],
        );
        assert_eq!(t.groups.len(), 2);
        assert_eq!(t.groups[1].label, "LOGICAL PROCESSORS");
        assert_eq!(t.logical(0, 3).unwrap().group_id, 1);
        assert_eq!(t.groups[1].l3_bytes, None);
    }
    #[test]
    fn conflicting_l3_and_split_smt_are_rejected_without_losing_cores() {
        for caches in [
            vec![cache(32, &[(0, 3)], false), cache(96, &[(0, 3)], false)],
            vec![cache(32, &[(0, 1)], false), cache(32, &[(0, 2)], false)],
        ] {
            let mut records = vec![core(0, &[(0, 3)])];
            records.extend(caches);
            let t = topology("AMD", records);
            assert!(!t.has_l3_groups);
            assert_eq!(t.logical_count, 2);
            assert!(t.status.contains("L3 grouping unavailable"));
        }
    }
    #[test]
    fn duplicate_cache_records_are_deduplicated() {
        let t = topology(
            "AMD",
            vec![
                core(0, &[(0, 3)]),
                cache(32, &[(0, 3)], true),
                cache(32, &[(0, 3)], false),
            ],
        );
        assert_eq!(t.groups.len(), 1);
        assert!(t.has_l3_groups);
    }
    #[test]
    fn malformed_native_buffers_fail_closed() {
        let valid = core(0, &[(0, 3)]);
        for length in 0..valid.len() {
            assert!(parse_windows(&valid[..length]).is_err());
        }
        let mut broken = valid.clone();
        broken[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert!(parse_windows(&broken).is_err());
        let mut broken = valid.clone();
        broken[30..32].copy_from_slice(&2u16.to_le_bytes());
        assert!(parse_windows(&broken).is_err());
        assert!(parse_windows(&[valid.clone(), valid].concat()).is_err());
        let mut broken = core(0, &[(0, 1)]);
        broken.extend(cache(32, &[(0, 1), (1, 1)], true));
        assert!(parse_windows(&broken).is_err());
    }

    #[test]
    fn failed_query_and_malformed_buffer_preserve_identity_without_fake_topology() {
        for result in [Err("fixture access failure".into()), Ok(vec![0; 5])] {
            let t = from_windows("Intel".into(), "Known identity".into(), result);
            assert_eq!((&*t.vendor, &*t.model), ("Intel", "Known identity"));
            assert!(!t.is_hybrid && !t.has_l3_groups && !t.asymmetric_l3);
            assert_eq!(
                (
                    t.physical_count,
                    t.logical_count,
                    t.p_core_count,
                    t.e_core_count
                ),
                (0, 0, 0, 0)
            );
            assert!(t.logicals.is_empty() && t.groups.is_empty());
            assert!(t.logical(0, 0).is_none());
            assert!(t.status.contains("unavailable"));
        }
    }
}
