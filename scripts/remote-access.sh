#!/usr/bin/env bash
# Give the user logged in at the screen read access to the remote control (only that device),
# so familynovice sees every button, also those X11 can't deliver to any app (e.g. OK in
# non-pointer mode = KEY_SELECT). Normal keyboards stay off-limits.
#
# It writes a udev rule that tags the remote's input devices with "uaccess": systemd-logind
# then grants the user of the active session read/write access to them (like for game
# controllers). familynovice only reads; it never grabs the device.
#
# Usage:  scripts/remote-access.sh                 choose the remote from a list
#         scripts/remote-access.sh 4842:0001       by USB id (vendor:product), several allowed
#         scripts/remote-access.sh --remove        remove the rule again
#
# Then calibrate it: admin panel → Home screen & media → Remote.

set -euo pipefail

RULE=/etc/udev/rules.d/70-familynovice-remote.rules

info() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
die() { printf '\033[1;31merror: %s\033[0m\n' "$*" >&2; exit 1; }

reload() {
    sudo udevadm control --reload-rules
    sudo udevadm trigger --action=change --subsystem-match=input
    sudo udevadm settle || true
}

if [[ ${1:-} == --remove ]]; then
    if [[ -f $RULE ]]; then
        sudo rm -f "$RULE"
        reload
        echo "Removed $RULE."
    else
        echo "Nothing to remove ($RULE doesn't exist)."
    fi
    exit 0
fi
[[ $EUID -ne 0 ]] || die "run this as a normal user (it uses sudo)"

# USB input devices: "vendor:product<TAB>name", one line per device (not per interface).
list_devices() {
    for dev in /sys/class/input/event*; do
        local id="${dev}/device/id"
        [[ $(cat "$id/bustype" 2>/dev/null) == 0003 ]] || continue # USB only
        printf '%s:%s\t%s\n' "$(cat "$id/vendor")" "$(cat "$id/product")" "$(cat "$dev/device/name")"
    done | sort -u -t$'\t' -k1,1
}

ids=()
if [[ $# -gt 0 ]]; then
    for arg in "$@"; do
        [[ $arg =~ ^[0-9a-fA-F]{4}:[0-9a-fA-F]{4}$ ]] || die "not a USB id (vendor:product, e.g. 4842:0001): $arg"
        ids+=("${arg,,}")
    done
else
    mapfile -t devices < <(list_devices)
    [[ ${#devices[@]} -gt 0 ]] || die "no USB input devices found. Is the remote's receiver plugged in?"
    echo "USB input devices:"
    for i in "${!devices[@]}"; do
        printf '  %d) %s   (%s)\n' "$((i + 1))" "${devices[$i]#*$'\t'}" "${devices[$i]%%$'\t'*}"
    done
    echo
    echo "Choose the REMOTE CONTROL (not your keyboard or mouse: those would become readable too)."
    read -r -p "Number(s), separated by spaces: " -a choice
    [[ ${#choice[@]} -gt 0 ]] || die "nothing chosen"
    for c in "${choice[@]}"; do
        [[ $c =~ ^[0-9]+$ && $c -ge 1 && $c -le ${#devices[@]} ]] || die "invalid choice: $c"
        ids+=("${devices[$((c - 1))]%%$'\t'*}")
    done
fi

info "Writing $RULE"
{
    echo "# familynovice: read access to these remote controls for the user at the screen"
    echo "# (scripts/remote-access.sh; remove with: scripts/remote-access.sh --remove)."
    # Keep remotes allowed earlier.
    [[ -f $RULE ]] && grep '^SUBSYSTEM' "$RULE" || true
    for id in "${ids[@]}"; do
        echo "SUBSYSTEM==\"input\", KERNEL==\"event*\", ENV{ID_VENDOR_ID}==\"${id%%:*}\", ENV{ID_MODEL_ID}==\"${id##*:}\", TAG+=\"uaccess\""
    done
} | awk '!seen[$0]++' | sudo tee "$RULE.new" >/dev/null
sudo mv "$RULE.new" "$RULE"
sudo chmod 644 "$RULE"
reload

info "Readable now"
found=0
for dev in /sys/class/input/event*; do
    node="/dev/input/${dev##*/}"
    if [[ -r $node ]]; then
        echo "  $node  $(cat "$dev/device/name")"
        found=1
    fi
done
[[ $found -eq 1 ]] || echo "  (none yet: log out and in again, or unplug and replug the receiver)"
echo
echo "Next: admin panel → Home screen & media → Remote → \"Calibrate\"."
