//! Reads and writes ~/.config/radio-tui/stations.json, and fetches a remote list.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Station {
    pub name: String,
    pub url: String,
    /// Local-only. Missing in old files, and never copied from a remote list.
    #[serde(default)]
    pub favorite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub stations: Vec<Station>,
    #[serde(default = "default_volume")]
    pub volume: f32,
}

#[derive(Debug)]
pub enum LoadError {
    NotFound,
    Invalid(String),
    Io(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotFound => write!(f, "no config file"),
            LoadError::Invalid(msg) => write!(f, "invalid config: {msg}"),
            LoadError::Io(msg) => write!(f, "config io error: {msg}"),
        }
    }
}

pub fn default_volume() -> f32 {
    80.0
}

pub fn clamp_volume(volume: f32) -> f32 {
    volume.clamp(0.0, 100.0)
}

pub fn config_dir() -> PathBuf {
    if let Some(dir) = dirs::config_dir() {
        return dir.join("radio-tui");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("radio-tui")
}

pub fn config_path() -> PathBuf {
    config_dir().join("stations.json")
}

pub fn default_stations() -> Vec<Station> {
    vec![
        Station {
            name: "BBC World Service".to_string(),
            url: "http://stream.live.vc.bbcmedia.co.uk/bbc_world_service".to_string(),
            favorite: false,
        },
        Station {
            name: "SomaFM Groove Salad".to_string(),
            url: "http://ice1.somafm.com/groovesalad-256-mp3".to_string(),
            favorite: false,
        },
    ]
}

pub fn default_config() -> Config {
    Config {
        stations: default_stations(),
        volume: default_volume(),
    }
}

pub fn name_for_url(url: &str) -> String {
    for station in default_stations() {
        if station.url == url {
            return station.name;
        }
    }
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

pub fn valid_stream_url(url: &str) -> bool {
    let url = url.trim();
    (url.starts_with("http://") || url.starts_with("https://")) && url.len() > 10
}

pub fn valid_list_url(url: &str) -> bool {
    valid_stream_url(url)
}

const REMOTE_LIST_MAX_BYTES: u64 = 1_000_000;

pub fn fetch_remote_stations(url: &str) -> Result<Vec<Station>, LoadError> {
    let url = url.trim();
    if !valid_list_url(url) {
        return Err(LoadError::Invalid(
            "list URL must start with http:// or https://".to_string(),
        ));
    }
    let response = ureq::get(url)
        .timeout(Duration::from_secs(15))
        .set("User-Agent", "radio-tui")
        .call()
        .map_err(|err| LoadError::Io(err.to_string()))?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(REMOTE_LIST_MAX_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|err| LoadError::Io(err.to_string()))?;
    if body.len() as u64 > REMOTE_LIST_MAX_BYTES {
        return Err(LoadError::Invalid("remote list is too large".to_string()));
    }
    let json = String::from_utf8(body)
        .map_err(|_| LoadError::Invalid("remote list is not valid UTF-8".to_string()))?;
    let mut config = parse_config(&json)?;
    if config.stations.is_empty() {
        return Err(LoadError::Invalid(
            "remote list has no stations".to_string(),
        ));
    }
    for station in &mut config.stations {
        station.favorite = false;
    }
    Ok(config.stations)
}

pub fn selected_new_stations(
    local: &[Station],
    remote: &[Station],
    chosen: &[bool],
) -> Vec<Station> {
    remote
        .iter()
        .zip(chosen.iter())
        .filter_map(|(station, pick)| {
            if *pick && !local.iter().any(|existing| existing.url == station.url) {
                Some(station.clone())
            } else {
                None
            }
        })
        .collect()
}

/// Wire format that accepts both the original URL-string list and named stations.
#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default)]
    stations: Vec<RawStation>,
    #[serde(default = "default_volume")]
    volume: f32,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawStation {
    Url(String),
    Named {
        name: String,
        url: String,
        #[serde(default)]
        favorite: bool,
    },
}

impl From<RawStation> for Station {
    fn from(raw: RawStation) -> Self {
        match raw {
            RawStation::Url(url) => Station {
                name: name_for_url(&url),
                url,
                favorite: false,
            },
            RawStation::Named {
                name,
                url,
                favorite,
            } => Station {
                name: if name.trim().is_empty() {
                    name_for_url(&url)
                } else {
                    name
                },
                url,
                favorite,
            },
        }
    }
}

pub fn parse_config(json: &str) -> Result<Config, LoadError> {
    let raw: RawConfig =
        serde_json::from_str(json).map_err(|err| LoadError::Invalid(err.to_string()))?;
    Ok(Config {
        stations: raw.stations.into_iter().map(Station::from).collect(),
        volume: clamp_volume(raw.volume),
    })
}

pub fn load_config() -> Result<Config, LoadError> {
    let path = config_path();
    if !path.exists() {
        return Err(LoadError::NotFound);
    }
    let json = fs::read_to_string(&path).map_err(|err| LoadError::Io(err.to_string()))?;
    parse_config(&json)
}

pub fn save_config(config: &Config) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = config_path();
    let tmp = dir.join("stations.json.tmp");
    let json = serde_json::to_string_pretty(config).context("serializing config")?;
    fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_legacy_url_list() {
        let json = r#"{
            "stations": [
                "http://stream.live.vc.bbcmedia.co.uk/bbc_world_service",
                "http://ice1.somafm.com/groovesalad-256-mp3"
            ],
            "volume": 80.0
        }"#;
        let config = parse_config(json).expect("legacy config should parse");
        assert_eq!(config.stations[0].name, "BBC World Service");
        assert_eq!(
            config.stations[0].url,
            "http://stream.live.vc.bbcmedia.co.uk/bbc_world_service"
        );
        assert_eq!(config.stations[1].name, "SomaFM Groove Salad");
        assert_eq!(config.volume, 80.0);
    }

    #[test]
    fn parse_named_stations() {
        let json = r#"{
            "stations": [
                {"name": "Custom", "url": "https://example.com/stream.mp3"}
            ],
            "volume": 40.0
        }"#;
        let config = parse_config(json).unwrap();
        assert_eq!(config.stations[0].name, "Custom");
        assert_eq!(config.stations[0].url, "https://example.com/stream.mp3");
        assert_eq!(config.volume, 40.0);
    }

    #[test]
    fn missing_volume_uses_default() {
        let json = r#"{"stations": [{"name": "A", "url": "https://example.com/a"}]}"#;
        let config = parse_config(json).unwrap();
        assert_eq!(config.volume, 80.0);
    }

    #[test]
    fn invalid_json_is_error() {
        match parse_config("{not json") {
            Err(LoadError::Invalid(_)) => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn empty_stations_parses() {
        let config = parse_config(r#"{"stations": [], "volume": 10}"#).unwrap();
        assert!(config.stations.is_empty());
        assert_eq!(config.volume, 10.0);
    }

    #[test]
    fn volume_is_clamped() {
        let config = parse_config(r#"{"stations": [], "volume": 9001}"#).unwrap();
        assert_eq!(config.volume, 100.0);
        let config = parse_config(r#"{"stations": [], "volume": -4}"#).unwrap();
        assert_eq!(config.volume, 0.0);
    }

    #[test]
    fn valid_stream_url_rejects_flags() {
        assert!(valid_stream_url("https://example.com/stream.mp3"));
        assert!(valid_stream_url("http://example.com/x"));
        assert!(!valid_stream_url("--input-ipc-server=/tmp/x"));
        assert!(!valid_stream_url("file:///etc/passwd"));
        assert!(!valid_stream_url(""));
    }

    #[test]
    fn name_for_unknown_url_uses_host() {
        assert_eq!(
            name_for_url("https://radio.example.com/hi.mp3"),
            "radio.example.com"
        );
    }

    #[test]
    fn selected_new_stations_skips_duplicates_and_unselected() {
        let local = vec![Station {
            name: "Have".into(),
            url: "https://a.example/x".into(),
            favorite: true,
        }];
        let remote = vec![
            Station {
                name: "Have".into(),
                url: "https://a.example/x".into(),
                favorite: true,
            },
            Station {
                name: "New".into(),
                url: "https://b.example/x".into(),
                favorite: true,
            },
            Station {
                name: "Skip".into(),
                url: "https://c.example/x".into(),
                favorite: false,
            },
        ];
        let chosen = vec![true, true, false];
        let added = selected_new_stations(&local, &remote, &chosen);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].name, "New");
        assert!(added[0].favorite);
    }

    #[test]
    fn local_favorite_flag_round_trips_and_missing_field_defaults_false() {
        let config = parse_config(
            r#"{"stations":[{"name":"Old","url":"https://old.example/x"},{"name":"Starred","url":"https://star.example/x","favorite":true}],"volume":70}"#,
        )
        .unwrap();
        assert!(!config.stations[0].favorite);
        assert!(config.stations[1].favorite);
    }
}
