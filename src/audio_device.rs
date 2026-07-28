//! Troca do dispositivo de saida de audio usado pelo Spotify especificamente
//! (nao o padrao do Windows inteiro) - a mesma funcionalidade da tela
//! "Configuracoes > Sistema > Som > Volume do app e preferencias de
//! dispositivo" do Windows 10/11.
//!
//! A Microsoft nao documenta/expoe essa funcionalidade via API publica.
//! A propria tela de Configuracoes usa por baixo dos panos uma interface
//! COM/WinRT interna chamada `IAudioPolicyConfigFactory`, ativada via
//! `RoGetActivationFactory("Windows.Media.Internal.AudioPolicyConfig", ...)`.
//! Isso foi reverse-engineered por ferramentas conhecidas como o EarTrumpet
//! (https://github.com/File-New-Project/EarTrumpet) e o SoundSwitch - o
//! layout de metodos e o GUID usados aqui vem do codigo-fonte do EarTrumpet
//! (EarTrumpet/Interop/MMDeviceAPI/IAudioPolicyConfigFactoryVariantFor21H2.cs
//! e EarTrumpet/Interop/Helpers/AudioPolicyConfigFactoryImplFor21H2.cs).
//!
#![allow(non_snake_case)] // nomes dos metodos da interface COM/WinRT undocumented

//! ATENCAO: por ser uma API interna nao documentada, so cobrimos aqui a
//! variante usada desde o Windows 10 21H2 (a maquina de desenvolvimento e
//! qualquer Windows 11 ja caem nesse caso). Se um dia a Microsoft mudar essa
//! interface interna num update do Windows, isso para de funcionar - por
//! isso todo erro aqui e no-fatal (loga e segue usando o dispositivo padrao).

use anyhow::{anyhow, Context, Result};
use windows::core::{interface, Interface, HRESULT, HSTRING};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
use windows::Win32::Media::Audio::{
    eConsole, eMultimedia, eRender, IAudioSessionControl2, IAudioSessionManager2,
    IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CLSCTX_ALL, CLSCTX_INPROC_SERVER, STGM_READ,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::WinRT::RoGetActivationFactory;

/// Interface COM/WinRT interna usada pela tela de som do Windows 10 21H2+
/// pra ler/gravar qual dispositivo de saida cada processo deve usar por
/// padrao. Os 19 metodos "incomplete_*" no comeco existem so pra ocupar os
/// slots certos da vtable (nao sabemos - e nao precisamos saber - a
/// assinatura real deles, so a posicao); os 3 de baixo sao os que
/// realmente usamos, confirmados contra o binding em C# do EarTrumpet.
#[interface("ab3d4648-e242-459f-b02f-541c70306324")]
unsafe trait IAudioPolicyConfigFactory: windows::core::IUnknown {
    // Slots 3..5 do objeto real: IInspectable::GetIids/GetRuntimeClassName/
    // GetTrustLevel. Declaramos como parametro de IUnknown (em vez de
    // IInspectable) porque a macro `#[interface]` so trata IUnknown como
    // caso especial pronto pra usar sem exigir um trait `_Impl` auxiliar -
    // esses 3 metodos aqui so existem pra preencher a posicao certa.
    fn inspectable_get_iids(&self) -> HRESULT;
    fn inspectable_get_runtime_class_name(&self) -> HRESULT;
    fn inspectable_get_trust_level(&self) -> HRESULT;
    fn incomplete_01(&self) -> HRESULT;
    fn incomplete_02(&self) -> HRESULT;
    fn incomplete_03(&self) -> HRESULT;
    fn incomplete_04(&self) -> HRESULT;
    fn incomplete_05(&self) -> HRESULT;
    fn incomplete_06(&self) -> HRESULT;
    fn incomplete_07(&self) -> HRESULT;
    fn incomplete_08(&self) -> HRESULT;
    fn incomplete_09(&self) -> HRESULT;
    fn incomplete_10(&self) -> HRESULT;
    fn incomplete_11(&self) -> HRESULT;
    fn incomplete_12(&self) -> HRESULT;
    fn incomplete_13(&self) -> HRESULT;
    fn incomplete_14(&self) -> HRESULT;
    fn incomplete_15(&self) -> HRESULT;
    fn incomplete_16(&self) -> HRESULT;
    fn incomplete_17(&self) -> HRESULT;
    fn incomplete_18(&self) -> HRESULT;
    fn incomplete_19(&self) -> HRESULT;
    // IMPORTANTE: `device_id` tem que ser `HSTRING` por valor, nao `&HSTRING`.
    // A macro `#[interface]` nao faz nenhuma conversao automatica pra tipos
    // comuns (so trata especialmente os marcadores `Ref<T>`/`OutRef<T>`), ou
    // seja, o tipo declarado aqui vira literalmente o tipo do parametro na
    // vtable COM crua. `&HSTRING` vira um ponteiro-para-ponteiro (endereco
    // da variavel local), nao o handle da string em si - isso foi
    // descoberto na pratica (retornava 0x80070057 "parametro incorreto" em
    // toda chamada, inclusive com HSTRING vazia).
    fn SetPersistedDefaultAudioEndpoint(
        &self,
        process_id: u32,
        flow: i32,
        role: i32,
        device_id: HSTRING,
    ) -> HRESULT;
    fn GetPersistedDefaultAudioEndpoint(
        &self,
        process_id: u32,
        flow: i32,
        role: i32,
        device_id: *mut HSTRING,
    ) -> HRESULT;
    fn ClearAllPersistedApplicationDefaultEndpoints(&self) -> HRESULT;
}

const DEVINTERFACE_AUDIO_RENDER: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";
const MMDEVAPI_TOKEN: &str = r"\\?\SWD#MMDEVAPI#";

#[derive(Debug, Clone)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

fn factory() -> Result<IAudioPolicyConfigFactory> {
    unsafe {
        RoGetActivationFactory(&HSTRING::from("Windows.Media.Internal.AudioPolicyConfig"))
            .context("RoGetActivationFactory(AudioPolicyConfig) falhou - Windows muito antigo?")
    }
}

fn device_enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .context("nao foi possivel criar o MMDeviceEnumerator")
    }
}

/// Lista os dispositivos de saida de audio ativos (nome amigavel + id),
/// pra popular o seletor na aba de Configuracoes.
pub fn list_output_devices() -> Result<Vec<OutputDevice>> {
    unsafe {
        let enumerator = device_enumerator()?;
        let collection: IMMDeviceCollection =
            enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let device: IMMDevice = collection.Item(i)?;
            let id = device.GetId()?.to_string()?;
            let name = friendly_name(&device).unwrap_or_else(|_| id.clone());
            out.push(OutputDevice { id, name });
        }
        Ok(out)
    }
}

unsafe fn friendly_name(device: &IMMDevice) -> Result<String> {
    let store = device.OpenPropertyStore(STGM_READ)?;
    let value = store.GetValue(&PKEY_Device_FriendlyName)?;
    let s = value.to_string();
    Ok(s)
}

/// Percorre as sessoes de audio de saida ativas em todos os dispositivos
/// (nao so o padrao, caso o usuario ja tenha redirecionado o Spotify
/// antes) e devolve a sessao (`IAudioSessionControl2`) do processo com
/// nome de imagem "spotify.exe", se houver uma ativa agora. Base de
/// `find_spotify_process_id` (PID) e das funcoes de volume abaixo
/// (`ISimpleAudioVolume`, obtida via `.cast()` a partir do mesmo
/// `IAudioSessionControl2`).
fn find_spotify_session() -> Result<Option<IAudioSessionControl2>> {
    unsafe {
        let enumerator = device_enumerator()?;
        let collection = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;
        for i in 0..count {
            let device: IMMDevice = collection.Item(i)?;
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_INPROC_SERVER, None)
            else {
                continue;
            };
            let Ok(session_enum) = manager.GetSessionEnumerator() else {
                continue;
            };
            let Ok(session_count) = session_enum.GetCount() else {
                continue;
            };
            for j in 0..session_count {
                let Ok(control) = session_enum.GetSession(j) else {
                    continue;
                };
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let Ok(pid) = control2.GetProcessId() else {
                    continue;
                };
                if pid == 0 {
                    continue;
                }
                if process_is_spotify(pid) {
                    return Ok(Some(control2));
                }
            }
        }
        Ok(None)
    }
}

/// Acha o PID do processo que hoje esta com uma sessao de audio de saida
/// aberta com nome de imagem "spotify.exe".
pub fn find_spotify_process_id() -> Result<Option<u32>> {
    let Some(session) = find_spotify_session()? else {
        return Ok(None);
    };
    unsafe { Ok(Some(session.GetProcessId()?)) }
}

/// Le o volume da sessao de audio do Spotify (0.0-1.0), independente do
/// volume mestre do Windows - mesma coisa que aparece no "Mixer de volume"
/// pro app Spotify. `Ok(None)` se o Spotify nao estiver com sessao de
/// audio ativa agora (equivalente a "volume desconhecido", nao um erro).
pub fn get_spotify_volume() -> Result<Option<f32>> {
    let Some(session) = find_spotify_session()? else {
        return Ok(None);
    };
    unsafe {
        let simple: ISimpleAudioVolume = session.cast()?;
        Ok(Some(simple.GetMasterVolume()?))
    }
}

/// Define o volume da sessao de audio do Spotify (0.0-1.0). Precisa que o
/// Spotify ja esteja com uma sessao de audio ativa (mesma limitacao de
/// `set_spotify_output_device`).
pub fn set_spotify_volume(volume: f32) -> Result<()> {
    let session = find_spotify_session()?
        .ok_or_else(|| anyhow!("Spotify nao esta com uma sessao de audio ativa agora"))?;
    unsafe {
        let simple: ISimpleAudioVolume = session.cast()?;
        simple
            .SetMasterVolume(volume.clamp(0.0, 1.0), std::ptr::null())
            .context("SetMasterVolume falhou")?;
    }
    Ok(())
}

fn process_is_spotify(pid: u32) -> bool {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut buf = [0u16; MAX_PATH as usize];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return false;
        }
        String::from_utf16_lossy(&buf[..len as usize])
            .to_lowercase()
            .ends_with("spotify.exe")
    }
}

fn wrap_device_id(raw_id: &str) -> HSTRING {
    HSTRING::from(format!("{MMDEVAPI_TOKEN}{raw_id}{DEVINTERFACE_AUDIO_RENDER}"))
}

/// Define (ou limpa, se `device_id` for `None`) o dispositivo de saida
/// padrao usado especificamente pelo processo do Spotify. Precisa que o
/// Spotify ja esteja aberto e tocando/com uma sessao de audio ativa pra
/// conseguirmos achar o PID dele.
pub fn set_spotify_output_device(device_id: Option<&str>) -> Result<()> {
    let pid = find_spotify_process_id()?
        .ok_or_else(|| anyhow!("Spotify nao esta com uma sessao de audio ativa agora"))?;
    let factory = factory()?;
    let hstring = match device_id {
        Some(id) => wrap_device_id(id),
        None => HSTRING::new(),
    };
    unsafe {
        factory
            .SetPersistedDefaultAudioEndpoint(pid, eRender.0, eConsole.0, hstring.clone())
            .ok()
            .context("SetPersistedDefaultAudioEndpoint (eConsole) falhou")?;
        factory
            .SetPersistedDefaultAudioEndpoint(pid, eRender.0, eMultimedia.0, hstring)
            .ok()
            .context("SetPersistedDefaultAudioEndpoint (eMultimedia) falhou")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Testes manuais de fumaca contra as APIs reais do Windows (nao rodam
    // em CI, so fazem sentido numa maquina Windows de verdade). Rode com
    // `cargo test -- --nocapture` pra ver a saida.
    #[test]
    fn smoke_list_output_devices() {
        crate::init_apartment();
        let devices = list_output_devices().expect("falha ao listar dispositivos de saida");
        for d in &devices {
            println!("dispositivo: {} ({})", d.name, d.id);
        }
        assert!(!devices.is_empty(), "esperava pelo menos um dispositivo de saida ativo");
    }

    #[test]
    fn smoke_set_and_clear_output_device() {
        crate::init_apartment();
        let devices = list_output_devices().expect("falha ao listar dispositivos");
        if find_spotify_process_id().unwrap_or(None).is_none() {
            println!("Spotify sem sessao de audio ativa agora - pulando teste (abra o Spotify e toque algo)");
            return;
        }
        let target = devices.first().expect("nenhum dispositivo de saida ativo");
        println!("[1] limpar (None) primeiro, pra testar o caminho mais simples");
        set_spotify_output_device(None).expect("limpar override falhou");
        println!("[2] definindo saida do Spotify para: {} (id={})", target.name, target.id);
        set_spotify_output_device(Some(&target.id)).expect("SetPersistedDefaultAudioEndpoint falhou");
        println!("revertendo para o dispositivo padrao do sistema");
        set_spotify_output_device(None).expect("limpar override falhou");
    }

    #[test]
    fn smoke_find_spotify_pid() {
        crate::init_apartment();
        match find_spotify_process_id() {
            Ok(Some(pid)) => println!("Spotify encontrado, PID {pid}"),
            Ok(None) => println!("Spotify nao esta com sessao de audio ativa agora (ok se nao estiver tocando nada)"),
            Err(e) => println!("erro ao procurar Spotify: {e}"),
        }
    }
}
