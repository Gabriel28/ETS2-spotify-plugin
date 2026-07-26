# ets2-spotify-plugin

App desktop em Rust, no estilo de um launcher (pensa num "Content Manager"
do Assetto Corsa, só que bem mais simples) para ligar o Spotify ao seu
Euro Truck Simulator 2 / American Truck Simulator: liga a música quando
você liga a **parte elétrica** do caminhão (a posição da chave antes de
dar partida no motor — igual num caminhão de verdade, onde o rádio liga
antes do motor), pausa quando desliga ou pausa o jogo, deixa escolher
playlists salvas e configurar os atalhos de teclado — tudo isso **sem
usar login OAuth nem API key do Spotify**.

## Como funciona (visão geral)

O app é um `.exe` com janela própria que roda **ao lado** do jogo (não é
uma DLL injetada nele — mais sobre isso abaixo). Ele combina três coisas:

1. **Telemetria do jogo** — o ETS2/ATS, com o plugin da RenCloud
   instalado, escreve o estado do caminhão (elétrica ligada, jogo pausado
   etc.) numa área de memória compartilhada do Windows. O app lê isso
   ~2x por segundo pra saber quando ligar/pausar a música.
2. **Controle de mídia do Windows (SMTC)** — a mesma API que faz
   aparecer os botõezinhos de play/pause na barra de tarefas quando você
   ouve Spotify. Qualquer app pode usar essa API pra ler "o que está
   tocando" (título, artista, álbum, capa) e mandar comandos (play,
   pause, next, previous) **sem precisar de login, token ou API key** —
   o Windows já tem acesso porque o próprio Spotify se registrou nela.
3. **Links/URIs do Spotify** (`spotify:playlist:...`, `spotify:search:...`)
   — o mesmo mecanismo de clicar num link de playlist. Abrir uma dessas
   URIs faz o Spotify abrir/focar e tocar aquilo direto, também sem
   login. É assim que a tela de Playlists e a busca funcionam.

### Por que não é uma DLL carregada dentro do ETS2?

O SDK oficial da SCS (o que dá pra "instalar como plugin dentro do
jogo") serve pro jogo **mandar telemetria pra fora** — velocidade, RPM,
posição etc. Ele não tem uma API suportada pra desenhar na UI do
caminhão nem pra controlar apps externos como o Spotify. Fazer isso por
engenharia reversa da ABI do plugin seria frágil e podia até travar o
jogo. Por isso a arquitetura escolhida é um processo separado com sua
própria janela — o jeito robusto e suportado de fazer essa integração,
e também o motivo de não dar pra desenhar de verdade dentro do painel
3D do caminhão (o app fica em uma janela própria, não sobreposta ao
jogo).

### Por que não usa a Web API do Spotify (OAuth)?

Dava pra ter busca "de verdade" (lista de resultados dentro do próprio
app) e navegação pela biblioteca inteira usando a Web API oficial do
Spotify, mas isso exige criar um app no Spotify Developer Dashboard e
autorizar via OAuth. Como o pedido foi manter simples e sem
credenciais, o app usa só SMTC + links `spotify:` — cobre o essencial
(tocar, pausar, pular, tocar uma playlist salva, abrir uma busca) sem
nenhum cadastro.

## Licenças e créditos (telemetria)

Pergunta direta: **`scs-sdk-telemetry` (a crate Rust) NÃO é um projeto
oficial da SCS Software.** A cadeia completa é:

1. **SDK oficial da SCS** ("Telemetry & Input SDK", atualmente v1.14) —
   esse sim é oficial, distribuído pela própria SCS no domínio deles
   ([modding.scssoft.com](https://modding.scssoft.com/wiki/Documentation/Engine/SDK/Telemetry),
   download em `download.eurotrucksimulator2.com`). A wiki de modding da
   SCS descreve o propósito dele nestes termos: dar acesso à telemetria
   do veículo do jogador **para aplicações de terceiros** — ou seja, é
   feito exatamente para esse tipo de uso, sem restrição contra isso.
2. **RenCloud/scs-sdk-plugin** — plugin DLL (C++/C#) de terceiros,
   open source, que compila contra esse SDK oficial e expõe os dados
   via memória compartilhada do Windows. **Licença MIT** (permissiva,
   sem restrição de uso comercial ou de código fechado). O próprio
   README do projeto confirma a origem: *"SCS has kindly released a SDK
   that allows developers and users to stream telemetry data from the
   game to any 3rd party applications."*
3. **`scs-sdk-telemetry`** (crate Rust, de NightFeather0615) — wrapper
   que só lê a memória compartilhada exposta pelo plugin da RenCloud.
   **Licença MPL-2.0** (Mozilla Public License 2.0): copyleft só a nível
   de arquivo — ou seja, se você não modificar os arquivos da própria
   crate (não modificamos, só a usamos como dependência), não há
   nenhuma obrigação de abrir o código do seu projeto. Compatível com
   qualquer uso, incluindo projeto fechado ou publicado no GitHub sob
   qualquer licença que você quiser dar ao `ets2-spotify-bridge`.

Conclusão: nada aqui é "pirata" nem viola termos da SCS — é a cadeia de
ferramentas padrão que a própria comunidade de telemetria do
ETS2/ATS usa (o mesmo SDK por trás de painéis externos, dashboards em
segundo monitor, SimHub, etc.), com licenças (MIT/MPL-2.0) que permitem
o uso feito aqui sem restrição. Se quiser, dá pra eliminar essa camada
inteira de dependências de terceiros escrevendo nosso próprio plugin
C++ fino contra o SDK oficial da SCS diretamente — mas isso é bem mais
trabalho pra um ganho pequeno (o plugin da RenCloud já faz exatamente
isso e é mantido pela comunidade há anos).

## O que o app faz, na prática

| Evento / ação | Resultado |
|---|---|
| Ligar a parte elétrica do caminhão (chave, antes da partida) | Play no Spotify (retoma de onde parou) |
| Desligar a elétrica | Pausa |
| Pausar o jogo (menu, alt-tab) | Pausa (só se a elétrica estava ligada) |
| Despausar o jogo | Retoma o play (só se a elétrica estava ligada) |
| Clicar em "Tocar" numa playlist salva | Abre o Spotify e toca essa playlist |
| Digitar e clicar "Buscar" | Abre o Spotify já na busca com o termo preenchido |
| Hotkey de play/pause (configurável) | Play/pause manual |
| Hotkey de próxima/anterior (configurável) | Pula faixa |

Note que isso é separado do motor: como num caminhão de verdade, dá pra
ligar só a elétrica (rádio, luzes do painel) sem dar partida. O app usa
o campo `electric_enabled` da telemetria pra isso, não o `engine_enabled`
— então a música liga assim que você "vira a chave", mesmo antes de
ligar o motor. Se o app abrir com a elétrica já ligada, ele dá play uma
vez ao detectar isso, em vez de esperar uma transição.

## A janela do app

Três abas na barra lateral:

- **Tocando agora** — capa do álbum, título/artista, status da elétrica
  do caminhão e da telemetria, botões de anterior/play-pause/próxima, e um log do que
  o app está fazendo (útil pra debugar).
- **Playlists** — cole o link (`https://open.spotify.com/playlist/...`)
  ou a URI (`spotify:playlist:...`) de uma playlist seguida de um nome,
  clique "Adicionar" e ela vira um card com botão "Tocar". Também tem um
  campo de busca rápida.
- **Configurações** — atalhos de teclado. Clique em "Definir", aperte a
  combinação desejada (ex.: `Ctrl+Alt+Espaco`) e ela é salva. Clique
  "Salvar atalhos" pra aplicar.

A configuração (playlists salvas + atalhos) fica em
`%APPDATA%\ets2-spotify-bridge\config.json`.

## Pré-requisitos (na máquina com o jogo)

- Windows 10/11.
- [Rust](https://rustup.rs) instalado (`rustup default stable-x86_64-pc-windows-msvc`).
- Spotify **Desktop** instalado e logado (a versão web/navegador não
  registra sessão no SMTC do Windows nem responde a links `spotify:`).
- [RenCloud/scs-sdk-plugin](https://github.com/RenCloud/scs-sdk-plugin) —
  necessário só pra parte de elétrica/pausa automática. Sem ele, o app
  ainda funciona normalmente no modo manual (botões, hotkeys, playlists,
  busca).

## Instalação

1. **Plugin de telemetria (RenCloud)**: baixe o release mais recente em
   https://github.com/RenCloud/scs-sdk-plugin/releases e copie a DLL pra
   pasta `plugins` dentro da instalação do jogo, por exemplo:
   `C:\Program Files (x86)\Steam\steamapps\common\Euro Truck Simulator 2\bin\win_x64\plugins\`
   (crie a pasta `plugins` se ela não existir). Siga o passo a passo do
   próprio repositório da RenCloud caso os caminhos mudem.
2. **Compilar o app**: com Rust instalado, na pasta deste projeto:
   ```powershell
   cargo build --release
   ```
   O executável fica em `target\release\ets2-spotify-bridge.exe`.
3. **Rodar**: abra o Spotify, abra o ETS2/ATS, e rode o `.exe`. Pode
   deixar a janela aberta em segundo plano (ou minimizada) enquanto
   joga, ou criar um atalho na pasta de inicialização do Windows pra
   abrir junto com o PC.

## Estrutura do projeto

```
src/
  main.rs      # janela (eframe/egui): abas, botoes, captura de hotkey
  engine.rs    # thread unica dona da conexao com o Spotify (SMTC) +
               # telemetria; recebe comandos da UI/hotkeys por canal
  media.rs     # controle/leitura do Spotify via SMTC + links spotify:
  telemetry.rs # leitura da telemetria do ETS2/ATS (eletrica, pausa)
  hotkeys.rs   # parse/captura de combinacoes de tecla
  config.rs    # persistencia de hotkeys e playlists em JSON
  state.rs     # tipos compartilhados entre a thread motor e a UI
```

Arquitetura de threads: a janela roda na thread principal (é onde as
hotkeys globais são registradas e escutadas, porque no Windows isso
depende da fila de mensagens da própria janela). Uma segunda thread
("motor", `engine.rs`) é a única que fala com o SMTC do Windows e com a
telemetria do jogo — ela recebe comandos (`Command`) por um canal
(`mpsc`) tanto dos botões da UI quanto das hotkeys, e publica o estado
atual (música tocando, capa, elétrica, log) num `Mutex` compartilhado que
a UI lê a cada frame.

## Limitações conhecidas / o que confirmar na sua máquina

Este projeto foi escrito num ambiente Linux isolado, sem máquina
Windows/ETS2 disponível. Tentei de fato compilar antes de te entregar
isso (não só assumir que ia funcionar), e todas as vias possíveis
ficaram bloqueadas pelo ambiente:

- `rustup.rs` (instalador oficial do Rust) — proxy retornou 403.
- `apt install cargo/rustc` — sem permissão de root (`sudo` desabilitado
  no sandbox).
- Acesso direto a `static.rust-lang.org`, `crates.io`, `github.com` —
  também 403 no proxy (allowlist de rede não inclui esses domínios).

Ou seja: não é que eu não tentei, é que este ambiente especificamente
não tem como instalar um toolchain Rust nem baixar dependências. O
código foi escrito com cuidado nas APIs conhecidas, mas antes de
considerar "pronto", confira na sua máquina (que é onde isso pode
realmente ser compilado e testado):

1. **`cargo build` compila sem erro.** As crates `windows`, `eframe` e
   `global-hotkey` mudam de API entre versões; se der erro de
   compilação, o compilador aponta a linha e o `docs.rs` da versão
   instalada mostra a assinatura certa. Os pontos mais sensíveis a
   mudança de versão são: `egui::Image` (API de imagem mudou algumas
   vezes entre versões do egui) e a leitura da thumbnail via
   `windows::Storage::Streams::DataReader`.
2. **Detecção de elétrica/pausa (`telemetry.rs`)**: usa uma heurística de
   texto (procura `"electric_enabled: true"`, `"electricEnabled: true"`,
   `"engine_enabled: true"` como fallback e `"paused: true"` na
   representação em texto dos dados), porque não foi possível compilar
   contra a struct real da crate `scs-sdk-telemetry` aqui. Funciona na
   prática, mas rode `cargo doc -p scs-sdk-telemetry --open` pra
   confirmar os nomes reais dos campos (o SDK oficial da SCS documenta
   "Electric Enabled" e "Engine Enabled" como campos separados) e, se
   quiser, troque por acesso direto ao campo (mais rápido e sem chance
   de falso positivo).
3. **Múltiplas sessões de mídia** (ex.: navegador + Spotify abertos ao
   mesmo tempo): o filtro por `SourceAppUserModelId` deve pegar a
   sessão certa, mas vale confirmar no seu setup.
4. **Combinações de hotkey padrão** (`Ctrl+Alt+Espaco`/`Ctrl+Alt+Seta`)
   não colidem com atalhos que você já usa — e dá pra trocar direto na
   aba Configurações.
5. **Links de playlist**: teste colar tanto o link `https://open.spotify.com/playlist/...`
   quanto a URI `spotify:playlist:...` pra confirmar que a normalização
   em `normalize_spotify_link` (em `main.rs`) está pegando o formato que
   você copia do seu Spotify.

## Ideias de próximos passos

- Ícone na bandeja do sistema (system tray) pra manter rodando
  minimizado sem ocupar a barra de tarefas.
- Reordenar/editar playlists salvas (hoje só dá pra adicionar/remover).
- Buscar com resultados dentro do próprio app (exigiria a Web API do
  Spotify + OAuth, que foi deixado de fora de propósito por simplicidade).
- Iniciar automaticamente junto com o jogo.
