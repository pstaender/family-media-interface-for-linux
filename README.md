# familynovice

A family-friendly media center for Linux. It turns an ordinary Linux computer into a fullscreen
"TV" interface that children can use on their own: Blu-rays, shared folders (videos, music,
pictures) and selected websites, with a PIN, time limits and a lock screen.

It has two parts:

- **Media center** (Rust, `src/`): the fullscreen interface on the TV.
- **Admin panel** (Bun + React, `admin/`): a web page for parents, at http://127.0.0.1:8787/.
  It writes the settings and applies them to the running media center.

familynovice only **reads** media. It never changes, moves or deletes files outside its own
folders (see [Where files are stored](#where-files-are-stored)).

> Status: early stage, tested on Linux Mint 22 (Cinnamon). Running it as a locked-down kiosk
> account is planned but not automated yet.

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
cargo build --workspace --release   # media center + website window (familynovice-web)
cd admin && bun install             # admin panel
```

`cargo build` without `--workspace` builds only the media center (no WebKit needed); website
tiles are then shown as "Not installed".

## Start

Start both parts (two terminals):

```sh
# Admin panel → http://127.0.0.1:8787/
cd admin && bun run start           # or: bun run dev (reloads on code changes)

# Media center
./target/release/familynovice               # fullscreen
./target/release/familynovice --windowed    # in a window (development)
```

Use the same build profile for both binaries: the media center finds `familynovice-web` next
to itself (`target/release/` or `target/debug/`), otherwise in `PATH`.

### First steps in the admin panel

1. Open http://127.0.0.1:8787/ and choose an admin password.
2. **Home screen & media:** title, disc settings, folders (with a folder picker) and websites.
3. **Password & PIN:** set a PIN (4–8 digits). It unlocks the media center when time is up and
   enables the lock button on the home screen.
4. **Parental control → Time limits:** time budgets or time windows for today, specific
   weekdays, this week or every day. **Status** shows usage and lets you unlock or lock.
5. Click **Save and apply**: the media center uses the new settings right away.

### Using the media center

| Key | Action |
| --- | --- |
| Arrow keys | move between tiles / entries |
| Enter, Space | open / select |
| Esc, Backspace, remote "back" | go back; closes a video |
| Hold Esc for 3 s | close a website |
| Ctrl+Q | quit the media center (development; will be PIN-protected) |

Everything also works with the mouse: tiles, menus, the "‹ Back" buttons and the PIN pad are
clickable.

## Debugging

### Logs

Both parts log to the terminal they were started from. The media center logs at `info` by
default; more detail with `RUST_LOG`:

```sh
RUST_LOG=debug ./target/debug/familynovice --windowed
RUST_LOG=warn,familynovice=debug ./target/debug/familynovice --windowed   # only our code
```

Player commands (mpv, VLC, `familynovice-web`) are logged with all arguments, so you can run
them by hand to see their own error messages.

### Test without touching your real settings

Both parts read `FAMILYNOVICE_CONFIG_DIR`; a second admin panel can run on another port:

```sh
export FAMILYNOVICE_CONFIG_DIR=/tmp/fn-test
(cd admin && FAMILYNOVICE_ADMIN_PORT=8788 bun run dev)
./target/debug/familynovice --windowed
```

`docs/config.example.json` is a complete example config (format: `docs/config.schema.json`).

### Ask the running media center

The media center listens on a local socket (only for your user). `status` shows the config
state, lock state and usage:

```sh
python3 -c 'import socket; s=socket.socket(socket.AF_UNIX); s.connect("/run/user/'$(id -u)'/familynovice.sock"); s.sendall(b"{\"command\":\"status\"}\n"); print(s.makefile().readline())'
```

Other commands: `reload`, `unlock` (`"minutes": 30` or `"rest_of_day": true`), `end_unlock`,
`lock`.

### Website window alone

```sh
./target/debug/familynovice-web --windowed --url https://www.kika.de/ --allow kika.de --allow '*.kika.de'
```

Blocked pages are logged as `familynovice-web: blocked …`.

### Common problems

| Problem | Cause / fix |
| --- | --- |
| Blu-ray tile says "Reading disc…" for long | First read of a disc takes 1–2 min (copy protection). Known discs are cached. |
| "The player stopped right away" for a Blu-ray | MakeMKV missing or no key: run `scripts/setup.sh`, check `~/.MakeMKV/settings.conf`. Test with `makemkvcon -r info disc:0`. |
| Website tiles say "Not installed" | Build the website window: `cargo build -p familynovice-web` (same profile as the media center). |
| Video window invisible or slow over remote desktop | No graphics acceleration there; mpv falls back to SDL. Works normally on the machine itself. |
| Media center shows "Discs are turned off" / "Settings could not be loaded" | `config.json` is invalid; the admin panel shows the error. The media center then fails closed (locked-down settings). |
| Locked and no PIN set | Unlock in the admin panel: Parental control → Status. |
| Admin panel: "Too many wrong passwords" | Wait 60 s. To reset the password: stop the admin panel, delete `admin.json` (see below), open the panel again. |

## Where files are stored

| Path | Content |
| --- | --- |
| `~/.config/familynovice/config.json` | settings (written by the admin panel) |
| `~/.config/familynovice/admin.json` | admin password hash |
| `~/.local/share/familynovice/` | resume positions, usage time (`usage.json`) |
| `~/.cache/familynovice/` | disc info, website cookies, player screenshots, MakeMKV build |
| `/run/user/<uid>/familynovice*.sock` | control sockets |

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
