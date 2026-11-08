#!/usr/bin/env bash
# curator uninstaller: undoes scripts/install.sh.
#
#   - restores the account's previous login session (e.g. the normal desktop),
#   - stops and removes the admin panel service, turns lingering off again (if it was off),
#   - removes /opt/curator and the "curator" login session.
#
# Settings, usage times, resume positions and caches of the media center account are kept,
# unless --purge is given. System packages (mpv, VLC, cage, …) and MakeMKV stay installed,
# because other programs may use them.
#
# Run it as the media center account (choose the normal desktop in the session menu of the
# login screen) or from another account with sudo rights.
#
# Usage:  scripts/uninstall.sh [--yes] [--purge]
#   --yes    don't ask for confirmation
#   --purge  also delete curator's settings, data and cache of the media center account

set -euo pipefail

PREFIX=/opt/curator
SESSION_FILE=/usr/share/wayland-sessions/curator.desktop
STATE_FILE=$PREFIX/install-state
SERVICE_NAME=curator-admin.service

info() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33mwarning: %s\033[0m\n' "$*" >&2; }
die() { printf '\033[1;31merror: %s\033[0m\n' "$*" >&2; exit 1; }

ASSUME_YES=0
PURGE=0
for arg in "$@"; do
    case "$arg" in
        --yes) ASSUME_YES=1 ;;
        --purge) PURGE=1 ;;
        -h | --help) sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option: $arg (try --help)" ;;
    esac
done

[[ $EUID -ne 0 ]] || die "run this as a normal user (it uses sudo where needed)"
[[ -f $STATE_FILE ]] || die "curator is not installed ($STATE_FILE is missing)"

# Read the state as data (KEY=VALUE lines), never execute it.
state() { sed -n "s/^$1=//p" "$STATE_FILE" | head -n1; }
INSTALL_USER="$(state INSTALL_USER)"
PREV_SESSION="$(state PREV_SESSION)"
PREV_SESSION="${PREV_SESSION:-$(state PREV_XSESSION)}"
INSTALL_HOME="$(state INSTALL_HOME)"
UNIT_DIR="$(state UNIT_DIR)"
[[ -n $INSTALL_USER && -n $INSTALL_HOME && -n $UNIT_DIR ]] || die "$STATE_FILE is incomplete"

cat <<EOF

This removes the media center from the account '$INSTALL_USER':
  * the account gets its previous login session back ('$(state PREV_XSESSION)$(state PREV_SESSION)' or the system default),
  * removes $PREFIX, $SESSION_FILE and the admin panel service.
EOF
if [[ $PURGE -eq 1 ]]; then
    echo "  * DELETES the curator settings, usage times, resume positions and caches of '$INSTALL_USER'."
else
    echo "  * Keeps the curator settings and data (use --purge to delete them)."
fi
if [[ $ASSUME_YES -eq 0 ]]; then
    echo
    read -r -p "Continue? Type 'yes': " answer
    [[ $answer == yes ]] || die "cancelled"
fi
sudo -v

# Commands as the media center account: directly if we are it, otherwise via sudo.
as_user() {
    if [[ $USER == "$INSTALL_USER" ]]; then "$@"; else sudo -u "$INSTALL_USER" -- "$@"; fi
}
user_systemctl() {
    if [[ $USER == "$INSTALL_USER" ]]; then
        systemctl --user "$@"
    else
        sudo systemctl --user --machine="$INSTALL_USER@" "$@"
    fi
}

info "Restoring the login session of '$INSTALL_USER'"
if user_path="$(busctl call org.freedesktop.Accounts /org/freedesktop/Accounts org.freedesktop.Accounts FindUserByName s "$INSTALL_USER" 2>/dev/null)"; then
    user_path="$(sed 's/^o "\(.*\)"$/\1/' <<<"$user_path")"
    accounts_set() {
        sudo busctl call org.freedesktop.Accounts "$user_path" org.freedesktop.Accounts.User "$1" s "$2" ||
            warn "could not restore $1; choose the session at the login screen"
    }
    accounts_set SetSession "$(state PREV_SESSION)"
    accounts_set SetXSession "$(state PREV_XSESSION)"
    accounts_set SetSessionType "$(state PREV_SESSION_TYPE)"
else
    warn "user '$INSTALL_USER' not found in AccountsService; choose the session at the login screen"
fi

info "Removing the admin panel service"
user_systemctl disable --now "$SERVICE_NAME" 2>/dev/null || warn "could not stop $SERVICE_NAME (it is removed anyway)"
as_user rm -f "$UNIT_DIR/$SERVICE_NAME" "$UNIT_DIR/default.target.wants/$SERVICE_NAME"
user_systemctl daemon-reload 2>/dev/null || true
if [[ $(state PREV_LINGER) != yes ]]; then
    sudo loginctl disable-linger "$INSTALL_USER"
fi

info "Removing $PREFIX, the login session and the remote access rule"
sudo rm -f "$SESSION_FILE"
if [[ -f /etc/udev/rules.d/70-curator-remote.rules ]]; then
    sudo rm -f /etc/udev/rules.d/70-curator-remote.rules /etc/udev/rules.d/71-curator-remote-power.rules
    sudo udevadm control --reload-rules || true
fi
sudo rm -rf "$PREFIX" "$PREFIX.new" "$PREFIX.old"

if [[ $PURGE -eq 1 ]]; then
    info "Deleting curator's settings, data and cache of '$INSTALL_USER'"
    # Default XDG locations of that account (only curator's own folders).
    as_user rm -rf "$INSTALL_HOME/.config/curator" "$INSTALL_HOME/.local/share/curator" \
        "$INSTALL_HOME/.cache/curator"
fi

cat <<EOF

$(printf '\033[1;32m')Done.$(printf '\033[0m') curator is removed; '$INSTALL_USER' gets the previous session at the next login.

Still installed (shared with other programs): mpv, VLC, cage, Java and the build packages
(see scripts/setup.sh), Rust (~/.cargo, ~/.rustup), Bun (~/.bun) and MakeMKV in /usr/local
together with ~/.MakeMKV.
EOF
