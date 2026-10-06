# Changelog

## 2.3.6 — 2026-10-06

### Product identity

- Rename NN6 Power Plan to **Game Power Plan Switcher** so the name explains its purpose: automatically switch Windows power plans when games start and stop.
- Update visible application branding, the download name, repository links and current project documentation.
- Preserve NN6 author credits, GPL licensing, previous release records and the existing application icon.

### Upgrade compatibility

- Keep existing settings, recovery data, IPC and startup identifiers compatible with previous releases. Internal `NN6` names remain where they protect upgrades and single-instance ownership.
- Retain the separate GUI, engine and guardian, process detection, power-plan policy, topology and sensor behavior. This is a naming release, not a new feature or performance claim.

Use the version-specific report and checksums attached to the 2.3.6 release for verification. Earlier results below apply to their recorded versions.


## 2.3.5 — 2026-10-06

### Visual and documentation

- Remove the yellow minimize action from Maintainer and About. Both dialogs retain the shared, accessible red dismiss control; the main header keeps minimize and close.
- Add a concise product overview and a full user and technical guide. Retain the existing compact layout, topology, telemetry and native controls.
- Consolidate project deliverables around the current release and canonical source; remove superseded builds and review copies at the user's request.

### Review fixes

- Isolate dry-run preview engine IPC, telemetry and persistent data from production. Reject a mismatched engine mode before permitting controller commands.
- Preserve Unknown session accounting after IPC disconnect, including repeated CSV/JSON or diagnostic exports. Resume known-plan accounting only after a live observation.
- Keep the current module structure after a full source review. Reuse the shared traffic-light component and add targeted regression coverage instead of an unrelated architectural rewrite.

### Verification

See RELEASE-VERIFICATION.txt for final build, regression, resource and observed UI results. Historical hardware, performance and display checks below are not new measurements for this patch.




## 2.3.4 — 2026-10-06



### Visual & Typography (UI/UX)



- Adopt the obsidian canvas, layered dark surfaces and restrained emerald/cyan signals while preserving Light/System themes, keyboard focus and reduced motion.

- Replace the large developer footer with a 28px avatar and compact inset profile pill, Discord shortcut and aligned copyright/About/version metadata. Preserve the profile snapshot and padded modal badges. The accent dot does not represent live Discord presence.

- Reuse the header's compact yellow/red traffic-light capsule in Developer Profile and About. Red dismisses only the dialog; yellow minimizes the application while preserving the open dialog, with keyboard focus and accessible labels.

- Add a motion-aware CPU warm-up indicator, softer selected-game treatment, inline bulk syntax feedback, and selection-dependent Remove controls.

- Keep a single caret on power selectors; filtering and keyboard navigation remain in the dedicated picker. Add sensor/energy guidance and an explicit profile scheduling reset.



### Core Engine & Features



- Add normalized topology presentation modes and conditional sections. Homogeneous/unknown CPUs have no P/E sections; AMD shared-L3 domains use CACHE GROUP labels, including a valid single-domain layout without blank gaps.

- Derive physical cores and SMT threads independently from Windows masks. Expand deterministic coverage for 7800X3D-like, 5900X-like, homogeneous Intel, hybrid/no-SMT and multi-group cases without claiming tests on unavailable hardware.

- Correlate settings and handover requests with per-client engine receipts. Show pending status immediately, report completion/error only from a matching receipt, and handle timeouts as unknown outcomes.

- Allow plan changes during monitoring through the existing serialized validate/pause/restore/save/resume transaction, retaining rollback and monitor ownership protections.

- Preserve separate GUI/daemon/guardian processes, early singleton/SHOW handover, OpenGL, stable changed-row models, hidden-view suspension, direct NVML/ADL and ten-second WMI fallback. No new topology or sensor dependency is required.



### Bug Fixes & Refinements



- Prevent contradictory pending handover/header state and duplicate settings submissions. An unrelated telemetry broadcast cannot acknowledge a requested change.

- Avoid trusting a repeated utilization value as evidence of freshness: stale/unknown samples remain distinct from measured zero.

- Extend monitored-fullscreen suspension to main-window sensor/model refreshes and the optional HUD. Keep the HUD preference and recreate it when fullscreen focus ends; do not claim GPU resource allocation reaches zero.

- Test rapid start/stop completion with correlated receipts rather than counting incidental status strings; add isolated configure/resume and denied-preview-handover checks.

- Keep performance, accessibility and hardware claims evidence-based. This revision does not advertise ETW activation, zero-copy protobuf, exclusive-fullscreen proof, zero GPU allocation, sub-millisecond delivery, guaranteed RAM use or WCAG AAA certification.



See `IMPLEMENTATION-2.3.4.txt` for specification corrections and `RELEASE-VERIFICATION.txt` for this build's actual checks and limitations. Earlier verification and benchmarks remain historical.



## 2.3.3 — 2026-10-05




### CPU Topology and Live Overview





- Added `src/cpu_topology.rs` to read active Windows physical-core, processor-group, efficiency-class and shared L3 relationships. Supported CPUID identity leaves provide the vendor/model; no topology dependency or per-core affinity mutation is required.


- Group Intel processors into emerald P-core and cyan E-core sections only when Windows reports distinct efficiency classes. Count physical cores separately from SMT threads; homogeneous or unclassified processors keep neutral labels. Lower efficiency classes do not establish an LP E-core identity.


- Group AMD processors by verified Windows L3 sharing domains, displaying each domain's own cache size and observed size asymmetry. A larger L3 is not labeled as proven 3D V-Cache, and a cache domain is not asserted to be a physical CCD/CCX.


- Join live PDH utilization by processor group and logical index, retaining measured zero, warm-up, unavailable values and scrolling grouped chips/heatmaps. Add a CPU model/topology detail dialog and additive section/L3 bindings without replacing existing AppWindow callbacks.


- Use the same authoritative topology mapping for P/E graph averages and chip sections; exclude unknown group/index pairs from class averages. Preserve class mapping when the rolling history resets, and reject future sample timestamps after a clock rollback.


- Anchor section headings and topology-dialog text explicitly to prevent overlap or clipped leading characters. Use the symbol font for the developer-name star glyph.


- Keep requested `has-ccx`/`ccx-groups` compatibility inputs explicitly unavailable (`false`/`-1`); expose measured grouping through `has-l3-groups`/`l3-groups`. Add the read-only `--topology-probe <absolute.json>` diagnostic without GUI/engine startup or power-plan changes.





### Developer Profile and Footer





- Separate the 20px developer display name, 11px username and muted subtitle from metadata with 12px spacing and a thin divider.


- Give metadata badges 4px vertical/10px horizontal padding and 8px gaps, with a supported conditional stacked layout for narrow widths.


- Refine the footer into an 8px-padded gradient pill with bordered avatar, distinct name/developer text, keyboard focus and restrained emerald hover glow. Preserve the public profile snapshot, verified Discord link, Twitch link and AboutSlint.





### Single Instance and Retained Optimizations





- Add a silent duplicate-launch notice through the existing registered tray icon after SHOW handover. Throttle attempts to once per five seconds, respect Windows quiet time, and discard delayed notifications; no extra tray icon is created.


- Preserve the 2.3.2 pre-Slint singleton, separate engine/guardian, OpenGL renderer, Compact defaults, stable changed-row models, hidden-view suspension and lazy HUD teardown/readiness handling.


- Retain pinned NVIDIA NVML 0.13.0, supported AMD ADL8 PMLog, and independent ten-second LHM/OHM WMI fallback. Add no assumed shared-memory provider, driver installation or fabricated readings.





These entries describe implemented changes. Build, topology fixtures, live hardware and UI observations belong in this version's final verification report. Previous 2.3.2 RAM/CPU measurements remain historical and are not relabeled as measurements of 2.3.3; support is not a claim of testing every 2020–2026 CPU model.





## 2.3.2 — 2026-10-05






### Window Lifecycle and Renderer







- Added an early GUI singleton scoped to the current Windows user/session, dry-run mode and isolated test directory. A user-only Global mutex and bounded local overlapped SHOW pipe restore the existing GUI without constructing another renderer or starting another engine. Abandoned mutex ownership permits crash recovery; the original engine/recovery lock remains independent.



- Retained requests received during startup and prevented the initial Start minimized action from hiding a window explicitly requested by a second launch. Windows foreground permission remains subject to OS policy.



- Changed the pinned Slint renderer from WGPU to FemtoVG OpenGL. New/missing preferences default to Compact while preserving an existing saved Expanded/Compact choice; no fixed memory saving or universal GPU-performance claim is made.



- Kept engine snapshots and tray state current while deferring hidden/minimized main-window model, chart and layout refreshes. Showing the window applies the retained current state. Disabled HUDs now release their component and native graphics resources.



- Wait for Slint's asynchronous Winit window-ready signal before positioning a dynamically enabled HUD. Abort pending setup on disable and reject callbacks from an older HUD generation.




- Added an isolated singleton probe/harness for concurrent launches, malformed requests, acknowledged handover, clean release and abandoned-owner recovery. This probe never initializes a GUI, engine or power/startup action.







### Hardware Observations







- Added persistent installed-driver NVIDIA NVML and supported AMD ADL Overdrive8 PMLog GPU readers, nominally every two seconds while a main view or HUD is visible. Report selected adapter/provider and optional temperature, utilization, fan percent and RPM independently.



- Kept CPU temperature/package power and other LHM/OHM WMI fallbacks on a separate asynchronous worker, requested at most every ten seconds except explicit Retry. Direct GPU sampling does not wait for WMI. Stale local caches are invalidated without inventing provider acquisition timestamps.



- Restricted vendor DLL loading to the Windows system directory and supported driver APIs. Added no driver installer, assumed LHM shared-memory interface or sysinfo GPU abstraction. Missing/unsupported values remain unavailable.







These entries describe implemented source changes. Current build, concurrent-process, renderer and hardware observations must be recorded against the final 2.3.2 executable; historical checks below are not relabeled as current results.







## 2.3.1 — 2026-10-04








### Visual & Typography









- Added a restrained active-tab bottom glow, consistent 18px heading weights and tracked uppercase labels while retaining 28px controls and the compact spacing grid.




- Stabilized monospace core/load values and expanded CPU graph tooltips with recorded P/E observations; unavailable classes remain explicitly unavailable.




- Retained the adaptive 584×526 / 800×600 layouts, 64/128-cell heatmaps, real Pro controls and AboutSlint. Installed-only SF Pro font fallbacks follow Segoe UI Variable and Inter.




- Updated Windows file/product metadata to 2.3.1.0 while embedding the current nine-frame Kinetic Squircle icon.









### Release Integrity & Attribution









- Licensed the current NN6 application release under GPL-3.0-or-later, preserving previous MIT grants and all third-party terms. Added clear authorship, optional citation and separate truthful-branding guidance.




- Added explicit offline release-manifest creation and comparison for the final EXE, raw `.text`/`.rdata` SHA-256 digests, PE mitigation flags and public build provenance. Integrity comparison does not enforce activation or block guardian recovery.




- Recorded available commit/dirty-state, build time, target/profile and optional build UUID honestly, including unknown values. Updated release optimization and stripping settings; no complete cross-language LTO or CFG guarantee is claimed.




- Documented Authenticode, detached Sigstore signatures, build provenance and legitimate false-positive submission. No publisher certificate, public signature, attestation or upload is implied by these instructions.




- Retained freely runnable behavior: no hardware lock, debugger/VM detection, false telemetry, periodic tamper scan, punitive shutdown or packer integration.









The accompanying release-security guide accounts for every requested Part 4 item and distinguishes implemented checks from alternatives and omitted mechanisms. Final 2.3.1 build/test/signature evidence is recorded separately; previous 2.3.0 test results are not relabeled as this release's results.









## 2.3.0 — 2026-10-04









### Visual & Typography









- Added persisted Compact and Expanded layouts, responsive chart sizing, a scrollable CPU grid, and an optional dense heatmap for larger processor counts.




- Added light and dark themes with Windows theme synchronization and an optional native window frame.




- Refined the font hierarchy, increased secondary labels to 10 logical pixels, and separated numeric labels into a monospace font to reduce width changes.




- Added CPU chart inspection, P/E-series presentation, interactive transition markers, and detailed event dialogs.









### Core Engine & Features









- Added real Windows tray controls and configurable global shortcuts. Quick profile selection uses the same serial power engine, ownership lock, recovery journal, and battery policy as automatic switching.




- Added AC/DC battery guarding through `GetSystemPowerStatus`. When enabled, battery or unavailable power-source data selects the configured Default plan with an explicit status. Existing configurations remain opt-in; new defaults enable the guard.




- Added optional foreground-only detection policy and local-time windows. Default detection still treats any watched process as running, including games in the background.




- Added per-game power-plan overrides and a persisted master switch for automatic per-game scheduling. New `physical` and `performance-physical` affinity presets select one logical processor per physical core on supported single-group systems.




- Added process and foreground pickers alongside bounded discovery of readable Steam, Epic, Ubisoft, and Xbox installation metadata.




- Added optional readings from an existing LibreHardwareMonitor or OpenHardwareMonitor provider, with explicit unavailable states and provider diagnostics.




- Added explicit `.pow` export/import and independent plan duplication, plus CSV/JSON export of observed session times and recorded transitions.




- Report Windows privilege failures with an explicit **Retry with Windows approval** action for the requested power-plan file operation; ordinary actions and diagnostics do not elevate automatically.




- Extended transition metadata with observed process IDs and the Windows account running the per-user engine, retaining compatibility with older records.




- Added diagnostic JSON export with current failure/status, automation and detector state, session history, and a bounded activity-log tail; absent or truncated logs are explicitly reported.




- Added an opt-in Pro diagnostic command for isolated preferences, policy-gate and export checks; disposable plan-file operations require `--live`.









### Bug Fixes









- Preflight configured global and per-game plans before stopping a valid monitoring session. Reject settings changes while earlier restoration remains pending, and retain rollback paths for save or synchronous restart failures.




- Apply the same restoration and rollback checks to imported profiles. Retry failed battery/time policy application on bounded policy ticks until it succeeds, without repeatedly applying unchanged successful policy.




- Preserve real zero CPU usage separately from unavailable data; clear stale charts on disconnection and keep gaps across missing or hidden intervals.




- Prevent window-location notifications from repeatedly reapplying foreground policy when the foreground process has not changed.




- Preserve legacy scheduling defaults and older configuration/protobuf records when new settings or event fields are absent.




- Escape CSV fields and neutralize spreadsheet formula prefixes in exported names.




- Distinguish an optional plan-file diagnostic blocked by Windows permissions from failed read-only checks. `requires_manual_approval` keeps `file_roundtrip_verified` false; unexpected errors or failed disposable-plan cleanup still fail the diagnostic.









Hardware sensors require an available external provider. Native Snap behavior depends on Windows. Affinity presets do not change firmware SMT settings. CPU samples and observed plan durations do not establish energy savings, zero overhead, instantaneous transitions, or anti-cheat certification. Ordinary-token testing verified disposable duplication and cleanup, but `.pow` export stopped at Windows error `0x522`; import and the approved retry were not exercised. Build, automated-test, and other live-verification results are recorded separately.

<!-- measured-local-release-hashes -->
## Measured local release hashes

These identify locally measured release bytes, not signed publications. Superseded binaries were removed by request.

| Version | SHA-256 |
|---|---|
| 2.1.0 | `f54c44678cbbad10b3568147c7615b7dc40a9dc9c6aff655e39af91910cf812a` |
| 2.2.0 | `d75eaf8d993a5dbafccd5a38a0abea1a627b0feed9e97fa76378740689c5d2f0` |
| 2.3.0 | `4c42e1329148691c3655428f6410e3dd052ad4a3df0ed7ea317569dc2014fdb0` |
| 2.3.1 | `32bcab53a7258daed5d63c6781b6059cc319681ca53c0c90d415a18687c7cfc6` |
| 2.3.2 | `ff762f64d11479d3ab5662dca244155e3b557b20c83f840fe86f6fb37bd577d9` |
| 2.3.3 | `5eb9af66b217121bfa9fd999051f8643cb9f656f238a362a95852fdfc0da5b96` |
| 2.3.4 | `53ca0d8d3a775fae025cf67acadfed9ab3e6a839eaa1168b47f024071a024ebd` |
| 2.3.5 | `a7543fff5ceb3b13d333162ca6efc2932a96a3e95eb957b051f30ec7089114df` |
