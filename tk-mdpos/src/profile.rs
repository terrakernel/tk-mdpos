//! Printer description. The second half of the rendering contract.
//!
//! Paper width and dialect cannot be inferred from a template — the same template must
//! render at 58mm and 80mm. So a `Profile` is always a separate input, never a constant
//! baked into the template or the engine (`INSTRUCTIONS.md` §1.4).

#[cfg(feature = "serde")]
use serde::Deserialize;

/// A target printer's fixed characteristics.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Deserialize))]
pub struct Profile {
    /// Command dialect. v0.1 emits Epson only.
    pub dialect: Dialect,
    /// Printable width in dots. 576 = 80mm, 384 = 58mm.
    pub width_dots: u16,
    /// Device font, which fixes character cell width.
    pub font: Font,
    /// Character encoding for [`Op::Text`](crate::Op::Text).
    pub code_page: CodePage,
    /// Whether `GS V 66` is honored. Clones vary; see `INSTRUCTIONS.md` §7.
    pub supports_partial_cut: bool,
}

impl Profile {
    /// The single hardcoded v0.1 profile: 80mm, 576 dots, Font A, Epson, CP437.
    ///
    /// A profile registry and TOML loading are explicitly out of v0.1 scope. Resist
    /// adding them until the layout engine is known to be good.
    pub fn epson_80mm() -> Self {
        Self {
            dialect: Dialect::Epson,
            width_dots: 576,
            font: Font::A,
            code_page: CodePage::Cp437,
            supports_partial_cut: true,
        }
    }

    /// Characters per line at magnification 1x.
    ///
    /// Always derive the column count — never hardcode 48. Layout must recompute this
    /// against *current* magnification, since `{size 2x2}` halves it mid-document.
    pub fn columns(&self) -> u16 {
        self.width_dots / self.font.char_width_dots()
    }

    /// Characters per line at the given horizontal magnification (1..=8).
    pub fn columns_at(&self, mag_w: u8) -> u16 {
        self.width_dots / (self.font.char_width_dots() * u16::from(mag_w.max(1)))
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self::epson_80mm()
    }
}

/// Command dialect. Bytes are not interchangeable across these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Deserialize))]
pub enum Dialect {
    #[default]
    Epson,
    // v0.2+: Star, and beyond that ZPL/TSPL, which are not ESC/POS at all.
}

/// Device font. Determines the character cell width in dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Deserialize))]
pub enum Font {
    /// 12 dots wide. 48 columns at 576 dots.
    #[default]
    A,
    /// 9 dots wide. 64 columns at 576 dots.
    B,
}

impl Font {
    /// Width of one character cell, in dots, at magnification 1x.
    pub fn char_width_dots(self) -> u16 {
        match self {
            Font::A => 12,
            Font::B => 9,
        }
    }
}

/// Text encoding. Page 0 is CP437 on essentially every device; the tables diverge
/// sharply after that, which is why v0.1 supports exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Deserialize))]
pub enum CodePage {
    #[default]
    Cp437,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_derived_not_hardcoded() {
        assert_eq!(Profile::epson_80mm().columns(), 48);

        let p58 = Profile {
            width_dots: 384,
            ..Profile::epson_80mm()
        };
        assert_eq!(p58.columns(), 32);
    }

    #[test]
    fn magnification_halves_the_grid() {
        let p = Profile::epson_80mm();
        assert_eq!(p.columns_at(1), 48);
        assert_eq!(p.columns_at(2), 24);
        assert_eq!(p.columns_at(4), 12);
    }

    #[test]
    fn font_b_is_narrower() {
        let p = Profile {
            font: Font::B,
            ..Profile::epson_80mm()
        };
        assert_eq!(p.columns(), 64);
    }
}
