#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ "$(uname -s)" != "Darwin" ]]; then
	echo "The macOS package must be built on macOS." >&2
	exit 1
fi

version="$(
	cargo metadata --no-deps --format-version 1 |
		jq -r '.packages[] | select(.name == "rejoin") | .version'
)"
if [[ -z "$version" || "$version" == "null" ]]; then
	echo "Could not determine the rejoin version." >&2
	exit 1
fi

output_dir="${1:-dist}"
mkdir -p "$output_dir"
output_dir="$(cd "$output_dir" && pwd)"

rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --locked --target aarch64-apple-darwin
cargo build --release --locked --target x86_64-apple-darwin

staging_root="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/rejoin-pkg.XXXXXX")"
payload_root="$staging_root/root"
install_bin="$payload_root/usr/local/bin"
install_docs="$payload_root/usr/local/share/doc/rejoin"
mkdir -p "$install_bin" "$install_docs"

lipo -create \
	target/aarch64-apple-darwin/release/rejoin \
	target/x86_64-apple-darwin/release/rejoin \
	-output "$install_bin/rejoin"
chmod 0755 "$install_bin/rejoin"
cp LICENSE README.md CHANGELOG.md "$install_docs/"

lipo -info "$install_bin/rejoin"
file "$install_bin/rejoin"

package="$output_dir/rejoin-$version-macos-universal-unsigned.pkg"
pkgbuild \
	--root "$payload_root" \
	--identifier dev.rejoin.cli \
	--version "$version" \
	--install-location / \
	"$package"

shasum -a 256 "$package" >"$package.sha256"
printf '%s\n' "$package"
