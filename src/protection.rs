// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Transparent release verification, not DRM or an anti-analysis mechanism.
//! Hashes describe the final on-disk executable. Authenticate the manifest using
//! an independently trusted signature before relying on its expected hashes.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Bound the read itself, not just a potentially stale metadata observation.
fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    reader
        .take(limit.checked_add(1).ok_or("Invalid read limit")?)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > limit {
        return Err(format!("File exceeds the {limit} byte inspection limit"));
    }
    Ok(data)
}

fn read_file_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    // Metadata and bytes come from the same open handle even if the path changes.
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size > limit {
        return Err(format!("File exceeds the {limit} byte inspection limit"));
    }
    let data = read_bounded(file, limit)?;
    if data.len() as u64 != size {
        return Err("File changed length while reading".into());
    }
    Ok(data)
}

fn create_manifest_temp(output: &Path) -> Result<(PathBuf, fs::File), String> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    for _ in 0..128 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".nn6-manifest-{}-{sequence}.tmp",
            std::process::id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Unable to allocate a unique manifest temporary file".into())
}

fn write_manifest_bytes(output: &Path, bytes: &[u8]) -> Result<(), String> {
    let (temporary, mut file) = create_manifest_temp(output)?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    // On Windows, std::fs::rename uses MoveFileExW with REPLACE_EXISTING.
    // A sibling file stays on the same volume. Failure preserves the destination;
    // no existing predictable .tmp or .bak file is opened or truncated.
    let result = written.and_then(|_| fs::rename(&temporary, output));
    if result.is_err() {
        let _ = fs::remove_file(&temporary); // Only the file created above is ours.
    }
    result.map_err(|e| e.to_string())
}

pub mod build_info {
    include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SectionDigest {
    pub name: String,
    pub raw_offset: usize,
    pub raw_size: usize,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeFlags {
    pub dynamic_base: bool,
    pub high_entropy_va: bool,
    pub nx_compatible: bool,
    /// This is a header observation, not evidence of complete CFG coverage.
    pub guard_cf_header: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema: u32,
    pub product: String,
    pub version: String,
    pub bytes: usize,
    pub sha256: String,
    pub sections: Vec<SectionDigest>,
    pub flags: PeFlags,
    pub provenance: serde_json::Value,
}

fn range(data: &[u8], start: usize, len: usize) -> Result<&[u8], String> {
    data.get(start..start.checked_add(len).ok_or("PE range overflow")?)
        .ok_or_else(|| "Truncated PE range".into())
}
fn u16_at(data: &[u8], offset: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        range(data, offset, 2)?.try_into().unwrap(),
    ))
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        range(data, offset, 4)?.try_into().unwrap(),
    ))
}
fn digest(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Parse only the bounded PE32+ fields needed for the manifest. This never maps
/// executable pages, inspects another process, or changes file contents.
fn describe(data: &[u8]) -> Result<(Vec<SectionDigest>, PeFlags), String> {
    if range(data, 0, 2)? != b"MZ" {
        return Err("Missing DOS header".into());
    }
    let pe = u32_at(data, 0x3c)? as usize;
    if pe < 0x40 || range(data, pe, 4)? != b"PE\0\0" {
        return Err("Invalid PE signature".into());
    }
    if u16_at(data, pe + 4)? != 0x8664 {
        return Err("Expected x86-64 PE".into());
    }
    let count = usize::from(u16_at(data, pe + 6)?);
    if !(1..=96).contains(&count) {
        return Err("Invalid PE section count".into());
    }
    let optional_len = usize::from(u16_at(data, pe + 20)?);
    if optional_len < 112 {
        return Err("Truncated optional header".into());
    }
    let optional = range(data, pe + 24, optional_len)?;
    if u16_at(optional, 0)? != 0x20b {
        return Err("Expected PE32+".into());
    }
    let flags = u16_at(optional, 70)?;
    let table = pe + 24 + optional_len;
    range(data, table, count * 40)?;
    let table_end = table + count * 40;
    let mut seen = Vec::<(usize, usize)>::new();
    let mut sections = Vec::new();
    for i in 0..count {
        let row = range(data, table + i * 40, 40)?;
        let size = u32_at(row, 16)? as usize;
        let start = u32_at(row, 20)? as usize;
        let raw = range(data, start, size)?;
        if size != 0 {
            let end = start + size;
            if start < table_end || seen.iter().any(|&(a, b)| start < b && a < end) {
                return Err("Overlapping PE section data".into());
            }
            seen.push((start, end));
        }
        let name = &row[..8];
        let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(8)];
        if name == b".text" || name == b".rdata" {
            let name = String::from_utf8(name.to_vec()).unwrap();
            if size == 0 || sections.iter().any(|s: &SectionDigest| s.name == name) {
                return Err("Empty or duplicate protected section".into());
            }
            sections.push(SectionDigest {
                name,
                raw_offset: start,
                raw_size: size,
                sha256: digest(raw),
            });
        }
    }
    if sections.len() != 2 {
        return Err("Required .text and .rdata sections are missing".into());
    }
    Ok((
        sections,
        PeFlags {
            dynamic_base: flags & 0x40 != 0,
            high_entropy_va: flags & 0x20 != 0,
            nx_compatible: flags & 0x100 != 0,
            guard_cf_header: flags & 0x4000 != 0,
        },
    ))
}

pub fn provenance() -> serde_json::Value {
    serde_json::json!({"version": build_info::VERSION, "authors": build_info::AUTHORS,
        "git_commit": build_info::GIT_COMMIT, "git_dirty": build_info::GIT_DIRTY,
        "hardening": build_info::HARDENING, "build_time_unix": build_info::BUILD_TIME_UNIX,
        "build_time_source": build_info::BUILD_TIME_SOURCE, "build_uuid": build_info::BUILD_UUID,
        "profile": build_info::PROFILE, "target": build_info::TARGET,
        "description": build_info::BUILD_PROVENANCE})
}

pub fn inspect(exe: &Path) -> Result<ReleaseManifest, String> {
    let data = read_file_bounded(exe, 512 * 1024 * 1024)?;
    let (sections, flags) = describe(&data)?;
    Ok(ReleaseManifest {
        schema: 1,
        product: "Game Power Plan Switcher".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        bytes: data.len(),
        sha256: digest(&data),
        sections,
        flags,
        provenance: provenance(),
    })
}

pub fn write_manifest(output: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // Never overwrite an executable (including the running image) through this CLI.
    if output
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("json"))
    {
        return Err("Manifest output must have a .json extension".into());
    }
    let bytes = serde_json::to_vec_pretty(&inspect(&exe)?).map_err(|e| e.to_string())?;
    write_manifest_bytes(output, &bytes)
}

pub fn verify_manifest(path: &Path) -> Result<(), String> {
    let expected: ReleaseManifest = serde_json::from_slice(&read_file_bounded(path, 1024 * 1024)?)
        .map_err(|e| format!("Invalid release manifest: {e}"))?;
    let actual = inspect(&std::env::current_exe().map_err(|e| e.to_string())?)?;
    verify(&expected, &actual)
}
fn verify(expected: &ReleaseManifest, actual: &ReleaseManifest) -> Result<(), String> {
    if expected.schema != 1
        || expected.product != actual.product
        || expected.version != actual.version
    {
        return Err("Release manifest product, version or schema mismatch".into());
    }
    if expected.bytes != actual.bytes
        || expected.sha256 != actual.sha256
        || expected.sections != actual.sections
        || expected.flags != actual.flags
    {
        return Err(
            "Release verification failed: executable bytes differ from the supplied manifest"
                .into(),
        );
    }
    if expected.provenance != actual.provenance {
        return Err("Release provenance differs from the supplied manifest".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory() -> PathBuf {
        for _ in 0..128 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "nn6-protection-test-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return path,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("Create test directory: {e}"),
            }
        }
        panic!("Unable to create an isolated test directory")
    }

    #[test]
    fn bounded_reader_stops_after_one_excess_byte() {
        let mut reader = std::io::Cursor::new(vec![1u8; 1024]);
        assert!(read_bounded(&mut reader, 64).is_err());
        assert_eq!(reader.position(), 65);
        assert_eq!(read_bounded(&[7u8; 64][..], 64).unwrap().len(), 64);
        assert!(read_bounded(&[][..], 0).unwrap().is_empty());
        assert!(read_bounded(&[1u8][..], 0).is_err());
        assert!(read_bounded(&[][..], u64::MAX).is_err());
    }

    #[test]
    fn manifest_replacement_preserves_unrelated_sibling_files() {
        let directory = test_directory();
        let output = directory.join("release.json");
        let existing_tmp = directory.join("release.tmp");
        let existing_backup = directory.join("release.bak");
        fs::write(&output, b"old manifest").unwrap();
        fs::write(&existing_tmp, b"unrelated temporary data").unwrap();
        fs::write(&existing_backup, b"unrelated backup data").unwrap();
        write_manifest_bytes(&output, b"new manifest").unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"new manifest");
        assert_eq!(
            fs::read(&existing_tmp).unwrap(),
            b"unrelated temporary data"
        );
        assert_eq!(
            fs::read(&existing_backup).unwrap(),
            b"unrelated backup data"
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 3);
        for path in [output, existing_tmp, existing_backup] {
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn failed_manifest_replacement_preserves_destination_and_cleans_own_temp() {
        let directory = test_directory();
        let destination = directory.join("occupied.json");
        fs::create_dir(&destination).unwrap();
        let sentinel = destination.join("keep.txt");
        fs::write(&sentinel, b"keep").unwrap();
        assert!(write_manifest_bytes(&destination, b"manifest").is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_file(sentinel).unwrap();
        fs::remove_dir(destination).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn temporary_files_are_exclusively_created_and_distinct() {
        let directory = test_directory();
        let output = directory.join("release.json");
        let (first, mut file) = create_manifest_temp(&output).unwrap();
        file.write_all(b"first").unwrap();
        drop(file);
        let (second, file) = create_manifest_temp(&output).unwrap();
        drop(file);
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).unwrap(), b"first");
        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    fn fixture() -> Vec<u8> {
        let mut data = vec![0u8; 0x600];
        data[..2].copy_from_slice(b"MZ");
        data[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        data[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        data[0x86..0x88].copy_from_slice(&2u16.to_le_bytes());
        data[0x94..0x96].copy_from_slice(&0xf0u16.to_le_bytes());
        data[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
        data[0xde..0xe0].copy_from_slice(&0x160u16.to_le_bytes());
        for (i, name) in [b".text\0\0\0", b".rdata\0\0"].iter().enumerate() {
            let at = 0x188 + i * 40;
            data[at..at + 8].copy_from_slice(*name);
            data[at + 16..at + 20].copy_from_slice(&0x200u32.to_le_bytes());
            data[at + 20..at + 24].copy_from_slice(&(0x200u32 + i as u32 * 0x200).to_le_bytes());
        }
        data
    }
    #[test]
    fn parses_sections_and_security_flags() {
        let (sections, flags) = describe(&fixture()).unwrap();
        assert_eq!(sections.len(), 2);
        assert!(flags.dynamic_base && flags.high_entropy_va && flags.nx_compatible);
        assert!(!flags.guard_cf_header);
    }
    #[test]
    fn rejects_truncation_overlap_and_missing_sections() {
        for length in [0, 2, 64, 128, 256, 512, 1024] {
            assert!(describe(&fixture()[..length]).is_err());
        }
        let mut data = fixture();
        data[0x1c4..0x1c8].copy_from_slice(&0x200u32.to_le_bytes());
        assert!(describe(&data).is_err());
        let mut data = fixture();
        data[0x1b0..0x1b8].copy_from_slice(b".other\0\0");
        assert!(describe(&data).is_err());
    }
    #[test]
    fn rejects_duplicate_protected_sections_and_invalid_headers() {
        let mut duplicate = fixture();
        duplicate[0x1b0..0x1b8].copy_from_slice(b".text\0\0\0");
        assert!(describe(&duplicate).is_err());
        for (offset, value) in [
            (0x84, 0x14cu16), // Unsupported x86 architecture.
            (0x98, 0x10b),    // PE32 instead of PE32+.
            (0x86, 0),        // No sections.
            (0x86, 97),       // Too many sections.
            (0x94, 111),      // Optional header too short.
        ] {
            let mut data = fixture();
            data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            assert!(describe(&data).is_err(), "offset {offset:x}");
        }
        let mut data = fixture();
        data[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(describe(&data).is_err());
    }
    #[test]
    fn changing_code_or_readonly_data_changes_digest() {
        let original = describe(&fixture()).unwrap().0;
        for offset in [0x240, 0x440] {
            let mut data = fixture();
            data[offset] ^= 1;
            assert_ne!(describe(&data).unwrap().0, original);
        }
    }
    #[test]
    fn rejects_modified_manifest_including_overlay_hash() {
        let data = fixture();
        let (sections, flags) = describe(&data).unwrap();
        let actual = ReleaseManifest {
            schema: 1,
            product: "Game Power Plan Switcher".into(),
            version: "test".into(),
            bytes: data.len(),
            sha256: digest(&data),
            sections,
            flags,
            provenance: serde_json::json!({}),
        };
        assert!(verify(&actual, &actual).is_ok());
        let mut bad = actual.clone();
        bad.sha256.replace_range(..1, "z");
        assert!(verify(&bad, &actual).is_err());
        let mut bad = actual.clone();
        bad.schema = 2;
        assert!(verify(&bad, &actual).is_err());
        let mut bad = actual.clone();
        bad.sections.pop();
        assert!(verify(&bad, &actual).is_err());
        let mut bad = actual.clone();
        bad.provenance = serde_json::json!({"git_commit":"forged"});
        assert!(verify(&bad, &actual).is_err());
    }
}
