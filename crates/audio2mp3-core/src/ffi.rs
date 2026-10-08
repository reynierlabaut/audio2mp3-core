//! API C (feature `ffi`). Contrato en `include/audio2mp3.h`.
//!
//! Único módulo con `unsafe`: convierte punteros del llamador en slices y
//! delega en `Mp3Encoder`.

use core::ptr;

use crate::{Bitrate, EncoderConfig, Error, Mp3Encoder};

pub const AUDIO2MP3_OK: i32 = 0;
pub const AUDIO2MP3_ERR_INVALID_SAMPLE_RATE: i32 = 1;
pub const AUDIO2MP3_ERR_INVALID_CHANNELS: i32 = 2;
pub const AUDIO2MP3_ERR_INVALID_BITRATE: i32 = 3;
pub const AUDIO2MP3_ERR_INVALID_INPUT: i32 = 4;
pub const AUDIO2MP3_ERR_BUFFER_TOO_SMALL: i32 = 5;
pub const AUDIO2MP3_ERR_FINISHED: i32 = 6;
pub const AUDIO2MP3_ERR_NULL_POINTER: i32 = 7;
pub const AUDIO2MP3_ERR_INTERNAL: i32 = 99;

/// Handle opaco para C.
pub struct Audio2Mp3Encoder {
    inner: Mp3Encoder,
}

/// Código C de un error de la API Rust.
pub fn error_code(error: Error) -> i32 {
    match error {
        Error::InvalidSampleRate => AUDIO2MP3_ERR_INVALID_SAMPLE_RATE,
        Error::InvalidChannels => AUDIO2MP3_ERR_INVALID_CHANNELS,
        Error::InvalidBitrate => AUDIO2MP3_ERR_INVALID_BITRATE,
        Error::InvalidInput => AUDIO2MP3_ERR_INVALID_INPUT,
        Error::BufferTooSmall => AUDIO2MP3_ERR_BUFFER_TOO_SMALL,
        Error::Finished => AUDIO2MP3_ERR_FINISHED,
        Error::Internal => AUDIO2MP3_ERR_INTERNAL,
    }
}

const fn parse_u32(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut value = 0;
    let mut i = 0;
    while i < bytes.len() {
        value = value * 10 + (bytes[i] - b'0') as u32;
        i += 1;
    }
    value
}

const VERSION: u32 = parse_u32(env!("CARGO_PKG_VERSION_MAJOR")) << 16
    | parse_u32(env!("CARGO_PKG_VERSION_MINOR")) << 8
    | parse_u32(env!("CARGO_PKG_VERSION_PATCH"));

#[no_mangle]
pub extern "C" fn audio2mp3_version() -> u32 {
    VERSION
}

/// # Safety
/// `out_encoder` debe ser NULL o apuntar a memoria válida para escribir un puntero.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_encoder_create(
    sample_rate: u32,
    channels: u8,
    bitrate_kbps: u16,
    out_encoder: *mut *mut Audio2Mp3Encoder,
) -> i32 {
    if out_encoder.is_null() {
        return AUDIO2MP3_ERR_NULL_POINTER;
    }
    let encoder = Bitrate::from_kbps(bitrate_kbps).and_then(|bitrate| {
        Mp3Encoder::new(EncoderConfig {
            sample_rate,
            channels,
            bitrate,
        })
    });
    let (handle, code) = match encoder {
        Ok(inner) => (
            Box::into_raw(Box::new(Audio2Mp3Encoder { inner })),
            AUDIO2MP3_OK,
        ),
        Err(e) => (ptr::null_mut(), error_code(e)),
    };
    // SAFETY: no nulo y válido para escritura según el contrato.
    unsafe { out_encoder.write(handle) };
    code
}

/// # Safety
/// `enc` debe ser NULL o un handle vivo devuelto por `audio2mp3_encoder_create`.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_encoder_max_output_size(
    enc: *const Audio2Mp3Encoder,
    num_samples: usize,
) -> usize {
    // SAFETY: NULL o handle vivo según el contrato.
    unsafe { enc.as_ref() }.map_or(0, |e| e.inner.max_output_size(num_samples))
}

/// # Safety
/// `enc` debe ser NULL o un handle vivo devuelto por `audio2mp3_encoder_create`.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_encoder_max_flush_size(enc: *const Audio2Mp3Encoder) -> usize {
    // SAFETY: NULL o handle vivo según el contrato.
    unsafe { enc.as_ref() }.map_or(0, |e| e.inner.max_flush_size())
}

/// # Safety
/// `enc` debe ser NULL o un handle vivo devuelto por `audio2mp3_encoder_create`.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_encoder_delay_samples(enc: *const Audio2Mp3Encoder) -> u32 {
    // SAFETY: NULL o handle vivo según el contrato.
    unsafe { enc.as_ref() }.map_or(0, |e| e.inner.delay_samples())
}

/// Convierte un buffer de salida del llamador en slice.
///
/// # Safety
/// `out` debe ser válido para escribir `capacity` elementos (o NULL con capacity 0).
unsafe fn out_slice<'a, T>(out: *mut T, capacity: usize) -> Result<&'a mut [T], i32> {
    if capacity == 0 {
        return Ok(&mut []);
    }
    if out.is_null() {
        return Err(AUDIO2MP3_ERR_NULL_POINTER);
    }
    if !out.is_aligned() || capacity > isize::MAX as usize / size_of::<T>() {
        return Err(AUDIO2MP3_ERR_INVALID_INPUT);
    }
    // SAFETY: no nulo, alineado, tamaño acotado y válido para escritura según el contrato.
    Ok(unsafe { core::slice::from_raw_parts_mut(out, capacity) })
}

/// Convierte un buffer de entrada del llamador en slice.
///
/// # Safety
/// `input` debe ser válido para leer `len` elementos (o NULL con len 0).
unsafe fn in_slice<'a, T>(input: *const T, len: usize) -> Result<&'a [T], i32> {
    if len == 0 {
        return Ok(&[]);
    }
    if input.is_null() {
        return Err(AUDIO2MP3_ERR_NULL_POINTER);
    }
    if !input.is_aligned() || len > isize::MAX as usize / size_of::<T>() {
        return Err(AUDIO2MP3_ERR_INVALID_INPUT);
    }
    // SAFETY: no nulo, alineado, tamaño acotado y legible según el contrato.
    Ok(unsafe { core::slice::from_raw_parts(input, len) })
}

/// Escribe el resultado de una llamada en `out_written` y devuelve el código.
///
/// # Safety
/// `out_written` debe ser válido para escritura.
unsafe fn finish(result: Result<usize, i32>, out_written: *mut usize) -> i32 {
    let (written, code) = match result {
        Ok(n) => (n, AUDIO2MP3_OK),
        Err(code) => (0, code),
    };
    // SAFETY: comprobado no nulo por el llamador de `finish`.
    unsafe { out_written.write(written) };
    code
}

/// # Safety
/// - `enc`: NULL o handle vivo, sin uso concurrente.
/// - `pcm`: válido para leer `num_samples` `int16_t` alineados (o NULL si es 0).
/// - `out`: válido para escribir `out_capacity` bytes, sin solaparse con `pcm`.
/// - `out_written`: válido para escritura.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_encoder_encode(
    enc: *mut Audio2Mp3Encoder,
    pcm: *const i16,
    num_samples: usize,
    out: *mut u8,
    out_capacity: usize,
    out_written: *mut usize,
) -> i32 {
    if enc.is_null() || out_written.is_null() {
        return AUDIO2MP3_ERR_NULL_POINTER;
    }
    let result = (|| {
        // SAFETY: según el contrato de esta función.
        let (samples, out) =
            unsafe { (in_slice(pcm, num_samples)?, out_slice(out, out_capacity)?) };
        // SAFETY: handle vivo y sin uso concurrente según el contrato.
        let encoder = unsafe { &mut (*enc).inner };
        encoder.encode_into(samples, out).map_err(error_code)
    })();
    // SAFETY: `out_written` comprobado no nulo arriba.
    unsafe { finish(result, out_written) }
}

/// # Safety
/// - `enc`: NULL o handle vivo, sin uso concurrente.
/// - `out`: válido para escribir `out_capacity` bytes.
/// - `out_written`: válido para escritura.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_encoder_flush(
    enc: *mut Audio2Mp3Encoder,
    out: *mut u8,
    out_capacity: usize,
    out_written: *mut usize,
) -> i32 {
    if enc.is_null() || out_written.is_null() {
        return AUDIO2MP3_ERR_NULL_POINTER;
    }
    let result = (|| {
        // SAFETY: según el contrato de esta función.
        let out = unsafe { out_slice(out, out_capacity) }?;
        // SAFETY: handle vivo y sin uso concurrente según el contrato.
        let encoder = unsafe { &mut (*enc).inner };
        encoder.flush_into(out).map_err(error_code)
    })();
    // SAFETY: `out_written` comprobado no nulo arriba.
    unsafe { finish(result, out_written) }
}

/// # Safety
/// `enc` debe ser NULL o un handle vivo que no se vuelva a usar.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_encoder_destroy(enc: *mut Audio2Mp3Encoder) {
    if !enc.is_null() {
        // SAFETY: creado con Box::into_raw en create y no liberado antes.
        drop(unsafe { Box::from_raw(enc) });
    }
}

#[cfg(feature = "pcm-convert")]
mod converter;
#[cfg(feature = "pcm-convert")]
pub use converter::*;

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = include_str!("../../../include/audio2mp3.h");

    #[test]
    fn header_matches_codes_and_functions() {
        let codes = [
            ("AUDIO2MP3_OK", AUDIO2MP3_OK),
            (
                "AUDIO2MP3_ERR_INVALID_SAMPLE_RATE",
                AUDIO2MP3_ERR_INVALID_SAMPLE_RATE,
            ),
            (
                "AUDIO2MP3_ERR_INVALID_CHANNELS",
                AUDIO2MP3_ERR_INVALID_CHANNELS,
            ),
            (
                "AUDIO2MP3_ERR_INVALID_BITRATE",
                AUDIO2MP3_ERR_INVALID_BITRATE,
            ),
            ("AUDIO2MP3_ERR_INVALID_INPUT", AUDIO2MP3_ERR_INVALID_INPUT),
            (
                "AUDIO2MP3_ERR_BUFFER_TOO_SMALL",
                AUDIO2MP3_ERR_BUFFER_TOO_SMALL,
            ),
            ("AUDIO2MP3_ERR_FINISHED", AUDIO2MP3_ERR_FINISHED),
            ("AUDIO2MP3_ERR_NULL_POINTER", AUDIO2MP3_ERR_NULL_POINTER),
            ("AUDIO2MP3_ERR_INTERNAL", AUDIO2MP3_ERR_INTERNAL),
            // Formatos de `audio2mp3_converter_create`.
            ("AUDIO2MP3_FORMAT_S16", 0),
            ("AUDIO2MP3_FORMAT_F32", 1),
        ];
        let defines: Vec<(&str, i32)> = HEADER
            .lines()
            .filter_map(|l| l.strip_prefix("#define AUDIO2MP3_"))
            .filter_map(|l| {
                let mut parts = l.split_whitespace();
                let name = parts.next()?;
                let value = parts.next()?.parse().ok()?;
                Some((name, value))
            })
            .collect();
        assert_eq!(defines.len(), codes.len());
        for (name, value) in codes {
            let short = name.trim_start_matches("AUDIO2MP3_");
            assert!(
                defines.contains(&(short, value)),
                "{name} = {value} no está en el header"
            );
        }
        for f in [
            "uint32_t audio2mp3_version(void);",
            "int32_t audio2mp3_encoder_create(uint32_t sample_rate, uint8_t channels,",
            "size_t audio2mp3_encoder_max_output_size(const Audio2Mp3Encoder *enc,",
            "size_t audio2mp3_encoder_max_flush_size(const Audio2Mp3Encoder *enc);",
            "uint32_t audio2mp3_encoder_delay_samples(const Audio2Mp3Encoder *enc);",
            "int32_t audio2mp3_encoder_encode(Audio2Mp3Encoder *enc,",
            "int32_t audio2mp3_encoder_flush(Audio2Mp3Encoder *enc,",
            "void audio2mp3_encoder_destroy(Audio2Mp3Encoder *enc);",
            "int32_t audio2mp3_converter_create(uint32_t in_rate, uint8_t in_channels,",
            "uint32_t audio2mp3_converter_out_rate(const Audio2Mp3Converter *c);",
            "uint8_t audio2mp3_converter_out_channels(const Audio2Mp3Converter *c);",
            "size_t audio2mp3_converter_max_output_samples(const Audio2Mp3Converter *c,",
            "size_t audio2mp3_converter_max_flush_samples(const Audio2Mp3Converter *c);",
            "int32_t audio2mp3_converter_process(Audio2Mp3Converter *c,",
            "int32_t audio2mp3_converter_flush(Audio2Mp3Converter *c,",
            "void audio2mp3_converter_destroy(Audio2Mp3Converter *c);",
        ] {
            assert!(HEADER.contains(f), "falta en el header: {f}");
        }
    }

    #[test]
    fn version_matches_cargo() {
        let expected = env!("CARGO_PKG_VERSION");
        let v = audio2mp3_version();
        assert_eq!(
            format!("{}.{}.{}", v >> 16, v >> 8 & 0xFF, v & 0xFF),
            expected
        );
    }

    #[test]
    fn create_reports_config_errors() {
        let mut enc = ptr::NonNull::dangling().as_ptr();
        // SAFETY: puntero de salida válido.
        unsafe {
            assert_eq!(
                audio2mp3_encoder_create(22_050, 2, 128, &mut enc),
                AUDIO2MP3_ERR_INVALID_SAMPLE_RATE
            );
            assert!(enc.is_null());
            assert_eq!(
                audio2mp3_encoder_create(44_100, 3, 128, &mut enc),
                AUDIO2MP3_ERR_INVALID_CHANNELS
            );
            assert_eq!(
                audio2mp3_encoder_create(44_100, 2, 100, &mut enc),
                AUDIO2MP3_ERR_INVALID_BITRATE
            );
            assert_eq!(
                audio2mp3_encoder_create(44_100, 2, 128, ptr::null_mut()),
                AUDIO2MP3_ERR_NULL_POINTER
            );
        }
    }

    #[test]
    fn null_pointers_are_rejected() {
        let mut written = 123;
        let mut buf = [0u8; 16];
        // SAFETY: los punteros no nulos son válidos; los nulos se rechazan.
        unsafe {
            let mut enc = ptr::null_mut();
            assert_eq!(
                audio2mp3_encoder_create(48_000, 1, 128, &mut enc),
                AUDIO2MP3_OK
            );
            let null_enc = ptr::null_mut();
            assert_eq!(
                audio2mp3_encoder_encode(
                    null_enc,
                    ptr::null(),
                    0,
                    buf.as_mut_ptr(),
                    16,
                    &mut written
                ),
                AUDIO2MP3_ERR_NULL_POINTER
            );
            assert_eq!(
                audio2mp3_encoder_encode(
                    enc,
                    ptr::null(),
                    0,
                    buf.as_mut_ptr(),
                    16,
                    ptr::null_mut()
                ),
                AUDIO2MP3_ERR_NULL_POINTER
            );
            assert_eq!(
                audio2mp3_encoder_encode(enc, ptr::null(), 4, buf.as_mut_ptr(), 16, &mut written),
                AUDIO2MP3_ERR_NULL_POINTER
            );
            assert_eq!(written, 0);
            assert_eq!(
                audio2mp3_encoder_flush(enc, ptr::null_mut(), 16, &mut written),
                AUDIO2MP3_ERR_NULL_POINTER
            );
            assert_eq!(audio2mp3_encoder_max_output_size(ptr::null(), 10_000), 0);
            assert_eq!(audio2mp3_encoder_max_flush_size(ptr::null()), 0);
            assert_eq!(audio2mp3_encoder_delay_samples(ptr::null()), 0);
            audio2mp3_encoder_destroy(enc);
            audio2mp3_encoder_destroy(ptr::null_mut());
        }
    }

    #[test]
    fn misaligned_pcm_is_rejected() {
        let raw = [0u8; 9];
        let misaligned = raw
            .as_ptr()
            .wrapping_add(usize::from((raw.as_ptr() as usize).is_multiple_of(2)))
            as *const i16;
        let mut out = [0u8; 4096];
        let mut written = 0;
        // SAFETY: el puntero desalineado se rechaza antes de leerlo.
        unsafe {
            let mut enc = ptr::null_mut();
            assert_eq!(
                audio2mp3_encoder_create(44_100, 1, 128, &mut enc),
                AUDIO2MP3_OK
            );
            assert_eq!(
                audio2mp3_encoder_encode(
                    enc,
                    misaligned,
                    2,
                    out.as_mut_ptr(),
                    out.len(),
                    &mut written
                ),
                AUDIO2MP3_ERR_INVALID_INPUT
            );
            audio2mp3_encoder_destroy(enc);
        }
    }

    /// El mismo audio por la API C y por la API Rust produce los mismos bytes.
    #[test]
    fn c_api_matches_rust_api() {
        let pcm: Vec<i16> = (0..2 * 5_000)
            .map(|i| ((i * 37 % 2000) as i16 - 1000) * 8)
            .collect();
        let config = EncoderConfig {
            sample_rate: 44_100,
            channels: 2,
            bitrate: Bitrate::Kbps192,
        };
        let mut rust = Mp3Encoder::new(config).unwrap();
        let mut expected = Vec::new();
        rust.encode(&pcm, &mut expected).unwrap();
        rust.flush(&mut expected).unwrap();

        let mut got = Vec::new();
        // SAFETY: todos los punteros son válidos durante la llamada.
        unsafe {
            let mut enc = ptr::null_mut();
            assert_eq!(
                audio2mp3_encoder_create(44_100, 2, 192, &mut enc),
                AUDIO2MP3_OK
            );
            let mut out = vec![0u8; audio2mp3_encoder_max_output_size(enc, pcm.len())];
            let mut written = 0;
            let mut small = [0u8; 1];
            assert_eq!(
                audio2mp3_encoder_encode(
                    enc,
                    pcm.as_ptr(),
                    pcm.len(),
                    small.as_mut_ptr(),
                    1,
                    &mut written
                ),
                AUDIO2MP3_ERR_BUFFER_TOO_SMALL
            );
            assert_eq!(
                audio2mp3_encoder_encode(
                    enc,
                    pcm.as_ptr(),
                    pcm.len(),
                    out.as_mut_ptr(),
                    out.len(),
                    &mut written
                ),
                AUDIO2MP3_OK
            );
            got.extend_from_slice(&out[..written]);
            let mut tail = vec![0u8; audio2mp3_encoder_max_flush_size(enc)];
            assert_eq!(
                audio2mp3_encoder_flush(enc, tail.as_mut_ptr(), tail.len(), &mut written),
                AUDIO2MP3_OK
            );
            got.extend_from_slice(&tail[..written]);
            assert_eq!(
                audio2mp3_encoder_flush(enc, tail.as_mut_ptr(), tail.len(), &mut written),
                AUDIO2MP3_ERR_FINISHED
            );
            audio2mp3_encoder_destroy(enc);
        }
        assert_eq!(got, expected);
    }
}
