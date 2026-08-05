# Changelog

All notable changes to rejoin are documented here.

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

[0.1.0]: https://github.com/Subhransu-De/rejoin/releases/tag/v0.1.0
