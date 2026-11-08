//! Emulators ("Emulators" row): DOS games in DOSBox, SNES games in Snes9x (run by RetroArch,
//! which brings automatic gamepad setup and the CRT filters). The settings become a generated
//! configuration file in our cache; the media center supervises the emulator like a program.
//!
//! Non-destructive: DOSBox 0.74 has no read-only mounts (`mount -ro` is ignored and the game
//! writes), so DOS folders and disk images run from a private copy in our data folder, where
//! the game's saves stay. RetroArch only reads the ROM, also from a copy in our folder.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{DosOptions, DriveMapping, SnesAspect, SnesFilter, SnesOptions};
use crate::tr;

/// The largest floppy image (2.88 MB); anything bigger is a hard disk.
const FLOPPY_MAX_BYTES: u64 = 2_949_120;
/// The geometry DOSBox needs for a hard disk image (it can't always work it out itself).
const HDD_SECTOR: u64 = 512;
const HDD_SECTORS: u64 = 63;
const HDD_HEADS: u64 = 16;
const MAX_COPY_DEPTH: u32 = 32;
/// What a zip file may unpack to: a damaged or malicious one must not fill the disk.
const MAX_UNZIP_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_UNZIP_ENTRIES: usize = 200_000;

/// Installed by `scripts/setup.sh`: `shaders/` and `autoconfig/`.
const ASSET_DIRS: [&str; 2] = [
    "/usr/local/share/curator/retroarch",
    "/usr/share/curator/retroarch",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Folder,
    Floppy,
    HardDisk { cylinders: u64 },
    Cd,
}

/// One DOS drive, ready for DOSBox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub letter: char,
    pub path: PathBuf,
    pub source: Source,
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// What kind of drive this path is (`None`: not a folder or a disk image we know).
pub fn classify(path: &Path, is_dir: bool, size: u64) -> Option<Source> {
    if is_dir {
        return Some(Source::Folder);
    }
    match extension(path).as_str() {
        // Unpacked, it is a folder (`prepare_dos`).
        "zip" => Some(Source::Folder),
        "iso" | "cue" => Some(Source::Cd),
        "img" | "ima" if size <= FLOPPY_MAX_BYTES => Some(Source::Floppy),
        "img" | "ima" => Some(Source::HardDisk {
            cylinders: size / (HDD_SECTOR * HDD_SECTORS * HDD_HEADS),
        }),
        _ => None,
    }
}

fn mount_line(mount: &Mount) -> String {
    let (letter, path) = (
        mount.letter.to_ascii_lowercase(),
        mount.path.to_string_lossy(),
    );
    match mount.source {
        Source::Folder => format!("mount {letter} \"{path}\""),
        Source::Floppy => format!("imgmount {letter} \"{path}\" -t floppy"),
        Source::Cd => format!("imgmount {letter} \"{path}\" -t iso"),
        Source::HardDisk { cylinders } => format!(
            "imgmount {letter} \"{path}\" -t hdd -fs fat -size {HDD_SECTOR},{HDD_SECTORS},{HDD_HEADS},{cylinders}"
        ),
    }
}

/// `GAMES\DOOM\DOOM.EXE -nosound`, optionally with a drive: go there and start it. Empty:
/// the DOS prompt on `default_drive`.
fn autostart_lines(autostart: &str, default_drive: char) -> Vec<String> {
    let text = autostart.trim();
    let (target, args) = text.split_once(' ').unwrap_or((text, ""));
    let target = target.replace('/', "\\");
    let (drive, path) = match target.as_bytes() {
        [letter, b':', ..] if letter.is_ascii_alphabetic() => {
            (letter.to_ascii_uppercase() as char, &target[2..])
        }
        _ => (default_drive, target.as_str()),
    };
    let mut lines = vec![format!("{drive}:")];
    let path = path.trim_start_matches('\\');
    if path.is_empty() {
        return lines;
    }
    let (dir, file) = path.rsplit_once('\\').unwrap_or(("", path));
    if !dir.is_empty() {
        lines.push(format!("cd \\{dir}"));
    }
    // Without `call`, a batch file wouldn't come back for the `exit`.
    let call = if file.to_ascii_lowercase().ends_with(".bat") {
        "call "
    } else {
        ""
    };
    lines.push(format!("{call}{file} {args}").trim_end().to_string());
    lines.push("exit".into());
    lines
}

/// The DOSBox configuration: fullscreen, then the drives, the startup script and the game.
pub fn dosbox_conf(options: &DosOptions, mounts: &[Mount], cache_dir: &Path) -> String {
    let cache = cache_dir.to_string_lossy();
    let mut conf = format!(
        "[sdl]\nfullscreen=true\nfullresolution=desktop\noutput=opengl\nautolock=true\n\
         mapperfile={cache}/dosbox-mapper.map\n\n\
         [dosbox]\ncaptures={cache}/capture\n\n\
         [render]\naspect={}\nscaler=none\n\n[autoexec]\n",
        options.correct_aspect
    );
    for mount in mounts {
        conf.push_str(&mount_line(mount));
        conf.push('\n');
    }
    for line in options.startup_script.lines() {
        conf.push_str(line);
        conf.push('\n');
    }
    if let Some(first) = mounts.first() {
        for line in autostart_lines(&options.autostart, first.letter) {
            conf.push_str(&line);
            conf.push('\n');
        }
    }
    conf
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Fresh, writable copy of `from`: symbolic links are skipped, so nothing outside is copied.
fn copy_tree(from: &Path, to: &Path, depth: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if depth > MAX_COPY_DEPTH {
        return Err(std::io::Error::other("folders nested too deeply"));
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target, depth + 1)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))?;
        }
    }
    Ok(())
}

/// A unique name next to the final one, so an unfinished copy is never used.
fn part_path(to: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned());
    to.with_file_name(format!("{}.part-{nanos}", name.unwrap_or_default()))
}

fn copy_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    let part = part_path(to);
    let done = copy_tree(from, &part, 0).and_then(|()| std::fs::rename(&part, to));
    if done.is_err() {
        let _ = std::fs::remove_dir_all(&part);
    }
    done
}

fn copy_image(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let part = part_path(to);
    let done = std::fs::copy(from, &part)
        .and_then(|_| std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o644)))
        .and_then(|()| std::fs::rename(&part, to));
    if done.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    done
}

fn unpack_zip_into(from: &Path, to: &Path, limit: u64) -> std::io::Result<()> {
    use std::io::Read;
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(from)?).map_err(std::io::Error::other)?;
    if archive.len() > MAX_UNZIP_ENTRIES {
        return Err(std::io::Error::other("too many files in the zip file"));
    }
    let mut left = limit;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(std::io::Error::other)?;
        // None: a name pointing outside ("../x"). Links are never created.
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        if entry.is_symlink() {
            continue;
        }
        let target = to.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        // Counted while unpacking: the sizes in a zip file's header can lie.
        let written = std::io::copy(&mut (&mut entry).take(left + 1), &mut out)?;
        if written > left {
            return Err(std::io::Error::other(
                "the zip file unpacks to too much data",
            ));
        }
        left -= written;
    }
    Ok(())
}

/// Unpack a zip file as a whole: an unfinished folder is never used.
fn unpack_zip(from: &Path, to: &Path, limit: u64) -> std::io::Result<()> {
    let part = part_path(to);
    let done = std::fs::create_dir_all(&part)
        .and_then(|()| unpack_zip_into(from, &part, limit))
        .and_then(|()| std::fs::rename(&part, to));
    if done.is_err() {
        let _ = std::fs::remove_dir_all(&part);
    }
    done
}

/// A zip file holding nothing but one disk image: that image is the drive.
fn only_image_in(folder: &Path) -> Option<PathBuf> {
    let mut items = std::fs::read_dir(folder).ok()?;
    let path = items.next()?.ok()?.path();
    let image = items.next().is_none()
        && path.is_file()
        && matches!(extension(&path).as_str(), "img" | "ima" | "iso");
    image.then_some(path)
}

/// Check the drives and make the private copies in `root`, our folder for this game. They
/// are made once, so saved games survive and the game still starts if the original is gone
/// (`keep_saves` off: a fresh copy every time). This can take a while: call it in a thread.
pub fn prepare_dos(
    drives: &[DriveMapping],
    root: &Path,
    keep_saves: bool,
) -> Result<Vec<Mount>, String> {
    let mut mounts = Vec::new();
    for drive in drives {
        let original = Path::new(&drive.path);
        let letter = drive.letter.chars().next().unwrap_or('C');
        let is_zip = extension(original) == "zip";
        let name = format!("{:016x}", fnv(&drive.path));
        let folder_copy = root.join(&name);
        let image_copy = root.join(format!("{name}.{}", extension(original)));
        if !keep_saves && original.exists() {
            let _ = std::fs::remove_dir_all(&folder_copy);
            let _ = std::fs::remove_file(&image_copy);
        }
        let existing = [&folder_copy, &image_copy].into_iter().find(|p| p.exists());
        let look_at = existing.map_or(original, PathBuf::as_path);
        let meta = std::fs::metadata(look_at).map_err(|e| {
            tr!(
                "{path} can't be read: {error}",
                path = drive.path,
                error = e
            )
        })?;
        let source = classify(look_at, meta.is_dir(), meta.len())
            .ok_or_else(|| tr!("{path} is not a folder or disk image.", path = drive.path))?;
        // A .cue file names the data file next to it, so it stays where it is (CDs are
        // read-only for DOSBox anyway).
        let path = if extension(original) == "cue" {
            original.to_path_buf()
        } else if let Some(copy) = existing {
            copy.clone()
        } else {
            let copy = if source == Source::Folder {
                folder_copy
            } else {
                image_copy
            };
            std::fs::create_dir_all(root)
                .and_then(|()| {
                    if source == Source::Folder {
                        if is_zip {
                            unpack_zip(original, &copy, MAX_UNZIP_BYTES)
                        } else {
                            copy_folder(original, &copy)
                        }
                    } else {
                        copy_image(original, &copy)
                    }
                })
                .map_err(|e| tr!("The game could not be copied: {error}", error = e))?;
            copy
        };
        let packed_image = is_zip.then(|| only_image_in(&path)).flatten();
        let packed = packed_image.and_then(|image| {
            let size = std::fs::metadata(&image).ok()?.len();
            Some((image.clone(), classify(&image, false, size)?))
        });
        let (path, source) = packed.unwrap_or((path, source));
        mounts.push(Mount {
            letter,
            path,
            source,
        });
    }
    Ok(mounts)
}

/// What the emulator gets once its files are in our folder.
pub enum Prepared {
    Dos(Vec<Mount>),
    Snes(PathBuf),
}

/// The ROM to play: an uploaded one (in `library`) as it is, others copied once to `roms`.
pub fn prepare_snes(rom: &str, library: &Path, roms: &Path) -> Result<PathBuf, String> {
    let original = Path::new(rom);
    let copy = roms.join(format!("{:016x}.{}", fnv(rom), extension(original)));
    if original.starts_with(library) {
        if original.is_file() {
            return Ok(original.to_path_buf());
        }
    } else if copy.is_file() {
        return Ok(copy);
    } else if original.is_file() {
        std::fs::create_dir_all(roms)
            .and_then(|()| copy_image(original, &copy))
            .map_err(|e| tr!("The game could not be copied: {error}", error = e))?;
        return Ok(copy);
    }
    Err(tr!("The game file was not found: {path}", path = rom))
}

pub fn dosbox_command(conf: &Path) -> Command {
    let mut cmd = Command::new("dosbox");
    cmd.arg("-conf")
        .arg(conf)
        .arg("-noconsole")
        .arg("-fullscreen");
    cmd.stdin(Stdio::null()).stdout(Stdio::null());
    crate::player::supervise(&mut cmd);
    cmd
}

/// The filter's preset, relative to `shaders/`.
fn filter_preset(filter: SnesFilter) -> Option<&'static str> {
    match filter {
        SnesFilter::None => None,
        SnesFilter::CrtEasymode => Some("crt/crt-easymode.glslp"),
        SnesFilter::ZfastCrt => Some("crt/zfast-crt.glslp"),
        SnesFilter::CrtGeom => Some("crt/crt-geom.glslp"),
        SnesFilter::CrtLottes => Some("crt/crt-lottes.glslp"),
        SnesFilter::CrtRoyale => Some("crt/crt-royale.glslp"),
    }
}

fn asset(relative: &str) -> Option<PathBuf> {
    ASSET_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(relative))
        .find(|p| p.exists())
}

/// The installed filter (`None`: no filter chosen, or `scripts/setup.sh` hasn't added them).
pub fn shader_path(filter: SnesFilter) -> Option<PathBuf> {
    asset(&format!("shaders/{}", filter_preset(filter)?))
}

/// The Snes9x core of the distribution's packages.
pub fn find_core() -> Option<PathBuf> {
    let arch = std::env::consts::ARCH;
    [
        format!("/usr/lib/{arch}-linux-gnu/libretro/snes9x_libretro.so"),
        "/usr/lib/libretro/snes9x_libretro.so".to_string(),
        "/usr/lib64/libretro/snes9x_libretro.so".to_string(),
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

/// RetroArch's gamepad profiles for this start, inside its cache folder.
pub const AUTOCONFIG_DIR: &str = "autoconfig";

/// The profiles `scripts/setup.sh` installed.
pub fn stock_autoconfig() -> Option<PathBuf> {
    asset("autoconfig")
}

/// RetroArch's profile folder: the stock ones, except those competing with `profiles`, and
/// `profiles` themselves. RetroArch reads only one folder and takes the first perfect match,
/// so adding ours isn't enough. `target` is entirely ours (in the cache) and made anew.
pub fn write_autoconfig(
    target: &Path,
    stock: Option<&Path>,
    profiles: &[crate::pad_setup::PadProfile],
) -> std::io::Result<()> {
    if target.exists() {
        std::fs::remove_dir_all(target)?;
    }
    let udev = target.join("udev");
    std::fs::create_dir_all(&udev)?;
    let stock_files = stock.and_then(|s| std::fs::read_dir(s.join("udev")).ok());
    for entry in stock_files.into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "cfg") {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let text = String::from_utf8_lossy(&bytes);
        if !profiles.iter().any(|p| p.replaces(&text)) {
            std::fs::write(udev.join(entry.file_name()), &bytes)?;
        }
    }
    for (index, profile) in profiles.iter().enumerate() {
        if let Some(text) = profile.cfg() {
            std::fs::write(udev.join(format!("curator-{index}.cfg")), text)?;
        }
    }
    Ok(())
}

pub struct RetroArchDirs {
    /// Where the core is: RetroArch needs its info file to know it can save states.
    pub core_dir: PathBuf,
    /// `$XDG_DATA_HOME`: saved games.
    pub data: PathBuf,
    /// `$XDG_CACHE_HOME`: everything else RetroArch likes to write.
    pub cache: PathBuf,
}

/// The RetroArch configuration: fullscreen, nothing written outside our folders, and no menu
/// (it could open other files, and it isn't made for a child with a remote).
pub fn retroarch_cfg(options: &SnesOptions, dirs: &RetroArchDirs, shader: bool) -> String {
    let (data, cache) = (dirs.data.to_string_lossy(), dirs.cache.to_string_lossy());
    let ratio = match options.aspect {
        SnesAspect::Ratio4x3 => 4.0 / 3.0,
        SnesAspect::Ratio8x7 => 8.0 / 7.0,
        SnesAspect::Fill => 16.0 / 9.0,
    };
    let mut lines = vec![
        ("video_driver", "gl".to_string()),
        ("video_fullscreen", "true".into()),
        ("video_windowed_fullscreen", "true".into()),
        ("video_vsync", "true".into()),
        ("video_smooth", "false".into()),
        ("video_font_enable", "false".into()),
        // 20 = "Config": use video_aspect_ratio.
        ("aspect_ratio_index", "20".into()),
        ("video_aspect_ratio_auto", "false".into()),
        ("video_aspect_ratio", format!("{ratio:.4}")),
        ("video_force_aspect", "true".into()),
        ("video_scale_integer", options.integer_scale.to_string()),
        ("video_shader_enable", shader.to_string()),
        ("audio_driver", "pulse".into()),
        ("input_autodetect_enable", "true".into()),
        ("input_joypad_driver", "udev".into()),
        ("input_menu_toggle", "nul".into()),
        ("input_menu_toggle_gamepad_combo", "0".into()),
        ("input_toggle_fullscreen", "nul".into()),
        ("input_exit_emulator", "escape".into()),
        ("savestate_auto_save", options.resume.to_string()),
        ("savestate_auto_load", options.resume.to_string()),
        // Battery saves also reach the disk if the emulator gets killed.
        ("autosave_interval", "30".into()),
        // A signal ends RetroArch without the automatic save state, so we send QUIT on its
        // standard input (`player::ask_to_stop`); once is enough.
        ("stdin_cmd_enable", "true".into()),
        ("quit_press_twice", "false".into()),
        ("config_save_on_exit", "false".into()),
        ("auto_overrides_enable", "false".into()),
        ("auto_remaps_enable", "false".into()),
        ("auto_shaders_enable", "false".into()),
        ("game_specific_options", "false".into()),
        ("history_list_enable", "false".into()),
        ("savefiles_in_content_dir", "false".into()),
        ("savestates_in_content_dir", "false".into()),
        ("systemfiles_in_content_dir", "false".into()),
        ("screenshots_in_content_dir", "false".into()),
        ("sort_savefiles_enable", "false".into()),
        ("sort_savestates_enable", "false".into()),
        ("savefile_directory", format!("{data}/saves")),
        ("savestate_directory", format!("{data}/saves")),
        ("system_directory", format!("{data}/system")),
        ("screenshot_directory", format!("{cache}/screenshots")),
        ("cache_directory", format!("{cache}/tmp")),
        ("playlist_directory", format!("{cache}/playlists")),
        ("content_history_path", format!("{cache}/history.lpl")),
        ("global_core_options", "true".into()),
        ("core_options_path", format!("{data}/core-options.cfg")),
        ("libretro_info_path", "/usr/share/libretro/info".into()),
        (
            "libretro_directory",
            dirs.core_dir.to_string_lossy().into_owned(),
        ),
    ];
    lines.push(("joypad_autoconfig_dir", format!("{cache}/{AUTOCONFIG_DIR}")));
    lines
        .into_iter()
        .map(|(key, value)| format!("{key} = \"{value}\"\n"))
        .collect()
}

pub fn retroarch_command(cfg: &Path, core: &Path, shader: Option<&Path>, rom: &Path) -> Command {
    let mut cmd = Command::new("retroarch");
    cmd.arg("--config")
        .arg(cfg)
        .arg("--fullscreen")
        .arg("-L")
        .arg(core);
    if let Some(shader) = shader {
        let mut arg = std::ffi::OsString::from("--set-shader=");
        arg.push(shader);
        cmd.arg(arg);
    }
    cmd.arg(rom);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null());
    crate::player::supervise(&mut cmd);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("curator-emu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn drives_by_what_they_are() {
        let c = |name: &str, dir, size| classify(Path::new(name), dir, size);
        assert_eq!(c("/g", true, 0), Some(Source::Folder));
        assert_eq!(c("/g/a.IMG", false, 1_474_560), Some(Source::Floppy));
        assert_eq!(
            c("/g/a.img", false, 20 * 1024 * 1024),
            Some(Source::HardDisk { cylinders: 40 })
        );
        assert_eq!(c("/g/a.iso", false, 650_000_000), Some(Source::Cd));
        assert_eq!(c("/g/a.ZIP", false, 10), Some(Source::Folder));
        assert_eq!(c("/g/a.exe", false, 10), None);
    }

    #[test]
    fn autostart() {
        assert_eq!(
            autostart_lines("GAMES\\DOOM\\DOOM.EXE -nosound", 'C'),
            ["C:", "cd \\GAMES\\DOOM", "DOOM.EXE -nosound", "exit"]
        );
        assert_eq!(
            autostart_lines("a:/run.bat", 'C'),
            ["A:", "call run.bat", "exit"]
        );
        assert_eq!(autostart_lines("", 'D'), ["D:"], "stays at the DOS prompt");
    }

    #[test]
    fn our_gamepad_profile_is_the_only_perfect_match() {
        use crate::pad_setup::{PadBind, PadProfile, SnesButton};
        let dir = std::env::temp_dir().join(format!("curator-autoconfig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let stock = dir.join("stock");
        std::fs::create_dir_all(stock.join("udev")).unwrap();
        let file = |name: &str, device: &str| {
            let text = format!(
                "input_driver = \"udev\"\ninput_device = \"{device}\"\n\
                 input_vendor_id = \"121\"\ninput_product_id = \"17\"\n"
            );
            std::fs::write(stock.join("udev").join(name), text).unwrap();
        };
        file("nes.cfg", "USB Gamepad ");
        file("saturn.cfg", "USB Gamepad ");
        file("other.cfg", "Other Pad");
        std::fs::write(stock.join("udev/readme.txt"), "x").unwrap();
        let ours = PadProfile {
            name: "USB Gamepad ".into(),
            vendor: 121,
            product: 17,
            binds: [(SnesButton::B, PadBind::Button(2))].into(),
        };
        let target = dir.join("out");
        std::fs::create_dir_all(target.join("udev")).unwrap();
        std::fs::write(target.join("udev/stale.cfg"), "old").unwrap();
        write_autoconfig(&target, Some(&stock), &[ours]).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(target.join("udev"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["curator-0.cfg", "other.cfg"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dosbox_configuration() {
        let mounts = [
            Mount {
                letter: 'C',
                path: "/my games/x".into(),
                source: Source::Folder,
            },
            Mount {
                letter: 'A',
                path: "/d.img".into(),
                source: Source::HardDisk { cylinders: 40 },
            },
        ];
        let options = DosOptions {
            drives: Vec::new(),
            autostart: "GAME.EXE".into(),
            startup_script: "keyb gr\ncycles 3000".into(),
            correct_aspect: false,
            keep_saves: true,
        };
        let conf = dosbox_conf(&options, &mounts, Path::new("/cache"));
        assert!(conf.contains("fullscreen=true") && conf.contains("aspect=false"));
        let auto = conf.split("[autoexec]\n").nth(1).unwrap();
        assert_eq!(
            auto.lines().collect::<Vec<_>>(),
            [
                "mount c \"/my games/x\"",
                "imgmount a \"/d.img\" -t hdd -fs fat -size 512,63,16,40",
                "keyb gr",
                "cycles 3000",
                "C:",
                "GAME.EXE",
                "exit",
            ]
        );
    }

    #[test]
    fn dos_games_run_from_a_private_copy() {
        let base = scratch("copy");
        let game = base.join("game");
        std::fs::create_dir_all(game.join("sub")).unwrap();
        std::fs::write(game.join("sub/A.TXT"), "original").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", game.join("link")).unwrap();
        let image = base.join("disk.img");
        std::fs::write(&image, vec![0u8; 1024]).unwrap();
        let drives = [
            DriveMapping {
                letter: "C".into(),
                path: game.to_string_lossy().into_owned(),
            },
            DriveMapping {
                letter: "A".into(),
                path: image.to_string_lossy().into_owned(),
            },
        ];
        let root = base.join("ours");
        let mounts = prepare_dos(&drives, &root, true).unwrap();
        assert!(mounts.iter().all(|m| m.path.starts_with(&root)));
        let copied = mounts[0].path.join("sub/A.TXT");
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "original");
        assert!(!mounts[0].path.join("link").exists(), "links stay out");
        // A saved game is kept, and the original stays as it was.
        std::fs::write(&copied, "saved").unwrap();
        assert_eq!(prepare_dos(&drives, &root, true).unwrap(), mounts);
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "saved");
        assert_eq!(
            std::fs::read_to_string(game.join("sub/A.TXT")).unwrap(),
            "original"
        );
        assert!(
            prepare_dos(
                &[DriveMapping {
                    letter: "C".into(),
                    path: "/no/such/game".into()
                }],
                &root,
                true
            )
            .is_err()
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    fn drive(path: &Path) -> [DriveMapping; 1] {
        [DriveMapping {
            letter: "C".into(),
            path: path.to_string_lossy().into_owned(),
        }]
    }

    #[test]
    fn a_zip_file_is_a_drive() {
        let base = scratch("zip");
        let game = base.join("game.zip");
        make_zip(&game, &[("GAME/START.EXE", b"exe"), ("README.TXT", b"hi")]);
        let (drives, root) = (drive(&game), base.join("ours"));
        let mounts = prepare_dos(&drives, &root, true).unwrap();
        assert_eq!(mounts[0].source, Source::Folder);
        assert!(mounts[0].path.starts_with(&root));
        let exe = mounts[0].path.join("GAME/START.EXE");
        assert_eq!(std::fs::read(&exe).unwrap(), b"exe");
        // Saved games stay; the option off unpacks it fresh; the zip gone still plays.
        std::fs::write(&exe, "saved").unwrap();
        prepare_dos(&drives, &root, true).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"saved");
        prepare_dos(&drives, &root, false).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"exe");
        std::fs::remove_file(&game).unwrap();
        assert_eq!(prepare_dos(&drives, &root, true).unwrap(), mounts);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_zip_with_only_a_disk_image_mounts_the_image() {
        let base = scratch("zipimg");
        let game = base.join("game.zip");
        make_zip(&game, &[("DISK.IMG", &[0u8; 2048])]);
        let (drives, root) = (drive(&game), base.join("ours"));
        for _ in 0..2 {
            let mounts = prepare_dos(&drives, &root, true).unwrap();
            assert_eq!(mounts[0].source, Source::Floppy);
            assert!(mounts[0].path.ends_with("DISK.IMG"));
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn zip_files_cannot_escape_or_fill_the_disk() {
        let base = scratch("zipbad");
        let evil = base.join("evil.zip");
        make_zip(&evil, &[("../../outside.txt", b"x"), ("OK.TXT", b"y")]);
        let target = base.join("a/b/out");
        unpack_zip(&evil, &target, 1024).unwrap();
        assert!(target.join("OK.TXT").is_file());
        assert!(!base.join("a/outside.txt").exists());
        assert!(!base.join("a/b/outside.txt").exists());
        let big = base.join("big.zip");
        make_zip(&big, &[("BIG.DAT", &[0u8; 4096])]);
        let refused = base.join("refused");
        assert!(unpack_zip(&big, &refused, 4095).is_err());
        assert!(!refused.exists(), "nothing half-done stays");
        let broken = base.join("broken.zip");
        std::fs::write(&broken, "not a zip").unwrap();
        assert!(prepare_dos(&drive(&broken), &base.join("ours"), true).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn saves_survive_a_missing_original_and_can_be_reset() {
        let base = scratch("keep");
        let image = base.join("disk.img");
        std::fs::write(&image, vec![1u8; 1024]).unwrap();
        let drives = [DriveMapping {
            letter: "A".into(),
            path: image.to_string_lossy().into_owned(),
        }];
        let root = base.join("ours");
        let first = prepare_dos(&drives, &root, true).unwrap();
        std::fs::write(&first[0].path, vec![2u8; 1024]).unwrap();
        // Off: a fresh copy, the saved state is gone.
        let fresh = prepare_dos(&drives, &root, false).unwrap();
        assert_eq!(std::fs::read(&fresh[0].path).unwrap(), vec![1u8; 1024]);
        // The original gone: the private copy still plays, also with the option off.
        std::fs::remove_file(&image).unwrap();
        assert_eq!(prepare_dos(&drives, &root, false).unwrap(), fresh);
        assert_eq!(std::fs::read(&fresh[0].path).unwrap(), vec![1u8; 1024]);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn roms_end_up_in_our_folder() {
        let base = scratch("rom");
        let (library, roms) = (base.join("library"), base.join("roms"));
        std::fs::create_dir_all(&library).unwrap();
        let uploaded = library.join("a.sfc");
        std::fs::write(&uploaded, "rom").unwrap();
        let text = uploaded.to_str().unwrap();
        assert_eq!(prepare_snes(text, &library, &roms).unwrap(), uploaded);
        let outside = base.join("b.sfc");
        std::fs::write(&outside, "rom").unwrap();
        let text = outside.to_str().unwrap();
        let copy = prepare_snes(text, &library, &roms).unwrap();
        assert!(copy.starts_with(&roms) && copy.is_file());
        std::fs::remove_file(&outside).unwrap();
        assert_eq!(prepare_snes(text, &library, &roms).unwrap(), copy, "kept");
        assert!(prepare_snes("/no/such.sfc", &library, &roms).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn retroarch_configuration_keeps_to_our_folders() {
        let dirs = RetroArchDirs {
            core_dir: "/cores".into(),
            data: "/data".into(),
            cache: "/cache".into(),
        };
        let options = SnesOptions {
            rom: "/roms/a.sfc".into(),
            filter: SnesFilter::CrtGeom,
            aspect: SnesAspect::Ratio8x7,
            integer_scale: true,
            resume: true,
        };
        let cfg = retroarch_cfg(&options, &dirs, true);
        for line in [
            "video_fullscreen = \"true\"",
            "video_aspect_ratio = \"1.1429\"",
            "video_scale_integer = \"true\"",
            "video_shader_enable = \"true\"",
            "input_menu_toggle = \"nul\"",
            "savefile_directory = \"/data/saves\"",
            "savefiles_in_content_dir = \"false\"",
            "savestate_auto_save = \"true\"",
            "stdin_cmd_enable = \"true\"",
            "quit_press_twice = \"false\"",
            "global_core_options = \"true\"",
            "libretro_directory = \"/cores\"",
            "savestate_auto_load = \"true\"",
            "config_save_on_exit = \"false\"",
        ] {
            assert!(cfg.lines().any(|l| l == line), "{line}");
        }
        let cmd = retroarch_command(
            Path::new("/c.cfg"),
            Path::new("/core.so"),
            Some(Path::new("/s.glslp")),
            Path::new("/roms/a b.sfc"),
        );
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.contains(&std::ffi::OsStr::new("--set-shader=/s.glslp")));
        assert_eq!(
            args.last().unwrap(),
            &"/roms/a b.sfc",
            "no shell, one argument"
        );
    }

    /// A real DOSBox with what we generate, when it is installed (no window is needed).
    #[test]
    fn real_dosbox_mounts_and_starts_the_game() {
        if Command::new("dosbox").arg("--version").output().is_err() {
            return;
        }
        let base = scratch("real");
        let game = base.join("my game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(
            game.join("RUN.BAT"),
            "@echo off\r\necho done > C:\\OUT.TXT\r\n",
        )
        .unwrap();
        let drives = [DriveMapping {
            letter: "C".into(),
            path: game.to_string_lossy().into_owned(),
        }];
        let root = base.join("ours");
        let mounts = prepare_dos(&drives, &root, true).unwrap();
        let options = DosOptions {
            drives: Vec::new(),
            autostart: "RUN.BAT".into(),
            startup_script: String::new(),
            correct_aspect: true,
            keep_saves: true,
        };
        let conf = base.join("t.conf");
        std::fs::write(&conf, dosbox_conf(&options, &mounts, &base)).unwrap();
        let status = dosbox_command(&conf)
            .env("SDL_VIDEODRIVER", "dummy")
            .env("SDL_AUDIODRIVER", "dummy")
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(mounts[0].path.join("OUT.TXT").exists(), "the game ran");
        assert!(!game.join("OUT.TXT").exists(), "in the private copy");
        std::fs::remove_dir_all(base).unwrap();
    }
}
