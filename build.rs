// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn git(manifest: &Path, arguments: &[&str]) -> Option<String> {
    let result = Command::new("git")
        .arg("-C")
        .arg(manifest)
        .args(arguments)
        .output()
        .ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

fn valid_commit(value: &str) -> bool {
    value == "unknown"
        || ([40, 64].contains(&value.len()) && value.bytes().all(|c| c.is_ascii_hexdigit()))
}

fn write_build_info() {
    for name in [
        "SOURCE_DATE_EPOCH",
        "NN6_BUILD_UUID",
        "NN6_GIT_COMMIT",
        "NN6_GIT_DIRTY",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let commit = match env::var("NN6_GIT_COMMIT") {
        Ok(value) => {
            assert!(
                valid_commit(&value),
                "NN6_GIT_COMMIT must be a 40/64-character hexadecimal commit or unknown"
            );
            value.to_ascii_lowercase()
        }
        Err(_) => git(&manifest, &["rev-parse", "--verify", "HEAD"])
            .filter(|value| valid_commit(value))
            .unwrap_or_else(|| "unknown".into()),
    };
    let dirty = match env::var("NN6_GIT_DIRTY") {
        Ok(value) => {
            assert!(
                ["clean", "dirty", "unknown"].contains(&value.as_str()),
                "NN6_GIT_DIRTY must be clean, dirty or unknown"
            );
            value
        }
        Err(_) => git(
            &manifest,
            &[
                "status",
                "--porcelain",
                "--untracked-files=normal",
                "--",
                ".",
            ],
        )
        .map(|value| if value.is_empty() { "clean" } else { "dirty" }.to_owned())
        .unwrap_or_else(|| "unknown".into()),
    };
    // Watch Git metadata when present. Source archives intentionally report
    // unknown unless their publisher supplies explicit provenance variables.
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = git(
            &manifest,
            &["rev-parse", "--path-format=absolute", "--git-path", name],
        ) {
            if Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    if let Some(reference) = git(&manifest, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(
            &manifest,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                &reference,
            ],
        ) {
            if Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    let (epoch, time_source) = match env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => {
            assert!(
                !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()),
                "SOURCE_DATE_EPOCH must contain nonnegative Unix seconds"
            );
            (
                value.parse::<u64>().expect("SOURCE_DATE_EPOCH exceeds u64"),
                "SOURCE_DATE_EPOCH",
            )
        }
        Err(_) => (
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("System clock before Unix epoch")
                .as_secs(),
            "build-script clock",
        ),
    };
    let build_uuid = env::var("NN6_BUILD_UUID")
        .map(|value| {
            assert!(
                value.len() == 36
                    && value.bytes().enumerate().all(|(i, c)| {
                        if [8, 13, 18, 23].contains(&i) {
                            c == b'-'
                        } else {
                            c.is_ascii_hexdigit()
                        }
                    }),
                "NN6_BUILD_UUID must be a canonical hexadecimal UUID"
            );
            value.to_ascii_lowercase()
        })
        .unwrap_or_else(|_| "not-supplied".into());
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let authors = env::var("CARGO_PKG_AUTHORS").unwrap_or_else(|_| "NN6".into());
    let profile = env::var("PROFILE").expect("PROFILE");
    let target = env::var("TARGET").expect("TARGET");
    let hardening = "PE ASLR, NX and high-entropy VA; CFG not enabled because complete Rust/std/C++ dependency instrumentation is not established";
    let description = format!(
        "Game Power Plan Switcher {version}\nAuthors: {authors}\nCommit: {commit} ({dirty})\nBuild Unix seconds: {epoch} ({time_source})\nBuild UUID: {build_uuid}\nProfile: {profile}\nTarget: {target}\n{hardening}"
    );
    let mut source = String::from(
        "// Generated public provenance. No secrets or runtime anti-analysis logic.\n",
    );
    for (name, value) in [
        ("VERSION", version.as_str()),
        ("AUTHORS", authors.as_str()),
        ("GIT_COMMIT", commit.as_str()),
        ("GIT_DIRTY", dirty.as_str()),
        ("BUILD_TIME_SOURCE", time_source),
        ("BUILD_UUID", build_uuid.as_str()),
        ("PROFILE", profile.as_str()),
        ("TARGET", target.as_str()),
        ("HARDENING", hardening),
        ("BUILD_PROVENANCE", description.as_str()),
    ] {
        // Rust's Debug string formatting escapes quotes/control characters,
        // so external provenance inputs cannot inject generated Rust source.
        source.push_str(&format!("pub const {name}: &str = {value:?};\n"));
    }
    source.push_str(&format!("pub const BUILD_TIME_UNIX: u64 = {epoch};\n"));
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("build_info.rs"),
        source,
    )
    .expect("Write build provenance");
}

fn main() {
    write_build_info();
    // Any source change refreshes provenance when Cargo reruns this script.
    for path in ["src", "ui", "native", "assets", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=CXX");
    println!("cargo:rerun-if-env-changed=NN6_WINDRES");
    let target = env::var("TARGET").expect("TARGET");
    assert_eq!(
        target, "x86_64-pc-windows-gnullvm",
        "Use the documented x64 Windows GNU-LLVM toolchain"
    );
    // GNU/MinGW spellings supported by staged lld's i386pep driver. These are
    // image mitigations, not encryption/obfuscation or a code-signing substitute.
    // Do not advertise CFG from a linker bit alone: prebuilt Rust/C++ libraries
    // have not all been compiled with matching indirect-call instrumentation.
    for argument in [
        "--dynamicbase",
        "--nxcompat",
        "--high-entropy-va",
        "--no-insert-timestamp",
    ] {
        println!("cargo:rustc-link-arg=-Wl,{argument}");
    }
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let resource_version = format!("{}.0", version);
    let resource = fs::read_to_string("native/app.rc").expect("Read app.rc");
    assert!(
        resource.contains(&format!(
            "FILEVERSION {}",
            resource_version.replace('.', ",")
        )),
        "app.rc FILEVERSION must match Cargo package version"
    );
    assert!(
        resource.contains(&format!(
            "PRODUCTVERSION {}",
            resource_version.replace('.', ",")
        )),
        "app.rc PRODUCTVERSION must match Cargo package version"
    );
    let manifest = fs::read_to_string("native/app.manifest").expect("Read app.manifest");
    assert!(
        manifest.contains(&format!("assemblyIdentity version=\"{resource_version}\"")),
        "app.manifest must match Cargo package version"
    );
    let icon = fs::read("assets/NN6.ico").expect("Read application icon");
    assert!(
        icon.len() >= 6 && icon[..4] == [0, 0, 1, 0] && u16::from_le_bytes([icon[4], icon[5]]) == 9,
        "NN6.ico must retain all nine icon frames"
    );
    slint_build::compile("ui/app.slint").expect("Compile native UI");
    cc::Build::new()
        .cpp(true)
        .cpp_link_stdlib("c++")
        .cpp_link_stdlib_static(true)
        .std("c++23")
        .file("native/bridge.cpp")
        .file("native/pro_bridge.cpp")
        .flag_if_supported("-Wno-missing-field-initializers")
        .compile("nn6_win32");
    for lib in [
        "powrprof", "wbemuuid", "ole32", "oleaut32", "advapi32", "user32", "gdi32", "shell32",
        "comdlg32", "pdh", "dwmapi",
    ] {
        println!("cargo:rustc-link-lib={lib}");
    }
    println!("cargo:rerun-if-changed=native/bridge.cpp");
    println!("cargo:rerun-if-changed=native/pro_bridge.cpp");
    if let Ok(cxx) = std::env::var("CXX") {
        let dir = std::path::Path::new(&cxx)
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        println!(
            "cargo:rustc-link-search=native={}",
            dir.join("x86_64-w64-mingw32/lib").display()
        );
        println!(
            "cargo:rustc-link-search=native={}",
            dir.join("lib").display()
        );
    }
    println!("cargo:rerun-if-changed=native/app.rc");
    println!("cargo:rerun-if-changed=assets/NN6.ico");
    println!("cargo:rerun-if-changed=assets/icon.png");
    println!("cargo:rerun-if-changed=assets/icon.svg");
    println!("cargo:rerun-if-changed=native/app.manifest");
    {
        let compiler = std::env::var("NN6_WINDRES").expect(
            "Set NN6_WINDRES to the LLVM-MinGW windres tool; Windows resources are mandatory",
        );
        let dest = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.o");
        assert!(
            std::process::Command::new(compiler)
                .args(["-i", "native/app.rc", "-o"])
                .arg(&dest)
                .status()
                .unwrap()
                .success()
        );
        println!("cargo:rustc-link-arg={}", dest.display());
    }
}
