//! Superficie `extern "C"` chamada pelo addon do ReShade (C++, ver
//! `reshade-addon/overlay_addon.cpp` e `reshade-addon/ets2_ffi.h`).
//!
//! Convencao: nada aqui deixa um panic atravessar a fronteira `extern "C"`
//! (seria undefined behavior) - toda funcao publica so faz operacoes que
//! nao panicam em uso normal (locks, copias de buffer de tamanho
//! verificado) ou engole erros e devolve um booleano de sucesso/falha.
//! Nenhuma struct `#[repr(C)]` daqui e exposta como ponteiro estavel pro
//! C++ guardar entre chamadas - tudo e copiado pra memoria que o proprio
//! C++ possui, pra nao ter que raciocinar sobre tempo de vida atraves da
//! fronteira FFI.

use crate::audio_device;
use crate::config::{self, Config};
use crate::engine;
use crate::state::{Command, SharedState, UiState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

const TITLE_CAP: usize = 128;
const ARTIST_CAP: usize = 96;
const STATUS_CAP: usize = 160;
const PLAYLIST_NAME_CAP: usize = 64;
const PLAYLIST_URI_CAP: usize = 160;
const DEVICE_ID_CAP: usize = 128;
const DEVICE_NAME_CAP: usize = 96;

/// Snapshot copiado do `UiState` compartilhado, pro C++ desenhar o HUD/
/// painel sem segurar nenhum lock do lado Rust por mais que um `memcpy`.
#[repr(C)]
pub struct FfiSnapshot {
    pub electrics_on: u32,
    pub game_paused: u32,
    pub telemetry_connected: u32,
    pub has_track: u32,
    pub title_len: u32,
    pub artist_len: u32,
    pub title: [u8; TITLE_CAP],
    pub artist: [u8; ARTIST_CAP],
    /// Incrementa a cada capa nova - o C++ so recria a textura da GPU
    /// quando esse valor muda (ver `ensure_album_art_texture` no addon).
    pub thumb_generation: u64,
    pub thumb_width: u32,
    pub thumb_height: u32,
    /// Mensagem de status de uma linha (login pendente, reconectando, erro
    /// - ver `state::UiState::status`). So o backend `Connect` preenche
    /// isso hoje; vazio (`status_len == 0`) = nada pra mostrar.
    pub status_len: u32,
    pub status: [u8; STATUS_CAP],
    /// Volume atual, 0-100 - ver `state::UiState::volume`.
    pub volume: u32,
    /// Posicao/duracao da faixa atual em milissegundos - ver
    /// `state::NowPlayingInfo::position_ms`/`duration_ms`. `duration_ms ==
    /// 0` = duracao desconhecida (painel nao desenha a barra de progresso).
    pub position_ms: u32,
    pub duration_ms: u32,
}

#[repr(C)]
pub struct FfiPlaylist {
    pub name_len: u32,
    pub uri_len: u32,
    pub name: [u8; PLAYLIST_NAME_CAP],
    pub uri: [u8; PLAYLIST_URI_CAP],
}

#[repr(C)]
pub struct FfiOutputDevice {
    pub id_len: u32,
    pub name_len: u32,
    pub id: [u8; DEVICE_ID_CAP],
    pub name: [u8; DEVICE_NAME_CAP],
}

struct Engine {
    state: SharedState,
    cmd_tx: Sender<Command>,
    running: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
    config: Mutex<Config>,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

/// Inicia a thread "motor" (SMTC + telemetria) e carrega a config salva.
/// Seguro chamar mais de uma vez (so a primeira chamada tem efeito).
///
/// NAO deve ser chamado direto de `DllMain`/`DLL_PROCESS_ATTACH` - criar
/// threads/inicializar COM enquanto o loader lock do processo esta preso
/// arrisca deadlock com outras DLLs carregando ao mesmo tempo. O addon
/// C++ chama isso de forma preguicosa (lazy) no primeiro frame desenhado,
/// bem depois do attach ja ter terminado.
#[no_mangle]
pub extern "C" fn ets2_engine_start() -> bool {
    if ENGINE.get().is_some() {
        return true;
    }
    let state: SharedState = Arc::new(Mutex::new(UiState::default()));
    let running = Arc::new(AtomicBool::new(true));
    let (cmd_tx, cmd_rx) = mpsc::channel();

    // Carregada aqui (nao dentro da thread motor) porque o backend
    // (Smtc/Connect - ver config::Backend) precisa ser decidido antes de
    // `engine::run` comecar.
    let cfg = config::load();
    let backend = cfg.backend;

    let thread_state = state.clone();
    let thread_running = running.clone();
    let join = std::thread::Builder::new()
        .name("ets2-spotify-engine".into())
        .spawn(move || engine::run(thread_state, cmd_rx, thread_running, backend))
        .ok();

    let _ = ENGINE.set(Engine {
        state,
        cmd_tx,
        running,
        join: Mutex::new(join),
        config: Mutex::new(cfg),
    });
    true
}

/// Sinaliza a thread motor pra parar e espera ela terminar. Chamado do
/// `DLL_PROCESS_DETACH` do addon - o ReShade pode descarregar o addon com
/// o jogo ainda aberto (ex.: desabilitado pelo menu), diferente do antigo
/// bridge (processo separado, que so morria junto do proprio processo).
/// `engine::run` faz `sleep(400ms)` entre cada checagem de `running`,
/// entao o join aqui volta rapido (nao ha WinRT bloqueante segurando isso
/// por muito tempo).
#[no_mangle]
pub extern "C" fn ets2_engine_shutdown() {
    let Some(engine) = ENGINE.get() else { return };
    engine.running.store(false, Ordering::SeqCst);
    if let Ok(mut guard) = engine.join.lock() {
        if let Some(handle) = guard.take() {
            let _ = handle.join();
        }
    }
}

/// Copia o estado atual (musica tocando, eletrica, telemetria) pro
/// `FfiSnapshot` apontado por `out`. Chamado uma vez por frame do ImGui.
/// `false` = motor ainda nao iniciado (chame `ets2_engine_start` primeiro).
///
/// # Safety
/// `out` precisa apontar pra um `FfiSnapshot` valido e alinhado; o
/// chamador (C++) possui essa memoria.
#[no_mangle]
pub unsafe extern "C" fn ets2_poll_snapshot(out: *mut FfiSnapshot) -> bool {
    if out.is_null() {
        return false;
    }
    let Some(engine) = ENGINE.get() else { return false };
    let Ok(state) = engine.state.lock() else { return false };

    let mut snap = FfiSnapshot {
        electrics_on: state.electrics_on as u32,
        game_paused: state.game_paused as u32,
        telemetry_connected: state.telemetry_connected as u32,
        has_track: state.now_playing.is_some() as u32,
        title_len: 0,
        artist_len: 0,
        title: [0u8; TITLE_CAP],
        artist: [0u8; ARTIST_CAP],
        thumb_generation: 0,
        thumb_width: 0,
        thumb_height: 0,
        status_len: 0,
        status: [0u8; STATUS_CAP],
        volume: state.volume,
        position_ms: 0,
        duration_ms: 0,
    };
    if let Some(np) = &state.now_playing {
        snap.title_len = fill_truncated(&mut snap.title, &np.title) as u32;
        snap.artist_len = fill_truncated(&mut snap.artist, &np.artist) as u32;
        snap.position_ms = np.position_ms;
        snap.duration_ms = np.duration_ms;
        if let Some(thumb) = &np.thumbnail {
            snap.thumb_generation = thumb.generation;
            snap.thumb_width = thumb.width;
            snap.thumb_height = thumb.height;
        }
    }
    snap.status_len = fill_truncated(&mut snap.status, &state.status) as u32;
    std::ptr::write(out, snap);
    true
}

/// Copia os pixels RGBA da capa atual pro buffer `out_buf` (capacidade
/// `buf_cap` bytes) fornecido pelo C++, so se a geracao ainda bater com
/// `expect_generation` (evita corrida com uma troca de faixa entre o poll
/// do snapshot e esta chamada). `out_len` recebe o numero de bytes
/// copiados. Devolve `false` sem copiar nada se nao houver capa, a
/// geracao mudou, ou o buffer for pequeno demais.
///
/// # Safety
/// `out_buf` precisa apontar pra pelo menos `buf_cap` bytes validos;
/// `out_len`, se nao-nulo, pra um `usize` valido.
#[no_mangle]
pub unsafe extern "C" fn ets2_get_thumbnail(
    expect_generation: u64,
    out_buf: *mut u8,
    buf_cap: usize,
    out_len: *mut usize,
) -> bool {
    if out_buf.is_null() {
        return false;
    }
    let Some(engine) = ENGINE.get() else { return false };
    let Ok(state) = engine.state.lock() else { return false };
    let Some(np) = &state.now_playing else { return false };
    let Some(thumb) = &np.thumbnail else { return false };
    if thumb.generation != expect_generation || thumb.rgba.len() > buf_cap {
        return false;
    }
    std::ptr::copy_nonoverlapping(thumb.rgba.as_ptr(), out_buf, thumb.rgba.len());
    if !out_len.is_null() {
        std::ptr::write(out_len, thumb.rgba.len());
    }
    true
}

/// Manda um comando pra thread motor. `text_ptr`/`text_len` so sao usados
/// pro kind 3 (PlayUri); ignorados nos outros. Espelhado em
/// `ets2::CommandKind` (reshade-addon/ets2_ffi.h) - mesmos valores.
///
/// # Safety
/// Se `text_len > 0`, `text_ptr` precisa apontar pra pelo menos
/// `text_len` bytes UTF-8 validos.
#[no_mangle]
pub unsafe extern "C" fn ets2_send_command(kind: u32, text_ptr: *const u8, text_len: usize) -> bool {
    let Some(engine) = ENGINE.get() else { return false };
    let text = || -> String {
        if text_ptr.is_null() || text_len == 0 {
            return String::new();
        }
        let slice = std::slice::from_raw_parts(text_ptr, text_len);
        String::from_utf8_lossy(slice).into_owned()
    };
    let cmd = match kind {
        0 => Command::PlayPause,
        1 => Command::Next,
        2 => Command::Previous,
        3 => Command::PlayUri(text()),
        4 => Command::Play,
        5 => Command::Pause,
        _ => return false,
    };
    engine.cmd_tx.send(cmd).is_ok()
}

/// Define o volume (0-100) - kind numerico separado de `ets2_send_command`
/// porque o canal de comandos so carrega texto (ver `text_ptr`/`text_len`
/// acima), nao um inteiro; mesmo padrao ja usado por `ets2_set_output_device`.
#[no_mangle]
pub extern "C" fn ets2_set_volume(percent: u32) -> bool {
    let Some(engine) = ENGINE.get() else { return false };
    engine.cmd_tx.send(Command::SetVolume(percent.min(100))).is_ok()
}

/// Copia ate `cap` playlists salvas pro array `out`. `out_count` recebe o
/// total de playlists salvas (pode ser maior que `cap`, se o buffer do
/// C++ for pequeno demais - nesse caso so as primeiras `cap` sao
/// copiadas).
///
/// # Safety
/// `out` precisa apontar pra pelo menos `cap` elementos `FfiPlaylist`
/// validos; `out_count`, se nao-nulo, pra um `usize` valido.
#[no_mangle]
pub unsafe extern "C" fn ets2_list_playlists(
    out: *mut FfiPlaylist,
    cap: usize,
    out_count: *mut usize,
) -> bool {
    let Some(engine) = ENGINE.get() else { return false };
    let Ok(cfg) = engine.config.lock() else { return false };
    if !out_count.is_null() {
        std::ptr::write(out_count, cfg.playlists.len());
    }
    if out.is_null() || cap == 0 {
        return true;
    }
    for (i, pl) in cfg.playlists.iter().take(cap).enumerate() {
        let mut item = FfiPlaylist {
            name_len: 0,
            uri_len: 0,
            name: [0u8; PLAYLIST_NAME_CAP],
            uri: [0u8; PLAYLIST_URI_CAP],
        };
        item.name_len = fill_truncated(&mut item.name, &pl.name) as u32;
        item.uri_len = fill_truncated(&mut item.uri, &pl.uri) as u32;
        std::ptr::write(out.add(i), item);
    }
    true
}

/// Adiciona uma playlist (aceita link `open.spotify.com/...` ou URI
/// `spotify:...` - normalizado internamente) e salva a config em disco.
///
/// # Safety
/// `name_ptr`/`uri_ptr` precisam apontar pra `name_len`/`uri_len` bytes
/// UTF-8 validos.
#[no_mangle]
pub unsafe extern "C" fn ets2_add_playlist(
    name_ptr: *const u8,
    name_len: usize,
    uri_ptr: *const u8,
    uri_len: usize,
) -> bool {
    if name_ptr.is_null() || uri_ptr.is_null() || name_len == 0 || uri_len == 0 {
        return false;
    }
    let Some(engine) = ENGINE.get() else { return false };
    let name = String::from_utf8_lossy(std::slice::from_raw_parts(name_ptr, name_len)).into_owned();
    let uri_raw = String::from_utf8_lossy(std::slice::from_raw_parts(uri_ptr, uri_len)).into_owned();
    let uri = config::normalize_spotify_link(uri_raw.trim());

    let Ok(mut cfg) = engine.config.lock() else { return false };
    cfg.playlists.push(config::Playlist { name, uri });
    let ok = config::save(&cfg).is_ok();
    ok
}

/// Remove a playlist no indice `index` (mesma ordem de `ets2_list_playlists`).
#[no_mangle]
pub extern "C" fn ets2_remove_playlist(index: usize) -> bool {
    let Some(engine) = ENGINE.get() else { return false };
    let Ok(mut cfg) = engine.config.lock() else { return false };
    if index >= cfg.playlists.len() {
        return false;
    }
    cfg.playlists.remove(index);
    config::save(&cfg).is_ok()
}

/// Lista os dispositivos de saida de audio ativos do Windows (pra
/// popular o seletor de saida do Spotify no painel).
///
/// # Safety
/// `out` precisa apontar pra pelo menos `cap` elementos `FfiOutputDevice`
/// validos; `out_count`, se nao-nulo, pra um `usize` valido.
#[no_mangle]
pub unsafe extern "C" fn ets2_list_output_devices(
    out: *mut FfiOutputDevice,
    cap: usize,
    out_count: *mut usize,
) -> bool {
    let Ok(devices) = audio_device::list_output_devices() else { return false };
    if !out_count.is_null() {
        std::ptr::write(out_count, devices.len());
    }
    if out.is_null() || cap == 0 {
        return true;
    }
    for (i, d) in devices.iter().take(cap).enumerate() {
        let mut item = FfiOutputDevice {
            id_len: 0,
            name_len: 0,
            id: [0u8; DEVICE_ID_CAP],
            name: [0u8; DEVICE_NAME_CAP],
        };
        item.id_len = fill_truncated(&mut item.id, &d.id) as u32;
        item.name_len = fill_truncated(&mut item.name, &d.name) as u32;
        std::ptr::write(out.add(i), item);
    }
    true
}

/// Define o dispositivo de saida de audio do Spotify (ou volta ao padrao
/// do sistema, se `id_len == 0`) e salva a preferencia na config.
///
/// # Safety
/// Se `id_len > 0`, `id_ptr` precisa apontar pra pelo menos `id_len`
/// bytes UTF-8 validos.
#[no_mangle]
pub unsafe extern "C" fn ets2_set_output_device(id_ptr: *const u8, id_len: usize) -> bool {
    let Some(engine) = ENGINE.get() else { return false };
    let id = if id_ptr.is_null() || id_len == 0 {
        None
    } else {
        Some(String::from_utf8_lossy(std::slice::from_raw_parts(id_ptr, id_len)).into_owned())
    };
    if audio_device::set_spotify_output_device(id.as_deref()).is_err() {
        return false;
    }
    let Ok(mut cfg) = engine.config.lock() else { return false };
    cfg.preferred_output_device = id;
    config::save(&cfg).is_ok()
}

fn fill_truncated(buf: &mut [u8], s: &str) -> usize {
    let bytes = s.as_bytes();
    let n = bytes.len().min(buf.len());
    // Evita cortar no meio de um caractere UTF-8 multibyte.
    let mut n = n;
    while n > 0 && (bytes[n - 1] & 0b1100_0000) == 0b1000_0000 {
        n -= 1;
    }
    buf[..n].copy_from_slice(&bytes[..n]);
    n
}
