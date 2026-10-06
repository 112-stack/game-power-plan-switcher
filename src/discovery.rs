// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Read only installed-library metadata; review candidates before adding them.
use crate::{model::Game, win};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub fn vdf_tokens(input: &str) -> Vec<String> {
    let mut result = vec![];
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut s = String::new();
            while let Some(c) = chars.next() {
                if c == '"' {
                    break;
                }
                if c == '\\' {
                    if let Some(&next) = chars.peek() {
                        if next == '\\' || next == '"' {
                            s.push(chars.next().unwrap());
                            continue;
                        }
                    }
                }
                s.push(c);
            }
            result.push(s);
        }
    }
    result
}
fn value(tokens: &[String], key: &str) -> Option<String> {
    tokens
        .windows(2)
        .find(|p| p[0].eq_ignore_ascii_case(key))
        .map(|p| p[1].clone())
}
fn candidates(dir: &Path, depth: u32, remaining: &mut usize, result: &mut Vec<PathBuf>) {
    if depth > 4 || *remaining == 0 {
        return;
    }
    let Ok(items) = fs::read_dir(dir) else { return };
    for item in items.flatten() {
        if *remaining == 0 {
            break;
        }
        *remaining -= 1;
        let path = item.path();
        let Ok(meta) = item.file_type() else { continue };
        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            candidates(&path, depth + 1, remaining, result)
        } else if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
        {
            let n = path.file_name().unwrap().to_string_lossy().to_lowercase();
            if ![
                "unins",
                "crash",
                "setup",
                "redist",
                "helper",
                "report",
                "updater",
                "anticheat",
                "battleye",
                "cef",
                "prereq",
            ]
            .iter()
            .any(|s| n.contains(s))
            {
                result.push(path)
            }
        }
    }
}
fn import_candidate(path: &Path, name: &str, source: &str) -> Option<Game> {
    let process = path.file_name()?.to_str()?.to_owned();
    if !crate::model::valid_process(&process) {
        return None;
    }
    Some(Game {
        id: fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_lowercase(),
        name: if name.is_empty() {
            process.trim_end_matches(".exe").to_owned()
        } else {
            name.into()
        },
        processes: vec![process],
        icon: vec![],
        source: format!("{source} | {}", path.display()),
        ..Default::default()
    })
}
pub fn import(paths: &[String]) -> Result<Vec<Game>, String> {
    let mut result = vec![];
    for p in paths.iter().take(500) {
        let path = Path::new(p);
        if !path.is_file() {
            return Err(format!("Executable not found: {p}"));
        }
        let mut g = import_candidate(path, "", "Imported")
            .ok_or_else(|| format!("Invalid executable name: {p}"))?;
        g.icon = win::icon(path).unwrap_or_default();
        result.push(g);
    }
    Ok(result)
}
pub fn scan() -> Result<Vec<Game>, String> {
    // These commands only read launcher installation metadata / Appx registrations.
    let script = r#"[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new();$ErrorActionPreference='SilentlyContinue';$r=@();$s=(Get-ItemProperty 'HKCU:\Software\Valve\Steam').SteamPath;if($s){$r+=@{source='SteamRoot';path=$s;name='Steam'}};foreach($k in @('HKLM:\SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs\*','HKLM:\SOFTWARE\Ubisoft\Launcher\Installs\*')){Get-ItemProperty $k|ForEach-Object{if($_.InstallDir){$r+=@{source='Ubisoft Connect';path=$_.InstallDir;name=(Split-Path $_.InstallDir -Leaf)}}}};Get-AppxPackage|Where-Object{$_.SignatureKind -eq 'Store' -and $_.InstallLocation}|ForEach-Object{if(Test-Path (Join-Path $_.InstallLocation 'MicrosoftGame.config')){$r+=@{source='Xbox Game Pass';path=$_.InstallLocation;name=$_.Name}}};ConvertTo-Json -InputObject @($r) -Compress"#;
    let ps = format!(
        "{}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
        std::env::var("SystemRoot").unwrap_or("C:\\Windows".into())
    );
    let output = win::hidden_command(&ps)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|e| e.to_string())?;
    let roots: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let mut libraries: Vec<(PathBuf, String, String)> = vec![];
    for root in roots {
        let path = PathBuf::from(root["path"].as_str().unwrap_or(""));
        let source = root["source"].as_str().unwrap_or("");
        if source == "SteamRoot" {
            let mut steam_roots = vec![path.clone()];
            if let Ok(s) = fs::read_to_string(path.join("steamapps/libraryfolders.vdf")) {
                let t = vdf_tokens(&s);
                for p in t.windows(2) {
                    if p[0] == "path" {
                        steam_roots.push(PathBuf::from(&p[1]))
                    }
                }
            }
            for library in steam_roots {
                let steamapps = library.join("steamapps");
                let Ok(files) = fs::read_dir(&steamapps) else {
                    continue;
                };
                for file in files.flatten() {
                    let name = file.file_name().to_string_lossy().into_owned();
                    if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                        continue;
                    }
                    if let Ok(s) = fs::read_to_string(file.path()) {
                        let t = vdf_tokens(&s);
                        if let Some(folder) = value(&t, "installdir") {
                            let dir = steamapps.join("common").join(folder);
                            libraries.push((
                                dir,
                                value(&t, "name").unwrap_or_default(),
                                "Steam".into(),
                            ));
                        }
                    }
                }
            }
        } else if !path.as_os_str().is_empty() {
            libraries.push((
                path,
                root["name"].as_str().unwrap_or("").into(),
                source.into(),
            ))
        }
    }
    let epic = PathBuf::from(std::env::var("ProgramData").unwrap_or("C:\\ProgramData".into()))
        .join("Epic/EpicGamesLauncher/Data/Manifests");
    if let Ok(entries) = fs::read_dir(epic) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|s| s == "item") {
                if let Ok(data) = fs::read(entry.path()) {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&data) {
                        if let Some(path) = v["InstallLocation"].as_str() {
                            libraries.push((
                                PathBuf::from(path),
                                v["DisplayName"].as_str().unwrap_or("").into(),
                                "Epic Games".into(),
                            ))
                        }
                    }
                }
            }
        }
    }
    // GDK packages may live outside WindowsApps; these registered default roots
    // are inspected only where readable, without taking ownership or elevating.
    for root in [r"C:\XboxGames", r"D:\XboxGames"] {
        let path = Path::new(root);
        if let Ok(entries) = fs::read_dir(path) {
            for e in entries.flatten() {
                if e.path().is_dir() {
                    libraries.push((
                        e.path().join("Content"),
                        e.file_name().to_string_lossy().into(),
                        "Xbox Game Pass".into(),
                    ))
                }
            }
        }
    }
    let mut found = vec![];
    let mut seen = HashSet::new();
    for (dir, title, source) in libraries {
        if !seen.insert(dir.to_string_lossy().to_lowercase()) {
            continue;
        }
        let mut files = vec![];
        candidates(&dir, 0, &mut 4000, &mut files);
        files.sort_by_key(|p| p.components().count());
        for path in files.into_iter().take(12) {
            if let Some(g) = import_candidate(&path, &title, &source) {
                found.push(g)
            }
            if found.len() >= 500 {
                break;
            }
        }
        if found.len() >= 500 {
            break;
        }
    }
    // Steam and Ubisoft can reference the same physical installation.
    let mut executables = HashSet::new();
    found.retain(|g| executables.insert(g.id.clone()));
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(found)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vdf_escaped_paths() {
        let t = vdf_tokens(r#""libraryfolders" { "0" { "path" "D:\\SteamLibrary" } }"#);
        assert_eq!(value(&t, "path"), Some(r"D:\SteamLibrary".into()));
    }
    #[test]
    fn vdf_manifest() {
        let t = vdf_tokens(r#""AppState" { "name" "Siege" "installdir" "Rainbow Six Siege" }"#);
        assert_eq!(value(&t, "installdir"), Some("Rainbow Six Siege".into()));
    }
}
