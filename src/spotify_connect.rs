//! Backend "connect": o addon vira um dispositivo Spotify Connect de
//! verdade (aparece na lista de dispositivos do app oficial em qualquer
//! aparelho - celular, outro PC) e toca o audio ele mesmo, sem precisar do
//! Spotify Desktop aberto no PC do jogo - ver README.md, secao "Backend
//! connect (sem o Spotify Desktop aberto)".
//!
//! Login via OAuth (PKCE, abre o navegador do Windows uma vez) usando o
//! client_id publico que o proprio librespot ja embute
//! (`SessionConfig::default().client_id`, "KEYMASTER_CLIENT_ID" em
//! librespot-core) - nao precisamos registrar nenhum app no dashboard da
//! Spotify nem guardar client secret. O token obtido so serve pra
//! autenticar a sessao (equivalente a um "login salvo"); depois do primeiro
//! login, as credenciais reutilizaveis ficam em cache em disco (ver
//! `cache_dir` em `start`) e o navegador nao abre de novo.
//!
//! Arquitetura: tudo isso (Session/Spirc do librespot) roda sobre tokio,
//! diferente do resto do engine (sincrono, polling de 400ms - ver
//! engine.rs). Em vez de misturar os dois, esse modulo sobe sua PROPRIA
//! thread OS dedicada com seu proprio `tokio::runtime::Runtime`, e expoe só
//! metodos sincronos (`play`/`pause`/`next`/`previous`/`launch_uri`) pro
//! resto do engine chamar sem se importar com async - os metodos do
//! `Spirc` do librespot ja sao sincronos por natureza (só empurram um
//! comando pra dentro de um channel que a task async processa).
//!
//! Diferente do backend SMTC (`media.rs`, onde `engine.rs` faz polling de
//! `now_playing()` a cada tick), aqui o "now playing" e a "status" (login
//! pendente, erro, etc.) sao escritos direto no `SharedState` de forma
//! assincrona, conforme os eventos do `Player` chegam - ver
//! `spawn_player_event_task`.

use anyhow::{anyhow, Context, Result};
use librespot_connect::{ConnectConfig, LoadRequest, LoadRequestOptions, Spirc};
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::{DeviceType, SessionConfig};
use librespot_core::session::Session;
use librespot_metadata::audio::UniqueFields;
use librespot_oauth::{OAuthClient, OAuthClientBuilder, OAuthToken};
use librespot_playback::audio_backend;
use librespot_playback::config::{AudioFormat, PlayerConfig};
use librespot_playback::mixer::{self, MixerConfig};
use librespot_playback::player::{Player, PlayerEvent, PlayerEventChannel};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::state::{NowPlayingInfo, SharedState, ThumbnailData};

/// A cada quantos ms o `Player` emite `PlayerEvent::PositionChanged`
/// enquanto uma faixa toca - ver `PlayerConfig::position_update_interval`
/// em `run_session`. Controla a suavidade da barra de progresso no painel;
/// 500ms e frequente o bastante pra parecer fluido sem gerar trafego
/// excessivo entre a task de eventos e o `SharedState`.
const POSITION_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

/// Escala interna de volume do librespot (`Spirc::set_volume`/
/// `PlayerEvent::VolumeChanged`) - 0 a 65535, igual ao volume "cru" do
/// protocolo Spotify Connect. O painel/FFI trabalham em 0-100 (ver
/// `state::UiState::volume`); a conversao entre as duas escalas fica toda
/// neste modulo.
const VOLUME_MAX: u32 = u16::MAX as u32;

/// Mesma lista de escopos que o CLI oficial do librespot pede pro login
/// OAuth (src/main.rs, `OAUTH_SCOPES`) - usar um subconjunto arriscaria
/// faltar permissao pra alguma chamada interna do protocolo Connect.
const OAUTH_SCOPES: &[&str] = &[
    "app-remote-control",
    "playlist-modify",
    "playlist-modify-private",
    "playlist-modify-public",
    "playlist-read",
    "playlist-read-collaborative",
    "playlist-read-private",
    "streaming",
    "ugc-image-upload",
    "user-follow-modify",
    "user-follow-read",
    "user-library-modify",
    "user-library-read",
    "user-modify",
    "user-modify-playback-state",
    "user-modify-private",
    "user-personalized",
    "user-read-birthdate",
    "user-read-currently-playing",
    "user-read-email",
    "user-read-play-history",
    "user-read-playback-position",
    "user-read-playback-state",
    "user-read-private",
    "user-read-recently-played",
    "user-top-read",
];

/// URI de redirect do login OAuth - PRECISA ter uma porta explicita na
/// URL. `librespot_oauth::get_socket_address` (oauth/src/lib.rs) so sobe um
/// servidor local de verdade pra receber o redirect quando `Url::port()`
/// retorna `Some` (ou seja, uma porta nao-default explicita na URI); sem
/// porta, ele cai no fluxo "cole a URL manualmente no stdin" - que nao
/// funciona aqui dentro (o addon roda injetado no processo do jogo, sem
/// console visivel pro usuario colar nada). Foi exatamente esse bug que
/// causou "conexao recusada" no navegador na primeira tentativa: o
/// redirect ia pra 127.0.0.1:80/login sem nada escutando la.
///
/// A porta em si pode ser (quase) qualquer uma - redirects loopback
/// (`127.0.0.1`) seguem a RFC 8252 sec. 7.3, que a maioria dos provedores
/// OAuth (Spotify incluso, e e por isso que `--oauth-port` do CLI aceita
/// qualquer valor) trata como coringa: so o esquema/host/path do
/// redirect_uri registrado importam, a porta pode variar por execucao. Se
/// 8898 colidir com outra coisa nesta maquina, troque o numero aqui.
const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:8898/login";

/// Quanto tempo esperamos o usuario terminar o login no navegador antes de
/// desistir. `OAuthClient::get_access_token` (librespot-oauth) e uma
/// chamada SINCRONA/bloqueante (usa `TcpListener::incoming().next()` por
/// baixo, sem timeout nenhum) - sem um limite proprio aqui, se o usuario
/// nunca completar o login (fechar a aba, demorar, etc.) essa chamada
/// travaria pra sempre. Ver `login_with_timeout`.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Quanto tempo `SpotifyConnect::drop` espera a thread do backend connect
/// terminar antes de desistir de esperar (mas SEM matar a thread - so para
/// de bloquear o encerramento do jogo por causa dela). Rede de seguranca:
/// mesmo que algum caminho de rede/IO dentro do librespot trave sem
/// timeout (algo que a gente nao previu), o jogo ainda assim consegue
/// fechar - ver `Drop for SpotifyConnect`.
const SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(5);

pub struct SpotifyConnect {
    spirc: Arc<Mutex<Option<Spirc>>>,
    shutdown: Arc<tokio::sync::Notify>,
    join: Option<JoinHandle<()>>,
}

impl SpotifyConnect {
    /// Sobe a thread do backend connect e volta na hora (nao bloqueia
    /// esperando o login/conexao terminar) - mesma filosofia de
    /// "nao-bloqueante" que o resto do engine ja segue pro SMTC/telemetria
    /// (ver engine.rs). Comandos mandados antes da conexao terminar
    /// simplesmente falham (ver `with_spirc`); `state.status` reflete o
    /// progresso (login pendente, conectado, erro) pro painel mostrar.
    pub fn start(state: SharedState, cache_dir: PathBuf) -> Self {
        let spirc_slot: Arc<Mutex<Option<Spirc>>> = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(tokio::sync::Notify::new());

        let slot = spirc_slot.clone();
        let shutdown_for_thread = shutdown.clone();
        let join = std::thread::Builder::new()
            .name("ets2-spotify-connect".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        set_status(&state, format!("Erro ao iniciar runtime async: {e}"));
                        return;
                    }
                };
                runtime.block_on(async move {
                    // Todo o processo de login/conexao roda dentro desse
                    // `select!` contra o sinal de shutdown - sem isso, se o
                    // usuario tentar fechar o jogo enquanto ainda estamos
                    // conectando/logando, nao haveria como abortar e essa
                    // chamada (e o `.join()` em `Drop`) ficaria esperando o
                    // login terminar sozinho. Ver `SHUTDOWN_JOIN_TIMEOUT`
                    // pra rede de seguranca adicional caso mesmo assim algo
                    // trave aqui dentro.
                    let session_result = tokio::select! {
                        biased;
                        _ = shutdown_for_thread.notified() => None,
                        res = run_session(state.clone(), cache_dir, shutdown_for_thread.clone()) => Some(res),
                    };
                    match session_result {
                        Some(Ok((spirc, spirc_task))) => {
                            *slot.lock().unwrap() = Some(spirc);
                            tokio::select! {
                                _ = spirc_task => {}
                                _ = shutdown_for_thread.notified() => {}
                            }
                            if let Some(spirc) = slot.lock().unwrap().take() {
                                let _ = spirc.shutdown();
                            }
                        }
                        Some(Err(e)) => set_status(&state, format!("Erro no Spotify Connect: {e:#}")),
                        // Shutdown pedido antes de terminar de conectar -
                        // nada pra desfazer (Spirc/Player nunca chegaram a
                        // existir), so encerra.
                        None => {}
                    }
                });
            })
            .expect("falha ao criar a thread do backend connect");

        Self {
            spirc: spirc_slot,
            shutdown,
            join: Some(join),
        }
    }

    pub fn play_pause(&self) -> Result<()> {
        self.with_spirc(|s| s.play_pause())
    }

    pub fn play(&self) -> Result<()> {
        self.with_spirc(|s| s.play())
    }

    pub fn pause(&self) -> Result<()> {
        self.with_spirc(|s| s.pause())
    }

    pub fn next(&self) -> Result<()> {
        self.with_spirc(|s| s.next())
    }

    pub fn previous(&self) -> Result<()> {
        self.with_spirc(|s| s.prev())
    }

    /// `percent` em 0-100 (mesma escala usada no painel/FFI) - convertido
    /// pra escala interna 0-65535 do protocolo Connect.
    pub fn set_volume(&self, percent: u32) -> Result<()> {
        let raw = (percent.min(100) * VOLUME_MAX / 100) as u16;
        self.with_spirc(|s| s.set_volume(raw))
    }

    /// Toca uma URI/link `spotify:...` (playlist/album/faixa) - equivalente
    /// ao `media::launch_uri` do backend SMTC, so que aqui o proprio addon
    /// e quem toca (via protocolo Connect), nao um Spotify Desktop externo.
    pub fn launch_uri(&self, uri: &str) -> Result<()> {
        self.with_spirc(|s| {
            let request = LoadRequest::from_context_uri(
                uri.to_string(),
                LoadRequestOptions {
                    start_playing: true,
                    ..Default::default()
                },
            );
            s.load(request)
        })
    }

    fn with_spirc<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&Spirc) -> std::result::Result<(), librespot_core::Error>,
    {
        let guard = self.spirc.lock().unwrap();
        let Some(spirc) = guard.as_ref() else {
            return Err(anyhow!(
                "Spotify Connect ainda nao esta pronto (login pendente ou reconectando)"
            ));
        };
        f(spirc).map_err(|e| anyhow!("{e}"))
    }
}

impl Drop for SpotifyConnect {
    fn drop(&mut self) {
        self.shutdown.notify_one();
        let Some(join) = self.join.take() else { return };

        // Join com timeout: mesmo que a thread do backend connect nao
        // termine a tempo (ex.: um caminho de rede/IO dentro do librespot
        // que a gente nao previu e que trava sem timeout proprio, igual o
        // bug que `login_with_timeout` corrige pro caso conhecido), isso
        // NAO deve travar o fechamento do jogo por causa disso - so paramos
        // de esperar e deixamos essa thread terminar sozinha em segundo
        // plano (o processo inteiro sendo encerrado limpa tudo de qualquer
        // jeito). `JoinHandle::join` do std nao tem timeout embutido, entao
        // fazemos isso com uma thread auxiliar + canal.
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = std::thread::Builder::new()
            .name("ets2-spotify-connect-join-waiter".into())
            .spawn(move || {
                let _ = join.join();
                let _ = tx.send(());
            });
        if let Ok(waiter) = waiter {
            if rx.recv_timeout(SHUTDOWN_JOIN_TIMEOUT).is_err() {
                // Timeout - segue em frente. A thread auxiliar continua
                // esperando o join original terminar sozinha; nao a
                // detach()amos porque join() de uma JoinHandle ja
                // consumida so pode ser feito uma vez, e isso e exatamente
                // o que ela esta fazendo, so que sem mais ninguem esperar
                // por ela.
                drop(waiter);
            }
        }
    }
}

/// Faz login (cache em disco, ou OAuth se ainda nao tiver credenciais
/// salvas) e sobe a sessao + player + Spirc. So retorna depois que o login
/// termina - por isso e chamado de dentro da thread dedicada (ver `start`),
/// nunca do thread do engine principal.
async fn run_session(
    state: SharedState,
    cache_dir: PathBuf,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<(Spirc, impl std::future::Future<Output = ()>)> {
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("nao foi possivel criar {}", cache_dir.display()))?;
    let cache = Cache::new(Some(&cache_dir), None, None, None)
        .context("nao foi possivel abrir o cache de credenciais do librespot")?;

    let session_config = SessionConfig::default();

    let credentials = match cache.credentials() {
        Some(creds) => {
            set_status(&state, "Reconectando ao Spotify (login salvo)...".into());
            creds
        }
        None => {
            set_status(
                &state,
                "Login necessario: abrindo o navegador para autorizar o Spotify...".into(),
            );
            let client = OAuthClientBuilder::new(
                &session_config.client_id,
                OAUTH_REDIRECT_URI,
                OAUTH_SCOPES.to_vec(),
            )
            .open_in_browser()
            .build()
            .context("nao foi possivel preparar o login OAuth")?;
            let token = login_with_timeout(client, &shutdown).await?;
            Credentials::with_access_token(token.access_token)
        }
    };

    let session = Session::new(session_config, Some(cache));

    let mut player_config = PlayerConfig::default();
    // Sem isso o `Player` so emite posicao em eventos "de borda" (trocou de
    // faixa, pausou, retomou) - com isso ele tambem emite
    // `PlayerEvent::PositionChanged` periodicamente enquanto toca, o que a
    // barra de progresso do painel precisa pra andar (ver
    // `spawn_player_event_task`).
    player_config.position_update_interval = Some(POSITION_UPDATE_INTERVAL);
    let backend = audio_backend::find(None)
        .ok_or_else(|| anyhow!("nenhum backend de audio disponivel (rodio-backend)"))?;
    let mixer_fn =
        mixer::find(None).ok_or_else(|| anyhow!("nenhum mixer de audio disponivel"))?;
    let mixer = mixer_fn(MixerConfig::default()).context("nao foi possivel abrir o mixer")?;
    let volume_getter = mixer.get_soft_volume();
    let initial_volume_percent = ConnectConfig::default().initial_volume as u32 * 100 / VOLUME_MAX;

    let player = Player::new(player_config, session.clone(), volume_getter, move || {
        backend(None, AudioFormat::default())
    });

    spawn_player_event_task(state.clone(), player.get_player_event_channel());

    let connect_config = ConnectConfig {
        name: "ETS2".to_string(),
        device_type: DeviceType::Automobile,
        ..Default::default()
    };

    let (spirc, spirc_task) = Spirc::new(connect_config, session, credentials, player, mixer)
        .await
        .context("nao foi possivel iniciar o dispositivo Spotify Connect")?;

    // Ativa o "ETS2" como dispositivo Connect automaticamente - sem isso o
    // usuario precisaria abrir o Spotify em outro aparelho e escolher
    // "ETS2" na lista de dispositivos manualmente antes de qualquer
    // comando daqui funcionar. Erro aqui nao e fatal (ex.: nenhuma sessao
    // Spotify ativa em lugar nenhum ainda pra "assumir") - o dispositivo
    // continua aparecendo na lista normalmente, so nao comeca pre-ativado.
    if let Ok(mut s) = state.lock() {
        s.volume = initial_volume_percent;
    }
    match spirc.activate() {
        Ok(()) => set_status(&state, "Conectado como dispositivo Connect \"ETS2\".".into()),
        Err(e) => set_status(
            &state,
            format!(
                "Conectado, mas nao foi possivel auto-ativar o dispositivo ({e}) - escolha \"ETS2\" nos dispositivos do Spotify."
            ),
        ),
    }

    Ok((spirc, spirc_task))
}

/// Espera o usuario terminar o login no navegador, sem travar a thread do
/// tokio indefinidamente se ele nunca terminar.
///
/// `client.get_access_token()` (librespot-oauth) e sincrona/bloqueante -
/// por baixo, espera uma conexao TCP chegar (`TcpListener::incoming()`)
/// sem NENHUM timeout proprio nem forma de ser cancelada de fora. Chamar
/// isso direto dentro de uma `async fn` prenderia a thread do runtime
/// tokio pra sempre caso o usuario feche o navegador sem logar, o que por
/// sua vez travaria o shutdown do addon (e o fechamento do jogo) esperando
/// essa thread terminar.
///
/// A solucao: rodar a chamada bloqueante numa thread `std` comum,
/// deliberadamente solta (nao rastreada por nenhum `JoinHandle` nosso) -
/// se o timeout ou o shutdown vencerem a corrida primeiro, essa thread
/// continua bloqueada em segundo plano ate o processo inteiro terminar
/// (que e quando o Windows recupera tudo de qualquer forma), mas isso NAO
/// impede o resto do addon/jogo de fechar normalmente, porque nada mais
/// espera por ela.
async fn login_with_timeout(client: OAuthClient, shutdown: &tokio::sync::Notify) -> Result<OAuthToken> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("ets2-spotify-oauth-login".into())
        .spawn(move || {
            // Erro ignorado de proposito: se ninguem esta mais esperando
            // (timeout/shutdown ja venceram a corrida), o receiver foi
            // dropado e o send simplesmente falha - nao ha nada a fazer.
            let _ = tx.send(client.get_access_token());
        })
        .context("nao foi possivel criar a thread de login OAuth")?;

    tokio::select! {
        biased;
        _ = shutdown.notified() => Err(anyhow!("login cancelado (jogo fechando)")),
        _ = tokio::time::sleep(LOGIN_TIMEOUT) => Err(anyhow!(
            "tempo esgotado esperando o login no navegador ({}s)", LOGIN_TIMEOUT.as_secs()
        )),
        result = rx => result
            .context("canal de login OAuth fechado inesperadamente")?
            .context("login OAuth falhou ou foi cancelado"),
    }
}

/// Consome os eventos do `Player` (faixa trocou, tocando, pausado, parado,
/// progresso, volume) numa task separada e atualiza `now_playing`/`status`/
/// `volume` no `SharedState` - diferente do backend SMTC, aqui isso e
/// orientado a evento (nao polling), entao escreve direto em vez de
/// esperar `engine.rs` perguntar.
fn spawn_player_event_task(state: SharedState, mut events: PlayerEventChannel) {
    tokio::spawn(async move {
        let http = reqwest::Client::new();
        // Incrementado a cada capa nova baixada com sucesso - mesma
        // convencao de `ThumbnailData::generation` que o backend SMTC ja
        // usa (engine.rs), pro C++ so recriar a textura da GPU quando o
        // valor muda.
        let mut thumb_generation: u64 = 0;

        while let Some(event) = events.recv().await {
            match event {
                PlayerEvent::TrackChanged { audio_item } => {
                    let (artist, album) = match &audio_item.unique_fields {
                        UniqueFields::Track { artists, album, .. } => (
                            artists
                                .0
                                .iter()
                                .map(|a| a.name.clone())
                                .collect::<Vec<_>>()
                                .join(", "),
                            album.clone(),
                        ),
                        _ => (String::new(), String::new()),
                    };
                    if let Ok(mut s) = state.lock() {
                        s.now_playing = Some(NowPlayingInfo {
                            title: audio_item.name.clone(),
                            artist,
                            album,
                            thumbnail: None,
                            position_ms: 0,
                            duration_ms: audio_item.duration_ms,
                        });
                    }

                    // A menor capa disponivel ja e suficiente pro tamanho
                    // que o painel desenha (64x64, ver overlay_addon.cpp) -
                    // evita baixar/decodificar a versao grande a toa.
                    if let Some(cover) = audio_item
                        .covers
                        .iter()
                        .filter(|c| c.width > 0 && c.height > 0)
                        .min_by_key(|c| c.width * c.height)
                    {
                        if let Ok(resp) = http.get(&cover.url).send().await {
                            if let Ok(bytes) = resp.bytes().await {
                                if let Ok(img) = image::load_from_memory(&bytes) {
                                    let rgba = img.to_rgba8();
                                    let (w, h) = rgba.dimensions();
                                    thumb_generation += 1;
                                    if let Ok(mut s) = state.lock() {
                                        if let Some(np) = s.now_playing.as_mut() {
                                            np.thumbnail = Some(ThumbnailData {
                                                width: w,
                                                height: h,
                                                rgba: rgba.into_raw(),
                                                generation: thumb_generation,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                PlayerEvent::Playing { position_ms, .. }
                | PlayerEvent::Paused { position_ms, .. }
                | PlayerEvent::PositionChanged { position_ms, .. }
                | PlayerEvent::PositionCorrection { position_ms, .. }
                | PlayerEvent::Seeked { position_ms, .. } => {
                    if let Ok(mut s) = state.lock() {
                        if let Some(np) = s.now_playing.as_mut() {
                            np.position_ms = position_ms;
                        }
                    }
                }
                PlayerEvent::VolumeChanged { volume } => {
                    if let Ok(mut s) = state.lock() {
                        s.volume = volume as u32 * 100 / VOLUME_MAX;
                    }
                }
                PlayerEvent::Stopped { .. } => {
                    if let Ok(mut s) = state.lock() {
                        s.now_playing = None;
                    }
                }
                _ => {}
            }
        }
    });
}

fn set_status(state: &SharedState, msg: String) {
    if let Ok(mut s) = state.lock() {
        s.status = msg;
    }
}
