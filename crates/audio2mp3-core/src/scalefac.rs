//! Scalefactors (§2.4.1.7, §2.4.2.7): elección de `scalefac_compress` y
//! escritura (part2 del main data). Sin scfsi ni preflag;
//! `scalefac_scale = 0` (cada paso amplifica una banda 2^0.5).

use crate::bitstream::BitWriter;
use crate::error::Error;

/// Scalefactors por granule y canal. Bloques largos: bandas 0..21.
/// Bloques cortos: banda corta s, ventana w → `3·s + w`, bandas 0..12.
pub(crate) type Values = [u8; MAX_VALUES];
pub(crate) const MAX_VALUES: usize = 36;

/// (slen1, slen2) de cada `scalefac_compress`.
const SLEN: [(u32, u32); 16] = [
    (0, 0),
    (0, 1),
    (0, 2),
    (0, 3),
    (3, 0),
    (1, 1),
    (1, 2),
    (1, 3),
    (2, 1),
    (2, 2),
    (2, 3),
    (3, 1),
    (3, 2),
    (3, 3),
    (4, 2),
    (4, 3),
];

/// (scalefactors con slen1, total) según el tipo de bloque.
fn split(short: bool) -> (usize, usize) {
    if short {
        (18, 36)
    } else {
        (11, 21)
    }
}

/// Mayor scalefactor permitido en la posición `index` (slen1 ≤ 4, slen2 ≤ 3).
pub(crate) fn max_value(index: usize, short: bool) -> u8 {
    if index < split(short).0 {
        15
    } else {
        7
    }
}

fn bits_for(v: u8) -> u32 {
    u8::BITS - v.leading_zeros()
}

/// `scalefac_compress` más barato que representa `values`, con los bits de part2.
pub(crate) fn choose(values: &Values, short: bool) -> Option<(u32, u32)> {
    let (first, total) = split(short);
    let max = |range: core::ops::Range<usize>| {
        values
            .get(range)
            .and_then(|v| v.iter().copied().max())
            .unwrap_or(0)
    };
    let (need1, need2) = (bits_for(max(0..first)), bits_for(max(first..total)));
    let (n1, n2) = (first as u32, (total - first) as u32);
    SLEN.iter()
        .enumerate()
        .filter(|(_, &(s1, s2))| s1 >= need1 && s2 >= need2)
        .map(|(c, &(s1, s2))| (c as u32, n1 * s1 + n2 * s2))
        .min_by_key(|&(_, bits)| bits)
}

/// Escribe los scalefactors con el `scalefac_compress` dado.
pub(crate) fn write(
    w: &mut BitWriter<'_>,
    values: &Values,
    compress: u32,
    short: bool,
) -> Result<(), Error> {
    let &(slen1, slen2) = SLEN.get(compress as usize).ok_or(Error::Internal)?;
    let (first, total) = split(short);
    for (i, &v) in values.iter().enumerate().take(total) {
        w.write(u32::from(v), if i < first { slen1 } else { slen2 })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_scalefactors_cost_nothing() {
        assert_eq!(choose(&[0; MAX_VALUES], false), Some((0, 0)));
        assert_eq!(choose(&[0; MAX_VALUES], true), Some((0, 0)));
    }

    #[test]
    fn picks_cheapest_compress() {
        let mut v = [0; MAX_VALUES];
        // v[3] = 5 necesita slen1 ≥ 3 y v[15] = 1, slen2 ≥ 1: el más barato
        // que cubre ambos es (3, 1) = 11·3 + 10·1 = 43 bits.
        v[3] = 5;
        v[15] = 1;
        assert_eq!(choose(&v, false), Some((11, 43)));
        v[15] = 7;
        assert_eq!(choose(&v, false), Some((13, 63)));
        v[3] = 15;
        assert_eq!(choose(&v, false), Some((15, 74)));
        // Cortos: 18 + 18 posiciones.
        let mut s = [0; MAX_VALUES];
        s[20] = 2;
        assert_eq!(choose(&s, true), Some((2, 36)));
    }

    #[test]
    fn rejects_values_beyond_slen() {
        let mut v = [0; MAX_VALUES];
        v[12] = max_value(12, false) + 1;
        assert_eq!(choose(&v, false), None);
    }

    #[test]
    fn writes_expected_bit_count() {
        let mut v = [0; MAX_VALUES];
        v[0] = 9;
        v[20] = 3;
        let (compress, bits) = choose(&v, false).unwrap();
        let mut buf = [0u8; 16];
        let mut w = BitWriter::new(&mut buf);
        write(&mut w, &v, compress, false).unwrap();
        assert_eq!(w.bit_len() as u32, bits);
    }
}
