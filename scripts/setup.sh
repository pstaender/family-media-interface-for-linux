#!/usr/bin/env bash
# familynovice setup: installs the disc players and builds MakeMKV from source.
#
# MakeMKV provides decryption for copy-protected Blu-rays (via libmmbd) and disc analysis
# (makemkvcon). It is downloaded from makemkv.com at install time and never stored in this
# repository. The legality of circumventing copy protection differs between countries;
# by running this script you decide that for your installation.
#
# Usage:  scripts/setup.sh
# Env:    MAKEMKV_VERSION (default 2.0.0), PREFIX (default /usr/local),
#         BUILD_DIR (default ~/.cache/familynovice/build)

set -euo pipefail

MAKEMKV_VERSION="${MAKEMKV_VERSION:-2.0.0}"
PREFIX="${PREFIX:-/usr/local}"
BUILD_DIR="${BUILD_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/familynovice/build}"
MAKEMKV_URL="https://www.makemkv.com/download"

APT_PACKAGES=(
    # MakeMKV build dependencies (GUI disabled, so no Qt)
    build-essential pkg-config libc6-dev libssl-dev libexpat1-dev libavcodec-dev zlib1g-dev
    # Players: mpv plays the main movie, VLC plays with disc menus
    mpv vlc libbluray2
    # Disc eject fallback (SCSI eject for drives that stay locked)
    eject
    # Blu-ray Java (BD-J) menus
    libbluray-bdj default-jre
)

info() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
die() { printf '\033[1;31merror: %s\033[0m\n' "$*" >&2; exit 1; }

install_packages() {
    info "Installing system packages"
    if command -v apt-get >/dev/null; then
        sudo apt-get update
        sudo apt-get install -y "${APT_PACKAGES[@]}"
    else
        die "Only apt-based distributions (Debian, Ubuntu, Mint) are supported so far.
Please install the equivalents of: ${APT_PACKAGES[*]}"
    fi
}

download() {
    local name="$1"
    local tarball="$name-$MAKEMKV_VERSION.tar.gz"
    if [[ ! -f "$BUILD_DIR/$tarball" ]]; then
        info "Downloading $tarball"
        curl -fL --proto '=https' -o "$BUILD_DIR/$tarball.part" "$MAKEMKV_URL/$tarball"
        mv "$BUILD_DIR/$tarball.part" "$BUILD_DIR/$tarball"
    fi
    rm -rf "${BUILD_DIR:?}/$name-$MAKEMKV_VERSION"
    tar -xzf "$BUILD_DIR/$tarball" -C "$BUILD_DIR"
}

build_makemkv() {
    mkdir -p "$BUILD_DIR"

    download makemkv-oss
    info "Building makemkv-oss $MAKEMKV_VERSION (libmmbd, libmakemkv)"
    (
        cd "$BUILD_DIR/makemkv-oss-$MAKEMKV_VERSION"
        ./configure --prefix="$PREFIX" --disable-gui
        make -j"$(nproc)"
        sudo make install
    )

    download makemkv-bin
    info "Building makemkv-bin $MAKEMKV_VERSION (makemkvcon)"
    echo "MakeMKV will now show its license agreement. Read it and type 'yes' to accept."
    (
        cd "$BUILD_DIR/makemkv-bin-$MAKEMKV_VERSION"
        make
        sudo make install PREFIX="$PREFIX"
    )
    sudo ldconfig
}

check() {
    info "Checking installation"
    command -v makemkvcon >/dev/null || die "makemkvcon not found in PATH"
    ldconfig -p | grep -q 'libmmbd.so.0' || die "libmmbd.so.0 not found by the dynamic linker"
    command -v mpv >/dev/null || die "mpv not found"
    command -v vlc >/dev/null || die "vlc not found"
    echo "makemkvcon: $(command -v makemkvcon)"
    echo "libmmbd:    $(ldconfig -p | awk '/libmmbd.so.0/ {print $NF; exit}')"
    echo "mpv:        $(command -v mpv)"
    echo "vlc:        $(command -v vlc)"
}

key_hint() {
    if ! grep -qs '^app_Key' "$HOME/.MakeMKV/settings.conf"; then
        cat <<EOF

MakeMKV needs a registration key (or the free beta key from the MakeMKV forum).
Add it to ~/.MakeMKV/settings.conf of EVERY user that plays discs,
including the media center account:

    app_Key = "<your key>"

EOF
    fi
}

install_packages
build_makemkv
check
key_hint
info "Done"
