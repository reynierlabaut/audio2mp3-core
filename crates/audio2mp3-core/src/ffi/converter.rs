//! API C del converter (features `ffi` + `pcm-convert`).

use core::ptr;

use super::{error_code, finish, in_slice, out_slice};
use super::{AUDIO2MP3_ERR_INVALID_INPUT, AUDIO2MP3_ERR_NULL_POINTER, AUDIO2MP3_OK};
use crate::{PcmConverter, SampleFormat};

/// Handle opaco para C.
pub struct Audio2Mp3Converter {
    inner: PcmConverter,
}

/// # Safety
/// `out_conv` debe ser NULL o válido para escribir un puntero.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_converter_create(
    in_rate: u32,
    in_channels: u8,
    in_format: u8,
    out_conv: *mut *mut Audio2Mp3Converter,
) -> i32 {
    if out_conv.is_null() {
        return AUDIO2MP3_ERR_NULL_POINTER;
    }
    let format = match in_format {
        0 => Some(SampleFormat::S16),
        1 => Some(SampleFormat::F32),
        _ => None,
    };
    let (handle, code) = match format.map(|f| PcmConverter::new(in_rate, in_channels, f)) {
        None => (ptr::null_mut(), AUDIO2MP3_ERR_INVALID_INPUT),
        Some(Err(e)) => (ptr::null_mut(), error_code(e)),
        Some(Ok(inner)) => (
            Box::into_raw(Box::new(Audio2Mp3Converter { inner })),
            AUDIO2MP3_OK,
        ),
    };
    // SAFETY: no nulo y válido para escritura según el contrato.
    unsafe { out_conv.write(handle) };
    code
}

/// # Safety
/// `c` debe ser NULL o un handle vivo de `audio2mp3_converter_create`.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_converter_out_rate(c: *const Audio2Mp3Converter) -> u32 {
    // SAFETY: NULL o handle vivo según el contrato.
    unsafe { c.as_ref() }.map_or(0, |c| c.inner.out_rate())
}

/// # Safety
/// `c` debe ser NULL o un handle vivo de `audio2mp3_converter_create`.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_converter_out_channels(c: *const Audio2Mp3Converter) -> u8 {
    // SAFETY: NULL o handle vivo según el contrato.
    unsafe { c.as_ref() }.map_or(0, |c| c.inner.out_channels())
}

/// # Safety
/// `c` debe ser NULL o un handle vivo de `audio2mp3_converter_create`.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_converter_max_output_samples(
    c: *const Audio2Mp3Converter,
    in_bytes: usize,
) -> usize {
    // SAFETY: NULL o handle vivo según el contrato.
    unsafe { c.as_ref() }.map_or(0, |c| c.inner.max_output_samples(in_bytes))
}

/// # Safety
/// `c` debe ser NULL o un handle vivo de `audio2mp3_converter_create`.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_converter_max_flush_samples(
    c: *const Audio2Mp3Converter,
) -> usize {
    // SAFETY: NULL o handle vivo según el contrato.
    unsafe { c.as_ref() }.map_or(0, |c| c.inner.max_flush_samples())
}

/// # Safety
/// - `c`: NULL o handle vivo, sin uso concurrente.
/// - `input`: válido para leer `in_bytes` bytes (o NULL si es 0).
/// - `out`: válido para escribir `out_capacity` `int16_t` alineados, sin solaparse con `input`.
/// - `out_written`: válido para escritura.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_converter_process(
    c: *mut Audio2Mp3Converter,
    input: *const u8,
    in_bytes: usize,
    out: *mut i16,
    out_capacity: usize,
    out_written: *mut usize,
) -> i32 {
    if c.is_null() || out_written.is_null() {
        return AUDIO2MP3_ERR_NULL_POINTER;
    }
    let result = (|| {
        // SAFETY: según el contrato de esta función.
        let (input, out) = unsafe { (in_slice(input, in_bytes)?, out_slice(out, out_capacity)?) };
        // SAFETY: handle vivo y sin uso concurrente según el contrato.
        let conv = unsafe { &mut (*c).inner };
        conv.process_into(input, out).map_err(error_code)
    })();
    // SAFETY: `out_written` comprobado no nulo arriba.
    unsafe { finish(result, out_written) }
}

/// # Safety
/// - `c`: NULL o handle vivo, sin uso concurrente.
/// - `out`: válido para escribir `out_capacity` `int16_t` alineados.
/// - `out_written`: válido para escritura.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_converter_flush(
    c: *mut Audio2Mp3Converter,
    out: *mut i16,
    out_capacity: usize,
    out_written: *mut usize,
) -> i32 {
    if c.is_null() || out_written.is_null() {
        return AUDIO2MP3_ERR_NULL_POINTER;
    }
    let result = (|| {
        // SAFETY: según el contrato de esta función.
        let out = unsafe { out_slice(out, out_capacity) }?;
        // SAFETY: handle vivo y sin uso concurrente según el contrato.
        let conv = unsafe { &mut (*c).inner };
        conv.flush_into(out).map_err(error_code)
    })();
    // SAFETY: `out_written` comprobado no nulo arriba.
    unsafe { finish(result, out_written) }
}

/// # Safety
/// `c` debe ser NULL o un handle vivo que no se vuelva a usar.
#[no_mangle]
pub unsafe extern "C" fn audio2mp3_converter_destroy(c: *mut Audio2Mp3Converter) {
    if !c.is_null() {
        // SAFETY: creado con Box::into_raw en create y no liberado antes.
        drop(unsafe { Box::from_raw(c) });
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        AUDIO2MP3_ERR_FINISHED, AUDIO2MP3_ERR_INVALID_CHANNELS, AUDIO2MP3_ERR_INVALID_SAMPLE_RATE,
    };
    use super::*;

    #[test]
    fn create_reports_errors() {
        let mut c = ptr::NonNull::dangling().as_ptr();
        // SAFETY: punteros de salida válidos o NULL.
        unsafe {
            assert_eq!(
                audio2mp3_converter_create(1_000, 2, 0, &mut c),
                AUDIO2MP3_ERR_INVALID_SAMPLE_RATE
            );
            assert!(c.is_null());
            assert_eq!(
                audio2mp3_converter_create(44_100, 9, 0, &mut c),
                AUDIO2MP3_ERR_INVALID_CHANNELS
            );
            assert_eq!(
                audio2mp3_converter_create(44_100, 2, 7, &mut c),
                AUDIO2MP3_ERR_INVALID_INPUT
            );
            assert_eq!(
                audio2mp3_converter_create(44_100, 2, 0, ptr::null_mut()),
                AUDIO2MP3_ERR_NULL_POINTER
            );
            assert_eq!(audio2mp3_converter_out_rate(ptr::null()), 0);
            assert_eq!(audio2mp3_converter_max_flush_samples(ptr::null()), 0);
            audio2mp3_converter_destroy(ptr::null_mut());
        }
    }

    /// La API C produce lo mismo que `PcmConverter`.
    #[test]
    fn c_api_matches_rust_api() {
        let input: Vec<u8> = (0..6 * 3_000)
            .map(|i| (i as f32 * 0.01).sin() * 0.8)
            .flat_map(|v: f32| v.to_ne_bytes())
            .collect();
        let mut rust = PcmConverter::new(96_000, 6, SampleFormat::F32).unwrap();
        let mut expected = vec![0; rust.max_output_samples(input.len())];
        let n = rust.process_into(&input, &mut expected).unwrap();
        let mut tail = vec![0; rust.max_flush_samples()];
        let m = rust.flush_into(&mut tail).unwrap();
        expected.truncate(n);
        expected.extend_from_slice(&tail[..m]);

        let mut got = Vec::new();
        // SAFETY: todos los punteros son válidos durante cada llamada.
        unsafe {
            let mut c = ptr::null_mut();
            assert_eq!(
                audio2mp3_converter_create(96_000, 6, 1, &mut c),
                AUDIO2MP3_OK
            );
            assert_eq!(
                (
                    audio2mp3_converter_out_rate(c),
                    audio2mp3_converter_out_channels(c)
                ),
                (48_000, 2)
            );
            let mut written = 0;
            // En dos llamadas, cortando una muestra por la mitad.
            for part in [&input[..1001], &input[1001..]] {
                let mut out = vec![0i16; audio2mp3_converter_max_output_samples(c, part.len())];
                let code = audio2mp3_converter_process(
                    c,
                    part.as_ptr(),
                    part.len(),
                    out.as_mut_ptr(),
                    out.len(),
                    &mut written,
                );
                assert_eq!(code, AUDIO2MP3_OK);
                got.extend_from_slice(&out[..written]);
            }
            let mut out = vec![0i16; audio2mp3_converter_max_flush_samples(c)];
            assert_eq!(
                audio2mp3_converter_flush(c, out.as_mut_ptr(), out.len(), &mut written),
                AUDIO2MP3_OK
            );
            got.extend_from_slice(&out[..written]);
            assert_eq!(
                audio2mp3_converter_flush(c, out.as_mut_ptr(), out.len(), &mut written),
                AUDIO2MP3_ERR_FINISHED
            );
            audio2mp3_converter_destroy(c);
        }
        assert_eq!(got, expected);
    }
}
