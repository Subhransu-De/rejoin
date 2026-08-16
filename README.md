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

Download the latest `rejoin-v<version>-windows-x64.msi`, open it, and follow the installer. The installer adds `rejoin` to the machine `PATH`.

For a direct portable download, use `rejoin-v<version>-windows-x64.exe`. The `rejoin-v<version>-windows-x64.zip` package also includes the executable, license, README, and changelog.

### Linux

Download the package for your distribution and install it:

```sh
version="0.2.0" # Replace with the release you downloaded.

# Debian or Ubuntu
sudo apt install "./rejoin_${version}_amd64.deb"

# Fedora or another RPM-based distribution
sudo dnf install "./rejoin-${version}-1.x86_64.rpm"
```

For other x86-64 Linux distributions, download the `.tar.gz` archive and install the binary:

```sh
version="0.2.0" # Replace with the release you downloaded.
tar -xzf "rejoin-v${version}-linux-x64.tar.gz"
sudo install -m 0755 rejoin /usr/local/bin/rejoin
```

Confirm the installation:

```sh
rejoin --version
```

Each release includes a GPG-signed `SHA256SUMS` manifest and the public release key. Verify the manifest and downloaded files:

```sh
gpg --show-keys --with-fingerprint rejoin-release-key.asc
gpg --import rejoin-release-key.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum --ignore-missing --check SHA256SUMS
```

The expected primary GPG fingerprint is `039E ED8E 5BFC C203 92DB DFD9 7D0D 1D64 441E CACF`. Check the downloaded public key against this fingerprint before trusting it.

The downloadable executables and installers are not Authenticode-signed. If you use the GitHub CLI, you can also verify build provenance:

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
| `n`            | Start a new session in the panel    |
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
