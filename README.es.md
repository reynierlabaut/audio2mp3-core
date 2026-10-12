# 🎚️ audio2mp3-core - Encoder MP3 en Rust

[English](https://github.com/reynierlabaut/audio2mp3-core/blob/main/README.md) | **Español**

Encoder MP3 (MPEG-1 Layer III, CBR) escrito en Rust: pequeño, sin dependencias de runtime y con memoria constante. Es el motor de la función "Convertir a MP3" de la app BeatStream en Android, mediante un puente JNI, y ofrece una API C para cualquier otra plataforma.

```text
PCM S16 intercalado ──► audio2mp3-core ──► frames MP3
```

## ✨ Características
- 🎧 **Calidad:** bit reservoir, joint stereo M/S, bloques cortos ante transitorios y outer loop psicoacústico con scalefactors.
- ⏱️ **Duración exacta:** frame Info (Xing) opcional con el retardo y el padding del encoder para reproducción sin huecos.
- 🔄 **Conversión de PCM:** S16/F32, de 1 a 8 canales y de 8 a 192 kHz a lo que necesita el encoder (resampling y downmix).
- 📱 **Listo para Android:** `libaudio2mp3.so` para arm64-v8a, armeabi-v7a y x86_64, alineado a páginas de 16 KB.
- 🧪 **Verificado:** tests de ida y vuelta con un decoder independiente (symphonia).

Los pendientes están en [`docs/BACKLOG.md`](https://github.com/reynierlabaut/audio2mp3-core/blob/main/docs/BACKLOG.md) y la especificación completa en [`docs/SPEC.md`](https://github.com/reynierlabaut/audio2mp3-core/blob/main/docs/SPEC.md).

## 🦀 Uso (Rust)

```rust
use audio2mp3_core::{Bitrate, EncoderConfig, Mp3Encoder};

let config = EncoderConfig { sample_rate: 44_100, channels: 2, bitrate: Bitrate::Kbps192 };
let mut encoder = Mp3Encoder::new(config)?;
let mut mp3 = Vec::new();
encoder.encode(&pcm, &mut mp3)?; // PCM i16 intercalado, en bloques de cualquier tamaño
encoder.flush(&mut mp3)?;
```

Frame Info (opcional, duración exacta y gapless): `encoder.enable_info_frame()?` antes del primer `encode`; tras `flush`, `encoder.info_frame_into(&mut mp3[..encoder.info_frame_size()])?` reescribe el primer frame (en un archivo: volver al inicio y sobrescribir esos bytes).

Prueba manual con un WAV de 16 bits:

```sh
cargo run --release --example encode -- entrada.wav salida.mp3 192
```

## 🔌 API C

Feature `ffi`. Contrato completo en [`include/audio2mp3.h`](https://github.com/reynierlabaut/audio2mp3-core/blob/main/include/audio2mp3.h):
`audio2mp3_encoder_create` → `encode` (las veces que haga falta) → `flush` → `destroy`.
Los buffers los reserva el llamador; Rust solo reserva el handle.

## 📱 Android (JNI)

`crates/audio2mp3-jni` genera `libaudio2mp3.so`, sin dependencias aparte del core.
La declaración Kotlin correspondiente (`expo.modules.audio2mp3.NativeEncoder`) está
en la documentación de `crates/audio2mp3-jni/src/lib.rs`. Usa `ByteBuffer` directos
(sin copias); `encode`/`flush` devuelven los bytes escritos o `−código` de error.

## 🗂️ Estructura

```text
crates/audio2mp3-core   Encoder (features: ffi, pcm-convert)
crates/audio2mp3-jni    Puente JNI para Android → libaudio2mp3.so
include/audio2mp3.h     Cabecera C de la FFI
scripts/                Build de Android (sh y ps1)
docs/                   SPEC.md (especificación) y BACKLOG.md (pendientes)
```

## 🛠️ Desarrollo

```sh
cargo build
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## 🤖 Build de Android

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

## 📄 Licencia

Bajo cualquiera de estas, a tu elección:

- Apache License 2.0 ([LICENSE-APACHE](https://github.com/reynierlabaut/audio2mp3-core/blob/main/LICENSE-APACHE))
- MIT ([LICENSE-MIT](https://github.com/reynierlabaut/audio2mp3-core/blob/main/LICENSE-MIT))

Implementación clean-room a partir de ISO/IEC 11172-3. No contiene código de LAME, shine, FFmpeg ni otros encoders GPL/LGPL.

La ventana del filterbank y las tablas Huffman (`crates/audio2mp3-core/src/tables/{window,huffman}.rs`) son datos derivados de [minimp3](https://github.com/lieff/minimp3) (CC0, dominio público) y verificados por tests independientes de esa fuente.

<div align="center">
<pre style="user-select: none; -webkit-user-select: none; -moz-user-select: none; -ms-user-select: none; display: inline-block; text-align: left; font-family: monospace; font-size: 12px; line-height: 1.2; background: transparent; border: none; box-shadow: none;">
███████╗███╗   ██╗ █████╗ ██╗  ██╗███████╗        ███████╗██╗   ██╗███████╗███████╗
██╔════╝████╗  ██║██╔══██╗██║ ██╔╝██╔════╝        ██╔════╝╚██╗ ██╔╝██╔════╝██╔════╝
███████╗██╔██╗ ██║███████║█████╔╝ █████╗          █████╗   ╚████╔╝ █████╗  ███████╗
╚════██║██║╚██╗██║██╔══██║██╔═██╗ ██╔══╝          ██╔══╝    ╚██╔╝  ██╔══╝  ╚════██║
███████║██║ ╚████║██║  ██║██║  ██╗███████╗███████╗███████╗   ██║   ███████╗███████║
╚══════╝╚═╝  ╚═══╝╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝╚══════╝╚══════╝   ╚═╝   ╚══════╝╚══════╝
</pre>
  <div style="text-align: center; font-family: sans-serif;">
  <strong><em>Code clean, Strike fast</em></strong><br>
  <span style="margin-top: 5px;">@RyLabaut</span>
  </div>
</div>
