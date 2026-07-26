//! Thread "motor" (do inglês/apelido do código, não confundir com o motor
//! do caminhão): a UNICA thread que fala com o SMTC do Windows (objetos
//! WinRT precisam ser usados a partir da mesma thread que os criou) e com a
//! telemetria do jogo. Recebe comandos da UI/hotkeys por um canal, e
//! publica o estado atual (musica tocando, eletrica do caminhao, log) num
//! Mutex compartilhado que a UI le a cada frame.

use crate::media::SpotifyMedia;
use crate::state::{Command, NowPlayingInfo, SharedState, ThumbnailData};
use crate::telemetry::GameTelemetry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub fn run(state: SharedState, cmd_rx: Receiver<Command>, running: Arc<AtomicBool>) {
    crate::init_apartment();

    let media = match SpotifyMedia::connect() {
        Ok(m) => m,
        Err(e) => {
            push_log(&state, format!("Erro ao conectar no SMTC do Windows: {e}"));
            return;
        }
    };
    let mut telemetry = GameTelemetry::connect();

    let mut prev_game_state: Option<crate::telemetry::GameState> = None;
    let mut thumb_generation: u64 = 0;
    let mut last_track_key = String::new();

    push_log(&state, "Motor de integracao iniciado.".to_string());

    while running.load(Ordering::SeqCst) {
        // 1) comandos vindos da UI ou das hotkeys (nao bloqueante)
        while let Ok(cmd) = cmd_rx.try_recv() {
            let result = match &cmd {
                Command::PlayPause => media.play_pause(),
                Command::Play => media.play(),
                Command::Pause => media.pause(),
                Command::Next => media.next(),
                Command::Previous => media.previous(),
                Command::PlayUri(uri) => media.launch_uri(uri),
                Command::SearchAndOpen(q) => media.search(q),
            };
            if let Err(e) = result {
                push_log(&state, format!("Erro ao executar comando: {e}"));
            }
        }

        // 2) telemetria do jogo: eletrica ligada/desligada, pausa. Usamos a
        // parte eletrica (nao o motor) de proposito - ver telemetry.rs.
        match telemetry.read_state() {
            Ok(gs) => {
                match prev_game_state {
                    None => {
                        if gs.electrics_on && !gs.paused {
                            let _ = media.play();
                            push_log(&state, "Eletrica ja ligada ao iniciar - play.".to_string());
                        }
                    }
                    Some(prev) => {
                        if gs.electrics_on && !prev.electrics_on {
                            push_log(&state, "Eletrica ligada - play.".to_string());
                            let _ = media.play();
                        } else if !gs.electrics_on && prev.electrics_on {
                            push_log(&state, "Eletrica desligada - pause.".to_string());
                            let _ = media.pause();
                        } else if gs.electrics_on {
                            if gs.paused && !prev.paused {
                                push_log(&state, "Jogo pausado - pause.".to_string());
                                let _ = media.pause();
                            } else if !gs.paused && prev.paused {
                                push_log(&state, "Jogo despausado - play.".to_string());
                                let _ = media.play();
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
                if let Ok(mut s) = state.lock() {
                    s.telemetry_connected = false;
                }
            }
        }

        // 3) now playing + capa (so recarrega a capa quando a faixa muda)
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
