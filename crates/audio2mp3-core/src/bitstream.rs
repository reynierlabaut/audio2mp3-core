//! Escritor de bits MSB-first sobre un buffer fijo, sin reservar memoria.

use crate::error::Error;

pub(crate) struct BitWriter<'a> {
    buf: &'a mut [u8],
    bit_pos: usize,
}

impl<'a> BitWriter<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, bit_pos: 0 }
    }

    /// Bits escritos hasta ahora.
    pub fn bit_len(&self) -> usize {
        self.bit_pos
    }

    /// Bytes ocupados (el último puede estar incompleto).
    pub fn byte_len(&self) -> usize {
        self.bit_pos.div_ceil(8)
    }

    /// Escribe los `bits` bits bajos de `value` (0..=32). `value` no puede
    /// tener bits por encima de `bits`. Los bytes se ponen a 0 al tocarlos
    /// por primera vez, así que el buffer no necesita venir limpio.
    pub fn write(&mut self, value: u32, bits: u32) -> Result<(), Error> {
        if bits == 0 {
            return Ok(());
        }
        if bits > 32 || (bits < 32 && value >> bits != 0) {
            return Err(Error::Internal);
        }
        if self.bit_pos + bits as usize > self.buf.len() * 8 {
            return Err(Error::Internal);
        }
        let mut remaining = bits;
        while remaining > 0 {
            let used = (self.bit_pos % 8) as u32;
            let free = 8 - used;
            let take = free.min(remaining);
            let shift = remaining - take;
            let chunk = ((value >> shift) & ((1u32 << take) - 1)) as u8;
            let slot = self.buf.get_mut(self.bit_pos / 8).ok_or(Error::Internal)?;
            if used == 0 {
                *slot = 0;
            }
            *slot |= chunk << (free - take);
            self.bit_pos += take as usize;
            remaining -= take;
        }
        Ok(())
    }

    /// Escribe `bits` ceros (relleno).
    pub fn write_zeros(&mut self, mut bits: usize) -> Result<(), Error> {
        while bits > 0 {
            let n = bits.min(32);
            self.write(0, n as u32)?;
            bits -= n;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_msb_first_across_bytes() {
        let mut buf = [0xAAu8; 4];
        let mut w = BitWriter::new(&mut buf);
        w.write(0b101, 3).unwrap();
        w.write(0x1FF, 9).unwrap();
        w.write(0xABCD, 16).unwrap();
        assert_eq!(w.bit_len(), 28);
        assert_eq!(w.byte_len(), 4);
        // 101 111111111 1010101111001101 0000
        assert_eq!(buf, [0b1011_1111, 0b1111_1010, 0b1011_1100, 0b1101_0000]);
    }

    #[test]
    fn writes_full_u32() {
        let mut buf = [0u8; 5];
        let mut w = BitWriter::new(&mut buf);
        w.write(0b1, 1).unwrap();
        w.write(0xDEAD_BEEF, 32).unwrap();
        assert_eq!(buf, [0xEF, 0x56, 0xDF, 0x77, 0x80]);
    }

    #[test]
    fn rejects_overflow_and_oversized_values() {
        let mut buf = [0u8; 1];
        let mut w = BitWriter::new(&mut buf);
        assert_eq!(w.write(0b100, 2), Err(Error::Internal));
        assert_eq!(w.write(0, 33), Err(Error::Internal));
        w.write(0, 6).unwrap();
        assert_eq!(w.write(0, 3), Err(Error::Internal));
        w.write_zeros(2).unwrap();
        assert_eq!(w.write(1, 1), Err(Error::Internal));
    }
}
