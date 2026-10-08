# Changelog

## [Unreleased]

- Estructura inicial del workspace.
- Fase 1: `Mp3Encoder` (MPEG-1 Layer III, CBR 64–320 kbps, 32/44.1/48 kHz, mono/estéreo),
  streaming con salida idéntica para cualquier tamaño de bloque y sin reservas tras `new()`.
- Tablas Huffman y ventana del filterbank derivadas de minimp3 (CC0) y verificadas por tests.
- Tests de ida y vuelta con symphonia (dev-dependency) y `examples/encode.rs` (WAV → MP3).
