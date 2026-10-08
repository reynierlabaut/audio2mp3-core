# Changelog

## [Unreleased]

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
