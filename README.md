# 🎚️ audio2mp3-core - Rust MP3 Encoder

**English** | [Español](README.es.md)

A small MP3 encoder (MPEG-1 Layer III, CBR) written in Rust, with zero runtime dependencies and constant memory. It powers the "Convert to MP3" feature of the BeatStream app on Android through a JNI bridge, and exposes a C API for any other platform.

```text
PCM S16 interleaved ──► audio2mp3-core ──► MP3 frames
```

## ✨ Features
- 🎧 **Quality:** bit reservoir, joint stereo M/S, short blocks on transients and a psychoacoustic outer loop with scalefactors.
- ⏱️ **Exact duration:** optional Info (Xing) frame with encoder delay and padding for gapless playback.
- 🔄 **PCM conversion:** S16/F32, 1–8 channels and 8–192 kHz to what the encoder needs (resampling and downmix).
- 📱 **Android ready:** `libaudio2mp3.so` for arm64-v8a, armeabi-v7a and x86_64, 16 KB page aligned.
- 🧪 **Verified:** round-trip tests against an independent decoder (symphonia).

Pending work is tracked in [`docs/BACKLOG.md`](docs/BACKLOG.md) and the full specification is in [`docs/SPEC.md`](docs/SPEC.md) (both in Spanish).

## 🦀 Usage (Rust)

```rust
use audio2mp3_core::{Bitrate, EncoderConfig, Mp3Encoder};

let config = EncoderConfig { sample_rate: 44_100, channels: 2, bitrate: Bitrate::Kbps192 };
let mut encoder = Mp3Encoder::new(config)?;
let mut mp3 = Vec::new();
encoder.encode(&pcm, &mut mp3)?; // interleaved i16 PCM, in blocks of any size
encoder.flush(&mut mp3)?;
```

Info frame (optional, exact duration and gapless): call `encoder.enable_info_frame()?` before the first `encode`; after `flush`, `encoder.info_frame_into(&mut mp3[..encoder.info_frame_size()])?` rewrites the first frame (for a file: seek back to the start and overwrite those bytes).

Manual test with a 16-bit WAV:

```sh
cargo run --release --example encode -- input.wav output.mp3 192
```

## 🔌 C API

Feature `ffi`. Full contract in [`include/audio2mp3.h`](include/audio2mp3.h):
`audio2mp3_encoder_create` → `encode` (as many times as needed) → `flush` → `destroy`.
The caller owns every buffer; Rust only allocates the handle.

## 📱 Android (JNI)

`crates/audio2mp3-jni` builds `libaudio2mp3.so`, with no dependencies besides the core.
The matching Kotlin declaration (`expo.modules.audio2mp3.NativeEncoder`) is documented in
`crates/audio2mp3-jni/src/lib.rs`. It uses direct `ByteBuffer`s (no copies);
`encode`/`flush` return the bytes written or `−code` on error.

## 🗂️ Structure

```text
crates/audio2mp3-core   Encoder (features: ffi, pcm-convert)
crates/audio2mp3-jni    JNI bridge for Android → libaudio2mp3.so
include/audio2mp3.h     C header of the FFI
scripts/                Android build (sh and ps1)
docs/                   SPEC.md (specification) and BACKLOG.md (pending work)
```

## 🛠️ Development

```sh
cargo build
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## 🤖 Android build

Requires rustup and the NDK (looked up in `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT` or the
newest one in `$ANDROID_HOME/ndk`). Missing Rust targets are installed automatically.

```sh
scripts/build-android.sh            # Linux, macOS, CI (or Git Bash on Windows)
.\scripts\build-android.ps1         # Windows PowerShell
```

Options: `--ndk <path>` / `-Ndk`, `--api 24` / `-Api` (minSdk), `--no-package` / `-NoPackage`.

It builds `arm64-v8a`, `armeabi-v7a` and `x86_64`, **fails** if any LOAD segment is not
16 KB aligned or if the exported `Java_*` symbols do not match the ones defined in
`audio2mp3-jni`, and produces:

```text
dist/android/jniLibs/<abi>/libaudio2mp3.so
dist/audio2mp3-android-v<version>.zip   (jniLibs/<abi>/libaudio2mp3.so)
dist/SHA256SUMS
```

## 📄 License

Licensed under either of, at your option:

- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT ([LICENSE-MIT](LICENSE-MIT))

Clean-room implementation based on ISO/IEC 11172-3. It contains no code from LAME, shine, FFmpeg or any other GPL/LGPL encoder.

The filterbank window and the Huffman tables (`crates/audio2mp3-core/src/tables/{window,huffman}.rs`) are data derived from [minimp3](https://github.com/lieff/minimp3) (CC0, public domain) and verified by tests that do not depend on that source.

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
