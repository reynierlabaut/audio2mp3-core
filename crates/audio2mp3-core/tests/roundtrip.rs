//! Ida y vuelta con un decoder independiente (symphonia, solo en dev-dependencies).

use audio2mp3_core::{Bitrate, EncoderConfig, Mp3Encoder};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as DecodeErr;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

const RATES: [u32; 3] = [32_000, 44_100, 48_000];
const BITRATES: [u16; 8] = [64, 96, 128, 160, 192, 224, 256, 320];

struct Decoded {
    /// Muestras intercaladas en [-1, 1].
    samples: Vec<f32>,
    channels: usize,
    sample_rate: u32,
    errors: usize,
}

fn decode(mp3: Vec<u8>) -> Decoded {
    decode_with(mp3, false)
}

fn decode_with(mp3: Vec<u8>, gapless: bool) -> Decoded {
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(mp3)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .expect("probe");
    let track = format.default_track(TrackType::Audio).expect("pista");
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .expect("audio")
        .clone();
    let track_id = track.id;
    // Sin Xing/Info, symphonia estima la duración por bitrate y en modo gapless
    // puede recortar el último frame; se desactiva para ver la salida completa.
    let options = AudioDecoderOptions::default().gapless(gapless);
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &options)
        .expect("decoder");
    let mut out = Decoded {
        samples: Vec::new(),
        channels: params.channels.as_ref().map_or(0, |c| c.count()),
        sample_rate: params.sample_rate.unwrap_or(0),
        errors: 0,
    };
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => panic!("error del demuxer: {e}"),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let start = out.samples.len();
                out.samples.resize(start + buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut out.samples[start..]);
            }
            Err(DecodeErr::DecodeError(_)) => out.errors += 1,
            Err(e) => panic!("error del decoder: {e}"),
        }
    }
    out
}

fn encode(samples: &[i16], sample_rate: u32, channels: u8, kbps: u16) -> (Vec<u8>, u32) {
    let config = EncoderConfig {
        sample_rate,
        channels,
        bitrate: Bitrate::from_kbps(kbps).unwrap(),
    };
    let mut enc = Mp3Encoder::new(config).unwrap();
    let mut mp3 = Vec::new();
    enc.encode(samples, &mut mp3).unwrap();
    enc.flush(&mut mp3).unwrap();
    (mp3, enc.delay_samples())
}

/// Codifica con frame Info y lo reescribe al final, como haría la app.
fn encode_with_info(samples: &[i16], sample_rate: u32, channels: u8, kbps: u16) -> (Vec<u8>, u32) {
    let config = EncoderConfig {
        sample_rate,
        channels,
        bitrate: Bitrate::from_kbps(kbps).unwrap(),
    };
    let mut enc = Mp3Encoder::new(config).unwrap();
    enc.enable_info_frame().unwrap();
    let mut mp3 = Vec::new();
    enc.encode(samples, &mut mp3).unwrap();
    enc.flush(&mut mp3).unwrap();
    let size = enc.info_frame_size();
    assert_eq!(enc.info_frame_into(&mut mp3[..size]), Ok(size));
    // Campo "frames": tras cabecera (4), side info, "Info" (4) y flags (4).
    let frames_at = 4 + if channels == 1 { 17 } else { 32 } + 8;
    let audio_frames = u32::from_be_bytes(mp3[frames_at..frames_at + 4].try_into().unwrap());
    (mp3, audio_frames)
}

/// Señal musical sintética: varios parciales y una envolvente lenta.
fn music(len: usize, channels: usize, sample_rate: u32) -> Vec<i16> {
    let fs = sample_rate as f32;
    (0..len * channels)
        .map(|i| {
            let t = (i / channels) as f32 / fs;
            let ch = (i % channels) as f32;
            let env = 0.6 + 0.4 * (t * 1.3).sin();
            let tau = 2.0 * std::f32::consts::PI;
            let v = 0.35 * (tau * (220.0 + 110.0 * ch) * t).sin()
                + 0.2 * (tau * 1_234.0 * t).sin()
                + 0.1 * (tau * 4_567.0 * t).sin()
                + 0.05 * (tau * 9_876.0 * t).sin();
            (env * v * 32767.0) as i16
        })
        .collect()
}

fn snr_db(decoded: &[f32], input: &[i16], channels: usize, delay: usize) -> f64 {
    let (mut signal, mut noise) = (0f64, 0f64);
    let skip = 2304 * channels; // arranque del filterbank
    for (i, &x) in input.iter().enumerate().skip(skip) {
        let Some(&y) = decoded.get(i + delay * channels) else {
            break;
        };
        let x = f64::from(x) / 32768.0;
        signal += x * x;
        noise += (f64::from(y) - x).powi(2);
    }
    10.0 * (signal / noise).log10()
}

/// Con el frame Info el decoder conoce el número exacto de frames: en modo
/// gapless ya no recorta el último (antes sí, al estimar por bitrate) y el
/// frame Info no se reproduce (el retardo sigue siendo el mismo).
#[test]
fn info_frame_gives_exact_duration_with_gapless_decoders() {
    for (rate, channels, kbps) in [(44_100, 1, 320), (48_000, 2, 128), (32_000, 2, 64)] {
        let len = rate as usize / 2;
        let input = music(len, usize::from(channels), rate);
        let (mp3, audio_frames) = encode_with_info(&input, rate, channels, kbps);
        let expected_frames = (len + 1057).div_ceil(1152) as u32;
        assert_eq!(audio_frames, expected_frames);
        let out = decode_with(mp3, true);
        let tag = format!("{rate} Hz {channels} ch {kbps} kbps");
        assert_eq!(out.errors, 0, "{tag}");
        assert_eq!(
            out.samples.len(),
            expected_frames as usize * 1152 * usize::from(channels),
            "{tag}"
        );
        let snr = snr_db(&out.samples, &input, usize::from(channels), 1057);
        assert!(
            snr > 20.0,
            "{tag}: SNR {snr:.1} dB (¿frame Info decodificado como audio?)"
        );
    }
}

#[test]
fn every_config_decodes_without_errors() {
    for rate in RATES {
        for channels in [1u8, 2] {
            for kbps in BITRATES {
                let len = rate as usize / 2;
                let input = music(len, usize::from(channels), rate);
                let (mp3, delay) = encode(&input, rate, channels, kbps);
                let frames = (len + delay as usize).div_ceil(1152);
                let out = decode(mp3);
                let tag = format!("{rate} Hz, {channels} ch, {kbps} kbps");
                assert_eq!(out.errors, 0, "{tag}");
                assert_eq!(out.channels, usize::from(channels), "{tag}");
                assert_eq!(out.sample_rate, rate, "{tag}");
                assert_eq!(
                    out.samples.len(),
                    frames * 1152 * usize::from(channels),
                    "{tag}"
                );
            }
        }
    }
}

#[test]
fn delay_matches_independent_decoder() {
    let input = music(44_100, 1, 44_100);
    let (mp3, delay) = encode(&input, 44_100, 1, 320);
    let out = decode(mp3);
    let best = (0..2304)
        .max_by(|&a, &b| {
            let corr = |d: usize| -> f64 {
                input
                    .iter()
                    .zip(&out.samples[d..])
                    .skip(4096)
                    .take(8192)
                    .map(|(&x, &y)| f64::from(x) * f64::from(y))
                    .sum()
            };
            corr(a).total_cmp(&corr(b))
        })
        .unwrap();
    assert_eq!(best, delay as usize);
}

/// Regresión de calidad (44.1 kHz estéreo, señal `music`).
/// Fase 1: 64 → 35.3 dB, 128 → 51.3, 192 → 57.8, 320 → 66.3.
/// Fase 2 (reservoir, M/S, ganancia común): 44.5 / 59.5 / 67.1 / 71.9.
/// Fase 3 (outer loop psicoacústico): 43.4 / 59.3 / 65.5 / 71.9. El outer
/// loop cambia algo de SNR por dejar el ruido bajo el umbral de
/// enmascaramiento (ver `encoder::tests::outer_loop_*`), así que el SNR ya
/// no es la medida de calidad principal. Umbrales ~2 dB por debajo.
#[test]
fn quality_does_not_regress() {
    for (kbps, min_snr) in [(64, 41.5), (128, 57.5), (192, 63.5), (320, 70.0)] {
        let input = music(44_100, 2, 44_100);
        let (mp3, delay) = encode(&input, 44_100, 2, kbps);
        let out = decode(mp3);
        let snr = snr_db(&out.samples, &input, 2, delay as usize);
        eprintln!("{kbps} kbps estéreo: SNR {snr:.1} dB");
        assert!(snr > min_snr, "{kbps} kbps: SNR {snr:.1} dB < {min_snr} dB");
    }
}

/// `main_data_begin` de cada frame (9 bits tras la cabecera).
fn main_data_begins(mp3: &[u8], frame_len: impl Fn(&[u8]) -> usize) -> Vec<u32> {
    let mut begins = Vec::new();
    let mut pos = 0;
    while pos + 6 <= mp3.len() {
        begins.push(u32::from(mp3[pos + 4]) << 1 | u32::from(mp3[pos + 5] >> 7));
        pos += frame_len(&mp3[pos..]);
    }
    begins
}

#[test]
fn reservoir_carries_bits_from_easy_to_hard_frames() {
    // 0.5 s de silencio, 0.5 s de música, 0.5 s de silencio, 0.5 s de música.
    let half = 22_050;
    let tune = music(half, 2, 44_100);
    let mut input = vec![0i16; 2 * half];
    input.extend_from_slice(&tune);
    input.extend(std::iter::repeat_n(0, 2 * half));
    input.extend_from_slice(&tune);
    let (mp3, delay) = encode(&input, 44_100, 2, 128);
    let begins = main_data_begins(&mp3, |f| {
        144 * 128_000 / 44_100 + usize::from(f[2] >> 1 & 1)
    });
    let used = begins.iter().filter(|&&b| b > 0).count();
    eprintln!("frames con main_data_begin > 0: {used} de {}", begins.len());
    assert!(used > begins.len() / 4);
    assert!(begins.iter().all(|&b| b <= 511));
    let out = decode(mp3);
    assert_eq!(out.errors, 0);
    let snr = snr_db(&out.samples, &input, 2, delay as usize);
    eprintln!("silencio/música 128 kbps: SNR {snr:.1} dB");
    assert!(snr > 55.0);
}

/// Golpes secos de ruido (tipo castañuela) cada 0.3 s sobre un tono bajo.
/// Devuelve la señal y la posición de cada golpe.
fn castanets(len: usize, channels: usize, sample_rate: u32) -> (Vec<i16>, Vec<usize>) {
    let period = (0.3 * f64::from(sample_rate)) as usize;
    let onsets: Vec<usize> = (period / 2..len - period / 2).step_by(period).collect();
    let mut seed = 12345u32;
    let mut pcm = Vec::with_capacity(len * channels);
    for n in 0..len {
        let t = n as f64 / f64::from(sample_rate);
        let mut v = 0.01 * (2.0 * std::f64::consts::PI * 200.0 * t).sin();
        if let Some(&onset) = onsets.iter().rev().find(|&&o| o <= n) {
            let age = (n - onset) as f64 / f64::from(sample_rate);
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = f64::from(seed >> 8) / f64::from(1u32 << 23) - 1.0;
            v += 0.7 * noise * (-age / 0.015).exp();
        }
        for _ in 0..channels {
            pcm.push((v * 32767.0) as i16);
        }
    }
    (pcm, onsets)
}

/// `block_type` de cada granule y canal (0 si no hay conmutación).
fn block_types(mp3: &[u8], channels: usize, frame_len: impl Fn(&[u8]) -> usize) -> Vec<u32> {
    let bit = |data: &[u8], pos: usize| u32::from(data[pos / 8] >> (7 - pos % 8) & 1);
    let bits =
        |data: &[u8], pos: usize, n: usize| (0..n).fold(0, |acc, i| acc << 1 | bit(data, pos + i));
    let prefix = if channels == 1 { 18 } else { 20 };
    let mut types = Vec::new();
    let mut pos = 0;
    while pos + 36 <= mp3.len() {
        let side = &mp3[pos + 4..];
        for unit in 0..2 * channels {
            // part2_3_length (12) + big_values (9) + global_gain (8) + scalefac_compress (4).
            let at = prefix + unit * 59 + 33;
            types.push(if bit(side, at) == 1 {
                bits(side, at + 1, 2)
            } else {
                0
            });
        }
        pos += frame_len(&mp3[pos..]);
    }
    types
}

/// Error entre `from` y `to` muestras antes de cada golpe respecto a la
/// energía del golpe, en dB.
fn pre_echo_db(
    decoded: &[f32],
    input: &[i16],
    onsets: &[usize],
    delay: usize,
    from: usize,
    to: usize,
) -> f64 {
    let (mut noise, mut burst) = (0f64, 0f64);
    for &onset in onsets {
        for n in onset - from..onset - to {
            let x = f64::from(input[n]) / 32768.0;
            noise += (f64::from(decoded[n + delay]) - x).powi(2);
        }
        for &x in &input[onset..onset + 1024] {
            burst += (f64::from(x) / 32768.0).powi(2);
        }
    }
    10.0 * (noise / burst).log10()
}

#[test]
fn transients_use_short_blocks_and_limit_pre_echo() {
    let (input, onsets) = castanets(2 * 44_100, 1, 44_100);
    let (mp3, delay) = encode(&input, 44_100, 1, 128);
    let types = block_types(&mp3, 1, |f| {
        144 * 128_000 / 44_100 + usize::from(f[2] >> 1 & 1)
    });
    let short = types.iter().filter(|&&t| t == 2).count();
    eprintln!(
        "granules cortos: {short} de {} ({} golpes)",
        types.len(),
        onsets.len()
    );
    assert!(short >= onsets.len());
    // Toda ventana start va seguida de bloques cortos y toda stop los sigue.
    for pair in types.windows(2) {
        assert!(pair[0] != 1 || pair[1] == 2, "start sin short: {pair:?}");
        assert!(pair[1] != 3 || pair[0] == 2, "stop sin short: {pair:?}");
    }
    let out = decode(mp3);
    assert_eq!(out.errors, 0);
    // Medido (128 kbps), sin → con bloques cortos:
    // 23 ms antes del golpe: −23.7 → −29.0 dB; de 23 a 10 ms antes (fuera
    // del alcance de una ventana corta): −34.9 → −58.4 dB.
    let near = pre_echo_db(&out.samples, &input, &onsets, delay as usize, 1024, 32);
    let far = pre_echo_db(&out.samples, &input, &onsets, delay as usize, 1024, 448);
    eprintln!("pre-eco 128 kbps: {near:.1} dB (23 ms), {far:.1} dB (23–10 ms) respecto al golpe");
    assert!(
        near < -27.0 && far < -55.0,
        "pre-eco {near:.1} / {far:.1} dB"
    );
}

#[test]
fn transients_decode_in_every_mode() {
    // Ráfagas de ruido blanco: casi incodificables a 64 kbps (medido 4.7 dB;
    // 5.7 dB sin bloques cortos, que gastan algo más de bits).
    for (rate, channels, kbps, min_snr) in [
        (48_000, 2, 64, 3.0),
        (32_000, 2, 320, 31.0),
        (44_100, 2, 192, 27.0),
    ] {
        let (input, _) = castanets(rate as usize, usize::from(channels), rate);
        let (mp3, delay) = encode(&input, rate, channels, kbps);
        let out = decode(mp3);
        assert_eq!(out.errors, 0, "{rate} Hz {channels} ch {kbps} kbps");
        let snr = snr_db(&out.samples, &input, usize::from(channels), delay as usize);
        eprintln!("castañuelas {rate} Hz {channels} ch {kbps} kbps: SNR {snr:.1} dB");
        assert!(
            snr > min_snr,
            "{rate} Hz {channels} ch {kbps} kbps: {snr:.1} dB"
        );
    }
}

#[test]
fn unbalanced_stereo_keeps_both_channels() {
    // Canal derecho 40 dB por debajo del izquierdo (y con otra melodía).
    let left = music(44_100, 1, 44_100);
    let right = music(44_100, 2, 44_100);
    let input: Vec<i16> = left
        .iter()
        .zip(right.iter().skip(1).step_by(2))
        .flat_map(|(&l, &r)| [l, r / 100])
        .collect();
    let (mp3, delay) = encode(&input, 44_100, 2, 128);
    let out = decode(mp3);
    assert_eq!(out.errors, 0);
    let channel =
        |c: usize, v: &[f32]| -> Vec<f32> { v.iter().skip(c).step_by(2).copied().collect() };
    let pcm = |c: usize| -> Vec<i16> { input.iter().skip(c).step_by(2).copied().collect() };
    let snr_l = snr_db(&channel(0, &out.samples), &pcm(0), 1, delay as usize);
    let snr_r = snr_db(&channel(1, &out.samples), &pcm(1), 1, delay as usize);
    eprintln!("estéreo desequilibrado 128 kbps: L {snr_l:.1} dB, R (−40 dB) {snr_r:.1} dB");
    assert!(snr_l > 50.0 && snr_r > 10.0);
}

#[test]
fn full_scale_and_silence_decode_cleanly() {
    let mut input: Vec<i16> = (0..44_100)
        .map(|i| {
            if (i / 50) % 2 == 0 {
                i16::MAX
            } else {
                i16::MIN
            }
        })
        .collect();
    input.extend(std::iter::repeat_n(0, 44_100));
    let (mp3, delay) = encode(&input, 44_100, 1, 128);
    let out = decode(mp3);
    assert_eq!(out.errors, 0);
    // El tramo de silencio sale en silencio (tras el retardo y la cola del filtro).
    let tail = &out.samples[44_100 + delay as usize + 2304..44_100 * 2 + delay as usize];
    assert!(tail.iter().all(|v| v.abs() < 1e-3));
}
