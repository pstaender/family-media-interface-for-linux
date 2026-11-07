//! Optical drive and disc detection.
//!
//! Uses only desktop-independent mechanisms:
//! - the kernel CD-ROM ioctl `CDROM_DRIVE_STATUS` for tray/disc state, and
//! - udev properties (`udevadm info`) for the media type (BD/DVD/CD) and volume label.
//!
//! The drive is polled. Opening the device node also makes the kernel check for
//! media changes, so the udev properties stay current even without udisks running.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::time::Duration;

// From <linux/cdrom.h>
const CDROM_DRIVE_STATUS: libc::c_ulong = 0x5326;
const CDROMEJECT: libc::c_ulong = 0x5309;
const CDROM_LOCKDOOR: libc::c_ulong = 0x5329;
const CDSL_CURRENT: libc::c_ulong = i32::MAX as libc::c_ulong;
const CDS_NO_INFO: i32 = 0;
const CDS_NO_DISC: i32 = 1;
const CDS_TRAY_OPEN: i32 = 2;
const CDS_DISC_OK: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    BluRay,
    Dvd,
    Cd,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disc {
    pub kind: MediaKind,
    /// Volume label, e.g. `KUBO_TWO_STRINGS`.
    pub label: Option<String>,
    /// Filesystem UUID; stable per disc, useful as cache key.
    pub uuid: Option<String>,
}

impl Disc {
    /// A key that identifies this disc (not the drive), e.g. `8d3f12ee17357a03_KUBO_TWO_STRINGS`.
    /// `None` if the disc has neither UUID nor label, so it can't be recognized again.
    pub fn id(&self) -> Option<String> {
        match (&self.uuid, &self.label) {
            (Some(u), Some(l)) => Some(format!("{u}_{l}")),
            (Some(x), None) | (None, Some(x)) => Some(x.clone()),
            (None, None) => None,
        }
    }

    /// Key for the current session: the disc id, or the device if the disc has no id.
    pub fn key(&self, device: &Path) -> String {
        self.id().unwrap_or_else(|| device.display().to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveState {
    Empty,
    TrayOpen,
    /// Disc inserted but not identified yet (spinning up, udev still probing).
    NotReady,
    Loaded(Disc),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveReport {
    pub device: PathBuf,
    pub state: DriveState,
}

/// All optical drives (`/dev/sr*`), sorted.
pub fn list_drives() -> Vec<PathBuf> {
    let mut drives: Vec<PathBuf> = std::fs::read_dir("/sys/class/block")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_sr = name
                .strip_prefix("sr")
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            is_sr.then(|| PathBuf::from("/dev").join(name))
        })
        .collect();
    drives.sort();
    drives
}

/// Determine the current state of one drive.
pub fn probe(device: &Path) -> DriveState {
    match drive_status(device) {
        Ok(CDS_NO_DISC) => DriveState::Empty,
        Ok(CDS_TRAY_OPEN) => DriveState::TrayOpen,
        // CDS_NO_INFO: the drive can't tell; ask udev directly.
        Ok(CDS_DISC_OK | CDS_NO_INFO) => match udev_properties(device) {
            Ok(props) => disc_from_properties(&props)
                .map(DriveState::Loaded)
                .unwrap_or(DriveState::NotReady),
            Err(e) => DriveState::Error(e),
        },
        Ok(_) => DriveState::NotReady,
        Err(e) => DriveState::Error(format!("{}: {e}", device.display())),
    }
}

fn drive_status(device: &Path) -> std::io::Result<i32> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(device)?;
    // SAFETY: valid open fd; CDROM_DRIVE_STATUS takes an integer slot argument.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), CDROM_DRIVE_STATUS as _, CDSL_CURRENT) };
    if rc < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(rc)
    }
}

/// Open the tray. If the disc is mounted (e.g. by the desktop's automounter), unmount it
/// with udisks first and try again. Blocks for a moment, so call it from a background thread.
pub fn eject(device: &Path) -> Result<(), String> {
    let Err(first) = eject_ioctl(device) else {
        log::info!("ejected {}", device.display());
        return Ok(());
    };
    log::info!(
        "eject refused ({first}), unmounting {} first",
        device.display()
    );
    let out = Command::new("udisksctl")
        .args(["unmount", "--no-user-interaction", "--block-device"])
        .arg(device)
        .output();
    match &out {
        Ok(o) if o.status.success() => {}
        Ok(o) => log::warn!(
            "udisksctl unmount failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => log::warn!("udisksctl not available: {e}"),
    }
    eject_ioctl(device).map_err(|e| {
        log::warn!("eject failed: {e}");
        "The disc could not be ejected. Is it still in use?".to_string()
    })?;
    log::info!("ejected {} after unmounting", device.display());
    Ok(())
}

fn eject_ioctl(device: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(device)?;
    let fd = file.as_raw_fd();
    // SAFETY: valid open fd; both ioctls take an integer argument.
    let rc = unsafe {
        // Unlocking fails while the disc is mounted; the eject below reports the real error.
        libc::ioctl(fd, CDROM_LOCKDOOR as _, 0 as libc::c_ulong);
        libc::ioctl(fd, CDROMEJECT as _, 0 as libc::c_ulong)
    };
    match rc {
        0 => Ok(()),
        rc if rc < 0 => Err(std::io::Error::last_os_error()),
        // Positive values are SCSI status codes passed through by the kernel, e.g. 2 =
        // CHECK CONDITION when the drive refuses because the disc is mounted.
        rc => Err(std::io::Error::other(format!(
            "drive refused (SCSI status {rc})"
        ))),
    }
}

fn udev_properties(device: &Path) -> Result<HashMap<String, String>, String> {
    let out = Command::new("udevadm")
        .args(["info", "--query=property", "--name"])
        .arg(device)
        .output()
        .map_err(|e| format!("udevadm: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "udevadm failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(parse_udev_properties(&String::from_utf8_lossy(&out.stdout)))
}

pub fn parse_udev_properties(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// Build a [`Disc`] from udev properties, or `None` if no medium is reported (yet).
pub fn disc_from_properties(props: &HashMap<String, String>) -> Option<Disc> {
    if props.get("ID_CDROM_MEDIA").map(String::as_str) != Some("1") {
        return None;
    }
    let has = |prefix: &str| props.iter().any(|(k, v)| k.starts_with(prefix) && v == "1");
    let kind = if has("ID_CDROM_MEDIA_BD") {
        MediaKind::BluRay
    } else if has("ID_CDROM_MEDIA_DVD") || has("ID_CDROM_MEDIA_HDDVD") {
        MediaKind::Dvd
    } else if has("ID_CDROM_MEDIA_CD") {
        MediaKind::Cd
    } else {
        MediaKind::Unknown
    };
    let non_empty = |key: &str| props.get(key).filter(|v| !v.is_empty()).cloned();
    Some(Disc {
        kind,
        label: non_empty("ID_FS_LABEL"),
        uuid: non_empty("ID_FS_UUID"),
    })
}

/// Turn a volume label like `KUBO_TWO_STRINGS` into `Kubo Two Strings`.
pub fn pretty_label(label: &str) -> String {
    label
        .split(['_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let lower = w.to_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Poll all drives in a background thread. Sends the full report whenever anything changes,
/// then calls `notify` (e.g. to wake up the UI).
pub fn spawn_watcher(
    interval: Duration,
    tx: Sender<Vec<DriveReport>>,
    notify: impl Fn() + Send + 'static,
) {
    std::thread::Builder::new()
        .name("disc-watcher".into())
        .spawn(move || {
            let mut last: Option<Vec<DriveReport>> = None;
            loop {
                let reports: Vec<DriveReport> = list_drives()
                    .into_iter()
                    .map(|device| DriveReport {
                        state: probe(&device),
                        device,
                    })
                    .collect();
                if last.as_ref() != Some(&reports) {
                    log::info!("drive state changed: {reports:?}");
                    if tx.send(reports.clone()).is_err() {
                        return; // UI is gone
                    }
                    notify();
                    last = Some(reports);
                }
                std::thread::sleep(interval);
            }
        })
        .expect("spawning a thread only fails if the OS is out of resources");
}

#[cfg(test)]
mod tests {
    use super::*;

    const BD_PROPS: &str = "DEVNAME=/dev/sr0\nID_CDROM=1\nID_CDROM_BD=1\nID_CDROM_DVD=1\n\
        ID_CDROM_MEDIA=1\nID_CDROM_MEDIA_BD=1\nID_FS_LABEL=KUBO_TWO_STRINGS\n\
        ID_FS_UUID=8d3f12ee17357a03\nID_FS_TYPE=udf\n";

    #[test]
    fn detects_bluray() {
        let disc = disc_from_properties(&parse_udev_properties(BD_PROPS)).unwrap();
        assert_eq!(disc.kind, MediaKind::BluRay);
        assert_eq!(disc.label.as_deref(), Some("KUBO_TWO_STRINGS"));
        assert_eq!(disc.uuid.as_deref(), Some("8d3f12ee17357a03"));
    }

    #[test]
    fn drive_capabilities_are_not_media() {
        // ID_CDROM_BD=1 describes the drive, not the inserted disc.
        let props = parse_udev_properties("ID_CDROM=1\nID_CDROM_BD=1\nID_CDROM_DVD=1\n");
        assert_eq!(disc_from_properties(&props), None);
    }

    #[test]
    fn detects_dvd() {
        let props = parse_udev_properties(
            "ID_CDROM_BD=1\nID_CDROM_MEDIA=1\nID_CDROM_MEDIA_DVD=1\nID_FS_LABEL=MOVIE\n",
        );
        assert_eq!(disc_from_properties(&props).unwrap().kind, MediaKind::Dvd);
    }

    #[test]
    fn disc_ids() {
        let disc = |uuid: Option<&str>, label: Option<&str>| Disc {
            kind: MediaKind::BluRay,
            uuid: uuid.map(String::from),
            label: label.map(String::from),
        };
        let dev = Path::new("/dev/sr0");
        assert_eq!(disc(Some("8d3f"), Some("KUBO")).key(dev), "8d3f_KUBO");
        assert_eq!(disc(None, Some("KUBO")).id().as_deref(), Some("KUBO"));
        assert_eq!(disc(None, None).id(), None);
        assert_eq!(disc(None, None).key(dev), "/dev/sr0");
    }

    #[test]
    fn pretty_labels() {
        assert_eq!(pretty_label("KUBO_TWO_STRINGS"), "Kubo Two Strings");
        assert_eq!(pretty_label("__A_b__"), "A B");
        assert_eq!(pretty_label(""), "");
    }
}
