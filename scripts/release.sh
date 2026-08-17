#!/usr/bin/env bash
set -euo pipefail

readonly MACOS_ARM_TARGET="aarch64-apple-darwin"
readonly MACOS_X64_TARGET="x86_64-apple-darwin"
readonly WINDOWS_TARGET="x86_64-pc-windows-gnu"
readonly LINUX_X64_TARGET="x86_64-unknown-linux-gnu"
readonly LINUX_ARM_TARGET="aarch64-unknown-linux-gnu"

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

bundle() {
    local name="$1"
    (
        cd "$staging_dir"
        zip -qry "${name}.zip" "$name"
        zip -T "${name}.zip" >/dev/null
    )
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly repo_root
cd "$repo_root"

[[ "$(uname -s)" == "Darwin" ]] || die "macOS is required to build the universal macOS bundle"
for command in cargo cross git install lipo rustup zip; do
    require_command "$command"
done
if ! command -v docker >/dev/null 2>&1 && ! command -v podman >/dev/null 2>&1; then
    die "cross requires Docker or Podman"
fi

git diff --quiet --ignore-submodules -- || die "tracked files have uncommitted changes"
git diff --cached --quiet --ignore-submodules -- || die "the index has uncommitted changes"

tag="$(git tag --points-at HEAD --sort=-version:refname | sed -n '1p')"
if [[ -n "$tag" ]]; then
    version="$(printf '%s' "$tag" | LC_ALL=C sed 's/[^A-Za-z0-9-]/_/g')"
else
    version="$(date -u +%Y%m%d)_$(git rev-parse --short=7 HEAD)"
fi

readonly release_root="$repo_root/release"
readonly release_dir="$release_root/$version"
[[ ! -e "$release_dir" ]] || die "$release_dir already exists"
mkdir -p "$release_root"
staging_dir="$(mktemp -d "$release_root/.${version}.XXXXXX")"
case "$staging_dir" in
    "$release_root"/.*) ;;
    *) die "unexpected staging directory: $staging_dir" ;;
esac
cleanup() {
    if [[ -n "${staging_dir:-}" && -d "$staging_dir" ]]; then
        rm -rf "$staging_dir"
    fi
}
trap cleanup EXIT

rustup target add --toolchain stable "$MACOS_ARM_TARGET" "$MACOS_X64_TARGET"
stable_toolchain_root="$(dirname "$(dirname "$(rustup which --toolchain stable cargo)")")"
stable_toolchain_bin="$stable_toolchain_root/bin"
stable_toolchain="$(basename "$stable_toolchain_root")"
readonly stable_toolchain_root stable_toolchain_bin stable_toolchain
export PATH="$stable_toolchain_bin:$PATH"
export RUSTUP_TOOLCHAIN="$stable_toolchain"

cargo build --locked --release -p openhp1-game -p openhp1-launcher --target "$MACOS_ARM_TARGET"
cargo build --locked --release -p openhp1-game -p openhp1-launcher --target "$MACOS_X64_TARGET"
cross build --locked --release -p openhp1-game -p openhp1-launcher --target "$WINDOWS_TARGET"
cross build --locked --release -p openhp1-game -p openhp1-launcher --target "$LINUX_X64_TARGET"
cross build --locked --release -p openhp1-game -p openhp1-launcher --target "$LINUX_ARM_TARGET"

mkdir -p \
    "$staging_dir/openhp1_win" \
    "$staging_dir/openhp1_macos" \
    "$staging_dir/openhp1_linux_x64" \
    "$staging_dir/openhp1_linux_arm"

for binary in openhp1-game openhp1-launcher; do
    lipo -create \
        "target/$MACOS_ARM_TARGET/release/$binary" \
        "target/$MACOS_X64_TARGET/release/$binary" \
        -output "$staging_dir/openhp1_macos/$binary"
    chmod 0755 "$staging_dir/openhp1_macos/$binary"
    lipo -verify_arch arm64 x86_64 "$staging_dir/openhp1_macos/$binary"

    install -m 0755 \
        "target/$WINDOWS_TARGET/release/$binary.exe" \
        "$staging_dir/openhp1_win/$binary.exe"
    install -m 0755 \
        "target/$LINUX_X64_TARGET/release/$binary" \
        "$staging_dir/openhp1_linux_x64/$binary"
    install -m 0755 \
        "target/$LINUX_ARM_TARGET/release/$binary" \
        "$staging_dir/openhp1_linux_arm/$binary"
done

for name in openhp1_win openhp1_macos openhp1_linux_x64 openhp1_linux_arm; do
    bundle "$name"
done

mv "$staging_dir" "$release_dir"
staging_dir=""
trap - EXIT
printf 'Created %s\n' "$release_dir"
