# ets2-spotify-plugin

App desktop em Rust que liga o Spotify à ignição do seu caminhão no
**Euro Truck Simulator 2** / **American Truck Simulator**: dá play quando
você liga a parte elétrica (antes de dar partida no motor, como num
caminhão de verdade) e pausa quando desliga ou pausa o jogo — sem usar
login OAuth nem API key do Spotify.

## Features

- Play/pause automático baseado na telemetria do jogo (elétrica ligada/
  desligada, jogo pausado/despausado).
- Playlists salvas: cole um link ou URI do Spotify e toque com um clique.
- Busca rápida que abre o Spotify já com o termo preenchido.
- Hotkeys globais configuráveis (play/pause, próxima, anterior).
- Sem OAuth, sem API key, sem cadastro — usa o controle de mídia nativo
  do Windows (SMTC) e links `spotify:`.

## Pré-requisitos

- Windows 10/11.
- [Rust](https://rustup.rs) (toolchain `stable-x86_64-pc-windows-msvc`).
- Spotify **Desktop** instalado e logado (a versão web não responde ao
  SMTC nem a links `spotify:`).
- [RenCloud/scs-sdk-plugin](https://github.com/RenCloud/scs-sdk-plugin) —
  necessário só para o play/pause automático via telemetria. Sem ele o
  app funciona normalmente no modo manual (botões, hotkeys, playlists).

## Instalação

1. Baixe o release mais recente do
   [scs-sdk-plugin](https://github.com/RenCloud/scs-sdk-plugin/releases)
   e copie a DLL para a pasta `plugins` dentro da instalação do jogo,
   por exemplo:
   ```
   <pasta do jogo>\bin\win_x64\plugins\
   ```
   (crie a pasta `plugins` se ela não existir).
2. Compile o app:
   ```powershell
   cargo build --release
   ```
   O executável fica em `target\release\ets2-spotify-bridge.exe`.
3. Abra o Spotify, abra o jogo e rode o `.exe`. Pode deixar a janela
   minimizada em segundo plano, ou criar um atalho na pasta de
   inicialização do Windows para abrir junto com o PC.

## Uso

A janela tem três abas:

- **Tocando agora** — capa do álbum, status da elétrica/telemetria,
  controles de anterior/play-pause/próxima e um log de atividade.
- **Playlists** — cole um link (`https://open.spotify.com/playlist/...`)
  ou URI (`spotify:playlist:...`), dê um nome e clique "Adicionar".
- **Configurações** — defina as combinações de hotkey e salve.

A configuração (playlists + atalhos) fica em
`%APPDATA%\ets2-spotify-bridge\config.json`.

## Estrutura do projeto

```
src/
  main.rs      # janela (eframe/egui): abas, botoes, captura de hotkey
  engine.rs    # thread unica dona da conexao com o Spotify (SMTC) + telemetria
  media.rs     # controle/leitura do Spotify via SMTC + links spotify:
  telemetry.rs # leitura da telemetria do ETS2/ATS (eletrica, pausa)
  hotkeys.rs   # parse/captura de combinacoes de tecla
  config.rs    # persistencia de hotkeys e playlists em JSON
  state.rs     # tipos compartilhados entre a thread motor e a UI
```

## Licença

[MIT](LICENSE)
