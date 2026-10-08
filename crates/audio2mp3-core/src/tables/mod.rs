//! Tablas del estándar ISO/IEC 11172-3.
//!
//! `window.rs` y `huffman.rs` son datos generados a partir de minimp3 (CC0);
//! no se editan a mano. Su corrección la comprueban los tests de `tests.rs`
//! de forma independiente de la fuente (ver spec §2).

pub(crate) mod huffman;
pub(crate) mod sfb;
pub(crate) mod window;

#[cfg(test)]
mod tests;

/// Coeficientes c_i de la reducción de aliasing (Anexo B, tabla B.9).
pub(crate) const ALIAS_CI: [f32; 8] = [
    -0.6, -0.535, -0.33, -0.185, -0.095, -0.041, -0.0142, -0.0037,
];
