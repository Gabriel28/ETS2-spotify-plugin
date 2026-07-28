# overlay-addon (ReShade)

Addon do [ReShade](https://reshade.me) que roda dentro do próprio jogo e dá
controle completo do Spotify: HUD compacto sempre visível (capa/faixa/
artista), um painel expandido (tecla **F9**) com play/pause/anterior/
próxima (ícones com tooltip) e playlists salvas, e hotkeys globais dentro
do jogo (Ctrl+PageUp/PageDown/Insert/Delete). Não depende de nenhum app
separado — é o projeto inteiro (ver [README.md da raiz](../README.md)).

## Como funciona

Diferente da versão antiga (bridge em processo separado + addon só leitor),
agora **toda** a integração real — SMTC do Spotify, telemetria do jogo,
dispositivo de áudio, config em disco — é uma **staticlib Rust**
(`../src/*.rs`) compilada e **linkada dentro desta mesma DLL** pelo
`build.bat`. Este arquivo (`overlay_addon.cpp`) só cuida do que só o C++
pode fazer aqui: registrar o addon no ReShade e desenhar os widgets do
ImGui (que o próprio ReShade já tem carregado dentro do processo do jogo).
Não há mais memória compartilhada nem processos separados — a comunicação
entre a lógica Rust e o desenho C++ é só chamada de função direta, ver
`ets2_ffi.h` (espelha `src/ffi.rs` à mão, mesma disciplina que o projeto já
usava pra sincronizar a antiga struct de shared memory entre as duas
linguagens, só que agora é assinatura de função em vez de struct binária).

Não há hook de DirectX feito por este projeto — quem já faz esse trabalho
pesado (hookar a renderização do jogo) é o ReShade; o addon só usa a API
dele.

## Pré-requisitos

1. **ReShade instalado no ETS2/ATS.** Baixe em https://reshade.me, escolha
   o `.exe` do jogo (`eurotrucks2.exe` / `amtrucks.exe`, dentro de
   `bin\win_x64\`) durante a instalação, e selecione a API gráfica correta
   (D3D11, que é o padrão do jogo). Pode pular a instalação de shaders/
   presets — não são necessários pra esse addon.
2. **Visual Studio** (Community serve, gratuito) com o workload "Desktop
   development with C++" instalado.
3. **[Rust](https://rustup.rs)** (toolchain `stable-x86_64-pc-windows-msvc`)
   — o `build.bat` chama `cargo build` automaticamente.

## Compilar

```powershell
.\build.bat
```

Primeiro compila a staticlib Rust (`cargo build --release --lib`, gera
`..\target\release\ets2_spotify_core.lib`), depois compila e linka
`overlay_addon.cpp` junto com essa lib numa DLL só:
`overlay_addon.dll`. O script acha o Visual Studio automaticamente (via
`vswhere`, com fallback pras instalações padrão do VS 2022); se falhar,
abra um "Developer Command Prompt for VS" e rode o `cl.exe` manualmente com
os mesmos parâmetros do `build.bat` (depois de rodar `cargo build --release
--lib` na raiz do projeto).

## Instalar

1. Renomeie `overlay_addon.dll` para `overlay_addon.addon`.
2. Copie pra pasta de addons do ReShade — por padrão é a mesma pasta onde o
   `ReShade64.dll` foi instalado, ex.:
   ```
   <pasta do jogo>\bin\win_x64\
   ```
   (o instalador do ReShade também oferece configurar uma pasta de addons
   separada; se você escolheu isso, use a que você configurou).
3. Abra o Spotify, abra o jogo. Aperte **Home** pra abrir o menu do ReShade
   uma vez (só pra confirmar que carregou o addon — aparece na aba
   "Add-ons" com o nome "ETS2 x Spotify"). O HUD compacto aparece sozinho;
   aperte **F9** dentro do jogo pra abrir o painel completo (play/pause,
   anterior/próxima, playlists). As hotkeys globais (Ctrl+PageUp/PageDown/
   Insert/Delete) funcionam com o painel aberto ou fechado — ver
   [README.md da raiz](../README.md#hotkeys).

## Superfície FFI (`ets2_ffi.h` / `../src/ffi.rs`)

| Função                                        | O que faz                                                              |
| ---------------------------------------------- | ------------------------------------------------------------------------ |
| `ets2_engine_start`/`ets2_engine_shutdown`     | Inicia/para a thread motor (Rust) - start é chamado de forma preguiçosa no primeiro frame, nunca do `DllMain` (risco de deadlock sob loader lock). |
| `ets2_poll_snapshot`                            | Copia título/artista/elétrica/telemetria/geração da capa - chamado uma vez por frame. |
| `ets2_get_thumbnail`                            | Copia os pixels RGBA da capa atual pro buffer do C++, que cria a textura via `reshade::api::device`. |
| `ets2_send_command`                             | Manda play/pause/play/pause-deterministico/anterior/próxima/tocar-URI pra thread motor - `kind` espelha `ets2::CommandKind`. |
| `ets2_list_playlists`/`ets2_add_playlist`/`ets2_remove_playlist` | CRUD das playlists salvas (persistidas em `config.json`).  |
| `ets2_list_output_devices`/`ets2_set_output_device` | Lista/define o dispositivo de saída de áudio só do Spotify.          |

## Layout da memória compartilhada

Não existe mais — essa seção descrevia o contrato binário entre o antigo
bridge (processo separado) e este addon via `CreateFileMappingW`. Como os
dois lados agora vivem na mesma DLL/processo, a "IPC" virou chamada de
função direta (ver tabela acima). Se você está vendo referências a
`OverlayShared`/`overlay_ipc.rs` em código antigo, é resquício da versão
pré-migração.
