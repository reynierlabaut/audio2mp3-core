# audio2mp3-core

Encoder MP3 (MPEG-1 Layer III) escrito en Rust: pequeño, sin dependencias de runtime y con memoria constante.

```text
PCM S16 interleaved ──► audio2mp3-core ──► frames MP3
```

> Estado: Fase 1 funcional (CBR, bloques largos, sin bit reservoir ni psicoacústica).
> Falta: FFI C, puente JNI, `pcm-convert` y builds de Android/iOS.

## Uso (Rust)

```rust
use audio2mp3_core::{Bitrate, EncoderConfig, Mp3Encoder};

let config = EncoderConfig { sample_rate: 44_100, channels: 2, bitrate: Bitrate::Kbps192 };
let mut encoder = Mp3Encoder::new(config)?;
let mut mp3 = Vec::new();
encoder.encode(&pcm, &mut mp3)?; // PCM i16 intercalado, en bloques de cualquier tamaño
encoder.flush(&mut mp3)?;
```

Prueba manual con un WAV de 16 bits:

```sh
cargo run --release --example encode -- entrada.wav salida.mp3 192
```

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
