//! Estado compartilhado entre a thread de "motor" (engine.rs, dona da
//! conexao com o Spotify/telemetria) e a thread de UI (main.rs).

use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct ThumbnailData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Incrementado a cada nova capa carregada, pra UI saber quando
    /// precisa recriar a textura em vez de fazer isso todo frame.
    pub generation: u64,
}

#[derive(Clone, Default)]
pub struct NowPlayingInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub thumbnail: Option<ThumbnailData>,
}

#[derive(Default)]
pub struct UiState {
    pub now_playing: Option<NowPlayingInfo>,
    /// Parte eletrica do caminhao ligada (nao exige o motor ligado - ver
    /// telemetry.rs pra entender a diferenca).
    pub electrics_on: bool,
    pub game_paused: bool,
    pub telemetry_connected: bool,
    pub log: Vec<String>,
}

pub type SharedState = Arc<Mutex<UiState>>;

/// Comandos que a UI (cliques de botao / hotkeys) manda pra thread que
/// possui a conexao com o Spotify.
#[derive(Clone)]
pub enum Command {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    PlayUri(String),
    SearchAndOpen(String),
}
