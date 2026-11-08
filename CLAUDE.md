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
- **Title (implemented):** the text in the top-left corner (`home.title`, default
  "familynovice") can be changed or hidden (`home.show_title`) in the admin interface.
- **Smart search:** one search field covering everything the current profile may use (apps,
  websites, discs, media). Make it typo-tolerant (fuzzy matching) and never let it show
  items the profile isn't allowed to see.
- **PIN and time limits (implemented, v1):**
  - PIN: 4–8 digits, set/changed/removed in the admin panel (`POST`/`DELETE /api/pin`), stored
    only as argon2id hash (`parental.pin_hash`, Bun.password); the hash never goes to the
    browser (`GET /api/config` returns `pinSet`), a normal `PUT /api/config` keeps it and may
    not set it. The media center verifies with the `argon2` crate (`src/pin.rs`, tested
    against a real Bun hash) in a thread.
  - `parental.pin_length` (4–8, set with the PIN, also never sent to the browser): the lock
    screen shows that many dots and checks the PIN as soon as the last digit is entered (no
    OK needed). Every check counts as a try, so there is no checking of partial PINs.
  - Wrong PIN: wait 5 s; with `pin_backoff_doubling` (default on) 5, 10, 20, 40 s … (max 1 h).
    Failures and the waiting time are persisted, so a restart doesn't reset them.
  - Limits (`parental.limits`, `src/limits.rs`, pure + tested): time **budget** (minutes) or
    time **window** (`from` optional, `until`), each for **today** (stores the date),
    **weekdays**, **this week** (stores the Monday; a week budget counts the whole week) or
    **every day**. Most specific wins: today > weekdays > this week > every day; a week budget
    applies in addition to a day budget.
  - Usage (`src/usage.rs`, `$XDG_DATA_HOME/familynovice/usage.json`, last 60 days) counts only
    while something is open or within 60 s after the last input (an idle home screen doesn't
    use up time). The UI evaluates the limits every second.
  - Time up / outside the window / locked by a parent: players, music and websites are stopped
    and the lock screen appears (`src/lock.rs`): number pad (remote arrows + OK, digit keys,
    mouse); after the right PIN: 30 more minutes, 1 more hour or rest of today. Without a PIN:
    "A parent can unlock it in the admin panel". In the last 5 minutes a countdown ("Time left 4:32",
    red in the last minute) shows in the top-right corner.
  - Lock button (padlock, top right of the home screen, only when a PIN is set): locks at
    once; reachable with "up" from the top row.
  - Admin panel → Parental control → Status: locked/unlocked, used today, time left, unlock
    (30 min / 1 h / rest of today), "End unlock" (back to the time limits; locks only if the
    time is used up) and "Lock now" (control socket `unlock` / `end_unlock` / `lock`).
- **Media keys (implemented):** next / previous / play-pause / stop of remotes and keyboards.
  Desktops grab these keys and send them to MPRIS players, so the media center is one
  (`src/mpris.rs`, `org.mpris.MediaPlayer2.familynovice` via `zbus` 5 blocking API;
  `PlaybackStatus` follows the screen). Without MPRIS, Mint shows "unavailable". In the kiosk
  session the keys reach the window directly (egui `Key::Media*`). Music: next/previous
  track, pause, stop; pictures: next/previous; videos (mpv, via its IPC socket): chapters,
  pause; Stop closes the player. mpv's own NEXT/PREV bindings also jump chapters.
- **Air-mouse remotes** (tested: "HAOBO Technology USB Composite Device"): arrows are arrow
  keys, OK is a left click, **return is a right click**. So a right click means "back":
  media center (`Keys.back`), mpv (`MBTN_RIGHT quit`), website window (leave page
  fullscreen → previous page → close on the first page; WebKit's context menu is off).
- **Remote calibration (implemented, `src/remote.rs`):** many remote buttons never reach a
  window: X11 can't deliver key codes > 247 (e.g. `KEY_SELECT` 353 = OK outside pointer mode
  on the HAOBO remote, USB 4842:0001), egui drops keys it doesn't know. So the media center
  also reads input devices directly (`/dev/input/event*`, read-only, never grabbed, hotplug
  every 3 s). Access only to the remote: `scripts/remote-access.sh` writes
  `/etc/udev/rules.d/70-familynovice-remote.rules` (`TAG+="uaccess"` by USB id; logind gives
  the active session's user an ACL); not the `input` group (that would expose every keyboard).
  Calibration (admin → Home screen & media → Remote → control socket `calibrate_remote` →
  `Screen::Calibrate`): OK, Back, Home, arrows, Play/Pause, Next, Previous; each press is
  gathered for 300 ms from the device and the window, a button already used is refused,
  10 s without a press skips. Saved to `$XDG_DATA_HOME/familynovice/remote.json`. **No double
  actions:** a button whose press also reached the window stays the window's (its key names
  are added as extra bindings); device codes act only for buttons the window doesn't get,
  and only while our window has focus. Without calibration: KEY_OK/KEY_SELECT = OK, KEY_NEXT/KEY_PREVIOUS (407/412) = next/previous. Calibrated HAOBO remote (non-pointer mode): OK 353, back 158, home 172, play/pause 164, next 163, previous 165; Cinnamon swallowed 163-165 (neither window nor MPRIS got them), so they are read from the device.
  KEY_HOMEPAGE = home. Holding home works through the device also while a player/website
  (including VLC) is in front. `status` reports the readable devices and learned buttons.
- **Home button (implemented):** holding the remote's home button (XF86HomePage; egui
  `Key::BrowserHome`, mpv `HOMEPAGE`, GDK `HomePage`) for 3 s stops everything and shows the
  home screen. Media center window: `App::home_button` / `go_home` (hint while held). mpv:
  `player::MPV_HOME_SCRIPT` (Lua, `--script`, tested via IPC `keydown`) quits with
  `player::EXIT_GO_HOME` (4); familynovice-web exits with the same code; the "Playing"
  screen then goes home instead of back to the folder. VLC (disc menus) has no hold
  detection. Not yet confirmed which key the tested remote sends for home.
- **Touch/mouse:** folder browser, picture viewer and music player have a "‹ Back" button
  (no keyboard needed); the lock screen's number pad is clickable.
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
  dropped within the end credits: the last 5 min, but at most the last 10 % (short videos). Resuming passes `--start=<secs>`.
  Unix socket paths are limited to 107 characters.
- **Stopping playback:** Esc, Backspace and the back keys of remotes/gamepads/mice close mpv
  (`player::MPV_INPUT_CONF`, written to our cache and passed via `--input-conf`); VLC quits on
  Esc (`--key-quit=Esc`). On the media center's "Playing…" screen, Esc/Back (or a click; ignored
  during the first second, so the click/OK that started it doesn't stop it again) sends
  SIGTERM to the player and SIGKILL after 2 s, for players that are hung or not visible. The
  resume position survives both (saved on quit, and every 5 s while playing).
- **Player in front:** while mpv/VLC runs, the media center sets its own window to
  `WindowLevel::AlwaysOnBottom` (window managers may open the player behind our fullscreen
  window); when the player exits: `Normal` + `Focus` (`ui::App::keep_player_in_front`).
- **mpv video output:** `--vo=gpu-next,gpu,sdl,x11`. Xv/VDPAU are skipped: without a usable GPU
  (e.g. over RDP, display `:10`) mpv hung on Xv with a tiny invisible window. VLC gets
  `--no-qt-privacy-ask` (no first-run dialog) and deinterlacing always on
  (`--deinterlace=1 --deinterlace-mode=yadif`; disc menus and extras are often interlaced).
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
  and chooses which media is visible: **videos**, **music** and/or **pictures** (fixed
  extension lists in `src/library.rs`). **Only these media files are ever shown**; other
  files (documents etc.) never appear. Picture formats are limited to what the built-in viewer
  can decode (JPEG, PNG, GIF, WebP, BMP, TIFF).
- On the home screen, each folder is a tile in the "Folders" row. It can be browsed (subfolders
  plus visible files); hidden files (`.name`) are never shown. A folder that doesn't exist or
  can't be read is shown semi-transparent.
- **Read-only** (see above). Browsing is confined to the configured folder: paths are
  canonicalized and must stay inside it, so `..` and symlinks pointing outside are refused.
- Opening:
  - **Pictures:** built-in viewer (`src/pictures.rs`, `image` crate): decoded in a thread,
    turned upright (EXIF orientation), scaled to the screen; ←/→ previous/next, Esc back.
  - **Music:** built-in player screen (`src/audio.rs`). The sound comes from mpv without any
    window (`--no-video --force-window=no`), controlled over its IPC socket
    (`$XDG_RUNTIME_DIR/familynovice-audio.sock`): pause, previous/next, ±30 s; plays from the
    chosen track to the end of the folder. **Cover art** (`src/cover.rs`, read-only, in a
    thread): the picture embedded in the file (`lofty`; front cover first), else a cover file
    in the track's folder (`cover`, `folder`, `front`, `album`, `albumart`, `thumb`, exact
    name before prefix, e.g. `AlbumArt_{…}_Large.jpg`), else the folder's only picture.
    Files outside the configured folder (symlinks) are ignored.
  - **Videos:** the same mpv setup as the disc's main movie (`player::mpv_movie`): preferred
    audio/subtitle languages from the disc settings, resume tracking ("Resume from h:mm:ss" /
    "Play from beginning"; key `file-<FNV-1a of the path>`), nothing written next to the media.
- Config: `folders` in `config.json` (id, name, absolute path, `video`/`audio`/`images`). The
  admin interface has a folder picker (`GET /api/browse`, lists directories only, read-only).
  The removed `extensions` field is still accepted from older files but ignored.

### Websites (implemented, v1)

- The admin adds websites: name, start page (`https://…`, public domain name only: no IP
  address, `localhost`, `*.local`, user/password) and **"Allow all subdomains"** (default on).
- Rules (`src/sites.rs`, shared with the browser window, heavily tested):
  - **Pages (main frame):** with subdomains: the start host without a leading `www.` and all
    its subdomains (`www.kika.de` → `kika.de`, `*.kika.de`); without: exactly the start host.
    Hosts are compared after WHATWG URL normalization (lowercase, IDN → punycode, trailing dot
    removed), never as substrings (`kika.de.evil.com`, `evil.com/?kika.de`, `kika.de@evil.com`
    don't match).
  - **Media, scripts, styles, embedded players/frames:** from anywhere (websites use CDNs) …
  - … **except** local addresses and non-web schemes, blocked for every frame: `localhost`,
    `*.localhost`, `*.local`, loopback/private/link-local/CGNAT IPs (also IPv4-mapped IPv6),
    `file:` and other schemes. So a website can't reach the admin interface or devices at home.
- **Browser window `familynovice-web`** (`web/`, WebKitGTK via `wry` + `tao`): fullscreen,
  no address bar, only a round "‹" back button (top left, page history, dimmed without a
  previous page) and a ✕ close button (top right; both GTK overlays the page can't hide;
  hidden while the page itself is fullscreen, e.g. a video, via WebKit's enter/leave-fullscreen
  signals; theme hover effects switched off in the CSS). Holding **Esc**
  (or the remote's back key) for **3 s** closes it; short Esc presses still reach the page.
  **Every visited address is checked** (main frame, via WebKit's `load-changed`, which only
  concerns the main frame; on start, redirect and commit): a page outside the rules is
  stopped before it's committed and a dialog ("This page is not part of <name>", showing the
  blocked host) offers **"Go back"** (stay on the last allowed page; Enter) or **"Close
  website"**. Frames: wry's navigation handler (fires for all
  frames) only blocks local addresses. Pop-ups never open a new window: allowed ones load in
  the same window, others show the same dialog. **Downloads are blocked** (non-destructive). Cookies/cache live in
  `$XDG_CACHE_HOME/familynovice/web`.
- **Logos (`show_logo`, default on):** when the config is saved (and at admin start for
  missing ones), the admin interface (`admin/src/logos.ts`, Bun `fetch` + `HTMLRewriter`)
  reads the start page and picks the largest usable picture: icons ≥ 96 px (web app
  manifest, apple-touch-icon, `<link rel=icon sizes>`), then the preview image
  (`og:image`/`twitter:image`), then small icons, then `/favicon.ico`; no SVG. Only http(s)
  to public domain names (DNS answers in the home network are refused too), redirects
  checked per hop, 10 s / 4 MB limits, magic-byte check (PNG/JPEG/GIF/WebP/ICO/BMP). Stored
  as `$XDG_CACHE_HOME/familynovice/logos/<id>.img` + `<id>.json` (source), atomically;
  turned-off/removed ones are deleted. The media center never fetches anything: it checks
  the files' times every 5 s, decodes in a thread and draws the logo on a light plate
  (`WebsiteTile`); otherwise the globe. Admin: preview + "Look again"
  (`GET|POST /api/websites/:id/logo`, POST only for the saved website's URL).
- The media center starts it like a player (`--url`, `--allow` rules, `--title`,
  `--data-dir`), keeps it in front and can stop it (Esc on the "Playing…" screen). It looks
  for `familynovice-web` next to its own binary, then in `PATH`; if missing, website tiles are
  semi-transparent ("Not installed").
- Known limits: sub-resource requests (images, `fetch`) to local IPs aren't filtered (only
  navigations); a public domain that resolves to a local IP (DNS rebinding) isn't detected.
  The admin interface is still protected by its Host check and login.

### Allowed software and scripts (set up by the admin)

- The admin can register specific programs or scripts as tiles (name, icon, command, arguments,
  working directory, allowed profiles).
- Only run what is registered. Launch with an explicit program + argument list, never via a
  shell string that includes user-supplied text, and never let the UI enter arbitrary commands.
- The media center supervises the child process: usage time counts toward limits, the process
  gets ended when time runs out or on a PIN-protected "stop", and the UI comes back to the
  front when it exits or crashes.

### Admin web interface (Bun + React)

- Layout: left menu with three pages, each with tabs (URL `#/page/tab`):
  **Home screen & media** (Home screen, Discs, Folders, Websites), **Parental control**
  (Status, Time limits), **Password & PIN** (PIN, Admin password). One "Save and apply" for all
  config tabs. Icons: Tabler via Iconify's offline data, extracted into `src/web/icons.ts` by
  `bun run icons` (no requests to iconify.design at runtime).

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
- Security: the admin interface must require authentication. It is reachable from the home
  network by default (decided: parents use it from their phone or laptop); `--host 127.0.0.1`
  limits it to the computer itself. Never store the PIN or
  passwords in plaintext; store a slow hash (e.g. argon2) that both sides verify the same way.

#### Implemented (v1)

- **Server (`admin/src/app.ts`):** `Bun.serve` on `0.0.0.0:8787` by default. Port and address
  (`admin/src/settings.ts`, tested): command line `--port`/`--host` > env
  `FAMILYNOVICE_ADMIN_PORT`/`FAMILYNOVICE_ADMIN_HOST` > `<config dir>/admin-server.json`
  (`port`, `host`, `allowed_hosts`; unknown keys rejected) > defaults. Invalid values: exit with
  an error. pm2: `ecosystem.config.js` in the repo root (CommonJS; `admin/` is an ES module
  package), one fork instance (sessions are in memory).
  Serves the React app (`src/web/`, bundled by Bun via HTML import) and a JSON API:
  `GET /api/state`, `POST /api/setup|login|logout|password`, `GET|PUT /api/config`,
  `GET /api/status`.
- **Security checks on every API call:** `Host` must carry our port and be an IP address,
  `localhost`, this computer's name (`<hostname>`, `<hostname>.local`) or an `allowed_hosts`
  entry; never another domain name (DNS rebinding); changing requests need `X-Familynovice: 1` and, if
  present, a same-origin `Origin` (CSRF); session cookie `HttpOnly; SameSite=Strict`, 8 h,
  in memory (a server restart logs everyone out); request bodies ≤ 64 KB.
- **Auth (`admin/src/auth.ts`):** first visit sets the admin password (≥ 8 characters), **only
  from the computer itself** (loopback client address; otherwise someone in the network could
  claim a fresh installation; `/api/state` returns `setupAllowed`),
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
  remote.rs       remote read from its input device, calibration (remote.json)
  library.rs      read-only folder browsing (media filters, confinement to the folder)
  sites.rs        website rules (allowed pages, always-blocked addresses); also used by web/
  pictures.rs     picture decoding for the built-in viewer
  audio.rs        built-in music player (windowless mpv controlled over IPC)
  makemkv.rs      makemkvcon robot-output parser, main title selection
  player.rs       builds the player commands (mpv / VLC with libmmbd)
  resume.rs       resume positions via mpv IPC
  store.rs        JSON files in XDG dirs (disc cache, resume), atomic writes
scripts/
  setup.sh        installs players, builds MakeMKV from source (--no-makemkv: packages only)
  install.sh      kiosk install for the current account (cage session, admin service)
  uninstall.sh    undoes install.sh (--purge: also settings and data)
  remote-access.sh  udev rule: read access to the remote only (for src/remote.rs)
  config.rs       config.json: types, validation, load/reload (fail closed)
  control.rs      control socket for the admin interface (reload/status)
web/              familynovice-web: locked-down fullscreen browser window (WebKitGTK via wry),
                  workspace member; includes src/sites.rs via #[path]
ecosystem.config.js  pm2 config for the admin panel (CommonJS, so it lives outside admin/)
admin/            Bun + React admin web interface
  src/server.ts   entry point
  src/app.ts      HTTP server, API routes, security checks
  src/auth.ts     admin password (argon2id), sessions, login rate limit
  src/config.ts   config types + validation (mirror of src/config.rs)
  src/settings.ts port / listen address / allowed Host names (args, env, admin-server.json)
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
cargo build -p familynovice-web              # website window (needs libwebkit2gtk-4.1-dev, libdbus-1-dev)
cargo clippy --workspace --all-targets -- -D warnings   # lint both, once the WebKit dev files exist
```

`cargo build` alone builds only the media center, so it works without the WebKit dev files.

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
  (The only fetches besides the websites themselves: the website logos, by the admin
  interface, from the configured websites when saving.)
- Follow the XDG Base Directory spec for config, data and cache locations.
- Stay desktop-environment agnostic (see Deployment target).
- Add dependencies sparingly and prefer well-maintained crates and packages. Explain why a new
  dependency is needed when adding it.
- Keep user-facing strings out of deep logic so localization is possible later.

## Planned architecture

### Admin interface: user, permissions, authentication

- **v1:** the Bun admin server runs as a **systemd user service of the media center user**. It
  owns `$XDG_CONFIG_HOME/familynovice/` and needs no extra permissions. It listens on all
  interfaces by default, so parents reach it from their own account or from other devices.
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
  protection. Plain HTTP in the home network (passwords readable on the network; see open decisions).

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
- **Implemented (example, `scripts/install.sh` / `scripts/uninstall.sh`, apt + systemd):** for
  the current account or `--user NAME` (run from an admin account; per-user steps via
  `sudo -u` / `systemctl --user --machine=NAME@`). It builds and installs to `/opt/familynovice` (root-owned, own copy of
  `bun`), and adds `/usr/share/wayland-sessions/familynovice.desktop`, which runs
  `bin/familynovice-session`: `cage -s` (VT switching stays) plus the system keyboard layout
  from `/etc/default/keyboard`, logging via `systemd-cat -t familynovice`. Inside cage,
  `bin/familynovice-kiosk` restarts the media center after crashes (exit 0 = Ctrl+Q ends the
  session; 5 quick crashes end it too). The default session is set via AccountsService
  (`SetSession`/`SetXSession`/`SetSessionType`, read by GDM and LightDM). The admin panel is a
  systemd user service (`familynovice-admin.service`) with lingering. `install-state`
  (KEY=VALUE, read as data, never sourced) keeps the previous session and linger state for
  the uninstaller. No autologin changes (system settings do that).
- **Kiosk escapes closed so far:** mpv's console (`` ` `` / `:`, its `run` command starts
  programs) is unbound in `MPV_INPUT_CONF`. Open: VLC's Qt menus and WebKit file-upload
  dialogs (file browsing, nothing can be started); no volume control in the session.

## Decisions made

- **UI toolkit:** `egui` via `eframe` (wgpu renderer; runs on Wayland and X11). The home screen
  is painted by hand (`Painter`) for a 10-foot look, scaled relative to a 1080p layout.
- **Disc playback:** separate fullscreen player processes. mpv plays the main movie, VLC plays
  with disc menus. Decryption comes from MakeMKV's `libmmbd`. The media center waits for the
  player to exit and then shows the home screen again.
- **System packages (apt):** see `APT_PACKAGES` in `scripts/setup.sh`.
- **Website engine:** WebKitGTK via `wry`/`tao` in a separate process (`web/`), so the media
  center itself doesn't depend on GTK/WebKit. Blocking: see "Websites".
- **Admin ↔ media center:** JSON lines over a Unix socket (`src/control.rs`); config as one
  `config.json`, validated on both sides.

## Open decisions

Record each choice here once it is made:

- whether to offer HTTPS for the admin interface (reachable from the network by default)
- when to move to the hardened config setup (see "Admin interface: user, permissions, authentication")
- support for non-apt distributions in `scripts/setup.sh`
