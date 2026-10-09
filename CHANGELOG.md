# Changelog

## [Unreleased]

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
