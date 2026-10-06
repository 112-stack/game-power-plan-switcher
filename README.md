# Game Power Plan Switcher

**Automatically switch Windows power plans when your games start and stop.**

Choose a Gaming plan, choose a Default plan, and add your games. This small native Windows app handles the switch and switches back when you finish. No Steam dependency.

[Download v2.3.6](https://github.com/112-stack/game-power-plan-switcher/releases/tag/v2.3.6) · [Overview](docs/OVERVIEW.md) · [Full guide](docs/Game-Power-Plan-Switcher-Full-Guide.pdf) · [Changelog](CHANGELOG.md)

![Live CPU activity and power-plan controls](docs/screenshots/overview.jpg)

## What it does

- **Automatic switching:** any watched game running selects Gaming; the last one closing restores Default. Optional battery, time and foreground rules let you adjust that behavior.
- **A workspace for your games:** add executable names, browse for a game or review discovered launcher entries. Give individual games their own plan if needed.
- **Useful live information:** CPU activity, topology-aware core groups, available GPU sensors and a power-transition timeline.
- **Everyday controls:** tray actions, a global shortcut, optional startup at user logon, searchable plan pickers and CSV/JSON session export.
- **Native and local:** Rust, C++23 and Slint. One GUI per user/session, with a separate engine and recovery guardian.

![Game profiles and process names](docs/screenshots/workspace.jpg)

## Get started

1. Download `Game-Power-Plan-Switcher-2.3.6.exe` from the release. Windows 10/11 x64 and a compatible OpenGL driver are required.
2. In **Settings**, select your installed Gaming and Default plans. The app does not supply a custom gaming plan. Use `powercfg /list` to see what is installed.
3. In **Workspace**, add the game’s actual `.exe` name. Rainbow Six Siege aliases are included initially; replace them or add other games.
4. Select **Start monitoring**. Close goes to the tray by default. **Pause** or **Exit** restores Default; enable **Startup** to run at your next logon.

To look around without changing power plans or startup settings:

```powershell
.\Game-Power-Plan-Switcher-2.3.6.exe --dry-run --start-paused
```

Preview uses separate settings and engine connections. An older monitor requires an explicit handover before normal monitoring begins.

## Same app, clearer name

Version 2.3.6 renames **NN6 Power Plan** to **Game Power Plan Switcher**. Existing settings, startup registration and recovery connections retain their internal names for compatibility. Copyright credits and prior license grants remain intact. This fresh repository starts from a new `main` root and distributes version 2.3.6 only; historical engineering notes are retained as records, not older release downloads.

## A few honest limits

Power plans are preferences, not an FPS guarantee. CPU/package temperatures need an existing LibreHardwareMonitor or OpenHardwareMonitor provider; unsupported readings stay unavailable. Energy savings are not estimated. The app does not inject into games or read their memory, but it is not anti-cheat certified.

The Windows executable is **unsigned**. Compare its SHA-256 with the release checksums and keep Windows security protections enabled. Read the version-specific verification report attached to the release for the checks actually run and their limits.

## Build and contribute

Follow [CONTRIBUTING.md](CONTRIBUTING.md) for the pinned Windows build and [ROADMAP.md](ROADMAP.md) for useful next steps. Small fixes, clear bug reports and hardware test results are welcome.

The source is **GPL-3.0-or-later**; the combined Slint distribution uses GPLv3. See [LICENSE.txt](LICENSE.txt), [NOTICE.txt](NOTICE.txt) and [publication notes](docs/PUBLICATION.md). Built by [nn6](https://discord.com/users/176078095957098497). No affiliation with game publishers or hardware vendors.
