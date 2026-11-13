//! Sink de audio proprio pro backend `connect` (ver spotify_connect.rs),
//! no lugar do `librespot_playback::audio_backend::rodio` padrao.
//!
//! Por que existe (issue #1): o sink rodio do librespot 0.8 abre o stream
//! WASAPI (via cpal) UMA vez so, no dispositivo padrao daquele momento,
//! com o callback de erro padrao do cpal (que so loga), e o `write()` dele
//! espera o buffer esvaziar com um `while sink.len() > 26 { sleep }` SEM
//! saida. Se o stream morre depois de aberto - dispositivo invalidado
//! porque o jogo reinicializou o audio (FMOD) ao carregar o save, o
//! dispositivo USB foi reenumerado, o dispositivo padrao do Windows mudou
//! etc. - ninguem mais consome o buffer, e a thread do `Player` fica presa
//! nesse loop pra sempre: a faixa aparece no HUD, mas a posicao nao anda,
//! nao sai som nenhum e play/pause nunca mais sao processados (o `Player`
//! processa comandos na mesma thread que esta presa).
//!
//! Este sink faz a mesma coisa que o original (stereo 44.1kHz em f32 via
//! rodio), mas:
//! - registra erros do stream (`with_error_callback`) numa flag;
//! - reabre o stream quando ha erro, quando o dispositivo padrao do
//!   Windows muda, ou quando o buffer para de esvaziar (stall) por
//!   `STALL_TIMEOUT`, mesmo sem erro reportado;
//! - nunca bloqueia o `Player` indefinidamente (nem no `write` nem no
//!   `stop`, que no original usa `sleep_until_end()` - mesmo problema).

use librespot_playback::audio_backend::{Sink, SinkError, SinkResult};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Mesmo limite do sink original do librespot: ~0.5s de audio enfileirado
/// (cada pacote tem ~256-3000 amostras).
const MAX_QUEUED_PACKETS: usize = 26;

/// Se o buffer nao diminuir nada por esse tempo enquanto esperamos, o
/// stream e considerado morto mesmo sem o cpal ter reportado erro.
const STALL_TIMEOUT: Duration = Duration::from_secs(2);

/// De quanto em quanto tempo conferimos se o dispositivo padrao do Windows
/// mudou (consultar o cpal a cada pacote seria caro demais).
const DEVICE_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Tempo maximo que o `stop()` espera o restante do buffer tocar.
const STOP_DRAIN_TIMEOUT: Duration = Duration::from_millis(1500);

struct Output {
    sink: rodio::Sink,
    _stream: rodio::OutputStream,
    failed: Arc<AtomicBool>,
    device_name: Option<String>,
}

pub struct ResilientSink {
    output: Option<Output>,
    started: bool,
    last_device_check: Instant,
}

impl ResilientSink {
    pub fn new() -> Self {
        Self {
            output: None,
            started: false,
            last_device_check: Instant::now(),
        }
    }

    /// Garante um stream vivo no dispositivo padrao atual, reabrindo se
    /// preciso. Erro so se nao for possivel abrir nenhum stream.
    fn ensure_output(&mut self) -> SinkResult<&mut Output> {
        let needs_reopen = match &self.output {
            None => true,
            Some(out) if out.failed.load(Ordering::SeqCst) => true,
            Some(out) => {
                if self.last_device_check.elapsed() >= DEVICE_CHECK_INTERVAL {
                    self.last_device_check = Instant::now();
                    default_device_name() != out.device_name
                } else {
                    false
                }
            }
        };

        if needs_reopen {
            // Derruba o stream antigo ANTES de abrir o novo (libera o
            // dispositivo, caso seja o mesmo).
            self.output = None;
            let out = open_output()?;
            if self.started {
                out.sink.play();
            } else {
                out.sink.pause();
            }
            self.last_device_check = Instant::now();
            self.output = Some(out);
        }

        Ok(self.output.as_mut().expect("output acabou de ser garantido"))
    }

    fn invalidate(&mut self) {
        self.output = None;
    }
}

impl Default for ResilientSink {
    fn default() -> Self {
        Self::new()
    }
}

impl Sink for ResilientSink {
    fn start(&mut self) -> SinkResult<()> {
        self.started = true;
        let out = self.ensure_output()?;
        out.sink.play();
        Ok(())
    }

    fn stop(&mut self) -> SinkResult<()> {
        self.started = false;
        if let Some(out) = self.output.as_ref() {
            // Deixa tocar o que ja esta no buffer (igual ao original), mas
            // com limite de tempo e desistindo se o stream morrer.
            let deadline = Instant::now() + STOP_DRAIN_TIMEOUT;
            while !out.sink.empty() && !out.failed.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            out.sink.pause();
        }
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let samples = packet
            .samples()
            .map_err(|e| SinkError::OnWrite(e.to_string()))?;
        let samples_f32: Vec<f32> = converter.f64_to_f32(samples);

        let out = self.ensure_output()?;
        out.sink.append(rodio::buffer::SamplesBuffer::new(
            NUM_CHANNELS as rodio::ChannelCount,
            SAMPLE_RATE,
            samples_f32,
        ));

        // Espera o buffer esvaziar um pouco - mas, diferente do original,
        // detecta stream morto (erro ou buffer parado) e reabre em vez de
        // ficar preso aqui pra sempre.
        let mut last_len = out.sink.len();
        let mut last_progress = Instant::now();
        loop {
            let Some(out) = self.output.as_ref() else { break };
            let len = out.sink.len();
            if len <= MAX_QUEUED_PACKETS {
                break;
            }
            if out.failed.load(Ordering::SeqCst) || last_progress.elapsed() >= STALL_TIMEOUT {
                // O audio enfileirado no stream morto se perde (~0.5s) -
                // aceitavel, o importante e destravar o Player.
                self.invalidate();
                self.ensure_output()?;
                break;
            }
            if len < last_len {
                last_len = len;
                last_progress = Instant::now();
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

fn default_device_name() -> Option<String> {
    rodio::cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}

/// Abre um stream no dispositivo padrao atual: primeiro stereo 44.1kHz
/// f32 (formato nativo do librespot), depois a taxa padrao do dispositivo
/// (o rodio reamostra), e por ultimo qualquer config suportada.
fn open_output() -> SinkResult<Output> {
    let host = rodio::cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| SinkError::ConnectionRefused("nenhum dispositivo de saida de audio".into()))?;
    let device_name = device.name().ok();

    let failed = Arc::new(AtomicBool::new(false));
    let failed_cb = failed.clone();
    let on_error = move |_e: rodio::cpal::StreamError| {
        failed_cb.store(true, Ordering::SeqCst);
    };

    let preferred = device.default_output_config().ok().map(|default_config| {
        device
            .supported_output_configs()
            .ok()
            .and_then(|mut cfgs| {
                cfgs.find(|c| c.channels() == NUM_CHANNELS as rodio::ChannelCount)
                    .and_then(|c| {
                        c.try_with_sample_rate(rodio::cpal::SampleRate(SAMPLE_RATE))
                            .or_else(|| c.try_with_sample_rate(default_config.sample_rate()))
                    })
            })
            .unwrap_or(default_config)
    });

    let exact = preferred.and_then(|config| {
        rodio::OutputStreamBuilder::default()
            .with_device(device.clone())
            .with_config(&config.config())
            .with_sample_format(rodio::cpal::SampleFormat::F32)
            .with_error_callback(on_error.clone())
            .open_stream()
            .ok()
    });

    let mut stream = match exact {
        Some(s) => s,
        None => rodio::OutputStreamBuilder::from_device(device)
            .map_err(|e| SinkError::ConnectionRefused(e.to_string()))?
            .with_error_callback(on_error)
            .open_stream_or_fallback()
            .map_err(|e| SinkError::ConnectionRefused(e.to_string()))?,
    };
    stream.log_on_drop(false);

    let sink = rodio::Sink::connect_new(stream.mixer());
    Ok(Output {
        sink,
        _stream: stream,
        failed,
        device_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silence_packet() -> AudioPacket {
        // ~23ms de stereo 44.1kHz (tamanho tipico de pacote do librespot).
        AudioPacket::Samples(vec![0.0; 2048])
    }

    /// Toca ~3s de silencio no dispositivo padrao real: o `write` tem que
    /// acompanhar o ritmo do audio (nem travar, nem passar direto).
    #[test]
    fn plays_through_default_device_without_hanging() {
        let mut sink = ResilientSink::new();
        let mut conv = Converter::new(None);
        sink.start().expect("abrir stream no dispositivo padrao");
        let t0 = Instant::now();
        let packets = (3 * SAMPLE_RATE as usize * NUM_CHANNELS as usize) / 2048;
        for _ in 0..packets {
            sink.write(silence_packet(), &mut conv).expect("write");
        }
        let elapsed = t0.elapsed();
        sink.stop().expect("stop");
        assert!(elapsed >= Duration::from_millis(2000), "rapido demais: {elapsed:?}");
        assert!(elapsed <= Duration::from_secs(6), "lento demais: {elapsed:?}");
    }

    /// Simula o bug da issue #1: o stream "morre" (flag de erro do cpal) com
    /// o buffer cheio. O sink original ficava preso pra sempre; este tem que
    /// reabrir o stream e seguir tocando.
    #[test]
    fn recovers_when_stream_dies() {
        let mut sink = ResilientSink::new();
        let mut conv = Converter::new(None);
        sink.start().expect("start");
        for _ in 0..40 {
            sink.write(silence_packet(), &mut conv).expect("write");
        }
        let old_flag = sink.output.as_ref().unwrap().failed.clone();
        // Para de consumir o buffer (como um stream morto) e sinaliza erro.
        sink.output.as_ref().unwrap().sink.pause();
        old_flag.store(true, Ordering::SeqCst);

        let t0 = Instant::now();
        for _ in 0..100 {
            sink.write(silence_packet(), &mut conv).expect("write apos falha");
        }
        assert!(t0.elapsed() <= Duration::from_secs(5), "travou: {:?}", t0.elapsed());
        let out = sink.output.as_ref().expect("stream reaberto");
        assert!(!Arc::ptr_eq(&out.failed, &old_flag), "stream nao foi reaberto");
        sink.stop().unwrap();
    }

    /// Stream que para de consumir SEM reportar erro (stall silencioso).
    #[test]
    fn recovers_from_silent_stall() {
        let mut sink = ResilientSink::new();
        let mut conv = Converter::new(None);
        sink.start().expect("start");
        let old_flag = sink.output.as_ref().unwrap().failed.clone();
        sink.output.as_ref().unwrap().sink.pause();

        let t0 = Instant::now();
        for _ in 0..60 {
            sink.write(silence_packet(), &mut conv).expect("write");
        }
        assert!(t0.elapsed() <= STALL_TIMEOUT * 3, "travou: {:?}", t0.elapsed());
        assert!(!Arc::ptr_eq(&sink.output.as_ref().unwrap().failed, &old_flag));
        sink.stop().unwrap();
    }
}
