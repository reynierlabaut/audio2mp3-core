//! `Mp3Encoder`: une filterbank, MDCT, cuantización y Huffman en frames.

use crate::bitstream::BitWriter;
use crate::config::EncoderConfig;
use crate::error::Error;
use crate::filterbank::{Filterbank, SUBBANDS};
use crate::frame::{FrameLayout, GranuleInfo, SideInfo, SAMPLES_PER_FRAME};
use crate::huffman::{self, Coding};
use crate::mdct::{AliasReduction, Mdct, GRANULE_LINES, SUBBAND_LEN};
use crate::quantize::{pow34, quantize};
use crate::tables::sfb::{long_bands, LongBands};
use crate::tables::window::ANALYSIS_WINDOW;

/// Retardo del encoder en muestras por canal: filterbank (análisis +
/// síntesis, 481) + MDCT/IMDCT (un granule, 576). Verificado en
/// `tests/roundtrip.rs` contra un decoder independiente.
const DELAY: u32 = 481 + 576;

/// Máximo de `part2_3_length` (12 bits).
const MAX_GRANULE_BITS: u32 = 4095;

/// Encoder MP3 MPEG-1 Layer III, CBR, en streaming.
///
/// Después de `new()`, `encode_into` y `flush_into` no reservan memoria.
pub struct Mp3Encoder {
    channels: usize,
    bands: &'static LongBands,
    layout: FrameLayout,
    /// PCM pendiente, normalizado a [-1, 1), por canal.
    pcm: [[f32; SAMPLES_PER_FRAME]; 2],
    pending: usize,
    /// Muestras por canal recibidas en total.
    consumed: u64,
    frames: u64,
    finished: bool,
    analysis: [Filterbank; 2],
    /// Muestras de subbanda del granule anterior, por canal y subbanda.
    overlap: [[[f32; SUBBAND_LEN]; SUBBANDS]; 2],
    mdct: Mdct,
    alias: AliasReduction,
    xr: [f32; GRANULE_LINES],
    xr34: [f32; GRANULE_LINES],
    /// Espectro cuantizado `[granule][canal]`.
    ix: [[[i32; GRANULE_LINES]; 2]; 2],
}

impl Mp3Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self, Error> {
        let layout = FrameLayout::new(&config)?;
        let bands = long_bands(config.sample_rate).ok_or(Error::InvalidSampleRate)?;
        Ok(Self {
            channels: usize::from(config.channels),
            bands,
            layout,
            pcm: [[0.0; SAMPLES_PER_FRAME]; 2],
            pending: 0,
            consumed: 0,
            frames: 0,
            finished: false,
            analysis: [
                Filterbank::new(&ANALYSIS_WINDOW),
                Filterbank::new(&ANALYSIS_WINDOW),
            ],
            overlap: [[[0.0; SUBBAND_LEN]; SUBBANDS]; 2],
            mdct: Mdct::new(),
            alias: AliasReduction::new(),
            xr: [0.0; GRANULE_LINES],
            xr34: [0.0; GRANULE_LINES],
            ix: [[[0; GRANULE_LINES]; 2]; 2],
        })
    }

    /// Tamaño máximo en bytes que puede producir `encode_into` con
    /// `num_samples` muestras interleaved (incluye lo acumulado).
    pub fn max_output_size(&self, num_samples: usize) -> usize {
        let per_channel = num_samples / self.channels;
        let frames = self.pending.saturating_add(per_channel) / SAMPLES_PER_FRAME;
        frames.saturating_mul(self.layout.max_frame_len())
    }

    /// Tamaño máximo que puede producir `flush_into`.
    pub fn max_flush_size(&self) -> usize {
        // pendiente (< 1152) + retardo (< 1152) caben en dos frames.
        2 * self.layout.max_frame_len()
    }

    /// Delay del encoder en muestras por canal.
    pub fn delay_samples(&self) -> u32 {
        DELAY
    }

    /// Codifica y escribe frames completos en `out`. Devuelve los bytes escritos.
    pub fn encode_into(&mut self, samples: &[i16], out: &mut [u8]) -> Result<usize, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        if !samples.len().is_multiple_of(self.channels) {
            return Err(Error::InvalidInput);
        }
        if out.len() < self.max_output_size(samples.len()) {
            return Err(Error::BufferTooSmall);
        }
        let mut written = 0;
        for frame in samples.chunks_exact(self.channels) {
            for (pcm, &s) in self.pcm.iter_mut().zip(frame) {
                *pcm.get_mut(self.pending).ok_or(Error::Internal)? = f32::from(s) / 32768.0;
            }
            self.pending += 1;
            if self.pending == SAMPLES_PER_FRAME {
                written += self.encode_frame(out.get_mut(written..).ok_or(Error::Internal)?)?;
            }
        }
        self.consumed += (samples.len() / self.channels) as u64;
        Ok(written)
    }

    /// Completa con silencio los frames necesarios para que el decoder
    /// reproduzca toda la entrada (incluido el retardo) y termina el encoder.
    pub fn flush_into(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        if out.len() < self.max_flush_size() {
            return Err(Error::BufferTooSmall);
        }
        let needed = if self.consumed == 0 {
            0
        } else {
            (self.consumed + u64::from(DELAY)).div_ceil(SAMPLES_PER_FRAME as u64)
        };
        let mut written = 0;
        while self.frames < needed {
            for pcm in &mut self.pcm {
                pcm.get_mut(self.pending..)
                    .ok_or(Error::Internal)?
                    .fill(0.0);
            }
            written += self.encode_frame(out.get_mut(written..).ok_or(Error::Internal)?)?;
        }
        self.finished = true;
        Ok(written)
    }

    /// Conveniencia: codifica añadiendo los bytes a `out`.
    pub fn encode(&mut self, samples: &[i16], out: &mut Vec<u8>) -> Result<(), Error> {
        let start = out.len();
        out.resize(start + self.max_output_size(samples.len()), 0);
        let result = self.encode_into(samples, out.get_mut(start..).unwrap_or_default());
        let written = result.as_ref().map_or(0, |&n| n);
        out.truncate(start + written);
        result.map(|_| ())
    }

    /// Conveniencia: vacía el encoder añadiendo los bytes a `out`.
    pub fn flush(&mut self, out: &mut Vec<u8>) -> Result<(), Error> {
        let start = out.len();
        out.resize(start + self.max_flush_size(), 0);
        let result = self.flush_into(out.get_mut(start..).unwrap_or_default());
        let written = result.as_ref().map_or(0, |&n| n);
        out.truncate(start + written);
        result.map(|_| ())
    }

    /// Codifica las 1152 muestras pendientes de cada canal en un frame.
    fn encode_frame(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        let spec = self.layout.next_frame();
        let mut side = SideInfo::default();
        let mut codings = [[Coding::default(); 2]; 2];
        let mut remaining = (spec.main_data_len * 8) as u32;
        let mut units_left = (2 * self.channels) as u32;

        for (gr, (codings_gr, side_gr)) in codings.iter_mut().zip(&mut side.granules).enumerate() {
            for (ch, (slot, info)) in codings_gr
                .iter_mut()
                .zip(side_gr)
                .take(self.channels)
                .enumerate()
            {
                self.transform(gr, ch)?;
                // Los bits que no usa un granule pasan a los siguientes del frame.
                let budget = (remaining / units_left).min(MAX_GRANULE_BITS);
                let (gain, coding) = self.quantize_granule(gr, ch, budget)?;
                remaining -= coding.bits;
                units_left -= 1;
                *slot = coding;
                *info = GranuleInfo {
                    part2_3_length: coding.bits,
                    big_values: coding.big_values,
                    global_gain: u32::from(gain),
                    table_select: coding.table_select,
                    region0_count: coding.region0_count,
                    region1_count: coding.region1_count,
                    count1table_select: coding.count1_b,
                    ..GranuleInfo::default()
                };
            }
        }

        let frame = out.get_mut(..spec.len).ok_or(Error::BufferTooSmall)?;
        let mut w = BitWriter::new(frame);
        w.write(spec.header, 32)?;
        side.write(&mut w, self.channels as u8)?;
        for (ix_gr, codings_gr) in self.ix.iter().zip(&codings) {
            for (ix, coding) in ix_gr.iter().zip(codings_gr).take(self.channels) {
                huffman::write(&mut w, ix, coding)?;
            }
        }
        // Relleno (datos auxiliares) hasta la longitud del frame.
        let pad = (spec.len * 8)
            .checked_sub(w.bit_len())
            .ok_or(Error::Internal)?;
        w.write_zeros(pad)?;

        self.pending = 0;
        self.frames += 1;
        Ok(spec.len)
    }

    /// Filterbank + MDCT + reducción de aliasing de un granule → `self.xr`.
    fn transform(&mut self, gr: usize, ch: usize) -> Result<(), Error> {
        let pcm = self.pcm.get(ch).ok_or(Error::Internal)?;
        let blocks = pcm
            .as_chunks::<SUBBANDS>()
            .0
            .iter()
            .skip(gr * SUBBAND_LEN)
            .take(SUBBAND_LEN);
        let analysis = self.analysis.get_mut(ch).ok_or(Error::Internal)?;
        // sub[subbanda][t]
        let mut sub = [[0.0f32; SUBBAND_LEN]; SUBBANDS];
        for (t, block) in blocks.enumerate() {
            let mut out = [0.0; SUBBANDS];
            analysis.analyze(block, &mut out);
            for (row, s) in sub.iter_mut().zip(out) {
                *row.get_mut(t).ok_or(Error::Internal)? = s;
            }
        }
        // Inversión de frecuencia: el decoder niega las muestras impares de
        // las subbandas impares tras la IMDCT; el encoder lo hace antes.
        for row in sub.iter_mut().skip(1).step_by(2) {
            for s in row.iter_mut().skip(1).step_by(2) {
                *s = -*s;
            }
        }
        let overlap = self.overlap.get_mut(ch).ok_or(Error::Internal)?;
        let lines = self.xr.as_chunks_mut::<SUBBAND_LEN>().0;
        for ((current, prev), xr) in sub.iter().zip(overlap.iter_mut()).zip(lines) {
            let mut input = [0.0; 2 * SUBBAND_LEN];
            let (head, tail) = input.split_at_mut(SUBBAND_LEN);
            head.copy_from_slice(prev);
            tail.copy_from_slice(current);
            self.mdct.forward(&input, xr);
            *prev = *current;
        }
        self.alias.apply(&mut self.xr);
        Ok(())
    }

    /// Rate loop: menor `global_gain` cuyo espectro cabe en `budget` bits.
    /// Si ni con 255 cabe, el granule se codifica como silencio.
    fn quantize_granule(
        &mut self,
        gr: usize,
        ch: usize,
        budget: u32,
    ) -> Result<(u8, Coding), Error> {
        pow34(&self.xr, &mut self.xr34);
        let ix = self
            .ix
            .get_mut(gr)
            .and_then(|g| g.get_mut(ch))
            .ok_or(Error::Internal)?;
        let (xr, xr34, bands) = (&self.xr, &self.xr34, self.bands);
        let mut attempt = |gain: u8| -> Option<Coding> {
            quantize(xr, xr34, gain, ix)?;
            huffman::plan(ix, bands).filter(|c| c.bits <= budget)
        };
        if attempt(u8::MAX).is_none() {
            ix.fill(0);
            let coding = huffman::plan(ix, bands).ok_or(Error::Internal)?;
            return Ok((u8::MAX, coding));
        }
        let (mut lo, mut hi) = (0u8, u8::MAX);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if attempt(mid).is_some() {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let coding = attempt(hi).ok_or(Error::Internal)?;
        Ok((hi, coding))
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

    fn signal(len: usize, channels: usize) -> Vec<i16> {
        (0..len * channels)
            .map(|i| {
                let t = (i / channels) as f32 / 44_100.0;
                let ch = (i % channels) as f32;
                let v = 0.5 * (t * 2.0 * core::f32::consts::PI * (440.0 + 220.0 * ch)).sin()
                    + 0.2 * (t * 2.0 * core::f32::consts::PI * 3_000.0).sin();
                (v * 32767.0) as i16
            })
            .collect()
    }

    fn encode_in_chunks(samples: &[i16], channels: u8, chunk: usize) -> Vec<u8> {
        let mut enc = Mp3Encoder::new(config(44_100, channels, 128)).unwrap();
        let mut out = Vec::new();
        for part in samples.chunks(chunk * usize::from(channels)) {
            enc.encode(part, &mut out).unwrap();
        }
        enc.flush(&mut out).unwrap();
        out
    }

    #[test]
    fn output_is_identical_for_any_chunk_size() {
        let samples = signal(44_100, 2);
        let reference = encode_in_chunks(&samples, 2, 100_000);
        assert!(!reference.is_empty());
        for chunk in [1, 7, 1152, 4096] {
            assert_eq!(
                encode_in_chunks(&samples, 2, chunk),
                reference,
                "chunk {chunk}"
            );
        }
    }

    #[test]
    fn frames_have_expected_count_and_sync() {
        let samples = signal(10_000, 1);
        let out = encode_in_chunks(&samples, 1, 4096);
        // ceil((10000 + DELAY) / 1152) frames
        let frames = (10_000 + DELAY as usize).div_ceil(1152);
        let mut pos = 0;
        let mut count = 0;
        while pos < out.len() {
            assert_eq!(out[pos], 0xFF);
            assert_eq!(out[pos + 1] & 0xE0, 0xE0);
            let padding = usize::from(out[pos + 2] >> 1 & 1);
            pos += 144 * 128_000 / 44_100 + padding;
            count += 1;
        }
        assert_eq!(pos, out.len());
        assert_eq!(count, frames);
    }

    #[test]
    fn buffer_too_small_does_not_consume_input() {
        let mut enc = Mp3Encoder::new(config(48_000, 2, 192)).unwrap();
        let samples = signal(2_000, 2);
        let mut small = [0u8; 10];
        assert_eq!(
            enc.encode_into(&samples, &mut small),
            Err(Error::BufferTooSmall)
        );
        assert_eq!(enc.pending, 0);
        let mut out = vec![0u8; enc.max_output_size(samples.len())];
        assert_eq!(enc.encode_into(&samples, &mut out), Ok(576));
    }

    #[test]
    fn rejects_bad_input_and_use_after_flush() {
        let mut enc = Mp3Encoder::new(config(32_000, 2, 64)).unwrap();
        let mut out = vec![0u8; 8192];
        assert_eq!(
            enc.encode_into(&[1, 2, 3], &mut out),
            Err(Error::InvalidInput)
        );
        assert_eq!(enc.flush_into(&mut out), Ok(0));
        assert_eq!(enc.encode_into(&[1, 2], &mut out), Err(Error::Finished));
        assert_eq!(enc.flush_into(&mut out), Err(Error::Finished));
    }

    #[test]
    fn full_scale_and_silence_fit_every_config() {
        for rate in [32_000, 44_100, 48_000] {
            for channels in [1u8, 2] {
                for kbps in [64, 320] {
                    let mut enc = Mp3Encoder::new(config(rate, channels, kbps)).unwrap();
                    let n = 3 * 1152 * usize::from(channels);
                    let loud: Vec<i16> = (0..n)
                        .map(|i| if i % 3 == 0 { i16::MAX } else { i16::MIN })
                        .collect();
                    let mut out = Vec::new();
                    enc.encode(&loud, &mut out).unwrap();
                    enc.encode(&vec![0; n], &mut out).unwrap();
                    enc.flush(&mut out).unwrap();
                    assert!(!out.is_empty());
                }
            }
        }
    }
}
