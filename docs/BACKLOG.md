# Backlog

Pendientes de `audio2mp3-core`, de más a menos prioritario. Las cifras son las últimas medidas.

## Para usarlo en producción

- [ ] **Release `v0.2.0`** con Fases 2 y 3 + frame Info: versión y `dist/` ya generados
      (`audio2mp3-android-v0.2.0.zip`, sha256 `2c73f435…`); falta el tag y subir zip +
      `SHA256SUMS` a GitHub Releases. `v0.1.0` solo tiene la Fase 1.
- [ ] **Velocidad en Android real**: objetivo ≥ 20× tiempo real en gama media arm64
      (spec §16). En PC: 43–51×. Si no llega: SIMD/NEON en filterbank (matriz 32×64) y en
      `quantize`, menos iteraciones del outer loop (`MAX_OUTER_ITERATIONS`).
- [ ] **Escucha a 128 kbps** (Fase 3, objetivo de bajar el bitrate recomendado): platillos,
      agudos, percusión. Las medidas objetivas lo apoyan (ruido sobre el umbral 72 → 8.6
      dB/frame), falta confirmarlo de oído. Si convence, cambiar el valor por defecto
      recomendado de 192 a 128 en la spec y en `expo-audio2mp3`.
- [ ] **Compatibilidad**: validar con `ffprobe` (sin warnings), FFmpeg, VLC, reproductor de
      Android (MediaPlayer/ExoPlayer) y Apple Music/QuickTime. Hoy solo con symphonia.
- [ ] **Gapless en FFmpeg/symphonia**: solo aplican retardo/padding del tag LAME si el
      identificador es `LAME`/`Lavf`/`Lavc`; el nuestro es `a2m<versión>` (no se suplanta a
      otro encoder). Sí usan el número de frames (duración exacta). Comprobar qué hacen
      ExoPlayer y MediaExtractor.

## Objetivos de la spec sin cumplir

- [ ] **Tamaño de `libaudio2mp3.so`**: arm64-v8a 299 KB, objetivo ≤ 200 KB (spec §16).
      `opt-level = "z"` no lo reduce; ~70 % es `.text`, probablemente `std` (formateo,
      panics/unwind). Medir con `cargo bloat`; valorar quitar bounds checks en caminos
      calientes o `-Z build-std` + `panic_abort` (nightly).
- [ ] **iOS**: `scripts/build-ios.sh` (staticlib → `Audio2Mp3Core.xcframework`, device +
      simulador, feature `ffi` sin `pcm-convert`). Requiere macOS o CI con macOS.
- [ ] **Fuzzing** (`cargo-fuzz` sobre la API FFI y el converter) y **miri** sobre `ffi.rs`.
      Requieren nightly; montarlos en CI.
- [ ] **CI** (GitHub Actions): fmt, clippy `-D warnings`, tests, build Android con
      verificación de 16 KB, `ffprobe` sobre los MP3 de los tests.
- [ ] **Benchmarks** (spec §16): RTF, memoria pico y tamaño de MP3 con 10 s, 1 min, 10 min
      y 30 min; documentarlos en el README.

## Mejoras de calidad (opcionales)

- [ ] Resampler de 48 taps: banda plana hasta ~0.8·Nyquist (hoy ~0.7) a cambio de +50 % de
      cálculo en el converter.
- [ ] Downmix 5.1: hoy normalizado (÷ 2.414, nunca satura, ~7.7 dB más bajo); valorar
      saturación sin escalar o un factor intermedio.
- [ ] Psicoacústica en M/S: el umbral se calcula sobre M y S por separado; LAME usa los
      umbrales de L/R para el canal S.
- [ ] `subblock_gain` en bloques cortos (hoy 0) y `preflag`/`scalefac_scale` (hoy 0).
- [ ] Repartir el reservoir según la entropía perceptual en vez de gastarlo en el frame
      siguiente.

## Fase 4 (no comprometida)

- [ ] MPEG-2/2.5 LSF (8–24 kHz). Innecesario mientras `pcm-convert` lleve todo a 32/44.1/48.
- [ ] VBR.

## Para `expo-audio2mp3` (otro repo)

- El frame Info obliga a reescribir los primeros `infoFrameSize()` bytes al terminar:
  `RandomAccessFile` sobre el `.part`, no basta `FileOutputStream`.
- `converterProcess` acepta `inOffset` para leer los buffers de `MediaCodec` sin copias.
- La declaración Kotlin exacta de `NativeEncoder` está en `crates/audio2mp3-jni/src/lib.rs`.
