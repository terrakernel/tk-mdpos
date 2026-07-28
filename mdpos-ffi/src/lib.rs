//! C ABI for [`mdpos`].
//!
//! The core library was designed around this boundary from the start (`INSTRUCTIONS.md`
//! §1.1): pure function, no I/O, no callbacks, no lifetimes in the return, one
//! dependency. That is why this crate is small — nothing here is working around the
//! shape of the Rust API.
//!
//! # The rules
//!
//! - Every entry point is panic-safe. Panics unwinding across a C frame are undefined
//!   behaviour, so each one is wrapped in [`catch_unwind`](std::panic::catch_unwind) and
//!   reports [`MDPOS_ERR_PANIC`] instead. **This requires `panic = "unwind"`.** Building
//!   with `panic = "abort"` silently removes the safety net; the workspace pins it.
//! - Every buffer handed out must be returned to [`mdpos_free`]. It was allocated by
//!   Rust's allocator and only Rust's allocator may release it — never `free()`.
//! - Buffers are always NUL-terminated at `ptr[len]`, and `len` never counts that byte.
//!   ESC/POS output contains embedded zero bytes (`GS V 66 0` ends in one), so treating
//!   output as a C string truncates it — but it can never read past the allocation.
//! - On *any* error the out-buffer receives a human-readable UTF-8 message. Template
//!   errors carry the source line, and that text is meant for whoever edits the template,
//!   so a bare status code would throw away the useful half.
//!
//! # Minimal C usage
//!
//! ```c
//! MdposProfile profile = mdpos_profile_epson_80mm();
//! MdposBuf out;
//!
//! if (mdpos_render((const uint8_t *)tmpl, strlen(tmpl), &profile, &out) == MDPOS_OK) {
//!     fwrite(out.ptr, 1, out.len, printer);
//! } else {
//!     fprintf(stderr, "mdpos: %s\n", (const char *)out.ptr);
//! }
//! mdpos_free(out);   // required in both branches
//! ```

use std::panic::{catch_unwind, AssertUnwindSafe};

use mdpos::{CodePage, Dialect, Font, Profile};

// --- status codes --------------------------------------------------------------------

/// Success.
pub const MDPOS_OK: i32 = 0;
/// The template was rejected. The out-buffer holds the message, including its line number.
pub const MDPOS_ERR_TEMPLATE: i32 = -1;
/// The template bytes were not valid UTF-8.
pub const MDPOS_ERR_INVALID_UTF8: i32 = -2;
/// A required pointer argument was null.
pub const MDPOS_ERR_NULL_ARG: i32 = -3;
/// A profile field held a value this build does not define.
pub const MDPOS_ERR_INVALID_PROFILE: i32 = -4;
/// A panic was caught at the boundary. This is a bug in mdpos; please report it.
pub const MDPOS_ERR_PANIC: i32 = -99;

// --- types ---------------------------------------------------------------------------

/// An owned buffer produced by mdpos. Release with [`mdpos_free`].
///
/// `cap` is carried so the allocation can be reconstructed exactly. Callers must treat it
/// as opaque and must not modify any field.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MdposBuf {
    /// Bytes. Always NUL-terminated at `ptr[len]`; never null after a successful call.
    pub ptr: *mut u8,
    /// Length in bytes, excluding the trailing NUL.
    pub len: usize,
    /// Allocation capacity. Opaque; required by [`mdpos_free`].
    pub cap: usize,
}

impl MdposBuf {
    const EMPTY: Self = Self {
        ptr: std::ptr::null_mut(),
        len: 0,
        cap: 0,
    };

    /// Take ownership of a byte vector, appending the terminator.
    fn from_vec(mut v: Vec<u8>) -> Self {
        let len = v.len();
        v.push(0);
        let mut v = std::mem::ManuallyDrop::new(v);
        MdposBuf {
            ptr: v.as_mut_ptr(),
            len,
            cap: v.capacity(),
        }
    }

    fn from_str(s: &str) -> Self {
        Self::from_vec(s.as_bytes().to_vec())
    }
}

/// Printer description. Mirrors [`mdpos::Profile`] with the enums flattened to integers.
///
/// Build one with [`mdpos_profile_epson_80mm`] and adjust fields rather than filling it
/// in from scratch, so unknown-value errors stay impossible as the enums grow.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MdposProfile {
    /// Command dialect. 0 = Epson. Anything else is rejected.
    pub dialect: u8,
    /// Printable width in dots. 576 = 80mm, 384 = 58mm. Must be non-zero.
    pub width_dots: u16,
    /// Device font. 0 = A (12 dots/char), 1 = B (9 dots/char).
    pub font: u8,
    /// Code page. 0 = CP437.
    pub code_page: u8,
    /// Whether `GS V 66` is honored by the mechanism.
    pub supports_partial_cut: bool,
}

impl MdposProfile {
    fn to_profile(self) -> Result<Profile, &'static str> {
        Ok(Profile {
            dialect: match self.dialect {
                0 => Dialect::Epson,
                _ => return Err("unknown dialect: expected 0 (Epson)"),
            },
            width_dots: match self.width_dots {
                0 => return Err("width_dots must be non-zero"),
                w => w,
            },
            font: match self.font {
                0 => Font::A,
                1 => Font::B,
                _ => return Err("unknown font: expected 0 (A) or 1 (B)"),
            },
            code_page: match self.code_page {
                0 => CodePage::Cp437,
                _ => return Err("unknown code page: expected 0 (CP437)"),
            },
            supports_partial_cut: self.supports_partial_cut,
        })
    }
}

// --- entry points --------------------------------------------------------------------

/// The v0.1 default profile: 80mm, 576 dots, Font A, CP437, Epson.
#[unsafe(no_mangle)]
pub extern "C" fn mdpos_profile_epson_80mm() -> MdposProfile {
    MdposProfile {
        dialect: 0,
        width_dots: 576,
        font: 0,
        code_page: 0,
        supports_partial_cut: true,
    }
}

/// Characters per line for a profile at magnification `mag` (1..=8).
///
/// Returns 0 if `profile` is null or invalid. Hosts that draw their own preview need this
/// to avoid hardcoding 48.
///
/// # Safety
///
/// `profile` must be null or point to a valid [`MdposProfile`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdpos_columns(profile: *const MdposProfile, mag: u8) -> u16 {
    guard_value(0, || {
        let Some(p) = (unsafe { profile.as_ref() }) else {
            return 0;
        };
        p.to_profile().map(|p| p.columns_at(mag)).unwrap_or(0)
    })
}

/// The highest template format version this build implements.
///
/// A template declaring `{v N}` above this is rejected. Hosts that store templates should
/// record this alongside them — the *string* carries the compatibility promise, not the
/// crate version (`INSTRUCTIONS.md` §1.3).
#[unsafe(no_mangle)]
pub extern "C" fn mdpos_format_version() -> u32 {
    mdpos::parse::MAX_VERSION
}

/// This crate's version, as a static NUL-terminated string. Never free it.
#[unsafe(no_mangle)]
pub extern "C" fn mdpos_version() -> *const std::ffi::c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const std::ffi::c_char
}

/// Render a template to ESC/POS bytes.
///
/// `template` is UTF-8 of `template_len` bytes and need not be NUL-terminated. On success
/// `out` receives the bytes; on any error it receives a UTF-8 message. Either way `out`
/// must be released with [`mdpos_free`].
///
/// # Safety
///
/// `template` must point to `template_len` readable bytes, `profile` must point to a valid
/// [`MdposProfile`], and `out` must point to writable storage for one [`MdposBuf`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdpos_render(
    template: *const u8,
    template_len: usize,
    profile: *const MdposProfile,
    out: *mut MdposBuf,
) -> i32 {
    unsafe { call(template, template_len, profile, out, |t, p| {
        mdpos::render(t, p).map(MdposBuf::from_vec)
    }) }
}

/// Render a template to a monospace plaintext preview.
///
/// Identical contract to [`mdpos_render`]; the buffer holds UTF-8 text.
///
/// # Safety
///
/// Same requirements as [`mdpos_render`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdpos_preview(
    template: *const u8,
    template_len: usize,
    profile: *const MdposProfile,
    out: *mut MdposBuf,
) -> i32 {
    unsafe { call(template, template_len, profile, out, |t, p| {
        mdpos::preview(t, p).map(|s| MdposBuf::from_str(&s))
    }) }
}

/// Release a buffer produced by this library.
///
/// Safe to call on a zeroed or already-freed buffer, and required even after an error,
/// since error messages are returned through the same buffer.
///
/// # Safety
///
/// `buf` must be a buffer returned by this library and not yet freed. Its fields must not
/// have been modified.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdpos_free(buf: MdposBuf) {
    if buf.ptr.is_null() {
        return;
    }
    // `len` excludes the terminator, which the allocation does include.
    guard_value((), || unsafe {
        drop(Vec::from_raw_parts(buf.ptr, buf.len + 1, buf.cap));
    });
}

// --- plumbing ------------------------------------------------------------------------

/// Shared body for the two rendering entry points.
///
/// # Safety
///
/// Same requirements as [`mdpos_render`].
unsafe fn call(
    template: *const u8,
    template_len: usize,
    profile: *const MdposProfile,
    out: *mut MdposBuf,
    render: impl FnOnce(&str, &Profile) -> Result<MdposBuf, mdpos::Error>,
) -> i32 {
    // Without somewhere to report, there is nothing useful to do but return.
    if out.is_null() {
        return MDPOS_ERR_NULL_ARG;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        let (code, buf) = (|| {
            if template.is_null() || profile.is_null() {
                return (
                    MDPOS_ERR_NULL_ARG,
                    MdposBuf::from_str("template and profile must not be null"),
                );
            }

            let bytes = unsafe { std::slice::from_raw_parts(template, template_len) };
            let Ok(text) = std::str::from_utf8(bytes) else {
                return (
                    MDPOS_ERR_INVALID_UTF8,
                    MdposBuf::from_str("template is not valid UTF-8"),
                );
            };

            let profile = match unsafe { *profile }.to_profile() {
                Ok(p) => p,
                Err(msg) => return (MDPOS_ERR_INVALID_PROFILE, MdposBuf::from_str(msg)),
            };

            match render(text, &profile) {
                Ok(buf) => (MDPOS_OK, buf),
                Err(e) => (MDPOS_ERR_TEMPLATE, MdposBuf::from_str(&e.to_string())),
            }
        })();

        unsafe { out.write(buf) };
        code
    }));

    match result {
        Ok(code) => code,
        Err(_) => {
            // The closure may have panicked before writing, so `out` could still be
            // uninitialized. Leave it in a state that is safe to free.
            unsafe { out.write(MdposBuf::EMPTY) };
            MDPOS_ERR_PANIC
        }
    }
}

/// Run a closure with panics contained, falling back to `fallback`.
fn guard_value<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the ABI exactly as a C caller would: raw pointers, explicit free.
    fn render(src: &str, profile: &MdposProfile) -> (i32, Vec<u8>, String) {
        let mut out = MdposBuf::EMPTY;
        let code = unsafe { mdpos_render(src.as_ptr(), src.len(), profile, &mut out) };

        let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) }.to_vec();
        // The terminator must be present and outside `len` in every case.
        assert_eq!(unsafe { *out.ptr.add(out.len) }, 0, "missing NUL terminator");

        let text = String::from_utf8_lossy(&bytes).into_owned();
        unsafe { mdpos_free(out) };
        (code, bytes, text)
    }

    #[test]
    fn renders_a_template_to_bytes() {
        let profile = mdpos_profile_epson_80mm();
        let (code, bytes, _) = render("{center}\nHI\n{cut}", &profile);

        assert_eq!(code, MDPOS_OK);
        assert_eq!(&bytes[..2], &[0x1B, 0x40], "document must start with ESC @");
        // GS V 66 0 — the trailing zero is exactly why output is not a C string.
        assert_eq!(&bytes[bytes.len() - 4..], &[0x1D, 0x56, 0x42, 0x00]);
        assert!(bytes.contains(&0), "byte output contains embedded NULs");
    }

    #[test]
    fn preview_matches_the_rust_api() {
        let src = "{cols 10,10:r}\na|b\n{cut}";
        let profile = mdpos_profile_epson_80mm();

        let mut out = MdposBuf::EMPTY;
        let code = unsafe { mdpos_preview(src.as_ptr(), src.len(), &profile, &mut out) };
        assert_eq!(code, MDPOS_OK);

        let text = String::from_utf8_lossy(unsafe {
            std::slice::from_raw_parts(out.ptr, out.len)
        })
        .into_owned();
        unsafe { mdpos_free(out) };

        assert_eq!(text, mdpos::preview(src, &Profile::epson_80mm()).unwrap());
    }

    #[test]
    fn a_rejected_template_returns_its_message_not_just_a_code() {
        let profile = mdpos_profile_epson_80mm();
        let (code, _, text) = render("{cols 20,6:r}\nItem | 1.250.000", &profile);

        assert_eq!(code, MDPOS_ERR_TEMPLATE);
        assert_eq!(
            text,
            "line 2: \"1.250.000\" overflows right-aligned column 2 (width 6); \
             right-aligned columns never wrap"
        );
    }

    #[test]
    fn invalid_utf8_is_reported_rather_than_lost() {
        let profile = mdpos_profile_epson_80mm();
        let bad = [0xFF_u8, 0xFE];
        let mut out = MdposBuf::EMPTY;
        let code = unsafe { mdpos_render(bad.as_ptr(), bad.len(), &profile, &mut out) };

        assert_eq!(code, MDPOS_ERR_INVALID_UTF8);
        assert!(!out.ptr.is_null(), "an error must still produce a message");
        unsafe { mdpos_free(out) };
    }

    #[test]
    fn null_arguments_do_not_crash() {
        let profile = mdpos_profile_epson_80mm();
        let mut out = MdposBuf::EMPTY;

        let code = unsafe { mdpos_render(std::ptr::null(), 0, &profile, &mut out) };
        assert_eq!(code, MDPOS_ERR_NULL_ARG);
        unsafe { mdpos_free(out) };

        // A null out-buffer leaves nowhere to report, so only the code comes back.
        let src = "HI";
        let code =
            unsafe { mdpos_render(src.as_ptr(), src.len(), &profile, std::ptr::null_mut()) };
        assert_eq!(code, MDPOS_ERR_NULL_ARG);
    }

    #[test]
    fn unknown_profile_values_are_rejected() {
        let src = "HI";
        for bad in [
            MdposProfile {
                dialect: 9,
                ..mdpos_profile_epson_80mm()
            },
            MdposProfile {
                font: 7,
                ..mdpos_profile_epson_80mm()
            },
            MdposProfile {
                code_page: 3,
                ..mdpos_profile_epson_80mm()
            },
            // Zero width would otherwise produce a zero-column grid.
            MdposProfile {
                width_dots: 0,
                ..mdpos_profile_epson_80mm()
            },
        ] {
            let (code, _, text) = render(src, &bad);
            assert_eq!(code, MDPOS_ERR_INVALID_PROFILE, "{bad:?}");
            assert!(!text.is_empty(), "{bad:?} produced no explanation");
        }
    }

    #[test]
    fn profile_round_trips_and_reports_its_grid() {
        let p = mdpos_profile_epson_80mm();
        assert_eq!(unsafe { mdpos_columns(&p, 1) }, 48);
        assert_eq!(unsafe { mdpos_columns(&p, 2) }, 24);

        let narrow = MdposProfile {
            width_dots: 384,
            ..p
        };
        assert_eq!(unsafe { mdpos_columns(&narrow, 1) }, 32);

        // Null and invalid profiles report 0 rather than trapping.
        assert_eq!(unsafe { mdpos_columns(std::ptr::null(), 1) }, 0);
        let bad = MdposProfile { font: 9, ..p };
        assert_eq!(unsafe { mdpos_columns(&bad, 1) }, 0);
    }

    #[test]
    fn freeing_is_idempotent_against_a_zeroed_buffer() {
        unsafe { mdpos_free(MdposBuf::EMPTY) };
    }

    #[test]
    fn version_getters_are_readable() {
        let v = unsafe { std::ffi::CStr::from_ptr(mdpos_version()) };
        assert_eq!(v.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
        assert_eq!(mdpos_format_version(), 1);
    }
}
