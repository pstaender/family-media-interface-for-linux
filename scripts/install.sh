#!/usr/bin/env bash
# familynovice installer (example for Ubuntu / Linux Mint and other apt + systemd systems).
#
# Turns a user account (default: the current one) into a media center account:
#   - installs the system packages, MakeMKV (scripts/setup.sh) and the kiosk compositor "cage",
#   - builds familynovice and installs it to /opt/familynovice (owned by root, read-only),
#   - adds a login session "familynovice" (cage runs only the media center: no desktop, panels,
#     app menu, Alt+Tab or notifications) and makes it this account's default session,
#   - runs the admin panel as a systemd user service at http://127.0.0.1:8787/, also while
#     nobody is logged in (lingering).
#
# Recommended: a separate account without admin rights for the media center, e.g.
#   sudo adduser tv && scripts/install.sh --user tv     (run from your own admin account)
#
# After the next login this account shows only the media center. Quitting it (Ctrl+Q) ends
# the session and returns to the login screen. Other accounts are not changed; the regular
# desktop stays selectable at the login screen (session menu), e.g. to run scripts/uninstall.sh.
#
# Usage:  scripts/install.sh [--user NAME] [--yes] [--no-makemkv] [--skip-packages]
#   --user NAME      the media center account (default: the current user); building and
#                    sudo happen as the current user
#   --yes            don't ask for confirmation
#   --no-makemkv     no Blu-ray support (don't build MakeMKV)
#   --skip-packages  don't run scripts/setup.sh (packages and MakeMKV are already installed)
#
# Undo with scripts/uninstall.sh.

set -euo pipefail

PREFIX=/opt/familynovice
SESSION_NAME=familynovice
SESSION_FILE=/usr/share/wayland-sessions/$SESSION_NAME.desktop
STATE_FILE=$PREFIX/install-state
SERVICE_NAME=familynovice-admin.service
KIOSK_PACKAGES=(cage xwayland)

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAGING="${XDG_CACHE_HOME:-$HOME/.cache}/familynovice/build/install"

info() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33mwarning: %s\033[0m\n' "$*" >&2; }
die() { printf '\033[1;31merror: %s\033[0m\n' "$*" >&2; exit 1; }

ASSUME_YES=0
WITH_MAKEMKV=1
SETUP_ARGS=()
SKIP_PACKAGES=0
TARGET_USER="$USER"
while [[ $# -gt 0 ]]; do
    arg="$1"
    shift
    case "$arg" in
        --user) [[ $# -gt 0 ]] || die "--user needs a name"; TARGET_USER="$1"; shift ;;
        --yes) ASSUME_YES=1 ;;
        --no-makemkv) WITH_MAKEMKV=0; SETUP_ARGS+=(--no-makemkv) ;;
        --skip-packages) SKIP_PACKAGES=1 ;;
        -h | --help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option: $arg (try --help)" ;;
    esac
done

# ---------------------------------------------------------------------------- checks

[[ $EUID -ne 0 ]] || die "run this as a normal user, not as root (it uses sudo where needed)"
command -v apt-get >/dev/null || die "this example installer needs an apt-based system (Ubuntu, Mint, Debian)"
command -v systemctl >/dev/null || die "systemd is required"
command -v busctl >/dev/null || die "busctl (systemd) is required"

TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6 || true)"
[[ -n $TARGET_HOME && -d $TARGET_HOME ]] || die "user '$TARGET_USER' doesn't exist (create it with: sudo adduser $TARGET_USER)"
TARGET_UID="$(id -u "$TARGET_USER")"
[[ $TARGET_UID -ge 1000 ]] || die "'$TARGET_USER' is a system account"
if [[ $TARGET_USER == "$USER" ]]; then
    UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
else
    UNIT_DIR="$TARGET_HOME/.config/systemd/user"
fi

# Commands as the media center account: directly if we are it, otherwise via sudo.
as_target() {
    if [[ $TARGET_USER == "$USER" ]]; then "$@"; else sudo -u "$TARGET_USER" -- "$@"; fi
}
target_systemctl() {
    if [[ $TARGET_USER == "$USER" ]]; then
        systemctl --user "$@"
    else
        sudo systemctl --user --machine="$TARGET_USER@" "$@"
    fi
}

if [[ -f $STATE_FILE ]] && ! grep -qx "INSTALL_USER=$TARGET_USER" "$STATE_FILE"; then
    die "familynovice is already installed for another user ($(grep '^INSTALL_USER=' "$STATE_FILE" | cut -d= -f2)). Run scripts/uninstall.sh first."
fi

DISPLAY_MANAGER="$(basename "$(readlink -f /etc/systemd/system/display-manager.service 2>/dev/null || true)" .service)"

cat <<EOF

This turns the account '$TARGET_USER' into a media center account:

  * After the next login, '$TARGET_USER' sees only the media center (no desktop, no other apps).
  * Admin panel (for parents): http://127.0.0.1:8787/ (from any account on this computer).
  * Installs to $PREFIX, $SESSION_FILE and $UNIT_DIR/$SERVICE_NAME.
  * No personal files are changed. Undo with scripts/uninstall.sh.
EOF
if id -nG "$TARGET_USER" | tr ' ' '\n' | grep -qxE 'sudo|admin|wheel'; then
    cat <<EOF

  ! '$TARGET_USER' is an administrator (sudo). For children, use a separate account without
    admin rights ('sudo adduser tv', then 'scripts/install.sh --user tv'), and give it a
    strong password, even with automatic login.
EOF
fi
case "$DISPLAY_MANAGER" in
    gdm | gdm3 | lightdm) ;;
    *) warn "display manager '${DISPLAY_MANAGER:-unknown}': choose the session 'familynovice' once at the login screen; it should be remembered." ;;
esac
if [[ $ASSUME_YES -eq 0 ]]; then
    echo
    read -r -p "Continue? Type 'yes': " answer
    [[ $answer == yes ]] || die "cancelled"
fi

sudo -v # ask for the password once, up front

# ---------------------------------------------------------------------------- packages

if [[ $SKIP_PACKAGES -eq 0 ]]; then
    "$REPO/scripts/setup.sh" "${SETUP_ARGS[@]}"
fi
info "Installing the kiosk compositor (${KIOSK_PACKAGES[*]})"
sudo apt-get install -y "${KIOSK_PACKAGES[@]}"

if ! command -v cargo >/dev/null && [[ -x $HOME/.cargo/bin/cargo ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v cargo >/dev/null; then
    info "Installing Rust (rustup, for this user)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v bun >/dev/null && [[ -x $HOME/.bun/bin/bun ]]; then
    export PATH="$HOME/.bun/bin:$PATH"
fi
if ! command -v bun >/dev/null; then
    info "Installing Bun (for this user)"
    curl -fsSL https://bun.sh/install | bash
    export PATH="$HOME/.bun/bin:$PATH"
fi

# ---------------------------------------------------------------------------- build

info "Building the media center and the website window"
cargo build --release --workspace --manifest-path "$REPO/Cargo.toml"

info "Preparing the admin panel"
rm -rf "$STAGING"
mkdir -p "$STAGING/bin" "$STAGING/admin"
cp "$REPO/target/release/familynovice" "$REPO/target/release/familynovice-web" "$STAGING/bin/"
cp "$(command -v bun)" "$STAGING/bin/bun" # own copy: the service doesn't depend on ~/.bun
cp -r "$REPO/admin/src" "$REPO/admin/package.json" "$REPO/admin/bun.lock" "$REPO/admin/tsconfig.json" "$STAGING/admin/"
(cd "$STAGING/admin" && "$STAGING/bin/bun" install --production --frozen-lockfile)

# Session: cage shows one fullscreen app at a time (the newest on top: players and the
# website window cover the media center and it comes back when they close).
# -s keeps Ctrl+Alt+F1…F6 working (those consoles ask for a password).
cat >"$STAGING/bin/familynovice-session" <<'EOF'
#!/bin/sh
# Started by the display manager for the "familynovice" session (see scripts/install.sh).
# Keyboard layout of the system (cage doesn't read the desktop's settings).
if [ -r /etc/default/keyboard ]; then
    . /etc/default/keyboard
    export XKB_DEFAULT_LAYOUT="${XKB_DEFAULT_LAYOUT:-$XKBLAYOUT}"
    export XKB_DEFAULT_VARIANT="${XKB_DEFAULT_VARIANT:-$XKBVARIANT}"
    export XKB_DEFAULT_MODEL="${XKB_DEFAULT_MODEL:-$XKBMODEL}"
    export XKB_DEFAULT_OPTIONS="${XKB_DEFAULT_OPTIONS:-$XKBOPTIONS}"
fi
# Logs: journalctl -t familynovice
if command -v systemd-cat >/dev/null; then
    exec systemd-cat -t familynovice -- cage -s -- /opt/familynovice/bin/familynovice-kiosk
fi
exec cage -s -- /opt/familynovice/bin/familynovice-kiosk
EOF

# Inside cage: restart the media center if it crashes. Quitting normally (exit code 0) ends
# the session; five crashes in a row (each within a minute) end it too, back to the login screen.
cat >"$STAGING/bin/familynovice-kiosk" <<'EOF'
#!/bin/sh
crashes=0
while :; do
    started=$(date +%s)
    /opt/familynovice/bin/familynovice && exit 0
    code=$?
    [ $(($(date +%s) - started)) -gt 60 ] && crashes=0
    crashes=$((crashes + 1))
    echo "familynovice exited with code $code (crash $crashes)" >&2
    [ "$crashes" -ge 5 ] && exit "$code"
    sleep $((crashes * 2))
done
EOF
chmod 755 "$STAGING/bin/"*

# ---------------------------------------------------------------------------- install

info "Installing to $PREFIX"
# Keep the session settings from before the FIRST installation, so uninstall restores them.
USER_PATH="$(busctl call org.freedesktop.Accounts /org/freedesktop/Accounts org.freedesktop.Accounts FindUserByName s "$TARGET_USER" | sed 's/^o "\(.*\)"$/\1/')"
get_prop() {
    busctl get-property org.freedesktop.Accounts "$USER_PATH" org.freedesktop.Accounts.User "$1" | sed 's/^s "\(.*\)"$/\1/'
}
if [[ -f $STATE_FILE ]]; then
    cp "$STATE_FILE" "$STAGING/install-state"
else
    {
        echo "INSTALL_USER=$TARGET_USER"
        echo "INSTALL_HOME=$TARGET_HOME"
        echo "UNIT_DIR=$UNIT_DIR"
        echo "PREV_SESSION=$(get_prop Session)"
        echo "PREV_XSESSION=$(get_prop XSession)"
        echo "PREV_SESSION_TYPE=$(get_prop SessionType)"
        echo "PREV_LINGER=$(loginctl show-user "$TARGET_USER" -p Linger --value 2>/dev/null || echo no)"
    } >"$STAGING/install-state"
fi

sudo rm -rf "$PREFIX.new"
sudo cp -r "$STAGING" "$PREFIX.new"
sudo chown -R root:root "$PREFIX.new"
sudo chmod -R u=rwX,go=rX "$PREFIX.new"
sudo rm -rf "$PREFIX.old"
[[ -d $PREFIX ]] && sudo mv "$PREFIX" "$PREFIX.old"
sudo mv "$PREFIX.new" "$PREFIX"
sudo rm -rf "$PREFIX.old"

info "Adding the login session '$SESSION_NAME'"
sudo tee "$SESSION_FILE" >/dev/null <<EOF
[Desktop Entry]
Name=familynovice
Comment=Family media center (only the media center, no desktop)
Exec=$PREFIX/bin/familynovice-session
Type=Application
DesktopNames=familynovice
EOF

info "Making '$SESSION_NAME' the default session of '$TARGET_USER'"
# AccountsService is read by GDM (Session) and LightDM (XSession).
accounts_set() {
    sudo busctl call org.freedesktop.Accounts "$USER_PATH" org.freedesktop.Accounts.User "$1" s "$2"
}
accounts_set SetSession "$SESSION_NAME"
accounts_set SetXSession "$SESSION_NAME"
accounts_set SetSessionType wayland || true

info "Starting the admin panel as a user service of '$TARGET_USER'"
# Keep the account's services (admin panel) running without a login, so parents can set
# things up; this also starts its service manager now.
sudo loginctl enable-linger "$TARGET_USER"
for _ in $(seq 20); do
    systemctl is-active --quiet "user@$TARGET_UID.service" && break
    sleep 0.5
done
as_target mkdir -p "$UNIT_DIR"
as_target tee "$UNIT_DIR/$SERVICE_NAME" >/dev/null <<EOF
[Unit]
Description=familynovice admin panel (http://127.0.0.1:8787/)

[Service]
WorkingDirectory=$PREFIX/admin
Environment=NODE_ENV=production
# No transpiler cache outside our folders.
Environment=BUN_RUNTIME_TRANSPILER_CACHE_PATH=0
ExecStart=$PREFIX/bin/bun src/server.ts
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF
target_systemctl daemon-reload
target_systemctl enable "$SERVICE_NAME"
target_systemctl restart "$SERVICE_NAME" ||
    warn "the admin panel didn't start: systemctl --user status $SERVICE_NAME (as '$TARGET_USER')"

# ---------------------------------------------------------------------------- MakeMKV key

# MakeMKV reads its key from the home of the account that plays the disc.
if [[ $WITH_MAKEMKV -eq 1 && -t 0 ]] && ! as_target grep -qs '^app_Key' "$TARGET_HOME/.MakeMKV/settings.conf"; then
    echo
    read -r -p "MakeMKV key for Blu-rays of '$TARGET_USER' (paste it, or press Enter to skip): " key
    if [[ -n $key ]]; then
        as_target mkdir -p "$TARGET_HOME/.MakeMKV"
        printf 'app_Key = "%s"\n' "$key" | as_target tee -a "$TARGET_HOME/.MakeMKV/settings.conf" >/dev/null
    fi
fi

sleep 1
cat <<EOF

$(printf '\033[1;32m')Done.$(printf '\033[0m')

  * Log in as '$TARGET_USER' (log out first if logged in): the media center starts instead
    of the desktop.
    (Automatic login: in the system settings under Users / Login Window.)
  * Admin panel: http://127.0.0.1:8787/  ($(target_systemctl is-active "$SERVICE_NAME" 2>/dev/null || true))
    Set an admin password and a PIN there first.
  * Leave the media center: Ctrl+Q (ends the session, back to the login screen).
  * Back to the normal desktop for this account: pick it in the session menu of the login
    screen, or run scripts/uninstall.sh.
  * Logs: journalctl -t familynovice (media center), journalctl --user -u $SERVICE_NAME (admin; both as '$TARGET_USER')
EOF
