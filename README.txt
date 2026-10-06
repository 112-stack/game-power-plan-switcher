Current release: Game Power Plan Switcher 2.3.6 (2026-10-06). This release renames NN6 Power Plan while preserving its monitoring behavior. Use the version-specific report attached to the 2.3.6 release for current verification. Older version-specific notes, hashes and benchmarks remain historical.

GAME POWER PLAN SWITCHER 2.3.6
Windows x64 | Rust 2024 + C++23 + Slint 1.18.1

Current release overview: README.md. Release integrity, signing instructions and
Part 4 requirement disposition: RELEASE-SECURITY.md. GPL-3.0-or-later is the final
license choice; no activation/HWID check restricts running the application.

This repository contains the current native source. Older review builds were
removed from the development workspace. Dated verification reports describe
their recorded binaries; historical measurements are not new release claims.
An installed legacy monitor is retained until the user explicitly hands over.

New or missing preferences default to Compact at 584 x 526 logical pixels
(also the minimum size). An existing saved Compact/Expanded choice is preserved;
Expanded is 800 x 600. The adaptive interface uses 8px margins,
10px card padding, 6px gaps and 28px controls. Light/dark themes, Windows theme
sync and the optional native window frame are available. Overview combines a
real 30-second P/E CPU trend, all-core chips/heatmap, observed session accounting
and clickable transition details. Settings now connects real plan tools, battery/
time/process policies, tray, global shortcuts and an optional desktop telemetry HUD.
Read PRO-2.3-GUIDE.txt for usage, UI-Refresh-Guide.txt for design/source details,
BRANDING.txt for icon exports, and PRO-Features-and-Bindings.txt for the contract.
The workspace-only NN6-2.3-Requirements.csv maps all 50 document paragraphs and corrects claims
such as zero overhead, guaranteed instant transitions and a universal minimum
WCAG font size. Final revision verification is recorded separately.
The earlier live resize inspection exposed a presentation input-grab defect. Header
dragging and the resize grip now use Slint's winit_030 window adapter to queue
native move/resize and reconcile release; minimize/maximize retain the bridge.

RUN
1. Open the new release Game-Power-Plan-Switcher-2.3.6.exe. No PS2EXE or extra PowerShell modules are needed.
   The GUI/monitor are compiled native code; launcher discovery and legacy-task
   handover use the Windows PowerShell 5.1 executable included with Windows.
2. Existing Siege aliases and saved WPF game names are migrated when compatible.
   Original settings.json is not overwritten. Review Settings and Workspace.
3. If the previous monitor is enabled, click “Use this app instead” on Overview
   when ready to transfer. This disables only GamePowerPlan-RainbowSix-<your SID>.
   Until that action, the native app stays paused and the existing monitor runs.
4. Start monitoring. By default any watched process selects Gaming; the final
   watched process exiting selects Default. Optional foreground, battery/time
   rules and per-game plan overrides modify that policy. Steam is not required.
5. Minimize to keep monitoring. Close hides to the tray by default. Use the tray
   or in-app Exit action to exit and restore Default, the previous overlay and
   scheduling. Pause also restores them. If Close to system tray is turned off,
   Close exits normally. Start minimized is a separate saved preference.
6. The “Startup” control copies the executable into the stable per-user directory
   below and registers HKCU\Software\Microsoft\Windows\CurrentVersion\Run,
   value NN6PowerPlanNative. The same button removes registration. It starts at
   user logon after reboot/shutdown; it does not run before Windows logon.
   The native version intentionally has no watchdog that reopens a closed GUI.

Opening the app again restores its existing GUI for the same Windows user and
session. This happens before another Slint renderer, window or engine starts.
A user-only Global mutex includes the Windows session in its identity; an
overlapped local named pipe queues SHOW, acknowledges it and exits the duplicate.
Early requests survive initial UI setup and take precedence over Start minimized
without changing that preference. Handover waits at most a bounded startup
interval; failure reports an error and does not create a second GUI. An abandoned
mutex permits recovery after a GUI crash. Older nonparticipating releases cannot
be retroactively deduplicated. Separate Windows sessions keep separate GUIs.
An existing registered tray icon may show the duplicate-launch notice. It is
silent, rate-limited to once per five seconds, and respects Windows notification
suppression. No extra icon is created for the notice and SHOW does not depend on
the notice being displayed.

The shipped application requires Windows 10/11 x64 with a compatible OpenGL
graphics driver for Slint's FemtoVG renderer, introduced in 2.3.2 and retained in
2.3.6. Earlier RAM/CPU measurements apply to their recorded 2.3.2 binary, not this
revision. This is not a promise of a particular memory/performance gain.
The tested development PC runs Windows 11. ARM64, other machines, mixed DPI and
multiple monitors have not been certified. This build is not Authenticode signed.

POWER PLANS
Gaming: LowLatency-Intel v2
319f9863-5b07-4924-b71a-3a687a155782
Default: Balanced
381b4222-f694-41f0-9685-ff5bb260df2e

Read installed GUIDs in Windows PowerShell:
    powercfg /list
    powercfg /getactivescheme

The application uses PowerGetActiveScheme / PowerSetActiveScheme directly,
checks before each write, and verifies the resulting active plan. Monitoring
does not change a plan's internal definition. Explicit Power tools can import
a .pow file, export or duplicate a selected plan, or open Windows advanced settings.
If a configured plan disappears, choose an installed replacement in Settings.

WORKSPACE AND SETTINGS
Search is literal and case insensitive. Clear X resets it. Three game profiles
are shown per page, with aliases inside each profile. Bulk input accepts comma,
semicolon or newline separated names and strips file paths and outer quotes.
Names must match letters/digits/underscore/hyphen followed by .exe. Duplicates
are ignored regardless of case. Up to 500 process names are supported.
Browse imports an EXE icon without launching the executable. Running and
Foreground app offer searchable read-only process candidates. Discover reads
accessible Steam, Epic, Ubisoft and Xbox metadata; review candidates because a
launcher may include helper binaries. Auto-discover is opt-in and populates
candidates for review. Custom/protected libraries can be missed.

To verify a game's real executable: launch it, open Task Manager > Details,
then locate its .exe name. Or use this read-only command:
    Get-Process -Name '*Rainbow*' | Select-Object ProcessName,Id,Path

Plan selectors support search, pagination, Up/Down, Enter and Escape. A chosen
plan is saved only on selection. Editing pauses and resumes monitoring safely.
Ctrl+Enter starts/pauses from the app when no dialog is open. Profile settings
offer a per-game plan override, optional normal/above-normal/high priority and
performance/efficiency/physical/performance-physical/hex CPU affinity.
These options are OFF by default; protected games can reject changes. They
affect the whole process, not selected threads, and do not address NPU hardware.
Physical policies choose one logical thread per physical core; they do not
disable SMT in firmware. Multi-processor-group class affinity is rejected.

The optional best-performance overlay uses dynamically discovered Windows
exports, saves the previous overlay, reports refusal, and restores it. Those
exports are not a guaranteed public SDK contract. The option is disabled by
default; it is independent of the named power plan. This Windows power policy is
distinct from the optional topmost desktop telemetry HUD. The HUD displays
available CPU/sensor values without game injection. No NPU prediction is present.

PRO CAPABILITIES
The CPU trend and session timing use actual engine observations. Session timing
starts at the first active-plan snapshot in this GUI window; unknown/third-party
plans and disconnections are excluded. It uses monotonic elapsed time. External
plan changes while the detector is paused are not independently observed, so
these are observed-session totals rather than audited lifetime statistics.
The history model keeps up to 64 events; the dialog scrolls all recorded events
and opens details/export. New events capture source/verified target plans,
available process/PID, engine Windows account and API duration. Older rows
lacking these fields
say “Source not captured” instead of inventing a previous plan or process.

Settings → Integrations reports actual hardware-provider status and offers
Connect sensors / Retry. A persistent worker queries installed NVIDIA NVML or
supported AMD ADL Overdrive8 PMLog GPU metrics nominally every two seconds while
the main window or HUD is visible. One selected adapter is identified; missing
metrics are not borrowed from a different GPU. Temperature, utilization, fan
percent and RPM are separate optional values, not estimates of one another.
CPU temperature/package power and fallback sensors use an existing
LibreHardwareMonitor/OpenHardwareMonitor WMI provider, at most every ten seconds
unless Retry is requested. WMI work cannot block the direct GPU sampling loop.
The app does not install a sensor driver, download a service, assume undocumented
LHM shared memory, or use sysinfo for GPU sensing. Missing readings stay
unavailable. WMI poll age is local receipt age, not proof of hardware freshness.
Energy savings requires an actual meter/baseline and remains unavailable.

Battery guard selects Default on DC or unknown power source. Optional local-time
rules gate automatic game eligibility; Apply saves HH:MM edits. Foreground-only
is off by default. Process-tuning master controls saved affinity/priority policies.
Per-game plan selection follows stable saved-profile order if multiple games run.
Fresh native defaults enable the battery guard; migrated older configurations
retain their previous opt-in behavior. Review the saved rule before monitoring.

The default Ctrl+Alt+P global shortcut toggles Gaming/Default manually. Apply
reports actual registration/conflict results; a blank chord disables it. Manual
selection lasts until pause/resume or the next game lifecycle; battery guard
still takes precedence. The optional HUD shortcut is separate. Tray actions and
Close-to-tray are real native operations. Exit performs normal restoration.

CPU TOPOLOGY AND LIVE ACTIVITY
Windows GetLogicalProcessorInformationEx provides active physical cores,
efficiency classes and shared L3 cache masks. CPUID supplies vendor/model identity.
The detector runs once during GUI setup; it does not change affinity, power policy
or the sampler. PDH values join by (processor group, logical index), never by an
assumed global ordinal. Info opens the model, counts and detection details.

Intel CPUs with distinct efficiency classes have P-CORES and E-CORES sections.
Physical counts include SMT siblings once. A uniform or zero class by itself
does not prove that a core is an E-core; those CPUs retain neutral C labels.
Lower classes on a hybrid are grouped as E, without claiming LP-E identification.
AMD uses Windows shared L3 domains, with each section's own cache size. Larger
domains in an asymmetric layout are labeled larger L3, not certified 3D V-Cache.
These masks do not establish physical CCD/CCX identity on every Zen generation.
Missing relationships remain unclassified rather than guessed. This supports
reported Windows topology without claiming testing on every 2020-2026 model.

No extra topology dependency is required: kernel32 and standard Rust CPUID
intrinsics suffice. nvml-wrapper remains pinned to 0.13.0 for GPU sensors.
To save a read-only topology report, from the EXE directory in PowerShell:
    $topologyReport = Join-Path $PWD 'Game-Power-Plan-Switcher-topology.json'
    & .\Game-Power-Plan-Switcher-2.3.6.exe --topology-probe $topologyReport
The report path must be absolute. This command does not start a GUI/engine or
modify power plans, affinity, startup, sensor providers or installed drivers.

CPU telemetry uses Windows PDH's \Processor Information(*)\% Processor Time
counter, nominally once per second while Overview is visible, foreground and
not minimized, occluded or suspended by fullscreen detection. An enabled HUD
also requests live telemetry while it is visible. Sampling
is independent of the power-monitoring toggle: pausing power-plan automation
does not turn the topology into a static status diagram. The grid uses 18px
chips with 4px gaps in scrollable topology sections. Optional micro-heatmap uses
8 columns in Compact or 16 in Expanded; headers and larger groups scroll. Hover for processor/group/load
details. These values describe activity during the sampled interval, not
instantaneous ownership of a core by a game or identification of its threads.
Resuming resets the query: the first observation warms up and the following
observation supplies a fresh interval. Unavailable or warming-up samples show
an em dash rather than invented zero; a measured 0% remains a valid result.
The footer reports the actual interval, mean of valid samples, processors at
or above 1% activity and any partial data. Group/index matching handles repeated
logical indices across processor groups. Homogeneous and parked cores are gray.
Disconnects, unreadable telemetry and fullscreen suspension invalidate old
percentages immediately while preserving known topology; resumed data requires
fresh observations. Paused power monitoring can therefore produce GUI frames
while live CPU readings change; it is not a zero-rendering-work state.
When the main window is hidden or minimized, incoming engine state is retained
without rebuilding its visible models, charts or layout on each snapshot. A
show/restore applies the current state once; engine/tray/recovery still operate.
An enabled visible HUD can continue independent CPU/sensor sampling. Disabling
the HUD drops its component/native adapter, instead of merely hiding a second
graphics context. These rules do not promise zero CPU or memory use.
Process monitoring continues on all tabs and when minimized. The timeline
contains up to 64 measured plan/overlay API events; it does not measure rendering
performance or universal process-detection latency.

The padded nn6 footer pill has a gradient surface, bordered avatar and emerald
hover glow. It opens the developer profile with separate name/username/subtitle,
a divider and padded metadata badges. Adjacent badges use 8px gaps and a narrow
layout fallback stacks them with 6px gaps. The verified Discord link is:
https://discord.com/users/176078095957098497
The native protocol is discord://-/users/176078095957098497 with HTTPS fallback.
There is no numeric Discord ID setting. Avatar and details are a public profile
snapshot from October 3, 2026, not an authenticated live Discord integration.

DATA AND RECOVERY
The public product name changed in 2.3.6. Internal NN6 data paths, IPC names,
startup identity, environment variables and recovery locks stay unchanged for
upgrade compatibility. Developer attribution and historical notices remain.
Renaming a download does not replace an already installed startup copy; use the
app's Startup control explicitly when updating that installation.

%LOCALAPPDATA%\NN6PowerPlan\native-settings.json  settings
%LOCALAPPDATA%\NN6PowerPlan\appearance.json       theme/tray/hotkey/HUD preferences
%LOCALAPPDATA%\NN6PowerPlan\native.log            errors and activity
%LOCALAPPDATA%\NN6PowerPlan\recovery.json         pending restoration journal
%LOCALAPPDATA%\NN6PowerPlan\NativeApp\            startup executable
%LOCALAPPDATA%\GamePowerPlan\switch.log           timestamped plan switches

The version-specific report attached to the 2.3.6 release records its checks and
limits; source changes alone are not verification evidence. Older reports apply
only to the binaries they identify. The versioned release is
Game-Power-Plan-Switcher-2.3.6.exe. Historical 2.0/2.1/2.2/2.3/2.3.1/2.3.2
reports and requirement audits remain in the original workspace; they are not
included in the public ZIP or relabeled as current verification. See README.md,
RELEASE-SECURITY.md and DEPENDENCY-SOURCES.json for public distribution details.

Timestamps are Unix epoch milliseconds. native.log rotates at approximately
2 MB. Recovery data is flushed before mutation. A guardian process waits for
the engine to exit; it restores pending state under the shared monitor lock.
Failed restores remain journaled and are retried before a later monitoring
session. Simultaneous termination of engine and guardian or a power failure
cannot be guaranteed recoverable immediately. A removed power plan cannot be
restored until an available default is configured. No user plan is deleted.

One per-user engine serializes mutations. The GUI singleton is an earlier,
separate gate; ordinary repeated launches restore that GUI. The existing engine
observer/Take control path remains for compatibility and independent engine
ownership, including an older GUI or monitor that predates the singleton.
Named pipes reject remote clients and restrict access to the current user SID.
Telemetry uses an eight-slot memory-mapped ring; protobuf encoding still copies
data. There is no zero-copy or static-allocation claim.

On this PC, fast WMI process trace subscriptions are denied to the ordinary
user token. The app explicitly reports compatibility mode: WMI intrinsic events
check process starts every one second; process-handle waits detect known exits.
No elevated task, ACL edit, kernel driver or game injection is installed. The
detector badge describes the actual WMI/Win32 mode, not app-owned ETW. Anti-cheat
compatibility is not certified; the badge does not guarantee a ban outcome.

SAFE PREVIEW
From Windows PowerShell, in the executable's directory:
    $env:NN6_TEST_DATA_DIR=Join-Path $env:TEMP ('NN6-preview-'+[guid]::NewGuid())
    Start-Process .\Game-Power-Plan-Switcher-2.3.6.exe -ArgumentList '--dry-run','--start-paused' -WindowStyle Hidden
    Remove-Item Env:\NN6_TEST_DATA_DIR

Preview mode can detect games but cannot change plans, install startup or take
over the legacy monitor. Without NN6_TEST_DATA_DIR, --dry-run uses the separate
%LOCALAPPDATA%\NN6PowerPlanPreview data directory and preview pipe/ring identity.
Explicit test directories are normalized and hashed into separate IPC identities;
dry and live modes differ. Reusing the same data directory retains a common engine
lock, so use a fresh root for concurrent fixtures. Production data aliases are
rejected. The initial snapshot mode is checked before HELLO or queued commands;
subsequent pipe and shared-memory snapshots are also mode-validated.
GUI singleton scope includes the normalized test directory and dry-run mode.
Exit a prior preview from its tray menu when finished; Close may hide its window.

BUILD FROM SOURCE
Use Rust 1.99.0 edition 2024 and LLVM-MinGW UCRT release 20260922 (x86_64 host).
Official downloads/documentation:
https://www.rust-lang.org/tools/install
https://github.com/mstorsjo/llvm-mingw/releases/tag/20260922
https://docs.slint.dev/latest/docs/slint/

1. Install rustup using its official installer. Extract LLVM-MinGW somewhere
   stable, for example C:\Tools\llvm-mingw-20260922-ucrt-x86_64.
2. In PowerShell:
    rustup toolchain install 1.99.0-x86_64-pc-windows-gnullvm --profile minimal
3. From this source directory:
    .\Build.ps1 -LlvmMingw C:\Tools\llvm-mingw-20260922-ucrt-x86_64 -Action test
    .\Build.ps1 -LlvmMingw C:\Tools\llvm-mingw-20260922-ucrt-x86_64 -Action build
4. Output:
    target\x86_64-pc-windows-gnullvm\release\Game-Power-Plan-Switcher.exe

Cargo.lock is committed with this source archive; --locked prevents accidental
dependency upgrades. build.rs embeds NN6.ico, version metadata, per-monitor DPI
manifest, the Slint UI, developer/avatar assets and dependency notices. Rust/C++
runtime linkage is static; optional sensor integrations dynamically use the
installed vendor driver DLLs and remain unavailable when those are absent.
PS2EXE is not used for this native source.
The verified workspace build script is work\Build-Native.ps1; it uses locally
staged toolchains without altering the user's permanent PATH.
The presentation adapter enables unstable-winit-030 with Slint pinned to exactly
1.18.1. It is version-specific, not an API-stability guarantee; recheck dragging,
resizing and subsequent clicks if Slint/winit are upgraded. Source file paths
remain ui/app.slint and src/ui.rs. See UI-Refresh-Guide.txt for the repair details.

SOURCE MAP
src/engine.rs        serialized state machine, recovery, event handling
src/win.rs           checked C ABI wrappers and Windows startup/ownership
native/bridge.cpp    C++23 COM/WMI, foreground/size hooks, power APIs, IPC
src/model.rs         settings, strict parser, migration, protobuf schema
src/discovery.rs     read-only launcher discovery and executable import
src/ui.rs            UI controller, live CPU view state and finite reveal motion
src/ui_pro.rs        theme, tray/hotkeys, exports, sensor/HUD controller
src/gui_instance.rs  early user/session GUI mutex and bounded SHOW handover
src/cpu_topology.rs  read-only Windows core/cache domains and CPUID identity
src/sensors.rs       persistent NVML/ADL GPU and asynchronous WMI sensor worker
src/preferences.rs   persisted appearance and shell preferences
src/automation.rs    foreground/battery/time/per-game policy
src/system_integration.rs native tray, shortcuts, theme and sensor boundary
src/power_tools.rs   explicit import/export/duplicate power-plan tools
src/session_export.rs CSV/JSON export of observed session data
src/pro_view.rs      measured CPU trend, observed session accounting, log rows
ui/app.slint         responsive GPU-rendered views, selectors and dialogs
src/power.rs         power provider boundary and mutation tests
src/exercise.rs      isolated IPC/process/power integration harness
src/frame_probe.rs   opt-in render-notifier benchmark, disposable fullscreen UI

TEST HARNESS
Unit tests do not change active power plans. The --exercise mode requires a
fresh NN6_TEST_DATA_DIR and refuses real Siege processes. Without --live it
uses disposable names and previews actions. --live changes real plans and
requires exclusive monitor ownership; use the supplied workspace wrapper only
with real games closed. It restores the old task/plan/Run value in finally.
The portable copy in this archive is Test-Live.ps1. Run it from an ordinary
interactive Windows PowerShell session, with real games closed:
    .\Test-Live.ps1 -Executable C:\Path\Game-Power-Plan-Switcher-2.3.6.exe
This is an explicit live test: it temporarily stops the exact previous NN6 task,
switches the actual plans, tests startup/overlay/scheduling, then restores state.
Render instrumentation is activated only by NN6_FRAME_REPORT and writes local
callback timings. It is not a GPU allocation or DWM profiler.

The included harness tools/Test-GuiSingleton.ps1 -Exe <absolute release path>
uses a fresh test directory and --gui-instance-probe. It checks concurrent
launches, acknowledged handover, malformed requests, clean shutdown and abandoned
ownership after terminating only its own disposable probe. It never initializes
Slint, starts an engine or mutates power/startup. Its report must be generated
against the current binary before claiming that these checks passed.

LICENSES
Application code: GPL-3.0-or-later, see LICENSE.txt and NOTICE.txt. Earlier MIT
grants remain valid; LICENSE-MIT-HISTORY.txt preserves the older notice verbatim.
Citation is optional and modification/rebranding are permitted under the GPL.
About > View dependency licenses opens
the notices embedded in the executable. This release selects Slint 1.18.1's
GPL-3.0-only alternative; the combined distribution is conveyed under GPLv3,
while NN6's own source grant is GPL-3.0-or-later. AboutSlint is retained. Third-party packages
retain their own licenses. Game artwork and the user-supplied profile avatar
are not relicensed as original NN6 code.

ROLL BACK
Remove the native startup entry with the startup toggle, choose Exit from the
tray (Close may only hide the window), then
re-enable the previous task if it was handed over:
    $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    Enable-ScheduledTask -TaskName ('GamePowerPlan-RainbowSix-'+$sid)
    Start-ScheduledTask -TaskName ('GamePowerPlan-RainbowSix-'+$sid)
Do not run both monitors without their shared ownership lock.
