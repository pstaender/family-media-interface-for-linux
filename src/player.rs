//! Launching the Blu-ray player.
//!
//! Decryption: MakeMKV's `libmmbd` stands in for `libaacs`/`libbdplus`. libbluray loads it when
//! `LIBAACS_PATH`/`LIBBDPLUS_PATH` are set to `libmmbd`, so no system library is replaced.
//! `libmmbd` itself talks to `makemkvcon`, so MakeMKV must be installed (see `scripts/setup.sh`).
//!
//! Players:
//! - **Main movie** → `mpv bd://mpls/<playlist>`: plays the main movie directly, no menus.
//! - **Disc menu** → `vlc bluray://<device>`: regular start with full disc menus (BD-J menus
//!   need Java and libbluray's BD-J support).

use std::path::Path;
use std::process::{Command, Stdio};

use crate::makemkv::DiscInfo;

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
            let mut cmd = Command::new("mpv");
            cmd.arg("--fullscreen")
                .arg("--no-terminal")
                .arg(format!("--bluray-device={}", device.display()));
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
            cmd.arg(target);
            cmd
        }
        PlayMode::DiscMenu => {
            let mut cmd = Command::new("vlc");
            cmd.args(["--fullscreen", "--play-and-exit", "--no-video-title-show"]);
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
        let socket = Path::new("/run/user/1000/familynovice-mpv.sock");
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
        assert!(a.contains(&"--input-ipc-server=/run/user/1000/familynovice-mpv.sock".to_string()));
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
