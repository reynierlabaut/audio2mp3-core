# audio2mp3-core

Encoder MP3 (MPEG-1 Layer III) escrito en Rust: pequeño, sin dependencias de runtime y con memoria constante.

```text
PCM S16 interleaved ──► audio2mp3-core ──► frames MP3
```

> Estado: Fase 1 funcional (CBR, bloques largos, sin bit reservoir ni psicoacústica),
> con `pcm-convert`, API C (`include/audio2mp3.h`), puente JNI y build de Android.
> Falta: build de iOS (XCFramework).

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

## API C

Feature `ffi`. Contrato completo en [`include/audio2mp3.h`](include/audio2mp3.h):
`audio2mp3_encoder_create` → `encode` (las veces que haga falta) → `flush` → `destroy`.
Los buffers los reserva el llamador; Rust solo reserva el handle.

## Android (JNI)

`crates/audio2mp3-jni` genera `libaudio2mp3.so`, sin dependencias aparte del core.
La declaración Kotlin correspondiente (`expo.modules.audio2mp3.NativeEncoder`) está
en la documentación de `crates/audio2mp3-jni/src/lib.rs`. Usa `ByteBuffer` directos
(sin copias); `encode`/`flush` devuelven los bytes escritos o `−código` de error.

## Pendientes conocidos

- TODO(fase2): outer loop con scalefactors (distorsión por banda). Necesita el ruido
  permitido por banda del modelo psicoacústico (Fase 3); hoy `scalefac_compress = 0`.
- TODO(fase2): frame Info/Xing + etiqueta LAME (duración exacta, delay/padding para
  gapless). Requiere API aditiva: reservar el primer frame y reescribirlo tras `flush`.

- TODO(fase1): tamaño de `libaudio2mp3.so` arm64-v8a = 285 KB con el converter, objetivo ≤ 200 KB (spec §16).
  `opt-level = "z"` no lo reduce; el 70 % es `.text`, probablemente de `std`
  (formateo, panics/unwind). Medir con `cargo bloat` y valorar quitar bounds checks
  en caminos calientes o `-Z build-std` con `panic_abort` (nightly).

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
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Build de Android

Requiere rustup y el NDK (lo busca en `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT` o el más
reciente de `$ANDROID_HOME/ndk`). Instala los targets de Rust que falten.

```sh
scripts/build-android.sh            # Linux, macOS, CI (o Git Bash en Windows)
.\scripts\build-android.ps1         # Windows PowerShell
```

Opciones: `--ndk <ruta>` / `-Ndk`, `--api 24` / `-Api` (minSdk), `--no-package` / `-NoPackage`.

Compila `arm64-v8a`, `armeabi-v7a` y `x86_64`, **falla** si algún segmento LOAD no está
alineado a 16 KB o si los símbolos `Java_*` exportados no coinciden con los de
`audio2mp3-jni`, y deja:

```text
dist/android/jniLibs/<abi>/libaudio2mp3.so
dist/audio2mp3-android-v<versión>.zip   (jniLibs/<abi>/libaudio2mp3.so)
dist/SHA256SUMS
```

## Licencia

Bajo cualquiera de estas, a tu elección:

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT ([LICENSE-MIT](LICENSE-MIT))

Implementación clean-room a partir de ISO/IEC 11172-3. No contiene código de LAME, shine, FFmpeg ni otros encoders GPL/LGPL.

La ventana del filterbank y las tablas Huffman (`crates/audio2mp3-core/src/tables/{window,huffman}.rs`) son datos derivados de [minimp3](https://github.com/lieff/minimp3) (CC0, dominio público) y verificados por tests independientes de esa fuente.
