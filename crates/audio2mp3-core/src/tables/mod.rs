//! Tablas del estándar ISO/IEC 11172-3.
//!
//! Pendientes de transcribir desde el estándar (spec §2):
//! - TODO(fase1): ventana de análisis C[0..511] (Anexo C, tabla C.1) → `window.rs`.
//! - TODO(fase1): tablas Huffman 0–31 (Anexo B, tabla B.7) → `huffman.rs`.

pub(crate) mod sfb;

/// Coeficientes c_i de la reducción de aliasing (Anexo B, tabla B.9).
pub(crate) const ALIAS_CI: [f32; 8] = [
    -0.6, -0.535, -0.33, -0.185, -0.095, -0.041, -0.0142, -0.0037,
];
