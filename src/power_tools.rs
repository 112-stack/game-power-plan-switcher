// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Explicit power-plan file operations. Never selects, edits or deletes a plan.
use std::{
    io::Read,
    path::Path,
    process::Stdio,
    thread,
    time::{Duration, Instant},
};
#[repr(C)]
struct Guid {
    a: u32,
    b: u16,
    c: u16,
    d: [u8; 8],
}
#[link(name = "ole32")]
unsafe extern "system" {
    fn CoCreateGuid(guid: *mut Guid) -> i32;
}
fn fresh_guid() -> Result<String, String> {
    let mut g = Guid {
        a: 0,
        b: 0,
        c: 0,
        d: [0; 8],
    };
    if unsafe { CoCreateGuid(&mut g) } < 0 {
        return Err("Windows could not create a plan identifier".into());
    }
    Ok(format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.a, g.b, g.c, g.d[0], g.d[1], g.d[2], g.d[3], g.d[4], g.d[5], g.d[6], g.d[7]
    ))
}
pub fn valid_guid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn run(args: &[std::ffi::OsString], elevated: bool) -> Result<String, String> {
    if elevated {
        crate::system_integration::elevated_powercfg(args)?;
        return Ok(String::new());
    }
    let exe = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .ok_or("SystemRoot unavailable")?
        .join("System32/powercfg.exe");
    let mut child = crate::win::hidden_command(&exe.to_string_lossy())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let reader = |pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut data = Vec::new();
            let _ = pipe.take(131072).read_to_end(&mut data);
            String::from_utf8_lossy(&data).into_owned()
        })
    };
    let stdout = reader(Box::new(out));
    let stderr = reader(Box::new(err));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("powercfg timed out after 30 seconds".into());
        }
        thread::sleep(Duration::from_millis(50));
    };
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();
    if status.success() {
        Ok(out.trim().into())
    } else {
        Err(format!(
            "powercfg {}: {} {}",
            status,
            out.trim(),
            err.trim()
        ))
    }
}
pub fn export(guid: &str, path: &Path) -> Result<String, String> {
    export_with(guid, path, false)
}
fn export_with(guid: &str, path: &Path, elevated: bool) -> Result<String, String> {
    if !valid_guid(guid) {
        return Err("Invalid power plan GUID".into());
    }
    run(
        &["/export".into(), path.as_os_str().into(), guid.into()],
        elevated,
    )?;
    if !path.is_file() {
        return Err("powercfg did not create the requested file".into());
    }
    Ok(format!("Exported {}", path.display()))
}
pub fn import(path: &Path) -> Result<String, String> {
    import_with(path, false)
}
fn import_with(path: &Path, elevated: bool) -> Result<String, String> {
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("pow"))
    {
        return Err("Select an existing .pow file".into());
    }
    let id = fresh_guid()?;
    run(
        &["/import".into(), path.as_os_str().into(), id.clone().into()],
        elevated,
    )
    .map_err(|e| format!("Import requested GUID {id}; outcome may be incomplete: {e}"))?;
    if !crate::win::plans()?
        .iter()
        .any(|p| p.guid.eq_ignore_ascii_case(&id))
    {
        return Err(format!("Imported scheme {id} could not be verified"));
    }
    Ok(format!("Imported new plan {id}; no activation requested"))
}
pub fn duplicate(guid: &str) -> Result<String, String> {
    duplicate_with(guid, false)
}
fn duplicate_with(guid: &str, elevated: bool) -> Result<String, String> {
    if !valid_guid(guid) {
        return Err("Invalid power plan GUID".into());
    }
    let id = fresh_guid()?;
    run(
        &["/duplicatescheme".into(), guid.into(), id.clone().into()],
        elevated,
    )
    .map_err(|e| format!("Duplicate requested GUID {id}; outcome may be incomplete: {e}"))?;
    if !crate::win::plans()?
        .iter()
        .any(|p| p.guid.eq_ignore_ascii_case(&id))
    {
        return Err(format!("Duplicated scheme {id} could not be verified"));
    }
    Ok(format!(
        "Created independent plan {id}; no activation requested"
    ))
}
#[derive(Clone)]
pub enum Operation {
    Export(String, std::path::PathBuf),
    Import(std::path::PathBuf),
    Duplicate(String),
}
impl Operation {
    pub fn run(&self, elevated: bool) -> Result<String, String> {
        match self {
            Self::Export(g, p) => export_with(g, p, elevated),
            Self::Import(p) => import_with(p, elevated),
            Self::Duplicate(g) => duplicate_with(g, elevated),
        }
    }
}
pub fn needs_elevation(error: &str) -> bool {
    let error = error.to_lowercase();
    error.contains("0x522")
        || error.contains("required privilege")
        || error.contains("access is denied")
        || error.contains("(0x5)")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guid_is_strict_and_generated_ids_unique() {
        assert!(valid_guid(crate::model::BALANCED));
        assert!(!valid_guid("/delete SCHEME_CURRENT"));
        assert!(!valid_guid("../../plan"));
        let a = fresh_guid().unwrap();
        let b = fresh_guid().unwrap();
        assert!(valid_guid(&a));
        assert_ne!(a, b);
    }
}
