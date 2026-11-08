//! Game music: our own 80s-style tunes as MIDI files (`assets/music/*.mid`, built into the
//! program). Linux can't play MIDI without a synthesizer and a sound bank, so the tunes are
//! turned into WAV files here, once, with a small chip-style synth (square and triangle waves,
//! noise drums) in our cache folder (`$XDG_CACHE_HOME/curator/music/`), and looped by mpv
//! without a window.
//!
//! MIDI channels: 0 lead, 1 arpeggio, 2 bass, 9 drums (36 kick, 38 snare, others hi-hat).

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};

const RATE: u32 = 22_050;
/// Part of the file names: change it when the synth's sound changes.
const RENDER_VERSION: u8 = 1;
/// Loudest sample (the game's sounds stay audible on top).
const PEAK: f32 = 0.55;

const TUNES: [(&str, &[u8]); 3] = [
    ("neon-run", include_bytes!("../assets/music/neon-run.mid")),
    (
        "night-drive",
        include_bytes!("../assets/music/night-drive.mid"),
    ),
    (
        "pixel-sunrise",
        include_bytes!("../assets/music/pixel-sunrise.mid"),
    ),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub key: u8,
    pub channel: u8,
    pub velocity: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Song {
    pub notes: Vec<Note>,
    /// Seconds, up to the end of the longest track (so a loop keeps the beat).
    pub length: f64,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.data.len());
        let end = end.ok_or("unexpected end of the MIDI file")?;
        let part = &self.data[self.at..end];
        self.at = end;
        Ok(part)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A MIDI variable-length number (at most 4 bytes).
    fn vlq(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let b = self.byte()?;
            value = (value << 7) | u32::from(b & 0x7f);
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err("invalid number in the MIDI file".into())
    }

    fn done(&self) -> bool {
        self.at >= self.data.len()
    }
}

/// Note on / off of one track, in ticks; and tempo changes, and the track's last tick.
struct Track {
    events: Vec<(u64, u8, u8, u8)>, // tick, channel, key, velocity (0 = off)
    tempos: Vec<(u64, u32)>,
    end: u64,
}

fn parse_track(data: &[u8]) -> Result<Track, String> {
    let mut r = Reader { data, at: 0 };
    let mut track = Track {
        events: Vec::new(),
        tempos: Vec::new(),
        end: 0,
    };
    let (mut tick, mut running) = (0u64, 0u8);
    while !r.done() {
        tick += u64::from(r.vlq()?);
        track.end = tick;
        let mut status = r.byte()?;
        let first = if status < 0x80 {
            // Running status: this byte was already the first data byte.
            let data = status;
            status = running;
            if status < 0x80 {
                return Err("MIDI data without a status".into());
            }
            Some(data)
        } else {
            None
        };
        match status {
            0xff => {
                let kind = r.byte()?;
                let len = r.vlq()? as usize;
                let payload = r.take(len)?;
                if kind == 0x51 && payload.len() == 3 {
                    let us = u32::from_be_bytes([0, payload[0], payload[1], payload[2]]);
                    track.tempos.push((tick, us.max(1)));
                }
            }
            0xf0 | 0xf7 => {
                let len = r.vlq()? as usize;
                r.take(len)?;
            }
            0x80..=0xef => {
                running = status;
                let a = match first {
                    Some(a) => a,
                    None => r.byte()?,
                };
                let kind = status & 0xf0;
                let b = if matches!(kind, 0xc0 | 0xd0) {
                    0
                } else {
                    r.byte()?
                };
                let channel = status & 0x0f;
                match kind {
                    0x90 => track.events.push((tick, channel, a, b)),
                    0x80 => track.events.push((tick, channel, a, 0)),
                    _ => {}
                }
            }
            _ => return Err("unsupported MIDI event".into()),
        }
    }
    Ok(track)
}

/// Read a Standard MIDI File (format 0 or 1, ticks per quarter note).
pub fn parse(data: &[u8]) -> Result<Song, String> {
    let mut r = Reader { data, at: 0 };
    if r.take(4)? != b"MThd" {
        return Err("not a MIDI file".into());
    }
    let header = r.u32()? as usize;
    let format = r.u16()?;
    let count = r.u16()?;
    let division = r.u16()?;
    r.take(header.saturating_sub(6))?;
    if format > 1 || division == 0 || division & 0x8000 != 0 {
        return Err("unsupported MIDI format".into());
    }
    let mut tracks = Vec::new();
    for _ in 0..count {
        let id = r.take(4)?;
        let len = r.u32()? as usize;
        let body = r.take(len)?;
        if id == b"MTrk" {
            tracks.push(parse_track(body)?);
        }
    }
    let mut tempos: Vec<(u64, u32)> = tracks.iter().flat_map(|t| t.tempos.clone()).collect();
    tempos.sort_by_key(|t| t.0);
    let seconds = |tick: u64| {
        let (mut secs, mut from, mut us) = (0.0, 0u64, 500_000u32);
        for &(at, tempo) in tempos.iter().take_while(|t| t.0 < tick) {
            secs += (at - from) as f64 * f64::from(us) / 1e6 / f64::from(division);
            (from, us) = (at, tempo);
        }
        secs + (tick - from) as f64 * f64::from(us) / 1e6 / f64::from(division)
    };
    let length = seconds(tracks.iter().map(|t| t.end).max().unwrap_or(0));
    let mut notes = Vec::new();
    for track in &tracks {
        // (channel, key) → start tick and velocity of the note sounding.
        let mut open: std::collections::HashMap<(u8, u8), (u64, u8)> = Default::default();
        let mut close = |channel: u8, key: u8, start: u64, velocity: u8, end: u64| {
            // Struck twice on the same tick: the empty first one makes no sound.
            if end == start {
                return;
            }
            notes.push(Note {
                start: seconds(start),
                end: seconds(end),
                key,
                channel,
                velocity,
            });
        };
        for &(tick, channel, key, velocity) in &track.events {
            if let Some((start, v)) = open.remove(&(channel, key)) {
                close(channel, key, start, v, tick);
            }
            if velocity > 0 {
                open.insert((channel, key), (tick, velocity));
            }
        }
        for ((channel, key), (start, v)) in open {
            close(channel, key, start, v, track.end);
        }
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(Song { notes, length })
}

fn add_tone(mix: &mut [f32], note: &Note, level: f32, wave: impl Fn(f32) -> f32) {
    let start = (note.start * f64::from(RATE)) as usize;
    let held = ((note.end - note.start).max(0.0) * f64::from(RATE)) as usize;
    let release = RATE as usize / 60;
    let attack = (RATE as usize / 300).max(1);
    let freq = 440.0 * 2f32.powf((f32::from(note.key) - 69.0) / 12.0);
    let loud = level * f32::from(note.velocity) / 127.0;
    for i in 0..held + release {
        let Some(sample) = mix.get_mut(start + i) else {
            break;
        };
        let t = i as f32 / RATE as f32;
        let fade_in = (i as f32 / attack as f32).min(1.0);
        let fade_out = if i < held {
            1.0
        } else {
            1.0 - (i - held) as f32 / release as f32
        };
        let decay = 0.7 + 0.3 * (-t * 5.0).exp();
        *sample += wave((t * freq).fract()) * loud * fade_in * fade_out * decay;
    }
}

fn add_drum(mix: &mut [f32], note: &Note) {
    let start = (note.start * f64::from(RATE)) as usize;
    let mut rng = 0x9E37_79B9u32 ^ (start as u32).wrapping_mul(2_654_435_761) | 1;
    let mut noise = move || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let loud = f32::from(note.velocity) / 127.0;
    let (secs, kind) = match note.key {
        36 => (0.12, 0),
        38 => (0.14, 1),
        _ => (0.04, 2),
    };
    let n = (secs * RATE as f32) as usize;
    let (mut phase, mut last) = (0f32, 0f32);
    for i in 0..n {
        let Some(sample) = mix.get_mut(start + i) else {
            break;
        };
        let t = i as f32 / RATE as f32;
        let env = (1.0 - i as f32 / n as f32).powi(2);
        let value = match kind {
            0 => {
                // A falling sine.
                phase += (45.0 + 130.0 * (-t * 35.0).exp()) / RATE as f32;
                (phase * std::f32::consts::TAU).sin() * 0.75
            }
            1 => noise() * 0.34 + (t * 190.0 * std::f32::consts::TAU).sin() * 0.16,
            _ => {
                // Only the high part of the noise.
                let n = noise();
                let high = n - last;
                last = n;
                high * 0.09
            }
        };
        *sample += value * env * loud;
    }
}

/// The song as 16-bit samples, exactly as long as the song (so it loops on the beat).
pub fn render(song: &Song) -> Vec<i16> {
    let len = (song.length * f64::from(RATE)).round() as usize;
    let mut mix = vec![0f32; len];
    let square = |duty: f32| move |p: f32| if p < duty { 1.0 } else { -1.0 };
    for note in &song.notes {
        match note.channel {
            9 => add_drum(&mut mix, note),
            0 => add_tone(&mut mix, note, 0.30, square(0.5)),
            2 => add_tone(&mut mix, note, 0.44, |p| 4.0 * (p - 0.5).abs() - 1.0),
            _ => add_tone(&mut mix, note, 0.12, square(0.25)),
        }
    }
    let peak = mix.iter().fold(0f32, |p, s| p.max(s.abs())).max(0.001);
    mix.iter()
        .map(|s| (s / peak * PEAK * f32::from(i16::MAX)) as i16)
        .collect()
}

fn fnv(data: &[u8]) -> u64 {
    data.iter()
        .fold(0xcbf2_9ce4_8422_2325 ^ u64::from(RENDER_VERSION), |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

/// Make the WAV files that are missing; delete WAV files of older versions (our own folder).
fn prepare_in(dir: &Path) -> Vec<PathBuf> {
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!("music: could not create {}: {e}", dir.display());
        return Vec::new();
    }
    let mut files = Vec::new();
    for (name, midi) in TUNES {
        let path = dir.join(format!("{name}-{:016x}.wav", fnv(midi)));
        if !path.is_file() {
            let written = parse(midi).and_then(|song| {
                let wav = crate::sound::wav_bytes(&render(&song));
                let temp = path.with_extension("tmp");
                std::fs::write(&temp, wav)
                    .and_then(|()| std::fs::rename(&temp, &path))
                    .map_err(|e| e.to_string())
            });
            if let Err(e) = written {
                log::warn!("music: {name}: {e}");
                continue;
            }
        }
        files.push(path);
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "wav") && !files.contains(&path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    files
}

/// Plays the tunes while wanted. The files are made in the background the first time.
#[derive(Default)]
pub struct Music {
    prepared: Option<Arc<Mutex<Option<Vec<PathBuf>>>>>,
    child: Option<Child>,
    /// mpv is missing or failed: no further tries.
    gave_up: bool,
}

impl Music {
    /// Call every frame. mpv is started from here (not from the preparing thread), because
    /// it is tied to the thread that starts it (see `player::game_music_command`).
    pub fn update(&mut self, wanted: bool) {
        if !wanted {
            self.stop();
            return;
        }
        if let Some(child) = &mut self.child {
            if !matches!(child.try_wait(), Ok(None)) {
                log::warn!("music: the player stopped");
                self.child = None;
                self.gave_up = true;
            }
            return;
        }
        if self.gave_up {
            return;
        }
        let shared = self.prepared.get_or_insert_with(|| {
            let shared: Arc<Mutex<Option<Vec<PathBuf>>>> = Arc::default();
            let result = shared.clone();
            std::thread::spawn(move || {
                let files = crate::store::music_dir()
                    .map(|dir| prepare_in(&dir))
                    .unwrap_or_default();
                *result.lock().unwrap_or_else(|p| p.into_inner()) = Some(files);
            });
            shared
        });
        let files = shared.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let Some(files) = files else { return };
        let cache = crate::store::prepare_player_cache();
        let spawned = crate::player::game_music_command(&files, cache.as_deref())
            .ok_or_else(|| "no tunes".to_string())
            .and_then(|mut cmd| cmd.spawn().map_err(|e| e.to_string()));
        match spawned {
            Ok(child) => self.child = Some(child),
            Err(e) => {
                log::warn!("music: could not start: {e}");
                self.gave_up = true;
            }
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Music {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with one track: 480 ticks per quarter note.
    fn midi(track: &[u8]) -> Vec<u8> {
        let mut f = b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk".to_vec();
        f.extend((track.len() as u32).to_be_bytes());
        f.extend(track);
        f
    }

    #[test]
    fn notes_tempo_and_running_status() {
        let song = parse(&midi(&[
            0, 0xff, 0x51, 3, 0x07, 0xa1, 0x20, // 120 bpm
            0, 0x90, 60, 100, // note on
            0x83, 0x60, 60, 0, // a quarter later: off by velocity 0, running status
            0, 62, 90, // on, running status
            0x83, 0x60, 0x80, 62, 0, // off
            0, 0xff, 0x2f, 0,
        ]))
        .unwrap();
        assert_eq!(song.notes.len(), 2);
        assert_eq!((song.notes[0].key, song.notes[0].velocity), (60, 100));
        assert!((song.notes[0].end - 0.5).abs() < 1e-9);
        assert!((song.notes[1].start - 0.5).abs() < 1e-9);
        assert!((song.length - 1.0).abs() < 1e-9);
    }

    #[test]
    fn tempo_changes_count_from_where_they_happen() {
        let song = parse(&midi(&[
            0, 0xff, 0x51, 3, 0x07, 0xa1, 0x20, // 120 bpm: a quarter is 0.5 s
            0x83, 0x60, 0xff, 0x51, 3, 0x03, 0xd0, 0x90, // 240 bpm from there
            0x83, 0x60, 0xff, 0x2f, 0,
        ]))
        .unwrap();
        assert!((song.length - 0.75).abs() < 1e-9, "{}", song.length);
    }

    #[test]
    fn broken_files_are_errors_not_crashes() {
        let good = TUNES[0].1;
        for cut in 0..good.len().min(400) {
            let _ = parse(&good[..cut]);
        }
        assert!(parse(b"hello").is_err());
        assert!(
            parse(&midi(&[0, 60, 100])).is_err(),
            "data without a status"
        );
        assert!(
            parse(&midi(&[0xff, 0xff, 0xff, 0xff, 0x7f])).is_err(),
            "number too long"
        );
        assert!(
            parse(&midi(&[0, 0xff, 0x01, 0x7f, 1])).is_err(),
            "meta longer than the track"
        );
        let mut smpte = midi(&[]);
        smpte[12] = 0xe7;
        assert!(parse(&smpte).is_err());
    }

    #[test]
    fn our_tunes_are_valid_and_use_every_voice() {
        for (name, data) in TUNES {
            let song = parse(data).unwrap();
            assert!(
                (50.0..80.0).contains(&song.length),
                "{name}: {}",
                song.length
            );
            for channel in [0, 1, 2, 9] {
                assert!(
                    song.notes.iter().any(|n| n.channel == channel),
                    "{name} ch {channel}"
                );
            }
            assert!(
                song.notes
                    .iter()
                    .all(|n| n.end <= song.length + 1e-6 && n.end > n.start)
            );
        }
    }

    #[test]
    fn rendering_keeps_the_song_length_and_stays_below_clipping() {
        let song = parse(TUNES[0].1).unwrap();
        let samples = render(&song);
        assert_eq!(
            samples.len(),
            (song.length * f64::from(RATE)).round() as usize
        );
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        let limit = (PEAK * f32::from(i16::MAX)) as u16;
        assert!(peak <= limit && peak > limit / 2, "{peak}");
    }

    #[test]
    fn files_are_made_once_and_old_ones_removed() {
        let dir = std::env::temp_dir().join(format!("curator-music-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old-version.wav"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"keep").unwrap();
        let files = prepare_in(&dir);
        assert_eq!(files.len(), TUNES.len());
        assert!(!dir.join("old-version.wav").exists());
        assert!(dir.join("notes.txt").exists());
        let made = std::fs::metadata(&files[0]).unwrap().modified().unwrap();
        assert_eq!(prepare_in(&dir), files);
        assert_eq!(
            std::fs::metadata(&files[0]).unwrap().modified().unwrap(),
            made
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
