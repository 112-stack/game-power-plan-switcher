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

#[cfg(test)]
#[path = "../build_support.rs"]
mod build_support;

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
    // Optional only so the legacy schema1 can still be read. validate_schema()
    // requires every field for schema2 and rejects them on schema1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<ToolchainProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tree_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tree_algorithm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing: Option<SigningMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolchainProvenance {
    /// Exact stdout from the compiler's rustc -vV, trimmed at the ends only.
    pub rustc: String,
    pub rustc_commit_hash: String,
    /// Distribution tag explicitly supplied by the publisher, otherwise unknown.
    pub llvm_mingw: String,
    pub cargo_lock_sha256: String,
    pub build_inputs_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct SigningMetadata {
    /// An external publisher assertion, never an Authenticode trust decision.
    /// Sign-Release/Build-Signed-Release must independently verify the PE first.
    pub signed: bool,
    pub signer_subject: Option<String>,
    pub signer_thumbprint: Option<String>,
    pub not_after: Option<String>,
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Accept a real UTC RFC3339 calendar time (optionally fractional seconds).
/// Certificate validity/trust is deliberately checked by Windows, not here.
fn valid_utc_timestamp(value: &str) -> bool {
    let Some(value) = value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix("+00:00"))
    else {
        return false;
    };
    let (time, fraction) = value
        .split_once('.')
        .map_or((value, None), |(a, b)| (a, Some(b)));
    if fraction.is_some_and(|digits| {
        digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit())
    }) {
        return false;
    }
    if time.len() != 19
        || time.as_bytes()[4] != b'-'
        || time.as_bytes()[7] != b'-'
        || time.as_bytes()[10] != b'T'
        || time.as_bytes()[13] != b':'
        || time.as_bytes()[16] != b':'
    {
        return false;
    }
    let bytes = time.as_bytes();
    if bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| ![4, 7, 10, 13, 16].contains(&index) && !byte.is_ascii_digit())
    {
        return false;
    }
    let number = |range: std::ops::Range<usize>| {
        bytes[range]
            .iter()
            .fold(0u32, |n, b| n * 10 + u32::from(b - b'0'))
    };
    let year = number(0..4);
    let month = number(5..7);
    let day = number(8..10);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return false,
    };
    year > 0
        && day > 0
        && day <= max_day
        && number(11..13) < 24
        && number(14..16) < 60
        && number(17..19) < 60
}

fn validate_signing(signing: &SigningMetadata) -> Result<(), String> {
    if !signing.signed {
        if signing.signer_subject.is_some()
            || signing.signer_thumbprint.is_some()
            || signing.not_after.is_some()
        {
            return Err("Unsigned manifest must not claim a signer or certificate expiry".into());
        }
        return Ok(());
    }
    let subject = signing
        .signer_subject
        .as_deref()
        .ok_or("Signed manifest lacks signer subject")?;
    let thumbprint = signing
        .signer_thumbprint
        .as_deref()
        .ok_or("Signed manifest lacks certificate thumbprint")?;
    let expiry = signing
        .not_after
        .as_deref()
        .ok_or("Signed manifest lacks certificate expiry")?;
    if subject.trim().is_empty()
        || subject.len() > 4096
        || subject.chars().any(char::is_control)
        || thumbprint.len() != 40
        || !thumbprint.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !valid_utc_timestamp(expiry)
    {
        return Err("Malformed external signing metadata".into());
    }
    Ok(())
}

fn validate_schema(manifest: &ReleaseManifest) -> Result<(), String> {
    match manifest.schema {
        1 => {
            if manifest.toolchain.is_some()
                || manifest.source_tree_sha256.is_some()
                || manifest.source_tree_algorithm.is_some()
                || manifest.signing.is_some()
                || manifest.provenance.get("manifest_schema").is_some()
            {
                return Err("Legacy schema1 must not contain schema2 provenance or fields".into());
            }
        }
        2 => {
            if manifest
                .provenance
                .get("manifest_schema")
                .and_then(serde_json::Value::as_u64)
                != Some(2)
            {
                return Err("Schema2 requires the embedded manifest_schema marker".into());
            }
            let toolchain = manifest
                .toolchain
                .as_ref()
                .ok_or("Schema2 lacks toolchain provenance")?;
            let source_hash = manifest
                .source_tree_sha256
                .as_deref()
                .ok_or("Schema2 lacks source tree digest")?;
            if manifest.source_tree_algorithm.as_deref() != Some("sha256-path-length-content-v1")
                || !valid_sha256(source_hash)
                || !valid_sha256(&toolchain.cargo_lock_sha256)
                || !valid_sha256(&toolchain.build_inputs_sha256)
                || toolchain.rustc.trim().is_empty()
                || toolchain.rustc.len() > 16384
                || toolchain.llvm_mingw.is_empty()
                || toolchain.llvm_mingw.len() > 128
                || !toolchain
                    .llvm_mingw
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
                || !(toolchain.rustc_commit_hash == "unknown"
                    || ([40, 64].contains(&toolchain.rustc_commit_hash.len())
                        && toolchain
                            .rustc_commit_hash
                            .bytes()
                            .all(|c| c.is_ascii_hexdigit())))
                || !toolchain.rustc.lines().any(|line| {
                    line.strip_prefix("commit-hash: ") == Some(toolchain.rustc_commit_hash.as_str())
                })
            {
                return Err("Malformed schema2 toolchain or source provenance".into());
            }
            validate_signing(
                manifest
                    .signing
                    .as_ref()
                    .ok_or("Schema2 lacks signing metadata")?,
            )?;
        }
        _ => return Err("Unsupported release manifest schema".into()),
    }
    if !valid_sha256(&manifest.sha256)
        || manifest
            .sections
            .iter()
            .any(|section| !valid_sha256(&section.sha256))
    {
        return Err("Malformed executable or section SHA-256".into());
    }
    Ok(())
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
    serde_json::json!({"manifest_schema": 2, "version": build_info::VERSION, "authors": build_info::AUTHORS,
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
        schema: 2,
        product: "Game Power Plan Switcher".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        bytes: data.len(),
        sha256: digest(&data),
        sections,
        flags,
        provenance: provenance(),
        toolchain: Some(ToolchainProvenance {
            rustc: build_info::RUSTC_VERBOSE_VERSION.into(),
            rustc_commit_hash: build_info::RUSTC_COMMIT_HASH.into(),
            llvm_mingw: build_info::LLVM_MINGW_VERSION.into(),
            cargo_lock_sha256: build_info::CARGO_LOCK_SHA256.into(),
            build_inputs_sha256: build_info::BUILD_INPUTS_SHA256.into(),
        }),
        source_tree_sha256: Some(build_info::SOURCE_TREE_SHA256.into()),
        source_tree_algorithm: Some(build_info::SOURCE_TREE_ALGORITHM.into()),
        // This CLI performs no signature/trust-store/network operations. The
        // explicit signing script replaces this default after verification.
        signing: Some(SigningMetadata::default()),
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
    let expected = parse_manifest(&read_file_bounded(path, 1024 * 1024)?)?;
    let actual = inspect(&std::env::current_exe().map_err(|e| e.to_string())?)?;
    verify(&expected, &actual)
}

fn parse_manifest(bytes: &[u8]) -> Result<ReleaseManifest, String> {
    // Option<T> intentionally accepts null for compatibility with legacy Rust
    // types, so separately reject v2 keys in v1 even when their value is null.
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid release manifest: {e}"))?;
    let object = value
        .as_object()
        .ok_or("Release manifest must be an object")?;
    let additions = [
        "toolchain",
        "source_tree_sha256",
        "source_tree_algorithm",
        "signing",
    ];
    match object.get("schema").and_then(serde_json::Value::as_u64) {
        Some(1) if additions.iter().any(|name| object.contains_key(*name)) => {
            return Err("Legacy schema1 must not contain schema2 keys, even null values".into());
        }
        Some(2) => {
            if additions.iter().any(|name| !object.contains_key(*name)) {
                return Err("Schema2 is missing mandatory provenance or signing fields".into());
            }
            let signing = object
                .get("signing")
                .and_then(serde_json::Value::as_object)
                .ok_or("Schema2 signing must be an object")?;
            if ["signed", "signer_subject", "signer_thumbprint", "not_after"]
                .iter()
                .any(|name| !signing.contains_key(*name))
            {
                return Err("Schema2 signing is missing mandatory fields".into());
            }
        }
        _ => {}
    }
    // Deserialize original bytes, not Value: serde then still rejects duplicate
    // struct fields and deny_unknown_fields prevents silently ignored metadata.
    let manifest: ReleaseManifest =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid release manifest: {e}"))?;
    validate_schema(&manifest)?;
    Ok(manifest)
}
fn verify(expected: &ReleaseManifest, actual: &ReleaseManifest) -> Result<(), String> {
    validate_schema(expected)?;
    validate_schema(actual)?;
    if expected.schema != actual.schema
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
    if expected.toolchain != actual.toolchain
        || expected.source_tree_sha256 != actual.source_tree_sha256
        || expected.source_tree_algorithm != actual.source_tree_algorithm
    {
        return Err(
            "Release toolchain or source fingerprint differs from embedded provenance".into(),
        );
    }
    // Signing facts are external metadata. Shape was checked above, but equality
    // with our unsigned default would reject legitimate post-build signing.
    // The companion verifier checks the actual certificate independently before
    // invoking this CLI. A matching manifest alone never authenticates a release.
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
            toolchain: None,
            source_tree_sha256: None,
            source_tree_algorithm: None,
            signing: None,
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

    fn manifest_fixture(schema: u32) -> ReleaseManifest {
        let data = fixture();
        let (sections, flags) = describe(&data).unwrap();
        let mut manifest = ReleaseManifest {
            schema,
            product: "Game Power Plan Switcher".into(),
            version: "test".into(),
            bytes: data.len(),
            sha256: digest(&data),
            sections,
            flags,
            provenance: serde_json::json!({"version":"test"}),
            toolchain: None,
            source_tree_sha256: None,
            source_tree_algorithm: None,
            signing: None,
        };
        if schema == 2 {
            let commit = "a".repeat(40);
            manifest.provenance["manifest_schema"] = serde_json::json!(2);
            manifest.toolchain = Some(ToolchainProvenance {
                rustc: format!("rustc test\ncommit-hash: {commit}\nLLVM version: test"),
                rustc_commit_hash: commit,
                llvm_mingw: "20260922".into(),
                cargo_lock_sha256: digest(b"lock"),
                build_inputs_sha256: digest(b"inputs"),
            });
            manifest.source_tree_sha256 = Some(digest(b"source tree"));
            manifest.source_tree_algorithm = Some("sha256-path-length-content-v1".into());
            manifest.signing = Some(SigningMetadata::default());
        }
        manifest
    }

    #[test]
    fn schema1_legacy_and_schema2_manifests_roundtrip() {
        for schema in [1, 2] {
            let actual = manifest_fixture(schema);
            let encoded = serde_json::to_vec(&actual).unwrap();
            let decoded = parse_manifest(&encoded).unwrap();
            assert!(verify(&decoded, &actual).is_ok());
        }
    }

    #[test]
    fn schema2_rejects_removed_fields_and_legacy_downgrade() {
        let actual = manifest_fixture(2);
        for key in [
            "toolchain",
            "source_tree_sha256",
            "source_tree_algorithm",
            "signing",
        ] {
            let mut value = serde_json::to_value(&actual).unwrap();
            value.as_object_mut().unwrap().remove(key);
            assert!(
                parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err(),
                "{key}"
            );
        }
        let mut downgraded = actual.clone();
        downgraded.schema = 1;
        assert!(verify(&downgraded, &actual).is_err());
        downgraded.toolchain = None;
        downgraded.source_tree_sha256 = None;
        downgraded.source_tree_algorithm = None;
        downgraded.signing = None;
        assert!(verify(&downgraded, &actual).is_err());
        downgraded
            .provenance
            .as_object_mut()
            .unwrap()
            .remove("manifest_schema");
        assert!(verify(&downgraded, &actual).is_err());
        let mut legacy = serde_json::to_value(manifest_fixture(1)).unwrap();
        legacy["signing"] = serde_json::Value::Null;
        assert!(parse_manifest(&serde_json::to_vec(&legacy).unwrap()).is_err());
    }

    #[test]
    fn schema2_rejects_changed_toolchain_and_source_provenance() {
        let actual = manifest_fixture(2);
        for field in [
            "rustc",
            "rustc_commit_hash",
            "llvm_mingw",
            "cargo_lock_sha256",
            "build_inputs_sha256",
        ] {
            let mut bad = serde_json::to_value(&actual).unwrap();
            bad["toolchain"][field] = serde_json::json!("b".repeat(64));
            let failed = parse_manifest(&serde_json::to_vec(&bad).unwrap())
                .and_then(|expected| verify(&expected, &actual));
            assert!(failed.is_err(), "{field}");
        }
        let mut bad = actual.clone();
        bad.source_tree_sha256 = Some(digest(b"changed source"));
        assert!(verify(&bad, &actual).is_err());
        bad.source_tree_algorithm = Some("unframed".into());
        assert!(verify(&bad, &actual).is_err());
        bad.schema = 3;
        assert!(verify(&bad, &actual).is_err());
    }

    #[test]
    fn signing_metadata_is_explicit_external_data_not_a_trust_decision() {
        let actual = manifest_fixture(2);
        let mut signed = actual.clone();
        signed.signing = Some(SigningMetadata {
            signed: true,
            signer_subject: Some("CN=Example Publisher".into()),
            signer_thumbprint: Some("A".repeat(40)),
            not_after: Some("2030-10-07T12:34:56.0000000Z".into()),
        });
        // Shape and exact executable are checked; the caller must still verify
        // the real Authenticode chain. Never equate this success with trust.
        assert!(verify(&signed, &actual).is_ok());
        signed.signing.as_mut().unwrap().signed = false;
        assert!(verify(&signed, &actual).is_err());
        signed.signing.as_mut().unwrap().signed = true;
        signed.signing.as_mut().unwrap().not_after = Some("2030-02-30T00:00:00Z".into());
        assert!(verify(&signed, &actual).is_err());
        signed.signing.as_mut().unwrap().not_after = Some("2030-10-07T12:34:56+00:00".into());
        signed.signing.as_mut().unwrap().signer_thumbprint = Some("z".repeat(40));
        assert!(verify(&signed, &actual).is_err());
    }

    #[test]
    fn manifest_parser_rejects_unknown_duplicate_or_missing_signing_fields() {
        let actual = manifest_fixture(2);
        let mut value = serde_json::to_value(&actual).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(&actual).unwrap();
        value["signing"]["trusted"] = serde_json::json!(true);
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        let mut value = serde_json::to_value(&actual).unwrap();
        value["signing"]
            .as_object_mut()
            .unwrap()
            .remove("not_after");
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        let encoded = serde_json::to_string(&actual).unwrap();
        let duplicated = encoded.replacen("\"schema\":2", "\"schema\":2,\"schema\":2", 1);
        assert!(parse_manifest(duplicated.as_bytes()).is_err());
    }

    #[test]
    fn certificate_dates_are_utc_calendar_times() {
        for valid in [
            "2028-02-29T23:59:59Z",
            "2030-01-01T00:00:00.1+00:00",
            "2030-01-01T00:00:00.0000000Z",
        ] {
            assert!(valid_utc_timestamp(valid), "{valid}");
        }
        for invalid in [
            "",
            "now",
            "0000-01-01T00:00:00Z",
            "2027-02-29T00:00:00Z",
            "2028-13-01T00:00:00Z",
            "2028-01-00T00:00:00Z",
            "2028-01-01T24:00:00Z",
            "2028-01-01T00:00:00+03:00",
            "2028-01-01T00:00:00.Z",
            "éééé-01-01T00:00:00Z",
        ] {
            assert!(!valid_utc_timestamp(invalid), "{invalid}");
        }
    }
}
