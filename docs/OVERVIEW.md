# What is Game Power Plan Switcher?

**Automatically switch Windows power plans when your games start and stop.**

Pick a Gaming plan and a Default plan, add the game processes you care about, and start monitoring. As long as any watched game is running, Gaming stays active. When the last one closes, the app returns to Default.

It began with Rainbow Six Siege and now supports a workspace of games, optional per-game plans, battery and time rules, tray controls and local session exports. Live CPU activity and available hardware sensors show what the computer is doing alongside those transitions.

## The three views

| View | What you use it for |
| --- | --- |
| **Overview** | Current plan, monitoring state, live CPU activity, available sensors and transition history. |
| **Workspace** | Game profiles, process aliases, executable icons and optional per-game settings. |
| **Settings** | Gaming/Default plans, behavior, startup, shortcuts, appearance, integrations and plan tools. |

Compact mode keeps the app at 584 × 526 logical pixels; Expanded mode gives the graph and core grid more room at 800 × 600. Close normally hides the app in the tray. Pause or explicit Exit restores Default.

## Languages

Version 2.3.7 follows the Windows display language automatically, with eight bundled languages and English fallback. The globe button lets you choose and save an override. See the [language guide](LOCALIZATION.md).

## How it works

The Rust engine owns all power changes. It reads the active Windows scheme before writing and checks the result. Windows process notifications drive detection, with a compatibility fallback when trace subscriptions are unavailable. A separate guardian uses the recovery journal if the engine exits unexpectedly.

The Slint GUI stays separate from that engine. A named mutex and local SHOW connection bring the existing window forward on repeated launches. Preview mode has separate storage and connections so you can explore without changing system policy.

CPU activity comes from Windows performance counters, joined to the topology Windows reports. NVIDIA NVML and supported AMD ADL interfaces provide optional GPU readings; CPU/package sensors require an existing LHM/OHM provider. Missing data stays unavailable.

## Upgrading from NN6 Power Plan

Version 2.3.6 changes the public product name. Existing data folders, configuration keys, startup identity and local communication names intentionally retain `NN6` where needed, so settings and recovery continue to work. Prior license grants and the developer’s attribution are preserved. Reinstalling startup through the app is an explicit user action; renaming the download alone does not replace an already installed startup copy.

The PDF guides and screenshots below illustrate version 2.3.6. Their recorded checks remain historical; the language guide and release-specific verification report cover the current revision.

## Where to go next

- [Download 2.3.7](https://github.com/112-stack/game-power-plan-switcher/releases/tag/v2.3.7)
- [Detailed operation notes](../README.txt)
- [Publication and verification scope](PUBLICATION.md)
- [Two-page overview PDF](Game-Power-Plan-Switcher-Overview.pdf)
- [Full user guide](Game-Power-Plan-Switcher-Full-Guide.pdf)
- [Contribute](../CONTRIBUTING.md) or [suggest a next step](../ROADMAP.md)
