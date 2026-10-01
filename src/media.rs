//! Desktop media keys. Omarchy grabs the keyboard's play and skip buttons and
//! sends them to an MPRIS player, so this process registers as one.

use std::sync::mpsc;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Play,
    Stop,
    Toggle,
    Next,
    Previous,
}

pub struct MediaKeys {
    controls: MediaControls,
    events: mpsc::Receiver<MediaControlEvent>,
    title: String,
    artist: String,
    playing: bool,
}

impl MediaKeys {
    pub fn start() -> Option<Self> {
        let (tx, events) = mpsc::channel();
        let mut controls = MediaControls::new(PlatformConfig {
            dbus_name: "radio_tui",
            display_name: "radio-tui",
            hwnd: None,
        })
        .ok()?;
        controls
            .attach(move |event| {
                let _ = tx.send(event);
            })
            .ok()?;
        Some(Self {
            controls,
            events,
            title: String::new(),
            artist: String::new(),
            playing: false,
        })
    }

    pub fn poll(&self) -> Vec<MediaAction> {
        let mut actions = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            let action = match event {
                MediaControlEvent::Play => Some(MediaAction::Play),
                MediaControlEvent::Pause | MediaControlEvent::Stop => Some(MediaAction::Stop),
                MediaControlEvent::Toggle => Some(MediaAction::Toggle),
                MediaControlEvent::Next => Some(MediaAction::Next),
                MediaControlEvent::Previous => Some(MediaAction::Previous),
                _ => None,
            };
            if let Some(action) = action {
                actions.push(action);
            }
        }
        actions
    }

    pub fn sync(&mut self, title: &str, artist: &str, playing: bool) {
        if self.title == title && self.artist == artist && self.playing == playing {
            return;
        }
        self.title = title.to_string();
        self.artist = artist.to_string();
        self.playing = playing;
        let playback = if playing {
            MediaPlayback::Playing { progress: None }
        } else {
            MediaPlayback::Stopped
        };
        let _ = self.controls.set_playback(playback);
        let _ = self.controls.set_metadata(MediaMetadata {
            title: Some(title),
            artist: if artist.is_empty() {
                None
            } else {
                Some(artist)
            },
            album: Some("Internet radio"),
            ..Default::default()
        });
    }
}
