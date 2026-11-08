//! MPRIS (`org.mpris.MediaPlayer2`) on the D-Bus session bus.
//!
//! Desktops (Cinnamon, GNOME, KDE, …) grab the media keys of keyboards and remotes (next,
//! previous, play/pause, stop) and send them to MPRIS players instead of the focused window.
//! Without an MPRIS player, Linux Mint just shows "unavailable". This is the freedesktop.org
//! standard, so it works the same on every desktop. In the kiosk session (cage) there is no
//! such service; the keys reach the window directly (see `ui::Keys`).

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedValue};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.familynovice";
const PATH: &str = "/org/mpris/MediaPlayer2";

/// A media key, from MPRIS or from the keyboard / remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    Next,
    Previous,
    PlayPause,
    Play,
    Pause,
    Stop,
}

/// What desktops show for us (`PlaybackStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Stopped,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Playing => "Playing",
            Status::Paused => "Paused",
            Status::Stopped => "Stopped",
        }
    }
}

struct Root;

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}
    fn quit(&self) {}
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> String {
        "familynovice".into()
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Player {
    commands: Sender<MediaCommand>,
    notify: Box<dyn Fn() + Send + Sync>,
    status: Arc<Mutex<Status>>,
}

impl Player {
    fn send(&self, command: MediaCommand) {
        log::debug!("MPRIS: {command:?}");
        let _ = self.commands.send(command);
        (self.notify)();
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.send(MediaCommand::Next);
    }
    fn previous(&self) {
        self.send(MediaCommand::Previous);
    }
    fn play_pause(&self) {
        self.send(MediaCommand::PlayPause);
    }
    fn play(&self) {
        self.send(MediaCommand::Play);
    }
    fn pause(&self) {
        self.send(MediaCommand::Pause);
    }
    fn stop(&self) {
        self.send(MediaCommand::Stop);
    }
    // Required by the spec; not supported.
    fn seek(&self, _offset: i64) {}
    fn set_position(&self, _track_id: ObjectPath<'_>, _position: i64) {}
    fn open_uri(&self, _uri: String) {}

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.status
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_str()
            .into()
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        HashMap::new()
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

/// Our MPRIS service. Without a session bus it does nothing (the keys still work when they
/// reach the window).
pub struct Mpris {
    connection: Option<Connection>,
    status: Arc<Mutex<Status>>,
}

impl Mpris {
    /// Register on the session bus. Media keys arrive in `commands`; `notify` wakes the UI.
    pub fn start(
        commands: Sender<MediaCommand>,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self::start_as(BUS_NAME, commands, notify)
    }

    fn start_as(
        bus_name: &str,
        commands: Sender<MediaCommand>,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let status = Arc::new(Mutex::new(Status::Stopped));
        let player = Player {
            commands,
            notify: Box::new(notify),
            status: status.clone(),
        };
        let connection = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.name(bus_name.to_owned()))
            .and_then(|b| b.serve_at(PATH, Root))
            .and_then(|b| b.serve_at(PATH, player))
            .and_then(|b| b.build());
        let connection = match connection {
            Ok(c) => {
                log::info!("media keys: MPRIS player {bus_name}");
                Some(c)
            }
            Err(e) => {
                log::warn!("media keys via the desktop (MPRIS) are not available: {e}");
                None
            }
        };
        Self { connection, status }
    }

    /// Tell the desktop whether something plays (sends `PropertiesChanged` on a change).
    pub fn set_status(&self, status: Status) {
        {
            let mut current = self.status.lock().unwrap_or_else(|p| p.into_inner());
            if *current == status {
                return;
            }
            *current = status;
        }
        let Some(connection) = &self.connection else {
            return;
        };
        let result = connection
            .object_server()
            .interface::<_, Player>(PATH)
            .and_then(|iface| {
                zbus::block_on(iface.get().playback_status_changed(iface.signal_emitter()))
            });
        if let Err(e) = result {
            log::debug!("MPRIS status update: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Needs a D-Bus session bus (skipped without one, e.g. in a bare CI container).
    #[test]
    fn media_keys_arrive_over_dbus() {
        let (tx, rx) = mpsc::channel();
        // Own name: the real one may belong to a running media center.
        let name = format!(
            "org.mpris.MediaPlayer2.familynovice_test{}",
            std::process::id()
        );
        let mpris = Mpris::start_as(&name, tx, || {});
        let Some(_) = &mpris.connection else {
            eprintln!("no session bus: skipped");
            return;
        };
        let client = Connection::session().unwrap();
        for method in ["Next", "Previous", "PlayPause", "Stop"] {
            client
                .call_method(
                    Some(name.as_str()),
                    PATH,
                    Some("org.mpris.MediaPlayer2.Player"),
                    method,
                    &(),
                )
                .unwrap();
        }
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            got,
            [
                MediaCommand::Next,
                MediaCommand::Previous,
                MediaCommand::PlayPause,
                MediaCommand::Stop
            ]
        );
        mpris.set_status(Status::Playing);
        let reply = client
            .call_method(
                Some(name.as_str()),
                PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.mpris.MediaPlayer2.Player", "PlaybackStatus"),
            )
            .unwrap();
        let value: OwnedValue = reply.body().deserialize().unwrap();
        assert_eq!(String::try_from(value).unwrap(), "Playing");
    }
}
