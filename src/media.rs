//! Controle e leitura do Spotify via System Media Transport Controls (SMTC)
//! do Windows, mais dois truques que evitam qualquer OAuth/API key:
//!
//! - **Tocar uma playlist especifica**: o Windows registra o protocolo
//!   `spotify:` quando o app desktop e instalado. Abrir uma URI como
//!   `spotify:playlist:37i9dQZF1...` (via `start`, o mesmo mecanismo de
//!   clicar num link) faz o Spotify abrir/focar e comecar a tocar aquilo.
//!   Sem login, sem API - e o mesmo que clicar num link de playlist.
//! - **Buscar**: mesma ideia com `spotify:search:<termo>`, que abre o
//!   Spotify direto na tela de busca com o termo preenchido.
//!
//! Controle fino (play/pause/next/previous) e leitura do "now playing"
//! (titulo, artista, album, capa) continuam via SMTC, tambem sem OAuth.

use anyhow::{anyhow, Result};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as MediaSession,
    GlobalSystemMediaTransportControlsSessionManager as MediaManager,
};
use windows::Storage::Streams::DataReader;

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
        // Equivalente a clicar num link spotify:... - o "start" do cmd é
        // quem sabe abrir URIs de protocolo registradas no Windows. O
        // primeiro argumento "" depois do start e o titulo da janela
        // (necessario pra o start nao confundir a URI com o titulo).
        std::process::Command::new("cmd")
            .args(["/C", "start", "", uri])
            .spawn()?;
        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<()> {
        let encoded = urlencoding::encode(query);
        self.launch_uri(&format!("spotify:search:{encoded}"))
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
