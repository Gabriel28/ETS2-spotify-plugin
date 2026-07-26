//! Conversao entre a string salva na config (ex.: "Ctrl+Alt+Space") e o
//! tipo `HotKey` da crate `global-hotkey`, alem de formatar uma combinacao
//! capturada na UI (eventos de teclado do egui) de volta pra essa string.
//!
//! ATENCAO: assim como o resto do projeto, isso nao pode ser compilado
//! neste ambiente. A lista de teclas mapeadas cobre as mais comuns (letras,
//! numeros, setas, F1-F12, espaco/enter/tab/esc). Se faltar alguma tecla
//! que voce queira usar, adicione o caso em `code_from_name`.

use eframe::egui::{Key as EguiKey, Modifiers as EguiModifiers};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

pub fn parse_hotkey(spec: &str) -> Option<HotKey> {
    if spec.trim().is_empty() {
        return None;
    }

    let mut mods = Modifiers::empty();
    let mut code: Option<Code> = None;

    for part in spec.split('+').map(|p| p.trim()) {
        if part.is_empty() {
            continue;
        }
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "win" | "super" | "meta" | "cmd" => mods |= Modifiers::SUPER,
            other => code = code_from_name(other),
        }
    }

    code.map(|c| HotKey::new(Some(mods), c))
}

fn code_from_name(name: &str) -> Option<Code> {
    use Code::*;
    Some(match name {
        "space" => Space,
        "left" | "arrowleft" => ArrowLeft,
        "right" | "arrowright" => ArrowRight,
        "up" | "arrowup" => ArrowUp,
        "down" | "arrowdown" => ArrowDown,
        "enter" | "return" => Enter,
        "tab" => Tab,
        "escape" | "esc" => Escape,
        "backspace" => Backspace,
        "delete" | "del" => Delete,
        "f1" => F1,
        "f2" => F2,
        "f3" => F3,
        "f4" => F4,
        "f5" => F5,
        "f6" => F6,
        "f7" => F7,
        "f8" => F8,
        "f9" => F9,
        "f10" => F10,
        "f11" => F11,
        "f12" => F12,
        "0" => Digit0,
        "1" => Digit1,
        "2" => Digit2,
        "3" => Digit3,
        "4" => Digit4,
        "5" => Digit5,
        "6" => Digit6,
        "7" => Digit7,
        "8" => Digit8,
        "9" => Digit9,
        s if s.len() == 1 && s.chars().next().unwrap().is_ascii_alphabetic() => {
            key_letter(s.chars().next().unwrap())
        }
        _ => return None,
    })
}

fn key_letter(c: char) -> Code {
    use Code::*;
    match c.to_ascii_uppercase() {
        'A' => KeyA,
        'B' => KeyB,
        'C' => KeyC,
        'D' => KeyD,
        'E' => KeyE,
        'F' => KeyF,
        'G' => KeyG,
        'H' => KeyH,
        'I' => KeyI,
        'J' => KeyJ,
        'K' => KeyK,
        'L' => KeyL,
        'M' => KeyM,
        'N' => KeyN,
        'O' => KeyO,
        'P' => KeyP,
        'Q' => KeyQ,
        'R' => KeyR,
        'S' => KeyS,
        'T' => KeyT,
        'U' => KeyU,
        'V' => KeyV,
        'W' => KeyW,
        'X' => KeyX,
        'Y' => KeyY,
        'Z' => KeyZ,
        _ => KeyA,
    }
}

/// Formata a combinacao capturada na UI (modificadores do egui + a ultima
/// tecla "de verdade" pressionada) na mesma sintaxe usada por
/// `parse_hotkey`, ex.: "Ctrl+Alt+Space".
pub fn format_from_egui(modifiers: EguiModifiers, key: EguiKey) -> String {
    let mut parts = Vec::new();
    if modifiers.ctrl {
        parts.push("Ctrl".to_string());
    }
    if modifiers.alt {
        parts.push("Alt".to_string());
    }
    if modifiers.shift {
        parts.push("Shift".to_string());
    }
    parts.push(format!("{key:?}"));
    parts.join("+")
}
