# Changelog

All notable changes to rejoin are documented here.

## Unreleased

- Fixed dialog sizing, scrolling, compact layouts, warning visibility, stable search, and keyboard navigation.
- Moved scanning and handoff generation off the UI thread and avoided idle list rebuilding.
- Hardened session parsing, title selection, partial scan failures, path handling, activity detection, and cache persistence.
- Preserved recent handoff instructions, masked common credential patterns, and stored handoffs outside working directories.
- Fixed agent wrapper launches, terminal ownership, interrupt handling, custom Pi session paths, and child exit codes.

## [0.2.0] - 2026-08-16

### Features

- Added a `[n - New]` action to start a fresh session in the focused agent panel.

### Bug fixes

- Excluded Codex and OpenCode sub-agent sessions from the session list.

### Reliability

- Added failure-safe terminal restoration that attempts raw-mode, alternate-screen, and cursor cleanup on errors and unwinding.
- Converted scanner thread panics into per-scanner warnings so other session sources can still load.
- Added black-box CLI tests for path overrides, stable JSON output, corrupt OpenCode data, and non-interactive terminal handling.

### Development and security

- Pinned the supported Rust toolchain, added targeted unsafe-code linting, and documented every Windows FFI safety invariant.
- Added Cargo advisory, license, duplicate, and source policy checks with scheduled CI coverage.
- Pinned GitHub Actions to immutable commits, checksum-verified the Hawk installer, added job timeouts, and added a latest-stable compatibility check.
- Added contributor guidance, a security policy, and code ownership for automation and distribution files.

### Distribution

- Connected smoke-tested MSI, DEB, and RPM packages to tagged GitHub Releases alongside portable ZIP and tar archives.
- Added a standalone portable Windows executable alongside the MSI and ZIP packages.
- Consolidated release checksums into a GPG-signed manifest while retaining GitHub build-provenance attestations.
- Removed macOS binary packaging while retaining macOS source compatibility testing.
- Disabled crates.io publication; GitHub Releases remain the only binary distribution channel.

## [0.1.0] - 2026-08-05

### Features

- Added one terminal dashboard for Claude Code, Codex, Cursor, Pi, and OpenCode sessions.
- Added exact current-folder scoping, with `--all` available for system-wide discovery.
- Added per-agent panels, keyboard navigation, activity and status indicators, search, filtering, and JSON output.
- Added session resume commands for every supported agent.
- Added reviewable agent-neutral handoff previews with copy, save, and cross-agent launch support. End-to-end handoff reliability remains on the roadmap.
- Added session-store discovery for Windows, macOS, and Linux.
- Added immediate animated feedback while an agent starts without delaying process creation.

### Bug fixes

- Restored the terminal correctly around agent launch and exited rejoin after the launched agent ended, preventing stale TUI output from being left in the shell.
- Made the search query and every typed character visible in a dedicated input bar.
- Preserved exact folder matching across canonical and Windows path variants.
- Removed redundant project and detail UI, simplified the footer path, and aligned activity values consistently.
- Improved active-session detection from process IDs and working directories.

### Performance

- Scanned agent stores and running processes concurrently.
- Cached parsed session metadata and limited JSONL reads to bounded head and tail sections.
- Resolved Cursor transcripts lazily instead of walking every transcript at startup.
- Reused normalized paths and process snapshots during folder and status matching.

### Maintenance

- Added Linux, macOS, and Windows CI with formatting, Clippy, tests, and release builds.
- Added CodeQL, dependency review, Dependabot, and Hawk dead-public-API checks.
- Added automated tagged releases with native archives and SHA-256 checksums.
- Added a sanitized product demo and a public handoff roadmap.

[0.2.0]: https://github.com/Subhransu-De/rejoin/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Subhransu-De/rejoin/releases/tag/v0.1.0
