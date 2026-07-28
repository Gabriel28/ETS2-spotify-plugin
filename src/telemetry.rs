//! Integracao com a telemetria do ETS2/ATS: detecta a parte eletrica do
//! caminhao ligada/desligada e o jogo pausado, pra dar play/pause no
//! Spotify automaticamente.
//!
//! Por que "eletrica" e nao "motor": num caminhao de verdade (e no jogo),
//! a chave tem uma posicao antes de dar partida no motor que so liga a
//! parte eletrica (radio, luzes do painel, etc.), sem ligar o motor. O
//! SDK da SCS expoe os dois estados separadamente: "Electric Enabled" e
//! "Engine Enabled" (ver Truck > Current Values na documentacao). Usamos
//! o eletrico de proposito, porque e o equivalente real de "ligar o som
//! do caminhao" - o motorista liga a eletrica pra ouvir musica antes
//! mesmo de dar partida, exatamente como aconteceria de verdade.
//!
//! Pre-requisito: instalar o plugin da RenCloud (scs-sdk-plugin) na pasta
//! `plugins` do jogo - ele escreve a telemetria em memoria compartilhada,
//! e esta crate (`scs-sdk-telemetry`) so le esses dados. Ver README.md,
//! secao "Como funciona" e "Instalacao".
//!
//! ATENCAO - leia antes de usar:
//! Os campos sao lidos fazendo um `format!("{data:?}")` do valor retornado
//! por `SharedMemory::read()` e procurando pelo texto `"<campo>: true"` /
//! `"<campo>: false"` dentro dele. Isso e proposital: como nao foi possivel
//! compilar este projeto contra a struct real da crate `scs-sdk-telemetry`
//! neste ambiente (sandbox Linux sem acesso as ferramentas do Rust/Windows),
//! usar o Debug-string evita depender do caminho exato dos campos
//! aninhados (`data.truck.electric_enabled` vs `data.common.electric_enabled`
//! etc.), que pode variar entre versoes da crate.
//!
//! A documentacao oficial do SDK da SCS (via o README do plugin da
//! RenCloud, que documenta a struct C# original 1:1 com o SDK oficial)
//! confirma que os campos existem mesmo: "Electric Enabled" e "Engine
//! Enabled" dentro de Truck > Current Values, e "Paused, game state" no
//! nivel raiz da telemetria. Isso aumenta bastante a confianca de que
//! `electric_enabled`/`paused` (convertido pra snake_case, convencao
//! comum em crates Rust) sao os nomes certos - mas como a crate Rust
//! usada aqui (`scs-sdk-telemetry`, de NightFeather0615) e uma
//! reimplementacao independente dessa struct, e nao um espelho
//! garantido, ainda vale confirmar com
//! `cargo doc -p scs-sdk-telemetry --open` na sua maquina antes de
//! trocar por acesso direto ao campo (mais rapido e sem chance de falso
//! positivo).

use anyhow::Result;
use scs_sdk_telemetry::shared_memory::SharedMemory;

pub struct GameTelemetry {
    mem: SharedMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameState {
    /// Parte eletrica do caminhao ligada (chave na primeira posicao) -
    /// nao exige o motor ligado.
    pub electrics_on: bool,
    pub paused: bool,
}

impl GameTelemetry {
    pub fn connect() -> Result<Self> {
        Ok(Self {
            mem: SharedMemory::connect()?,
        })
    }

    pub fn read_state(&mut self) -> Result<GameState> {
        let data = self.mem.read();
        let debug = format!("{data:?}");

        Ok(GameState {
            electrics_on: field_is_true(&debug, "electric_enabled")
                || field_is_true(&debug, "electricEnabled")
                || field_is_true(&debug, "electric")
                // Se por algum motivo essa versao da crate so expuser o
                // motor (e nao a eletrica separadamente), motor ligado
                // implica eletrica ligada, entao serve como fallback.
                || field_is_true(&debug, "engine_enabled")
                || field_is_true(&debug, "engineEnabled"),
            paused: field_is_true(&debug, "paused"),
        })
    }
}

/// Heuristica: procura `"<nome>: true"` (formato Debug padrao do Rust,
/// com ou sem aspas ao redor do nome do campo) dentro de uma string.
fn field_is_true(haystack: &str, field: &str) -> bool {
    haystack.contains(&format!("{field}: true")) || haystack.contains(&format!("\"{field}\": true"))
}
