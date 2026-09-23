//! Station list, keyboard actions, and mpv playback.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::config::{
    clamp_volume, default_config, fetch_remote_stations, load_config, save_config,
    selected_new_stations, valid_list_url, valid_stream_url, Config, LoadError, Station,
};
use crate::visualizer::Visualizer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Help,
    Edit,
    NowPlaying,
    AddName {
        buffer: String,
        cursor: usize,
    },
    AddUrl {
        name: String,
        buffer: String,
        cursor: usize,
    },
    EditUrl {
        index: usize,
        buffer: String,
        cursor: usize,
    },
    ImportUrl {
        buffer: String,
        cursor: usize,
    },
    ImportPick,
}

pub struct ImportPick {
    pub source: String,
    pub stations: Vec<Station>,
    pub chosen: Vec<bool>,
    pub already: Vec<bool>,
    pub list_state: ListState,
}

pub const NOW_PLAYING_AFTER: Duration = Duration::from_secs(60);

pub struct App {
    pub stations: Vec<Station>,
    pub selected: usize,
    pub volume: f32,
    pub status: String,
    pub mode: Mode,
    pub list_state: ListState,
    save_enabled: bool,
    mpv: Option<Child>,
    ipc_path: Option<PathBuf>,
    playing_url: Option<String>,
    visualizer: Visualizer,
    ipc_request_id: i64,
    playing_since: Option<Instant>,
    last_input: Instant,
    last_metadata_at: Option<Instant>,
    track_title: Option<String>,
    icy_name: Option<String>,
    pub import: Option<ImportPick>,
}

fn byte_index(text: &str, cursor: usize) -> usize {
    text.char_indices()
        .nth(cursor)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

fn insert_at_cursor(buffer: &mut String, cursor: &mut usize, text: &str) {
    let at = byte_index(buffer, *cursor);
    buffer.insert_str(at, text);
    *cursor += text.chars().count();
}

impl App {
    #[cfg(test)]
    pub fn fixture() -> Self {
        let mut app = Self {
            stations: vec![Station {
                name: "BBC World Service".to_string(),
                url: "https://example.com/bbc".to_string(),
                favorite: true,
            }],
            selected: 0,
            volume: 70.0,
            status: "Ready".to_string(),
            mode: Mode::Normal,
            list_state: ListState::default(),
            save_enabled: false,
            mpv: None,
            ipc_path: None,
            playing_url: None,
            visualizer: Visualizer::new(),
            ipc_request_id: 1,
            playing_since: None,
            last_input: Instant::now(),
            last_metadata_at: None,
            track_title: None,
            icy_name: None,
            import: None,
        };
        app.sync_list_state();
        app
    }

    pub fn new() -> Self {
        let (stations, volume, status, should_save) = match load_config() {
            Ok(config) => {
                let status = if config.stations.is_empty() {
                    "No stations saved. Press a to add one.".to_string()
                } else {
                    "Ready".to_string()
                };
                (config.stations, config.volume, status, false)
            }
            Err(LoadError::NotFound) => {
                let config = default_config();
                (
                    config.stations,
                    config.volume,
                    "Created default station list".to_string(),
                    true,
                )
            }
            Err(err) => {
                let config = default_config();
                (
                    config.stations,
                    config.volume,
                    format!("{err} — using defaults, original file left untouched"),
                    false,
                )
            }
        };

        let mut app = Self {
            selected: 0,
            volume: clamp_volume(volume),
            status,
            mode: Mode::Normal,
            list_state: ListState::default(),
            save_enabled: true,
            mpv: None,
            ipc_path: None,
            playing_url: None,
            visualizer: Visualizer::new(),
            ipc_request_id: 1,
            playing_since: None,
            last_input: Instant::now(),
            last_metadata_at: None,
            track_title: None,
            icy_name: None,
            import: None,
            stations,
        };
        app.sync_list_state();
        if should_save {
            app.persist();
        }
        app
    }

    pub fn is_playing(&self) -> bool {
        self.mpv.is_some() && self.playing_url.is_some()
    }

    pub fn playing_index(&self) -> Option<usize> {
        let url = self.playing_url.as_ref()?;
        self.stations.iter().position(|station| &station.url == url)
    }

    pub fn persist(&mut self) {
        if !self.save_enabled {
            return;
        }
        let config = Config {
            stations: self.stations.clone(),
            volume: self.volume,
        };
        if let Err(err) = save_config(&config) {
            self.status = format!("Failed to save config: {err}");
        }
    }

    pub fn reap(&mut self) {
        let Some(child) = self.mpv.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.mpv = None;
                self.playing_url = None;
                self.cleanup_ipc();
                self.visualizer.reset();
                self.clear_playback_state();
                self.status = if status.success() {
                    "Playback ended".to_string()
                } else {
                    "Playback stopped (mpv exited)".to_string()
                };
            }
            Ok(None) => {}
            Err(err) => {
                self.mpv = None;
                self.playing_url = None;
                self.cleanup_ipc();
                self.visualizer.reset();
                self.clear_playback_state();
                self.status = format!("Lost mpv process: {err}");
            }
        }
    }

    pub fn play_selected(&mut self) {
        let Some(station) = self.stations.get(self.selected).cloned() else {
            self.status = "No station selected".to_string();
            return;
        };
        self.start_station(&station);
    }

    pub fn toggle_playback(&mut self) {
        if self.is_playing() {
            self.stop();
            self.status = "Stopped".to_string();
        } else {
            self.play_selected();
        }
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.mpv.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.playing_url = None;
        self.visualizer.reset();
        self.cleanup_ipc();
        self.clear_playback_state();
    }

    pub fn tick(&mut self) {
        self.reap();
        self.update_visualizer();
        self.update_metadata();
        self.maybe_enter_now_playing();
    }

    pub fn track_title(&self) -> Option<&str> {
        self.track_title.as_deref()
    }

    pub fn icy_name(&self) -> Option<&str> {
        self.icy_name.as_deref()
    }

    pub fn playing_station_name(&self) -> &str {
        self.playing_index()
            .and_then(|i| self.stations.get(i))
            .map(|s| s.name.as_str())
            .unwrap_or("radio-tui")
    }

    pub fn eq_energy(&self) -> f32 {
        self.visualizer.energy()
    }

    pub fn eq_tick(&self) -> u32 {
        self.visualizer.tick()
    }

    fn clear_playback_state(&mut self) {
        self.playing_since = None;
        self.last_metadata_at = None;
        self.track_title = None;
        self.icy_name = None;
        if self.mode == Mode::NowPlaying {
            self.mode = Mode::Normal;
        }
    }

    fn maybe_enter_now_playing(&mut self) {
        if self.is_playing() && self.ready_for_now_playing(Instant::now()) {
            self.mode = Mode::NowPlaying;
        }
    }

    fn ready_for_now_playing(&self, now: Instant) -> bool {
        self.mode == Mode::Normal
            && self
                .playing_since
                .is_some_and(|started| now.saturating_duration_since(started) >= NOW_PLAYING_AFTER)
            && now.saturating_duration_since(self.last_input) >= NOW_PLAYING_AFTER
    }

    fn update_metadata(&mut self) {
        if !self.is_playing() {
            return;
        }
        let now = Instant::now();
        if self
            .last_metadata_at
            .is_some_and(|at| now.duration_since(at) < Duration::from_millis(800))
        {
            return;
        }
        self.last_metadata_at = Some(now);
        if let Some(data) = self.mpv_get_property("metadata") {
            let (title, name) = parse_stream_metadata(&data);
            if title.is_some() {
                self.track_title = title;
            }
            if name.is_some() {
                self.icy_name = name;
            }
        }
        if self.track_title.is_none() {
            if let Some(value) = self.mpv_get_property("media-title") {
                self.track_title = title_from_value(&value);
            }
        }
    }

    pub fn update_visualizer(&mut self) {
        if !self.is_playing() {
            self.visualizer.decay();
            return;
        }
        let Some(data) = self.mpv_get_property("af-metadata/vu") else {
            self.visualizer.decay();
            return;
        };
        match crate::visualizer::parse_af_metadata(&data) {
            Some(levels) => self.visualizer.push_levels(levels),
            None => self.visualizer.decay(),
        }
    }

    pub fn eq_bars(&self) -> &[f32] {
        self.visualizer.bars()
    }

    pub fn adjust_volume(&mut self, delta: f32) {
        let next = clamp_volume(self.volume + delta);
        if (next - self.volume).abs() < f32::EPSILON {
            return;
        }
        self.volume = next;
        self.send_mpv(&format!(
            "{{\"command\":[\"set_property\",\"volume\",{}]}}\n",
            self.volume
        ));
        self.status = format!("Volume: {:.0}%", self.volume);
        self.persist();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        self.last_input = Instant::now();

        match self.mode.clone() {
            Mode::NowPlaying => {
                return self.handle_now_playing_key(key);
            }
            Mode::Help => {
                self.mode = Mode::Normal;
            }
            Mode::Edit => self.handle_edit_key(key),
            Mode::EditUrl {
                index,
                mut buffer,
                mut cursor,
            } => {
                if self.handle_input_key(key, &mut buffer, &mut cursor) {
                    match key.code {
                        KeyCode::Esc => {
                            self.mode = Mode::Edit;
                            self.status = "URL edit cancelled".to_string();
                        }
                        KeyCode::Enter => self.finish_edit_url(index, buffer),
                        _ => {
                            self.mode = Mode::EditUrl {
                                index,
                                buffer,
                                cursor,
                            }
                        }
                    }
                } else {
                    self.mode = Mode::EditUrl {
                        index,
                        buffer,
                        cursor,
                    };
                }
            }
            Mode::ImportUrl {
                mut buffer,
                mut cursor,
            } => {
                if self.handle_input_key(key, &mut buffer, &mut cursor) {
                    match key.code {
                        KeyCode::Esc => {
                            self.mode = Mode::Edit;
                            self.status = "Import cancelled".to_string();
                        }
                        KeyCode::Enter => self.start_import(buffer),
                        _ => self.mode = Mode::ImportUrl { buffer, cursor },
                    }
                } else {
                    self.mode = Mode::ImportUrl { buffer, cursor };
                }
            }
            Mode::ImportPick => self.handle_import_pick_key(key),
            Mode::AddName {
                mut buffer,
                mut cursor,
            } => {
                if self.handle_input_key(key, &mut buffer, &mut cursor) {
                    match key.code {
                        KeyCode::Esc => {
                            self.mode = Mode::Normal;
                            self.status = "Add cancelled".to_string();
                        }
                        KeyCode::Enter => {
                            let name = buffer.trim().to_string();
                            if name.is_empty() {
                                self.status = "Name cannot be empty".to_string();
                                self.mode = Mode::AddName {
                                    buffer: String::new(),
                                    cursor: 0,
                                };
                            } else {
                                self.mode = Mode::AddUrl {
                                    name,
                                    buffer: String::new(),
                                    cursor: 0,
                                };
                                self.status =
                                    "Paste or type the stream URL, then Enter".to_string();
                            }
                        }
                        _ => self.mode = Mode::AddName { buffer, cursor },
                    }
                } else {
                    self.mode = Mode::AddName { buffer, cursor };
                }
            }
            Mode::AddUrl {
                name,
                mut buffer,
                mut cursor,
            } => {
                if self.handle_input_key(key, &mut buffer, &mut cursor) {
                    match key.code {
                        KeyCode::Esc => {
                            self.mode = Mode::Normal;
                            self.status = "Add cancelled".to_string();
                        }
                        KeyCode::Enter => {
                            let url = buffer.trim().to_string();
                            self.finish_add(name, url);
                        }
                        _ => {
                            self.mode = Mode::AddUrl {
                                name,
                                buffer,
                                cursor,
                            }
                        }
                    }
                } else {
                    self.mode = Mode::AddUrl {
                        name,
                        buffer,
                        cursor,
                    };
                }
            }
            Mode::Normal => {
                if self.handle_normal_key(key) {
                    return true;
                }
            }
        }
        false
    }

    pub fn handle_paste(&mut self, text: String) {
        let text = text.replace(['\n', '\r'], "");
        match &mut self.mode {
            Mode::AddName { buffer, cursor }
            | Mode::AddUrl { buffer, cursor, .. }
            | Mode::EditUrl { buffer, cursor, .. }
            | Mode::ImportUrl { buffer, cursor } => {
                insert_at_cursor(buffer, cursor, &text);
            }
            _ => {}
        }
    }

    pub fn input_field(&self) -> Option<(&str, &str, usize)> {
        match &self.mode {
            Mode::AddName { buffer, cursor } => Some(("Station name", buffer.as_str(), *cursor)),
            Mode::AddUrl { buffer, cursor, .. } => Some(("Stream URL", buffer.as_str(), *cursor)),
            Mode::EditUrl { buffer, cursor, .. } => Some(("Stream URL", buffer.as_str(), *cursor)),
            Mode::ImportUrl { buffer, cursor } => {
                Some(("Remote station list URL", buffer.as_str(), *cursor))
            }
            _ => None,
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return true,
            KeyCode::Down | KeyCode::Char('j') => {
                self.select_delta(1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.select_delta(-1);
            }
            KeyCode::Enter => self.play_selected(),
            KeyCode::Char(' ') => self.toggle_playback(),
            KeyCode::Char('+') | KeyCode::Char('=') => self.adjust_volume(5.0),
            KeyCode::Char('-') | KeyCode::Char('_') => self.adjust_volume(-5.0),
            KeyCode::Char('a') => {
                self.mode = Mode::AddName {
                    buffer: String::new(),
                    cursor: 0,
                };
                self.status = "Type a station name, then Enter".to_string();
            }
            KeyCode::Char('e') => {
                self.mode = Mode::Edit;
                self.status = "Edit mode".to_string();
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
            }
            KeyCode::Char('f') => {
                if self.is_playing() {
                    self.mode = Mode::NowPlaying;
                } else {
                    self.status = "Play a station first, then press f".to_string();
                }
            }
            KeyCode::Char('*') => self.toggle_favorite(),
            _ => {}
        }
        false
    }

    fn handle_now_playing_key(&mut self, _key: KeyEvent) -> bool {
        self.mode = Mode::Normal;
        self.status = "Ready".to_string();
        false
    }

    fn handle_edit_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
                self.status = "Ready".to_string();
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_delta(1),
            KeyCode::Up | KeyCode::Char('k') => self.select_delta(-1),
            KeyCode::Char('J') => self.move_selected(1),
            KeyCode::Char('K') => self.move_selected(-1),
            KeyCode::Char('*') => self.toggle_favorite(),
            KeyCode::Char('d') | KeyCode::Char('x') | KeyCode::Delete => self.delete_selected(),
            KeyCode::Enter | KeyCode::Char(' ') => self.play_selected(),
            KeyCode::Char('u') => self.begin_edit_url(),
            KeyCode::Char('i') => {
                self.mode = Mode::ImportUrl {
                    buffer: String::new(),
                    cursor: 0,
                };
                self.status = "Paste a stations.json URL, then Enter".to_string();
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
    }

    fn start_import(&mut self, url: String) {
        let url = url.trim().to_string();
        if !valid_list_url(&url) {
            self.status = "List URL must start with http:// or https://".to_string();
            self.mode = Mode::ImportUrl {
                buffer: url.clone(),
                cursor: url.chars().count(),
            };
            return;
        }
        match fetch_remote_stations(&url) {
            Ok(stations) => {
                let already: Vec<bool> = stations
                    .iter()
                    .map(|station| self.stations.iter().any(|local| local.url == station.url))
                    .collect();
                let chosen: Vec<bool> = already.iter().map(|have| !have).collect();
                let mut list_state = ListState::default();
                if !stations.is_empty() {
                    list_state.select(Some(0));
                }
                let new_count = chosen.iter().filter(|pick| **pick).count();
                self.import = Some(ImportPick {
                    source: url,
                    stations,
                    chosen,
                    already,
                    list_state,
                });
                self.mode = Mode::ImportPick;
                self.status = if new_count == 0 {
                    "Every station in that list is already saved".to_string()
                } else {
                    format!("{new_count} new • Space toggle • Enter Do it!")
                };
            }
            Err(err) => {
                self.status = format!("Import failed: {err}");
                self.mode = Mode::ImportUrl {
                    buffer: url.clone(),
                    cursor: url.chars().count(),
                };
            }
        }
    }

    fn handle_import_pick_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.import = None;
                self.mode = Mode::Edit;
                self.status = "Import cancelled".to_string();
            }
            KeyCode::Down | KeyCode::Char('j') => self.import_delta(1),
            KeyCode::Up | KeyCode::Char('k') => self.import_delta(-1),
            KeyCode::Char(' ') => self.toggle_import_choice(),
            KeyCode::Char('a') => self.set_import_choices(true),
            KeyCode::Char('n') => self.set_import_choices(false),
            KeyCode::Enter | KeyCode::Char('d') => self.apply_import(),
            _ => {}
        }
    }

    fn import_delta(&mut self, delta: isize) {
        let Some(import) = self.import.as_mut() else {
            return;
        };
        let len = import.stations.len();
        if len == 0 {
            return;
        }
        let current = import.list_state.selected().unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(len as isize) as usize;
        import.list_state.select(Some(next));
    }

    fn toggle_import_choice(&mut self) {
        let outcome = {
            let Some(import) = self.import.as_mut() else {
                return;
            };
            let Some(index) = import.list_state.selected() else {
                return;
            };
            if import.already.get(index).copied().unwrap_or(false) {
                None
            } else {
                if let Some(flag) = import.chosen.get_mut(index) {
                    *flag = !*flag;
                }
                Some(import.chosen.iter().filter(|pick| **pick).count())
            }
        };
        self.status = match outcome {
            None => "Already in your list".to_string(),
            Some(n) => format!("{n} selected • Enter Do it!"),
        };
    }

    fn set_import_choices(&mut self, value: bool) {
        let n = {
            let Some(import) = self.import.as_mut() else {
                return;
            };
            for (i, flag) in import.chosen.iter_mut().enumerate() {
                *flag = value && !import.already.get(i).copied().unwrap_or(false);
            }
            import.chosen.iter().filter(|pick| **pick).count()
        };
        self.status = if value {
            format!("{n} selected • Enter Do it!")
        } else {
            "None selected".to_string()
        };
    }

    fn apply_import(&mut self) {
        let Some(import) = self.import.take() else {
            self.mode = Mode::Edit;
            return;
        };
        let added = selected_new_stations(&self.stations, &import.stations, &import.chosen);
        if added.is_empty() {
            self.mode = Mode::Edit;
            self.status = "Nothing new to add".to_string();
            return;
        }
        let count = added.len();
        self.stations.extend(added);
        self.selected = self.stations.len() - 1;
        self.sync_list_state();
        self.mode = Mode::Edit;
        self.status = format!("Do it! Added {count} station(s)");
        self.persist();
    }

    fn handle_input_key(&mut self, key: KeyEvent, buffer: &mut String, cursor: &mut usize) -> bool {
        let len = buffer.chars().count();
        *cursor = (*cursor).min(len);
        match key.code {
            KeyCode::Esc | KeyCode::Enter => true,
            KeyCode::Left => {
                *cursor = cursor.saturating_sub(1);
                false
            }
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                *cursor = cursor.saturating_sub(1);
                false
            }
            KeyCode::Right => {
                if *cursor < len {
                    *cursor += 1;
                }
                false
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if *cursor < len {
                    *cursor += 1;
                }
                false
            }
            KeyCode::Home | KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                *cursor = 0;
                false
            }
            KeyCode::End | KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                *cursor = len;
                false
            }
            KeyCode::Backspace => {
                if *cursor > 0 {
                    let remove_at = *cursor - 1;
                    buffer.replace_range(
                        byte_index(buffer, remove_at)..byte_index(buffer, *cursor),
                        "",
                    );
                    *cursor = remove_at;
                }
                false
            }
            KeyCode::Delete => {
                if *cursor < len {
                    buffer.replace_range(
                        byte_index(buffer, *cursor)..byte_index(buffer, *cursor + 1),
                        "",
                    );
                }
                false
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                insert_at_cursor(buffer, cursor, &ch.to_string());
                false
            }
            _ => false,
        }
    }

    fn begin_edit_url(&mut self) {
        let Some(station) = self.stations.get(self.selected) else {
            self.status = "No station selected".to_string();
            return;
        };
        let buffer = station.url.clone();
        let cursor = buffer.chars().count();
        self.mode = Mode::EditUrl {
            index: self.selected,
            buffer,
            cursor,
        };
        self.status = "Fix the URL, then Enter".to_string();
    }

    fn finish_edit_url(&mut self, index: usize, url: String) {
        let url = url.trim().to_string();
        if !valid_stream_url(&url) {
            self.status = "URL must start with http:// or https://".to_string();
            self.mode = Mode::EditUrl {
                index,
                cursor: url.chars().count(),
                buffer: url,
            };
            return;
        }
        let Some(station) = self.stations.get(index) else {
            self.mode = Mode::Edit;
            self.status = "That station is gone".to_string();
            return;
        };
        if station.url == url {
            self.mode = Mode::Edit;
            self.status = "URL unchanged".to_string();
            return;
        }
        if self
            .stations
            .iter()
            .enumerate()
            .any(|(i, other)| i != index && other.url == url)
        {
            self.status = "That stream is already in the list".to_string();
            self.mode = Mode::EditUrl {
                index,
                cursor: url.chars().count(),
                buffer: url,
            };
            return;
        }
        let was_playing = self.playing_url.as_deref() == Some(station.url.as_str());
        let name = station.name.clone();
        self.stations[index].url = url.clone();
        if was_playing {
            self.playing_url = Some(url);
        }
        self.mode = Mode::Edit;
        self.status = format!("Updated URL for {name}");
        self.persist();
    }

    fn finish_add(&mut self, name: String, url: String) {
        if !valid_stream_url(&url) {
            self.status = "URL must start with http:// or https://".to_string();
            self.mode = Mode::AddUrl {
                name,
                cursor: url.chars().count(),
                buffer: url,
            };
            return;
        }
        if self.stations.iter().any(|station| station.url == url) {
            self.status = "That stream is already in the list".to_string();
            self.mode = Mode::Normal;
            return;
        }
        self.stations.push(Station {
            name: name.clone(),
            url,
            favorite: false,
        });
        self.selected = self.stations.len() - 1;
        self.sync_list_state();
        self.mode = Mode::Normal;
        self.status = format!("Added {name}");
        self.persist();
    }

    fn select_delta(&mut self, delta: isize) {
        let len = self.stations.len();
        if len == 0 {
            self.selected = 0;
            self.sync_list_state();
            return;
        }
        let next = (self.selected as isize + delta).rem_euclid(len as isize) as usize;
        self.selected = next;
        self.sync_list_state();
        if let Some(station) = self.stations.get(self.selected) {
            self.status = format!("Selected: {}", station.name);
        }
    }

    fn toggle_favorite(&mut self) {
        let Some(index) = self.list_state.selected() else {
            self.status = "No station selected".to_string();
            return;
        };
        if index >= self.stations.len() {
            return;
        }
        let was_favorite = self.stations[index].favorite;
        let mut station = self.stations.remove(index);
        station.favorite = !was_favorite;
        let insert_at = if station.favorite {
            0
        } else {
            self.stations
                .iter()
                .position(|item| !item.favorite)
                .unwrap_or(self.stations.len())
        };
        let name = station.name.clone();
        self.stations.insert(insert_at, station);
        self.selected = insert_at;
        self.sync_list_state();
        self.status = if was_favorite {
            format!("Unfavorited {name}")
        } else {
            format!("Favorited {name}")
        };
        self.persist();
    }

    fn move_selected(&mut self, delta: isize) {
        let len = self.stations.len();
        if len < 2 {
            return;
        }
        let from = self.selected;
        let to = (from as isize + delta).rem_euclid(len as isize) as usize;
        self.stations.swap(from, to);
        self.selected = to;
        self.sync_list_state();
        self.status = "Reordered stations".to_string();
        self.persist();
    }

    fn delete_selected(&mut self) {
        if self.stations.is_empty() {
            self.status = "No stations to delete".to_string();
            return;
        }
        let station = self.stations.remove(self.selected);
        if self.playing_url.as_deref() == Some(station.url.as_str()) {
            self.stop();
        }
        if self.selected >= self.stations.len() && self.selected > 0 {
            self.selected -= 1;
        }
        self.sync_list_state();
        self.status = format!("Removed {}", station.name);
        self.persist();
    }

    fn sync_list_state(&mut self) {
        if self.stations.is_empty() {
            self.selected = 0;
            self.list_state.select(None);
            return;
        }
        self.selected = self.selected.min(self.stations.len() - 1);
        self.list_state.select(Some(self.selected));
    }

    fn start_station(&mut self, station: &Station) {
        self.stop();
        let ipc = ipc_socket_path();
        let _ = fs::remove_file(&ipc);
        let volume = self.volume.round().clamp(0.0, 100.0) as u8;

        match Command::new("mpv")
            .args([
                "--no-video",
                "--really-quiet",
                "--no-input-terminal",
                "--idle=no",
                "--af=@vu:astats=metadata=1:reset=1:measure_overall=RMS_level+Peak_level:measure_perchannel=RMS_level+Peak_level+Crest_factor+Zero_crossings_rate",
                &format!("--volume={volume}"),
                &format!("--input-ipc-server={}", ipc.display()),
                "--",
                &station.url,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                self.mpv = Some(child);
                self.ipc_path = Some(ipc);
                self.playing_url = Some(station.url.clone());
                self.playing_since = Some(Instant::now());
                self.last_input = Instant::now();
                self.last_metadata_at = None;
                self.track_title = None;
                self.icy_name = None;
                self.status = format!("Playing: {}", station.name);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.status = "Failed to start mpv — is it installed?".to_string();
            }
            Err(err) => {
                self.status = format!("Failed to start mpv: {err}");
            }
        }
    }

    fn send_mpv(&self, payload: &str) {
        let Some(path) = &self.ipc_path else {
            return;
        };
        if let Ok(mut stream) = UnixStream::connect(path) {
            let _ = stream.write_all(payload.as_bytes());
        }
    }

    fn mpv_get_property(&mut self, name: &str) -> Option<serde_json::Value> {
        let path = self.ipc_path.as_ref()?;
        let mut stream = UnixStream::connect(path).ok()?;
        stream
            .set_read_timeout(Some(Duration::from_millis(20)))
            .ok()?;
        stream
            .set_write_timeout(Some(Duration::from_millis(20)))
            .ok()?;
        self.ipc_request_id = self.ipc_request_id.wrapping_add(1);
        if self.ipc_request_id <= 0 {
            self.ipc_request_id = 1;
        }
        let request_id = self.ipc_request_id;
        let payload = serde_json::json!({
            "command": ["get_property", name],
            "request_id": request_id,
        });
        stream.write_all(format!("{payload}\n").as_bytes()).ok()?;
        let mut reader = BufReader::new(stream);
        let deadline = Instant::now() + Duration::from_millis(40);
        for _ in 0..8 {
            if Instant::now() >= deadline {
                break;
            }
            let mut line = String::new();
            if reader.read_line(&mut line).ok()? == 0 {
                break;
            }
            let value: serde_json::Value = serde_json::from_str(&line).ok()?;
            if value.get("request_id").and_then(|id| id.as_i64()) != Some(request_id) {
                continue;
            }
            if value.get("error").and_then(|err| err.as_str()) == Some("success") {
                return value.get("data").cloned();
            }
            return None;
        }
        None
    }

    fn cleanup_ipc(&mut self) {
        if let Some(path) = self.ipc_path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}

fn ipc_socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("radio-tui-{}.sock", std::process::id()))
}

fn parse_stream_metadata(data: &serde_json::Value) -> (Option<String>, Option<String>) {
    let Some(map) = data.as_object() else {
        return (None, None);
    };
    (
        string_entry(map, "icy-title").or_else(|| string_entry(map, "title")),
        string_entry(map, "icy-name"),
    )
}

fn string_entry(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match map.get(key)? {
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

fn title_from_value(value: &serde_json::Value) -> Option<String> {
    let serde_json::Value::String(text) = value else {
        return None;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app(stations: Vec<Station>) -> App {
        let mut app = App {
            selected: 0,
            volume: 80.0,
            status: String::new(),
            mode: Mode::Normal,
            list_state: ListState::default(),
            save_enabled: false,
            mpv: None,
            ipc_path: None,
            playing_url: None,
            visualizer: Visualizer::new(),
            ipc_request_id: 1,
            playing_since: None,
            last_input: Instant::now(),
            last_metadata_at: None,
            track_title: None,
            icy_name: None,
            import: None,
            stations,
        };
        app.sync_list_state();
        app
    }

    fn station(name: &str, url: &str) -> Station {
        Station {
            name: name.to_string(),
            url: url.to_string(),
            favorite: false,
        }
    }

    #[test]
    fn wrapping_navigation_on_empty_list_is_safe() {
        let mut app = test_app(vec![]);
        app.select_delta(1);
        app.select_delta(-1);
        assert_eq!(app.selected, 0);
        assert!(app.list_state.selected().is_none());
    }

    #[test]
    fn wrapping_navigation() {
        let mut app = test_app(vec![
            station("a", "https://a.example/x"),
            station("b", "https://b.example/x"),
        ]);
        app.select_delta(-1);
        assert_eq!(app.selected, 1);
        app.select_delta(1);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn delete_last_station_adjusts_selection() {
        let mut app = test_app(vec![
            station("a", "https://a.example/x"),
            station("b", "https://b.example/x"),
        ]);
        app.selected = 1;
        app.delete_selected();
        assert_eq!(app.stations.len(), 1);
        assert_eq!(app.selected, 0);
        assert_eq!(app.stations[0].name, "a");
    }

    #[test]
    fn edit_url_keeps_the_station_and_follows_playback() {
        let mut app = test_app(vec![
            station("a", "https://a.example/x"),
            station("b", "https://b.example/x"),
        ]);
        app.selected = 1;
        app.playing_url = Some("https://b.example/x".to_string());
        app.finish_edit_url(1, "https://b.example/fixed".to_string());
        assert_eq!(app.stations[1].url, "https://b.example/fixed");
        assert_eq!(app.stations[1].name, "b");
        assert_eq!(app.playing_index(), Some(1));
        assert_eq!(app.mode, Mode::Edit);

        app.finish_edit_url(1, "https://a.example/x".to_string());
        assert_eq!(app.stations[1].url, "https://b.example/fixed");
        assert!(matches!(app.mode, Mode::EditUrl { index: 1, .. }));
    }

    #[test]
    fn url_editor_deletes_the_character_under_the_cursor() {
        let mut buffer = "https://example.com/x".to_string();
        let mut cursor = 4;
        let key = KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE);
        let mut app = test_app(vec![]);
        assert!(!app.handle_input_key(key, &mut buffer, &mut cursor));
        assert_eq!(buffer, "http://example.com/x");
        assert_eq!(cursor, 4);

        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        app.handle_input_key(key, &mut buffer, &mut cursor);
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        app.handle_input_key(key, &mut buffer, &mut cursor);
        assert_eq!(buffer, "htp://example.com/x");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn reorder_swaps_neighbors() {
        let mut app = test_app(vec![
            station("a", "https://a.example/x"),
            station("b", "https://b.example/x"),
            station("c", "https://c.example/x"),
        ]);
        app.move_selected(1);
        assert_eq!(app.stations[0].name, "b");
        assert_eq!(app.stations[1].name, "a");
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn favorite_pops_to_top_and_unfavorite_lands_after_remaining_stars() {
        let mut app = test_app(vec![
            station("a", "https://a.example/x"),
            station("b", "https://b.example/x"),
            station("c", "https://c.example/x"),
        ]);
        app.playing_url = Some("https://c.example/x".to_string());
        app.selected = 2;
        app.sync_list_state();
        app.toggle_favorite();
        assert!(app.stations[0].favorite);
        assert_eq!(app.stations[0].name, "c");
        assert_eq!(app.selected, 0);
        assert_eq!(app.playing_index(), Some(0));

        app.selected = 2;
        app.sync_list_state();
        app.toggle_favorite();
        assert_eq!(app.stations[0].name, "b");
        assert!(app.stations[0].favorite);
        assert!(app.stations[1].favorite);
        assert_eq!(app.playing_index(), Some(1));

        app.selected = 0;
        app.sync_list_state();
        app.toggle_favorite();
        assert_eq!(
            app.stations
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "b", "a"]
        );
        assert!(!app.stations[1].favorite);
        assert!(app.stations[0].favorite);
        assert_eq!(app.selected, 1);
        assert_eq!(app.playing_index(), Some(0));
    }

    #[test]
    fn apply_import_appends_only_chosen_new_stations() {
        let mut app = test_app(vec![station("a", "https://a.example/x")]);
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        app.import = Some(ImportPick {
            source: "http://192.168.1.10/stations.json".into(),
            stations: vec![
                station("a", "https://a.example/x"),
                station("b", "https://b.example/x"),
                station("c", "https://c.example/x"),
            ],
            chosen: vec![true, true, false],
            already: vec![true, false, false],
            list_state,
        });
        app.apply_import();
        assert_eq!(app.stations.len(), 2);
        assert_eq!(app.stations[1].name, "b");
        assert!(app.import.is_none());
        assert_eq!(app.mode, Mode::Edit);
    }

    #[test]
    fn icy_metadata_parses_title_and_name() {
        let data = serde_json::json!({
            "icy-br": "256",
            "icy-name": "Groove Salad: chilled [SomaFM]",
            "icy-title": "Beat.dowsing - Renegades"
        });
        let (title, name) = parse_stream_metadata(&data);
        assert_eq!(title.as_deref(), Some("Beat.dowsing - Renegades"));
        assert_eq!(name.as_deref(), Some("Groove Salad: chilled [SomaFM]"));
    }

    #[test]
    fn media_title_ignores_urls() {
        assert!(title_from_value(&serde_json::json!("https://example.com/x")).is_none());
        assert_eq!(
            title_from_value(&serde_json::json!("Artist - Song")).as_deref(),
            Some("Artist - Song")
        );
    }

    #[test]
    fn now_playing_esc_returns_to_list() {
        let mut app = test_app(vec![station("a", "https://a.example/x")]);
        app.mode = Mode::NowPlaying;
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(!app.handle_key(key));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn now_playing_any_key_returns_to_list() {
        let mut app = test_app(vec![station("a", "https://a.example/x")]);
        app.mode = Mode::NowPlaying;
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!app.handle_key(key));
        assert_eq!(app.mode, Mode::Normal);

        app.mode = Mode::NowPlaying;
        let key = KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE);
        assert!(!app.handle_key(key));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn idle_timer_requires_a_minute_of_playback_and_no_input() {
        let mut app = test_app(vec![station("a", "https://a.example/x")]);
        let now = Instant::now();
        app.playing_since = Some(now - Duration::from_secs(90));
        app.last_input = now - Duration::from_secs(10);
        assert!(!app.ready_for_now_playing(now));
        app.last_input = now - Duration::from_secs(90);
        assert!(app.ready_for_now_playing(now));
        app.mode = Mode::Edit;
        assert!(!app.ready_for_now_playing(now));
    }
}
