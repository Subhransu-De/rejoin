# Contributing to rejoin

## Prerequisites

- Git
- rustup
- The Rust toolchain declared in `rust-toolchain.toml`

Running a Cargo command from the repository installs/selects Rust 1.97.1 through rustup. The project declares Rust 1.97.1 as its minimum supported version.

## Local checks

Run these commands before opening a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --all-features --locked
cargo build --release --locked
cargo doc --no-deps --all-features --locked
```

The command-line integration tests use temporary homes and do not read or update real agent session stores.

## Dependency policy

Install the version used to validate `deny.toml`, then run all advisory, license, ban, and source checks:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo deny check
```

Duplicate dependency versions are warnings by default. Known unavoidable duplicates are documented in `deny.toml`; review new duplicates instead of adding broad exceptions.

## Hawk API checks

Hawk checks that production code only uses public Rust APIs:

```sh
cargo +1.97.1 hawk check --target-dir target/hawk -D warnings
```

Hawk requires its supported compiler components and is authoritative in the Ubuntu CI job.

## Workflow changes

Validate every GitHub Actions workflow after editing it:

```powershell
$workflowPaths = Get-ChildItem .github/workflows -File | Select-Object -ExpandProperty FullName
actionlint $workflowPaths
```

Action references must use full commit SHAs with the release version in a trailing comment. Dependabot maintains these pins.

## Pull requests

- Keep changes focused.
- Add tests for user-visible behavior and failure paths.
- Run the local checks above.
- Update `CHANGELOG.md` when behavior changes.

## Releases

Releases are published from version tags through GitHub Actions. The tag must match the version in `Cargo.toml`.

The release workflow builds, tests, checksums, attests, and publishes:

- A Windows MSI and portable ZIP
- Linux DEB and RPM packages
- A Linux tarball fallback

Do not publish rejoin to crates.io or add a WinGet manifest unless the distribution policy is intentionally changed first.
