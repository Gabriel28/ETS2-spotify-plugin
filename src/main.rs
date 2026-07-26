//! ets2-spotify-bridge
//!
//! App desktop (nao uma DLL injetada no ETS2) no estilo "launcher", parecido
//! com o Content Manager do Assetto Corsa: uma janela com abas pra ver o
//! que esta tocando, escolher playlists e configurar atalhos de teclado.
//! Roda ao lado do jogo no Windows e:
//!
//!   1. Liga/pausa o Spotify automaticamente quando voce liga/desliga a
//!      parte eletrica do caminhao, e pausa quando o jogo e pausado (menu/alt-tab).
//!   2. Deixa escolher playlists salvas (tocam via link/URI do Spotify,
//!      sem OAuth) e fazer buscas rapidas.
//!   3. Tem atalhos de teclado globais configuraveis pela propria UI, pra
//!      controlar play/pause/next/previous com o volante ou teclado.
//!
//! Ver README.md pra explicacao completa de como cada parte funciona.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod engine;
mod hotkeys;
mod media;
mod state;
mod telemetry;

use eframe::egui;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};
use state::{Command, SharedState, UiState};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

/// Inicializa o apartment COM/WinRT nesta thread. Necessario porque as APIs
/// de SMTC (windows::Media::Control) sao WinRT e cada thread que as usa
/// precisa inicializar o runtime antes. Chamar isso mais de uma vez na
/// mesma thread e seguro (o erro de "ja inicializado" e ignorado).
pub fn init_apartment() {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    NowPlaying,
    Playlists,
    Settings,
}

struct App {
    state: SharedState,
    cmd_tx: mpsc::Sender<Command>,
    running: Arc<AtomicBool>,
    config: config::Config,
    tab: Tab,

    hotkey_manager: GlobalHotKeyManager,
    registered: HashMap<u32, Command>,
    capturing: Option<&'static str>,

    new_playlist_name: String,
    new_playlist_uri: String,
    search_query: String,

    texture: Option<egui::TextureHandle>,
    texture_generation: u64,
}

impl App {
    fn new() -> Self {
        init_apartment();

        let config = config::load();
        let state: SharedState = Arc::new(Mutex::new(UiState::default()));
        let running = Arc::new(AtomicBool::new(true));
        let (cmd_tx, cmd_rx) = mpsc::channel();

        {
            let state = state.clone();
            let running = running.clone();
            thread::spawn(move || engine::run(state, cmd_rx, running));
        }

        let hotkey_manager =
            GlobalHotKeyManager::new().expect("falha ao criar gerenciador de hotkeys");

        let mut app = Self {
            state,
            cmd_tx,
            running,
            config,
            tab: Tab::NowPlaying,
            hotkey_manager,
            registered: HashMap::new(),
            capturing: None,
            new_playlist_name: String::new(),
            new_playlist_uri: String::new(),
            search_query: String::new(),
            texture: None,
            texture_generation: 0,
        };
        app.reload_hotkeys();
        app
    }

    /// Recria o gerenciador de hotkeys e registra de novo a partir da
    /// config atual. A crate `global-hotkey` nao oferece um jeito simples
    /// de "trocar" uma hotkey ja registrada, entao recriar o manager inteiro
    /// e a forma mais segura de aplicar mudancas.
    fn reload_hotkeys(&mut self) {
        self.hotkey_manager =
            GlobalHotKeyManager::new().expect("falha ao recriar gerenciador de hotkeys");
        self.registered.clear();

        let specs = [
            (self.config.hotkeys.play_pause.clone(), Command::PlayPause),
            (self.config.hotkeys.next.clone(), Command::Next),
            (self.config.hotkeys.previous.clone(), Command::Previous),
        ];

        for (spec, cmd) in specs {
            let Some(hk) = hotkeys::parse_hotkey(&spec) else {
                continue;
            };
            if self.hotkey_manager.register(hk).is_ok() {
                self.registered.insert(hk.id(), cmd);
            }
        }
    }

    fn sync_texture(&mut self, ctx: &egui::Context) {
        if let Ok(state) = self.state.lock() {
            if let Some(np) = &state.now_playing {
                if let Some(thumb) = &np.thumbnail {
                    if thumb.generation != self.texture_generation {
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(
                            [thumb.width as usize, thumb.height as usize],
                            &thumb.rgba,
                        );
                        self.texture =
                            Some(ctx.load_texture("album_art", color_image, Default::default()));
                        self.texture_generation = thumb.generation;
                    }
                }
            }
        }
    }

    fn ui_now_playing(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.sync_texture(ctx);

        let snapshot = self.state.lock().ok().map(|s| {
            (
                s.now_playing.clone(),
                s.electrics_on,
                s.game_paused,
                s.telemetry_connected,
            )
        });

        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            if let Some(tex) = &self.texture {
                ui.add(egui::Image::new(tex).fit_to_exact_size(egui::vec2(160.0, 160.0)));
            } else {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(160.0, 160.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 8.0, egui::Color32::from_gray(40));
            }
            ui.add_space(16.0);
            ui.vertical(|ui| {
                match &snapshot {
                    Some((Some(np), electrics_on, paused, connected)) => {
                        ui.heading(&np.title);
                        ui.label(&np.artist);
                        ui.weak(&np.album);
                        ui.add_space(8.0);
                        ui.label(format!(
                            "Eletrica: {}   |   Jogo: {}   |   Telemetria: {}",
                            if *electrics_on { "ligada" } else { "desligada" },
                            if *paused { "pausado" } else { "rodando" },
                            if *connected { "conectada" } else { "sem conexao" },
                        ));
                    }
                    _ => {
                        ui.heading("Nada tocando");
                        ui.weak("Abra o Spotify e toque alguma coisa.");
                    }
                }
            });
        });

        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            if ui.button("<<  Anterior").clicked() {
                let _ = self.cmd_tx.send(Command::Previous);
            }
            if ui.button("Play / Pause").clicked() {
                let _ = self.cmd_tx.send(Command::PlayPause);
            }
            if ui.button("Proxima  >>").clicked() {
                let _ = self.cmd_tx.send(Command::Next);
            }
        });

        ui.add_space(24.0);
        ui.separator();
        ui.label("Log:");
        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
            if let Ok(state) = self.state.lock() {
                for line in state.log.iter().rev().take(50) {
                    ui.weak(line);
                }
            }
        });
    }

    fn ui_playlists(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.0);
        ui.heading("Suas playlists");
        ui.weak(
            "Cole o link ou URI da playlist do Spotify (botao direito na playlist > \
             Compartilhar > Copiar link/URI da playlist).",
        );
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            ui.label("Nome:");
            ui.text_edit_singleline(&mut self.new_playlist_name);
            ui.label("Link/URI:");
            ui.text_edit_singleline(&mut self.new_playlist_uri);
            if ui.button("Adicionar").clicked() && !self.new_playlist_uri.trim().is_empty() {
                self.config.playlists.push(config::Playlist {
                    name: if self.new_playlist_name.trim().is_empty() {
                        "Sem nome".to_string()
                    } else {
                        self.new_playlist_name.clone()
                    },
                    uri: normalize_spotify_link(self.new_playlist_uri.trim()),
                });
                self.new_playlist_name.clear();
                self.new_playlist_uri.clear();
                let _ = config::save(&self.config);
            }
        });

        ui.add_space(16.0);
        let mut to_remove = None;
        let mut to_play = None;
        egui::Grid::new("playlists_grid")
            .num_columns(3)
            .spacing([12.0, 12.0])
            .show(ui, |ui| {
                for (i, pl) in self.config.playlists.iter().enumerate() {
                    ui.label(&pl.name);
                    if ui.button("Tocar").clicked() {
                        to_play = Some(pl.uri.clone());
                    }
                    if ui.button("Remover").clicked() {
                        to_remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(uri) = to_play {
            let _ = self.cmd_tx.send(Command::PlayUri(uri));
        }
        if let Some(i) = to_remove {
            self.config.playlists.remove(i);
            let _ = config::save(&self.config);
        }

        ui.add_space(24.0);
        ui.separator();
        ui.heading("Buscar no Spotify");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.search_query);
            if ui.button("Buscar").clicked() && !self.search_query.trim().is_empty() {
                let _ = self
                    .cmd_tx
                    .send(Command::SearchAndOpen(self.search_query.clone()));
            }
        });
        ui.weak(
            "Abre o Spotify direto na busca com o termo preenchido - os resultados aparecem \
             la mesmo, o app nao lista musicas aqui.",
        );
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(16.0);
        ui.heading("Atalhos de teclado");
        ui.weak("Clique em \"Definir\" e aperte a combinacao desejada (ex.: Ctrl+Alt+Espaco).");
        ui.add_space(12.0);

        self.hotkey_row(ui, ctx, "Play / Pause", "play_pause");
        self.hotkey_row(ui, ctx, "Proxima faixa", "next");
        self.hotkey_row(ui, ctx, "Faixa anterior", "previous");

        ui.add_space(24.0);
        if ui.button("Salvar atalhos").clicked() {
            let _ = config::save(&self.config);
            self.reload_hotkeys();
        }

        ui.add_space(24.0);
        ui.separator();
        ui.weak(
            "Dica: mapeie a mesma combinacao no software do seu volante (Logitech Hub, \
             Thrustmaster Panel, SimHub etc.) pra ter um botao fisico, sem depender do ETS2.",
        );
    }

    fn hotkey_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, label: &str, key: &'static str) {
        let current_display = match key {
            "play_pause" => self.config.hotkeys.play_pause.clone(),
            "next" => self.config.hotkeys.next.clone(),
            _ => self.config.hotkeys.previous.clone(),
        };

        ui.horizontal(|ui| {
            ui.label(label);
            ui.add_space(8.0);
            ui.monospace(current_display.as_str());
            ui.add_space(8.0);
            let capturing_this = self.capturing == Some(key);
            let btn_text = if capturing_this { "Aperte uma tecla..." } else { "Definir" };
            if ui.button(btn_text).clicked() {
                self.capturing = Some(key);
            }
        });

        if self.capturing == Some(key) {
            let modifiers = ctx.input(|i| i.modifiers);
            let pressed_key = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, .. } => Some(*key),
                    _ => None,
                })
            });
            if let Some(k) = pressed_key {
                let spec = hotkeys::format_from_egui(modifiers, k);
                match key {
                    "play_pause" => self.config.hotkeys.play_pause = spec,
                    "next" => self.config.hotkeys.next = spec,
                    _ => self.config.hotkeys.previous = spec,
                }
                self.capturing = None;
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if let Some(cmd) = self.registered.get(&event.id).cloned() {
                let _ = self.cmd_tx.send(cmd);
            }
        }

        apply_dark_theme(ctx);

        egui::SidePanel::left("sidebar").resizable(false).min_width(170.0).show(ctx, |ui| {
            ui.add_space(12.0);
            ui.heading("ETS2 x Spotify");
            ui.add_space(20.0);
            if ui.selectable_label(self.tab == Tab::NowPlaying, "  Tocando agora").clicked() {
                self.tab = Tab::NowPlaying;
            }
            if ui.selectable_label(self.tab == Tab::Playlists, "  Playlists").clicked() {
                self.tab = Tab::Playlists;
            }
            if ui.selectable_label(self.tab == Tab::Settings, "  Configuracoes").clicked() {
                self.tab = Tab::Settings;
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::NowPlaying => self.ui_now_playing(ui, ctx),
            Tab::Playlists => self.ui_playlists(ui),
            Tab::Settings => self.ui_settings(ui, ctx),
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(300));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.running.store(false, Ordering::SeqCst);
    }
}

/// Aceita tanto `https://open.spotify.com/playlist/ID?...` quanto
/// `spotify:playlist:ID` e normaliza pro formato de URI que o
/// `SpotifyMedia::launch_uri` usa.
fn normalize_spotify_link(input: &str) -> String {
    if input.starts_with("spotify:") {
        return input.to_string();
    }
    if let Some(rest) = input.split("open.spotify.com/").nth(1) {
        let clean = rest.split('?').next().unwrap_or("");
        let mut parts = clean.split('/');
        if let (Some(kind), Some(id)) = (parts.next(), parts.next()) {
            if !kind.is_empty() && !id.is_empty() {
                return format!("spotify:{kind}:{id}");
            }
        }
    }
    input.to_string()
}

fn apply_dark_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(18, 18, 22);
    visuals.window_fill = egui::Color32::from_rgb(24, 24, 28);
    visuals.override_text_color = Some(egui::Color32::from_gray(230));
    ctx.set_visuals(visuals);
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native(
        "ETS2 x Spotify Bridge",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
