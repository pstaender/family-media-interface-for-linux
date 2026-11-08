//! Short game sounds (retro beeps), made here as WAV files in our cache folder
//! (`$XDG_CACHE_HOME/curator/sounds/`) and played with the system's sound player:
//! `pw-play` (PipeWire), else `paplay` (PulseAudio), else `aplay` (ALSA). No sound library,
//! nothing bundled. Playing never blocks the game; a missing player just means silence.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const RATE: u32 = 22_050;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Wall,
    Paddle,
    Brick,
    HardBrick,
    Lost,
    Cleared,
    ExtraLife,
    GameOver,
}

impl Sound {
    const ALL: [Sound; 8] = [
        Sound::Wall,
        Sound::Paddle,
        Sound::Brick,
        Sound::HardBrick,
        Sound::Lost,
        Sound::Cleared,
        Sound::ExtraLife,
        Sound::GameOver,
    ];

    fn name(self) -> &'static str {
        match self {
            Sound::Wall => "wall",
            Sound::Paddle => "paddle",
            Sound::Brick => "brick",
            Sound::HardBrick => "hard-brick",
            Sound::Lost => "lost",
            Sound::Cleared => "cleared",
            Sound::ExtraLife => "extra-life",
            Sound::GameOver => "game-over",
        }
    }

    /// Notes: (frequency in Hz, milliseconds); 0 Hz = silence.
    fn notes(self) -> &'static [(f32, u32)] {
        match self {
            Sound::Wall => &[(392.0, 25)],
            Sound::Paddle => &[(523.0, 40)],
            Sound::Brick => &[(784.0, 35)],
            Sound::HardBrick => &[(1047.0, 45)],
            Sound::Lost => &[(392.0, 90), (311.0, 90), (247.0, 90), (196.0, 220)],
            Sound::Cleared => &[(523.0, 80), (659.0, 80), (784.0, 80), (1047.0, 200)],
            Sound::ExtraLife => &[
                (784.0, 60),
                (0.0, 20),
                (1047.0, 60),
                (0.0, 20),
                (1319.0, 120),
            ],
            Sound::GameOver => &[(330.0, 160), (262.0, 160), (196.0, 160), (131.0, 400)],
        }
    }
}

/// A 16-bit mono WAV file: soft square waves (short fade in / out, so nothing clicks).
pub fn wav(notes: &[(f32, u32)]) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    for &(freq, ms) in notes {
        let n = (RATE * ms / 1000) as usize;
        let fade = (RATE as usize / 400).min(n / 2).max(1);
        for i in 0..n {
            if freq <= 0.0 {
                samples.push(0);
                continue;
            }
            let phase = (i as f32 * freq / RATE as f32).fract();
            let square = if phase < 0.5 { 1.0 } else { -1.0 };
            let envelope = (i.min(n - 1 - i) as f32 / fade as f32).min(1.0);
            samples.push((square * envelope * 6000.0) as i16);
        }
    }
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

struct Player {
    program: &'static str,
    dir: PathBuf,
}

/// Write the sound files (once) and find a player; `None`: no sound possible.
fn player() -> Option<&'static Player> {
    static PLAYER: OnceLock<Option<Player>> = OnceLock::new();
    PLAYER
        .get_or_init(|| {
            let dir = crate::store::sounds_dir()?;
            std::fs::create_dir_all(&dir).ok()?;
            for sound in Sound::ALL {
                let path = dir.join(format!("{}.wav", sound.name()));
                let data = wav(sound.notes());
                if std::fs::read(&path).ok().as_deref() != Some(&data[..])
                    && let Err(e) = std::fs::write(&path, &data)
                {
                    log::warn!("sound: could not write {}: {e}", path.display());
                    return None;
                }
            }
            let program = ["pw-play", "paplay", "aplay"]
                .into_iter()
                .find(|p| in_path(p))?;
            log::info!("sound: playing with {program}");
            Some(Player { program, dir })
        })
        .as_ref()
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(program)))
    })
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Play a sound in the background.
pub fn play(sound: Sound) {
    let Some(player) = player() else {
        return;
    };
    let mut cmd = Command::new(player.program);
    if player.program == "aplay" {
        cmd.arg("-q");
    }
    cmd.arg(player.dir.join(format!("{}.wav", sound.name())))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd.spawn() {
        // Wait in a thread, so no finished player is left as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::debug!("sound: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_files_are_valid() {
        for sound in Sound::ALL {
            let data = wav(sound.notes());
            assert_eq!(&data[0..4], b"RIFF");
            assert_eq!(&data[8..16], b"WAVEfmt ");
            let data_len = u32::from_le_bytes(data[40..44].try_into().unwrap()) as usize;
            assert_eq!(data.len(), 44 + data_len);
            let samples: u32 = sound.notes().iter().map(|n| RATE * n.1 / 1000).sum();
            assert_eq!(data_len, samples as usize * 2, "{sound:?}");
            // Starts and ends quietly (fade), so playing it doesn't click.
            let first = i16::from_le_bytes([data[44], data[45]]);
            assert!(first.abs() < 100);
        }
    }
}
