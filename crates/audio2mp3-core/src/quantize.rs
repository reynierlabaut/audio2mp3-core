//! Cuantización no uniforme y partición del espectro (§2.4.3.4.7, Anexo C.1.5.4).

use crate::mdct::GRANULE_LINES;
use crate::tables::sfb::LongBands;

/// Mayor valor codificable: 15 + (2^13 − 1) con linbits = 13.
pub(crate) const MAX_QUANT: i32 = 15 + 8191;

/// |xr|^(3/4), calculado una vez por granule y reutilizado en el rate loop.
pub(crate) fn pow34(xr: &[f32; GRANULE_LINES], out: &mut [f32; GRANULE_LINES]) {
    for (o, x) in out.iter_mut().zip(xr) {
        *o = x.abs().powf(0.75);
    }
}

/// ix = sign(xr) · nint((|xr| / 2^((gg − 210) / 4))^(3/4) − 0.0946).
///
/// Devuelve el máximo |ix|, o `None` si algún valor supera `MAX_QUANT`
/// (el rate loop debe subir `global_gain`).
pub(crate) fn quantize(
    xr: &[f32; GRANULE_LINES],
    xr34: &[f32; GRANULE_LINES],
    global_gain: u8,
    ix: &mut [i32; GRANULE_LINES],
) -> Option<i32> {
    let step = (-0.1875 * (f32::from(global_gain) - 210.0)).exp2();
    let limit = (MAX_QUANT + 1) as f32;
    let mut max = 0;
    for ((q, &a), &x) in ix.iter_mut().zip(xr34).zip(xr) {
        let v = a * step + 0.4054;
        if v >= limit {
            return None;
        }
        // v ≥ 0, así que truncar es floor.
        let m = v as i32;
        *q = if x < 0.0 { -m } else { m };
        max = max.max(m);
    }
    Some(max)
}

/// División del granule en big_values (pares), count1 (cuádruplas con
/// |ix| ≤ 1) y rzero (ceros finales).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Partition {
    /// Número de pares en la región big_values (≤ 288).
    pub big_values: u32,
    /// Número de cuádruplas en la región count1.
    pub count1: u32,
}

impl Partition {
    pub fn of(ix: &[i32; GRANULE_LINES]) -> Self {
        let mut end = GRANULE_LINES;
        while end >= 2 && ix[end - 1] == 0 && ix[end - 2] == 0 {
            end -= 2;
        }
        let mut count1 = 0;
        while end >= 4 && ix[end - 4..end].iter().all(|v| v.abs() <= 1) {
            end -= 4;
            count1 += 1;
        }
        Self {
            big_values: (end / 2) as u32,
            count1,
        }
    }

    /// Primera línea después de la región count1.
    pub fn count1_end(&self) -> usize {
        (self.big_values as usize * 2) + self.count1 as usize * 4
    }
}

/// Elige `region0_count` (≤ 15) y `region1_count` (≤ 7) para repartir
/// big_values en tres regiones aproximadamente iguales, alineadas a bandas.
pub(crate) fn split_regions(bands: &LongBands, big_values: u32) -> (u32, u32) {
    let end = big_values * 2;
    let closest = |target: u32, range: core::ops::RangeInclusive<usize>| {
        range
            .min_by_key(|&i| {
                bands
                    .get(i)
                    .map_or(u32::MAX, |&b| u32::from(b).abs_diff(target))
            })
            .unwrap_or(1)
    };
    // Límite de region0: bands[r0 + 1], r0 ∈ 0..=15.
    let b0 = closest(end / 3, 1..=16);
    // Límite de region1: bands[r0 + r1 + 2], r1 ∈ 0..=7, índice ≤ 22.
    let b1 = closest(end * 2 / 3, (b0 + 1)..=(b0 + 8).min(22));
    ((b0 - 1) as u32, (b1 - b0 - 1) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::sfb::long_bands;

    fn spectrum() -> [f32; GRANULE_LINES] {
        core::array::from_fn(|i| ((i as f32) * 0.71).sin() * 0.3 / (1.0 + i as f32 * 0.05))
    }

    #[test]
    fn quantization_roundtrips_within_one_step() {
        let xr = spectrum();
        let mut xr34 = [0.0; GRANULE_LINES];
        pow34(&xr, &mut xr34);
        let mut ix = [0; GRANULE_LINES];
        for gg in [150u8, 180, 210] {
            assert!(quantize(&xr, &xr34, gg, &mut ix).is_some());
            let step = (0.25 * (f32::from(gg) - 210.0)).exp2();
            for (&q, &x) in ix.iter().zip(&xr) {
                let deq = (q.abs() as f32).powf(4.0 / 3.0) * step * q.signum() as f32;
                let next = ((q.abs() + 1) as f32).powf(4.0 / 3.0) * step;
                assert!((deq - x).abs() <= next - deq.abs() + 1e-6);
                assert!(q == 0 || q.signum() == x.signum() as i32);
            }
        }
    }

    #[test]
    fn quantization_reports_overflow() {
        let xr = [1.0; GRANULE_LINES];
        let mut xr34 = [0.0; GRANULE_LINES];
        pow34(&xr, &mut xr34);
        let mut ix = [0; GRANULE_LINES];
        assert_eq!(quantize(&xr, &xr34, 0, &mut ix), None);
        assert!(quantize(&xr, &xr34, 210, &mut ix).is_some());
    }

    #[test]
    fn partition_regions() {
        let mut ix = [0; GRANULE_LINES];
        assert_eq!(
            Partition::of(&ix),
            Partition {
                big_values: 0,
                count1: 0
            }
        );

        ix[0] = 5;
        ix[1] = -3;
        ix[2] = 1;
        ix[5] = -1;
        ix[9] = 1;
        let p = Partition::of(&ix);
        // 0..2 big_values, 2..10 count1 (dos cuádruplas), resto ceros.
        assert_eq!(
            p,
            Partition {
                big_values: 1,
                count1: 2
            }
        );
        assert_eq!(p.count1_end(), 10);

        ix[575] = 2;
        assert_eq!(
            Partition::of(&ix),
            Partition {
                big_values: 288,
                count1: 0
            }
        );
    }

    #[test]
    fn regions_fit_side_info_fields() {
        for rate in [32_000, 44_100, 48_000] {
            let bands = long_bands(rate).unwrap();
            for big_values in 0..=288 {
                let (r0, r1) = split_regions(bands, big_values);
                assert!(r0 <= 15 && r1 <= 7);
                assert!(r0 + r1 + 2 <= 22);
            }
        }
    }
}
