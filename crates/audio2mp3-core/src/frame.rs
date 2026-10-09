//! Cabecera, side info y longitud de frame MPEG-1 Layer III (ISO/IEC 11172-3 §2.4.1).

use crate::bitstream::BitWriter;
use crate::config::{sample_rate_index, EncoderConfig};
use crate::error::Error;

/// Muestras por canal en un frame (dos granules de 576).
pub(crate) const SAMPLES_PER_FRAME: usize = 1152;
/// Cota global de longitud de frame: 320 kbps a 32 kHz con padding.
pub(crate) const MAX_FRAME_LEN: usize = 1441;
/// Bytes máximos de cabecera + side info (estéreo).
pub(crate) const MAX_HEAD_LEN: usize = HEADER_LEN + 32;

const HEADER_LEN: usize = 4;
/// syncword (12) + ID=1 (MPEG-1) + layer=01 (III) + protection_bit=1 (sin CRC).
const HEADER_FIXED: u32 = 0xFFFB_0000;
const MODE_STEREO: u32 = 0b00;
const MODE_MONO: u32 = 0b11;

/// Longitud y cabecera de cada frame, con el padding repartido por
/// acumulación fraccional para que el bitrate medio sea exacto.
pub(crate) struct FrameLayout {
    header_base: u32,
    base_len: usize,
    frac: u32,
    sample_rate: u32,
    acc: u32,
    side_info_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameSpec {
    pub header: u32,
    /// Bytes totales del frame (cabecera + side info + main data).
    pub len: usize,
    /// Bytes disponibles para main data.
    pub main_data_len: usize,
}

impl FrameLayout {
    pub fn new(config: &EncoderConfig) -> Result<Self, Error> {
        config.validate()?;
        let sf = sample_rate_index(config.sample_rate)?;
        let mode = if config.channels == 1 {
            MODE_MONO
        } else {
            MODE_STEREO
        };
        // private=0, mode_extension=00, copyright=0, original=1, emphasis=00.
        let header_base =
            HEADER_FIXED | config.bitrate.index() << 12 | sf << 10 | mode << 6 | 1 << 2;
        let num = 144 * u64::from(config.bitrate.kbps()) * 1000;
        let rate = u64::from(config.sample_rate);
        Ok(Self {
            header_base,
            base_len: (num / rate) as usize,
            frac: (num % rate) as u32,
            sample_rate: config.sample_rate,
            acc: 0,
            side_info_len: side_info_len(config.channels),
        })
    }

    /// Longitud máxima de un frame con esta configuración.
    pub fn max_frame_len(&self) -> usize {
        self.base_len + usize::from(self.frac != 0)
    }

    /// Área de main data más pequeña (frame sin padding).
    pub fn min_main_data_len(&self) -> usize {
        self.base_len - HEADER_LEN - self.side_info_len
    }

    /// Bytes de cabecera + side info.
    pub fn head_len(&self) -> usize {
        HEADER_LEN + self.side_info_len
    }

    /// Calcula el siguiente frame y avanza el acumulador de padding.
    pub fn next_frame(&mut self) -> FrameSpec {
        self.acc += self.frac;
        let padding = self.acc >= self.sample_rate;
        if padding {
            self.acc -= self.sample_rate;
        }
        let len = self.base_len + usize::from(padding);
        FrameSpec {
            header: self.header_base | u32::from(padding) << 9,
            len,
            main_data_len: len - HEADER_LEN - self.side_info_len,
        }
    }
}

/// Cabecera con `mode = 01` (joint stereo) y `mode_extension = 10` (M/S, sin
/// intensity stereo).
pub(crate) fn with_mid_side(header: u32) -> u32 {
    header & !(0b1111 << 4) | 0b01 << 6 | 0b10 << 4
}

pub(crate) fn side_info_len(channels: u8) -> usize {
    if channels == 1 {
        17
    } else {
        32
    }
}

/// Side info de un granule y canal (solo bloques largos en Fase 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GranuleInfo {
    pub part2_3_length: u32,
    /// Pares de valores en la región big_values (≤ 288).
    pub big_values: u32,
    pub global_gain: u32,
    pub scalefac_compress: u32,
    pub table_select: [u32; 3],
    pub region0_count: u32,
    pub region1_count: u32,
    pub preflag: bool,
    pub scalefac_scale: bool,
    pub count1table_select: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SideInfo {
    /// 0 en Fase 1 (sin bit reservoir).
    pub main_data_begin: u32,
    pub scfsi: [[bool; 4]; 2],
    /// Indexado como `[granule][canal]`.
    pub granules: [[GranuleInfo; 2]; 2],
}

impl SideInfo {
    pub fn write(&self, w: &mut BitWriter<'_>, channels: u8) -> Result<(), Error> {
        let channels = usize::from(channels);
        w.write(self.main_data_begin, 9)?;
        w.write(0, if channels == 1 { 5 } else { 3 })?; // private_bits
        for scfsi in self.scfsi.iter().take(channels) {
            for &band in scfsi {
                w.write(u32::from(band), 1)?;
            }
        }
        for granule in &self.granules {
            for gi in granule.iter().take(channels) {
                w.write(gi.part2_3_length, 12)?;
                w.write(gi.big_values, 9)?;
                w.write(gi.global_gain, 8)?;
                w.write(gi.scalefac_compress, 4)?;
                w.write(0, 1)?; // window_switching_flag: bloques largos
                for &table in &gi.table_select {
                    w.write(table, 5)?;
                }
                w.write(gi.region0_count, 4)?;
                w.write(gi.region1_count, 3)?;
                w.write(u32::from(gi.preflag), 1)?;
                w.write(u32::from(gi.scalefac_scale), 1)?;
                w.write(u32::from(gi.count1table_select), 1)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Bitrate;

    fn config(sample_rate: u32, channels: u8, kbps: u16) -> EncoderConfig {
        EncoderConfig {
            sample_rate,
            channels,
            bitrate: Bitrate::from_kbps(kbps).unwrap(),
        }
    }

    #[test]
    fn header_bits() {
        let mut stereo = FrameLayout::new(&config(44_100, 2, 128)).unwrap();
        assert_eq!(stereo.next_frame().header, 0xFFFB_9004);
        let mut mono = FrameLayout::new(&config(48_000, 1, 320)).unwrap();
        assert_eq!(mono.next_frame().header, 0xFFFB_E4C4);
        let mut low = FrameLayout::new(&config(32_000, 2, 64)).unwrap();
        assert_eq!(low.next_frame().header, 0xFFFB_5804);
    }

    #[test]
    fn padding_keeps_exact_average_at_44100() {
        let mut layout = FrameLayout::new(&config(44_100, 2, 128)).unwrap();
        let frames = 1000u64;
        let mut total = 0u64;
        let mut padded = 0;
        for _ in 0..frames {
            let f = layout.next_frame();
            assert!(f.len == 417 || f.len == 418);
            assert_eq!(f.header >> 9 & 1 == 1, f.len == 418);
            padded += usize::from(f.len == 418);
            total += f.len as u64;
        }
        assert!(padded > 0);
        assert_eq!(total, frames * 144 * 128_000 / 44_100);
    }

    #[test]
    fn no_padding_at_32k_and_48k() {
        for rate in [32_000, 48_000] {
            for kbps in [64, 96, 128, 160, 192, 224, 256, 320] {
                let mut layout = FrameLayout::new(&config(rate, 2, kbps)).unwrap();
                let expected = 144 * usize::from(kbps) * 1000 / rate as usize;
                assert_eq!(layout.max_frame_len(), expected);
                for _ in 0..10 {
                    assert_eq!(layout.next_frame().len, expected);
                }
            }
        }
    }

    #[test]
    fn max_frame_len_bound() {
        let mut worst = 0;
        for rate in [32_000, 44_100, 48_000] {
            for kbps in [64, 96, 128, 160, 192, 224, 256, 320] {
                let layout = FrameLayout::new(&config(rate, 2, kbps)).unwrap();
                worst = worst.max(layout.max_frame_len());
            }
        }
        assert!(worst <= MAX_FRAME_LEN);
    }

    #[test]
    fn main_data_len_excludes_header_and_side_info() {
        let mut mono = FrameLayout::new(&config(48_000, 1, 192)).unwrap();
        assert_eq!(mono.next_frame().main_data_len, 576 - 4 - 17);
        let mut stereo = FrameLayout::new(&config(48_000, 2, 192)).unwrap();
        assert_eq!(stereo.next_frame().main_data_len, 576 - 4 - 32);
    }

    #[test]
    fn side_info_sizes() {
        for channels in [1u8, 2] {
            let mut buf = [0u8; 32];
            let mut w = BitWriter::new(&mut buf);
            SideInfo::default().write(&mut w, channels).unwrap();
            assert_eq!(w.bit_len(), side_info_len(channels) * 8);
        }
    }

    #[test]
    fn side_info_rejects_out_of_range_fields() {
        let mut info = SideInfo::default();
        info.granules[0][0].big_values = 512;
        let mut buf = [0u8; 32];
        let mut w = BitWriter::new(&mut buf);
        assert_eq!(info.write(&mut w, 2), Err(Error::Internal));
    }
}
