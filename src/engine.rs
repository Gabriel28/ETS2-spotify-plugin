//! Thread "motor" (do inglês/apelido do código, não confundir com o motor
//! do caminhão): a UNICA thread que fala com o SMTC do Windows (objetos
//! WinRT precisam ser usados a partir da mesma thread que os criou) e com a
//! telemetria do jogo. Recebe comandos do addon do ReShade (via
//! `crate::ffi::ets2_send_command`) por um canal, e publica o estado atual
//! (musica tocando, eletrica do caminhao, log) num Mutex compartilhado que
//! o addon le a cada frame via `crate::ffi::ets2_poll_snapshot` - ver
//! `src/ffi.rs`.
//!
//! Backend do Spotify (`config::Backend`): `Smtc` (media.rs) controla um
//! Spotify Desktop ja aberto, e sua conexao/now-playing sao geridos aqui
//! mesmo, por polling, dentro deste loop. `Connect` (spotify_connect.rs) e
//! diferente: o addon VIRA um dispositivo Spotify Connect (sem precisar do
//! Spotify Desktop), rodando numa thread propria com seu proprio runtime
//! async - aqui so seguramos o handle e mandamos comandos; now-playing e
//! status sao escritos direto no `SharedState` por aquele modulo, nao por
//! este loop.

use crate::config::Backend;
use crate::media::SpotifyMedia;
use crate::spotify_connect::SpotifyConnect;
use crate::state::{Command, NowPlayingInfo, SharedState, ThumbnailData};
use crate::telemetry::GameTelemetry;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// A cada quantos ticks do loop principal (cada tick = 400ms) tentamos
/// reconectar um recurso que esta faltando (Spotify/telemetria). ~2s.
const RETRY_EVERY_TICKS: u32 = 5;

/// Backend do Spotify ativo - so existe (`Some`) depois de conectado (SMTC)
/// ou depois de a thread do Connect ter sido criada (o Connect pode
/// demorar pra ficar "pronto" de verdade - ver `SpotifyConnect::start` -
/// mas a partir do momento em que a thread sobe ja da pra guardar o
/// handle; comandos mandados antes do login terminar so falham, mesmo
/// comportamento de "ainda nao conectado" que o SMTC ja tinha).
enum MediaBackend {
    Smtc(SpotifyMedia),
    Connect(SpotifyConnect),
}

impl MediaBackend {
    fn play_pause(&self) -> Result<()> {
        match self {
            Self::Smtc(m) => m.play_pause(),
            Self::Connect(c) => c.play_pause(),
        }
    }
    fn play(&self) -> Result<()> {
        match self {
            Self::Smtc(m) => m.play(),
            Self::Connect(c) => c.play(),
        }
    }
    fn pause(&self) -> Result<()> {
        match self {
            Self::Smtc(m) => m.pause(),
            Self::Connect(c) => c.pause(),
        }
    }
    fn next(&self) -> Result<()> {
        match self {
            Self::Smtc(m) => m.next(),
            Self::Connect(c) => c.next(),
        }
    }
    fn previous(&self) -> Result<()> {
        match self {
            Self::Smtc(m) => m.previous(),
            Self::Connect(c) => c.previous(),
        }
    }
    fn launch_uri(&self, uri: &str) -> Result<()> {
        match self {
            Self::Smtc(m) => m.launch_uri(uri),
            Self::Connect(c) => c.launch_uri(uri),
        }
    }
    /// `percent` em 0-100. Backend Smtc: volume da sessao de audio do
    /// processo do Spotify (`audio_device`), independente do volume mestre
    /// do Windows. Backend Connect: volume do proprio protocolo Spotify
    /// Connect (`Spirc::set_volume`), sincronizado com outros clientes.
    fn set_volume(&self, percent: u32) -> Result<()> {
        match self {
            Self::Smtc(_) => crate::audio_device::set_spotify_volume(percent.min(100) as f32 / 100.0),
            Self::Connect(c) => c.set_volume(percent),
        }
    }
}

pub fn run(state: SharedState, cmd_rx: Receiver<Command>, running: Arc<AtomicBool>, backend: Backend) {
    crate::init_apartment();

    // IMPORTANTE: nem `media` (Spotify/SMTC) nem `telemetry` (jogo) bloqueiam
    // o inicio do loop principal. Antes, conectar no Spotify era a PRIMEIRA
    // coisa feita de forma bloqueante - se o Spotify nao estivesse aberto/
    // logado, a thread inteira ficava parada esperando e nem a telemetria
    // nem o overlay chegavam a rodar. Agora os dois sao `Option` e cada um
    // tenta (re)conectar sozinho dentro do loop, sem travar o outro.
    let mut media: Option<MediaBackend> = None;
    let mut telemetry: Option<GameTelemetry> = None;
    let mut media_retry_tick: u32 = 0;
    let mut telemetry_retry_tick: u32 = 0;
    let mut media_wait_logged = false;

    let mut prev_game_state: Option<crate::telemetry::GameState> = None;
    let mut thumb_generation: u64 = 0;
    let mut last_track_key = String::new();

    push_log(&state, "Motor de integracao iniciado.".to_string());

    // O backend `Connect` gerencia login/reconexao sozinho na propria
    // thread (ver spotify_connect.rs) - so criamos o handle uma vez aqui,
    // sem entrar no mesmo laço de retry usado pelo SMTC (que precisa
    // reconectar toda hora porque so funciona enquanto o Spotify Desktop
    // estiver aberto).
    if backend == Backend::Connect {
        let connect = SpotifyConnect::start(state.clone(), crate::config::librespot_cache_dir());
        media = Some(MediaBackend::Connect(connect));
    }

    while running.load(Ordering::SeqCst) {
        // 0a) (re)conecta o Spotify via SMTC, sem bloquear o resto do loop.
        // So se aplica ao backend Smtc - o Connect ja foi iniciado acima.
        if backend == Backend::Smtc && media.is_none() {
            if media_retry_tick == 0 {
                match SpotifyMedia::connect() {
                    Ok(m) => {
                        push_log(&state, "Conectado ao SMTC do Windows.".to_string());
                        let _ = m.ensure_running();
                        media = Some(MediaBackend::Smtc(m));
                        media_wait_logged = false;
                    }
                    Err(e) => {
                        if !media_wait_logged {
                            push_log(&state, format!("Aguardando SMTC do Windows (abra o Spotify): {e}"));
                            media_wait_logged = true;
                        }
                    }
                }
            }
            media_retry_tick = (media_retry_tick + 1) % RETRY_EVERY_TICKS;
        }

        // 0b) (re)conecta a telemetria do jogo, tambem sem bloquear.
        if telemetry.is_none() {
            if telemetry_retry_tick == 0 {
                if let Ok(t) = GameTelemetry::connect() {
                    telemetry = Some(t);
                    prev_game_state = None;
                    push_log(&state, "Conectado a telemetria do jogo.".to_string());
                }
            }
            telemetry_retry_tick = (telemetry_retry_tick + 1) % RETRY_EVERY_TICKS;
        }

        // 1) comandos vindos do painel do addon ou das hotkeys dentro do
        // jogo (nao bloqueante) - ver crate::ffi::ets2_send_command.
        while let Ok(cmd) = cmd_rx.try_recv() {
            let Some(media) = media.as_ref() else {
                push_log(&state, "Spotify ainda nao conectado - comando ignorado.".to_string());
                continue;
            };
            let result = match &cmd {
                Command::PlayPause => media.play_pause(),
                Command::Play => media.play(),
                Command::Pause => media.pause(),
                Command::Next => media.next(),
                Command::Previous => media.previous(),
                Command::PlayUri(uri) => media.launch_uri(uri),
                Command::SetVolume(percent) => media.set_volume(*percent),
            };
            if let Err(e) = result {
                push_log(&state, format!("Erro ao executar comando: {e}"));
            }
        }

        // 2) telemetria do jogo: eletrica ligada/desligada, pausa. Usamos a
        // parte eletrica (nao o motor) de proposito - ver telemetry.rs.
        // Se a conexao cair (jogo fechado), solta o recurso pra tentar de
        // novo em vez de continuar reportando erro.
        if let Some(tel) = telemetry.as_mut() {
            match tel.read_state() {
                Ok(gs) => {
                    let play = |media: &Option<MediaBackend>| {
                        if let Some(media) = media {
                            let _ = media.play();
                        }
                    };
                    match prev_game_state {
                        None => {
                            if gs.electrics_on && !gs.paused {
                                play(&media);
                                push_log(&state, "Eletrica ja ligada ao iniciar - play.".to_string());
                            }
                        }
                        Some(prev) => {
                            if gs.electrics_on && !prev.electrics_on {
                                push_log(&state, "Eletrica ligada - play.".to_string());
                                play(&media);
                            } else if !gs.electrics_on && prev.electrics_on {
                                push_log(&state, "Eletrica desligada - pause.".to_string());
                                if let Some(media) = media.as_ref() {
                                    let _ = media.pause();
                                }
                            } else if gs.electrics_on {
                                if gs.paused && !prev.paused {
                                    push_log(&state, "Jogo pausado - pause.".to_string());
                                    if let Some(media) = media.as_ref() {
                                        let _ = media.pause();
                                    }
                                } else if !gs.paused && prev.paused {
                                    push_log(&state, "Jogo despausado - play.".to_string());
                                    play(&media);
                                }
                            }
                        }
                    }
                    prev_game_state = Some(gs);
                    if let Ok(mut s) = state.lock() {
                        s.telemetry_connected = true;
                        s.electrics_on = gs.electrics_on;
                        s.game_paused = gs.paused;
                    }
                }
                Err(_) => {
                    // Provavelmente o jogo fechou - solta a conexao pra
                    // tentar de novo em vez de continuar reportando erro.
                    telemetry = None;
                    prev_game_state = None;
                    if let Ok(mut s) = state.lock() {
                        s.telemetry_connected = false;
                    }
                }
            }
        }

        // 3) now playing + capa (so recarrega a capa quando a faixa muda).
        // So se aplica ao backend Smtc - e por polling porque o SMTC do
        // Windows nao tem como "avisar" quando a faixa troca. O backend
        // Connect e orientado a evento e ja escreve `now_playing`/`status`
        // direto no SharedState sozinho (ver spotify_connect.rs), entao
        // nao ha nada pra fazer aqui nesse caso.
        if let Some(MediaBackend::Smtc(media)) = media.as_ref() {
            if let Ok(Some(np)) = media.now_playing() {
                let key = format!("{}|{}", np.artist, np.title);
                let mut new_thumb: Option<ThumbnailData> = None;
                if key != last_track_key {
                    last_track_key = key;
                    if let Ok(Some((w, h, pixels))) = media.thumbnail_rgba() {
                        thumb_generation += 1;
                        new_thumb = Some(ThumbnailData {
                            width: w,
                            height: h,
                            rgba: pixels,
                            generation: thumb_generation,
                        });
                    }
                }
                let (position_ms, duration_ms) = media.timeline_ms().ok().flatten().unwrap_or((0, 0));

                if let Ok(mut s) = state.lock() {
                    let prev_thumb = s.now_playing.as_ref().and_then(|n| n.thumbnail.clone());
                    s.now_playing = Some(NowPlayingInfo {
                        title: np.title,
                        artist: np.artist,
                        album: np.album,
                        thumbnail: new_thumb.or(prev_thumb),
                        position_ms,
                        duration_ms,
                    });
                }
            }
            // Volume da sessao de audio do Spotify (independente do now
            // playing - segue disponivel mesmo sem faixa tocando agora).
            if let Ok(Some(vol)) = crate::audio_device::get_spotify_volume() {
                if let Ok(mut s) = state.lock() {
                    s.volume = (vol.clamp(0.0, 1.0) * 100.0).round() as u32;
                }
            }
        }

        // 4) Nao ha mais um passo de "publicar" separado: o addon do
        // ReShade le direto do mesmo `state` (Mutex compartilhado) via
        // `crate::ffi::ets2_poll_snapshot` a cada frame - ver src/ffi.rs.
        // Ate a migracao pro addon (ver plano em .claude/plans), esse passo
        // publicava pra uma area de memoria compartilhada do Windows lida
        // por um processo separado; agora tudo roda no mesmo processo.

        thread::sleep(Duration::from_millis(400));
    }

    push_log(&state, "Motor de integracao encerrado.".to_string());
    // `media` (SpotifyConnect, se for o backend ativo) e derrubado aqui ao
    // sair de escopo - seu `Drop` sinaliza a thread do Connect pra parar e
    // espera ela terminar, mesma disciplina de shutdown que
    // `ets2_engine_shutdown` (ffi.rs) ja aplica a esta thread motor.
}

fn push_log(state: &SharedState, line: String) {
    if let Ok(mut s) = state.lock() {
        s.log.push(line);
        if s.log.len() > 200 {
            s.log.remove(0);
        }
    }
}
