use super::*;

fn s16_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_ne_bytes()).collect()
}

fn f32_bytes(samples: &[f32]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_ne_bytes()).collect()
}

/// Convierte `input` en bloques de `chunk` bytes y vacía el converter.
fn convert_all(conv: &mut PcmConverter, input: &[u8], chunk: usize) -> Vec<i16> {
    let mut out = Vec::new();
    for part in input.chunks(chunk.max(1)) {
        let mut buf = vec![0; conv.max_output_samples(part.len())];
        let n = conv.process_into(part, &mut buf).unwrap();
        out.extend_from_slice(&buf[..n]);
    }
    let mut buf = vec![0; conv.max_flush_samples()];
    let n = conv.flush_into(&mut buf).unwrap();
    out.extend_from_slice(&buf[..n]);
    out
}

fn sine(rate: u32, freq: f64, frames: usize, channels: usize, amp: f64) -> Vec<f32> {
    (0..frames * channels)
        .map(|i| {
            let t = (i / channels) as f64 / f64::from(rate);
            (amp * (2.0 * core::f64::consts::PI * freq * t).sin()) as f32
        })
        .collect()
}

/// SNR de `got` (S16) frente a una sinusoide ideal a la tasa de salida,
/// ignorando `skip` tramas en cada extremo.
fn snr_vs_sine(got: &[i16], rate: u32, freq: f64, amp: f64, skip: usize) -> f64 {
    let (mut s, mut e) = (0f64, 0f64);
    let end = got.len().saturating_sub(skip);
    for (n, &y) in got.iter().enumerate().take(end).skip(skip) {
        let x =
            amp * 32767.0 * (2.0 * core::f64::consts::PI * freq * n as f64 / f64::from(rate)).sin();
        s += x * x;
        e += (f64::from(y) - x).powi(2);
    }
    10.0 * (s / e).log10()
}

#[test]
fn output_rate_rule() {
    for (input, expected) in [
        (32_000, 32_000),
        (44_100, 44_100),
        (48_000, 48_000),
        (8_000, 32_000),
        (16_000, 32_000),
        (11_025, 44_100),
        (22_050, 44_100),
        (88_200, 44_100),
        (176_400, 44_100),
        (12_000, 48_000),
        (24_000, 48_000),
        (96_000, 48_000),
        (192_000, 48_000),
        (37_800, 48_000),
    ] {
        assert_eq!(output_rate(input), expected, "{input}");
    }
}

#[test]
fn rejects_invalid_parameters() {
    assert!(matches!(
        PcmConverter::new(7_999, 2, SampleFormat::S16),
        Err(Error::InvalidSampleRate)
    ));
    assert!(matches!(
        PcmConverter::new(192_001, 2, SampleFormat::S16),
        Err(Error::InvalidSampleRate)
    ));
    assert!(matches!(
        PcmConverter::new(44_100, 0, SampleFormat::S16),
        Err(Error::InvalidChannels)
    ));
    assert!(matches!(
        PcmConverter::new(44_100, 9, SampleFormat::F32),
        Err(Error::InvalidChannels)
    ));
}

#[test]
fn passthrough_is_bit_exact_with_any_chunking() {
    let samples: Vec<i16> = (0..4_000)
        .map(|i| (i * 7919 % 65_536 - 32_768) as i16)
        .collect();
    let bytes = s16_bytes(&samples);
    for chunk in [1, 3, 5, 4096, bytes.len()] {
        let mut conv = PcmConverter::new(44_100, 2, SampleFormat::S16).unwrap();
        assert_eq!((conv.out_rate(), conv.out_channels()), (44_100, 2));
        assert_eq!(
            convert_all(&mut conv, &bytes, chunk),
            samples,
            "chunk {chunk}"
        );
    }
}

#[test]
fn f32_saturates_and_rounds() {
    let mut conv = PcmConverter::new(48_000, 1, SampleFormat::F32).unwrap();
    let input = f32_bytes(&[1.5, -2.0, 0.5, f32::NAN, -0.25, 1.0]);
    assert_eq!(
        convert_all(&mut conv, &input, 7),
        [32767, -32767, 16384, 0, -8192, 32767]
    );
}

#[test]
fn downmix_5_1_follows_spec_and_never_clips() {
    let mut conv = PcmConverter::new(48_000, 6, SampleFormat::S16).unwrap();
    assert_eq!(conv.out_channels(), 2);
    // L, R, C, LFE, Ls, Rs
    let frame = [
        1000, 0, 1000, 30_000, 0, 1000, 32_767, 32_767, 32_767, 32_767, 32_767, 32_767,
    ];
    let out = convert_all(&mut conv, &s16_bytes(&frame), 12);
    let norm = 1.0 / (1.0 + 2.0 * core::f64::consts::FRAC_1_SQRT_2);
    let l = (1000.0 + 1000.0 * core::f64::consts::FRAC_1_SQRT_2) * norm;
    let r = (2.0 * 1000.0 * core::f64::consts::FRAC_1_SQRT_2) * norm;
    assert!((f64::from(out[0]) - l).abs() <= 1.0, "{} vs {l}", out[0]);
    assert!((f64::from(out[1]) - r).abs() <= 1.0, "{} vs {r}", out[1]);
    // Todos los canales a fondo de escala: no satura.
    assert_eq!(&out[2..], [32_767, 32_767]);
}

#[test]
fn mono_stays_mono_and_stereo_stays_stereo() {
    assert_eq!(
        PcmConverter::new(22_050, 1, SampleFormat::F32)
            .unwrap()
            .out_channels(),
        1
    );
    assert_eq!(
        PcmConverter::new(22_050, 2, SampleFormat::F32)
            .unwrap()
            .out_channels(),
        2
    );
    assert_eq!(
        PcmConverter::new(22_050, 8, SampleFormat::F32)
            .unwrap()
            .out_channels(),
        2
    );
}

#[test]
fn resampled_length_is_exact() {
    for in_rate in [
        8_000, 11_025, 22_050, 37_800, 47_999, 64_000, 96_000, 192_000,
    ] {
        for frames in [0, 1, 999, 10_000] {
            let mut conv = PcmConverter::new(in_rate, 2, SampleFormat::F32).unwrap();
            let out_rate = u64::from(conv.out_rate());
            let input = f32_bytes(&sine(in_rate, 440.0, frames, 2, 0.5));
            let out = convert_all(&mut conv, &input, 4096);
            let expected = (frames as u64 * out_rate).div_ceil(u64::from(in_rate)) as usize;
            assert_eq!(out.len(), expected * 2, "{in_rate} Hz, {frames} tramas");
        }
    }
}

#[test]
fn resampling_is_identical_with_any_chunking() {
    let input = f32_bytes(&sine(22_050, 1234.0, 3_000, 2, 0.7));
    let mut reference = PcmConverter::new(22_050, 2, SampleFormat::F32).unwrap();
    let expected = convert_all(&mut reference, &input, input.len());
    for chunk in [1, 3, 7, 1000] {
        let mut conv = PcmConverter::new(22_050, 2, SampleFormat::F32).unwrap();
        assert_eq!(
            convert_all(&mut conv, &input, chunk),
            expected,
            "chunk {chunk}"
        );
    }
}

#[test]
fn passband_tones_are_preserved() {
    // Tonos hasta ~0.7 del Nyquist de la tasa más baja (banda plana del filtro).
    for (in_rate, freq) in [
        (22_050, 1_000.0),
        (22_050, 7_500.0),
        (8_000, 2_500.0),
        (96_000, 5_000.0),
        (96_000, 16_000.0),
        (37_800, 13_000.0),
        (64_000, 15_000.0),
        (44_100, 9_000.0),
    ] {
        let mut conv = PcmConverter::new(in_rate, 1, SampleFormat::F32).unwrap();
        let out_rate = conv.out_rate();
        let input = f32_bytes(&sine(in_rate, freq, in_rate as usize / 4, 1, 0.5));
        let out = convert_all(&mut conv, &input, 4096);
        let snr = snr_vs_sine(&out, out_rate, freq, 0.5, 200);
        eprintln!("{in_rate} → {out_rate} Hz, tono {freq} Hz: SNR {snr:.1} dB");
        assert!(snr > 60.0, "{in_rate} → {out_rate}, {freq} Hz: {snr:.1} dB");
    }
}

#[test]
fn downsampling_rejects_aliases() {
    for (in_rate, freq) in [(96_000, 30_000.0), (192_000, 40_000.0), (88_200, 26_000.0)] {
        let mut conv = PcmConverter::new(in_rate, 1, SampleFormat::F32).unwrap();
        let input = f32_bytes(&sine(in_rate, freq, in_rate as usize / 4, 1, 0.5));
        let out = convert_all(&mut conv, &input, 4096);
        let body = &out[200..out.len() - 200];
        let rms =
            (body.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / body.len() as f64).sqrt();
        let level = 20.0 * (rms / (0.5 * 32767.0 / 2f64.sqrt())).log10();
        eprintln!("{in_rate} Hz, tono {freq} Hz por encima del Nyquist de salida: {level:.1} dB");
        assert!(level < -60.0, "{in_rate} Hz, {freq} Hz: {level:.1} dB");
    }
}

#[test]
fn buffer_too_small_does_not_consume_and_finished_after_flush() {
    let mut conv = PcmConverter::new(22_050, 2, SampleFormat::S16).unwrap();
    let input = s16_bytes(&[100; 2_000]);
    let mut small = [0i16; 4];
    assert_eq!(
        conv.process_into(&input, &mut small),
        Err(Error::BufferTooSmall)
    );
    assert_eq!(conv.flush_into(&mut small), Err(Error::BufferTooSmall));
    let mut out = vec![0; conv.max_output_samples(input.len())];
    assert!(conv.process_into(&input, &mut out).unwrap() > 0);
    let mut tail = vec![0; conv.max_flush_samples()];
    conv.flush_into(&mut tail).unwrap();
    assert_eq!(conv.process_into(&input, &mut out), Err(Error::Finished));
    assert_eq!(conv.flush_into(&mut tail), Err(Error::Finished));
}
