//! QR symbol geometry.
//!
//! This module does **not** encode QR codes. `GS ( k` hands the printer the data and the
//! mechanism generates the symbol itself, so there is no encoder, no bitmap, and no
//! dependency here. What layout still needs is the one thing the printer will not tell it
//! in advance: how wide the finished symbol will be, so an overflowing template can be
//! rejected instead of printing a clipped, unscannable code.
//!
//! # Error correction is fixed at M
//!
//! Level M is the right default for payment codes and QRIS assumes it. Fixing it keeps
//! this table one column instead of four. Exposing an EC knob later means adding the L, Q
//! and H capacity columns and threading the level through [`Op::Qr`](crate::Op::Qr) — the
//! shape is ready for that, but v0.2 deliberately does not ship the knob.
//!
//! # Sizing is an upper bound
//!
//! Capacities below are for **byte mode**, the most expensive of the four encoding modes.
//! The printer picks the mode itself, so a purely numeric payload may produce a smaller
//! symbol than predicted here. The error is always in the safe direction — occasionally
//! rejecting a template that would in fact have fit, never printing one that does not.
//! In practice this only bites within a few dots of the paper edge.

/// Module size the printer uses when a template does not say otherwise.
///
/// At version 10 (a typical QRIS payload) this is a 342-dot symbol, roughly 24mm on 80mm
/// paper — comfortably scannable off a phone screen without dominating the receipt.
pub const DEFAULT_MODULE: u8 = 6;

/// `GS ( k` accepts module sizes 1..=16.
pub const MIN_MODULE: u8 = 1;
/// `GS ( k` accepts module sizes 1..=16.
pub const MAX_MODULE: u8 = 16;

/// Quiet zone per side, in modules, as required by the QR specification.
///
/// Counted against the paper width here because Epson's `GS ( k` does not reserve it:
/// the symbol is printed at its bare module dimensions. A symbol flush against the paper
/// edge scans poorly or not at all, so the margin is treated as part of the footprint.
pub const QUIET_ZONE_MODULES: u16 = 4;

/// The largest QR version, and therefore the capacity ceiling.
pub const MAX_VERSION: u8 = 40;

/// Byte-mode capacity in bytes at error-correction level M, indexed by version - 1.
const BYTE_CAPACITY_M: [u16; MAX_VERSION as usize] = [
    14, 26, 42, 62, 84, 106, 122, 152, 180, 213, //  1..10
    251, 287, 331, 362, 412, 450, 504, 560, 624, 666, // 11..20
    711, 779, 857, 911, 997, 1059, 1125, 1190, 1264, 1370, // 21..30
    1452, 1538, 1628, 1722, 1809, 1911, 2013, 2099, 2213, 2331, // 31..40
];

/// The smallest version that holds `byte_len` bytes, or `None` past version 40.
pub fn version_for(byte_len: usize) -> Option<u8> {
    BYTE_CAPACITY_M
        .iter()
        .position(|&cap| usize::from(cap) >= byte_len)
        .map(|i| i as u8 + 1)
}

/// Side length of a version's symbol, in modules. Version 1 is 21, growing by 4.
pub fn modules_for_version(version: u8) -> u16 {
    17 + 4 * u16::from(version)
}

/// Printed footprint of a symbol, in dots, including both quiet zones.
///
/// `None` when the payload exceeds what a QR code can carry at all.
pub fn footprint_dots(byte_len: usize, module: u8) -> Option<u16> {
    let version = version_for(byte_len)?;
    let modules = modules_for_version(version) + 2 * QUIET_ZONE_MODULES;
    Some(modules.saturating_mul(u16::from(module)))
}

/// Capacity ceiling, for error messages that need to say what the limit was.
pub fn max_bytes() -> u16 {
    BYTE_CAPACITY_M[BYTE_CAPACITY_M.len() - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_grows_with_payload() {
        assert_eq!(version_for(1), Some(1));
        assert_eq!(version_for(14), Some(1));
        assert_eq!(version_for(15), Some(2));
        assert_eq!(version_for(2331), Some(40));
        assert_eq!(version_for(2332), None);
    }

    #[test]
    fn module_counts_match_the_specification() {
        assert_eq!(modules_for_version(1), 21);
        assert_eq!(modules_for_version(10), 57);
        assert_eq!(modules_for_version(40), 177);
    }

    #[test]
    fn a_real_qris_payload_fits_80mm_at_the_default_module() {
        // 210 bytes is a representative QRIS payload — it carries the merchant name and
        // city inline, so it is near the top of what these codes run to in practice.
        let version = version_for(210).unwrap();
        assert_eq!(version, 10);

        // 57 modules + 8 of quiet zone, at 6 dots = 390, inside 576.
        assert_eq!(footprint_dots(210, DEFAULT_MODULE), Some(390));
        assert!(footprint_dots(210, DEFAULT_MODULE).unwrap() <= 576);
    }

    #[test]
    fn the_same_payload_overflows_once_the_module_is_large_enough() {
        // 65 modules at 9 dots is 585 — six dots over an 80mm line, which is exactly the
        // kind of near-miss the check exists to catch.
        assert_eq!(footprint_dots(210, 9), Some(585));
        assert!(footprint_dots(210, 9).unwrap() > 576);
    }

    #[test]
    fn oversized_payloads_have_no_footprint_at_all() {
        assert_eq!(footprint_dots(9000, 1), None);
        assert_eq!(max_bytes(), 2331);
    }
}