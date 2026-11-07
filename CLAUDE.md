# CLAUDE.md

This file gives Claude Code guidance for working in this repository.

## Project overview

**familynovice** turns an ordinary Linux computer into a family-friendly media center. The target
users are families, including children and non-technical adults, so simplicity, safety and
reliability matter more than power-user features.

It has two parts:

1. **Media center UI (Rust):** a native, fullscreen desktop application. It is the only thing
   family members see and use. It shows a row-based home screen, similar to streaming services,
   with the most important functions and apps.
2. **Admin web interface (Bun + React):** a browser-based admin tool that parents use to set
   everything up. It writes the JSON config files and then applies them to the running media
   center app (reload, or restart if needed).

### Deployment target

- The media center is meant to run under a **dedicated Linux user account that only runs this
  app** (autostarted on login, fullscreen). Family members never see the regular desktop.
- **Linux desktop only.** It must work on all major desktop environments (GNOME, KDE Plasma,
  Cinnamon, …) without depending on any one of them. Use freedesktop.org standards (XDG
  directories, D-Bus, UDisks2, `.desktop` autostart) and support both Wayland and X11. Avoid
  code paths specific to one desktop environment. If one can't be avoided, it must be optional
  and must fall back cleanly.

### Configuration

- Config files are **JSON**, stored in `$XDG_CONFIG_HOME/familynovice/` of the media center's
  user account (falls back to `~/.config/familynovice/`). Resolve the path with the standard
  XDG rules (e.g. the `directories`/`dirs` crate in Rust), never hardcode `~/.config`.
- Usage data/statistics go into `$XDG_DATA_HOME/familynovice/` (default `~/.local/share/familynovice/`).
- The config schema is the contract between the two parts: `docs/config.schema.json`, with
  `docs/config.example.json` checked by the tests on both sides. Validation lives in
  `src/config.rs` (Rust) and `admin/src/config.ts` (TypeScript) and must accept and reject the
  same things (`rejects_bad_configs` / `rejects %p` test lists). Bump `version` for
  incompatible changes and update both sides together.
- Files in the config dir: `config.json` (media center settings, written by the admin
  interface) and `admin.json` (admin password hash, mode 0600, never read by the media center).
- `FAMILYNOVICE_CONFIG_DIR` overrides the config directory on both sides (tests, later the
  hardened `/etc/familynovice/` setup).
- Missing `config.json`: defaults. Invalid at startup: locked-down settings (discs off),
  error reported via `status`. Invalid on reload: last valid config stays active.

> Status: early stage. Implemented so far: the home screen with a Blu-ray tile, disc detection
> and Blu-ray playback (main movie / disc menu / resume / eject), the config file with live
> reload over the control socket, and a first admin interface (password, disc settings).
> Update this file as more gets decided.

## Non-destructive: read-only media access

familynovice is a **non-destructive media center**. It never writes, moves, renames or deletes
anything outside its own folders:

- `$XDG_CONFIG_HOME/familynovice/` (config, admin password hash),
- `$XDG_DATA_HOME/familynovice/` (resume positions, later usage data),
- `$XDG_CACHE_HOME/familynovice/` (disc info, build files, player screenshots),
- `$XDG_RUNTIME_DIR` / temp dir (sockets).

Media (discs, shared folders, everything else) is only ever **read**. Code that touches media
uses read-only operations (`read_dir`, `metadata`, open for reading); no create/write/rename/
remove calls on media paths, and no "convenience" features that change files (thumbnails next
to media, playlists, tags, deleting). Players and viewers we start must not write next to the
media either: mpv gets `--screenshot-dir` and `--watch-later-dir` inside our cache, VLC gets
`--snapshot-path` inside our cache. The only exception touching a device is ejecting a disc
(and unmounting it for that), which doesn't change any file.

## Features

### Media center UI (native, Rust)

- **Home screen:** horizontal rows/shelves of large tiles (e.g. "Discs", "Websites", "Apps",
  "Recently used"). Only the most important functions are on the first screen. Use our own visual
  design; don't copy any existing product's look.
- **Smart search:** one search field covering everything the current profile may use (apps,
  websites, discs, media). Make it typo-tolerant (fuzzy matching) and never let it show
  items the profile isn't allowed to see.
- **PIN protection:** a general PIN protects settings, leaving the media center, restricted
  items and extending time limits.
- **Timer / usage monitor:** track usage time per profile and per app/site/disc. Enforce daily
  limits and allowed time windows, warn before time runs out, and stop or lock cleanly when it
  does. Usage data stays on the machine and the admin interface can show it.
- **10-foot UI:** usable from the couch with a remote, gamepad or keyboard (mouse optional).
  Large targets, clear focus highlight, few choices per screen.

### Optical disc playback (DVD and Blu-ray, including copy-protected discs)

- Play DVDs and Blu-ray discs straight from the drive, including discs with copy protection
  (CSS on DVD; AACS and BD+ on Blu-ray).
- Expected libraries: `libdvdread`/`libdvdnav` + `libdvdcss` for DVD, and `libbluray` for
  Blu-ray. Blu-ray decryption comes from **MakeMKV** (see below).
- **Do not bundle or ship decryption keys, `KEYDB.cfg`, MakeMKV or other decryption libraries
  in this repository.** `scripts/setup.sh` downloads MakeMKV from makemkv.com at install time
  and builds it from source; everything else is loaded at runtime from the system. If
  something is missing, detect it and show a clear, friendly message instead of failing.
  The legality of circumventing copy protection differs between countries, so leave that
  decision to the user's installation.
- Detect inserted and ejected discs and offer playback automatically. Support disc menus where
  the backend allows it, and otherwise start the main title.

#### Blu-ray (implemented, v1)

- **Decryption:** MakeMKV's `libmmbd` stands in for `libaacs`/`libbdplus`. Players load it via
  the env vars `LIBAACS_PATH=libmmbd` and `LIBBDPLUS_PATH=libmmbd`, so no system library gets
  replaced. `libmmbd` calls `makemkvcon`, which needs a MakeMKV key in
  `~/.MakeMKV/settings.conf` of **every user that plays discs** (including the media center
  account).
- **Detection (`src/disc.rs`):** poll `/dev/sr*` every 2 s with the `CDROM_DRIVE_STATUS` ioctl
  (tray/disc state) and read udev properties (`udevadm info`: `ID_CDROM_MEDIA_BD`,
  `ID_FS_LABEL`, `ID_FS_UUID`). This is desktop independent and works without udisks.
  Later this could switch to a udev monitor instead of polling.
- **Disc analysis (`src/makemkv.rs`):** `makemkvcon -r --progress=-same info dev:/dev/srX`
  runs in the background (about 1–2 min for a BD+ disc) and returns the disc name and titles.
  Its `PRGC`/`PRGV` progress lines drive the progress shown on the tile. MakeMKV is silent
  for long stretches (decrypting, BD+), so the percentage is an estimate weighted by
  measured phase durations, and time moves the bar within a phase. The main movie is the longest
  title (≥ 20 min), then most chapters, then biggest size. Discs with "playlist obfuscation"
  can fool this, so a per-disc override is planned.
- **Tile:** the "Discs" row shows a Blu-ray tile with our own disc icon (not the Blu-ray
  logo). It's fully visible with the disc name when a Blu-ray is inserted, and semi-transparent
  (35 %) otherwise, with a hint such as "Insert a Blu-ray" or "Tray is open".
- **Disc id:** `<ID_FS_UUID>_<ID_FS_LABEL>`. The label alone isn't unique: the 2D and 3D
  discs of "Kubo and the Two Strings" both have `KUBO_TWO_STRINGS`.
- **Disc info cache (`src/store.rs`):** scan results are stored as JSON in
  `$XDG_CACHE_HOME/familynovice/discs/<disc id>.json`, so a known disc is ready right away.
  Playback still takes ~16 s to start, because libmmbd unlocks the disc again in the player.
- **Resume (`src/resume.rs`):** mpv gets `--input-ipc-server=$XDG_RUNTIME_DIR/familynovice-mpv.sock`.
  A thread observes `time-pos`/`duration`, saves every 5 s and when mpv exits, to
  `$XDG_DATA_HOME/familynovice/resume/<disc id>.json`. Resume is offered from 1 min on and
  dropped within the last 5 min (end credits). Resuming passes `--start=<secs>`.
  Unix socket paths are limited to 107 characters.
- **Eject (`disc::eject`):** 1. `CDROM_LOCKDOOR` + `CDROMEJECT` ioctls. 2. If refused, unmount
  with `udisksctl` (desktop automount) and retry. 3. If still refused, run util-linux `eject`,
  which uses SCSI commands. Needed on the Pioneer BDR-UD04: after unmounting, the kernel
  keeps the door locked (`EBUSY`, other processes have the device open). Note: `CDROMEJECT`
  can return a **positive** SCSI status (e.g. 2 = CHECK CONDITION) instead of an error, so
  treat any non-zero result as failure.
- **Start options** (asked when the tile is activated, `src/player.rs`): "Resume from h:mm:ss"
  (only if played before), "Play movie" / "Play from beginning", "Disc menu", "Eject disc",
  "Back". In terms of the original wish list:
  - **(b) Play movie:** `mpv bd://mpls/<main playlist>`. Plays the main movie directly with no
    menus or trailers. Works reliably, apart from obfuscated discs.
  - **(c) Disc menu:** `vlc bluray:///dev/srX`. Starts the disc like a normal player, with
    menus (BD-J menus need Java + `libbluray-bdj`). Works on most discs; some BD-J menus fail.
  - **(a) Skip intros/warnings and jump straight to the menu:** not implemented yet. The disc
    itself forbids skipping during warnings (UOP masks), and libbluray enforces that. Plan:
    build libbluray from source with a patch that ignores UOP masks, then call
    "top menu" right after first play. Best effort: BD-J discs often only build their menu
    after the intro, so it won't work on every disc.
- Parental controls apply to discs too: respect DVD/Blu-ray parental levels where they exist,
  and let profiles block or require approval for disc playback.

### Folders (implemented, v1)

- The admin adds folders (e.g. a shared movie, music or photo folder). Each folder gets a name
  and a filter for which files are visible: **videos**, **music**, **pictures** (fixed
  extension lists in `src/library.rs`) and/or **extra file extensions** (e.g. `pdf`).
- On the home screen, each folder is a tile in the "Folders" row. It can be browsed (subfolders
  plus visible files); hidden files (`.name`) are never shown. A folder that doesn't exist or
  can't be read is shown semi-transparent.
- **Read-only** (see above). Browsing is confined to the configured folder: paths are
  canonicalized and must stay inside it, so `..` and symlinks pointing outside are refused.
- Opening: videos with mpv (one file), music with mpv (from the chosen track to the end of the
  folder), pictures with mpv as a slideshow of the folder's pictures starting at the chosen one,
  extra extensions with the desktop's default application (`xdg-open`). Note: such
  applications may offer "save as" and similar; only add extensions whose default viewer is
  harmless.
- Config: `folders` in `config.json` (id, name, absolute path, `video`/`audio`/`images`
  booleans, `extensions`). The admin interface has a folder picker (`GET /api/browse`, lists
  directories only, read-only).

### Allowed websites (domain allowlist with wildcards)

- The admin sets up which websites each profile may visit. Entries are domains with optional
  wildcards, e.g. `example.org`, `*.example.org`.
- Default deny: anything not on the allowlist is blocked, including redirects, popups, new
  windows and embedded frames to other domains.
- Matching rules must be exact and tested. Specify (and test) whether `*.example.org` also
  matches the apex `example.org`. Compare against the normalized host (lowercase,
  punycode/IDN, trailing dot removed), never against a substring of the URL, so
  `example.org.evil.com` or `evil.com/?example.org` don't match.
- Always block `localhost`, loopback (`127.0.0.0/8`, `::1`) and private network addresses,
  whatever the allowlist says, so the child's browser can never reach the admin interface.
- Websites open in a locked-down browser view controlled by the media center (no address bar,
  no way to get to other sites or the desktop) and come back to the home screen on exit.

### Allowed software and scripts (set up by the admin)

- The admin can register specific programs or scripts as tiles (name, icon, command, arguments,
  working directory, allowed profiles).
- Only run what is registered. Launch with an explicit program + argument list, never via a
  shell string that includes user-supplied text, and never let the UI enter arbitrary commands.
- The media center supervises the child process: usage time counts toward limits, the process
  gets ended when time runs out or on a PIN-protected "stop", and the UI comes back to the
  front when it exits or crashes.

### Admin web interface (Bun + React)

- Built with **Bun** (runtime, package manager, bundler, test runner) and **React**
  (TypeScript preferred).
- Manages profiles, PIN, time limits, allowed websites, registered apps/scripts, disc settings,
  and shows usage statistics.
- It writes the JSON config files into the media center user's config directory (see
  Configuration) and then **applies** them to the running Rust app. The Rust app reloads the
  config (file watching and/or an explicit reload signal or local IPC), or the admin
  interface restarts it when a reload isn't enough. Apply must not leave the screen black or
  show the desktop. If the app doesn't come back, show an error in the admin interface.
- The Rust app must check config files and handle invalid ones safely
  (keep the last valid config, log the error, report it to the admin interface).
- Security: the admin interface must require authentication. By default it should only listen
  on localhost. Allow LAN access only when explicitly turned on. Never store the PIN or
  passwords in plaintext; store a slow hash (e.g. argon2) that both sides verify the same way.

#### Implemented (v1)

- **Server (`admin/src/app.ts`):** `Bun.serve` on `127.0.0.1:8787` (`FAMILYNOVICE_ADMIN_PORT`).
  Serves the React app (`src/web/`, bundled by Bun via HTML import) and a JSON API:
  `GET /api/state`, `POST /api/setup|login|logout|password`, `GET|PUT /api/config`,
  `GET /api/status`.
- **Security checks on every API call:** `Host` must be `127.0.0.1:<port>` or
  `localhost:<port>` (DNS rebinding); changing requests need `X-Familynovice: 1` and, if
  present, a same-origin `Origin` (CSRF); session cookie `HttpOnly; SameSite=Strict`, 8 h,
  in memory (a server restart logs everyone out); request bodies ≤ 64 KB.
- **Auth (`admin/src/auth.ts`):** first visit sets the admin password (≥ 8 characters),
  stored as argon2id PHC string in `admin.json` (created with `O_EXCL`, so setup can't be
  repeated). 5 wrong passwords within 15 min lock the login for 60 s.
- **Apply:** `PUT /api/config` validates, writes `config.json` atomically, then sends
  `{"command":"reload"}` to the media center's control socket (`src/control.rs`,
  `$XDG_RUNTIME_DIR/familynovice.sock`, mode 0600). If the media center isn't running, the
  config is still saved and applies at its next start.
- **Settings so far:** discs on/off, offer "Disc menu", preferred audio and subtitle languages
  (ISO 639-2/T codes; players also get the /B variant, e.g. `deu,ger`).
- Not yet: PIN, profiles, time limits, websites, apps, LAN access, `restart`, file-watch
  fallback, systemd user service.

## Repository layout

```
Cargo.toml        Rust media center (single crate for now; may become a workspace)
src/
  main.rs         entry point, window setup (`--windowed` for development)
  ui.rs           home screen / front row (egui): rows with focus navigation, disc chooser,
                  folder browser, overlays
  disc.rs         optical drive + disc detection (ioctl + udev)
  library.rs      read-only folder browsing (file type filters, confinement to the folder)
  makemkv.rs      makemkvcon robot-output parser, main title selection
  player.rs       builds the player commands (mpv / VLC with libmmbd)
  resume.rs       resume positions via mpv IPC
  store.rs        JSON files in XDG dirs (disc cache, resume), atomic writes
scripts/
  setup.sh        installs players, builds MakeMKV from source
  config.rs       config.json: types, validation, load/reload (fail closed)
  control.rs      control socket for the admin interface (reload/status)
admin/            Bun + React admin web interface
  src/server.ts   entry point
  src/app.ts      HTTP server, API routes, security checks
  src/auth.ts     admin password (argon2id), sessions, login rate limit
  src/config.ts   config types + validation (mirror of src/config.rs)
  src/control.ts  control socket client
  src/files.ts    XDG paths, atomic JSON writes
  src/web/        React app (index.html, main.tsx, styles.css)
  test/           bun tests (config validation, API incl. security checks)
docs/
  config.schema.json   config format (contract between both parts)
  config.example.json  example, validated by tests on both sides
```

## Development commands

Rust media center:

```sh
cargo build                                  # debug build
cargo build --release                        # release build
cargo run                                    # run the media center (fullscreen)
cargo run -- --windowed                      # run in a window (development)
cargo test                                   # run all tests
cargo test <name>                            # run tests matching a name
cargo fmt                                    # format code
cargo clippy --all-targets -- -D warnings    # lint (treat warnings as errors)
```

System setup (players, MakeMKV built from source into `/usr/local`; needs sudo, asks to accept
the MakeMKV license):

```sh
scripts/setup.sh
```

Development keys: arrows to move, Enter/Space to select, Esc/Backspace to go back,
Ctrl+Q to quit (to be protected by the PIN).

Admin web interface (from `admin/`):

```sh
bun install          # install dependencies
bun run dev          # start the server with hot reload → http://127.0.0.1:8787/
bun run start        # start in production mode
bun run typecheck    # tsc --noEmit
bun test             # run tests
```

For experiments without touching the real config, run both parts with
`FAMILYNOVICE_CONFIG_DIR=/some/tmp/dir` (and `FAMILYNOVICE_ADMIN_PORT` for a second server).

Before you consider a change done, run the formatter, linter and tests for every part you touched.

## Code conventions

### Rust

- Use stable Rust (2021 edition or newer) and idiomatic code. Keep it formatted with `rustfmt`
  and free of clippy warnings.
- Error handling: return `Result` instead of calling `unwrap()`/`expect()` in non-test code,
  unless the invariant really cannot fail (and then say why in the `expect` message).
- Keep the policy logic (profiles, PIN, time limits, domain matching, allowed apps) separate from
  the UI and from OS integration, so it can be tested without a display.
- Put unit tests next to the code in `#[cfg(test)] mod tests` and integration tests in `tests/`.

### Bun / React

- TypeScript, function components and hooks. Use Bun's built-in tools (`bun test`, the Bun bundler)
  before adding other tooling.
- Validate all input on the server side before writing config files. Write files atomically
  (write a temp file, then rename) so the media center never reads a half-written file.

### General

- Security- and child-safety-relevant code (PIN checks, domain matching, time limits, launcher)
  must have tests, including tests for ways around it.
- Fail closed: if config is missing or invalid, or a check errors, deny access instead of allowing it.
- Non-destructive: never write outside familynovice's own folders (see "Non-destructive:
  read-only media access"). Review every new file operation against this.
- No telemetry and no unneeded network calls. Usage data and config stay on the machine.
- Follow the XDG Base Directory spec for config, data and cache locations.
- Stay desktop-environment agnostic (see Deployment target).
- Add dependencies sparingly and prefer well-maintained crates and packages. Explain why a new
  dependency is needed when adding it.
- Keep user-facing strings out of deep logic so localization is possible later.

## Planned architecture

### Admin interface: user, permissions, authentication

- **v1:** the Bun admin server runs as a **systemd user service of the media center user**. It
  owns `$XDG_CONFIG_HOME/familynovice/` and needs no extra permissions. It listens on
  `127.0.0.1` by default, so parents can also reach it from their own account on the same computer.
- **Known weakness:** every process of the media center user can write the config, including
  registered apps and scripts. Registered apps must therefore not give file access (no file
  managers, terminals, "save as" dialogs with free paths). Document this in the admin interface
  where apps are registered.
- **Later hardening:** the admin server runs as a separate system user and the config moves to
  `/etc/familynovice/` (writable by the admin user, read-only for the media center user). To make
  this switch easy, the Rust app reads `$XDG_CONFIG_HOME/familynovice/` by default but accepts an
  override path (command-line flag / environment variable).
- **Authentication:** on first start, the admin interface asks to set an admin password.
  Passwords and the PIN are hashed with argon2 via `Bun.password.hash()`. The result is a
  standard PHC string that the Rust app verifies with the `argon2` crate, so both sides check
  the same stored hash. Use cookie sessions (`HttpOnly`, `SameSite=Strict`) with CSRF
  protection. LAN access is opt-in.

### Applying config changes (reload / restart)

- The Rust app opens a **Unix socket at `$XDG_RUNTIME_DIR/familynovice.sock`** with the commands
  `reload`, `restart` and `status`. Replies are structured (JSON), e.g. "ok" or the exact
  validation error, so the admin interface can show the result immediately.
- **Reload is the normal case:** load the new config, validate it, swap it in live. If it's
  invalid, keep the old config and return the error.
- **File watching** (`notify` crate, debounced) is a fallback, for example for hand-edited JSON.
- **Restart:** the app re-executes itself (`exec` of its own binary) and keeps the window up
  until the new instance is ready. No dependency on systemd's graphical-session support, which
  isn't equally reliable on every desktop. Only needed for changes that can't be applied live
  (e.g. switching the playback backend).
- **Crash recovery:** a small supervisor mode (`familynovice --supervise`, started at login)
  restarts the app if it exits unexpectedly. Back off if it keeps crashing, and show an error
  screen instead of the desktop.

### Dedicated media center account and session

- **Default: kiosk session.** A minimal Wayland kiosk compositor (e.g. **cage**) runs only
  familynovice. It's installed as its own session entry (`/usr/share/wayland-sessions/`), next
  to GNOME, KDE, Cinnamon, …, and the display manager logs the media center user into it
  automatically. This is desktop-environment independent and has no desktop to escape to (no
  panels, Super key, Alt+Tab, notifications). Registered apps and the browser show fullscreen
  on top. When they close, the media center is back.
- **Fallback: desktop autostart.** A normal desktop session that starts the app via an XDG
  autostart `.desktop` file. Easier, but each desktop's shortcuts can reach the desktop, so it's
  only for testing or younger children, not the locked-down mode.
- **Account hardening:** normal user without admin rights (not in `sudo`/`wheel`), strong
  password even with autologin. Ctrl+Alt+F1–F6 lead to the other text consoles, which ask for a
  password.
- **Leaving the media center:** with the PIN, it logs out to the login screen, where parents log
  into their own account.
- **Installer / setup script:** autologin works differently per display manager (GDM, SDDM,
  LightDM). Handle that in a setup script that supports all three, not in the app, so the app
  itself stays desktop-agnostic.

## Decisions made

- **UI toolkit:** `egui` via `eframe` (wgpu renderer; runs on Wayland and X11). The home screen
  is painted by hand (`Painter`) for a 10-foot look, scaled relative to a 1080p layout.
- **Disc playback:** separate fullscreen player processes. mpv plays the main movie, VLC plays
  with disc menus. Decryption comes from MakeMKV's `libmmbd`. The media center waits for the
  player to exit and then shows the home screen again.
- **System packages (apt):** see `APT_PACKAGES` in `scripts/setup.sh`.
- **Admin ↔ media center:** JSON lines over a Unix socket (`src/control.rs`); config as one
  `config.json`, validated on both sides.

## Open decisions

Record each choice here once it is made:

- embedded browser engine for the website allowlist (e.g. WebKitGTK, CEF) and how blocking is enforced
- whether to offer HTTPS when LAN access to the admin interface is turned on
- when to move to the hardened config setup (see "Admin interface: user, permissions, authentication")
- support for non-apt distributions in `scripts/setup.sh`
