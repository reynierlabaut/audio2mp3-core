//! Puente JNI de audio2mp3-core para Android (`libaudio2mp3.so`).
//!
//! Declaración Kotlin que corresponde a estas funciones (expo-audio2mp3):
//!
//! ```kotlin
//! package expo.modules.audio2mp3
//!
//! internal object NativeEncoder {
//!     init { System.loadLibrary("audio2mp3") }
//!
//!     @JvmStatic external fun version(): Int
//!     /** 0 si la configuración es válida; si no, código AUDIO2MP3_ERR_*. */
//!     @JvmStatic external fun validate(sampleRate: Int, channels: Int, bitrateKbps: Int): Int
//!     /** Handle opaco; 0 si la configuración no es válida (ver validate). */
//!     @JvmStatic external fun create(sampleRate: Int, channels: Int, bitrateKbps: Int): Long
//!     @JvmStatic external fun maxOutputSize(handle: Long, numSamples: Int): Int
//!     @JvmStatic external fun maxFlushSize(handle: Long): Int
//!     @JvmStatic external fun delaySamples(handle: Long): Int
//!     /** Bytes escritos (≥ 0) o −código de error. */
//!     @JvmStatic external fun encode(handle: Long, pcm: ByteBuffer, numSamples: Int, out: ByteBuffer): Int
//!     /** Bytes escritos (≥ 0) o −código de error. */
//!     @JvmStatic external fun flush(handle: Long, out: ByteBuffer): Int
//!     @JvmStatic external fun destroy(handle: Long)
//! }
//! ```
//!
//! - Los `ByteBuffer` deben ser directos (`ByteBuffer.allocateDirect`); se usan
//!   desde su dirección base, sin tener en cuenta `position`/`limit`.
//! - `pcm` contiene S16 intercalado en orden nativo
//!   (`.order(ByteOrder.nativeOrder())`); `pcm` y `out` no pueden ser el mismo buffer.
//! - Un handle no es thread-safe: usarlo desde un solo hilo a la vez.
//! - Los códigos de error son los de `include/audio2mp3.h`.

#![deny(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;

use audio2mp3_core::{Bitrate, EncoderConfig, Error, Mp3Encoder};

// Códigos de include/audio2mp3.h.
const ERR_INVALID_SAMPLE_RATE: i32 = 1;
const ERR_INVALID_CHANNELS: i32 = 2;
const ERR_INVALID_BITRATE: i32 = 3;
const ERR_INVALID_INPUT: i32 = 4;
const ERR_BUFFER_TOO_SMALL: i32 = 5;
const ERR_FINISHED: i32 = 6;
const ERR_NULL_POINTER: i32 = 7;
const ERR_INTERNAL: i32 = 99;

// ------------------------------------------------------------------ JNI mínimo

/// `JNIEnv` de C: puntero a la tabla de funciones `JNINativeInterface`.
#[repr(C)]
pub struct JNIEnv {
    functions: *const *const c_void,
}

type JObject = *mut c_void;
type JClass = *mut c_void;

/// Índices en `JNINativeInterface` (especificación JNI; comprobados contra jni-sys).
const GET_DIRECT_BUFFER_ADDRESS: usize = 230;
const GET_DIRECT_BUFFER_CAPACITY: usize = 231;

type GetDirectBufferAddress = unsafe extern "system" fn(*mut JNIEnv, JObject) -> *mut c_void;
type GetDirectBufferCapacity = unsafe extern "system" fn(*mut JNIEnv, JObject) -> i64;

/// Zona de memoria de un `ByteBuffer` directo.
struct Region {
    ptr: *mut u8,
    len: usize,
}

impl Region {
    fn overlaps(&self, other: &Region) -> bool {
        let (a, b) = (self.ptr as usize, other.ptr as usize);
        a < b + other.len && b < a + self.len
    }
}

/// # Safety
/// `env` debe ser el `JNIEnv*` de la llamada en curso y `buf` una referencia local válida.
unsafe fn direct_buffer(env: *mut JNIEnv, buf: JObject) -> Result<Region, i32> {
    if env.is_null() || buf.is_null() {
        return Err(ERR_NULL_POINTER);
    }
    // SAFETY: `env` es un JNIEnv válido: su tabla tiene al menos 232 entradas.
    let (address, capacity) = unsafe {
        let table = (*env).functions;
        if table.is_null() {
            return Err(ERR_NULL_POINTER);
        }
        let address = *table.add(GET_DIRECT_BUFFER_ADDRESS);
        let capacity = *table.add(GET_DIRECT_BUFFER_CAPACITY);
        if address.is_null() || capacity.is_null() {
            return Err(ERR_INTERNAL);
        }
        (
            core::mem::transmute::<*const c_void, GetDirectBufferAddress>(address),
            core::mem::transmute::<*const c_void, GetDirectBufferCapacity>(capacity),
        )
    };
    // SAFETY: funciones de la JVM llamadas con el env y el objeto de esta llamada.
    let (ptr, len) = unsafe { (address(env, buf), capacity(env, buf)) };
    // Buffers no directos: dirección NULL y capacidad −1.
    if ptr.is_null() || len < 0 {
        return Err(ERR_NULL_POINTER);
    }
    let len = usize::try_from(len).map_err(|_| ERR_INVALID_INPUT)?;
    Ok(Region {
        ptr: ptr.cast(),
        len,
    })
}

// ------------------------------------------------------------------ Lógica

fn error_code(error: Error) -> i32 {
    match error {
        Error::InvalidSampleRate => ERR_INVALID_SAMPLE_RATE,
        Error::InvalidChannels => ERR_INVALID_CHANNELS,
        Error::InvalidBitrate => ERR_INVALID_BITRATE,
        Error::InvalidInput => ERR_INVALID_INPUT,
        Error::BufferTooSmall => ERR_BUFFER_TOO_SMALL,
        Error::Finished => ERR_FINISHED,
        _ => ERR_INTERNAL,
    }
}

fn new_encoder(sample_rate: i32, channels: i32, bitrate_kbps: i32) -> Result<Mp3Encoder, i32> {
    let sample_rate = u32::try_from(sample_rate).map_err(|_| ERR_INVALID_SAMPLE_RATE)?;
    let channels = u8::try_from(channels).map_err(|_| ERR_INVALID_CHANNELS)?;
    let kbps = u16::try_from(bitrate_kbps).map_err(|_| ERR_INVALID_BITRATE)?;
    let bitrate = Bitrate::from_kbps(kbps).map_err(error_code)?;
    Mp3Encoder::new(EncoderConfig {
        sample_rate,
        channels,
        bitrate,
    })
    .map_err(error_code)
}

/// # Safety
/// `handle` debe ser 0 o un valor devuelto por `create` y aún no destruido.
unsafe fn encoder<'a>(handle: i64) -> Result<&'a mut Mp3Encoder, i32> {
    // SAFETY: 0 → NULL; si no, puntero de Box::into_raw vivo según el contrato.
    unsafe { (handle as usize as *mut Mp3Encoder).as_mut() }.ok_or(ERR_NULL_POINTER)
}

fn to_jint(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn to_result(r: Result<usize, i32>) -> i32 {
    match r {
        Ok(n) => i32::try_from(n).unwrap_or(-ERR_INTERNAL),
        Err(code) => -code,
    }
}

// ------------------------------------------------------------------ Exportadas

#[no_mangle]
pub extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_version(
    _env: *mut JNIEnv,
    _class: JClass,
) -> i32 {
    let v = |s: &str| s.parse::<i32>().unwrap_or(0);
    v(env!("CARGO_PKG_VERSION_MAJOR")) << 16
        | v(env!("CARGO_PKG_VERSION_MINOR")) << 8
        | v(env!("CARGO_PKG_VERSION_PATCH"))
}

#[no_mangle]
pub extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_validate(
    _env: *mut JNIEnv,
    _class: JClass,
    sample_rate: i32,
    channels: i32,
    bitrate_kbps: i32,
) -> i32 {
    new_encoder(sample_rate, channels, bitrate_kbps).map_or_else(|code| code, |_| 0)
}

#[no_mangle]
pub extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_create(
    _env: *mut JNIEnv,
    _class: JClass,
    sample_rate: i32,
    channels: i32,
    bitrate_kbps: i32,
) -> i64 {
    match new_encoder(sample_rate, channels, bitrate_kbps) {
        Ok(enc) => Box::into_raw(Box::new(enc)) as usize as i64,
        Err(_) => 0,
    }
}

/// # Safety
/// `handle`: 0 o handle vivo de `create`.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_maxOutputSize(
    _env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
    num_samples: i32,
) -> i32 {
    // SAFETY: según el contrato.
    let Ok(enc) = (unsafe { encoder(handle) }) else {
        return 0;
    };
    to_jint(enc.max_output_size(usize::try_from(num_samples).unwrap_or(0)))
}

/// # Safety
/// `handle`: 0 o handle vivo de `create`.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_maxFlushSize(
    _env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
) -> i32 {
    // SAFETY: según el contrato.
    unsafe { encoder(handle) }.map_or(0, |enc| to_jint(enc.max_flush_size()))
}

/// # Safety
/// `handle`: 0 o handle vivo de `create`.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_delaySamples(
    _env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
) -> i32 {
    // SAFETY: según el contrato.
    unsafe { encoder(handle) }.map_or(0, |enc| to_jint(enc.delay_samples() as usize))
}

/// # Safety
/// Llamada desde la JVM: `env` válido, `handle` 0 o vivo y sin uso concurrente,
/// `pcm` y `out` ByteBuffers directos sin solaparse.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_encode(
    env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
    pcm: JObject,
    num_samples: i32,
    out: JObject,
) -> i32 {
    let result = (|| {
        // SAFETY: según el contrato.
        let enc = unsafe { encoder(handle) }?;
        // SAFETY: env y objetos de esta llamada.
        let (input, output) = unsafe { (direct_buffer(env, pcm)?, direct_buffer(env, out)?) };
        let num_samples = usize::try_from(num_samples).map_err(|_| ERR_INVALID_INPUT)?;
        let bytes = num_samples.checked_mul(2).ok_or(ERR_INVALID_INPUT)?;
        if bytes > input.len || !input.ptr.cast::<i16>().is_aligned() || input.overlaps(&output) {
            return Err(ERR_INVALID_INPUT);
        }
        // SAFETY: la JVM garantiza `len` bytes válidos en cada buffer directo
        // mientras dura la llamada; tamaños y alineación comprobados; sin solape.
        let (samples, out) = unsafe {
            (
                core::slice::from_raw_parts(input.ptr.cast::<i16>(), num_samples),
                core::slice::from_raw_parts_mut(output.ptr, output.len),
            )
        };
        enc.encode_into(samples, out).map_err(error_code)
    })();
    to_result(result)
}

/// # Safety
/// Llamada desde la JVM: `env` válido, `handle` 0 o vivo y sin uso concurrente,
/// `out` ByteBuffer directo.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_flush(
    env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
    out: JObject,
) -> i32 {
    let result = (|| {
        // SAFETY: según el contrato.
        let enc = unsafe { encoder(handle) }?;
        // SAFETY: env y objeto de esta llamada.
        let output = unsafe { direct_buffer(env, out) }?;
        // SAFETY: `len` bytes válidos del buffer directo durante la llamada.
        let out = unsafe { core::slice::from_raw_parts_mut(output.ptr, output.len) };
        enc.flush_into(out).map_err(error_code)
    })();
    to_result(result)
}

/// # Safety
/// `handle`: 0 o handle vivo de `create` que no se vuelva a usar.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_destroy(
    _env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
) {
    let ptr = handle as usize as *mut Mp3Encoder;
    if !ptr.is_null() {
        // SAFETY: creado con Box::into_raw en `create` y no liberado antes.
        drop(unsafe { Box::from_raw(ptr) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ptr;

    /// "ByteBuffer directo" de prueba: el jobject apunta a esta estructura.
    struct FakeBuffer {
        data: Vec<u8>,
        direct: bool,
    }

    unsafe extern "system" fn fake_address(_env: *mut JNIEnv, obj: JObject) -> *mut c_void {
        // SAFETY: en los tests el jobject siempre es un `FakeBuffer`.
        let buf = unsafe { &mut *obj.cast::<FakeBuffer>() };
        if buf.direct {
            buf.data.as_mut_ptr().cast()
        } else {
            ptr::null_mut()
        }
    }

    unsafe extern "system" fn fake_capacity(_env: *mut JNIEnv, obj: JObject) -> i64 {
        // SAFETY: en los tests el jobject siempre es un `FakeBuffer`.
        let buf = unsafe { &*obj.cast::<FakeBuffer>() };
        if buf.direct {
            buf.data.len() as i64
        } else {
            -1
        }
    }

    struct FakeEnv {
        _table: Vec<*const c_void>,
        env: JNIEnv,
    }

    fn fake_env() -> FakeEnv {
        let mut table = vec![ptr::null(); 236];
        table[GET_DIRECT_BUFFER_ADDRESS] = fake_address as *const c_void;
        table[GET_DIRECT_BUFFER_CAPACITY] = fake_capacity as *const c_void;
        let env = JNIEnv {
            functions: table.as_ptr(),
        };
        FakeEnv { _table: table, env }
    }

    fn obj(buf: &mut FakeBuffer) -> JObject {
        (buf as *mut FakeBuffer).cast()
    }

    const CLASS: JClass = ptr::null_mut();

    #[test]
    fn validate_and_create() {
        let env = ptr::null_mut();
        assert_eq!(
            Java_expo_modules_audio2mp3_NativeEncoder_validate(env, CLASS, 44_100, 2, 192),
            0
        );
        assert_eq!(
            Java_expo_modules_audio2mp3_NativeEncoder_validate(env, CLASS, 22_050, 2, 192),
            ERR_INVALID_SAMPLE_RATE
        );
        assert_eq!(
            Java_expo_modules_audio2mp3_NativeEncoder_validate(env, CLASS, 44_100, -1, 192),
            ERR_INVALID_CHANNELS
        );
        assert_eq!(
            Java_expo_modules_audio2mp3_NativeEncoder_validate(env, CLASS, 44_100, 2, 100),
            ERR_INVALID_BITRATE
        );
        assert_eq!(
            Java_expo_modules_audio2mp3_NativeEncoder_create(env, CLASS, 44_100, 2, 100),
            0
        );
        assert_eq!(
            Java_expo_modules_audio2mp3_NativeEncoder_version(env, CLASS),
            1 << 8
        );
    }

    #[test]
    fn encode_and_flush_through_jni_match_rust_api() {
        let samples: Vec<i16> = (0..2 * 6_000)
            .map(|i| ((i * 53 % 4000) as i16 - 2000) * 4)
            .collect();
        let mut rust = Mp3Encoder::new(EncoderConfig {
            sample_rate: 48_000,
            channels: 2,
            bitrate: Bitrate::Kbps128,
        })
        .unwrap();
        let mut expected = Vec::new();
        rust.encode(&samples, &mut expected).unwrap();
        rust.flush(&mut expected).unwrap();

        let mut fake = fake_env();
        let env: *mut JNIEnv = &mut fake.env;
        let mut pcm = FakeBuffer {
            data: samples.iter().flat_map(|s| s.to_ne_bytes()).collect(),
            direct: true,
        };
        let mut got = Vec::new();
        // SAFETY: env falso válido, handle vivo, buffers falsos válidos y distintos.
        unsafe {
            let h = Java_expo_modules_audio2mp3_NativeEncoder_create(env, CLASS, 48_000, 2, 128);
            assert_ne!(h, 0);
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_delaySamples(env, CLASS, h),
                1057
            );
            let n = samples.len() as i32;
            let size = Java_expo_modules_audio2mp3_NativeEncoder_maxOutputSize(env, CLASS, h, n);
            let mut small = FakeBuffer {
                data: vec![0; 10],
                direct: true,
            };
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_encode(
                    env,
                    CLASS,
                    h,
                    obj(&mut pcm),
                    n,
                    obj(&mut small)
                ),
                -ERR_BUFFER_TOO_SMALL
            );
            let mut out = FakeBuffer {
                data: vec![0; size as usize],
                direct: true,
            };
            let written = Java_expo_modules_audio2mp3_NativeEncoder_encode(
                env,
                CLASS,
                h,
                obj(&mut pcm),
                n,
                obj(&mut out),
            );
            assert!(written > 0);
            got.extend_from_slice(&out.data[..written as usize]);

            let flush_size = Java_expo_modules_audio2mp3_NativeEncoder_maxFlushSize(env, CLASS, h);
            let mut tail = FakeBuffer {
                data: vec![0; flush_size as usize],
                direct: true,
            };
            let written =
                Java_expo_modules_audio2mp3_NativeEncoder_flush(env, CLASS, h, obj(&mut tail));
            assert!(written >= 0);
            got.extend_from_slice(&tail.data[..written as usize]);
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_flush(env, CLASS, h, obj(&mut tail)),
                -ERR_FINISHED
            );
            Java_expo_modules_audio2mp3_NativeEncoder_destroy(env, CLASS, h);
        }
        assert_eq!(got, expected);
    }

    #[test]
    fn rejects_bad_buffers_and_handles() {
        let mut fake = fake_env();
        let env: *mut JNIEnv = &mut fake.env;
        let mut pcm = FakeBuffer {
            data: vec![0; 8],
            direct: true,
        };
        let mut out = FakeBuffer {
            data: vec![0; 4096],
            direct: true,
        };
        let mut heap = FakeBuffer {
            data: vec![0; 4096],
            direct: false,
        };
        // SAFETY: env falso válido; handles 0 o vivos; buffers falsos válidos.
        unsafe {
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_encode(
                    env,
                    CLASS,
                    0,
                    obj(&mut pcm),
                    2,
                    obj(&mut out)
                ),
                -ERR_NULL_POINTER
            );
            let h = Java_expo_modules_audio2mp3_NativeEncoder_create(env, CLASS, 32_000, 1, 64);
            // Más muestras de las que caben en el buffer de entrada.
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_encode(
                    env,
                    CLASS,
                    h,
                    obj(&mut pcm),
                    5,
                    obj(&mut out)
                ),
                -ERR_INVALID_INPUT
            );
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_encode(
                    env,
                    CLASS,
                    h,
                    obj(&mut pcm),
                    -1,
                    obj(&mut out)
                ),
                -ERR_INVALID_INPUT
            );
            // ByteBuffer no directo.
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_encode(
                    env,
                    CLASS,
                    h,
                    obj(&mut pcm),
                    2,
                    obj(&mut heap)
                ),
                -ERR_NULL_POINTER
            );
            // Mismo buffer como entrada y salida.
            let same = obj(&mut out);
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_encode(env, CLASS, h, same, 2, same),
                -ERR_INVALID_INPUT
            );
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_encode(
                    env,
                    CLASS,
                    h,
                    obj(&mut pcm),
                    4,
                    obj(&mut out)
                ),
                0
            );
            Java_expo_modules_audio2mp3_NativeEncoder_destroy(env, CLASS, h);
            Java_expo_modules_audio2mp3_NativeEncoder_destroy(env, CLASS, 0);
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_maxFlushSize(env, CLASS, 0),
                0
            );
        }
    }
}
