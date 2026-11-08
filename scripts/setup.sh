#!/usr/bin/env bash
# curator setup: installs the disc players and builds MakeMKV from source.
#
# MakeMKV provides decryption for copy-protected Blu-rays (via libmmbd) and disc analysis
# (makemkvcon). It is downloaded from makemkv.com at install time and never stored in this
# repository. The legality of circumventing copy protection differs between countries;
# by running this script you decide that for your installation.
#
# Usage:  scripts/setup.sh [--no-makemkv]
#         --no-makemkv: only install the system packages and emulators (no Blu-ray support)
# Env:    MAKEMKV_VERSION (default 2.0.0), PREFIX (default /usr/local),
#         BUILD_DIR (default ~/.cache/curator/build)

set -euo pipefail

MAKEMKV_VERSION="${MAKEMKV_VERSION:-2.0.0}"
PREFIX="${PREFIX:-/usr/local}"
BUILD_DIR="${BUILD_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/curator/build}"
MAKEMKV_URL="https://www.makemkv.com/download"
# RetroArch's packages come without CRT filters and gamepad profiles: pinned versions.
SHADERS_COMMIT=f277bd87e454a68c50e01e61e3588118d56b1365
SHADERS_SHA256=94127dc1ad7f4bdc77f6451acedd7d6aa049d42dbaba548933ede9fe66e36ca0
JOYPADS_COMMIT=2e3b8a2728d5efd00fa0207d07ba2488b89e87b2
JOYPADS_SHA256=0143f69b5b16ec14a0e09812c1c06501591440890b0220344e0432f08bc19b3a
RETROARCH_DIR="$PREFIX/share/curator/retroarch"

APT_PACKAGES=(
    # MakeMKV build dependencies (GUI disabled, so no Qt)
    build-essential pkg-config libc6-dev libssl-dev libexpat1-dev libavcodec-dev zlib1g-dev
    # Players: mpv plays the main movie, VLC plays with disc menus
    mpv vlc libbluray2
    # Disc eject fallback (SCSI eject for drives that stay locked)
    eject
    # Blu-ray Java (BD-J) menus
    libbluray-bdj default-jre
    # Website window (curator-web: WebKitGTK via wry/tao)
    libwebkit2gtk-4.1-dev libdbus-1-dev
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

# One pinned libretro repository, only the given folders, into $RETROARCH_DIR/<target>.
install_libretro_files() {
    local repo="$1" commit="$2" sha="$3" target="$4"
    shift 4
    local tarball="$BUILD_DIR/$repo-$commit.tar.gz"
    if [[ ! -f $tarball ]]; then
        curl -fL --proto '=https' --retry 3 -o "$tarball.part" "https://codeload.github.com/libretro/$repo/tar.gz/$commit" || return 1
        mv "$tarball.part" "$tarball"
    fi
    echo "$sha  $tarball" | sha256sum -c --quiet - || { rm -f "$tarball"; return 1; }
    local tmp
    tmp="$(mktemp -d)"
    local patterns=()
    local folder
    for folder in "$@"; do patterns+=("*/$folder/*"); done
    tar xzf "$tarball" -C "$tmp" --strip-components=1 --wildcards "${patterns[@]}" || { rm -rf "$tmp"; return 1; }
    sudo mkdir -p "$RETROARCH_DIR/$target"
    sudo cp -r "$tmp"/. "$RETROARCH_DIR/$target/"
    rm -rf "$tmp"
}

# Not fatal: everything else works without the emulators, and the TV says what is missing.
install_emulators() {
    info "Installing the emulators (DOSBox, RetroArch with Snes9x)"
    # Separate: Snes9x is in an optional package section (multiverse / non-free).
    sudo apt-get install -y dosbox retroarch libretro-snes9x ||
        printf 'warning: not all emulator packages could be installed\n' >&2
    mkdir -p "$BUILD_DIR"
    install_libretro_files glsl-shaders "$SHADERS_COMMIT" "$SHADERS_SHA256" shaders crt blurs ||
        printf 'warning: CRT filters not installed; games run without them\n' >&2
    install_libretro_files retroarch-joypad-autoconfig "$JOYPADS_COMMIT" "$JOYPADS_SHA256" autoconfig udev ||
        printf 'warning: gamepad profiles not installed\n' >&2
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
    # Not `ldconfig -p | grep -q`: grep stops at the first match, ldconfig dies of SIGPIPE
    # while still writing, and with pipefail that counts as "not found".
    grep -q 'libmmbd.so.0' <<<"$(ldconfig -p)" || die "libmmbd.so.0 not found by the dynamic linker"
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

WITH_MAKEMKV=1
for arg in "$@"; do
    case "$arg" in
        --no-makemkv) WITH_MAKEMKV=0 ;;
        *) die "unknown option: $arg (see the comment at the top of $0)" ;;
    esac
done

install_packages
install_emulators
if [[ $WITH_MAKEMKV == 1 ]]; then
    build_makemkv
    check
    key_hint
fi
info "Done"
