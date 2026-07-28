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

/// Qual mecanismo o engine usa pra falar com o Spotify - ver
/// `src/media.rs` (Smtc) e `src/spotify_connect.rs` (Connect).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Controla remotamente o Spotify Desktop já aberto via System Media
    /// Transport Controls do Windows. Sem login/OAuth, mas exige o
    /// aplicativo do Spotify rodando no mesmo PC.
    Smtc,
    /// O addon vira um dispositivo Spotify Connect de verdade (aparece na
    /// lista de dispositivos do app oficial em qualquer aparelho) e toca o
    /// áudio ele mesmo, sem precisar do Spotify Desktop aberto. Exige
    /// login OAuth (abre o navegador uma vez; login fica salvo depois) e
    /// conta Premium.
    #[default]
    Connect,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Config {
    #[serde(default)]
    pub playlists: Vec<Playlist>,
    /// Id do dispositivo de saida de audio escolhido pra tocar o Spotify
    /// (ver audio_device.rs). None = usa o dispositivo padrao do Windows.
    /// So se aplica ao backend `Smtc` - o backend `Connect` usa a saida de
    /// audio padrao do Windows (o processo que "toca" e o proprio jogo).
    #[serde(default)]
    pub preferred_output_device: Option<String>,
    /// Qual backend usar - ver `Backend`. Default `Connect`.
    #[serde(default)]
    pub backend: Backend,
}

pub fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
    base.join("ets2-spotify-bridge").join("config.json")
}

/// Onde o backend `Connect` (ver `spotify_connect.rs`) guarda as
/// credenciais de login reutilizaveis (cache do librespot), pra so pedir
/// login OAuth de novo se esse diretorio for apagado.
pub fn librespot_cache_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
    base.join("ets2-spotify-bridge").join("librespot-cache")
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
