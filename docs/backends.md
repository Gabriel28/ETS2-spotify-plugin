# Backends

Detalhes dos dois backends do Spotify (`config.json` → `"backend"`) — visão
geral em [README.md](../README.md#como-funciona).

## `connect` (padrão)

O addon vira um dispositivo **Spotify Connect** de verdade (aparece na
lista de dispositivos do app oficial em qualquer aparelho — celular, outro
PC), usando o [librespot](https://github.com/librespot-org/librespot), uma
reimplementação **não-oficial** (engenharia reversa) e open source do
protocolo Spotify Connect. Não precisa do Spotify Desktop aberto no PC do
jogo — o próprio addon decodifica e toca o áudio (ver `src/spotify_connect.rs`).

- **Login**: na primeira vez, abre o navegador padrão do Windows pra você
  autorizar via OAuth (o HUD mostra "Login necessário..." enquanto isso).
  Depois do primeiro login, a sessão fica em cache em
  `%APPDATA%\ets2-spotify-bridge\librespot-cache\` e o navegador não abre
  de novo, mesmo fechando/abrindo o jogo. Pra forçar um novo login (trocar
  de conta, por exemplo), apague essa pasta.
- **Auto-ativação**: assim que conecta, o addon já se torna o dispositivo
  Connect ativo sozinho (`Spirc::activate`) — não precisa abrir o Spotify
  em outro aparelho e escolher "ETS2" na lista manualmente. Se a
  auto-ativação falhar (ex.: nenhuma sessão Spotify ativa em lugar nenhum
  ainda), o status no HUD avisa e "ETS2" continua aparecendo normal na
  lista de dispositivos pra escolher na mão.
- **Não precisa registrar seu próprio app no Spotify for Developers**: o
  login usa o client_id público que o próprio librespot já embute —
  nenhuma credencial nossa entra em jogo.
- **Exige conta Premium** — o protocolo Connect não inicia sessão de
  streaming em conta free.
- **Sua senha nunca passa pelo addon** — o login é OAuth de verdade (tela
  oficial da Spotify no navegador); o que fica salvo em cache é um token de
  sessão, revogável a qualquer momento em
  [spotify.com/account/apps](https://www.spotify.com/account/apps).
- Como é uma reimplementação não-oficial do protocolo, existe um risco
  (baixo, mas real) de quebrar numa mudança futura do lado da Spotify — se
  isso acontecer, troque pra `"backend": "smtc"` no `config.json` como
  alternativa (aí sim exige o Spotify Desktop aberto).
- Capa do álbum, volume e progresso da faixa funcionam nos dois backends.

## `smtc`

Controla um Spotify Desktop já aberto via System Media Transport Controls
do Windows (`src/media.rs`) — sem login/OAuth, mas precisa do app rodando
no mesmo PC (a versão web não responde ao SMTC nem a links `spotify:`).
Era o único modo antes da versão com backend `connect`.

Volume é controlado via a sessão de áudio do processo do Spotify
(`ISimpleAudioVolume`, `src/audio_device.rs`), independente do volume
mestre do Windows — mesmo mecanismo usado pra escolher o dispositivo de
saída de áudio só do Spotify (`preferred_output_device` no `config.json`).

## Por que não tem busca

Nenhum dos dois backends expõe "buscar no catálogo" — o SMTC só controla a
sessão do Spotify Desktop que já está ativa; o protocolo Spotify Connect
também não, ele só recebe comandos de reprodução. Busca de verdade com
resultados dentro do painel exigiria a API Web oficial do Spotify (REST,
diferente do protocolo Connect usado aqui), o que este projeto evita de
propósito.

Pra tocar algo específico escolhido na hora: no backend `smtc`, abra o
Spotify normalmente por fora do jogo e escolha lá; no backend `connect`,
abra o Spotify em qualquer outro aparelho e escolha "ETS2" na lista de
dispositivos, ou toque algo lá e depois use os botões/hotkeys daqui pra
controlar. Playlists salvas (`config.json`) cobrem o caso de reabrir algo
sem sair do jogo, nos dois backends.
