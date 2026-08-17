#!/usr/bin/env bash
set -euo pipefail

readonly MACOS_ARM_TARGET="aarch64-apple-darwin"
readonly MACOS_X64_TARGET="x86_64-apple-darwin"
readonly WINDOWS_TARGET="x86_64-pc-windows-msvc"
readonly WINDOWS_ARM_TARGET="aarch64-pc-windows-msvc"
readonly LINUX_X64_TARGET="x86_64-unknown-linux-gnu"
readonly LINUX_ARM_TARGET="aarch64-unknown-linux-gnu"
readonly LINUX_X64_IMAGE="openhp1-linux-x86_64:glibc-2.31"
readonly LINUX_ARM_IMAGE="openhp1-linux-aarch64:glibc-2.31"

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

bundle() {
    local directory="$1"
    local archive="$2"
    (
        cd "$staging_dir"
        zip -qry "${archive}.zip" "$directory"
        zip -T "${archive}.zip" >/dev/null
    )
}

build_linux_image() {
    local image="$1"
    local base_image="$2"
    local deb_arch="$3"
    "$container_engine" build \
        --platform linux/amd64 \
        --build-arg "CROSS_BASE_IMAGE=$base_image" \
        --build-arg "CROSS_DEB_ARCH=$deb_arch" \
        --tag "$image" \
        --file docker/linux/Dockerfile \
        docker/linux
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly repo_root
cd "$repo_root"
readonly linux_x64_target_dir="$repo_root/target/cross/$LINUX_X64_TARGET"
readonly linux_arm_target_dir="$repo_root/target/cross/$LINUX_ARM_TARGET"

[[ "$(uname -s)" == "Darwin" ]] || die "macOS is required to build the universal macOS bundle"
for command in cargo cargo-xwin cross git install lipo rustup zip; do
    require_command "$command"
done
if command -v docker >/dev/null 2>&1; then
    container_engine="docker"
elif command -v podman >/dev/null 2>&1; then
    container_engine="podman"
else
    die "cross requires Docker or Podman"
fi
readonly container_engine

if [[ -n "$(git status --porcelain)" ]]; then
    printf 'warning: building from a dirty working tree\n' >&2
fi

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

rustup target add --toolchain stable \
    "$MACOS_ARM_TARGET" \
    "$MACOS_X64_TARGET" \
    "$WINDOWS_TARGET" \
    "$WINDOWS_ARM_TARGET"
stable_toolchain_root="$(dirname "$(dirname "$(rustup which --toolchain stable cargo)")")"
stable_toolchain_bin="$stable_toolchain_root/bin"
stable_toolchain="$(basename "$stable_toolchain_root")"
readonly stable_toolchain_root stable_toolchain_bin stable_toolchain
export PATH="$stable_toolchain_bin:$PATH"
export RUSTUP_TOOLCHAIN="$stable_toolchain"

build_linux_image \
    "$LINUX_X64_IMAGE" \
    "ghcr.io/cross-rs/x86_64-unknown-linux-gnu:latest" \
    amd64
build_linux_image \
    "$LINUX_ARM_IMAGE" \
    "ghcr.io/cross-rs/aarch64-unknown-linux-gnu:latest" \
    arm64

cargo build --locked --release -p openhp1-game -p openhp1-launcher --target "$MACOS_ARM_TARGET"
cargo build --locked --release -p openhp1-game -p openhp1-launcher --target "$MACOS_X64_TARGET"
cargo xwin build --locked --release -p openhp1-game -p openhp1-launcher --target "$WINDOWS_TARGET"
cargo xwin build --locked --release -p openhp1-game -p openhp1-launcher --target "$WINDOWS_ARM_TARGET"
CARGO_TARGET_DIR="$linux_x64_target_dir" \
    cross build --locked --release -p openhp1-game -p openhp1-launcher --target "$LINUX_X64_TARGET"
CARGO_TARGET_DIR="$linux_arm_target_dir" \
    cross build --locked --release -p openhp1-game -p openhp1-launcher --target "$LINUX_ARM_TARGET"

mkdir -p \
    "$staging_dir/openhp1_win" \
    "$staging_dir/openhp1_win_arm" \
    "$staging_dir/openhp1_macos" \
    "$staging_dir/openhp1_linux_x64" \
    "$staging_dir/openhp1_linux_arm"

for binary in openhp1-game openhp1-launcher; do
    lipo -create \
        "target/$MACOS_ARM_TARGET/release/$binary" \
        "target/$MACOS_X64_TARGET/release/$binary" \
        -output "$staging_dir/openhp1_macos/$binary"
    chmod 0755 "$staging_dir/openhp1_macos/$binary"
    lipo "$staging_dir/openhp1_macos/$binary" -verify_arch arm64
    lipo "$staging_dir/openhp1_macos/$binary" -verify_arch x86_64

    install -m 0755 \
        "target/$WINDOWS_TARGET/release/$binary.exe" \
        "$staging_dir/openhp1_win/$binary.exe"
    install -m 0755 \
        "target/$WINDOWS_ARM_TARGET/release/$binary.exe" \
        "$staging_dir/openhp1_win_arm/$binary.exe"
    install -m 0755 \
        "$linux_x64_target_dir/$LINUX_X64_TARGET/release/$binary" \
        "$staging_dir/openhp1_linux_x64/$binary"
    install -m 0755 \
        "$linux_arm_target_dir/$LINUX_ARM_TARGET/release/$binary" \
        "$staging_dir/openhp1_linux_arm/$binary"
done

bundle openhp1_win openhp1_windows_x64
bundle openhp1_win_arm openhp1_windows_arm
bundle openhp1_macos openhp1_macos_universal
bundle openhp1_linux_x64 openhp1_linux_x64
bundle openhp1_linux_arm openhp1_linux_arm

mv "$staging_dir" "$release_dir"
staging_dir=""
trap - EXIT
printf 'Created %s\n' "$release_dir"
