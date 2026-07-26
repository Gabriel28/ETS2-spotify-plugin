//! Configuracao persistida do usuario: hotkeys e playlists salvas.
//! Fica em `%APPDATA%\ets2-spotify-bridge\config.json` (Windows).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Hotkeys {
    pub play_pause: String,
    pub next: String,
    pub previous: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            play_pause: "Ctrl+Alt+Space".to_string(),
            next: "Ctrl+Alt+ArrowRight".to_string(),
            previous: "Ctrl+Alt+ArrowLeft".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Playlist {
    pub name: String,
    /// URI no formato `spotify:playlist:<id>` (tambem aceitamos link
    /// `https://open.spotify.com/playlist/<id>` e convertemos na hora
    /// de adicionar - ver `normalize_spotify_link` em main.rs).
    pub uri: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub hotkeys: Hotkeys,
    #[serde(default)]
    pub playlists: Vec<Playlist>,
}

pub fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
    base.join("ets2-spotify-bridge").join("config.json")
}

pub fn load() -> Config {
    let path = config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(config: &Config) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(path, json)?;
    Ok(())
}
