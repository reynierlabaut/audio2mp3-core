//! WAV (PCM 16 bits) → MP3, para pruebas manuales.
//!
//! cargo run --release --example encode -- entrada.wav salida.mp3 [kbps]

use std::time::Instant;

use audio2mp3_core::{Bitrate, EncoderConfig, Mp3Encoder};

struct Wav {
    sample_rate: u32,
    channels: u16,
    samples: Vec<i16>,
}

fn read_wav(data: &[u8]) -> Result<Wav, String> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("no es un archivo WAV".into());
    }
    let u16_at = |p: usize| u16::from_le_bytes([data[p], data[p + 1]]);
    let u32_at = |p: usize| u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
    let (mut pos, mut format) = (12, None);
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = u32_at(pos + 4) as usize;
        let body = pos + 8;
        let end = (body + len).min(data.len());
        if id == b"fmt " && len >= 16 {
            let (tag, channels, rate, bits) = (
                u16_at(body),
                u16_at(body + 2),
                u32_at(body + 4),
                u16_at(body + 14),
            );
            // 1 = PCM, 0xFFFE = WAVE_FORMAT_EXTENSIBLE (se asume PCM).
            if !(tag == 1 || tag == 0xFFFE) || bits != 16 {
                return Err(format!(
                    "solo PCM de 16 bits (formato {tag:#x}, {bits} bits)"
                ));
            }
            format = Some((rate, channels));
        } else if id == b"data" {
            let (sample_rate, channels) = format.ok_or("chunk data antes que fmt")?;
            let samples = data[body..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&b| i16::from_le_bytes(b))
                .collect();
            return Ok(Wav {
                sample_rate,
                channels,
                samples,
            });
        }
        pos = body + len + (len & 1);
    }
    Err("no se encontró el chunk data".into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output, rest @ ..] = args.as_slice() else {
        return Err("uso: encode <entrada.wav> <salida.mp3> [kbps]".into());
    };
    let kbps = rest.first().map_or(Ok(192), |k| k.parse())?;
    let wav = read_wav(&std::fs::read(input)?)?;
    let config = EncoderConfig {
        sample_rate: wav.sample_rate,
        channels: u8::try_from(wav.channels)?,
        bitrate: Bitrate::from_kbps(kbps)?,
    };
    let mut encoder = Mp3Encoder::new(config)?;
    let mut mp3 = Vec::new();
    let start = Instant::now();
    encoder.encode(&wav.samples, &mut mp3)?;
    encoder.flush(&mut mp3)?;
    let elapsed = start.elapsed().as_secs_f64();
    std::fs::write(output, &mp3)?;
    let duration = wav.samples.len() as f64 / f64::from(wav.channels) / f64::from(wav.sample_rate);
    println!(
        "{duration:.1} s de audio → {} KB en {elapsed:.2} s ({:.0}× tiempo real)",
        mp3.len() / 1024,
        duration / elapsed
    );
    Ok(())
}
