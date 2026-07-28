//! ets2-spotify-core
//!
//! Staticlib linkada dentro da DLL do addon do ReShade
//! (`reshade-addon/overlay_addon.cpp`) — substitui o antigo processo
//! separado "bridge". Toda a integracao com Spotify (SMTC), telemetria do
//! jogo, saida de audio e config continua aqui em Rust; o C++ do lado do
//! addon so cuida do que so ele pode fazer (registro do addon no ReShade,
//! widgets do ImGui, criacao de textura da capa do album).
//!
//! Ver `src/ffi.rs` pra superficie `extern "C"` chamada pelo C++.

mod audio_device;
mod config;
mod engine;
mod ffi;
mod media;
mod spotify_connect;
mod state;
mod telemetry;

use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

/// Inicializa o apartment COM/WinRT nesta thread. Necessario porque as APIs
/// de SMTC (windows::Media::Control) sao WinRT e cada thread que as usa
/// precisa inicializar o runtime antes. Chamar isso mais de uma vez na
/// mesma thread e seguro (o erro de "ja inicializado" e ignorado).
pub(crate) fn init_apartment() {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
}
