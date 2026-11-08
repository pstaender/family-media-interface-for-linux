# curator

A family-friendly media center for Linux. It turns an ordinary Linux computer into a fullscreen
"TV" interface that children can use on their own: Blu-rays, shared folders (videos, music,
pictures) and selected websites, with a PIN, time limits and a lock screen.

It has two parts:

- **Media center** (Rust, `src/`): the fullscreen interface on the TV.
- **Admin panel** (Bun + React, `admin/`): a web page for parents, at http://127.0.0.1:8787/
  on the computer or `http://<computer's IP>:8787/` from a phone or laptop in the home network.
  It writes the settings and applies them to the running media center.

curator only **reads** media. It never changes, moves or deletes files outside its own
folders (see [Where files are stored](#where-files-are-stored)).

> **Built and tested on Linux Mint** (22, Cinnamon). Other distributions and desktops should
> work (it only uses freedesktop.org standards), but haven't been tested. Status: early
> stage; the kiosk installer (`scripts/install.sh`) is an example for Ubuntu/Mint.

## Install as a media center account (kiosk)

`scripts/install.sh` turns a user account into a media center account. Run it from your
own (admin) account; without `--user` it converts the current account:

```sh
sudo adduser tv                 # recommended: a separate account without admin rights
scripts/install.sh --user tv    # options: --no-makemkv, --skip-packages, --yes
scripts/install.sh --user tv --update   # later: only rebuild and replace curator
```

`--update` skips packages, MakeMKV, the Rust/Bun installs, the questions and the session
settings; it rebuilds the media center, website window and admin panel, replaces them in
`/opt/curator` and restarts the admin panel. The running media center keeps the old
version until it restarts.

It installs everything (packages, MakeMKV, Rust, Bun; builds curator into
`/opt/curator`). It also adds a login session **"curator"** and makes it the
account's default session. That session runs the media center in the kiosk compositor
[cage](https://github.com/cage-kiosk/cage), with no desktop, panels, app menu, Alt+Tab or
notifications. Players and the website window open on top, and the media center comes back
when they close. If it crashes, it restarts.

- **Next login** of the account: only the media center. **Ctrl+Q** ends the session (back to
  the login screen). Automatic login can be turned on in the system settings (Users / Login
  Window).
- **Admin panel:** runs as a user service of that account, also without a login, at
  http://127.0.0.1:8787/ or `http://<IP>:8787/` from other devices (the address is in
  `journalctl --user -u curator-admin` of that account). Set the admin password on the
  computer itself first.
- **Normal desktop again:** choose it in the session menu of the login screen (only for this
  login), or uninstall:

```sh
scripts/uninstall.sh           # restores the previous session, removes /opt/curator
scripts/uninstall.sh --purge   # also deletes curator's settings, usage and cache
```

Run the uninstaller from an account with sudo rights (for `--user tv`: your own account). System packages and
MakeMKV stay installed.

Limits of the example: made for GDM and LightDM (they read the default session from
AccountsService). With SDDM, pick the session once at the login screen. Inside the session
the remote's volume buttons change the computer's sound output (see "Using the media
center"). Website
file-upload fields and VLC's menus still open file dialogs; you can browse files there, but
nothing can be started.

The steps below are for development (running from the repository).

## Setup

Requirements: a Debian/Ubuntu/Mint system (the setup script uses `apt`), `sudo`, `curl`.

### 1. Tools

```sh
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Bun (admin panel)
curl -fsSL https://bun.sh/install | bash
```

Open a new terminal afterwards so `cargo` and `bun` are in your `PATH`.

### 2. System packages and MakeMKV

Run this in a normal terminal (it asks for your sudo password and for accepting the MakeMKV
license):

```sh
scripts/setup.sh
```

It installs mpv, VLC, Java (Blu-ray menus), the WebKitGTK build files (website window) and
builds **MakeMKV** from source into `/usr/local`. MakeMKV unlocks copy-protected Blu-rays; the
legality of that differs between countries, so you decide by running the script.

MakeMKV needs a key (a purchased key or the free beta key from the MakeMKV forum) in
`~/.MakeMKV/settings.conf` of **every user that plays discs**:

```
app_Key = "<your key>"
```

Without Blu-rays you can skip MakeMKV; folders and websites work without it.

### 3. Build

```sh
cargo build --workspace --release   # media center + website window (curator-web)
cd admin && bun install             # admin panel
```

`cargo build` without `--workspace` builds only the media center (no WebKit needed); website
tiles are then shown as "Not installed".

## Start

Start both parts (two terminals):

```sh
# Admin panel → http://127.0.0.1:8787/ (prints its network addresses too)
cd admin && bun run start           # or: bun run dev (port 8686, reloads on code changes)

# Media center
./target/release/curator               # fullscreen
./target/release/curator --windowed    # in a window (development)
```

### Admin panel: port, network access, pm2

By default the admin panel listens on **port 8787 on all network interfaces**, so it can be
opened from other devices in the home network. Change that with arguments, environment
variables or a file (highest priority first):

```sh
bun run start -- --port 8080 --host 127.0.0.1   # 127.0.0.1 = only this computer
CURATOR_ADMIN_PORT=8080 CURATOR_ADMIN_HOST=127.0.0.1 bun run start
```

`~/.config/curator/admin-server.json`:

```json
{ "port": 8080, "host": "0.0.0.0", "allowed_hosts": ["tv.fritz.box"] }
```

The panel answers to IP addresses, `localhost` and the computer's name (`<name>`,
`<name>.local`); other names (e.g. a router's DNS name) must be listed in `allowed_hosts`.
The **first admin password can only be set on the computer itself**; afterwards you can log
in from any device. The connection is plain HTTP, so only use it in a network you trust. A
firewall (e.g. `ufw`) may need `sudo ufw allow 8787/tcp`.

With **pm2** instead of `bun run start` (as the media center's user):

```sh
pm2 start ecosystem.config.js      # repository root; port/host: see args in the file
pm2 logs curator-admin
pm2 save && pm2 startup            # start again after a reboot
```

(`scripts/install.sh` uses a systemd user service instead; use only one of them.)

Use the same build profile for both binaries: the media center finds `curator-web` next
to itself (`target/release/` or `target/debug/`), otherwise in `PATH`.

### First steps in the admin panel

1. Open http://127.0.0.1:8787/ on the computer and choose an admin password.
2. **Home screen & media:** title, language of the TV interface (automatic, English or
   German), disc settings, folders (with a folder picker) and websites.
   For websites, the panel looks up the site's logo after saving and shows it on the tile
   (option "Show the website's logo on the tile"; without it, a globe).
   **Screensaver:** off, a moving title (default, after 10 minutes), or aerial videos (Apple
   TV style): pick up to 10 from the list (AerialWindows/Aerial project); they are downloaded
   from Apple's server after saving (100–900 MB each, depending on the quality). While a
   screensaver is set up, the computer doesn't switch the screen off.
   **Home screen → Restart media center** starts it again (e.g. after `install.sh --update`).
   **Games:** Block Stacker (falling blocks, 1 or 2 players), Brick Breaker (1 player, 24 stages and 8 bonus stages), both with high score lists, and Ping Pong for 1 player (against the computer) or 2 players, with USB gamepads
   (SNES, Xbox or PlayStation style; just plug them in), the remote's arrows or the keyboard.
3. **Password & PIN:** set a PIN (4–8 digits). It unlocks the media center when time is up and
   enables the lock button on the home screen.
4. **Parental control → Time limits:** time budgets or time windows for today, specific
   weekdays, this week or every day. **Status** shows usage and lets you unlock or lock.
5. Click **Save and apply**: the media center uses the new settings right away.

### Remote control

Arrow keys, OK (as a click) and Back (as a right click, like air-mouse remotes send it) work
right away. For the other buttons (OK outside pointer mode, home, play/pause, next,
previous, volume up / down, mute):

1. Run `scripts/remote-access.sh` once (in a normal terminal; asks for your password) and
   choose the remote from the list. It lets the logged-in user read **only that device**, so
   the media center gets every button, also those X11 never delivers to any window (many
   remotes send OK as `KEY_SELECT`). Undo: `scripts/remote-access.sh --remove`.
2. Admin panel → **Home screen & media → Remote → Set up the remote on the TV**. Switch the
   remote's mouse / pointer mode off first (in that mode it sends other buttons). The TV asks
   for one button after the other; a button your remote doesn't have is skipped after 10 s.
   The result is saved in `~/.local/share/curator/remote.json` and shown in the admin
   panel.

### Using the media center

| Key | Action |
| --- | --- |
| Arrow keys | move between tiles / entries |
| Enter, Space | open / select |
| Esc, Backspace, remote "back" | go back; closes a video |
| Right click (the "return" button of air-mouse remotes) | like Esc; on websites: leave fullscreen, previous page, or close on the first page |
| Media keys (remote, keyboard) | music: next / previous / play-pause / stop; pictures: next / previous; video: chapters, play-pause, stop |
| Hold Esc for 3 s | close a website |
| Volume up / down, mute (remote, keyboard) | system volume of the sound output (e.g. HDMI to the TV or receiver), in 5 % steps up to 100 %; also while a video or website is in front. Set the TV / receiver fairly loud once. On a normal desktop, the desktop handles the standard volume keys itself |
| Hold the remote's home button for 3 s | stop what is playing (music, video, website) and show the home screen |
| Power button at the top right (or Ctrl+Q) | short press: asks (exit / shut down / cancel); hold 3 s and release: log out; hold 8 s: shut the computer down (a ring and a progress bar show how long). With a PIN set, only with the PIN. Logging out in the kiosk session goes back to the login screen |

Everything also works with the mouse: tiles, menus, the "‹ Back" buttons and the PIN pad are
clickable.

## Debugging

### Logs

Both parts log to the terminal they were started from. The media center logs at `info` by
default; more detail with `RUST_LOG`:

```sh
RUST_LOG=debug ./target/debug/curator --windowed
RUST_LOG=warn,curator=debug ./target/debug/curator --windowed   # only our code
```

Player commands (mpv, VLC, `curator-web`) are logged with all arguments, so you can run
them by hand to see their own error messages.

### Test without touching your real settings

Both parts read `CURATOR_CONFIG_DIR`; a second admin panel can run on another port:

```sh
export CURATOR_CONFIG_DIR=/tmp/fn-test
(cd admin && CURATOR_ADMIN_PORT=8788 bun run dev)
./target/debug/curator --windowed
```

`docs/config.example.json` is a complete example config (format: `docs/config.schema.json`).

### Ask the running media center

The media center listens on a local socket (only for your user). `status` shows the config
state, lock state and usage:

```sh
python3 -c 'import socket; s=socket.socket(socket.AF_UNIX); s.connect("/run/user/'$(id -u)'/curator.sock"); s.sendall(b"{\"command\":\"status\"}\n"); print(s.makefile().readline())'
```

Other commands: `reload`, `unlock` (`"minutes": 30` or `"rest_of_day": true`), `end_unlock`,
`lock`.

### Website window alone

```sh
./target/debug/curator-web --windowed --url https://www.kika.de/ --allow kika.de --allow '*.kika.de'
```

Blocked pages are logged as `curator-web: blocked …`.

### Common problems

| Problem | Cause / fix |
| --- | --- |
| Blu-ray tile says "Reading disc…" for long | First read of a disc takes 1–2 min (copy protection). Known discs are cached. |
| "The player stopped right away" for a Blu-ray | MakeMKV missing or no key: run `scripts/setup.sh`, check `~/.MakeMKV/settings.conf`. Test with `makemkvcon -r info disc:0`. |
| Website tiles say "Not installed" | Build the website window: `cargo build -p curator-web` (same profile as the media center). |
| Video window invisible or slow over remote desktop | No graphics acceleration there; mpv falls back to SDL. Works normally on the machine itself. |
| Kiosk session goes back to the login screen | The media center crashed 5 times in a row: `journalctl -t curator -b`. |
| Admin panel not reachable after install | `systemctl --user status curator-admin` (as the media center account). Port 8787 already used by a development server? |
| Media center shows "Discs are turned off" / "Settings could not be loaded" | `config.json` is invalid; the admin panel shows the error. The media center then fails closed (locked-down settings). |
| Locked and no PIN set | Unlock in the admin panel: Parental control → Status. |
| Admin panel: "Too many wrong passwords" | Wait 60 s. To reset the password: stop the admin panel, delete `admin.json` (see below), open the panel again. |

## Where files are stored

| Path | Content |
| --- | --- |
| `~/.config/curator/config.json` | settings (written by the admin panel) |
| `~/.config/curator/admin.json` | admin password hash |
| `~/.local/share/curator/` | resume positions, usage statistics (`statistics/`, only if turned on), usage time (`usage.json`), remote buttons (`remote.json`), recently played files per folder (`recent.json`) |
| `/etc/udev/rules.d/70-curator-remote.rules` | read access to the remote (`scripts/remote-access.sh`) |
| `~/.cache/curator/` | disc info, website cookies and logos, screensaver videos (`aerial/`), player screenshots, MakeMKV build |
| `/run/user/<uid>/curator*.sock` | control sockets |
| `/opt/curator/` | installed programs (`scripts/install.sh`), `install-state` for the uninstaller |
| `/usr/share/wayland-sessions/curator.desktop` | the kiosk login session |
| `~/.config/systemd/user/curator-admin.service` | admin panel service |

(The standard XDG variables `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` are respected.)

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test
cd admin && bun run typecheck && bun test
```

The config format is validated identically in `src/config.rs` and `admin/src/config.ts`;
change both together. More details on the architecture and decisions: [CLAUDE.md](CLAUDE.md).
