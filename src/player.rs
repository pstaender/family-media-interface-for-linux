//! Launching players: Blu-ray (mpv / VLC), video files (mpv) and music (mpv without window).
//!
//! Decryption: MakeMKV's `libmmbd` stands in for `libaacs`/`libbdplus`. libbluray loads it when
//! `LIBAACS_PATH`/`LIBBDPLUS_PATH` are set to `libmmbd`, so no system library is replaced.
//! `libmmbd` itself talks to `makemkvcon`, so MakeMKV must be installed (see `scripts/setup.sh`).
//!
//! Players:
//! - **Main movie** → `mpv bd://mpls/<playlist>`: plays the main movie directly, no menus.
//! - **Disc menu** → `vlc bluray://<device>`: regular start with full disc menus (BD-J menus
//!   need Java and libbluray's BD-J support).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::makemkv::DiscInfo;

/// Keep mpv from writing next to the media (screenshots, watch-later files): everything
/// goes into our cache dir. See "Non-destructive" in CLAUDE.md.
fn mpv_no_writes(cmd: &mut Command, cache_dir: Option<&Path>) {
    if let Some(dir) = cache_dir {
        cmd.arg(format!(
            "--screenshot-dir={}",
            dir.join("screenshots").display()
        ))
        .arg(format!(
            "--watch-later-dir={}",
            dir.join("watch_later").display()
        ));
    }
    cmd.arg("--no-save-position-on-quit");
}

/// Extra mpv key bindings (written into our cache dir by `store::prepare_player_cache`):
/// Esc, Backspace and the back keys of remotes, gamepads and mice close the video, and so
/// does a right click (the "return" button of air-mouse remotes). The
/// position is kept: the resume monitor saves it when mpv closes. mpv's defaults stay active,
/// except the console (` and :), whose `run` command could start any program (kiosk escape).
/// The remote's next/previous keys jump between chapters (as via MPRIS, see `mpv_media_key`).
/// The volume keys change the system volume (`volume`), not mpv's own, so they're ignored here.
pub const MPV_INPUT_CONF: &str = "\
ESC quit
BS quit
BACK quit
GAMEPAD_BACK quit
MBTN_BACK quit
MBTN_RIGHT quit
` ignore
: ignore
NEXT add chapter 1
PREV add chapter -1
VOLUME_UP ignore
VOLUME_DOWN ignore
MUTE ignore
";
pub const MPV_INPUT_CONF_NAME: &str = "mpv-input.conf";

/// Exit code of a player / the website window when the remote's home button was held for
/// 3 s: the media center then shows the home screen (instead of the folder the video came
/// from). Same value in `web/src/main.rs`.
pub const EXIT_GO_HOME: i32 = 4;

/// mpv script (written next to `MPV_INPUT_CONF`): holding the home button (`HOMEPAGE`,
/// XF86HomePage) for 3 s quits with `EXIT_GO_HOME`. Plain input.conf bindings can't tell a
/// held key from a press.
pub const MPV_HOME_SCRIPT: &str = r#"-- curator: hold the remote's home button for 3 s to go to the home screen.
local timer = nil
mp.add_forced_key_binding("HOMEPAGE", "curator-home", function(e)
    if e.event == "down" then
        if timer then timer:kill() end
        mp.osd_message("{hold}", 3)
        timer = mp.add_timeout(3, function() mp.commandv("quit", "4") end)
    elseif e.event == "up" then
        if timer then timer:kill() end
        timer = nil
        mp.osd_message("")
    end
end, { complex = true })
"#;
pub const MPV_HOME_SCRIPT_NAME: &str = "curator-home.lua";

/// `MPV_HOME_SCRIPT` with the hint in the interface language.
pub fn mpv_home_script() -> String {
    let hold = crate::tr!("Keep holding to go to the home screen…")
        .replace(['\\', '"'], "")
        .replace('\n', " ");
    MPV_HOME_SCRIPT.replace("{hold}", &hold)
}

/// Forward a media key to the running video (mpv's IPC socket, see `resume`): play/pause,
/// next/previous chapter. Does nothing if mpv isn't listening (VLC, already closed).
pub fn mpv_media_key(socket: &Path, key: crate::mpris::MediaCommand) {
    use crate::mpris::MediaCommand;
    let command = match key {
        MediaCommand::PlayPause => serde_json::json!(["cycle", "pause"]),
        MediaCommand::Play => serde_json::json!(["set", "pause", "no"]),
        MediaCommand::Pause => serde_json::json!(["set", "pause", "yes"]),
        MediaCommand::Next => serde_json::json!(["add", "chapter", 1]),
        MediaCommand::Previous => serde_json::json!(["add", "chapter", -1]),
        MediaCommand::Stop => return, // handled by the "Playing…" screen
    };
    mpv_send(socket, &command);
}

/// Show a short text in the running video (e.g. the new volume).
pub fn mpv_show_text(socket: &Path, text: &str) {
    mpv_send(socket, &serde_json::json!(["show-text", text, 2000]));
}

/// Send one command to mpv's IPC socket; does nothing if mpv isn't listening.
fn mpv_send(socket: &Path, command: &serde_json::Value) {
    use std::io::Write;
    let result = std::os::unix::net::UnixStream::connect(socket).and_then(|mut stream| {
        stream.set_write_timeout(Some(std::time::Duration::from_millis(500)))?;
        writeln!(stream, "{}", serde_json::json!({ "command": command }))
    });
    if let Err(e) = result {
        log::debug!("command for mpv: {e}");
    }
}

/// mpv for a movie (disc main movie or video file): fullscreen, languages, resume position,
/// IPC socket for position tracking, nothing written next to the media.
fn mpv_movie(movie: &MovieOptions) -> Command {
    let mut cmd = Command::new("mpv");
    cmd.arg("--fullscreen")
        .arg("--no-terminal")
        // GPU first; then SDL/X11. The old Xv/VDPAU outputs are skipped: without a usable GPU
        // (e.g. remote desktop) mpv hung there with a tiny, invisible window.
        .arg("--vo=gpu-next,gpu,sdl,x11");
    mpv_no_writes(&mut cmd, movie.cache_dir);
    if let Some(dir) = movie.cache_dir {
        // Esc / Backspace / remote "back" close the video (see MPV_INPUT_CONF).
        cmd.arg(format!(
            "--input-conf={}",
            dir.join(MPV_INPUT_CONF_NAME).display()
        ))
        .arg(format!(
            "--script={}",
            dir.join(MPV_HOME_SCRIPT_NAME).display()
        ));
    }
    if let Some(start) = movie.start_secs {
        cmd.arg(format!("--start={start:.1}"));
    }
    if let Some(socket) = movie.ipc_socket {
        cmd.arg(format!("--input-ipc-server={}", socket.display()));
    }
    if !movie.audio_languages.is_empty() {
        cmd.arg(format!("--alang={}", language_list(movie.audio_languages)));
    }
    if !movie.subtitle_languages.is_empty() {
        cmd.arg(format!(
            "--slang={}",
            language_list(movie.subtitle_languages)
        ));
    }
    cmd
}

/// A video file from a folder, played like the disc's main movie (same mpv setup).
pub fn video_file_command(file: &Path, movie: &MovieOptions) -> Command {
    let mut cmd = mpv_movie(movie);
    // "--" so that no file name can ever be read as an option.
    cmd.arg("--").arg(file);
    cmd.stdin(Stdio::null()).stdout(Stdio::null());
    cmd
}

/// Music from a folder: mpv without any window, controlled by curator over its IPC
/// socket (curator shows the player screen). Plays `files` from `start` to the end.
pub fn audio_command(
    files: &[PathBuf],
    start: usize,
    ipc_socket: &Path,
    cache_dir: Option<&Path>,
) -> Option<Command> {
    let rest = files.get(start..).filter(|r| !r.is_empty())?;
    let mut cmd = Command::new("mpv");
    cmd.args([
        "--no-video",
        "--force-window=no",
        "--audio-display=no",
        "--no-terminal",
        "--idle=no",
    ])
    .arg(format!("--input-ipc-server={}", ipc_socket.display()));
    mpv_no_writes(&mut cmd, cache_dir);
    cmd.arg("--").args(rest);
    cmd.stdin(Stdio::null()).stdout(Stdio::null());
    Some(cmd)
}

/// Key bindings of the screensaver's mpv (`screensaver_command`, which turns mpv's own
/// bindings off): every key, button and click ends it. Buttons only the remote's input device
/// reports (e.g. OK = KEY_SELECT) end it through the media center.
pub const MPV_SAVER_INPUT_CONF: &str = "\
ANY_UNICODE quit
ESC quit
ENTER quit
KP_ENTER quit
KP0 quit
KP1 quit
KP2 quit
KP3 quit
KP4 quit
KP5 quit
KP6 quit
KP7 quit
KP8 quit
KP9 quit
KP_UP quit
KP_DOWN quit
KP_LEFT quit
KP_RIGHT quit
SPACE quit
BS quit
TAB quit
UP quit
DOWN quit
LEFT quit
RIGHT quit
PGUP quit
PGDWN quit
HOME quit
END quit
INS quit
DEL quit
MENU quit
BACK quit
FORWARD quit
HOMEPAGE quit
SEARCH quit
PLAY quit
PAUSE quit
PLAYPAUSE quit
PLAYONLY quit
PAUSEONLY quit
STOP quit
NEXT quit
PREV quit
REWIND quit
VOLUME_UP quit
VOLUME_DOWN quit
MUTE quit
CHANNEL_UP quit
CHANNEL_DOWN quit
F1 quit
F2 quit
F3 quit
F4 quit
F5 quit
F6 quit
F7 quit
F8 quit
F9 quit
F10 quit
F11 quit
F12 quit
MBTN_LEFT quit
MBTN_RIGHT quit
MBTN_MID quit
MBTN_BACK quit
MBTN_FORWARD quit
WHEEL_UP quit
WHEEL_DOWN quit
GAMEPAD_ACTION_DOWN quit
GAMEPAD_ACTION_RIGHT quit
GAMEPAD_ACTION_LEFT quit
GAMEPAD_ACTION_UP quit
GAMEPAD_MENU quit
GAMEPAD_BACK quit
GAMEPAD_START quit
GAMEPAD_DPAD_UP quit
GAMEPAD_DPAD_DOWN quit
GAMEPAD_DPAD_LEFT quit
GAMEPAD_DPAD_RIGHT quit
CLOSE_WIN quit
";
pub const MPV_SAVER_INPUT_CONF_NAME: &str = "mpv-screensaver-input.conf";

/// The aerial screensaver: the downloaded videos without sound, in random order, looping, no
/// on-screen display, nothing written anywhere (see `screensaver`).
pub fn screensaver_command(files: &[PathBuf], cache_dir: Option<&Path>) -> Option<Command> {
    if files.is_empty() {
        return None;
    }
    let mut cmd = Command::new("mpv");
    cmd.args([
        "--fullscreen",
        "--no-terminal",
        "--vo=gpu-next,gpu,sdl,x11",
        "--hwdec=auto-safe",
        "--no-audio",
        "--shuffle",
        "--loop-playlist=inf",
        "--osd-level=0",
        "--osc=no",
        "--cursor-autohide=always",
        "--no-input-default-bindings",
        "--no-resume-playback",
        "--title=curator screensaver",
    ]);
    mpv_no_writes(&mut cmd, cache_dir);
    if let Some(dir) = cache_dir {
        cmd.arg(format!(
            "--input-conf={}",
            dir.join(MPV_SAVER_INPUT_CONF_NAME).display()
        ));
    }
    cmd.arg("--").args(files);
    cmd.stdin(Stdio::null()).stdout(Stdio::null());
    Some(cmd)
}

/// End `makemkvcon guiserver` processes that outlived the player that started them.
///
/// libmmbd starts `makemkvcon guiserver` for each player; it puts itself into its own
/// process group and keeps running after the player exits (orphaned: its parent becomes
/// init or the user's service manager). Each one keeps the drive open, so later disc scans
/// hang and ejecting fails. Only our own user's orphaned guiservers are ended; a guiserver
/// whose player still runs has that player as its parent and is left alone.
pub fn stop_leftover_makemkv_servers() {
    // SAFETY: getuid() can't fail and has no side effects.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        let dir = entry.path();
        let (Ok(status), Ok(cmdline)) = (
            std::fs::read_to_string(dir.join("status")),
            std::fs::read(dir.join("cmdline")),
        ) else {
            continue;
        };
        let parent_comm = status_field(&status, "PPid")
            .and_then(|p| std::fs::read_to_string(format!("/proc/{p}/comm")).ok());
        if is_leftover_makemkv_server(&status, &cmdline, uid, parent_comm.as_deref()) {
            log::info!("ending leftover makemkvcon guiserver (pid {pid})");
            // SAFETY: plain kill(2) on a process of our own user.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

fn status_field<'a>(status: &'a str, name: &str) -> Option<&'a str> {
    status
        .lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
}

/// `/proc/<pid>/status` + `cmdline` of a process: our user's `makemkvcon guiserver` whose
/// parent is gone (now pid 1 or a `systemd` service manager).
fn is_leftover_makemkv_server(
    status: &str,
    cmdline: &[u8],
    uid: u32,
    parent_comm: Option<&str>,
) -> bool {
    let ours = status_field(status, "Uid")
        .and_then(|u| u.split_whitespace().next())
        .and_then(|u| u.parse::<u32>().ok())
        == Some(uid);
    let mut args = cmdline.split(|b| *b == 0);
    let program = args.next().unwrap_or_default();
    let is_server = program.rsplit(|b| *b == b'/').next() == Some(b"makemkvcon".as_slice())
        && args.next() == Some(b"guiserver".as_slice());
    let orphaned = status_field(status, "PPid") == Some("1")
        || parent_comm.is_some_and(|c| c.trim() == "systemd");
    ours && is_server && orphaned
}

/// ISO 639-2 has two codes for some languages (terminology / bibliographic), and discs use
/// both. The admin interface stores the terminology code; players get both variants.
const ALTERNATIVE_CODES: [(&str, &str); 19] = [
    ("deu", "ger"),
    ("fra", "fre"),
    ("nld", "dut"),
    ("ces", "cze"),
    ("ell", "gre"),
    ("zho", "chi"),
    ("ron", "rum"),
    ("slk", "slo"),
    ("sqi", "alb"),
    ("hye", "arm"),
    ("eus", "baq"),
    ("mya", "bur"),
    ("kat", "geo"),
    ("isl", "ice"),
    ("mkd", "mac"),
    ("msa", "may"),
    ("fas", "per"),
    ("cym", "wel"),
    ("bod", "tib"),
];

/// `["deu", "eng"]` → `"deu,ger,eng"`.
fn language_list(languages: &[String]) -> String {
    let mut out: Vec<&str> = Vec::new();
    for lang in languages {
        out.push(lang);
        let alt = ALTERNATIVE_CODES
            .iter()
            .find_map(|&(t, b)| (lang == t).then_some(b).or((lang == b).then_some(t)));
        if let Some(alt) = alt
            && !languages.iter().any(|l| l == alt)
        {
            out.push(alt);
        }
    }
    out.join(",")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    /// (b) Play the main movie directly.
    MainMovie,
    /// (c) Start the disc regularly, with its menus.
    DiscMenu,
}

/// Options for starting playback.
#[derive(Debug, Clone, Default)]
pub struct MovieOptions<'a> {
    /// Start position in seconds (resume; main movie only).
    pub start_secs: Option<f64>,
    /// mpv JSON IPC socket, used to track the playback position (main movie only).
    pub ipc_socket: Option<&'a Path>,
    /// Preferred audio languages (ISO 639-2), most preferred first.
    pub audio_languages: &'a [String],
    /// Preferred subtitle languages; empty means no subtitles.
    pub subtitle_languages: &'a [String],
    /// Our cache dir: players put screenshots etc. there instead of next to the media.
    pub cache_dir: Option<&'a Path>,
}

/// Build the player command for a mode. `info` comes from the MakeMKV scan; without it,
/// the main movie falls back to mpv's "longest title" choice.
pub fn command(
    mode: PlayMode,
    device: &Path,
    info: Option<&DiscInfo>,
    movie: &MovieOptions,
) -> Command {
    let mut cmd = match mode {
        PlayMode::MainMovie => {
            let target = info
                .and_then(DiscInfo::main_title)
                .and_then(|t| t.playlist())
                .map(|n| format!("bd://mpls/{n}"))
                .unwrap_or_else(|| "bd://longest".into());
            let mut cmd = mpv_movie(movie);
            cmd.arg(format!("--bluray-device={}", device.display()))
                .arg(target);
            cmd
        }
        PlayMode::DiscMenu => {
            let mut cmd = Command::new("vlc");
            cmd.args([
                "--fullscreen",
                "--play-and-exit",
                "--no-video-title-show",
                // No first-run "privacy and network access" dialog.
                "--no-qt-privacy-ask",
                // Deinterlacing always on (not only for streams flagged as interlaced):
                // many discs' menus and extras are interlaced. yadif: good quality, cheap.
                "--deinterlace=1",
                "--deinterlace-mode=yadif",
            ])
            // Esc closes the player (VLC allows one key per action). The volume keys change
            // the system volume (`volume`), so VLC's own volume keeps only its Ctrl/m keys.
            .args([
                "--key-quit=Esc",
                "--key-leave-fullscreen=Ctrl+Alt+Shift+F12",
                "--key-vol-up=Ctrl+Up",
                "--key-vol-down=Ctrl+Down",
                "--key-vol-mute=m",
            ])
            // Decode the audio instead of passing Dolby/DTS through to the TV or receiver:
            // on HDMI that failed ("digital pass-through stream connection failure"), and
            // the system volume only works on decoded sound.
            .arg("--no-spdif");
            if let Some(dir) = movie.cache_dir {
                cmd.arg(format!(
                    "--snapshot-path={}",
                    dir.join("screenshots").display()
                ));
            }
            if !movie.audio_languages.is_empty() {
                cmd.arg(format!(
                    "--audio-language={}",
                    language_list(movie.audio_languages)
                ));
            }
            if !movie.subtitle_languages.is_empty() {
                cmd.arg(format!(
                    "--sub-language={}",
                    language_list(movie.subtitle_languages)
                ));
            }
            cmd.arg(format!("bluray://{}", device.display()));
            cmd
        }
    };
    cmd.env("LIBAACS_PATH", "libmmbd")
        .env("LIBBDPLUS_PATH", "libmmbd")
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::makemkv::parse_robot_output;

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn home_script_quits_with_the_go_home_code() {
        let quit = format!(r#"mp.commandv("quit", "{EXIT_GO_HOME}")"#);
        assert!(MPV_HOME_SCRIPT.contains(&quit));
        assert!(MPV_HOME_SCRIPT.contains(r#""HOMEPAGE""#));
        let a = args(&video_file_command(
            Path::new("/m/a.mkv"),
            &MovieOptions {
                cache_dir: Some(Path::new("/c")),
                ..Default::default()
            },
        ));
        assert!(a.contains(&"--script=/c/curator-home.lua".to_string()));
    }

    #[test]
    fn mpv_console_is_disabled() {
        // The console's `run` command could start any program (kiosk escape).
        for key in ["`", ":"] {
            assert!(MPV_INPUT_CONF.lines().any(|l| l == format!("{key} ignore")));
        }
    }

    #[test]
    fn main_movie_uses_detected_playlist() {
        let info = parse_robot_output("TINFO:0,9,0,\"1:40:00\"\nTINFO:0,16,0,\"00800.mpls\"\n");
        let cmd = command(
            PlayMode::MainMovie,
            Path::new("/dev/sr0"),
            Some(&info),
            &MovieOptions::default(),
        );
        assert_eq!(cmd.get_program(), "mpv");
        let a = args(&cmd);
        assert!(a.contains(&"--bluray-device=/dev/sr0".to_string()));
        assert_eq!(a.last().unwrap(), "bd://mpls/800");
    }

    #[test]
    fn main_movie_without_scan_falls_back_to_longest() {
        let cmd = command(
            PlayMode::MainMovie,
            Path::new("/dev/sr0"),
            None,
            &MovieOptions::default(),
        );
        assert_eq!(args(&cmd).last().unwrap(), "bd://longest");
    }

    #[test]
    fn main_movie_resume_and_ipc() {
        let socket = Path::new("/run/user/1000/curator-mpv.sock");
        let cmd = command(
            PlayMode::MainMovie,
            Path::new("/dev/sr0"),
            None,
            &MovieOptions {
                start_secs: Some(2533.0),
                ipc_socket: Some(socket),
                ..Default::default()
            },
        );
        let a = args(&cmd);
        assert!(a.contains(&"--start=2533.0".to_string()));
        assert!(a.contains(&"--input-ipc-server=/run/user/1000/curator-mpv.sock".to_string()));
        assert_eq!(a.last().unwrap(), "bd://longest");
    }

    #[test]
    fn languages_are_passed_to_both_players() {
        let audio = vec!["deu".to_string(), "eng".to_string()];
        let subs = vec!["deu".to_string()];
        let options = MovieOptions {
            audio_languages: &audio,
            subtitle_languages: &subs,
            ..Default::default()
        };
        let mpv = args(&command(
            PlayMode::MainMovie,
            Path::new("/dev/sr0"),
            None,
            &options,
        ));
        assert!(mpv.contains(&"--alang=deu,ger,eng".to_string()));
        assert!(mpv.contains(&"--slang=deu,ger".to_string()));
        let vlc = args(&command(
            PlayMode::DiscMenu,
            Path::new("/dev/sr0"),
            None,
            &options,
        ));
        assert!(vlc.contains(&"--audio-language=deu,ger,eng".to_string()));
        assert!(vlc.contains(&"--deinterlace=1".to_string()));
        assert!(vlc.contains(&"--deinterlace-mode=yadif".to_string()));
        assert!(vlc.contains(&"--sub-language=deu,ger".to_string()));
        assert_eq!(vlc.last().unwrap(), "bluray:///dev/sr0");
        // No preference: no language options at all.
        let plain = args(&command(
            PlayMode::MainMovie,
            Path::new("/dev/sr0"),
            None,
            &MovieOptions::default(),
        ));
        assert!(
            !plain
                .iter()
                .any(|a| a.starts_with("--alang") || a.starts_with("--slang"))
        );
    }

    #[test]
    fn language_lists_include_alternative_codes() {
        let l = |v: &[&str]| language_list(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(l(&["deu", "eng"]), "deu,ger,eng");
        assert_eq!(l(&["fre"]), "fre,fra");
        assert_eq!(l(&["deu", "ger"]), "deu,ger"); // no duplicates
        assert_eq!(l(&[]), "");
    }

    #[test]
    fn players_write_only_into_our_cache() {
        let cache = Path::new("/home/u/.cache/curator/player");
        let options = MovieOptions {
            cache_dir: Some(cache),
            ..Default::default()
        };
        let mpv = args(&command(
            PlayMode::MainMovie,
            Path::new("/dev/sr0"),
            None,
            &options,
        ));
        assert!(mpv.contains(&format!("--screenshot-dir={}/screenshots", cache.display())));
        assert!(mpv.contains(&"--no-save-position-on-quit".to_string()));
        let vlc = args(&command(
            PlayMode::DiscMenu,
            Path::new("/dev/sr0"),
            None,
            &options,
        ));
        assert!(vlc.contains(&format!("--snapshot-path={}/screenshots", cache.display())));
    }

    #[test]
    fn video_files_use_the_movie_setup() {
        let langs = vec!["deu".to_string()];
        let options = MovieOptions {
            start_secs: Some(90.0),
            ipc_socket: Some(Path::new("/run/s.sock")),
            audio_languages: &langs,
            cache_dir: Some(Path::new("/c")),
            ..Default::default()
        };
        let a = args(&video_file_command(Path::new("/m/-weird.mkv"), &options));
        for expected in [
            "--input-conf=/c/mpv-input.conf",
            "--fullscreen",
            "--start=90.0",
            "--input-ipc-server=/run/s.sock",
            "--alang=deu,ger",
            "--screenshot-dir=/c/screenshots",
        ] {
            assert!(a.contains(&expected.to_string()), "{expected}");
        }
        assert_eq!(&a[a.len() - 2..], ["--", "/m/-weird.mkv"]);
    }

    #[test]
    fn music_plays_without_window_from_the_chosen_track() {
        let files: Vec<PathBuf> = ["/m/1.mp3", "/m/2.mp3", "/m/3.mp3"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let cmd = audio_command(&files, 1, Path::new("/run/a.sock"), None).unwrap();
        let a = args(&cmd);
        assert!(a.contains(&"--no-video".to_string()));
        assert!(a.contains(&"--force-window=no".to_string()));
        assert!(a.contains(&"--input-ipc-server=/run/a.sock".to_string()));
        let dashdash = a.iter().position(|x| x == "--").unwrap();
        assert_eq!(&a[dashdash + 1..], ["/m/2.mp3", "/m/3.mp3"]);
        assert!(audio_command(&files, 3, Path::new("/run/a.sock"), None).is_none());
    }

    #[test]
    fn finds_leftover_makemkv_servers_only() {
        let status = |uid: u32, ppid: u32| {
            format!("Name:\tmakemkvcon\nPPid:\t{ppid}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n")
        };
        let server = b"/usr/local/bin/makemkvcon\0guiserver\0A0001+std\0".as_slice();
        assert!(is_leftover_makemkv_server(
            &status(1003, 1),
            server,
            1003,
            Some("systemd\n")
        ));
        // Reparented to the user's service manager.
        assert!(is_leftover_makemkv_server(
            &status(1003, 1669),
            server,
            1003,
            Some("systemd\n")
        ));
        // Its player still runs.
        assert!(!is_leftover_makemkv_server(
            &status(1003, 13984),
            server,
            1003,
            Some("vlc\n")
        ));
        // Another user's.
        assert!(!is_leftover_makemkv_server(
            &status(1000, 1),
            server,
            1003,
            None
        ));
        // Not a guiserver (e.g. our own disc scan).
        let scan = b"makemkvcon\0-r\0info\0dev:/dev/sr0\0".as_slice();
        assert!(!is_leftover_makemkv_server(
            &status(1003, 1),
            scan,
            1003,
            None
        ));
        assert!(!is_leftover_makemkv_server(
            &status(1003, 1),
            b"/tmp/makemkvcon-evil\0guiserver\0",
            1003,
            None
        ));
    }

    #[test]
    fn screensaver_mpv() {
        assert!(screensaver_command(&[], None).is_none());
        let files = [PathBuf::from("/c/aerial/A.1080p_h264.mov")];
        let a = args(&screensaver_command(&files, Some(Path::new("/c"))).unwrap());
        for arg in [
            "--no-audio",
            "--shuffle",
            "--loop-playlist=inf",
            "--no-input-default-bindings",
            "--input-conf=/c/mpv-screensaver-input.conf",
            "--screenshot-dir=/c/screenshots",
        ] {
            assert!(a.contains(&arg.to_string()), "{arg}");
        }
        assert_eq!(&a[a.len() - 2..], ["--", "/c/aerial/A.1080p_h264.mov"]);
        // Every common key, button and click ends it; nothing else is bound.
        for key in [
            "ANY_UNICODE",
            "ENTER",
            "ESC",
            "UP",
            "MBTN_LEFT",
            "MBTN_RIGHT",
            "HOMEPAGE",
        ] {
            assert!(
                MPV_SAVER_INPUT_CONF
                    .lines()
                    .any(|l| l == format!("{key} quit"))
            );
        }
        assert!(MPV_SAVER_INPUT_CONF.lines().all(|l| l.ends_with(" quit")));
    }

    #[test]
    fn players_leave_the_volume_keys_to_the_system_volume() {
        for key in ["VOLUME_UP", "VOLUME_DOWN", "MUTE"] {
            assert!(MPV_INPUT_CONF.lines().any(|l| l == format!("{key} ignore")));
        }
        let vlc = args(&command(
            PlayMode::DiscMenu,
            Path::new("/dev/sr0"),
            None,
            &MovieOptions::default(),
        ));
        for arg in [
            "--key-vol-up=Ctrl+Up",
            "--key-vol-down=Ctrl+Down",
            "--key-vol-mute=m",
        ] {
            assert!(vlc.contains(&arg.to_string()), "{arg}");
        }
    }

    #[test]
    fn disc_menu_uses_vlc_with_libmmbd() {
        let cmd = command(
            PlayMode::DiscMenu,
            Path::new("/dev/sr1"),
            None,
            &MovieOptions::default(),
        );
        assert_eq!(cmd.get_program(), "vlc");
        assert_eq!(args(&cmd).last().unwrap(), "bluray:///dev/sr1");
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(envs.contains(&("LIBAACS_PATH".as_ref(), Some("libmmbd".as_ref()))));
    }
}
