// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Deterministic, path-sensitive build fingerprints shared with unit tests.
//!
//! Encoding v1: the ASCII domain below, then each UTF-8 relative path in byte
//! order, u64-LE path length, path bytes, u64-LE file length, raw SHA-256 bytes.
//! The individual content hashes avoid concatenation ambiguity without placing
//! a complete source tree in memory. Timestamps and absolute paths are excluded.
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

pub const ALGORITHM: &str = "sha256-path-length-content-v1";
const DOMAIN: &[u8] = b"GamePowerPlanSwitcher/source-tree/v1\0";

#[derive(Debug)]
pub struct FingerprintEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: [u8; 32],
}

pub fn hash_file(path: &Path) -> Result<([u8; 32], u64), String> {
    let mut file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or("File size overflow")?;
        hasher.update(&buffer[..count]);
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != bytes
        || after.len() != bytes
        || before.modified().ok() != after.modified().ok()
    {
        return Err(format!("{} changed during fingerprinting", path.display()));
    }
    Ok((hasher.finalize().into(), bytes))
}

pub fn aggregate(mut entries: Vec<FingerprintEntry>) -> Result<String, String> {
    entries.sort_unstable_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    let mut previous: Option<&str> = None;
    for entry in &entries {
        if entry.path.is_empty()
            || entry.path.starts_with('/')
            || entry.path.contains('\\')
            || entry
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || previous == Some(entry.path.as_str())
        {
            return Err("Source fingerprint requires unique normalized relative paths".into());
        }
        hasher.update((entry.path.len() as u64).to_le_bytes());
        hasher.update(entry.path.as_bytes());
        hasher.update(entry.bytes.to_le_bytes());
        hasher.update(entry.sha256);
        previous = Some(entry.path.as_str());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect(
    root: &Path,
    relative: &Path,
    entries: &mut Vec<FingerprintEntry>,
) -> Result<(), String> {
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    // Refuse links/junctions: a release must fingerprint its actual contained
    // source, not an external tree whose identity can change mid-build.
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(format!("Reparse point in build inputs: {}", path.display()));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(format!("Symbolic link in build inputs: {}", path.display()));
    }
    if metadata.is_dir() {
        for child in fs::read_dir(&path).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            collect(root, &relative.join(child.file_name()), entries)?;
        }
    } else if metadata.is_file() {
        let name = relative
            .to_str()
            .ok_or("Non-UTF-8 build input path")?
            .replace('\\', "/");
        let (sha256, bytes) = hash_file(&path)?;
        entries.push(FingerprintEntry {
            path: name,
            bytes,
            sha256,
        });
    } else {
        return Err(format!("Unsupported build input type: {}", path.display()));
    }
    Ok(())
}

pub fn fingerprint(root: &Path, required: &[&str], optional: &[&str]) -> Result<String, String> {
    let mut entries = Vec::new();
    for relative in required {
        collect(root, Path::new(relative), &mut entries)?;
    }
    for relative in optional {
        // Metadata errors other than missing files must not silently omit input.
        match fs::symlink_metadata(root.join(relative)) {
            Ok(_) => collect(root, Path::new(relative), &mut entries)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    aggregate(entries)
}

/// Keep placeholders lossless across both Slint @tr and Rust formatting.
fn placeholders(text: &str) -> Result<std::collections::BTreeMap<usize, usize>, String> {
    let mut result = std::collections::BTreeMap::new();
    let mut remaining = text;
    while let Some(start) = remaining.find('{') {
        let tail = &remaining[start + 1..];
        let end = tail.find('}').ok_or("Unclosed translation placeholder")?;
        let index = tail[..end]
            .parse::<usize>()
            .map_err(|_| "Use indexed {0} translation placeholders")?;
        *result.entry(index).or_insert(0) += 1;
        remaining = &tail[end + 1..];
    }
    Ok(result)
}

pub fn bundle_translations(catalogs: &Path, out: &Path) -> Result<std::path::PathBuf, String> {
    use std::collections::BTreeMap;
    let read = |language: &str| -> Result<BTreeMap<String, String>, String> {
        let path = catalogs.join(format!("{language}.json"));
        serde_json::from_slice(&fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    let english = read("en")?;
    if english.is_empty()
        || english
            .iter()
            .any(|(key, value)| key != value || key.is_empty())
    {
        return Err("English catalog must contain nonempty source text mapped to itself".into());
    }
    let destination = out.join("translations");
    for language in ["ar", "es", "pt-BR", "fr", "de", "ru", "zh-CN"] {
        let translated = read(language)?;
        if english.keys().ne(translated.keys()) {
            return Err(format!("{language}: translation keys do not match English"));
        }
        let header = format!(
            "Content-Type: text/plain; charset=UTF-8\nLanguage: {language}\nMIME-Version: 1.0\nContent-Transfer-Encoding: 8bit\n"
        );
        let mut po = format!(
            "msgid \"\"\nmsgstr {}\n\n",
            serde_json::to_string(&header).unwrap()
        );
        for (key, value) in &translated {
            if value.trim().is_empty() || placeholders(key)? != placeholders(value)? {
                return Err(format!(
                    "{language}: empty translation or changed placeholders: {key}"
                ));
            }
            po.push_str(&format!(
                "msgid {}\nmsgstr {}\n\n",
                serde_json::to_string(key).unwrap(),
                serde_json::to_string(value).unwrap()
            ));
        }
        let directory = destination.join(language).join("LC_MESSAGES");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        fs::write(directory.join("game-power-plan-switcher.po"), po).map_err(|e| e.to_string())?;
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translation_bundle_writes_utf8_po_and_rejects_broken_catalogs() {
        let directory = std::env::temp_dir().join(format!(
            "gpps-translations-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let languages = ["en", "ar", "es", "pt-BR", "fr", "de", "ru", "zh-CN"];
        for language in languages {
            fs::write(
                directory.join(format!("{language}.json")),
                r#"{"Value {0}":"Value {0}"}"#,
            )
            .unwrap();
        }
        fs::write(directory.join("ar.json"), r#"{"Value {0}":"القيمة {0}"}"#).unwrap();
        let out = directory.join("out");
        let bundle = bundle_translations(&directory, &out).unwrap();
        let po =
            fs::read_to_string(bundle.join("ar/LC_MESSAGES/game-power-plan-switcher.po")).unwrap();
        assert!(po.contains("القيمة {0}"));
        assert!(po.contains("charset=UTF-8"));
        fs::write(directory.join("ar.json"), r#"{"Value {0}":"القيمة"}"#).unwrap();
        assert!(bundle_translations(&directory, &out).is_err());
        fs::write(directory.join("ar.json"), r#"{"Other":"Other"}"#).unwrap();
        assert!(bundle_translations(&directory, &out).is_err());
        // This generated, uniquely named temporary directory contains only this fixture.
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn translation_placeholders_allow_reordering_but_not_loss_or_duplication() {
        assert_eq!(
            placeholders("{0} of {1}").unwrap(),
            placeholders("{1} : {0}").unwrap()
        );
        assert_ne!(
            placeholders("{0}").unwrap(),
            placeholders("{0} {0}").unwrap()
        );
        assert_ne!(placeholders("{0}").unwrap(), placeholders("{1}").unwrap());
        assert!(placeholders("{broken}").is_err());
        assert!(placeholders("{0").is_err());
    }
    fn entry(path: &str, data: &[u8]) -> FingerprintEntry {
        FingerprintEntry {
            path: path.into(),
            bytes: data.len() as u64,
            sha256: Sha256::digest(data).into(),
        }
    }

    #[test]
    fn fingerprint_is_order_independent_and_matches_versioned_vector() {
        let first = aggregate(vec![
            entry("ui/app.slint", b"ui"),
            entry("src/main.rs", b"main"),
        ])
        .unwrap();
        let reverse = aggregate(vec![
            entry("src/main.rs", b"main"),
            entry("ui/app.slint", b"ui"),
        ])
        .unwrap();
        assert_eq!(first, reverse);
        // This independent fixed vector makes changes to byte framing explicit.
        assert_eq!(
            first,
            "16a5083a52b06ed0fc645d27738a0bc936a7d905157d70d0cac71a5741675ddf"
        );
        assert_eq!(ALGORITHM, "sha256-path-length-content-v1");
    }

    #[test]
    fn fingerprint_detects_content_path_and_length_changes() {
        let original = aggregate(vec![entry("src/main.rs", b"main")]).unwrap();
        for changed in [
            entry("src/main.rs", b"Main"),
            entry("src/other.rs", b"main"),
            entry("src/main.rs", b"main\0"),
        ] {
            assert_ne!(original, aggregate(vec![changed]).unwrap());
        }
        let mut changed = entry("src/main.rs", b"main");
        changed.bytes += 1;
        assert_ne!(original, aggregate(vec![changed]).unwrap());
        assert_ne!(
            aggregate(vec![entry("a", b"bc")]).unwrap(),
            aggregate(vec![entry("ab", b"c")]).unwrap()
        );
    }

    #[test]
    fn fingerprint_rejects_duplicate_and_noncanonical_paths() {
        assert!(aggregate(vec![entry("a", b"one"), entry("a", b"two")]).is_err());
        for path in ["", "/root", "../a", "a//b", "a/./b", "a\\b"] {
            assert!(aggregate(vec![entry(path, b"data")]).is_err(), "{path}");
        }
    }

    #[test]
    fn directory_fingerprint_tracks_added_and_removed_inputs() {
        let directory = std::env::temp_dir().join(format!(
            "nn6-fingerprint-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let tree = directory.join("src");
        fs::create_dir(&tree).unwrap();
        fs::write(tree.join("a.rs"), b"a").unwrap();
        let first = fingerprint(&directory, &["src"], &["absent.toml"]).unwrap();
        fs::write(tree.join("b.rs"), b"b").unwrap();
        assert_ne!(first, fingerprint(&directory, &["src"], &[]).unwrap());
        fs::remove_file(tree.join("b.rs")).unwrap();
        assert_eq!(first, fingerprint(&directory, &["src"], &[]).unwrap());
        assert!(fingerprint(&directory, &["missing"], &[]).is_err());
        fs::remove_file(tree.join("a.rs")).unwrap();
        fs::remove_dir(tree).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
