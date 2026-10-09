# audio2mp3-core — Especificación v2

## 1. Descripción

`audio2mp3-core` es un encoder MP3 (MPEG-1 Layer III) escrito en **Rust**, propio, pequeño y sin dependencias de runtime.

```text
PCM S16 interleaved ──► audio2mp3-core ──► frames MP3
```

Es una biblioteca independiente de cualquier framework. Su primer consumidor es `expo-audio2mp3`, pero debe poder usarse desde cualquier lenguaje vía FFI C.

## 2. Licencia y origen del código

- Licencia del proyecto: **MIT OR Apache-2.0**.
- Implementación **clean-room** a partir de:
  - ISO/IEC 11172-3 (MPEG-1 Audio);
  - ISO/IEC 13818-3 (solo si en el futuro se añaden tasas bajas);
  - literatura técnica pública (papers, libros, documentación de formato).
- **Prohibido** copiar, traducir o portar código de LAME, shine, FFmpeg u otros encoders con licencia GPL/LGPL.
- Las tablas numéricas del estándar (ventanas, Huffman, bandas de scalefactors) se transcriben desde el estándar o se **derivan de fuentes de dominio público (CC0)**, nunca de código GPL/LGPL. Solo se toman los datos, no el código.
  - Ventana del filterbank y tablas Huffman: derivadas de minimp3 (CC0) con una herramienta externa de extracción; los archivos generados indican su origen y no se editan a mano.
  - Toda tabla derivada se **verifica con tests independientes de la fuente**: códigos prefijo completos (Kraft), dimensiones y linbits, valores conocidos del estándar, simetría de la ventana y reconstrucción análisis + síntesis del estándar.

## 3. Prioridades

En orden:

1. Producir MP3 válido y reproducible en cualquier reproductor.
2. Tamaño binario reducido.
3. Cero dependencias de runtime.
4. Memoria constante (streaming).
5. Calidad de audio.
6. Rendimiento suficiente para móviles.
7. API simple y estable, apta para FFI.

## 4. Fases

El encoder se construye en fases. **La API pública no cambia entre fases**: cada fase solo mejora la calidad interna.

### Fase 1 — MP3 válido (MVP)

- MPEG-1 Layer III.
- Sample rates: **32 000, 44 100, 48 000 Hz**.
- Mono y stereo (L/R simple, sin joint stereo).
- CBR.
- Bitrates: 64, 96, 128, 160, 192, 224, 256, 320 kbps.
- Filterbank polifásico de 32 subbandas.
- MDCT solo con **bloques largos** (`block_type = 0`).
- Reducción de aliasing (butterflies).
- Cuantización con **inner loop** (rate loop sobre `global_gain`); scalefactors a 0.
- Huffman: big_values, count1, división en regiones, selección de tablas.
- Sin bit reservoir (`main_data_begin = 0`).
- Streaming con estado entre llamadas.
- Módulo `pcm-convert` (resampler + downmix + f32→s16), ver §9.
- API Rust + FFI C.

**Bitrate recomendado por defecto para los consumidores en esta fase: 192 kbps.**

### Fase 2 — Eficiencia

- Bit reservoir.
- Joint stereo M/S (decisión por frame).
- Outer loop (distorsión por banda + scalefactors). Implementado junto con el modelo
  psicoacústico de la Fase 3.
- Frame **Info/Xing** opcional (duración exacta, delay/padding para gapless). **TODO**:
  pospuesto; requiere funciones nuevas (aditivas) para reescribir el primer frame tras `flush`.

Implementado: bit reservoir, M/S por frame (estimación de bits por banda) y rate loop
con ganancia común por frame.

### Fase 3 — Calidad

- Modelo psicoacústico (energía, spreading, enmascaramiento, umbral absoluto de audición).
- Bloques cortos y ventanas de transición (start/stop) con detección de transitorios.
- Ajuste para bajar el bitrate recomendado a 128 kbps.

Implementado: bloques cortos (detector de ataques en subbandas con un granule de
lookahead) y modelo psicoacústico en el dominio MDCT (planitud espectral, Schroeder,
Terhardt) con outer loop. Pendiente: validar con escucha si 128 kbps basta como
recomendación; las medidas objetivas (ruido sobre el umbral) lo apoyan.

### Posible Fase 4 (no comprometida)

- MPEG-2/2.5 LSF (8–24 kHz).
- VBR.

## 5. Fuera del alcance

- Decodificación de cualquier formato (AAC, MP3, FLAC, Opus…).
- Parsers de contenedores (MP4, MKV, WebM, WAV obligatorio).
- Metadata ID3, carátulas, letras.
- Efectos, normalización, ecualizador, mezclador.
- Acceso al sistema de archivos (el core trabaja solo con buffers).
- Reproducción, red, async/Tokio.
- CLI (solo un `examples/encode.rs` para pruebas).

## 6. Dependencias

```toml
[dependencies]
# vacío
```

- Runtime: **ninguna**. Solo `core` + `alloc` + `std`.
- `[dev-dependencies]` permitidas (no afectan al binario): un decoder MP3 para tests de ida y vuelta (ej. `minimp3` o `symphonia`), utilidades de test y benchmarks.
- Cualquier dependencia de runtime futura requiere medir antes su impacto en el tamaño del `.so` y del XCFramework.

## 7. Formato de entrada

- PCM **signed 16-bit**, endianness nativa, **interleaved**:
  - mono: `M M M …`
  - stereo: `L R L R …`
- `samples.len()` debe ser múltiplo de `channels`. Si no, `Error::InvalidInput`.
- Se aceptan bloques de **cualquier tamaño**, incluido 0. El encoder acumula internamente hasta tener 1152 muestras por canal (un granule pair = un frame).

## 8. API Rust

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bitrate { Kbps64, Kbps96, Kbps128, Kbps160, Kbps192, Kbps224, Kbps256, Kbps320 }

impl Bitrate {
    pub fn from_kbps(kbps: u16) -> Result<Self, Error>;
    pub fn kbps(self) -> u16;
}

#[derive(Clone, Copy, Debug)]
pub struct EncoderConfig {
    pub sample_rate: u32, // 32000 | 44100 | 48000
    pub channels: u8,     // 1 | 2
    pub bitrate: Bitrate,
}

pub struct Mp3Encoder { /* estado interno, sin Box<dyn>, sin Vec que crezca */ }

impl Mp3Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self, Error>;

    /// Tamaño máximo en bytes que puede producir una llamada a
    /// `encode_into` con `num_samples` muestras interleaved
    /// (incluye lo que haya acumulado internamente).
    pub fn max_output_size(&self, num_samples: usize) -> usize;

    /// Tamaño máximo que puede producir `flush_into`.
    pub fn max_flush_size(&self) -> usize;

    /// Codifica y escribe frames completos en `out`.
    /// Devuelve los bytes escritos. Sin asignaciones de memoria.
    pub fn encode_into(&mut self, samples: &[i16], out: &mut [u8]) -> Result<usize, Error>;

    /// Rellena con silencio el último frame parcial, lo emite y deja
    /// el encoder terminado. Llamadas posteriores a encode → Error::Finished.
    pub fn flush_into(&mut self, out: &mut [u8]) -> Result<usize, Error>;

    /// Delay del encoder en muestras por canal (constante, documentada).
    pub fn delay_samples(&self) -> u32;

    // Conveniencia (usa las anteriores):
    pub fn encode(&mut self, samples: &[i16], out: &mut Vec<u8>) -> Result<(), Error>;
    pub fn flush(&mut self, out: &mut Vec<u8>) -> Result<(), Error>;
}
```

Reglas:

- Si `out` es más pequeño que `max_output_size`, se devuelve `Error::BufferTooSmall` **sin consumir la entrada** y sin cambiar el estado.
- Cota usada: frames emitidos ≤ `ceil((pendientes + num_samples/ch) / 1152)`, y cada frame ocupa como máximo 1441 bytes (320 kbps a 32 kHz con padding).
- Después de `new()`, `encode_into` y `flush_into` **no reservan memoria**.

### Errores

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    InvalidSampleRate,
    InvalidChannels,
    InvalidBitrate,
    InvalidInput,
    BufferTooSmall,
    Finished,
    Internal,
}
```

- Implementar `Display` y `std::error::Error` a mano (sin `thiserror`).
- `Internal` indica un bug. No debe ocurrir nunca; existe para no hacer panic.

## 9. Módulo `pcm-convert` (feature opcional)

Existe porque Android no tiene resampler público y los decoders entregan cualquier tasa o número de canales. iOS **no lo necesita** (AVFoundation convierte), así que el XCFramework se compila sin esta feature.

```rust
#[derive(Clone, Copy)]
pub enum SampleFormat { S16, F32 }

pub struct PcmConverter { /* estado */ }

impl PcmConverter {
    pub fn new(
        in_rate: u32, in_channels: u8, in_format: SampleFormat,
    ) -> Result<Self, Error>;

    pub fn out_rate(&self) -> u32;      // tasa elegida (regla abajo)
    pub fn out_channels(&self) -> u8;   // 1 o 2
    pub fn max_output_samples(&self, in_bytes: usize) -> usize;

    /// `input` son bytes crudos del decoder en `in_format`.
    pub fn process_into(&mut self, input: &[u8], out: &mut [i16]) -> Result<usize, Error>;
    pub fn flush_into(&mut self, out: &mut [i16]) -> Result<usize, Error>;
}
```

### Regla de tasa de salida (ratios enteros siempre que se pueda)

| Entrada | Salida |
|---|---|
| 32 000, 44 100, 48 000 | igual (sin resampling) |
| 8 000, 16 000 | 32 000 |
| 11 025, 22 050, 88 200, 176 400 | 44 100 |
| 12 000, 24 000, 96 000, 192 000 | 48 000 |
| cualquier otra | 48 000 |

### Resampler

- Polifásico con sinc enventanado, kernel fijo pequeño (≈16–32 taps), coeficientes calculados en `new()`.
- Filtro antialiasing correcto al bajar de tasa (ej. 96k → 48k).
- Estado entre bloques (streaming).

### Canales

- 1 canal → mono (no se duplica a stereo).
- 2 canales → stereo.
- Más de 2 → downmix a stereo con orden estándar WAVE (L, R, C, LFE, Ls, Rs, …):
  - `L' = L + 0.7071·C + 0.7071·Ls`
  - `R' = R + 0.7071·C + 0.7071·Rs`
  - LFE se descarta.
  - Resultado escalado para evitar clipping y saturado a i16.

### Formato

- F32 → S16 con saturación (`clamp(-1.0, 1.0) * 32767`).

### Decisiones de implementación

- **Entrada**: 8 000–192 000 Hz y 1–8 canales; fuera de rango → `InvalidSampleRate` / `InvalidChannels`.
- **Bloques que cortan una muestra**: los bytes sobrantes se guardan para la siguiente llamada; la salida es idéntica con cualquier troceo. `flush` descarta una trama incompleta.
- **Tasas arbitrarias**: el kernel se tabula con 128 puntos por muestra de entrada y se interpola linealmente; el instante de cada salida se lleva con aritmética racional exacta (n·M/L). Memoria acotada para cualquier relación.
- **Filtro**: 32 taps (a la tasa más baja), Blackman, corte 0.9·Nyquist. Medido: plano (SNR > 80 dB) hasta ~0.7·Nyquist, > 60 dB de atenuación por encima del Nyquist de salida. Ensanchar a 48 taps llevaría la banda plana a ~0.8.
- **Sin retardo**: filtro simétrico; salida n en el instante n·M/L; tras `flush`, salida = ceil(entrada · L / M) tramas.
- **Downmix**: pesos normalizados por la mayor suma de una salida (5.1: ÷ 2.414), nunca satura; 7 canales: centro trasero 0.5 a cada lado; 7.1: laterales y traseros como Ls/Rs.
- **API**: se añade `max_flush_samples()` (y `audio2mp3_converter_max_flush_samples`) para dimensionar el buffer de `flush`.

## 10. Formato del frame (Fase 1)

| Campo | Valor |
|---|---|
| sync | 0x7FF |
| version | MPEG-1 |
| layer | III |
| protection | 1 (sin CRC) |
| bitrate_index | según config |
| sampling_frequency | según config |
| padding | algoritmo de acumulación fraccional (necesario en 44.1 kHz) |
| private | 0 |
| mode | 11 mono / 00 stereo (Fase 2: 01 joint stereo) |
| mode_extension | 00 |
| copyright | 0 |
| original | 1 |
| emphasis | 00 |

- Longitud del frame: `floor(144 · bitrate / sample_rate) + padding`.
- Side info: 17 bytes en mono, 32 en stereo.
- En Fase 1, el main data de cada frame debe caber en `frame_len − 4 − side_info_len`. El sobrante se rellena con bits de stuffing (`part2_3_length` lo refleja).

## 11. Pipeline interno

```text
PCM S16 ─► buffer 1152/canal ─► Filterbank (32 subbandas × 36)
       ─► MDCT (18 → 576 líneas por granule) ─► reducción de aliasing
       ─► [F3: psicoacústica + decisión de bloque]
       ─► [F2: M/S]
       ─► cuantización (inner loop; F2: outer loop)
       ─► Huffman ─► [F2: reservoir] ─► empaquetado del frame
```

Requisitos de implementación:

- Buffers de tamaño fijo dentro de `Mp3Encoder`.
- Tablas `const` / `static` (no se generan en runtime salvo que reduzca el binario y se mida).
- Cálculo en `f32`. Sin SIMD explícito en Fase 1; se puede evaluar después.
- Sin `unsafe` fuera de `ffi.rs`.
- **Ningún panic alcanzable** en rutas públicas: nada de `unwrap`/`expect` y nada de indexación que pueda salirse de rango sin estar probado. Se compila con `panic = "abort"`, así que un panic tumbaría la app host.

## 12. FFI C

Archivo `include/audio2mp3.h`, escrito a mano y verificado en CI contra `ffi.rs`.

```c
#include <stdint.h>
#include <stddef.h>

#define AUDIO2MP3_OK                   0
#define AUDIO2MP3_ERR_INVALID_SAMPLE_RATE 1
#define AUDIO2MP3_ERR_INVALID_CHANNELS    2
#define AUDIO2MP3_ERR_INVALID_BITRATE     3
#define AUDIO2MP3_ERR_INVALID_INPUT       4
#define AUDIO2MP3_ERR_BUFFER_TOO_SMALL    5
#define AUDIO2MP3_ERR_FINISHED            6
#define AUDIO2MP3_ERR_NULL_POINTER        7
#define AUDIO2MP3_ERR_INTERNAL            99

typedef struct Audio2Mp3Encoder Audio2Mp3Encoder;

uint32_t audio2mp3_version(void); /* (major<<16)|(minor<<8)|patch */

int32_t audio2mp3_encoder_create(uint32_t sample_rate, uint8_t channels,
                                 uint16_t bitrate_kbps,
                                 Audio2Mp3Encoder **out_encoder);

size_t  audio2mp3_encoder_max_output_size(const Audio2Mp3Encoder *enc,
                                          size_t num_samples);
size_t  audio2mp3_encoder_max_flush_size(const Audio2Mp3Encoder *enc);

int32_t audio2mp3_encoder_encode(Audio2Mp3Encoder *enc,
                                 const int16_t *pcm, size_t num_samples,
                                 uint8_t *out, size_t out_capacity,
                                 size_t *out_written);

int32_t audio2mp3_encoder_flush(Audio2Mp3Encoder *enc,
                                uint8_t *out, size_t out_capacity,
                                size_t *out_written);

uint32_t audio2mp3_encoder_delay_samples(const Audio2Mp3Encoder *enc); /* añadida en la implementación */

void    audio2mp3_encoder_destroy(Audio2Mp3Encoder *enc); /* acepta NULL */

/* Solo con feature pcm-convert */
typedef struct Audio2Mp3Converter Audio2Mp3Converter;
int32_t audio2mp3_converter_create(uint32_t in_rate, uint8_t in_channels,
                                   uint8_t in_format /* 0=S16, 1=F32 */,
                                   Audio2Mp3Converter **out_conv);
uint32_t audio2mp3_converter_out_rate(const Audio2Mp3Converter *c);
uint8_t  audio2mp3_converter_out_channels(const Audio2Mp3Converter *c);
size_t  audio2mp3_converter_max_output_samples(const Audio2Mp3Converter *c,
                                               size_t in_bytes);
int32_t audio2mp3_converter_process(Audio2Mp3Converter *c,
                                    const uint8_t *in, size_t in_bytes,
                                    int16_t *out, size_t out_capacity,
                                    size_t *out_written);
int32_t audio2mp3_converter_flush(Audio2Mp3Converter *c,
                                  int16_t *out, size_t out_capacity,
                                  size_t *out_written);
void    audio2mp3_converter_destroy(Audio2Mp3Converter *c);
```

### Propiedad de la memoria

| Recurso | Reserva | Escribe | Libera |
|---|---|---|---|
| Handle (`Encoder`, `Converter`) | Rust (`create`) | Rust | Rust (`destroy`) |
| Buffer PCM de entrada | Llamador | Llamador | Llamador |
| Buffer MP3 / PCM de salida | Llamador | Rust | Llamador |

- Ninguna memoria reservada por Rust cruza la frontera salvo el handle opaco.
- Un handle no es thread-safe: un hilo por handle a la vez. Distintos handles sí pueden usarse en paralelo.
- Todas las funciones comprueban punteros nulos y devuelven `ERR_NULL_POINTER`.

### JNI (Android)

Crate aparte en el workspace: `audio2mp3-jni`. Exporta las funciones `Java_…` que llaman a la API C/Rust usando **`DirectByteBuffer`** (sin copias de arrays). Las declaraciones JNI se escriben a mano o con `jni-sys` (solo tipos, sin código de runtime). Así el `.so` final es uno solo y el consumidor **no compila C ni Rust**.

Implementado con declaraciones a mano (sin dependencias; `jni-sys` 0.4 arrastra proc-macros). Interfaz Kotlin: `object NativeEncoder` con métodos `@JvmStatic external` (`validate`, `create`, `maxOutputSize`, `maxFlushSize`, `delaySamples`, `encode`, `flush`, `destroy`, `version`); la firma exacta está en `crates/audio2mp3-jni/src/lib.rs`. Errores: `create` devuelve 0 si la configuración no es válida (`validate` da el código); `encode`/`flush` devuelven bytes escritos o `−código`.

## 13. Estructura del repositorio

```text
audio2mp3-core/
├── Cargo.toml                 # workspace
├── LICENSE-MIT
├── LICENSE-APACHE
├── README.md
├── CHANGELOG.md
├── include/
│   └── audio2mp3.h
├── crates/
│   ├── audio2mp3-core/
│   │   ├── Cargo.toml         # features: ffi, pcm-convert
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── encoder.rs
│   │       ├── config.rs      # Bitrate, EncoderConfig
│   │       ├── error.rs
│   │       ├── bitstream.rs   # escritor de bits
│   │       ├── frame.rs       # header + side info
│   │       ├── filterbank.rs
│   │       ├── mdct.rs
│   │       ├── quantize.rs
│   │       ├── huffman.rs
│   │       ├── tables/        # tablas del estándar
│   │       ├── reservoir.rs   # F2
│   │       ├── stereo.rs      # F2
│   │       ├── psy.rs         # F3
│   │       ├── convert/       # feature pcm-convert
│   │       │   ├── mod.rs
│   │       │   ├── resample.rs
│   │       │   └── downmix.rs
│   │       └── ffi.rs         # feature ffi
│   └── audio2mp3-jni/
│       ├── Cargo.toml         # crate-type = ["cdylib"]
│       └── src/lib.rs
├── tests/
├── benches/
├── fuzz/
├── examples/
│   └── encode.rs              # WAV → MP3 para pruebas manuales
└── scripts/
    ├── build-android.sh       # cargo-ndk, todas las ABIs
    └── build-ios.sh           # staticlib → XCFramework
```

## 14. Compilación y artefactos

### Perfil

```toml
[profile.release]
opt-level = "s"     # comparar con "z" y "3"; documentar el elegido
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

### Android

- Targets: `aarch64-linux-android` (obligatorio), `armv7-linux-androideabi`, `x86_64-linux-android` (emulador).
- `crate-type = ["cdylib"]` → `libaudio2mp3.so`.
- **Alineación de páginas de 16 KB** (requisito de Google Play): NDK r28+ o `-C link-arg=-Wl,-z,max-page-size=16384`. Verificar en CI.
- Features: `ffi`, `pcm-convert`.

### iOS

- Targets: `aarch64-apple-ios` (device), `aarch64-apple-ios-sim`, `x86_64-apple-ios` (simulador Intel).
- `crate-type = ["staticlib"]` → `Audio2Mp3Core.xcframework` (device y simulador separados).
- Features: `ffi` (sin `pcm-convert`).
- Incluye `audio2mp3.h` + `module.modulemap` para importarlo desde Swift.

### Releases

Cada tag `vX.Y.Z` publica en GitHub Releases:

- `audio2mp3-android-vX.Y.Z.zip` (jniLibs por ABI)
- `Audio2Mp3Core-vX.Y.Z.xcframework.zip`
- `SHA256SUMS`

## 15. Tests

### Unitarios

- Bitstream writer.
- Header: sync, versión, layer, bitrate, sample rate, modo, padding y longitud de frame para todas las combinaciones.
- Tablas: checksums contra los valores del estándar.
- Filterbank/MDCT: comparación con una implementación de referencia lenta (f64) dentro de los tests.
- Huffman: cada tabla codifica y decodifica correctamente.
- Resampler: respuesta en frecuencia (atenuación por encima de Nyquist), longitud de salida y continuidad entre bloques.
- Downmix y conversión f32→s16 con saturación.

### Ida y vuelta (con decoder en dev-dependencies)

- Señales: silencio, seno 1 kHz, barrido de frecuencias, ruido blanco, amplitud máxima (±32767), cambios bruscos.
- Mono y stereo; 32/44.1/48 kHz; todos los bitrates.
- Validar:
  - el decoder acepta todos los frames sin errores;
  - longitud decodificada = entrada + delay + padding del último frame;
  - el seno conserva el pico espectral en su frecuencia;
  - SNR por encima de un umbral fijado tras la primera baseline (regresión: no puede bajar).
- Streaming: el mismo audio en chunks de 1, 7, 1152, 4096 y 100 000 muestras produce **bytes idénticos**.

### Compatibilidad (CI)

- `ffprobe` sin errores ni warnings.
- Decodificación con FFmpeg sin errores.
- Prueba manual por versión: VLC, reproductor de Android, Apple Music/QuickTime.

### Robustez

- `cargo-fuzz` sobre la API FFI y el converter (entradas aleatorias, tamaños extremos, buffers pequeños).
- `cargo miri test` sobre `ffi.rs`.
- `cargo clippy -- -D warnings`.

## 16. Benchmarks y objetivos

Medir con 10 s, 1 min, 10 min y 30 min de audio:

- tiempo de encoding y real-time factor (RTF);
- memoria pico;
- tamaño del MP3.

| Métrica | Objetivo Fase 1 |
|---|---|
| `libaudio2mp3.so` arm64-v8a | ≤ 200 KB |
| Contribución del XCFramework al IPA (arm64) | ≤ 200 KB |
| Memoria del encoder (`size_of` + heap) | ≤ 256 KB |
| Memoria pico durante 30 min | constante, sin crecer |
| RTF en Android gama media arm64 | ≥ 20× |

Los objetivos se revisan con las mediciones reales; si no se cumplen, se documenta el motivo.

## 17. Criterios de aceptación — Fase 1

- [ ] Compila sin warnings en Linux, macOS, Windows, Android (3 ABIs) e iOS (device + simulador).
- [ ] Cero dependencias de runtime (`cargo tree -e normal` vacío).
- [ ] Todos los tests de §15 en verde en CI.
- [ ] Los MP3 generados pasan `ffprobe` y se reproducen en VLC, Android e iOS.
- [ ] Salida idéntica con cualquier tamaño de chunk.
- [ ] Memoria constante con archivos de 30 min.
- [ ] Objetivos de tamaño de §16 medidos y documentados.
- [ ] Artefactos Android (alineados a 16 KB) e iOS publicados en un release.
- [ ] README con ejemplo Rust y C.

## 18. Regla principal

El proyecto hace **una sola cosa**: PCM → MP3 (más la conversión de PCM mínima que necesita Android).

Cualquier funcionalidad nueva o dependencia de runtime requiere justificarla y medir su impacto en el binario antes de añadirla.
