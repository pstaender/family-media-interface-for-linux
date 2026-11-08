//! Keep the computer from blanking the screen or switching the signal off while curator
//! has its own screensaver: a TV that loses the signal shows "no signal" or goes to standby.
//!
//! Standard session D-Bus interfaces, tried in order (both stay held while the connection
//! is open and are released with their cookie):
//! 1. `org.freedesktop.ScreenSaver.Inhibit` (KDE, GNOME, Xfce, Cinnamon, …),
//! 2. `org.gnome.SessionManager.Inhibit` with the "idle" flag (GNOME and Cinnamon sessions).
//!
//! The kiosk session (cage) never blanks by itself; there neither service exists, which is
//! fine. The TV's own timers (e.g. "auto power off after 4 h without remote input") can't be
//! reached from here; that would need HDMI-CEC.

use std::sync::mpsc::{self, Receiver, Sender};

use zbus::blocking::Connection;

const APP: &str = "curator";
const REASON: &str = "curator shows its own screensaver";

pub struct Inhibitor {
    tx: Sender<bool>,
    wanted: Option<bool>,
}

impl Inhibitor {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        if let Err(e) = std::thread::Builder::new()
            .name("inhibit".into())
            .spawn(move || worker(&rx))
        {
            log::warn!("inhibit: no worker thread: {e}");
        }
        Self { tx, wanted: None }
    }

    /// Hold (true) or release (false) the inhibition; cheap to call every frame.
    pub fn set(&mut self, on: bool) {
        if self.wanted != Some(on) {
            self.wanted = Some(on);
            let _ = self.tx.send(on);
        }
    }
}

enum Held {
    ScreenSaver(u32),
    SessionManager(u32),
}

fn worker(rx: &Receiver<bool>) {
    let mut connection: Option<Connection> = None;
    let mut held: Option<Held> = None;
    while let Ok(mut on) = rx.recv() {
        on = rx.try_iter().last().unwrap_or(on);
        if on == held.is_some() {
            continue;
        }
        if connection.is_none() {
            match Connection::session() {
                Ok(c) => connection = Some(c),
                Err(e) => {
                    log::info!("inhibit: no session bus ({e}); the screen may blank");
                    continue;
                }
            }
        }
        let Some(conn) = &connection else { continue };
        if on {
            held = inhibit(conn);
        } else if let Some(h) = held.take() {
            release(conn, h);
        }
    }
}

fn inhibit(conn: &Connection) -> Option<Held> {
    let freedesktop = conn
        .call_method(
            Some("org.freedesktop.ScreenSaver"),
            "/org/freedesktop/ScreenSaver",
            Some("org.freedesktop.ScreenSaver"),
            "Inhibit",
            &(APP, REASON),
        )
        .and_then(|m| m.body().deserialize::<u32>());
    match freedesktop {
        Ok(cookie) => {
            log::info!("inhibit: screen blanking off (org.freedesktop.ScreenSaver)");
            return Some(Held::ScreenSaver(cookie));
        }
        Err(e) => log::debug!("inhibit: org.freedesktop.ScreenSaver: {e}"),
    }
    // Flags: 8 = idle (the session doesn't go idle: no blanking, no screen lock).
    let gnome = conn
        .call_method(
            Some("org.gnome.SessionManager"),
            "/org/gnome/SessionManager",
            Some("org.gnome.SessionManager"),
            "Inhibit",
            &(APP, 0u32, REASON, 8u32),
        )
        .and_then(|m| m.body().deserialize::<u32>());
    match gnome {
        Ok(cookie) => {
            log::info!("inhibit: screen blanking off (org.gnome.SessionManager)");
            Some(Held::SessionManager(cookie))
        }
        Err(e) => {
            log::info!("inhibit: no screensaver service to ask ({e}); fine in the kiosk session");
            None
        }
    }
}

fn release(conn: &Connection, held: Held) {
    let result = match held {
        Held::ScreenSaver(cookie) => conn.call_method(
            Some("org.freedesktop.ScreenSaver"),
            "/org/freedesktop/ScreenSaver",
            Some("org.freedesktop.ScreenSaver"),
            "UnInhibit",
            &(cookie,),
        ),
        Held::SessionManager(cookie) => conn.call_method(
            Some("org.gnome.SessionManager"),
            "/org/gnome/SessionManager",
            Some("org.gnome.SessionManager"),
            "Uninhibit",
            &(cookie,),
        ),
    };
    match result {
        Ok(_) => log::info!("inhibit: screen blanking allowed again"),
        Err(e) => log::warn!("inhibit: release failed: {e}"),
    }
}
