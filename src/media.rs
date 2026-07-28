//! Controle e leitura do Spotify via System Media Transport Controls (SMTC)
//! do Windows, mais um truque que evita qualquer OAuth/API key:
//!
//! - **Tocar uma playlist especifica**: o Windows registra o protocolo
//!   `spotify:` quando o app desktop e instalado. Abrir uma URI como
//!   `spotify:playlist:37i9dQZF1...` (via `ShellExecuteW`, o mesmo
//!   mecanismo de clicar num link) faz o Spotify abrir/focar e comecar a
//!   tocar aquilo. Sem login, sem API - e o mesmo que clicar num link de
//!   playlist.
//!
//! Controle fino (play/pause/next/previous) e leitura do "now playing"
//! (titulo, artista, album, capa) continuam via SMTC, tambem sem OAuth.
//!
//! NAO ha busca: o SMTC nao expoe "buscar no catalogo", so controla a
//! sessao que ja esta ativa. Pra tocar uma musica especifica escolhida na
//! hora, o jeito e abrir o Spotify normalmente (fora do jogo) e escolher
//! la - depois disso, play/pause/next/previous aqui dentro do caminhao
//! continuam controlando o que estiver tocando, seja lá o que for.

use anyhow::{anyhow, Result};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as MediaSession,
    GlobalSystemMediaTransportControlsSessionManager as MediaManager,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub struct SpotifyMedia {
    manager: MediaManager,
}

impl SpotifyMedia {
    pub fn connect() -> Result<Self> {
        let manager = MediaManager::RequestAsync()?.get()?;
        Ok(Self { manager })
    }

    /// Procura, entre todas as sessoes de midia ativas, a que pertence ao
    /// Spotify. Se o Spotify nao estiver rodando/tocando nada, retorna None.
    fn spotify_session(&self) -> Result<Option<MediaSession>> {
        let sessions = self.manager.GetSessions()?;
        for session in sessions {
            let app_id = session.SourceAppUserModelId()?.to_string_lossy();
            if app_id.to_lowercase().contains("spotify") {
                return Ok(Some(session));
            }
        }
        Ok(None)
    }

    pub fn now_playing(&self) -> Result<Option<NowPlaying>> {
        let Some(session) = self.spotify_session()? else {
            return Ok(None);
        };
        let props = session.TryGetMediaPropertiesAsync()?.get()?;
        Ok(Some(NowPlaying {
            title: props.Title()?.to_string_lossy(),
            artist: props.Artist()?.to_string_lossy(),
            album: props.AlbumTitle()?.to_string_lossy(),
        }))
    }

    /// Le a capa do album/faixa atual e devolve como pixels RGBA
    /// (largura, altura, bytes), prontos pra virar uma textura na UI.
    /// Retorna `Ok(None)` se nao tiver sessao do Spotify ou a sessao nao
    /// expuser capa - nao e um erro fatal, so significa "sem capa agora".
    pub fn thumbnail_rgba(&self) -> Result<Option<(u32, u32, Vec<u8>)>> {
        let Some(session) = self.spotify_session()? else {
            return Ok(None);
        };
        let props = session.TryGetMediaPropertiesAsync()?.get()?;
        let Ok(thumb_ref) = props.Thumbnail() else {
            return Ok(None);
        };

        let stream = thumb_ref.OpenReadAsync()?.get()?;
        let size = stream.Size()? as u32;
        if size == 0 {
            return Ok(None);
        }

        let reader = DataReader::CreateDataReader(&stream)?;
        reader.LoadAsync(size)?.get()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf)?;

        let img = image::load_from_memory(&buf)?.to_rgba8();
        let (w, h) = img.dimensions();
        Ok(Some((w, h, img.into_raw())))
    }

    /// Posicao atual e duracao total da faixa (milissegundos), via
    /// `GetTimelineProperties` do SMTC. `Ok(None)` se nao tiver sessao do
    /// Spotify ou a sessao nao expuser timeline (nem todo player WinRT
    /// preenche isso) - tratado como "sem progresso pra mostrar", nao como
    /// erro fatal, igual `thumbnail_rgba`.
    pub fn timeline_ms(&self) -> Result<Option<(u32, u32)>> {
        let Some(session) = self.spotify_session()? else {
            return Ok(None);
        };
        let props = session.GetTimelineProperties()?;
        let position = props.Position()?.Duration;
        let end = props.EndTime()?.Duration;
        if end <= 0 {
            return Ok(None);
        }
        // `TimeSpan::Duration` e em unidades de 100ns (mesma convencao do
        // .NET/WinRT) - divide por 10_000 pra virar milissegundos.
        let position_ms = (position.max(0) / 10_000) as u32;
        let duration_ms = (end / 10_000) as u32;
        Ok(Some((position_ms, duration_ms)))
    }

    pub fn ensure_running(&self) -> Result<()> {
        if self.spotify_session()?.is_some() {
            return Ok(());
        }
        Err(anyhow!(
            "Spotify nao esta aberto; o bridge nao executa o processo do Spotify em segundo plano"
        ))
    }

    pub fn play_pause(&self) -> Result<()> {
        self.with_session(|s| s.TryTogglePlayPauseAsync()?.get().map_err(Into::into))
    }

    pub fn play(&self) -> Result<()> {
        self.with_session(|s| s.TryPlayAsync()?.get().map_err(Into::into))
    }

    pub fn pause(&self) -> Result<()> {
        self.with_session(|s| s.TryPauseAsync()?.get().map_err(Into::into))
    }

    pub fn next(&self) -> Result<()> {
        self.with_session(|s| s.TrySkipNextAsync()?.get().map_err(Into::into))
    }

    pub fn previous(&self) -> Result<()> {
        self.with_session(|s| s.TrySkipPreviousAsync()?.get().map_err(Into::into))
    }

    /// Abre uma URI `spotify:...` (playlist, album, faixa, busca) usando o
    /// handler de protocolo do proprio Windows/Spotify - sem OAuth.
    pub fn launch_uri(&self, uri: &str) -> Result<()> {
        // Equivalente a clicar num link spotify:... - via ShellExecuteW
        // (mesma API que o Explorer usa pra abrir um link/arquivo pelo
        // handler registrado), NAO via `cmd /C start` (usado antes): o
        // cmd.exe reinterpreta `&`/`|`/`^` etc. mesmo dentro de argumentos
        // aspeados, entao uma URI vinda de uma playlist salva maliciosa
        // (ex. `spotify:playlist:x & calc.exe`) executaria comando
        // arbitrario. ShellExecuteW nao envolve shell nenhum - so pede ao
        // Windows pra abrir esse texto com o handler de protocolo dele.
        let wide = HSTRING::from(uri);
        let result = unsafe {
            ShellExecuteW(None, w!("open"), &wide, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
        };
        // Convencao legada do Win32: retorno > 32 = sucesso; <= 32 = codigo
        // de erro (ver docs do ShellExecuteW).
        if (result.0 as isize) <= 32 {
            return Err(anyhow!("ShellExecuteW falhou (codigo {})", result.0 as isize));
        }
        Ok(())
    }

    fn with_session<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&MediaSession) -> Result<bool>,
    {
        let Some(session) = self.spotify_session()? else {
            return Err(anyhow!("Spotify nao esta rodando ou sem sessao de midia ativa"));
        };
        f(&session)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
}
