# audio2mp3-core

Encoder MP3 (MPEG-1 Layer III) escrito en Rust: pequeño, sin dependencias de runtime y con memoria constante.

```text
PCM S16 interleaved ──► audio2mp3-core ──► frames MP3
```

> Estado: en desarrollo (Fase 1). Todavía no hay API utilizable.

## Estructura

```text
crates/audio2mp3-core   Encoder (features: ffi, pcm-convert)
crates/audio2mp3-jni    Puente JNI para Android → libaudio2mp3.so
include/audio2mp3.h     Cabecera C de la FFI
scripts/                Builds de Android (cargo-ndk) e iOS (XCFramework)
```

## Desarrollo

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

## Licencia

Bajo cualquiera de estas, a tu elección:

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT ([LICENSE-MIT](LICENSE-MIT))

Implementación clean-room a partir de ISO/IEC 11172-3. No contiene código de LAME, shine, FFmpeg ni otros encoders GPL/LGPL.

La ventana del filterbank y las tablas Huffman (`crates/audio2mp3-core/src/tables/{window,huffman}.rs`) son datos derivados de [minimp3](https://github.com/lieff/minimp3) (CC0, dominio público) y verificados por tests independientes de esa fuente.
