//! Thread "motor" (do inglês/apelido do código, não confundir com o motor
//! do caminhão): a UNICA thread que fala com o SMTC do Windows (objetos
//! WinRT precisam ser usados a partir da mesma thread que os criou) e com a
//! telemetria do jogo. Recebe comandos do addon do ReShade (via
//! `crate::ffi::ets2_send_command`) por um canal, e publica o estado atual
//! (musica tocando, eletrica do caminhao, log) num Mutex compartilhado que
//! o addon le a cada frame via `crate::ffi::ets2_poll_snapshot` - ver
//! `src/ffi.rs`.

use crate::media::SpotifyMedia;
use crate::state::{Command, NowPlayingInfo, SharedState, ThumbnailData};
use crate::telemetry::GameTelemetry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// A cada quantos ticks do loop principal (cada tick = 400ms) tentamos
/// reconectar um recurso que esta faltando (Spotify/telemetria). ~2s.
const RETRY_EVERY_TICKS: u32 = 5;

pub fn run(state: SharedState, cmd_rx: Receiver<Command>, running: Arc<AtomicBool>) {
    crate::init_apartment();

    // IMPORTANTE: nem `media` (Spotify/SMTC) nem `telemetry` (jogo) bloqueiam
    // o inicio do loop principal. Antes, conectar no Spotify era a PRIMEIRA
    // coisa feita de forma bloqueante - se o Spotify nao estivesse aberto/
    // logado, a thread inteira ficava parada esperando e nem a telemetria
    // nem o overlay chegavam a rodar. Agora os dois sao `Option` e cada um
    // tenta (re)conectar sozinho dentro do loop, sem travar o outro.
    let mut media: Option<SpotifyMedia> = None;
    let mut telemetry: Option<GameTelemetry> = None;
    let mut media_retry_tick: u32 = 0;
    let mut telemetry_retry_tick: u32 = 0;
    let mut media_wait_logged = false;

    let mut prev_game_state: Option<crate::telemetry::GameState> = None;
    let mut thumb_generation: u64 = 0;
    let mut last_track_key = String::new();

    push_log(&state, "Motor de integracao iniciado.".to_string());

    while running.load(Ordering::SeqCst) {
        // 0a) (re)conecta o Spotify via SMTC, sem bloquear o resto do loop.
        if media.is_none() {
            if media_retry_tick == 0 {
                match SpotifyMedia::connect() {
                    Ok(m) => {
                        push_log(&state, "Conectado ao SMTC do Windows.".to_string());
                        let _ = m.ensure_running();
                        media = Some(m);
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
                    let play = |media: &Option<SpotifyMedia>| {
                        if let Some(media) = media {
                            if media.play().is_err() {
                                let _ = media.ensure_running();
                                let _ = media.play();
                            }
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

        // 3) now playing + capa (so recarrega a capa quando a faixa muda)
        if let Some(media) = media.as_ref() {
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

                if let Ok(mut s) = state.lock() {
                    let prev_thumb = s.now_playing.as_ref().and_then(|n| n.thumbnail.clone());
                    s.now_playing = Some(NowPlayingInfo {
                        title: np.title,
                        artist: np.artist,
                        album: np.album,
                        thumbnail: new_thumb.or(prev_thumb),
                    });
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
}

fn push_log(state: &SharedState, line: String) {
    if let Ok(mut s) = state.lock() {
        s.log.push(line);
        if s.log.len() > 200 {
            s.log.remove(0);
        }
    }
}
