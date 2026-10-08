//! Codificación Huffman del granule: selección de tablas, conteo de bits y
//! escritura (§2.4.1.7 y §2.4.2.7, Anexo C.1.5.4.4).

use crate::bitstream::BitWriter;
use crate::error::Error;
use crate::mdct::GRANULE_LINES;
use crate::quantize::{split_regions, Partition};
use crate::tables::huffman::{PairTable, COUNT1_A, COUNT1_B, PAIR_TABLES};
use crate::tables::sfb::LongBands;

/// Tablas sin linbits, de menor a mayor rango.
const SMALL_TABLES: [usize; 13] = [1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15];
/// Familias con linbits (mismos códigos, distinto `linbits` creciente).
const LINBITS_FAMILIES: [core::ops::RangeInclusive<usize>; 2] = [16..=23, 24..=31];

/// Cómo se codifica un granule y cuántos bits ocupa (`part2_3_length`,
/// sin scalefactors en Fase 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Coding {
    pub bits: u32,
    pub big_values: u32,
    pub table_select: [u32; 3],
    pub region0_count: u32,
    pub region1_count: u32,
    /// `true`: tabla count1 B.
    pub count1_b: bool,
    /// Final de la región count1 (primera línea en cero).
    count1_end: usize,
    /// Final de region0 y region1.
    region_ends: [usize; 2],
}

fn table(t: usize) -> Option<&'static PairTable> {
    PAIR_TABLES.get(t)?.as_ref()
}

/// Bits de un par (código + linbits + signos), o `None` si la tabla no
/// puede representarlo.
fn pair_bits(tb: &PairTable, x: u32, y: u32) -> Option<u32> {
    let xlen = u32::from(tb.xlen);
    let linbits = u32::from(tb.linbits);
    let split = |v: u32| -> Option<(u32, u32)> {
        if linbits > 0 && v >= 15 {
            ((v - 15) >> linbits == 0).then_some((15, linbits))
        } else {
            (v < xlen).then_some((v, 0))
        }
    };
    let (cx, ex) = split(x)?;
    let (cy, ey) = split(y)?;
    let len = u32::from(*tb.lens.get((cx * xlen + cy) as usize)?);
    Some(len + ex + ey + u32::from(x != 0) + u32::from(y != 0))
}

fn region_bits(t: usize, ix: &[i32]) -> Option<u32> {
    if t == 0 {
        return ix.iter().all(|&v| v == 0).then_some(0);
    }
    let tb = table(t)?;
    ix.as_chunks::<2>().0.iter().try_fold(0, |acc, &[x, y]| {
        Some(acc + pair_bits(tb, x.unsigned_abs(), y.unsigned_abs())?)
    })
}

/// Tabla más barata para una región, con sus bits.
fn best_table(ix: &[i32]) -> Option<(u32, u32)> {
    let max = ix.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
    if max == 0 {
        return Some((0, 0));
    }
    let mut best: Option<(u32, u32)> = None;
    let mut consider = |t: usize| {
        if let Some(bits) = region_bits(t, ix) {
            if best.is_none_or(|(_, b)| bits < b) {
                best = Some((t as u32, bits));
            }
        }
    };
    if max < 16 {
        for t in SMALL_TABLES {
            if table(t).is_some_and(|tb| u32::from(tb.xlen) > max) {
                consider(t);
            }
        }
    } else {
        // En cada familia basta la de menos linbits que cubra `max`.
        for family in LINBITS_FAMILIES {
            let fits = |t: &usize| table(*t).is_some_and(|tb| (max - 15) >> tb.linbits == 0);
            if let Some(t) = family.clone().find(fits) {
                consider(t);
            }
        }
    }
    best
}

fn quad_index(q: &[i32; 4]) -> usize {
    q.iter().fold(0, |acc, &v| acc << 1 | usize::from(v != 0))
}

fn quad_bits(table: &[(u8, u8); 16], q: &[i32; 4]) -> u32 {
    let index = quad_index(q);
    let len = table.get(index).map_or(0, |&(_, l)| u32::from(l));
    len + index.count_ones()
}

/// Planifica la codificación de `ix`. `None` si algún valor excede las tablas.
pub(crate) fn plan(ix: &[i32; GRANULE_LINES], bands: &LongBands) -> Option<Coding> {
    let partition = Partition::of(ix);
    let bv_end = partition.big_values as usize * 2;
    let count1_end = partition.count1_end();
    let (region0_count, region1_count) = if partition.big_values > 0 {
        split_regions(bands, partition.big_values)
    } else {
        (0, 0)
    };
    let bound = |i: usize| usize::from(bands.get(i).copied().unwrap_or(576)).min(bv_end);
    let region_ends = [
        bound(region0_count as usize + 1),
        bound((region0_count + region1_count) as usize + 2),
    ];

    let mut bits = 0;
    let mut table_select = [0; 3];
    let mut start = 0;
    for (select, end) in table_select
        .iter_mut()
        .zip([region_ends[0], region_ends[1], bv_end])
    {
        let (t, b) = best_table(ix.get(start..end)?)?;
        *select = t;
        bits += b;
        start = end;
    }

    let quads = ix.get(bv_end..count1_end)?.as_chunks::<4>().0;
    let bits_a: u32 = quads.iter().map(|q| quad_bits(&COUNT1_A, q)).sum();
    let bits_b: u32 = quads.iter().map(|q| quad_bits(&COUNT1_B, q)).sum();
    let count1_b = bits_b < bits_a;
    bits += bits_a.min(bits_b);

    Some(Coding {
        bits,
        big_values: partition.big_values,
        table_select,
        region0_count,
        region1_count,
        count1_b,
        count1_end,
        region_ends,
    })
}

/// Escribe el main data de un granule según `coding`.
pub(crate) fn write(
    w: &mut BitWriter<'_>,
    ix: &[i32; GRANULE_LINES],
    coding: &Coding,
) -> Result<(), Error> {
    let start_bits = w.bit_len();
    let bv_end = coding.big_values as usize * 2;
    let mut start = 0;
    for (&t, end) in
        coding
            .table_select
            .iter()
            .zip([coding.region_ends[0], coding.region_ends[1], bv_end])
    {
        let region = ix.get(start..end).ok_or(Error::Internal)?;
        start = end;
        if t == 0 {
            continue;
        }
        let tb = table(t as usize).ok_or(Error::Internal)?;
        for &[x, y] in region.as_chunks::<2>().0 {
            write_pair(w, tb, x, y)?;
        }
    }
    let quads = ix.get(bv_end..coding.count1_end).ok_or(Error::Internal)?;
    let count1 = if coding.count1_b {
        &COUNT1_B
    } else {
        &COUNT1_A
    };
    for q in quads.as_chunks::<4>().0 {
        let &(code, len) = count1.get(quad_index(q)).ok_or(Error::Internal)?;
        w.write(u32::from(code), u32::from(len))?;
        for &v in q.iter().filter(|&&v| v != 0) {
            w.write(u32::from(v < 0), 1)?;
        }
    }
    if (w.bit_len() - start_bits) as u32 != coding.bits {
        return Err(Error::Internal);
    }
    Ok(())
}

fn write_pair(w: &mut BitWriter<'_>, tb: &PairTable, x: i32, y: i32) -> Result<(), Error> {
    let linbits = u32::from(tb.linbits);
    let xlen = u32::from(tb.xlen);
    let (ax, ay) = (x.unsigned_abs(), y.unsigned_abs());
    let clamp = |v: u32| if linbits > 0 { v.min(15) } else { v };
    let index = (clamp(ax) * xlen + clamp(ay)) as usize;
    let code = *tb.codes.get(index).ok_or(Error::Internal)?;
    let len = u32::from(*tb.lens.get(index).ok_or(Error::Internal)?);
    w.write(code, len)?;
    // Orden del estándar: hcod, linbitsx, signx, linbitsy, signy.
    for (abs, v) in [(ax, x), (ay, y)] {
        if linbits > 0 && abs >= 15 {
            w.write(abs - 15, linbits)?;
        }
        if v != 0 {
            w.write(u32::from(v < 0), 1)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::sfb::long_bands;

    fn bands() -> &'static LongBands {
        long_bands(44_100).unwrap()
    }

    /// Decodifica con búsqueda lineal en las tablas (independiente de `write`).
    struct Reader<'a> {
        buf: &'a [u8],
        pos: usize,
    }

    impl Reader<'_> {
        fn bit(&mut self) -> u32 {
            let b = (self.buf[self.pos / 8] >> (7 - self.pos % 8)) & 1;
            self.pos += 1;
            u32::from(b)
        }
        fn bits(&mut self, n: u32) -> u32 {
            (0..n).fold(0, |acc, _| acc << 1 | self.bit())
        }
        fn code(&mut self, codes: &[(u32, u32)]) -> usize {
            let (mut value, mut len) = (0, 0);
            loop {
                value = value << 1 | self.bit();
                len += 1;
                if let Some(i) = codes.iter().position(|&(c, l)| l == len && c == value) {
                    return i;
                }
                assert!(len <= 19, "código inválido");
            }
        }
    }

    fn decode(buf: &[u8], coding: &Coding) -> [i32; GRANULE_LINES] {
        let mut r = Reader { buf, pos: 0 };
        let mut ix = [0; GRANULE_LINES];
        let bv_end = coding.big_values as usize * 2;
        let mut start = 0;
        for (&t, end) in
            coding
                .table_select
                .iter()
                .zip([coding.region_ends[0], coding.region_ends[1], bv_end])
        {
            if t != 0 {
                let tb = table(t as usize).unwrap();
                let codes: Vec<(u32, u32)> = tb
                    .codes
                    .iter()
                    .zip(tb.lens)
                    .map(|(&c, &l)| (c, u32::from(l)))
                    .collect();
                let xlen = usize::from(tb.xlen);
                for i in (start..end).step_by(2) {
                    let k = r.code(&codes);
                    for (slot, v) in [(i, k / xlen), (i + 1, k % xlen)] {
                        let mut v = v as u32;
                        if tb.linbits > 0 && v == 15 {
                            v += r.bits(u32::from(tb.linbits));
                        }
                        let neg = v != 0 && r.bit() == 1;
                        ix[slot] = if neg { -(v as i32) } else { v as i32 };
                    }
                }
            }
            start = end;
        }
        let quad = if coding.count1_b {
            &COUNT1_B
        } else {
            &COUNT1_A
        };
        let codes: Vec<(u32, u32)> = quad
            .iter()
            .map(|&(c, l)| (u32::from(c), u32::from(l)))
            .collect();
        for i in (bv_end..coding.count1_end).step_by(4) {
            let k = r.code(&codes);
            for j in 0..4 {
                if k >> (3 - j) & 1 == 1 {
                    ix[i + j] = if r.bit() == 1 { -1 } else { 1 };
                }
            }
        }
        assert_eq!(r.pos as u32, coding.bits);
        ix
    }

    fn roundtrip(ix: &[i32; GRANULE_LINES]) -> Coding {
        let coding = plan(ix, bands()).expect("plan");
        let mut buf = [0u8; 4096];
        let mut w = BitWriter::new(&mut buf);
        write(&mut w, ix, &coding).unwrap();
        assert_eq!(decode(&buf, &coding), *ix);
        coding
    }

    #[test]
    fn silence_costs_nothing() {
        let coding = roundtrip(&[0; GRANULE_LINES]);
        assert_eq!((coding.bits, coding.big_values), (0, 0));
    }

    #[test]
    fn roundtrip_typical_spectrum() {
        let ix: [i32; GRANULE_LINES] = core::array::from_fn(|i| {
            let mag = (40.0 / (1.0 + i as f32 * 0.1)) as i32;
            let v = (i as i32 * 7919) % (mag + 1);
            if i % 3 == 0 {
                -v
            } else {
                v
            }
        });
        let coding = roundtrip(&ix);
        assert!(coding.big_values > 0);
    }

    #[test]
    fn roundtrip_uses_linbits_up_to_max() {
        let mut ix = [0; GRANULE_LINES];
        ix[0] = crate::quantize::MAX_QUANT;
        ix[1] = -16;
        ix[2] = 15;
        ix[3] = -300;
        ix[40] = 1;
        ix[45] = -1;
        let coding = roundtrip(&ix);
        assert!(coding.table_select[0] >= 16);
    }

    #[test]
    fn roundtrip_every_small_magnitude() {
        for max in 1..=20 {
            let ix: [i32; GRANULE_LINES] = core::array::from_fn(|i| {
                if i < 300 {
                    (i as i32 % (max + 1)) * if i % 2 == 0 { 1 } else { -1 }
                } else {
                    0
                }
            });
            roundtrip(&ix);
        }
    }

    #[test]
    fn rejects_values_beyond_tables() {
        let mut ix = [0; GRANULE_LINES];
        ix[0] = crate::quantize::MAX_QUANT + 1;
        assert!(plan(&ix, bands()).is_none());
    }
}
