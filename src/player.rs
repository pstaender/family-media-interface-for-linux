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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    /// (b) Play the main movie directly.
    MainMovie,
    /// (c) Start the disc regularly, with its menus.
    DiscMenu,
}

/// Options for the main movie (mpv only).
#[derive(Debug, Clone, Default)]
pub struct MovieOptions<'a> {
    /// Start position in seconds (resume).
    pub start_secs: Option<f64>,
    /// mpv JSON IPC socket, used to track the playback position.
    pub ipc_socket: Option<&'a Path>,
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
            cmd.arg(target);
            cmd
        }
        PlayMode::DiscMenu => {
            let mut cmd = Command::new("vlc");
            cmd.args(["--fullscreen", "--play-and-exit", "--no-video-title-show"])
                .arg(format!("bluray://{}", device.display()));
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
            },
        );
        let a = args(&cmd);
        assert!(a.contains(&"--start=2533.0".to_string()));
        assert!(a.contains(&"--input-ipc-server=/run/user/1000/familynovice-mpv.sock".to_string()));
        assert_eq!(a.last().unwrap(), "bd://longest");
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
