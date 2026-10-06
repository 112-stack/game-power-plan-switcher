# Roadmap

Game Power Plan Switcher’s job is simple: make power-plan switching reliable enough that you stop thinking about it. These are directions for future work, not features promised in the current release.

## Next priorities

- **Broader hardware testing.** Validate AMD cache groups, more Intel generations, laptops and systems with multiple processor groups. Publish the actual test matrix.
- **Measured efficiency.** Compare visible, minimized and HUD-enabled CPU/RAM use under repeatable conditions. Fix regressions before making performance claims.
- **A smoother first run.** Make choosing available plans and reviewing discovered games easier, especially when the developer’s custom plan is not installed.
- **Release automation.** Build a Windows CI pipeline for the pinned toolchain, tests, artifact hashes and source packaging. Add signing when a publisher identity is available.
- **Display and input coverage.** Exercise 100%, 125% and 150% DPI, mixed monitors, keyboard navigation and screen-reader behavior.

## Worth exploring

- More documented, read-only sensor providers with clear availability and freshness.
- Better session comparisons based on measured data, without guessing energy savings.
- Safer profile sharing with clear import previews and versioned formats.

The project will stay offline-first and transparent: no activation requirement, game injection, hidden telemetry or fabricated sensor values. Ideas that make the core workflow clearer are more useful than adding another crowded dashboard.
