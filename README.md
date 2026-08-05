# rejoin

[![CI](https://github.com/Subhransu-De/rejoin/actions/workflows/ci.yml/badge.svg)](https://github.com/Subhransu-De/rejoin/actions/workflows/ci.yml)
[![CodeQL](https://github.com/Subhransu-De/rejoin/actions/workflows/codeql.yml/badge.svg)](https://github.com/Subhransu-De/rejoin/actions/workflows/codeql.yml)
[![Dependency review](https://github.com/Subhransu-De/rejoin/actions/workflows/dependency-review.yml/badge.svg)](https://github.com/Subhransu-De/rejoin/actions/workflows/dependency-review.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A fast terminal session manager for Claude Code, Codex, Cursor, Pi, and OpenCode.

It finds sessions for the current folder, shows every agent in one dashboard, and resumes a selected session.

## Demo

[![Watch the Rejoin demo](https://raw.githubusercontent.com/Subhransu-De/rejoin/main/assets/rejoin-demo.png)](https://github.com/Subhransu-De/rejoin/blob/main/assets/rejoin-demo.mp4)

## Install

Install the latest version with Rust:

```sh
cargo install --git https://github.com/Subhransu-De/rejoin --locked
```

This installs the `rejoin` executable in Cargo's global bin directory. Make sure `~/.cargo/bin` is on `PATH`.

Update an existing installation:

```sh
cargo install --git https://github.com/Subhransu-De/rejoin --locked --force
```

## Use

Run inside a project to show only that folder's sessions:

```sh
rejoin
```

Use `rejoin --all` to search every discovered session, `rejoin list` for a plain-text list, or `rejoin paths` to inspect the detected session stores.

| Key | Action |
| --- | --- |
| `Ctrl` + arrow | Move between agent panels |
| `Up` / `Down` | Select a session |
| `Enter` | Resume the selected session |
| `x` | Launch another agent with a handoff |
| `h` | Preview the handoff |
| `/` | Search |
| `f` | Filter |
| `q` | Quit |

## Roadmap

- [ ] Agent-neutral handoff workflow

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for features, fixes, and performance improvements in each release.

## Build

```sh
cargo build --release --locked
cargo test --locked
```

Licensed under the [MIT License](LICENSE).
