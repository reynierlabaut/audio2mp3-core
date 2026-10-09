# audio2mp3-core — contexto para agentes

Encoder MP3 (MPEG-1 Layer III, CBR) en Rust, sin dependencias de runtime. Consumidor:
`expo-audio2mp3` (Android vía JNI) para la app BeatStream. Textos, comentarios y commits en
**español**. Fuente de verdad: `docs/SPEC.md` (qué se pide) y `docs/BACKLOG.md` (qué falta).
No re-leer la spec entera: buscar la sección con Grep.

## Entorno (Windows, PowerShell 5.1)

- Rust con toolchain **GNU** (no hay MSVC). `cargo` no está en el PATH de la sesión:
  empezar cada comando con `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path";`.
- El cwd se resetea entre comandos: `Set-Location C:\Users\ReynierLS\audio2mp3-core;` al
  inicio, o rutas absolutas (los bloqueos de seguridad evalúan rutas relativas contra otro dir).
- NDK: `%LOCALAPPDATA%\Android\Sdk\ndk\27.1.12297006` (lo encuentra el script de build).
- `git push` lo hace el usuario (clave SSH con passphrase): no intentarlo, se cuelga.
- Ediciones con PowerShell sobre archivos: usar `[IO.File]::ReadAllText/WriteAllText`
  (no `Set-Content`, cambia la codificación). Preferir la herramienta Edit.

## Comandos (siempre con la salida filtrada)

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets --all-features --quiet -- -D warnings 2>&1 | Select-String '^(error|warning)|-->'
cargo test --workspace --all-features --quiet --release 2>&1 | Select-String 'test result|FAILED|panicked'
cargo test --quiet --release -p audio2mp3-core --test roundtrip -- --nocapture 2>&1 | Select-String 'SNR|pre-eco'
cargo run --quiet --release --example encode -- in.wav out.mp3 128   # WAV 16 bits → MP3, imprime ×tiempo real
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-android.ps1 [-NoPackage]
```

Tests en `--release` (en debug los de ida y vuelta tardan mucho). Hay ~93 tests; todos
deben pasar y clippy debe quedar limpio antes de cada commit.

## Mapa (`crates/audio2mp3-core/src`)

| Módulo | Qué hace / datos clave |
|---|---|
| `encoder.rs` | `Mp3Encoder`. Entrada por granules (576) con 1 de lookahead (cola de 3) → tipo de bloque → MDCT → M/S → outer loop → Huffman → `reservoir`. `DELAY = 1057`. |
| `filterbank.rs`, `mdct.rs` | Análisis 32 subbandas; MDCT larga/start/short/stop (escala 2/18 y 2/6, ganancia 1). |
| `quantize.rs` | Cuantización, partición big_values/count1, `pow43_table`, `distortion`. |
| `huffman.rs` | Selección de tablas y escritura; `plan(.., switching)` para bloques cortos. |
| `psy.rs` | Umbral por banda en dominio MDCT (planitud, Schroeder, Terhardt). `FULL_SCALE_ENERGY_*` calibrados por test. |
| `scalefac.rs` | `scalefac_compress` y part2. |
| `reservoir.rs` | Bit reservoir (≤ 511 bytes, buffer decoder 960) y montaje de frames. |
| `frame.rs`, `info.rs` | Cabecera/side info; frame Info (Xing CBR + LAME, id `a2m<versión>`). |
| `convert/` | `PcmConverter` (feature `pcm-convert`): downmix, resampler 32 taps, F32→S16. |
| `ffi.rs`, `ffi/converter.rs` | API C (feature `ffi`); contrato en `include/audio2mp3.h`. |
| `tables/` | `window.rs` y `huffman.rs` **generados** desde minimp3 (CC0): no editar a mano; `tests.rs` los verifica. |
| `../tests/roundtrip.rs` | Ida y vuelta con symphonia (dev-dependency): errores, retardo, SNR, pre-eco, gapless. |

JNI: `crates/audio2mp3-jni/src/lib.rs` (sin dependencias; tabla JNI a mano, índices 230/231).
La declaración Kotlin de `NativeEncoder` está en su doc de cabecera: mantenerla al día.

## Reglas

- **Cero dependencias de runtime** (`cargo tree -e normal` solo muestra el crate).
  `symphonia` solo en `dev-dependencies`.
- **Sin `unsafe`** fuera de `ffi*.rs` y del crate JNI; cada bloque con comentario `SAFETY`.
- **Sin panics alcanzables**: nada de `unwrap`/`expect`/indexado no probado en código de
  librería; usar `get`, iteradores y `Error::Internal`. `panic = "abort"` tumbaría la app.
- **Sin reservar memoria tras `new()`** en `encode_into`/`flush_into`/`process_into`.
- **Salida determinista**: mismos bytes con cualquier tamaño de bloque de entrada (hay test).
- **API pública solo aditiva** (Rust, C y JNI a la vez). Al tocar la FFI: actualizar
  `include/audio2mp3.h` (el test `header_matches_codes_and_functions` lo exige). Al tocar
  JNI: el script de build compara los `Java_*` exportados con los definidos.
- **Clean-room**: nada de código de LAME/shine/FFmpeg ni GPL/LGPL; datos de tablas del
  estándar o CC0, verificados por tests. No suplantar a otro encoder (p. ej. id `LAME`).
- Estilo: `rustfmt` por defecto; comentarios de cabecera `//!` por módulo; constantes con
  el porqué y la medida que las justifica.

## Cómo trabajar

- **Medir antes de decidir**: para comparar variantes, cambiar temporalmente la constante
  (p. ej. `MAX_OUTER_ITERATIONS`, `MIN_GAIN`, `ATTACK_RATIO`), ejecutar el test con
  `--nocapture` y restaurar el archivo. Registrar las cifras en el doc de la constante.
- Umbrales de regresión en `tests/roundtrip.rs` ≈ 2 dB por debajo de lo medido; solo suben.
  Desde la Fase 3 el SNR no es la medida principal (el outer loop cambia SNR por ruido
  enmascarado): ver `encoder::tests::outer_loop_*`.
- Al terminar una tarea: CHANGELOG (`[Unreleased]`), README/SPEC si cambia el estado,
  `docs/BACKLOG.md` (marcar o añadir), commit en español con cuerpo breve.
- README en dos archivos (formato de los repos del autor, igual que BeatStream):
  `README.md` en inglés (principal) y `README.es.md` en español, con el selector
  `**English** | [Español](README.es.md)` arriba y mismo contenido en ambos: cualquier
  cambio va en los dos. Al final de cada uno, la firma (bloque `<div align="center">` con
  el banner SNAKE_EYES, *Code clean, Strike fast* y @RyLabaut): no tocarla ni quitarla.

## Ahorro de tokens

- Grep/Glob antes de Read; leer solo el rango necesario (`offset`/`limit`).
- No volcar salidas: filtrar con `Select-String` / `Select-Object -First/-Last`.
- No releer archivos ya leídos ni re-explicar lo que está aquí o en la spec.
- Cambios con Edit; Write solo para archivos nuevos.
- Respuestas breves: qué se hizo, cifras verificadas, pendientes.
