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
- The config schema is the contract between the two parts. Document it in one place (e.g. JSON
  Schema in `docs/`), version it, and update both sides together when it changes.

> Status: early stage. The codebase does not exist yet. Update this file as the architecture,
> dependencies and commands get decided.

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
- Expected libraries: `libdvdread`/`libdvdnav` + `libdvdcss` for DVD, and `libbluray` +
  `libaacs` (+ `libbdplus`) for Blu-ray. These are often used through the playback backend
  (e.g. mpv or GStreamer) rather than called directly.
- **Do not bundle or ship decryption keys, `KEYDB.cfg` or the decryption libraries in this
  repository.** Load them at runtime from the system (installed by the user or their distro).
  If they are missing, detect it and show a clear, friendly message instead of failing.
  The legality of circumventing copy protection differs between countries, so leave that
  decision to the user's installation.
- Detect inserted and ejected discs (e.g. udev/UDisks2 over D-Bus) and offer playback
  automatically. Support disc menus where the backend allows it, and otherwise start the
  main title.
- Parental controls apply to discs too: respect DVD/Blu-ray parental levels where they exist,
  and let profiles block or require approval for disc playback.

### Allowed websites (domain allowlist with wildcards)

- The admin sets up which websites each profile may visit. Entries are domains with optional
  wildcards, e.g. `example.org`, `*.example.org`.
- Default deny: anything not on the allowlist is blocked, including redirects, popups, new
  windows and embedded frames to other domains.
- Matching rules must be exact and tested. Specify (and test) whether `*.example.org` also
  matches the apex `example.org`. Compare against the normalized host (lowercase,
  punycode/IDN, trailing dot removed), never against a substring of the URL, so
  `example.org.evil.com` or `evil.com/?example.org` don't match.
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

## Repository layout (proposed)

Adjust once decided:

```
/              Cargo workspace root (Rust media center)
  src/ or crates/   Rust code (UI, playback, policy/parental logic, launcher, config)
admin/         Bun + React admin web interface
docs/          config JSON schema and documentation
```

## Development commands

Rust media center:

```sh
cargo build                                  # debug build
cargo build --release                        # release build
cargo run                                    # run the media center
cargo test                                   # run all tests
cargo test <name>                            # run tests matching a name
cargo fmt                                    # format code
cargo clippy --all-targets -- -D warnings    # lint (treat warnings as errors)
```

Admin web interface (from `admin/`):

```sh
bun install          # install dependencies
bun run dev          # start the dev server
bun run build        # production build
bun test             # run tests
```

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
- No telemetry and no unneeded network calls. Usage data and config stay on the machine.
- Follow the XDG Base Directory spec for config, data and cache locations.
- Stay desktop-environment agnostic (see Deployment target).
- Add dependencies sparingly and prefer well-maintained crates and packages. Explain why a new
  dependency is needed when adding it.
- Keep user-facing strings out of deep logic so localization is possible later.

## Open decisions

Record each choice here once it is made:

- UI toolkit / rendering approach for the native Rust UI
- playback backend (mpv/libmpv, GStreamer, …)
- embedded browser engine for the website allowlist (e.g. WebKitGTK, CEF) and how blocking is enforced
- how the admin interface is served (separate Bun service vs. started by the Rust app), which
  user it runs as, how it gets write access to the media center user's config directory,
  and its authentication
- apply mechanism: file watching vs. signal vs. local IPC (e.g. Unix socket / D-Bus) for reload,
  and how a restart is triggered (e.g. systemd user service)
- how the dedicated user account is set up (autostart, preventing access to the desktop)
- system packages needed to build and run
