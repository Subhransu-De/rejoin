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

rejoin is distributed through [GitHub Releases](https://github.com/Subhransu-De/rejoin/releases) only.

### Windows

Download the latest `rejoin-<version>-windows-x64.msi`, open it, and follow the installer. The installer adds `rejoin` to the machine `PATH`.

For a portable installation, download the `rejoin-v<version>-x86_64-pc-windows-msvc.zip`, extract it, and place `rejoin.exe` in a directory on `PATH`.

### Linux

Download the package for your distribution and install it:

```sh
version="0.1.0" # Replace with the release you downloaded.

# Debian or Ubuntu
sudo apt install "./rejoin_${version}_amd64.deb"

# Fedora or another RPM-based distribution
sudo dnf install "./rejoin-${version}-1.x86_64.rpm"
```

For other x86-64 Linux distributions, download the `.tar.gz` archive and install the binary:

```sh
version="0.1.0" # Replace with the release you downloaded.
tar -xzf "rejoin-v${version}-x86_64-unknown-linux-gnu.tar.gz"
sudo install -m 0755 rejoin /usr/local/bin/rejoin
```

Confirm the installation:

```sh
rejoin --version
```

Each release includes SHA-256 checksum files. If you use the GitHub CLI, you can also verify build provenance:

```sh
gh attestation verify "path/to/downloaded-file" --repo Subhransu-De/rejoin
```

### Build from source

To build the latest version directly from the GitHub repository:

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

| Key            | Action                              |
| -------------- | ----------------------------------- |
| `Ctrl` + arrow | Move between agent panels           |
| `Up` / `Down`  | Select a session                    |
| `Enter`        | Resume the selected session         |
| `x`            | Launch another agent with a handoff |
| `h`            | Preview the handoff                 |
| `/`            | Search                              |
| `f`            | Filter                              |
| `q`            | Quit                                |

## Roadmap

- [ ] Agent-neutral handoff workflow

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for features, fixes, and performance improvements in each release.

## Build

```sh
cargo build --release --locked
cargo test --all-targets --all-features --locked
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the complete development checks.

Licensed under the [MIT License](LICENSE).
