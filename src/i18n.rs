//! Interface languages (English, German).
//!
//! The English text is the key (`tr!("Back")`), like gettext: code stays readable and English
//! needs no table. Other languages look the text up in their table; anything missing falls back
//! to English. Placeholders are named: `tr!("Track {n} of {total}", n = 3, total = 12)`.
//! `tests::every_text_is_translated` scans the sources, so a new `tr!` without a German entry
//! (or a German entry nobody uses any more) fails the tests.
//!
//! The language is global (set from the config each frame: `set`); the website window gets it
//! with `--lang`. Shared with `web/` via `#[path]`, so this file only uses `std`.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::AtomicU8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En = 0,
    De = 1,
}

impl Lang {
    /// The code used in the config (`home.language`) and on the command line.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        match code {
            "en" => Some(Lang::En),
            "de" => Some(Lang::De),
            _ => None,
        }
    }

    /// From a locale like `de_DE.UTF-8`, `de_AT@euro`, `de` or `C`. Unknown → `None`.
    pub fn from_locale(locale: &str) -> Option<Lang> {
        let lang = locale
            .split(['_', '.', '@', '-'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        Lang::from_code(&lang)
    }

    /// The computer's language: the first known one of `LANGUAGE` (a list), `LC_ALL`,
    /// `LC_MESSAGES`, `LANG`; English otherwise. `get` reads one variable.
    pub fn detect_with(get: impl Fn(&str) -> Option<String>) -> Lang {
        // Like gettext: LC_ALL / LC_MESSAGES / LANG decide; LANGUAGE (a priority list) is
        // only used when that locale isn't "C".
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|v| get(v).filter(|s| !s.is_empty()));
        if locale.as_deref().is_some_and(|l| l == "C" || l == "POSIX") {
            return Lang::En;
        }
        let languages = get("LANGUAGE").unwrap_or_default();
        languages
            .split(':')
            .find_map(Lang::from_locale)
            .or_else(|| locale.as_deref().and_then(Lang::from_locale))
            .unwrap_or(Lang::En)
    }

    pub fn detect() -> Lang {
        static DETECTED: LazyLock<Lang> =
            LazyLock::new(|| Lang::detect_with(|v| std::env::var(v).ok()));
        *DETECTED
    }

    /// The config value: `auto` (or anything unknown) → detected.
    pub fn from_setting(setting: &str) -> Lang {
        Lang::from_code(setting).unwrap_or_else(Lang::detect)
    }
}

#[cfg_attr(test, allow(dead_code))]
static CURRENT: AtomicU8 = AtomicU8::new(Lang::En as u8);

#[cfg(test)]
thread_local! {
    /// Tests run in parallel threads: a language set in one test stays in its thread.
    static TEST_LANG: std::cell::Cell<u8> = const { std::cell::Cell::new(Lang::En as u8) };
}

pub fn set(lang: Lang) {
    #[cfg(test)]
    TEST_LANG.with(|l| l.set(lang as u8));
    #[cfg(not(test))]
    CURRENT.store(lang as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn current() -> Lang {
    #[cfg(test)]
    let code = TEST_LANG.with(std::cell::Cell::get);
    #[cfg(not(test))]
    let code = CURRENT.load(std::sync::atomic::Ordering::Relaxed);
    match code {
        1 => Lang::De,
        _ => Lang::En,
    }
}

/// The text in the current language (use `tr!`).
pub fn tr(text: &'static str) -> &'static str {
    match current() {
        Lang::En => text,
        Lang::De => {
            static TABLE: LazyLock<HashMap<&'static str, &'static str>> =
                LazyLock::new(|| DE.iter().copied().collect());
            TABLE.get(text).copied().unwrap_or(text)
        }
    }
}

/// `tr!("Back")` → `&'static str`; `tr!("Track {n} of {total}", n = 3, total = 12)` → `String`.
#[macro_export]
macro_rules! tr {
    ($text:literal) => {
        $crate::i18n::tr($text)
    };
    ($text:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut text = $crate::i18n::tr($text).to_string();
        $(
            text = text.replace(concat!("{", stringify!($name), "}"), &$value.to_string());
        )+
        text
    }};
}

/// Marks an English text for translation without translating it here (like gettext's `N_`):
/// for texts that are also sent to the admin panel (English); the TV translates them with
/// `i18n::tr` when it draws them.
#[macro_export]
macro_rules! tr_noop {
    ($text:literal) => {
        $text
    };
}

/// German. Keep the English keys exactly as written in the code.
const DE: &[(&str, &str)] = &[
    // Home screen
    ("Discs", "Discs"),
    ("Folders", "Ordner"),
    ("Websites", "Webseiten"),
    ("Not available", "Nicht verfügbar"),
    ("Not installed", "Nicht installiert"),
    (
        "Settings could not be loaded",
        "Einstellungen konnten nicht geladen werden",
    ),
    ("Discs are turned off", "Discs sind ausgeschaltet"),
    ("No disc drive found", "Kein Laufwerk gefunden"),
    (
        "DVDs are not supported yet",
        "DVDs werden noch nicht unterstützt",
    ),
    ("This disc is not a Blu-ray", "Diese Disc ist keine Blu-ray"),
    ("Reading disc…", "Disc wird gelesen …"),
    ("Tray is open", "Laufwerk ist offen"),
    ("Insert a Blu-ray", "Blu-ray einlegen"),
    ("Drive not accessible", "Kein Zugriff auf das Laufwerk"),
    ("Videos", "Videos"),
    ("Music", "Musik"),
    ("Pictures", "Bilder"),
    ("Invalid address", "Ungültige Adresse"),
    ("Time left {time}", "Noch {time}"),
    // Disc scan
    ("Starting", "Start"),
    ("Reading disc", "Disc wird gelesen"),
    ("Unlocking copy protection", "Kopierschutz wird entsperrt"),
    ("Finding the movie", "Film wird gesucht"),
    // Menus
    ("Back", "Zurück"),
    ("‹  Back", "‹  Zurück"),
    ("Resume from {time}", "Fortsetzen bei {time}"),
    (
        "Continues where you stopped",
        "Macht dort weiter, wo du aufgehört hast",
    ),
    ("Play from beginning", "Von vorne abspielen"),
    (
        "Starts the video from the start",
        "Startet das Video von Anfang an",
    ),
    (
        "Starts the movie from the start",
        "Startet den Film von Anfang an",
    ),
    ("Play movie", "Film abspielen"),
    (
        "Starts the main movie right away",
        "Startet sofort den Hauptfilm",
    ),
    ("Disc menu", "Disc-Menü"),
    (
        "Starts the disc like a normal player",
        "Startet die Disc wie ein normaler Player",
    ),
    ("Eject disc", "Disc auswerfen"),
    ("Opens the tray", "Öffnet das Laufwerk"),
    // Overlays
    ("Ejecting…", "Wird ausgeworfen …"),
    (
        "The tray opens in a moment.",
        "Das Laufwerk öffnet sich gleich.",
    ),
    ("Stopping…", "Wird beendet …"),
    ("Esc / Back: stop", "Esc / Zurück: beenden"),
    ("Playing…", "Läuft …"),
    ("Something went wrong", "Etwas ist schiefgegangen"),
    (
        "This file could not be played ({status}).",
        "Diese Datei konnte nicht abgespielt werden ({status}).",
    ),
    (
        "The website could not be opened ({status}).",
        "Die Webseite konnte nicht geöffnet werden ({status}).",
    ),
    (
        "The player stopped right away ({status}).\nIs MakeMKV installed and registered? See scripts/setup.sh.",
        "Der Player hat sofort aufgehört ({status}).\nIst MakeMKV installiert und registriert? Siehe scripts/setup.sh.",
    ),
    (
        "Lost track of the player: {error}",
        "Verbindung zum Player verloren: {error}",
    ),
    (
        "Could not start the player ({player}): {error}.\nRun scripts/setup.sh to install it.",
        "Der Player ({player}) konnte nicht gestartet werden: {error}.\nZum Installieren scripts/setup.sh ausführen.",
    ),
    (
        "Could not start the player (mpv): {error}",
        "Der Player (mpv) konnte nicht gestartet werden: {error}",
    ),
    (
        "Could not start the music player (mpv): {error}",
        "Der Musikplayer (mpv) konnte nicht gestartet werden: {error}",
    ),
    (
        "This website can't be opened: {error}",
        "Diese Webseite kann nicht geöffnet werden: {error}",
    ),
    (
        "Could not open the website: {error}",
        "Die Webseite konnte nicht geöffnet werden: {error}",
    ),
    (
        "MakeMKV is not installed (makemkvcon not found). Run scripts/setup.sh.",
        "MakeMKV ist nicht installiert (makemkvcon fehlt). Bitte scripts/setup.sh ausführen.",
    ),
    (
        "MakeMKV found no titles: {detail}",
        "MakeMKV hat keine Titel gefunden: {detail}",
    ),
    (
        "The disc could not be ejected. Is it still in use?",
        "Die Disc konnte nicht ausgeworfen werden. Wird sie noch benutzt?",
    ),
    (
        "The disc could not be ejected (eject: {error}).",
        "Die Disc konnte nicht ausgeworfen werden (eject: {error}).",
    ),
    // Folder browser
    (
        "This folder can't be opened right now",
        "Dieser Ordner kann gerade nicht geöffnet werden",
    ),
    (
        "this folder is outside the shared folder",
        "dieser Ordner liegt außerhalb des freigegebenen Ordners",
    ),
    ("Nothing to show here", "Hier gibt es nichts zu sehen"),
    ("Recently played", "Zuletzt abgespielt"),
    (
        "Enter: open     Esc / Backspace: back",
        "Enter: öffnen     Esc / Rücktaste: zurück",
    ),
    // Pictures and music
    (
        "This picture can't be shown",
        "Dieses Bild kann nicht angezeigt werden",
    ),
    ("Loading…", "Wird geladen …"),
    (
        "{n} / {total}     ← →  ·  Esc: back",
        "{n} / {total}     ← →  ·  Esc: zurück",
    ),
    (
        "{folder}  ·  Track {n} of {total}",
        "{folder}  ·  Titel {n} von {total}",
    ),
    ("Paused", "Pausiert"),
    (
        "Enter: pause   ← →: previous / next   ↑ ↓: ±30 s   Esc: stop",
        "Enter: Pause   ← →: zurück / weiter   ↑ ↓: ±30 s   Esc: beenden",
    ),
    // Home button
    (
        "Keep holding to go to the home screen…",
        "Gedrückt halten, um zum Startbildschirm zu gehen …",
    ),
    // Lock screen and time limits
    ("30 more minutes", "30 Minuten mehr"),
    ("1 more hour", "1 Stunde mehr"),
    ("Rest of today", "Den Rest des Tages"),
    ("Wrong PIN", "Falsche PIN"),
    ("Locked", "Gesperrt"),
    ("Time is up", "Die Zeit ist um"),
    ("Not now", "Gerade nicht"),
    (
        "A parent can unlock it in the admin panel.",
        "Ein Elternteil kann im Admin-Bereich entsperren.",
    ),
    ("Cancel", "Abbrechen"),
    ("Unlock for…", "Entsperren für …"),
    ("Checking…", "Wird geprüft …"),
    (
        "Wrong PIN. Try again in {secs} s",
        "Falsche PIN. Nochmal in {secs} s",
    ),
    ("The PIN has 4 to 8 digits", "Die PIN hat 4 bis 8 Ziffern"),
    ("Locked by a parent", "Von einem Elternteil gesperrt"),
    (
        "The time for today is used up",
        "Die Zeit für heute ist aufgebraucht",
    ),
    ("Available from {time}", "Verfügbar ab {time}"),
    ("Not available any more today", "Heute nicht mehr verfügbar"),
    // Remote setup
    ("Remote setup", "Fernbedienung einrichten"),
    (
        "Saved. Your remote's buttons:",
        "Gespeichert. Die Tasten deiner Fernbedienung:",
    ),
    (
        "Press any button to continue",
        "Beliebige Taste drücken, um weiterzumachen",
    ),
    (
        "Switch the remote's mouse / pointer mode off first.",
        "Schalte zuerst den Maus-/Zeigermodus der Fernbedienung aus.",
    ),
    ("Button {n} of {total}", "Taste {n} von {total}"),
    (
        "Press this button on the remote:",
        "Drücke diese Taste auf der Fernbedienung:",
    ),
    (
        "No such button? It's skipped in {secs} s.",
        "Keine solche Taste? Sie wird in {secs} s übersprungen.",
    ),
    (
        "That button is already {action}. Press another one, or wait to skip.",
        "Diese Taste ist schon „{action}“. Drücke eine andere oder warte zum Überspringen.",
    ),
    ("OK", "OK"),
    ("Back / Return", "Zurück"),
    ("Home", "Home"),
    ("Up", "Hoch"),
    ("Down", "Runter"),
    ("Left", "Links"),
    ("Right", "Rechts"),
    ("Play / Pause", "Wiedergabe / Pause"),
    ("Next", "Weiter"),
    ("Previous", "Zurück (Titel)"),
    ("Volume up", "Lauter"),
    ("Games", "Spiele"),
    ("Block Stacker", "Blockstapler"),
    ("1 player", "1 Person"),
    ("Start level: {n}", "Startlevel: {n}"),
    ("Goal: clear 25 lines", "Ziel: 25 Reihen abräumen"),
    ("Goal: endless", "Ziel: endlos"),
    ("Height: {n}", "Höhe: {n}"),
    ("High scores", "Bestenliste"),
    ("25 lines cleared!", "25 Reihen geschafft!"),
    ("Player 1 wins!", "Spieler 1 gewinnt!"),
    ("Player 2 wins!", "Spieler 2 gewinnt!"),
    ("Player 1 wins the round", "Spieler 1 gewinnt die Runde"),
    ("Player 2 wins the round", "Spieler 2 gewinnt die Runde"),
    ("Nobody wins this round", "Niemand gewinnt diese Runde"),
    ("Player {n}: {wins} of 4", "Spieler {n}: {wins} von 4"),
    ("Next piece", "Nächster Stein"),
    ("Level", "Level"),
    ("Lines", "Reihen"),
    ("New high score: {score}", "Neuer Bestwert: {score}"),
    ("Enter your name", "Gib deinen Namen ein"),
    (
        "Up / down: letter · left / right: position · OK: done",
        "Hoch / runter: Buchstabe · links / rechts: Stelle · OK: fertig",
    ),
    ("No high scores yet", "Noch keine Bestwerte"),
    ("OK or back: continue", "OK oder zurück: weiter"),
    ("Home screens", "Startseiten"),
    ("Shown now", "Wird gerade gezeigt"),
    ("Switch to “{name}”", "Zu „{name}“ wechseln"),
    (
        "Enter the PIN to switch the home screen. Back: cancel.",
        "Zum Wechseln der Startseite die PIN eingeben. Zurück: abbrechen.",
    ),
    ("Brick Breaker", "Mauerbrecher"),
    ("Clear the walls of bricks", "Räume die Mauern ab"),
    ("Play", "Spielen"),
    (
        "Left / right: move · hold a button: faster · best: {score}",
        "Links / rechts: bewegen · Taste halten: schneller · Bestwert: {score}",
    ),
    (
        "Game over · {score} points",
        "Spiel vorbei · {score} Punkte",
    ),
    ("Bonus stage", "Bonusrunde"),
    ("Bonus stage over", "Bonusrunde vorbei"),
    ("Stage {n}", "Runde {n}"),
    ("Stage cleared!", "Runde geschafft!"),
    ("Time: {secs}", "Zeit: {secs}"),
    ("Points", "Punkte"),
    ("Best", "Bestwert"),
    ("Sound: on", "Ton: an"),
    ("Sound: off", "Ton: aus"),
    ("Paddles: {n}", "Schläger: {n}"),
    ("Exit", "Beenden"),
    ("Shut down the computer", "Computer ausschalten"),
    (
        "Tip: hold the power button for 8 seconds",
        "Tipp: den Ein/Aus-Knopf 8 Sekunden gedrückt halten",
    ),
    (
        "Enter the PIN to shut down. Back: cancel.",
        "Zum Ausschalten die PIN eingeben. Zurück: abbrechen.",
    ),
    ("Release now: log out", "Jetzt loslassen: abmelden"),
    (
        "Keep holding: the computer will be shut down",
        "Weiter gedrückt halten: der Computer wird ausgeschaltet",
    ),
    (
        "The computer could not be shut down: {error}",
        "Der Computer konnte nicht ausgeschaltet werden: {error}",
    ),
    ("Exit the media center?", "Mediencenter beenden?"),
    ("Exit the media center", "Mediencenter beenden"),
    (
        "Ends the media center and goes back to the login screen",
        "Beendet das Mediencenter und geht zurück zur Anmeldung",
    ),
    (
        "Enter the PIN to exit. Back: cancel.",
        "Zum Beenden die PIN eingeben. Zurück: abbrechen.",
    ),
    ("Ping Pong", "Ping Pong"),
    ("Game for 1 or 2 players", "Spiel für 1 oder 2 Personen"),
    (
        "1 player (against the computer)",
        "1 Person (gegen den Computer)",
    ),
    ("2 players", "2 Personen"),
    ("1 gamepad connected", "1 Gamepad angeschlossen"),
    ("{n} gamepads connected", "{n} Gamepads angeschlossen"),
    (
        "No gamepad found: use the remote's arrows, or connect a gamepad.",
        "Kein Gamepad gefunden: mit den Pfeiltasten der Fernbedienung spielen oder ein Gamepad anschließen.",
    ),
    ("Continue", "Weiter"),
    ("End the game", "Spiel beenden"),
    ("Play again", "Noch einmal"),
    ("You win!", "Du gewinnst!"),
    ("The computer wins", "Der Computer gewinnt"),
    ("Left wins!", "Links gewinnt!"),
    ("Right wins!", "Rechts gewinnt!"),
    ("Volume down", "Leiser"),
    ("Mute", "Ton aus"),
    ("Muted", "Ton aus"),
    ("Volume {percent} %", "Lautstärke {percent} %"),
    (
        "No volume control found",
        "Keine Lautstärkeregelung gefunden",
    ),
    ("not set up (skipped)", "nicht eingerichtet (übersprungen)"),
    ("{key} key", "Taste {key}"),
    ("{button} click", "{button}-Klick"),
    (
        "{command} via the desktop (media key)",
        "{command} über den Desktop (Medientaste)",
    ),
    ("remote code {code}", "Code {code}"),
    (
        "read from the remote directly",
        "direkt von der Fernbedienung gelesen",
    ),
    // Website window
    (
        "This page is not part of {site}",
        "Diese Seite gehört nicht zu {site}",
    ),
    ("Go back", "Zurück"),
    ("Close website", "Webseite schließen"),
    ("Close", "Schließen"),
    ("Previous page", "Vorige Seite"),
    (
        "Keep holding Esc to close…",
        "Esc gedrückt halten zum Schließen …",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_language() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(Lang::detect_with(env(&[("LANG", "de_DE.UTF-8")])), Lang::De);
        assert_eq!(Lang::detect_with(env(&[("LANG", "de_AT@euro")])), Lang::De);
        assert_eq!(Lang::detect_with(env(&[("LANG", "en_US.UTF-8")])), Lang::En);
        assert_eq!(Lang::detect_with(env(&[("LANG", "fr_FR.UTF-8")])), Lang::En);
        assert_eq!(Lang::detect_with(env(&[])), Lang::En);
        // LC_ALL wins over LANG; LANGUAGE (a list) wins, its first known entry.
        assert_eq!(
            Lang::detect_with(env(&[("LANG", "de_DE.UTF-8"), ("LC_ALL", "en_GB.UTF-8")])),
            Lang::En
        );
        assert_eq!(
            Lang::detect_with(env(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "fr:de:en")])),
            Lang::De
        );
        // "C" switches LANGUAGE off (as in gettext).
        assert_eq!(
            Lang::detect_with(env(&[("LC_ALL", "C"), ("LANGUAGE", "de")])),
            Lang::En
        );
        assert_eq!(Lang::from_setting("de"), Lang::De);
        assert_eq!(Lang::from_setting("en"), Lang::En);
    }

    #[test]
    fn placeholders_are_filled() {
        // The language is per thread in tests.
        set(Lang::De);
        assert_eq!(
            crate::tr!(
                "{folder}  ·  Track {n} of {total}",
                folder = "Hits",
                n = 2,
                total = 9
            ),
            "Hits  ·  Titel 2 von 9"
        );
        assert_eq!(crate::tr!("Not in the table"), "Not in the table");
        set(Lang::En);
        assert_eq!(
            crate::tr!("Button {n} of {total}", n = 1, total = 10),
            "Button 1 of 10"
        );
    }

    /// Every `tr!("…")` in the sources has a German entry with the same placeholders, and
    /// every German entry is used.
    #[test]
    fn every_text_is_translated() {
        // The repository root (also when compiled into web/).
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = if manifest.join("web/src").is_dir() {
            manifest
        } else {
            manifest.parent().unwrap()
        };
        let mut used = std::collections::BTreeSet::new();
        for dir in ["src", "web/src"] {
            let Ok(read) = std::fs::read_dir(root.join(dir)) else {
                continue;
            };
            for file in read.flatten() {
                let path = file.path();
                if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                used.extend(tr_literals(&text));
            }
        }
        let table: HashMap<&str, &str> = DE.iter().copied().collect();
        assert_eq!(table.len(), DE.len(), "duplicate keys in DE");
        let missing: Vec<_> = used
            .iter()
            .filter(|t| !table.contains_key(t.as_str()))
            .collect();
        assert!(missing.is_empty(), "no German text for: {missing:#?}");
        let unused: Vec<_> = DE
            .iter()
            .filter(|(k, _)| !used.contains(*k))
            .map(|(k, _)| k)
            .collect();
        assert!(unused.is_empty(), "German texts nobody uses: {unused:#?}");
        for (en, de) in DE {
            assert_eq!(
                placeholders(en),
                placeholders(de),
                "placeholders differ: {en}"
            );
        }
    }

    /// The string literals right after `tr!(` / `tr_noop!(` (escapes: `\n`, `\"`).
    fn tr_literals(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = source;
        loop {
            // The next `tr!(` / `tr_noop!(` that is a whole name (not `include_str!(`) and
            // starts with a string literal (rustfmt may put it on the next line).
            let whole = |i: usize| !rest[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_');
            let next = ["tr!(", "tr_noop!("]
                .iter()
                .filter_map(|m| {
                    rest.match_indices(m)
                        .find(|(i, _)| {
                            whole(*i) && rest[i + m.len()..].trim_start().starts_with('"')
                        })
                        .map(|(i, _)| i + m.len())
                })
                .min();
            let Some(start) = next else { break };
            rest = rest[start..].trim_start();
            rest = &rest[1..]; // the opening quote
            let mut text = String::new();
            let mut chars = rest.char_indices();
            while let Some((j, c)) = chars.next() {
                match c {
                    '"' => {
                        rest = &rest[j + 1..];
                        break;
                    }
                    '\\' => match chars.next() {
                        Some((_, 'n')) => text.push('\n'),
                        Some((_, other)) => text.push(other),
                        None => {}
                    },
                    c => text.push(c),
                }
            }
            found.push(text);
        }
        found
    }

    fn placeholders(text: &str) -> std::collections::BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .filter_map(|p| p.split_once('}').map(|(name, _)| name))
            .collect()
    }
}
