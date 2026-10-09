//! Bit reservoir y montaje de frames (§2.4.3.4.6, Anexo C.1.5.4.2).
//!
//! El main data de un frame puede empezar `main_data_begin` bytes antes de
//! su área, en el espacio que dejaron libre los frames anteriores. Se modela
//! el main data como un flujo continuo (las áreas de todos los frames, sin
//! cabeceras ni side info): el área del frame f ocupa `[A_f, A_f + cap_f)`
//! y sus datos `[A_f − begin_f, …)`, siempre dentro de su propia área al
//! final. Un frame se emite cuando ningún frame futuro puede escribir ya en
//! su área.

use crate::error::Error;
use crate::frame::MAX_FRAME_LEN;

/// Frames retenidos como máximo (incluido el que se acaba de añadir).
pub(crate) const MAX_PENDING: usize = 8;
/// Buffer de entrada del decoder Layer III: 7680 bits.
const DECODER_BUFFER_BYTES: usize = 960;
/// `main_data_begin` tiene 9 bits.
const MAX_MAIN_DATA_BEGIN: usize = 511;

pub(crate) struct Assembler {
    slots: [[u8; MAX_FRAME_LEN]; MAX_PENDING],
    /// Por slot: longitud del frame, bytes de cabecera + side info, inicio del área.
    len: [usize; MAX_PENDING],
    head: [usize; MAX_PENDING],
    area: [u64; MAX_PENDING],
    first: usize,
    count: usize,
    /// Inicio del área del próximo frame.
    next_area: u64,
    /// Fin de los datos escritos; lo anterior ya es definitivo.
    data_end: u64,
    max_begin: usize,
}

impl Assembler {
    /// `max_frame_len` y `min_area` (área de main data más pequeña) de la configuración.
    pub fn new(max_frame_len: usize, min_area: usize) -> Self {
        // Los frames pendientes (salvo el actual y uno de margen) cubren a
        // lo sumo `max_begin` bytes de área.
        let max_begin = MAX_MAIN_DATA_BEGIN
            .min(DECODER_BUFFER_BYTES.saturating_sub(max_frame_len))
            .min((MAX_PENDING - 2) * min_area);
        Self {
            slots: [[0; MAX_FRAME_LEN]; MAX_PENDING],
            len: [0; MAX_PENDING],
            head: [0; MAX_PENDING],
            area: [0; MAX_PENDING],
            first: 0,
            count: 0,
            next_area: 0,
            data_end: 0,
            max_begin,
        }
    }

    /// Bytes de reservoir para el próximo frame (su `main_data_begin`).
    pub fn available(&self) -> usize {
        let free = self.next_area - self.data_end;
        usize::try_from(free).map_or(self.max_begin, |f| f.min(self.max_begin))
    }

    /// Frames retenidos a la espera de completar su área.
    pub fn pending(&self) -> usize {
        self.count
    }

    /// Añade un frame de `frame_len` bytes cuya cabecera + side info es
    /// `head` y cuyo main data `main_data` empieza `begin` bytes antes de su
    /// área. Escribe en `out` los frames que quedan completos.
    pub fn push(
        &mut self,
        head: &[u8],
        frame_len: usize,
        begin: usize,
        main_data: &[u8],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        if self.count == MAX_PENDING || begin > self.available() || head.len() > frame_len {
            return Err(Error::Internal);
        }
        let area_len = frame_len - head.len();
        let start = self.next_area - begin as u64;
        let end = start + main_data.len() as u64;
        if end > self.next_area + area_len as u64 {
            return Err(Error::Internal);
        }
        let slot = (self.first + self.count) % MAX_PENDING;
        let bytes = self.slots.get_mut(slot).ok_or(Error::Internal)?;
        let frame = bytes.get_mut(..frame_len).ok_or(Error::Internal)?;
        frame.fill(0);
        frame
            .get_mut(..head.len())
            .ok_or(Error::Internal)?
            .copy_from_slice(head);
        self.len[slot] = frame_len;
        self.head[slot] = head.len();
        self.area[slot] = self.next_area;
        self.count += 1;
        self.next_area += area_len as u64;

        self.write_stream(start, main_data)?;
        self.data_end = end;
        self.release(out, false)
    }

    /// Emite todos los frames retenidos (el área sin usar queda en ceros).
    pub fn finish(&mut self, out: &mut [u8]) -> Result<usize, Error> {
        self.release(out, true)
    }

    /// Copia `data` en el flujo de main data a partir de `pos`.
    fn write_stream(&mut self, mut pos: u64, mut data: &[u8]) -> Result<(), Error> {
        for k in 0..self.count {
            if data.is_empty() {
                break;
            }
            let slot = (self.first + k) % MAX_PENDING;
            let (area, head, len) = (self.area[slot], self.head[slot], self.len[slot]);
            let area_end = area + (len - head) as u64;
            if pos < area || pos >= area_end {
                continue;
            }
            let offset = head + (pos - area) as usize;
            let n = data.len().min((area_end - pos) as usize);
            let (now, rest) = data.split_at(n);
            self.slots
                .get_mut(slot)
                .and_then(|s| s.get_mut(offset..offset + n))
                .ok_or(Error::Internal)?
                .copy_from_slice(now);
            data = rest;
            pos += n as u64;
        }
        if data.is_empty() {
            Ok(())
        } else {
            Err(Error::Internal)
        }
    }

    fn release(&mut self, out: &mut [u8], all: bool) -> Result<usize, Error> {
        let mut written = 0;
        while self.count > 0 {
            let slot = self.first;
            let (len, area_end) = (
                self.len[slot],
                self.area[slot] + (self.len[slot] - self.head[slot]) as u64,
            );
            if !all && area_end > self.data_end {
                break;
            }
            let frame = self
                .slots
                .get(slot)
                .and_then(|s| s.get(..len))
                .ok_or(Error::Internal)?;
            out.get_mut(written..written + len)
                .ok_or(Error::Internal)?
                .copy_from_slice(frame);
            written += len;
            self.first = (self.first + 1) % MAX_PENDING;
            self.count -= 1;
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames de 100 bytes con 10 de cabecera: área de 90.
    fn frames(asm: &mut Assembler, data: &[(usize, usize)]) -> Vec<u8> {
        let mut out = vec![0u8; 100 * MAX_PENDING];
        let mut all = Vec::new();
        for &(begin, used) in data {
            let head = [0xEE; 10];
            let main: Vec<u8> = (0..used).map(|i| (i % 250) as u8 + 1).collect();
            let n = asm.push(&head, 100, begin, &main, &mut out).unwrap();
            all.extend_from_slice(&out[..n]);
        }
        let n = asm.finish(&mut out).unwrap();
        all.extend_from_slice(&out[..n]);
        all
    }

    #[test]
    fn without_reservoir_frames_are_released_immediately_when_full() {
        let mut asm = Assembler::new(100, 90);
        let mut out = vec![0u8; 1000];
        assert_eq!(asm.push(&[1; 10], 100, 0, &[7; 90], &mut out), Ok(100));
        assert_eq!(asm.pending(), 0);
        assert_eq!(asm.available(), 0);
    }

    #[test]
    fn reservoir_places_data_in_previous_frames() {
        let mut asm = Assembler::new(100, 90);
        // Frame 0 usa 40 de 90 bytes: quedan 50 de reservoir.
        let mut out = vec![0u8; 1000];
        assert_eq!(asm.push(&[1; 10], 100, 0, &[7; 40], &mut out), Ok(0));
        assert_eq!(asm.available(), 50);
        // Frame 1 empieza 50 bytes antes de su área y usa 120.
        let n = asm.push(&[2; 10], 100, 50, &[9; 120], &mut out).unwrap();
        assert_eq!(n, 100);
        let frame0 = &out[..100];
        assert_eq!(&frame0[..10], &[1; 10]);
        assert_eq!(&frame0[10..50], &[7; 40]);
        assert_eq!(&frame0[50..100], &[9; 50]);
        assert_eq!(asm.available(), 20);
        let n = asm.finish(&mut out).unwrap();
        assert_eq!(n, 100);
        assert_eq!(&out[10..80], &[9; 70]);
        assert_eq!(&out[80..100], &[0; 20]);
    }

    #[test]
    fn reservoir_is_limited_by_main_data_begin_and_decoder_buffer() {
        let asm = Assembler::new(1045, 1009);
        assert_eq!(asm.max_begin, 0);
        let asm = Assembler::new(418, 382);
        assert_eq!(asm.max_begin, 511);
        let asm = Assembler::new(627, 595);
        assert_eq!(asm.max_begin, 333);
    }

    #[test]
    fn output_is_a_concatenation_of_complete_frames() {
        let mut asm = Assembler::new(100, 90);
        let out = frames(&mut asm, &[(0, 10), (80, 150), (20, 30), (0, 90), (0, 0)]);
        assert_eq!(out.len(), 500);
        for frame in out.chunks(100) {
            assert_eq!(&frame[..10], &[0xEE; 10]);
        }
    }

    #[test]
    fn rejects_inconsistent_requests() {
        let mut asm = Assembler::new(100, 90);
        let mut out = vec![0u8; 1000];
        assert_eq!(
            asm.push(&[0; 10], 100, 1, &[1; 10], &mut out),
            Err(Error::Internal)
        );
        assert_eq!(
            asm.push(&[0; 10], 100, 0, &[1; 91], &mut out),
            Err(Error::Internal)
        );
    }
}
