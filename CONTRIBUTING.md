# Contributing

Thanks for helping Game Power Plan Switcher become a better everyday utility. Start with a small, focused change or a clear issue describing what happened and what you expected.

## Build on Windows

Install [rustup](https://www.rust-lang.org/tools/install) and extract the **UCRT x64** [LLVM-MinGW 20260922 toolchain](https://github.com/mstorsjo/llvm-mingw/releases/tag/20260922). From the repository root:

```powershell
rustup toolchain install 1.99.0-x86_64-pc-windows-gnullvm --profile minimal
.\Build.ps1 -LlvmMingw C:\Tools\llvm-mingw-20260922-ucrt-x86_64 -Action test
.\Build.ps1 -LlvmMingw C:\Tools\llvm-mingw-20260922-ucrt-x86_64 -Action build
```

The executable is `target\x86_64-pc-windows-gnullvm\release\Game-Power-Plan-Switcher.exe`. The build script uses `Cargo.lock`; keep Slint and the window adapter pinned unless the change specifically requires an upgrade.

For UI work, launch with `--dry-run --start-paused`. This preview has separate data and cannot change plans or startup ownership. Use a fresh `NN6_TEST_DATA_DIR` for disposable concurrent fixtures. Ordinary unit tests do not switch plans; **`Test-Live.ps1` does**, so read its instructions before using it.

## Keep the important boundaries

- Keep power writes in the engine, and preserve the shared ownership lock and recovery journal.
- Keep GUI, engine and guardian separate. Preserve the singleton and preview isolation.
- Keep legacy NN6 data/IPC/startup identities compatible when changing public branding; migrations need explicit review.
- Report missing or stale sensors as unavailable. Never fill gaps with invented zeroes.
- Use valid Slint 1.18.1 properties and keep existing public controller bindings compatible.
- Keep the interface compact, keyboard accessible and clear about what each control does.

In your pull request, explain the problem, the change and the checks you actually ran. Include before/after screenshots for visible changes. Hardware test reports should identify the CPU/GPU and Windows version, without personal account names or raw private logs.

Read [SECURITY.md](SECURITY.md) before reporting a vulnerability. Contributions must be compatible with the project’s GPL terms and retain applicable third-party notices; only submit code and artwork you have permission to share.
