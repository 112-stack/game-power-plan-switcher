# Languages

Game Power Plan Switcher **2.3.7** includes eight offline interface languages. The release remains unsigned; no publisher certificate has been configured.

## Choose your language

Open the **globe button** in the header, or **Settings → Window & keys → Language**. Search by a language's native name, English name, or code. With the search field focused, use the arrow keys and Enter to select a result, or click it. Tab moves between controls; Escape closes the picker. A clear empty state appears when nothing matches.

| Language | Code |
| --- | --- |
| English | `en` |
| العربية — Arabic | `ar` |
| Español — Spanish | `es` |
| Português (Brasil) — Brazilian Portuguese | `pt-BR` |
| Français — French | `fr` |
| Deutsch — German | `de` |
| Русский — Russian | `ru` |
| 简体中文 — Simplified Chinese | `zh-CN` |

The initial **System default** choice follows your Windows display language at launch and refreshes when Windows sends a language-setting change notification. It adds no periodic language polling loop. An explicit choice is saved with your appearance preferences and applies immediately, including the native tray menu. Return to System default to follow Windows again. Unsupported languages fall back to English. Portuguese regional preferences use Brazilian Portuguese; unsupported Traditional Chinese preferences fall back to English rather than silently presenting a different script.

Arabic uses right-aligned interface labels and mixed-direction text. This is not a fully mirrored window layout: graphs, window controls and technical values keep their useful positions and direction. Arabic selects Segoe UI and Simplified Chinese selects Microsoft YaHei UI for script coverage; numeric-only readouts remain monospaced. Fonts come from Windows, not from a download or a redistributed font bundle.

Windows may require signing out before a new display language becomes effective. The app follows the effective Windows UI language; it cannot apply a pending Windows language change itself.

All catalogs are embedded in the app. Selecting a language needs no connection or runtime download. Rust parses the selected catalog only when needed instead of parsing all languages at startup; Slint uses its compiled translation bundle.

## Translation scope

The interface, common status messages and tray commands use the selected language. Game and process names, installed Windows power-plan names, GUIDs, shortcuts and file paths retain their original values. Some hardware/provider descriptions, Windows errors, engine diagnostics, logs and third-party AboutSlint text retain their original language. Structured CSV/JSON session records stay unchanged; human-readable selected-event details in diagnostic exports follow the interface language. Changing language does not change monitoring rules, power plans or process detection.

These are starter translations. Native-speaker corrections are welcome, especially for compact wording, Arabic typography and technical terms. Please include the language, the current wording, your proposed wording and the screen where it appears.

## Contribute a translation

1. Edit the matching UTF-8 JSON file under [`ui/translations`](../ui/translations). `en.json` is the canonical list: each key is an exact English message or format template.
2. Keep every key and all numbered placeholders, including repeated occurrences. You can reorder `{0}` and `{1}` to fit the language. Preserve technical tokens such as `.exe`, GUID, CPU, GPU and keyboard shortcut notation where they identify real input or data.
3. Run the catalog checks from the repository root:

   ```powershell
   python tools/Check-Translations.py --self-test
   ```

4. Build with the pinned toolchain in [CONTRIBUTING.md](../CONTRIBUTING.md), then check Compact and Expanded layouts. Test long labels, search, keyboard selection, disabled controls, modal dialogs and the tray menu. Arabic also needs visual review of mixed Arabic and Latin text; catalog checks cannot prove layout or translation quality.

`build.rs` and `build_support.rs` validate the catalogs and generate Slint's bundled translation files in Cargo's output directory. Slint `@tr(...)` labels and Rust's `src/localization.rs` use the same JSON source. The compiler uses `DefaultTranslationContext::None` because these catalogs are keyed by message, not component name. Preserve that setting: a context mismatch silently leaves Slint labels untranslated even when language selection succeeds. Do not edit generated PO files or add runtime catalog fetches. Missing Rust presentation strings fall back to the original English text; catalog validation still requires complete, matching key sets for the supported languages.

For an isolated preview, use the 2.3.7 executable or a newly built version:

```powershell
$env:GPPS_PREVIEW_LANGUAGE = 'ar'
.\Game-Power-Plan-Switcher-2.3.7.exe --dry-run --start-paused
Remove-Item Env:\GPPS_PREVIEW_LANGUAGE
```

The environment override is honored only in dry-run mode. Preview settings and engine connections remain separate from the normal app. Keep language changes out of IPC identifiers, process matching, power-plan identity and structured operational records.
