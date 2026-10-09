//! `Mp3Encoder`: une filterbank, MDCT, cuantización y Huffman en frames.

use core::f32::consts::FRAC_1_SQRT_2;

use crate::bitstream::BitWriter;
use crate::config::EncoderConfig;
use crate::error::Error;
use crate::filterbank::{Filterbank, SUBBANDS};
use crate::frame::{
    self, FrameLayout, GranuleInfo, SideInfo, MAX_FRAME_LEN, MAX_HEAD_LEN, SAMPLES_PER_FRAME,
};
use crate::huffman::{self, Coding};
use crate::mdct::{AliasReduction, BlockType, Mdct, GRANULE_LINES, SUBBAND_LEN};
use crate::quantize::{pow34, quantize};
use crate::reservoir::{Assembler, MAX_PENDING};
use crate::tables::sfb::{long_bands, short_bands, LongBands, ShortBands};
use crate::tables::window::ANALYSIS_WINDOW;

/// Muestras por canal en un granule.
const GRANULE_SAMPLES: usize = SAMPLES_PER_FRAME / 2;

/// Detector de transitorios: un intervalo de subbanda (32 muestras) es un
/// ataque si su energía supera `ATTACK_RATIO` veces la media de los
/// `ATTACK_HISTORY` anteriores más `ATTACK_FLOOR` (evita disparos en
/// señales casi silenciosas). Se ignora la subbanda 0 (graves).
const ATTACK_RATIO: f32 = 10.0;
const ATTACK_HISTORY: usize = 6;
const ATTACK_FLOOR: f32 = 1e-5;

/// Granule analizado (filterbank) a la espera de la MDCT.
#[derive(Clone, Copy)]
struct Analyzed {
    /// `[canal][subbanda][t]`, ya con la inversión de frecuencia.
    sub: [[[f32; SUBBAND_LEN]; SUBBANDS]; 2],
    /// Hay un ataque en el centro de la ventana de este granule.
    attack: bool,
}

const EMPTY_GRANULE: Analyzed = Analyzed {
    sub: [[[0.0; SUBBAND_LEN]; SUBBANDS]; 2],
    attack: false,
};

/// Retardo del encoder en muestras por canal: filterbank (análisis +
/// síntesis, 481) + MDCT/IMDCT (un granule, 576). Verificado en
/// `tests/roundtrip.rs` contra un decoder independiente.
const DELAY: u32 = 481 + 576;

/// Máximo de `part2_3_length` (12 bits).
const MAX_GRANULE_BITS: u32 = 4095;

/// Ganancia mínima del rate loop: por debajo, el ruido de cuantización queda
/// bajo el de un PCM de 16 bits (~1.5 dB por paso); afinar más solo gasta
/// bits, que se quedan en el bit reservoir. Calibrado con la señal de
/// `tests/roundtrip.rs`: 140 limitaba 320 kbps (69.2 dB), desde 130 ya no
/// (71.9 dB); 125 deja margen.
const MIN_GAIN: u8 = 125;

/// Ganancia de referencia para comparar L/R y M/S en el primer frame.
const INITIAL_PROBE_GAIN: u8 = 180;

/// Main data de un frame: reservoir (≤ 511) + área del frame.
const MAIN_DATA_CAP: usize = 511 + MAX_FRAME_LEN;

/// Encoder MP3 MPEG-1 Layer III, CBR, en streaming.
///
/// Usa bit reservoir, joint stereo M/S y bloques cortos ante transitorios
/// (con un granule de lookahead), así que la salida va unos frames por
/// detrás de la entrada (`max_output_size` lo tiene en cuenta).
/// Después de `new()`, `encode_into` y `flush_into` no reservan memoria.
pub struct Mp3Encoder {
    channels: usize,
    bands: &'static LongBands,
    short_bands: &'static ShortBands,
    layout: FrameLayout,
    /// PCM del granule en curso, normalizado a [-1, 1), por canal.
    pcm: [[f32; GRANULE_SAMPLES]; 2],
    pending: usize,
    /// Muestras por canal recibidas en total.
    consumed: u64,
    frames: u64,
    finished: bool,
    analysis: [Filterbank; 2],
    /// Granules analizados: los dos del próximo frame y uno de lookahead.
    queue: [Analyzed; 3],
    queued: usize,
    /// Energía de los intervalos 6..17 del granule anterior, por canal.
    energy_history: [[f32; 12]; 2],
    /// Tipo de bloque del último granule transformado.
    last_block: BlockType,
    /// Tipos de bloque del frame en curso.
    block_types: [BlockType; 2],
    /// Muestras de subbanda del granule anterior, por canal y subbanda.
    overlap: [[[f32; SUBBAND_LEN]; SUBBANDS]; 2],
    mdct: Mdct,
    alias: AliasReduction,
    /// Espectro `[granule][canal]` (L/R o M/S).
    xr: [[[f32; GRANULE_LINES]; 2]; 2],
    /// |xr|^(3/4) `[granule][canal]`.
    xr34: [[[f32; GRANULE_LINES]; 2]; 2],
    /// Espectro cuantizado `[granule][canal]`.
    ix: [[[i32; GRANULE_LINES]; 2]; 2],
    /// Ganancia del frame anterior: punto de partida del rate loop y nivel
    /// de ruido para la decisión L/R vs M/S.
    probe_gain: u8,
    main_data: [u8; MAIN_DATA_CAP],
    assembler: Assembler,
}

impl Mp3Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self, Error> {
        let layout = FrameLayout::new(&config)?;
        let bands = long_bands(config.sample_rate).ok_or(Error::InvalidSampleRate)?;
        let short_bands = short_bands(config.sample_rate).ok_or(Error::InvalidSampleRate)?;
        let assembler = Assembler::new(layout.max_frame_len(), layout.min_main_data_len());
        Ok(Self {
            channels: usize::from(config.channels),
            bands,
            short_bands,
            layout,
            pcm: [[0.0; GRANULE_SAMPLES]; 2],
            pending: 0,
            consumed: 0,
            frames: 0,
            finished: false,
            analysis: [
                Filterbank::new(&ANALYSIS_WINDOW),
                Filterbank::new(&ANALYSIS_WINDOW),
            ],
            queue: [EMPTY_GRANULE; 3],
            queued: 0,
            energy_history: [[0.0; 12]; 2],
            last_block: BlockType::Normal,
            block_types: [BlockType::Normal; 2],
            overlap: [[[0.0; SUBBAND_LEN]; SUBBANDS]; 2],
            mdct: Mdct::new(),
            alias: AliasReduction::new(),
            xr: [[[0.0; GRANULE_LINES]; 2]; 2],
            xr34: [[[0.0; GRANULE_LINES]; 2]; 2],
            ix: [[[0; GRANULE_LINES]; 2]; 2],
            probe_gain: INITIAL_PROBE_GAIN,
            main_data: [0; MAIN_DATA_CAP],
            assembler,
        })
    }

    /// Tamaño máximo en bytes que puede producir `encode_into` con
    /// `num_samples` muestras interleaved (incluye lo acumulado y los
    /// frames retenidos por el bit reservoir).
    pub fn max_output_size(&self, num_samples: usize) -> usize {
        let per_channel = num_samples / self.channels;
        let granules = self
            .queued
            .saturating_add(self.pending.saturating_add(per_channel) / GRANULE_SAMPLES);
        // Cada frame necesita sus dos granules más uno de lookahead.
        let frames = granules.saturating_sub(1) / 2;
        frames
            .saturating_add(self.assembler.pending())
            .saturating_mul(self.layout.max_frame_len())
    }

    /// Tamaño máximo que puede producir `flush_into`.
    pub fn max_flush_size(&self) -> usize {
        // Granules en cola y pendiente (< 1728) + retardo (< 1152) caben en
        // tres frames, más los retenidos por el reservoir.
        (3 + MAX_PENDING) * self.layout.max_frame_len()
    }

    /// Delay del encoder en muestras por canal.
    pub fn delay_samples(&self) -> u32 {
        DELAY
    }

    /// Codifica y escribe los frames completos en `out`. Devuelve los bytes escritos.
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
            if self.pending == GRANULE_SAMPLES {
                written += self.push_granule(out.get_mut(written..).ok_or(Error::Internal)?)?;
            }
        }
        self.consumed += (samples.len() / self.channels) as u64;
        Ok(written)
    }

    /// Completa con silencio los frames necesarios para que el decoder
    /// reproduzca toda la entrada (incluido el retardo), emite los frames
    /// retenidos y termina el encoder.
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
            written += self.push_granule(out.get_mut(written..).ok_or(Error::Internal)?)?;
        }
        written += self
            .assembler
            .finish(out.get_mut(written..).ok_or(Error::Internal)?)?;
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

    /// Analiza el granule completo de `self.pcm` y, si con él hay dos
    /// granules más uno de lookahead, codifica un frame.
    fn push_granule(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        self.analyze_granule()?;
        self.pending = 0;
        self.queued += 1;
        if self.queued < self.queue.len() {
            return Ok(0);
        }
        let written = self.encode_frame(out)?;
        // El granule de lookahead pasa a ser el primero del próximo frame.
        self.queue.rotate_left(2);
        self.queued = 1;
        Ok(written)
    }

    /// Filterbank del granule en curso → `self.queue[self.queued]`, con la
    /// detección de ataques.
    fn analyze_granule(&mut self) -> Result<(), Error> {
        let Self {
            channels,
            pcm,
            analysis,
            queue,
            queued,
            energy_history,
            ..
        } = self;
        let entry = queue.get_mut(*queued).ok_or(Error::Internal)?;
        entry.attack = false;
        let channels = pcm
            .iter()
            .zip(analysis)
            .zip(&mut entry.sub)
            .zip(energy_history)
            .take(*channels);
        for (((pcm, analysis), sub), history) in channels {
            for (t, block) in pcm.as_chunks::<SUBBANDS>().0.iter().enumerate() {
                let mut out = [0.0; SUBBANDS];
                analysis.analyze(block, &mut out);
                for (row, s) in sub.iter_mut().zip(out) {
                    *row.get_mut(t).ok_or(Error::Internal)? = s;
                }
            }
            // Inversión de frecuencia: el decoder niega las muestras impares
            // de las subbandas impares tras la IMDCT; el encoder lo hace antes.
            for row in sub.iter_mut().skip(1).step_by(2) {
                for s in row.iter_mut().skip(1).step_by(2) {
                    *s = -*s;
                }
            }
            entry.attack |= detect_attack(sub, history);
        }
        Ok(())
    }

    /// Codifica los dos primeros granules de la cola en un frame y escribe
    /// en `out` los frames que el reservoir deja completos.
    fn encode_frame(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        let spec = self.layout.next_frame();
        let begin = self.assembler.available();

        let [g0, g1, g2] = &self.queue;
        let first = BlockType::choose(self.last_block, g0.attack, g1.attack);
        let second = BlockType::choose(first, g1.attack, g2.attack);
        self.block_types = [first, second];
        self.last_block = second;
        for gr in 0..2 {
            for ch in 0..self.channels {
                self.transform(gr, ch)?;
            }
        }
        let mid_side = self.channels == 2 && self.prefer_mid_side();
        if mid_side {
            for granule in &mut self.xr {
                let [left, right] = granule;
                for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                    let (m, s) = ((*l + *r) * FRAC_1_SQRT_2, (*l - *r) * FRAC_1_SQRT_2);
                    *l = m;
                    *r = s;
                }
            }
        }

        let mut side = SideInfo {
            main_data_begin: begin as u32,
            ..SideInfo::default()
        };
        // Bits del área del frame más el reservoir disponible; lo que no se
        // use queda en el reservoir para los frames siguientes.
        let budget = ((begin + spec.main_data_len) * 8) as u32;
        let (gain, codings) = self.quantize_frame(budget)?;
        let granules = codings.iter().zip(&mut side.granules).zip(self.block_types);
        for ((codings_gr, side_gr), block_type) in granules {
            for (coding, info) in codings_gr.iter().zip(side_gr).take(self.channels) {
                *info = GranuleInfo {
                    part2_3_length: coding.bits,
                    big_values: coding.big_values,
                    global_gain: u32::from(gain),
                    block_type,
                    table_select: coding.table_select,
                    region0_count: coding.region0_count,
                    region1_count: coding.region1_count,
                    count1table_select: coding.count1_b,
                    ..GranuleInfo::default()
                };
            }
        }
        self.probe_gain = gain;

        let mut w = BitWriter::new(&mut self.main_data);
        for (ix_gr, codings_gr) in self.ix.iter().zip(&codings) {
            for (ix, coding) in ix_gr.iter().zip(codings_gr).take(self.channels) {
                huffman::write(&mut w, ix, coding)?;
            }
        }
        let used = w.bit_len().div_ceil(8);

        let header = if mid_side {
            frame::with_mid_side(spec.header)
        } else {
            spec.header
        };
        let mut head = [0u8; MAX_HEAD_LEN];
        let mut hw = BitWriter::new(&mut head);
        hw.write(header, 32)?;
        side.write(&mut hw, self.channels as u8)?;
        let head_len = self.layout.head_len();

        let written = self.assembler.push(
            head.get(..head_len).ok_or(Error::Internal)?,
            spec.len,
            begin,
            self.main_data.get(..used).ok_or(Error::Internal)?,
            out,
        )?;
        self.frames += 1;
        Ok(written)
    }

    /// `true` si M/S necesitaría menos bits que L/R. Estimación de alta
    /// resolución por banda de scalefactors: n líneas con energía E y ruido
    /// D por línea cuestan ≈ (n/2)·log2(E / (n·D)) bits (0 bajo el ruido).
    /// Con la misma ganancia el ruido es el mismo en L/R y en M/S (la
    /// transformación es ortonormal), así que gana la menor suma. E_M y E_S
    /// salen de E_L, E_R y Σ l·r, sin cuantizar nada.
    fn prefer_mid_side(&self) -> bool {
        let step = (0.25 * (f32::from(self.probe_gain) - 210.0)).exp2();
        let noise = step * step / 12.0;
        let cost = |energy: f32, n: f32| {
            let per_line = energy / n;
            if per_line > noise {
                0.5 * n * (per_line / noise).log2()
            } else {
                0.0
            }
        };
        let mut delta = 0.0f32;
        for [left, right] in &self.xr {
            for band in self.bands.windows(2) {
                let &[start, end] = band else { continue };
                let range = usize::from(start)..usize::from(end);
                let (Some(l), Some(r)) = (left.get(range.clone()), right.get(range)) else {
                    continue;
                };
                let (mut el, mut er, mut elr) = (0.0f32, 0.0f32, 0.0f32);
                for (&x, &y) in l.iter().zip(r) {
                    el += x * x;
                    er += y * y;
                    elr += x * y;
                }
                let n = l.len() as f32;
                let em = 0.5 * (el + er) + elr;
                let es = (0.5 * (el + er) - elr).max(0.0);
                delta += cost(em, n) + cost(es, n) - cost(el, n) - cost(er, n);
            }
        }
        delta < 0.0
    }

    /// MDCT del granule `gr` de la cola (canal `ch`) con su tipo de bloque
    /// → `self.xr[gr][ch]`. Bloques largos: reducción de aliasing. Bloques
    /// cortos: orden del bitstream (banda corta, ventana, frecuencia).
    fn transform(&mut self, gr: usize, ch: usize) -> Result<(), Error> {
        let block = *self.block_types.get(gr).ok_or(Error::Internal)?;
        let sub = self
            .queue
            .get(gr)
            .and_then(|g| g.sub.get(ch))
            .ok_or(Error::Internal)?;
        let overlap = self.overlap.get_mut(ch).ok_or(Error::Internal)?;
        let xr = self
            .xr
            .get_mut(gr)
            .and_then(|g| g.get_mut(ch))
            .ok_or(Error::Internal)?;
        for ((current, prev), lines) in sub
            .iter()
            .zip(overlap.iter_mut())
            .zip(xr.as_chunks_mut::<SUBBAND_LEN>().0)
        {
            let mut input = [0.0; 2 * SUBBAND_LEN];
            let (head, tail) = input.split_at_mut(SUBBAND_LEN);
            head.copy_from_slice(prev);
            tail.copy_from_slice(current);
            self.mdct.forward(block, &input, lines);
            *prev = *current;
        }
        if block == BlockType::Short {
            reorder_short(xr, self.short_bands);
        } else {
            self.alias.apply(xr);
        }
        Ok(())
    }

    /// Rate loop del frame: la menor ganancia común (≥ `MIN_GAIN`) con la
    /// que los dos granules de todos los canales caben juntos en `budget`
    /// bits (y cada uno en `part2_3_length`). Misma ganancia = mismo paso de
    /// cuantización = ruido repartido por igual, lo óptimo en error
    /// cuadrático para un número fijo de bits. Si ni con 255 cabe, el frame
    /// se codifica como silencio.
    fn quantize_frame(&mut self, budget: u32) -> Result<(u8, [[Coding; 2]; 2]), Error> {
        for (xr_gr, xr34_gr) in self.xr.iter().zip(&mut self.xr34) {
            for (xr, xr34) in xr_gr.iter().zip(xr34_gr).take(self.channels) {
                pow34(xr, xr34);
            }
        }
        let Some(gain) = self.lowest_fitting_gain(budget) else {
            let mut codings = [[Coding::default(); 2]; 2];
            for ((ix_gr, codings_gr), block) in
                self.ix.iter_mut().zip(&mut codings).zip(self.block_types)
            {
                for (ix, coding) in ix_gr.iter_mut().zip(codings_gr) {
                    ix.fill(0);
                    *coding = huffman::plan(ix, self.bands, block != BlockType::Normal)
                        .ok_or(Error::Internal)?;
                }
            }
            return Ok((u8::MAX, codings));
        };
        let codings = self.try_gain(gain, budget).ok_or(Error::Internal)?;
        Ok((gain, codings))
    }

    /// Menor ganancia en `MIN_GAIN..=255` con la que el frame cabe. La
    /// búsqueda parte de la ganancia del frame anterior (cambia poco entre
    /// frames) con pasos que se duplican hasta acotarla, y termina en
    /// binario: ~4 intentos en lugar de ~9.
    fn lowest_fitting_gain(&mut self, budget: u32) -> Option<u8> {
        let start = self.probe_gain.max(MIN_GAIN);
        // Invariante: cabe con `hi`; no cabe con `lo − 1` (o `lo == MIN_GAIN`).
        let (mut lo, mut hi);
        let mut step = 1u8;
        if self.try_gain(start, budget).is_some() {
            hi = start;
            loop {
                if hi == MIN_GAIN {
                    lo = MIN_GAIN;
                    break;
                }
                let cand = hi.saturating_sub(step).max(MIN_GAIN);
                if self.try_gain(cand, budget).is_some() {
                    hi = cand;
                    step = step.saturating_mul(2);
                } else {
                    lo = cand + 1;
                    break;
                }
            }
        } else {
            lo = start.checked_add(1)?;
            loop {
                let cand = lo.saturating_add(step - 1);
                if self.try_gain(cand, budget).is_some() {
                    hi = cand;
                    break;
                }
                lo = cand.checked_add(1)?;
                step = step.saturating_mul(2);
            }
        }
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.try_gain(mid, budget).is_some() {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        Some(hi)
    }

    /// Cuantiza todo el frame con `gain` en `self.ix`; `None` si no cabe.
    fn try_gain(&mut self, gain: u8, budget: u32) -> Option<[[Coding; 2]; 2]> {
        let mut codings = [[Coding::default(); 2]; 2];
        let mut total = 0u32;
        let units = self
            .xr
            .iter()
            .zip(&self.xr34)
            .zip(&mut self.ix)
            .zip(&mut codings)
            .zip(self.block_types);
        for ((((xr_gr, xr34_gr), ix_gr), codings_gr), block) in units {
            let switching = block != BlockType::Normal;
            let channels = xr_gr.iter().zip(xr34_gr).zip(ix_gr).zip(codings_gr);
            for (((xr, xr34), ix), coding) in channels.take(self.channels) {
                quantize(xr, xr34, gain, ix)?;
                *coding = huffman::plan(ix, self.bands, switching)
                    .filter(|c| c.bits <= MAX_GRANULE_BITS)?;
                total += coding.bits;
                if total > budget {
                    return None;
                }
            }
        }
        Some(codings)
    }
}

/// `true` si hay un ataque en el centro de la ventana del granule: los
/// intervalos 12..17 del granule anterior y 0..11 del actual. `history`
/// guarda la energía de los intervalos 6..17 del granule anterior y se
/// actualiza con los del actual.
fn detect_attack(sub: &[[f32; SUBBAND_LEN]; SUBBANDS], history: &mut [f32; 12]) -> bool {
    // seq[0..12]: intervalos 6..17 del granule anterior; seq[12..30]: actual.
    let mut seq = [0.0f32; 12 + SUBBAND_LEN];
    let (old, new) = seq.split_at_mut(12);
    old.copy_from_slice(history);
    for (t, e) in new.iter_mut().enumerate() {
        *e = sub
            .iter()
            .skip(1)
            .filter_map(|row| row.get(t))
            .map(|s| s * s)
            .sum();
    }
    let attack = (ATTACK_HISTORY..ATTACK_HISTORY + SUBBAND_LEN).any(|j| {
        let (Some(window), Some(&e)) = (seq.get(j - ATTACK_HISTORY..j), seq.get(j)) else {
            return false;
        };
        let background = window.iter().sum::<f32>() / ATTACK_HISTORY as f32;
        e > ATTACK_RATIO * background + ATTACK_FLOOR
    });
    if let Some(tail) = seq.get(SUBBAND_LEN..) {
        history.copy_from_slice(tail);
    }
    attack
}

/// Pasa un granule de bloques cortos del orden de la MDCT (subbanda,
/// línea, ventana: `sb·18 + 3·m + w`) al del bitstream: por banda corta,
/// las líneas de la ventana 0, luego la 1 y luego la 2.
fn reorder_short(xr: &mut [f32; GRANULE_LINES], bands: &ShortBands) {
    let source = *xr;
    let mut pos = 0;
    for band in bands.windows(2) {
        let &[start, end] = band else { continue };
        for w in 0..3 {
            for f in usize::from(start)..usize::from(end) {
                let from = (f / 6) * SUBBAND_LEN + 3 * (f % 6) + w;
                if let (Some(dst), Some(&v)) = (xr.get_mut(pos), source.get(from)) {
                    *dst = v;
                }
                pos += 1;
            }
        }
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
        let written = enc.encode_into(&samples, &mut out).unwrap();
        // El primer frame puede quedar retenido por el reservoir.
        assert!(written == 0 || written == 576);
        assert_eq!(written + 576 * enc.assembler.pending(), 576);
    }

    #[test]
    fn rejects_bad_input_and_use_after_flush() {
        let mut enc = Mp3Encoder::new(config(32_000, 2, 64)).unwrap();
        let mut out = vec![0u8; 16_384];
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

    #[test]
    fn identical_channels_use_mid_side() {
        let mono = signal(4 * 1152, 1);
        let stereo: Vec<i16> = mono.iter().flat_map(|&s| [s, s]).collect();
        let mut enc = Mp3Encoder::new(config(44_100, 2, 128)).unwrap();
        let mut out = Vec::new();
        enc.encode(&stereo, &mut out).unwrap();
        enc.flush(&mut out).unwrap();
        // Byte 3 de la cabecera: mode (2 bits) y mode_extension (2 bits).
        assert_eq!(out[3] >> 4, 0b0110, "joint stereo M/S");
    }
}
