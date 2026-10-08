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
//!
//!     // Converter: PCM del decoder (inFormat 0 = S16, 1 = F32) → S16 para el encoder.
//!     @JvmStatic external fun converterValidate(inRate: Int, inChannels: Int, inFormat: Int): Int
//!     @JvmStatic external fun converterCreate(inRate: Int, inChannels: Int, inFormat: Int): Long
//!     @JvmStatic external fun converterOutRate(handle: Long): Int
//!     @JvmStatic external fun converterOutChannels(handle: Long): Int
//!     /** Máximo de muestras S16 que escribe converterProcess con inBytes bytes. */
//!     @JvmStatic external fun converterMaxOutputSamples(handle: Long, inBytes: Int): Int
//!     @JvmStatic external fun converterMaxFlushSamples(handle: Long): Int
//!     /** Lee inBytes desde inOffset (BufferInfo.offset de MediaCodec). Muestras escritas o −código. */
//!     @JvmStatic external fun converterProcess(handle: Long, input: ByteBuffer, inOffset: Int, inBytes: Int, out: ByteBuffer): Int
//!     /** Muestras escritas o −código. */
//!     @JvmStatic external fun converterFlush(handle: Long, out: ByteBuffer): Int
//!     @JvmStatic external fun converterDestroy(handle: Long)
//! }
//! ```
//!
//! - Los `ByteBuffer` deben ser directos (`ByteBuffer.allocateDirect` o los de
//!   `MediaCodec`); se usan desde su dirección base, sin tener en cuenta
//!   `position`/`limit` (salvo `inOffset` en `converterProcess`).
//! - El PCM (entrada del encoder, salida del converter) es S16 intercalado en
//!   orden nativo (`.order(ByteOrder.nativeOrder())`). Entrada y salida no pueden
//!   ser el mismo buffer.
//! - Los handles del encoder y del converter no son intercambiables ni
//!   thread-safe: cada uno desde un solo hilo a la vez.
//! - Los códigos de error son los de `include/audio2mp3.h`.

#![deny(unsafe_op_in_unsafe_fn)]

use core::ffi::c_void;

use audio2mp3_core::{Bitrate, EncoderConfig, Error, Mp3Encoder, PcmConverter, SampleFormat};

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

// ------------------------------------------------------------------ Converter

fn new_converter(in_rate: i32, in_channels: i32, in_format: i32) -> Result<PcmConverter, i32> {
    let format = match in_format {
        0 => SampleFormat::S16,
        1 => SampleFormat::F32,
        _ => return Err(ERR_INVALID_INPUT),
    };
    let in_rate = u32::try_from(in_rate).map_err(|_| ERR_INVALID_SAMPLE_RATE)?;
    let in_channels = u8::try_from(in_channels).map_err(|_| ERR_INVALID_CHANNELS)?;
    PcmConverter::new(in_rate, in_channels, format).map_err(error_code)
}

/// # Safety
/// `handle` debe ser 0 o un valor devuelto por `converterCreate` y aún no destruido.
unsafe fn converter<'a>(handle: i64) -> Result<&'a mut PcmConverter, i32> {
    // SAFETY: 0 → NULL; si no, puntero de Box::into_raw vivo según el contrato.
    unsafe { (handle as usize as *mut PcmConverter).as_mut() }.ok_or(ERR_NULL_POINTER)
}

/// Vista de un buffer directo como muestras S16.
///
/// # Safety
/// `region` debe venir de `direct_buffer` en la llamada en curso y no
/// solaparse con otro slice vivo.
unsafe fn i16_slice<'a>(region: &Region) -> Result<&'a mut [i16], i32> {
    let ptr = region.ptr.cast::<i16>();
    if !ptr.is_aligned() {
        return Err(ERR_INVALID_INPUT);
    }
    // SAFETY: memoria válida de la JVM durante la llamada, alineada, `len / 2` muestras.
    Ok(unsafe { core::slice::from_raw_parts_mut(ptr, region.len / 2) })
}

#[no_mangle]
pub extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_converterValidate(
    _env: *mut JNIEnv,
    _class: JClass,
    in_rate: i32,
    in_channels: i32,
    in_format: i32,
) -> i32 {
    new_converter(in_rate, in_channels, in_format).map_or_else(|code| code, |_| 0)
}

#[no_mangle]
pub extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_converterCreate(
    _env: *mut JNIEnv,
    _class: JClass,
    in_rate: i32,
    in_channels: i32,
    in_format: i32,
) -> i64 {
    match new_converter(in_rate, in_channels, in_format) {
        Ok(conv) => Box::into_raw(Box::new(conv)) as usize as i64,
        Err(_) => 0,
    }
}

/// # Safety
/// `handle`: 0 o handle vivo de `converterCreate`.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_converterOutRate(
    _env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
) -> i32 {
    // SAFETY: según el contrato.
    unsafe { converter(handle) }.map_or(0, |c| i32::try_from(c.out_rate()).unwrap_or(0))
}

/// # Safety
/// `handle`: 0 o handle vivo de `converterCreate`.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_converterOutChannels(
    _env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
) -> i32 {
    // SAFETY: según el contrato.
    unsafe { converter(handle) }.map_or(0, |c| i32::from(c.out_channels()))
}

/// # Safety
/// `handle`: 0 o handle vivo de `converterCreate`.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_converterMaxOutputSamples(
    _env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
    in_bytes: i32,
) -> i32 {
    // SAFETY: según el contrato.
    let Ok(c) = (unsafe { converter(handle) }) else {
        return 0;
    };
    to_jint(c.max_output_samples(usize::try_from(in_bytes).unwrap_or(0)))
}

/// # Safety
/// `handle`: 0 o handle vivo de `converterCreate`.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_converterMaxFlushSamples(
    _env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
) -> i32 {
    // SAFETY: según el contrato.
    unsafe { converter(handle) }.map_or(0, |c| to_jint(c.max_flush_samples()))
}

/// # Safety
/// Llamada desde la JVM: `env` válido, `handle` 0 o vivo y sin uso concurrente,
/// `input` y `out` ByteBuffers directos sin solaparse.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_converterProcess(
    env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
    input: JObject,
    in_offset: i32,
    in_bytes: i32,
    out: JObject,
) -> i32 {
    let result = (|| {
        // SAFETY: según el contrato.
        let conv = unsafe { converter(handle) }?;
        // SAFETY: env y objetos de esta llamada.
        let (src, dst) = unsafe { (direct_buffer(env, input)?, direct_buffer(env, out)?) };
        let offset = usize::try_from(in_offset).map_err(|_| ERR_INVALID_INPUT)?;
        let bytes = usize::try_from(in_bytes).map_err(|_| ERR_INVALID_INPUT)?;
        let end = offset.checked_add(bytes).ok_or(ERR_INVALID_INPUT)?;
        if end > src.len || src.overlaps(&dst) {
            return Err(ERR_INVALID_INPUT);
        }
        // SAFETY: rango comprobado dentro del buffer directo; sin solape con `dst`.
        let input = unsafe { core::slice::from_raw_parts(src.ptr.add(offset), bytes) };
        // SAFETY: buffer de esta llamada, sin solape con `input`.
        let out = unsafe { i16_slice(&dst) }?;
        conv.process_into(input, out).map_err(error_code)
    })();
    to_result(result)
}

/// # Safety
/// Llamada desde la JVM: `env` válido, `handle` 0 o vivo y sin uso concurrente,
/// `out` ByteBuffer directo.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_converterFlush(
    env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
    out: JObject,
) -> i32 {
    let result = (|| {
        // SAFETY: según el contrato.
        let conv = unsafe { converter(handle) }?;
        // SAFETY: env y objeto de esta llamada.
        let dst = unsafe { direct_buffer(env, out) }?;
        // SAFETY: buffer de esta llamada, único slice vivo sobre él.
        let out = unsafe { i16_slice(&dst) }?;
        conv.flush_into(out).map_err(error_code)
    })();
    to_result(result)
}

/// # Safety
/// `handle`: 0 o handle vivo de `converterCreate` que no se vuelva a usar.
#[no_mangle]
pub unsafe extern "system" fn Java_expo_modules_audio2mp3_NativeEncoder_converterDestroy(
    _env: *mut JNIEnv,
    _class: JClass,
    handle: i64,
) {
    let ptr = handle as usize as *mut PcmConverter;
    if !ptr.is_null() {
        // SAFETY: creado con Box::into_raw en `converterCreate` y no liberado antes.
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

    #[test]
    fn converter_validate_reports_errors() {
        let env = ptr::null_mut();
        let validate = Java_expo_modules_audio2mp3_NativeEncoder_converterValidate;
        assert_eq!(validate(env, CLASS, 96_000, 6, 1), 0);
        assert_eq!(validate(env, CLASS, 1_000, 2, 0), ERR_INVALID_SAMPLE_RATE);
        assert_eq!(validate(env, CLASS, 44_100, 9, 0), ERR_INVALID_CHANNELS);
        assert_eq!(validate(env, CLASS, 44_100, 2, 2), ERR_INVALID_INPUT);
        assert_eq!(
            Java_expo_modules_audio2mp3_NativeEncoder_converterCreate(env, CLASS, 44_100, 2, 2),
            0
        );
    }

    /// Cadena de Android: buffer de MediaCodec (F32 5.1 a 96 kHz, con offset)
    /// → converter → encoder, igual que con la API Rust.
    #[test]
    fn decoder_to_mp3_pipeline_matches_rust_api() {
        let pcm: Vec<f32> = (0..6 * 20_000)
            .map(|i| ((i / 6) as f32 * 0.013).sin() * 0.6)
            .collect();
        let bytes: Vec<u8> = pcm.iter().flat_map(|v| v.to_ne_bytes()).collect();

        // Referencia con la API Rust.
        let mut conv = PcmConverter::new(96_000, 6, SampleFormat::F32).unwrap();
        let mut s16 = vec![0; conv.max_output_samples(bytes.len())];
        let n = conv.process_into(&bytes, &mut s16).unwrap();
        let mut tail = vec![0; conv.max_flush_samples()];
        let m = conv.flush_into(&mut tail).unwrap();
        s16.truncate(n);
        s16.extend_from_slice(&tail[..m]);
        let mut enc = Mp3Encoder::new(EncoderConfig {
            sample_rate: 48_000,
            channels: 2,
            bitrate: Bitrate::Kbps192,
        })
        .unwrap();
        let mut expected = Vec::new();
        enc.encode(&s16, &mut expected).unwrap();
        enc.flush(&mut expected).unwrap();

        let mut fake = fake_env();
        let env: *mut JNIEnv = &mut fake.env;
        let mut mp3 = Vec::new();
        // SAFETY: env falso válido, handles vivos, buffers falsos válidos y distintos.
        unsafe {
            let c =
                Java_expo_modules_audio2mp3_NativeEncoder_converterCreate(env, CLASS, 96_000, 6, 1);
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_converterOutRate(env, CLASS, c),
                48_000
            );
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_converterOutChannels(env, CLASS, c),
                2
            );
            let e = Java_expo_modules_audio2mp3_NativeEncoder_create(env, CLASS, 48_000, 2, 192);

            let encode = |s16: &mut FakeBuffer, samples: i32, mp3: &mut Vec<u8>| {
                let size =
                    Java_expo_modules_audio2mp3_NativeEncoder_maxOutputSize(env, CLASS, e, samples);
                let mut out = FakeBuffer {
                    data: vec![0; size as usize],
                    direct: true,
                };
                let w = Java_expo_modules_audio2mp3_NativeEncoder_encode(
                    env,
                    CLASS,
                    e,
                    obj(s16),
                    samples,
                    obj(&mut out),
                );
                assert!(w >= 0, "encode: {w}");
                mp3.extend_from_slice(&out.data[..w as usize]);
            };

            // Bloques de 4099 bytes (cortan muestras) con 16 bytes de offset delante.
            for block in bytes.chunks(4099) {
                let mut input = FakeBuffer {
                    data: vec![0xAB; 16],
                    direct: true,
                };
                input.data.extend_from_slice(block);
                let len = block.len() as i32;
                let max = Java_expo_modules_audio2mp3_NativeEncoder_converterMaxOutputSamples(
                    env, CLASS, c, len,
                );
                let mut s16 = FakeBuffer {
                    data: vec![0; 2 * max as usize],
                    direct: true,
                };
                let got = Java_expo_modules_audio2mp3_NativeEncoder_converterProcess(
                    env,
                    CLASS,
                    c,
                    obj(&mut input),
                    16,
                    len,
                    obj(&mut s16),
                );
                assert!(got >= 0, "converterProcess: {got}");
                encode(&mut s16, got, &mut mp3);
            }
            let max =
                Java_expo_modules_audio2mp3_NativeEncoder_converterMaxFlushSamples(env, CLASS, c);
            let mut s16 = FakeBuffer {
                data: vec![0; 2 * max as usize],
                direct: true,
            };
            let got = Java_expo_modules_audio2mp3_NativeEncoder_converterFlush(
                env,
                CLASS,
                c,
                obj(&mut s16),
            );
            assert!(got >= 0);
            encode(&mut s16, got, &mut mp3);
            let size = Java_expo_modules_audio2mp3_NativeEncoder_maxFlushSize(env, CLASS, e);
            let mut out = FakeBuffer {
                data: vec![0; size as usize],
                direct: true,
            };
            let w = Java_expo_modules_audio2mp3_NativeEncoder_flush(env, CLASS, e, obj(&mut out));
            mp3.extend_from_slice(&out.data[..w as usize]);

            // Offset fuera del buffer.
            let mut input = FakeBuffer {
                data: vec![0; 8],
                direct: true,
            };
            let mut s16 = FakeBuffer {
                data: vec![0; 64],
                direct: true,
            };
            assert_eq!(
                Java_expo_modules_audio2mp3_NativeEncoder_converterProcess(
                    env,
                    CLASS,
                    c,
                    obj(&mut input),
                    4,
                    8,
                    obj(&mut s16)
                ),
                -ERR_INVALID_INPUT
            );
            Java_expo_modules_audio2mp3_NativeEncoder_converterDestroy(env, CLASS, c);
            Java_expo_modules_audio2mp3_NativeEncoder_destroy(env, CLASS, e);
        }
        assert_eq!(mp3, expected);
    }
}
