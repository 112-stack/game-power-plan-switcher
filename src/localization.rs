// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
//! Presentation-only localization. Protocols, file names, process identities,
//! Windows plan names and exported diagnostics retain their original values.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

pub const CODES: [&str; 9] = [
    "system", "en", "ar", "es", "pt-BR", "fr", "de", "ru", "zh-CN",
];
const NAMES: [&str; 9] = [
    "System default",
    "English · en",
    "العربية · Arabic",
    "Español · Spanish",
    "Português (Brasil) · Portuguese",
    "Français · French",
    "Deutsch · German",
    "Русский · Russian",
    "简体中文 · Simplified Chinese",
];
static ACTIVE: AtomicUsize = AtomicUsize::new(1);
type Catalog = BTreeMap<String, String>;
static CATALOGS: [OnceLock<Catalog>; 7] = [const { OnceLock::new() }; 7];
const CATALOG_JSON: [&str; 8] = [
    include_str!("../ui/translations/en.json"),
    include_str!("../ui/translations/ar.json"),
    include_str!("../ui/translations/es.json"),
    include_str!("../ui/translations/pt-BR.json"),
    include_str!("../ui/translations/fr.json"),
    include_str!("../ui/translations/de.json"),
    include_str!("../ui/translations/ru.json"),
    include_str!("../ui/translations/zh-CN.json"),
];

fn catalog_at(locale: usize, caches: &[OnceLock<Catalog>; 7]) -> Option<&Catalog> {
    // English is the source string, so it allocates no catalog. Other languages
    // parse only on their first actual Rust presentation lookup.
    (2..=8).contains(&locale).then(|| {
        caches[locale - 2]
            .get_or_init(|| serde_json::from_str(CATALOG_JSON[locale - 1]).unwrap_or_default())
    })
}

/// Normalize a requested interface language. Unsupported Chinese script/region
/// preferences fall back to English instead of mislabeling Simplified Chinese.
pub fn normalize(value: &str) -> &'static str {
    let value = value.trim().replace('_', "-").to_ascii_lowercase();
    let language = value.split('-').next().unwrap_or("");
    match language {
        "system" => "system",
        "en" => "en",
        "ar" => "ar",
        "es" => "es",
        "pt" => "pt-BR",
        "fr" => "fr",
        "de" => "de",
        "ru" => "ru",
        "zh" if value == "zh"
            || value == "zh-cn"
            || value == "zh-sg"
            || value.starts_with("zh-hans") =>
        {
            "zh-CN"
        }
        _ => "en",
    }
}

pub fn index(value: &str) -> i32 {
    CODES
        .iter()
        .position(|code| *code == normalize(value))
        .unwrap_or(1) as i32
}

pub fn code(index: i32) -> Option<&'static str> {
    usize::try_from(index)
        .ok()
        .and_then(|index| CODES.get(index).copied())
}

pub fn resolved(requested: &str, system_locale: &str) -> &'static str {
    match normalize(requested) {
        "system" => match normalize(system_locale) {
            "system" => "en",
            code => code,
        },
        code => code,
    }
}

fn resolve_with(requested: &str, system_locale: impl FnOnce() -> String) -> &'static str {
    let requested = normalize(requested);
    if requested == "system" {
        resolved(requested, &system_locale())
    } else {
        requested
    }
}

fn refresh_with(
    requested: &str,
    active: &str,
    system_locale: impl FnOnce() -> String,
) -> Option<&'static str> {
    if normalize(requested) != "system" {
        return None;
    }
    let next = resolve_with(requested, system_locale);
    (next != active).then_some(next)
}

/// Check only on a Windows settings notification or window restore/focus event.
/// Explicit language choices bypass Windows entirely and cannot be overwritten.
pub fn system_refresh_target(requested: &str) -> Option<&'static str> {
    refresh_with(
        requested,
        CODES[ACTIVE.load(Ordering::Relaxed).clamp(1, 8)],
        windows_ui_locale,
    )
}

pub fn preview_request(saved: &str, dry_run: bool, preview: Option<&str>) -> &'static str {
    // Test/preview overrides can never change the normal app's selected locale.
    normalize(if dry_run {
        preview.unwrap_or(saved)
    } else {
        saved
    })
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
    fn LCIDToLocaleName(locale: u32, name: *mut u16, capacity: i32, flags: u32) -> i32;
}

fn windows_ui_locale() -> String {
    let mut buffer = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH, including terminator.
    let count = unsafe {
        LCIDToLocaleName(
            GetUserDefaultUILanguage() as u32,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            0,
        )
    };
    if count > 1 && count as usize <= buffer.len() {
        String::from_utf16_lossy(&buffer[..count as usize - 1])
    } else {
        "en".into()
    }
}

/// Must run on Slint's UI thread after AppWindow construction. Both Slint and
/// Rust presentation strings change together; a bundle failure keeps English.
pub fn select(requested: &str) -> Result<&'static str, String> {
    let locale = resolve_with(requested, windows_ui_locale);
    match slint::select_bundled_translation(locale) {
        Ok(()) => {
            ACTIVE.store(index(locale) as usize, Ordering::Relaxed);
            Ok(locale)
        }
        Err(error) => {
            let _ = slint::select_bundled_translation("en");
            ACTIVE.store(1, Ordering::Relaxed);
            Err(error.to_string())
        }
    }
}

pub fn is_rtl() -> bool {
    ACTIVE.load(Ordering::Relaxed) == 2
}

fn font_for_language<'a>(language: &str, preferred: &'a str) -> &'a str {
    match language {
        "zh-CN" => "Microsoft YaHei UI",
        "ar" => "Segoe UI",
        _ => preferred,
    }
}

/// Use Windows' installed script-appropriate UI families. The numeric-only
/// readouts retain their independently configured monospace font.
pub fn interface_font(preferred: &str) -> String {
    let code = CODES[ACTIVE.load(Ordering::Relaxed).clamp(1, 8)];
    font_for_language(code, preferred).to_owned()
}

pub fn names() -> Vec<slint::SharedString> {
    NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if i == 0 {
                tr(name).into()
            } else {
                (*name).into()
            }
        })
        .collect()
}

pub fn filter(query: &str) -> Vec<i32> {
    let query = query.trim().to_lowercase();
    NAMES
        .iter()
        .zip(CODES)
        .enumerate()
        .filter_map(|(index, (name, code))| {
            let label = if index == 0 {
                tr(name)
            } else {
                (*name).to_owned()
            };
            (query.is_empty()
                || format!("{label} {name} {code}")
                    .to_lowercase()
                    .contains(&query))
            .then_some(index as i32)
        })
        .collect()
}

fn translate<'a>(catalog: &'a Catalog, message: &'a str) -> &'a str {
    catalog
        .get(message)
        .filter(|s| !s.trim().is_empty())
        .map(String::as_str)
        .unwrap_or(message)
}

pub fn tr(message: &str) -> String {
    let locale = ACTIVE.load(Ordering::Relaxed).clamp(1, 8);
    catalog_at(locale, &CATALOGS)
        .map(|catalog| translate(catalog, message))
        .unwrap_or(message)
        .to_owned()
}

/// Indexed placeholders allow natural reordering. Replacement is a single pass:
/// an argument containing `{1}` is data and must never become another template.
pub fn format(message: &str, args: &[&str]) -> String {
    if ACTIVE.load(Ordering::Relaxed) == 1 {
        return interpolate(message, args);
    }
    let translated = tr(message);
    let template = if placeholders(&translated) == placeholders(message) {
        &translated
    } else {
        message
    };
    interpolate(template, args)
}

fn placeholders(template: &str) -> BTreeSet<usize> {
    let mut result = BTreeSet::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('}') else { break };
        if let Ok(index) = rest[..end].parse::<usize>() {
            result.insert(index);
        }
        rest = &rest[end + 1..];
    }
    result
}

fn interpolate(template: &str, args: &[&str]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find('}') else { break };
        match rest[1..end]
            .parse::<usize>()
            .ok()
            .and_then(|index| args.get(index))
        {
            Some(argument) => output.push_str(argument),
            None => output.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_font_uses_script_coverage_without_overriding_other_locales() {
        assert_eq!(
            font_for_language("zh-CN", "Segoe UI Variable"),
            "Microsoft YaHei UI"
        );
        assert_eq!(font_for_language("ar", "Segoe UI Variable"), "Segoe UI");
        assert_eq!(font_for_language("de", "Inter"), "Inter");
        assert_eq!(
            font_for_language("en", "Segoe UI Variable"),
            "Segoe UI Variable"
        );
    }

    #[test]
    fn locale_selection_normalizes_regions_without_guessing_unsupported_scripts() {
        for (input, expected) in [
            ("ar-SA", "ar"),
            (" es_MX ", "es"),
            ("pt-PT", "pt-BR"),
            ("de-DE", "de"),
            ("zh-Hans-CN", "zh-CN"),
            ("zh-TW", "en"),
            ("zh-Hant", "en"),
            ("ja-JP", "en"),
            ("SYSTEM", "system"),
        ] {
            assert_eq!(normalize(input), expected);
        }
        assert_eq!(resolved("system", "fr-CA"), "fr");
        assert_eq!(resolved("ru", "ar-SA"), "ru");
        assert_eq!(resolved("system", "unknown"), "en");
        assert_eq!(code(-1), None);
        assert_eq!(code(9), None);
    }

    #[test]
    fn preview_override_cannot_change_normal_app_selection() {
        assert_eq!(preview_request("en", false, Some("ar")), "en");
        assert_eq!(preview_request("en", true, Some("ar")), "ar");
        assert_eq!(preview_request("system", true, None), "system");
    }

    #[test]
    fn picker_filters_native_names_english_names_codes_and_empty_results() {
        assert_eq!(filter(""), (0..9).collect::<Vec<_>>());
        assert_eq!(filter("العربية"), vec![2]);
        assert_eq!(filter("RUSSIAN"), vec![7]);
        assert_eq!(filter("pt-BR"), vec![4]);
        assert_eq!(filter("简体"), vec![8]);
        assert!(filter("does not exist").is_empty());
    }

    #[test]
    fn exact_translation_preserves_unknown_labels_and_empty_falls_back() {
        let catalog = BTreeMap::from([
            ("Paused".into(), "En pause".into()),
            ("Empty".into(), " ".into()),
        ]);
        assert_eq!(translate(&catalog, "Paused"), "En pause");
        assert_eq!(
            translate(&catalog, "My Paused Gaming Plan"),
            "My Paused Gaming Plan"
        );
        assert_eq!(translate(&catalog, "RainbowSix.exe"), "RainbowSix.exe");
        assert_eq!(translate(&catalog, "Empty"), "Empty");
    }

    #[test]
    fn indexed_formatting_reorders_without_interpreting_argument_text() {
        assert_eq!(
            interpolate("{1} · {0} · {1}", &["{1}.exe", "42"]),
            "42 · {1}.exe · 42"
        );
        assert_eq!(interpolate("未知 {7} {", &["x"]), "未知 {7} {");
        assert_eq!(placeholders("{1} · {0}"), placeholders("{0} · {1}"));
        assert_ne!(placeholders("{0}"), placeholders("{0} · {1}"));
    }

    #[test]
    fn bundled_catalogs_are_valid_and_keep_format_arguments() {
        let all =
            CATALOG_JSON.map(|json| serde_json::from_str::<Catalog>(json).expect("valid catalog"));
        assert!(
            !all[0].is_empty(),
            "English catalog must not silently fail to load"
        );
        for (index, catalog) in all.iter().enumerate() {
            assert!(!catalog.is_empty(), "Missing catalog: {}", CODES[index + 1]);
            for (message, translated) in catalog {
                assert_eq!(
                    placeholders(message),
                    placeholders(translated),
                    "{}: {message}",
                    CODES[index + 1]
                );
            }
        }
    }

    #[test]
    fn system_display_language_selects_english_supported_languages_and_fallback() {
        for (windows, expected) in [
            ("en-GB", "en"),
            ("ar-SA", "ar"),
            ("es-MX", "es"),
            ("pt-BR", "pt-BR"),
            ("fr-CA", "fr"),
            ("de-DE", "de"),
            ("ru-RU", "ru"),
            ("zh-CN", "zh-CN"),
            ("ja-JP", "en"),
        ] {
            assert_eq!(resolve_with("system", || windows.into()), expected);
        }
    }

    #[test]
    fn explicit_selection_and_refresh_do_not_query_or_override_windows_language() {
        assert_eq!(
            resolve_with("en", || panic!("explicit language must not query Windows")),
            "en"
        );
        assert_eq!(
            refresh_with("de", "de", || panic!(
                "manual override must remain selected"
            )),
            None
        );
        assert_eq!(refresh_with("system", "en", || "de-DE".into()), Some("de"));
        assert_eq!(refresh_with("system", "de", || "de-AT".into()), None);
        assert_eq!(refresh_with("system", "de", || "en-US".into()), Some("en"));
    }

    #[test]
    fn english_loads_no_catalog_and_other_languages_are_cached_individually() {
        let caches: [OnceLock<Catalog>; 7] = [const { OnceLock::new() }; 7];
        assert!(catalog_at(1, &caches).is_none());
        assert!(caches.iter().all(|cache| cache.get().is_none()));
        let german = catalog_at(6, &caches).unwrap();
        assert!(!german.is_empty());
        assert_eq!(
            caches.iter().filter(|cache| cache.get().is_some()).count(),
            1
        );
        assert!(std::ptr::eq(german, catalog_at(6, &caches).unwrap()));
        assert!(catalog_at(2, &caches).is_some());
        assert_eq!(
            caches.iter().filter(|cache| cache.get().is_some()).count(),
            2
        );
    }
}
