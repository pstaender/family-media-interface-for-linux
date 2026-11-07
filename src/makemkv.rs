//! Disc analysis with MakeMKV's command line tool (`makemkvcon`).
//!
//! `makemkvcon -r info dev:/dev/sr0` prints machine-readable ("robot") lines, e.g.
//!
//! ```text
//! CINFO:2,0,"Kubo and the Two Strings"
//! TINFO:0,9,0,"1:41:39"
//! TINFO:0,16,0,"00800.mpls"
//! MSG:5010,0,0,"Failed to open disc","Failed to open disc"
//! ```
//!
//! We use it to get the disc's real name and to find the main movie's playlist.
//! Attribute ids are from MakeMKV's `apdefs.h`.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

const AP_NAME: u32 = 2;
const AP_CHAPTER_COUNT: u32 = 8;
const AP_DURATION: u32 = 9;
const AP_DISK_SIZE_BYTES: u32 = 11;
const AP_SOURCE_FILE_NAME: u32 = 16;

/// Titles shorter than this are never considered the main movie.
const MIN_MAIN_TITLE_SECS: u32 = 20 * 60;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscInfo {
    /// Disc title, e.g. from the disc's metadata (`Kubo and the Two Strings`).
    pub name: Option<String>,
    pub titles: Vec<TitleInfo>,
    /// Messages MakeMKV printed (errors, registration hints, …).
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleInfo {
    pub id: u32,
    pub duration_secs: u32,
    pub chapters: u32,
    pub size_bytes: u64,
    /// Source file, e.g. `00800.mpls`.
    pub source_file: Option<String>,
}

impl TitleInfo {
    /// Blu-ray playlist number from the source file (`00800.mpls` → 800).
    pub fn playlist(&self) -> Option<u32> {
        self.source_file
            .as_deref()?
            .strip_suffix(".mpls")?
            .parse()
            .ok()
    }
}

impl DiscInfo {
    /// Pick the main movie: the longest title, then most chapters, then biggest.
    ///
    /// Limitation: discs with "playlist obfuscation" (many fake playlists of the same length)
    /// can fool this heuristic. A per-disc override in the config is planned.
    pub fn main_title(&self) -> Option<&TitleInfo> {
        self.titles
            .iter()
            .filter(|t| t.duration_secs >= MIN_MAIN_TITLE_SECS)
            .max_by(|a, b| {
                (
                    a.duration_secs,
                    a.chapters,
                    a.size_bytes,
                    std::cmp::Reverse(a.id),
                )
                    .cmp(&(
                        b.duration_secs,
                        b.chapters,
                        b.size_bytes,
                        std::cmp::Reverse(b.id),
                    ))
            })
    }
}

/// Phases of a MakeMKV disc scan, in order, as reported by `PRGC` progress lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    FindingDrive,
    ReadingDisc,
    Decrypting,
    CopyProtection,
    FindingMovie,
}

impl Phase {
    pub const ALL: [Phase; 5] = [
        Phase::FindingDrive,
        Phase::ReadingDisc,
        Phase::Decrypting,
        Phase::CopyProtection,
        Phase::FindingMovie,
    ];

    /// Map MakeMKV's progress codes (`PRGC:<code>,…`) to a phase.
    fn from_code(code: u32) -> Option<Phase> {
        match code {
            5018 => Some(Phase::FindingDrive), // Scanning CD-ROM devices
            3400 | 3401 | 3404 => Some(Phase::ReadingDisc), // AV clips, playlists, opening
            3402 => Some(Phase::Decrypting),
            3406 => Some(Phase::CopyProtection), // BD+
            3103 => Some(Phase::FindingMovie),   // Processing titles
            _ => None,
        }
    }

    /// Text for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Phase::FindingDrive => "Starting",
            Phase::ReadingDisc => "Reading disc",
            Phase::Decrypting | Phase::CopyProtection => "Unlocking copy protection",
            Phase::FindingMovie => "Finding the movie",
        }
    }

    /// Typical duration in seconds, measured on a real Blu-ray. Used to weight the phases
    /// for the overall estimate, because MakeMKV reports no progress during the long ones.
    fn expected_secs(self) -> f32 {
        match self {
            Phase::FindingDrive => 18.0,
            Phase::ReadingDisc => 2.0,
            Phase::Decrypting => 30.0,
            Phase::CopyProtection => 45.0,
            Phase::FindingMovie => 13.0,
        }
    }
}

/// Latest progress of a running scan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScanProgress {
    pub phase: Phase,
    /// Progress inside the phase (0–1) as reported by MakeMKV; often stays 0 for a long time.
    pub phase_fraction: f32,
}

impl ScanProgress {
    /// Estimated overall progress (0–1). Within a phase, the reported fraction is used, or the
    /// elapsed time compared with the phase's typical duration (capped at 90 % of the phase),
    /// whichever is further along. This way the bar keeps moving even when MakeMKV is silent.
    pub fn estimate(&self, elapsed_in_phase: std::time::Duration) -> f32 {
        let total: f32 = Phase::ALL.iter().map(|p| p.expected_secs()).sum();
        let done: f32 = Phase::ALL
            .iter()
            .take_while(|p| **p != self.phase)
            .map(|p| p.expected_secs())
            .sum();
        let expected = self.phase.expected_secs();
        let by_time = (elapsed_in_phase.as_secs_f32() / expected).min(0.9);
        let within = self.phase_fraction.clamp(0.0, 1.0).max(by_time);
        ((done + within * expected) / total).clamp(0.0, 0.99)
    }
}

/// Update `current` from one robot line. Returns true if the progress changed.
fn apply_progress_line(line: &str, current: &mut Option<ScanProgress>) -> bool {
    let Some((kind, rest)) = line.split_once(':') else {
        return false;
    };
    let f = parse_fields(rest);
    let num = |i: usize| f.get(i).and_then(|s| s.trim().parse::<u32>().ok());
    let new = match kind {
        "PRGC" => match num(0).and_then(Phase::from_code) {
            Some(phase) if current.map(|c| c.phase) != Some(phase) => Some(ScanProgress {
                phase,
                phase_fraction: 0.0,
            }),
            _ => None,
        },
        "PRGV" => match (current.as_ref(), num(0), num(2)) {
            (Some(c), Some(cur), Some(max)) if max > 0 => Some(ScanProgress {
                phase: c.phase,
                phase_fraction: cur as f32 / max as f32,
            }),
            _ => None,
        },
        _ => None,
    };
    match new {
        Some(p) if Some(p) != *current => {
            *current = Some(p);
            true
        }
        _ => false,
    }
}

/// Run `makemkvcon` on a drive. This takes a while (about 1–2 minutes for a Blu-ray with
/// BD+), so call it from a background thread. `on_progress` is called whenever the
/// progress changes.
pub fn scan(device: &Path, mut on_progress: impl FnMut(ScanProgress)) -> Result<DiscInfo, String> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let mut child = Command::new("makemkvcon")
        .args(["-r", "--cache=1", "--progress=-same", "info"])
        .arg(format!("dev:{}", device.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "MakeMKV is not installed (makemkvcon not found). Run scripts/setup.sh.".into()
            } else {
                format!("Could not run makemkvcon: {e}")
            }
        })?;
    let stdout = child
        .stdout
        .take()
        .expect("stdout is piped, so it is always present");
    let mut text = String::new();
    let mut progress = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| format!("Reading makemkvcon output failed: {e}"))?;
        if apply_progress_line(&line, &mut progress)
            && let Some(p) = progress
        {
            on_progress(p);
        }
        text.push_str(&line);
        text.push('\n');
    }
    let status = child
        .wait()
        .map_err(|e| format!("makemkvcon did not finish: {e}"))?;
    let info = parse_robot_output(&text);
    if info.titles.is_empty() {
        let detail = info
            .messages
            .last()
            .cloned()
            .unwrap_or_else(|| format!("makemkvcon exited with {status}"));
        return Err(format!("MakeMKV found no titles: {detail}"));
    }
    Ok(info)
}

pub fn parse_robot_output(text: &str) -> DiscInfo {
    let mut info = DiscInfo::default();
    for line in text.lines() {
        let Some((kind, rest)) = line.split_once(':') else {
            continue;
        };
        let f = parse_fields(rest);
        let num = |i: usize| f.get(i).and_then(|s| s.parse::<u32>().ok());
        match kind {
            "CINFO" => {
                if num(0) == Some(AP_NAME) {
                    info.name = f.get(2).filter(|s| !s.is_empty()).cloned();
                }
            }
            "TINFO" => {
                let (Some(id), Some(attr), Some(value)) = (num(0), num(1), f.get(3)) else {
                    continue;
                };
                let title = match info.titles.iter_mut().position(|t| t.id == id) {
                    Some(i) => &mut info.titles[i],
                    None => {
                        info.titles.push(TitleInfo {
                            id,
                            ..Default::default()
                        });
                        info.titles.last_mut().expect("just pushed")
                    }
                };
                match attr {
                    AP_CHAPTER_COUNT => title.chapters = value.parse().unwrap_or(0),
                    AP_DURATION => title.duration_secs = parse_duration(value).unwrap_or(0),
                    AP_DISK_SIZE_BYTES => title.size_bytes = value.parse().unwrap_or(0),
                    AP_SOURCE_FILE_NAME => title.source_file = Some(value.clone()),
                    _ => {}
                }
            }
            "MSG" => {
                if let Some(msg) = f.get(3) {
                    info.messages.push(msg.clone());
                }
            }
            _ => {}
        }
    }
    info
}

/// `h:mm:ss` → seconds.
fn parse_duration(s: &str) -> Option<u32> {
    s.split(':')
        .try_fold(0u32, |acc, part| Some(acc * 60 + part.parse::<u32>().ok()?))
}

/// Split a robot line's payload: comma separated, strings in double quotes
/// (with `\"` or `""` as escaped quote).
fn parse_fields(s: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    let mut in_quotes = false;
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '\\' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => fields.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"MSG:1005,0,1,"MakeMKV v2.0.0 linux(x64-release) started","%1 started","MakeMKV v2.0.0 linux(x64-release)"
DRV:0,2,999,12,"BD-RW BDR-UD04","KUBO_TWO_STRINGS","/dev/sr0"
TCOUNT:3
CINFO:1,6209,"Blu-ray disc"
CINFO:2,0,"Kubo and the Two Strings"
CINFO:32,0,"KUBO_TWO_STRINGS"
TINFO:0,2,0,"Kubo and the Two Strings"
TINFO:0,8,0,"16"
TINFO:0,9,0,"1:41:39"
TINFO:0,11,0,"31262791680"
TINFO:0,16,0,"00800.mpls"
TINFO:1,8,0,"1"
TINFO:1,9,0,"0:12:03"
TINFO:1,16,0,"00801.mpls"
TINFO:2,8,0,"2"
TINFO:2,9,0,"1:41:39"
TINFO:2,11,0,"1000"
TINFO:2,16,0,"00805.mpls"
SINFO:0,0,1,6201,"Video"
"#;

    #[test]
    fn parses_name_and_titles() {
        let info = parse_robot_output(SAMPLE);
        assert_eq!(info.name.as_deref(), Some("Kubo and the Two Strings"));
        assert_eq!(info.titles.len(), 3);
        let t0 = &info.titles[0];
        assert_eq!(t0.duration_secs, 3600 + 41 * 60 + 39);
        assert_eq!(t0.chapters, 16);
        assert_eq!(t0.size_bytes, 31_262_791_680);
        assert_eq!(t0.playlist(), Some(800));
        assert_eq!(info.messages.len(), 1);
    }

    #[test]
    fn main_title_prefers_longest_then_chapters() {
        let info = parse_robot_output(SAMPLE);
        assert_eq!(info.main_title().unwrap().playlist(), Some(800));
    }

    #[test]
    fn no_main_title_if_everything_is_short() {
        let info = parse_robot_output("TINFO:0,9,0,\"0:05:00\"\n");
        assert_eq!(info.main_title(), None);
    }

    #[test]
    fn field_parsing_handles_quotes_and_commas() {
        assert_eq!(
            parse_fields(r#"1,"a, b","say ""hi""","x\"y""#),
            vec!["1", "a, b", "say \"hi\"", "x\"y"]
        );
    }

    #[test]
    fn progress_follows_phases_and_values() {
        let mut p = None;
        assert!(apply_progress_line(
            r#"PRGC:5018,0,"Scanning CD-ROM devices""#,
            &mut p
        ));
        assert_eq!(p.unwrap().phase, Phase::FindingDrive);
        assert!(apply_progress_line(r#"PRGC:3402,0,"Decrypting""#, &mut p));
        assert!(apply_progress_line("PRGV:16384,11924,65536", &mut p));
        assert_eq!(
            p,
            Some(ScanProgress {
                phase: Phase::Decrypting,
                phase_fraction: 0.25
            })
        );
        // Same value again: no change. Other lines: ignored.
        assert!(!apply_progress_line("PRGV:16384,12000,65536", &mut p));
        assert!(!apply_progress_line(r#"TINFO:0,9,0,"1:42:00""#, &mut p));
        // Unknown phase codes keep the current phase.
        assert!(!apply_progress_line(
            r#"PRGC:9999,0,"Something new""#,
            &mut p
        ));
        assert_eq!(p.unwrap().phase, Phase::Decrypting);
    }

    #[test]
    fn estimate_grows_and_stays_below_100() {
        use std::time::Duration;
        let at = |phase, frac, secs| {
            ScanProgress {
                phase,
                phase_fraction: frac,
            }
            .estimate(Duration::from_secs(secs))
        };
        let start = at(Phase::FindingDrive, 0.0, 0);
        let mid = at(Phase::CopyProtection, 0.0, 10);
        let late = at(Phase::FindingMovie, 1.0, 100);
        assert_eq!(start, 0.0);
        assert!(start < mid && mid < late);
        assert!(late <= 0.99);
        // Time moves the bar inside a silent phase, but never past the phase's end.
        let silent = at(Phase::CopyProtection, 0.0, 1000);
        assert!(silent < at(Phase::FindingMovie, 0.0, 0));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("1:41:39"), Some(6099));
        assert_eq!(parse_duration("0:00:05"), Some(5));
        assert_eq!(parse_duration("x"), None);
    }
}

#[cfg(test)]
mod real_disc_tests {
    use super::*;

    /// Real `makemkvcon -r info` output of "Kubo and the Two Strings" (16 titles, extras + main movie).
    const KUBO: &str = include_str!("../tests/data/makemkv_kubo.txt");

    #[test]
    fn kubo_main_movie_is_playlist_801() {
        let info = parse_robot_output(KUBO);
        assert_eq!(info.name.as_deref(), Some("Kubo and the Two Strings"));
        assert_eq!(info.titles.len(), 16);
        let main = info.main_title().unwrap();
        assert_eq!(main.playlist(), Some(801));
        assert_eq!(main.duration_secs, 6120);
    }
}
