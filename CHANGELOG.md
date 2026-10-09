# Changelog

## [Unreleased]

Fase 2 (sin cambios en la API pública):

- Bit reservoir: `main_data_begin` hasta 511 bytes, limitado por el buffer de 960 bytes
  del decoder (0 a 320 kbps / 44.1 kHz). La salida va hasta unos frames por detrás;
  `max_output_size` / `max_flush_size` lo incluyen.
- Joint stereo M/S por frame, decidido estimando los bits de L/R y M/S por banda.
- Rate loop por frame con ganancia común (ruido repartido por igual entre granules y
  canales) y ganancia mínima 125 (ruido bajo 16 bits; el sobrante va al reservoir).
- SNR 44.1 kHz estéreo: 64 kbps 35.3 → 44.5 dB, 128 kbps 51.3 → 59.5, 192 kbps
  57.8 → 67.1, 320 kbps 66.3 → 71.9. Velocidad en PC: 47× → 64× tiempo real.

Fase 3 (en curso):

- Bloques cortos y ventanas start/stop ante transitorios: detector de ataques sobre
  la energía de las subbandas con un granule de lookahead; mismo tipo de bloque en
  ambos canales. Pre-eco (castañuelas, 128 kbps) de 23 a 10 ms antes del golpe:
  −34.9 → −58.4 dB. Sin cambios en señales sin transitorios.
- Modelo psicoacústico en el dominio MDCT (tonalidad por planitud espectral,
  propagación de Schroeder en Bark, umbral absoluto de Terhardt) y outer loop con
  scalefactors (`scalefac_compress`, part2) de hasta 8 iteraciones. Ruido sobre el umbral
  (señal de los tests): 64 kbps 95.9 → 75.0 dB/frame, 128 kbps 72.3 → 8.6, 192 kbps
  46.6 → 0.0. El SNR baja como mucho 1.6 dB (192 kbps). Velocidad en PC: 43–51×.

## [0.1.0] - 2026-10-08

Primera versión (Fase 1). Pendiente conocido: `libaudio2mp3.so` arm64-v8a ocupa 285 KB
(objetivo ≤ 200 KB) y no hay build de iOS todavía.

- Estructura inicial del workspace.
- Fase 1: `Mp3Encoder` (MPEG-1 Layer III, CBR 64–320 kbps, 32/44.1/48 kHz, mono/estéreo),
  streaming con salida idéntica para cualquier tamaño de bloque y sin reservas tras `new()`.
- Tablas Huffman y ventana del filterbank derivadas de minimp3 (CC0) y verificadas por tests.
- Tests de ida y vuelta con symphonia (dev-dependency) y `examples/encode.rs` (WAV → MP3).
- API C (feature `ffi`) e `include/audio2mp3.h`; añade `audio2mp3_encoder_delay_samples`.
- Puente JNI (`libaudio2mp3.so`) sin dependencias: `NativeEncoder` con `ByteBuffer` directos.
- `pcm-convert`: `PcmConverter` (S16/F32, 1–8 canales, 8–192 kHz → S16 1–2 canales a
  32/44.1/48 kHz) con downmix normalizado y resampler polifásico de 32 taps; API C
  (`audio2mp3_converter_*`, añade `max_flush_samples`) y JNI (`converter*`, con `inOffset`).
- `scripts/build-android.{sh,ps1}`: 3 ABIs con el NDK (sin cargo-ndk), verificación de
  alineado a 16 KB y de símbolos JNI, zip + `SHA256SUMS` para la release.
