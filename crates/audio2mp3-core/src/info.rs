//! Frame Info (cabecera Xing de un MP3 CBR) con la extensión LAME.
//!
//! Es un frame silencioso que va el primero del archivo: los decoders lo
//! reconocen y no lo reproducen. Da el número de frames (duración exacta),
//! el tamaño, una tabla de búsqueda (TOC) y, en la extensión LAME, el
//! retardo del encoder y el padding final para la reproducción sin huecos.
//!
//! Formato público de Xing/LAME. El identificador del encoder es propio
//! (`a2m` + versión); algunos decoders (FFmpeg, symphonia) solo aplican
//! retardo/padding si es `LAME`/`Lavf`/`Lavc`, pero sí usan el número de
//! frames.

use crate::error::Error;
use crate::frame::FrameSpec;

/// Flags: frames, bytes, TOC y calidad presentes.
const FLAGS: u32 = 0x0F;
/// "Info" + flags + frames + bytes + TOC (100) + calidad.
const XING_LEN: usize = 4 + 4 + 4 + 4 + 100 + 4;
/// Extensión LAME completa (incluido su CRC final).
const LAME_LEN: usize = 36;
/// Bytes del frame que necesita el tag tras cabecera y side info.
pub(crate) const TAG_LEN: usize = XING_LEN + LAME_LEN;
// El tag cabe en el frame más pequeño: 64 kbps a 48 kHz, 192 bytes, estéreo
// (4 de cabecera + 32 de side info).
const _: () = assert!(4 + 32 + TAG_LEN <= 192);

/// Retardo de decodificación que los decoders añaden al del tag (528 + 1).
const DECODER_DELAY: u32 = 529;

/// Datos del flujo que guarda el frame Info.
pub(crate) struct InfoFields {
    /// Frames de audio (sin contar el frame Info).
    pub audio_frames: u32,
    /// Bytes del archivo completo, incluido el frame Info.
    pub total_bytes: u32,
    pub bitrate_kbps: u16,
    /// Retardo total del encoder (entrada → salida del decoder) en muestras.
    pub delay: u32,
    /// Muestras por canal de la entrada.
    pub input_samples: u64,
}

/// Escribe el frame Info completo en `out` (`spec.len` bytes).
pub(crate) fn write(
    out: &mut [u8],
    spec: &FrameSpec,
    head_len: usize,
    fields: &InfoFields,
) -> Result<usize, Error> {
    let frame = out.get_mut(..spec.len).ok_or(Error::BufferTooSmall)?;
    if head_len + TAG_LEN > frame.len() {
        return Err(Error::Internal);
    }
    frame.fill(0);
    // Cabecera; side info en ceros (frame sin audio).
    frame
        .get_mut(..4)
        .ok_or(Error::Internal)?
        .copy_from_slice(&spec.header.to_be_bytes());

    // Extensión LAME. Retardo y padding con su convención: los decoders
    // saltan `delay + 529` al principio y `padding − 529` al final.
    let enc_delay = fields.delay.saturating_sub(DECODER_DELAY).min(0xFFF);
    let decoded = u64::from(fields.audio_frames) * 1152;
    let padding = decoded
        .saturating_sub(fields.input_samples + u64::from(enc_delay))
        .min(0xFFF) as u32;
    let trim = (enc_delay << 12 | padding).to_be_bytes();
    // TOC lineal: en CBR la posición en bytes es proporcional al tiempo.
    let toc: [u8; 100] = core::array::from_fn(|i| (i * 256 / 100) as u8);

    let parts: [&[u8]; 20] = [
        b"Info",
        &FLAGS.to_be_bytes(),
        &fields.audio_frames.to_be_bytes(),
        &fields.total_bytes.to_be_bytes(),
        &toc,
        &[0; 4], // calidad
        &encoder_id(),
        &[0x01], // revisión 0, método CBR
        &[0],    // lowpass desconocido
        &[0; 4], // pico ReplayGain
        &[0; 2], // ganancia radio
        &[0; 2], // ganancia audiófilo
        &[0],    // flags de codificación y tipo de ATH
        &[u8::try_from(fields.bitrate_kbps).unwrap_or(u8::MAX)],
        &trim[1..],
        &[0],                              // misc
        &[0],                              // MP3 gain
        &[0; 2],                           // preset y surround
        &fields.total_bytes.to_be_bytes(), // longitud de la música
        &[0; 2],                           // CRC de la música (no calculado)
    ];
    let mut pos = head_len;
    for part in parts {
        frame
            .get_mut(pos..pos + part.len())
            .ok_or(Error::Internal)?
            .copy_from_slice(part);
        pos += part.len();
    }
    let crc_pos = pos;
    let crc = crc16(frame.get(..crc_pos).ok_or(Error::Internal)?);
    frame
        .get_mut(crc_pos..crc_pos + 2)
        .ok_or(Error::Internal)?
        .copy_from_slice(&crc.to_be_bytes());
    Ok(spec.len)
}

/// Frame Info provisional: solo "Info" y flags = 0 (sin datos). Si nunca se
/// reemplaza por el definitivo, los decoders lo saltan y estiman la
/// duración como sin frame Info (no recortan nada).
pub(crate) fn write_placeholder(
    out: &mut [u8],
    spec: &FrameSpec,
    head_len: usize,
) -> Result<usize, Error> {
    let frame = out.get_mut(..spec.len).ok_or(Error::BufferTooSmall)?;
    frame.fill(0);
    frame
        .get_mut(..4)
        .ok_or(Error::Internal)?
        .copy_from_slice(&spec.header.to_be_bytes());
    frame
        .get_mut(head_len..head_len + 4)
        .ok_or(Error::Internal)?
        .copy_from_slice(b"Info");
    Ok(spec.len)
}

/// `a2m` + versión, 9 bytes.
fn encoder_id() -> [u8; 9] {
    let mut id = [b' '; 9];
    let text = concat!("a2m", env!("CARGO_PKG_VERSION"));
    for (dst, src) in id.iter_mut().zip(text.bytes()) {
        *dst = src;
    }
    id
}

/// CRC-16/ARC (polinomio 0x8005 reflejado, inicio 0), el del tag LAME.
fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |mut crc, &byte| {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                crc >> 1 ^ 0xA001
            } else {
                crc >> 1
            };
        }
        crc
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_matches_arc_check_value() {
        assert_eq!(crc16(b"123456789"), 0xBB3D);
    }

    #[test]
    fn writes_fields_at_standard_offsets() {
        let spec = FrameSpec {
            header: 0xFFFB_9004,
            len: 417,
            main_data_len: 417 - 36,
        };
        let fields = InfoFields {
            audio_frames: 100,
            total_bytes: 41_800,
            bitrate_kbps: 128,
            delay: 1057,
            input_samples: 100_000,
        };
        let mut out = [0xAAu8; 417];
        assert_eq!(write(&mut out, &spec, 36, &fields), Ok(417));
        assert_eq!(&out[..4], &[0xFF, 0xFB, 0x90, 0x04]);
        assert!(out[4..36].iter().all(|&b| b == 0));
        assert_eq!(&out[36..40], b"Info");
        assert_eq!(&out[44..48], &100u32.to_be_bytes());
        assert_eq!(&out[48..52], &41_800u32.to_be_bytes());
        let lame = 36 + XING_LEN;
        assert_eq!(&out[lame..lame + 3], b"a2m");
        // Retardo 1057 − 529 = 528; padding 115200 − 100000 − 528 = 14672 → 0xFFF.
        let trim = u32::from_be_bytes([0, out[lame + 21], out[lame + 22], out[lame + 23]]);
        assert_eq!(trim >> 12, 528);
        assert_eq!(trim & 0xFFF, 0xFFF);
        let crc_pos = 36 + TAG_LEN - 2;
        assert_eq!(
            u16::from_be_bytes([out[crc_pos], out[crc_pos + 1]]),
            crc16(&out[..crc_pos])
        );
        assert!(out[36 + TAG_LEN..].iter().all(|&b| b == 0));
    }
}
