//! Configuracao persistida do usuario: playlists salvas e dispositivo de
//! saida de audio preferido do Spotify. Fica em
//! `%APPDATA%\ets2-spotify-bridge\config.json` (Windows).
//!
//! `hotkeys`/`window` (do antigo app desktop) foram removidos na migracao
//! pro addon do ReShade - ver plano em `.claude/plans` - mas configs
//! antigas com esses campos ainda carregam normalmente (serde ignora
//! campos desconhecidos).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Playlist {
    pub name: String,
    /// URI no formato `spotify:playlist:<id>` (tambem aceitamos link
    /// `https://open.spotify.com/playlist/<id>` e convertemos na hora de
    /// adicionar - ver `normalize_spotify_link`).
    pub uri: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub playlists: Vec<Playlist>,
    /// Id do dispositivo de saida de audio escolhido pra tocar o Spotify
    /// (ver audio_device.rs). None = usa o dispositivo padrao do Windows.
    #[serde(default)]
    pub preferred_output_device: Option<String>,
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

/// Aceita tanto um link `https://open.spotify.com/<tipo>/<id>` quanto uma
/// URI `spotify:...` ja pronta, e devolve sempre no formato URI - o unico
/// que `media::launch_uri` sabe abrir.
pub fn normalize_spotify_link(input: &str) -> String {
    if input.starts_with("spotify:") {
        return input.to_string();
    }
    if let Some(rest) = input.split("open.spotify.com/").nth(1) {
        let clean = rest.split('?').next().unwrap_or("");
        let mut parts = clean.split('/');
        if let (Some(kind), Some(id)) = (parts.next(), parts.next()) {
            if !kind.is_empty() && !id.is_empty() {
                return format!("spotify:{kind}:{id}");
            }
        }
    }
    input.to_string()
}
