# ets2-spotify-plugin

Addon do [ReShade](https://reshade.me) que controla o Spotify **de dentro do
próprio jogo** no **Euro Truck Simulator 2** / **American Truck Simulator**:
play/pause automático baseado na elétrica do caminhão, um painel dentro do
jogo com play/pause manual, anterior/próxima e playlists salvas, e hotkeys
globais dentro do jogo — sem login OAuth nem API key do Spotify, e **sem
precisar de nenhum app separado rodando ao lado**.

Compatível com qualquer caminhão: o painel é um overlay de tela (via
ReShade), não depende do modelo/interior de nenhum veículo específico, e a
leitura da elétrica/pausa do jogo já é genérica em nível de SDK da SCS.

## Features

- Play/pause automático baseado na telemetria do jogo (elétrica ligada/
  desligada, jogo pausado/despausado).
- Painel dentro do jogo (tecla **F9**, ver [`reshade-addon/`](reshade-addon))
  com capa do álbum, título/artista, botões de play/pause/anterior/próxima
  (com tooltip ao passar o mouse) e playlists salvas.
- Hotkeys globais dentro do jogo, funcionam com o painel aberto ou fechado
  — ver [Hotkeys](#hotkeys) abaixo.
- Sem OAuth, sem API key, sem cadastro — usa o controle de mídia nativo do
  Windows (SMTC) e links `spotify:`.
- Escolha do dispositivo de saída de áudio usado **só pelo Spotify**
  (independente do padrão do Windows) — hoje só via `config.json`, ver
  [Playlists e config](#playlists-e-config).

## Pré-requisitos

- Windows 10/11.
- Spotify **Desktop** instalado e logado (a versão web não responde ao SMTC
  nem a links `spotify:`).
- [ReShade](https://reshade.me) instalado no ETS2/ATS (escolha o `.exe` do
  jogo — `eurotrucks2.exe`/`amtrucks.exe`, dentro de `bin\win_x64\` — durante
  a instalação, API gráfica D3D11).
- [RenCloud/scs-sdk-plugin](https://github.com/RenCloud/scs-sdk-plugin) —
  necessário só para o play/pause automático via telemetria. Sem ele o
  painel funciona normalmente no modo manual (botões, hotkeys, playlists).
- [Rust](https://rustup.rs) + Visual Studio (Community serve, gratuito) com
  "Desktop development with C++" — só pra **compilar** o addon.

## Instalação

1. Baixe o release mais recente do
   [scs-sdk-plugin](https://github.com/RenCloud/scs-sdk-plugin/releases) e
   copie a DLL para a pasta `plugins` dentro da instalação do jogo, por
   exemplo `<pasta do jogo>\bin\win_x64\plugins\` (crie a pasta `plugins` se
   ela não existir).
2. Compile o addon:
   ```powershell
   cd reshade-addon
   .\build.bat
   ```
   Isso compila a lib Rust (`cargo build --release`) e linka tudo numa DLL
   só: `reshade-addon\overlay_addon.dll`.
3. Renomeie `overlay_addon.dll` para `overlay_addon.addon` e copie pra pasta
   de addons do ReShade — por padrão a mesma pasta onde o `ReShade64.dll`
   foi instalado, ex. `<pasta do jogo>\bin\win_x64\`.
4. Abra o Spotify, abra o jogo. Aperte **Home** pra abrir o menu do ReShade
   uma vez (só pra confirmar que carregou — aparece na aba "Add-ons" com o
   nome "ETS2 x Spotify"). O HUD compacto (capa + faixa + artista) aparece
   sozinho; aperte **F9** dentro do jogo pra abrir o painel completo.

## Uso

- **HUD compacto** (sempre visível): capa do álbum, título, artista, e
  status da elétrica/telemetria.
- **F9**: abre/fecha o painel expandido — botões `|<<` (anterior), `>||`
  (play/pause) e `>>|` (próxima), com tooltip ao passar o mouse, e a lista
  de playlists salvas (clique pra tocar). Enquanto o painel está aberto, o
  teclado fica capturado pelo painel (não mexe no volante/câmera do
  caminhão).
- A elétrica do caminhão liga/pausa o Spotify automaticamente (chave na
  posição antes de dar partida no motor, igual um caminhão de verdade).

### Hotkeys

Funcionam com o jogo em foco, painel aberto ou fechado:

| Ação          | Tecla            |
| ------------- | ---------------- |
| Próxima faixa | `Ctrl + PageUp`   |
| Faixa anterior| `Ctrl + PageDown` |
| Play          | `Ctrl + Insert`   |
| Pause         | `Ctrl + Delete`   |

Pra mudar, edite as constantes `kHotkeyNext`/`kHotkeyPrevious`/
`kHotkeyPlay`/`kHotkeyPause` no topo de
[`reshade-addon/overlay_addon.cpp`](reshade-addon/overlay_addon.cpp) e
recompile.

### Por que não tem busca

O SMTC (a API do Windows usada aqui pra controlar/ler o Spotify sem OAuth)
não tem "buscar no catálogo" — só controla a sessão que já está ativa.
Buscar de verdade com resultados dentro do painel exigiria a API Web
oficial do Spotify com login OAuth, o que este projeto evita de propósito
(sem cadastro, sem credenciais). Pra tocar algo específico escolhido na
hora, abra o Spotify normalmente por fora do jogo e escolha lá — o play/
pause/anterior/próxima daqui dentro do caminhão continuam controlando o
que estiver tocando, seja lá o que for. Playlists salvas (abaixo) cobrem o
caso de reabrir algo sem sair do jogo.

### Playlists e config

A configuração (playlists salvas, dispositivo de áudio preferido) fica em
`%APPDATA%\ets2-spotify-bridge\config.json`. Ainda não existe um formulário
dentro do painel pra adicionar playlist nova — edite o arquivo à mão. Use
[`config.example.json`](config.example.json) como modelo:

```json
{
  "playlists": [
    { "name": "Rock de estrada", "uri": "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M" },
    { "name": "Sertanejo", "uri": "https://open.spotify.com/playlist/37i9dQZF1DX0SM0LYsmbMT" }
  ],
  "preferred_output_device": null
}
```

- `uri` aceita tanto `spotify:playlist:<id>` quanto um link
  `https://open.spotify.com/playlist/<id>` (funciona igual pra álbum/
  faixa/artista, não só playlist) — o addon converte sozinho.
- Pra pegar o link/URI de uma playlist: no Spotify, clique nos "..." dela →
  Compartilhar → Copiar link (ou Copiar URI da Spotify, se aparecer).
- Depois de editar e salvar o arquivo, feche e abra o jogo de novo (a
  config é lida uma vez, quando o addon inicia).
- `preferred_output_device`: `null` usa o padrão do Windows; pra usar um
  dispositivo específico, o id vem de `ets2_list_output_devices` (ver
  [`reshade-addon/README.md`](reshade-addon/README.md)) — hoje só
  acessível via essa função FFI, ainda sem seletor no painel.

## Estrutura do projeto

```
config.example.json  # modelo de config.json (playlists) pra copiar/editar

src/                # staticlib Rust (linkada dentro do addon, ver Cargo.toml [lib])
  lib.rs             # raiz do crate
  ffi.rs             # superficie extern "C" chamada pelo addon C++
  engine.rs           # thread unica dona da conexao com o Spotify (SMTC) + telemetria
  media.rs             # controle/leitura do Spotify via SMTC + links spotify:
  telemetry.rs         # leitura da telemetria do ETS2/ATS (eletrica, pausa)
  audio_device.rs       # troca do dispositivo de saida de audio so do Spotify
  config.rs              # persistencia de playlists e audio em JSON
  state.rs                # tipos compartilhados entre a thread motor e o addon

reshade-addon/       # addon C++ (ver README proprio) - so ReShade/ImGui,
                      # a integracao de verdade vive na staticlib acima
  overlay_addon.cpp   # painel, icones, hotkeys globais (kHotkey*)
  ets2_ffi.h          # espelha src/ffi.rs pro lado C++
  build.bat           # compila a staticlib Rust E o addon C++, linka tudo numa DLL
```

## Licença

[MIT](LICENSE)
