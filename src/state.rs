//! Estado compartilhado entre a thread de "motor" (engine.rs, dona da
//! conexao com o Spotify/telemetria) e o addon do ReShade, que le esse
//! estado a cada frame via `crate::ffi::ets2_poll_snapshot` (ver
//! src/ffi.rs).

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
    /// Nao exposto no FFI (o painel do addon so mostra titulo/artista) -
    /// mantido pra uso futuro sem precisar mexer em media.rs de novo.
    #[allow(dead_code)]
    pub album: String,
    pub thumbnail: Option<ThumbnailData>,
    /// Posicao atual e duracao total da faixa, em milissegundos - ver
    /// `media::SpotifyMedia::timeline` (backend Smtc, via
    /// `GetTimelineProperties`) e `spotify_connect` (backend Connect, via
    /// `AudioItem::duration_ms` + eventos `PlayerEvent::Playing/Paused/
    /// PositionChanged/Seeked`). `duration_ms == 0` = duracao desconhecida
    /// (painel nao desenha a barra de progresso nesse caso).
    pub position_ms: u32,
    pub duration_ms: u32,
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
    /// Mensagem de status de uma linha so, sobrescrita conforme o backend
    /// ativo (ver config::Backend) muda de fase - hoje so o backend
    /// `Connect` usa isso (login OAuth pendente, reconectando, erro). Vazio
    /// = nada pra mostrar. Exposta no painel via FFI (`ets2_poll_snapshot`).
    pub status: String,
    /// Volume atual (0-100). Backend Connect: espelha `PlayerEvent::
    /// VolumeChanged` (convertido da escala interna 0-65535 do librespot).
    /// Backend Smtc: espelha o volume da sessao de audio do processo do
    /// Spotify (`audio_device::spotify_session_volume`), independente do
    /// volume mestre do Windows.
    pub volume: u32,
}

pub type SharedState = Arc<Mutex<UiState>>;

/// Comandos que o addon (cliques no painel / hotkeys dentro do jogo) manda
/// pra thread que possui a conexao com o Spotify.
///
/// `Play`/`Pause` (deterministicos) existem separados de `PlayPause`
/// (toggle) porque as hotkeys globais (ver overlay_addon.cpp) usam teclas
/// dedicadas pra cada acao - com um toggle so, apertar a tecla errada
/// deixaria no estado errado sem o usuario conseguir ver o player pra
/// perceber e corrigir.
#[derive(Clone)]
pub enum Command {
    PlayPause,
    Play,
    Pause,
    Next,
    Previous,
    PlayUri(String),
    /// Volume absoluto, 0-100 - convertido pra escala interna de cada
    /// backend (ver `state::UiState::volume`).
    SetVolume(u32),
}
