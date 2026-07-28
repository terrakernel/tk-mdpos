/*
 * tk_mdpos — turn a formatted template string into ESC/POS bytes.
 *
 * Hand-written to match tk-mdpos-ffi/src/lib.rs. If you change one, change the other;
 * the Rust side has tests that pin this contract.
 *
 * Three rules:
 *
 *   1. Every buffer handed out must be released with tk_mdpos_free(), including the ones
 *      returned alongside an error. It was allocated by Rust's allocator and only
 *      tk_mdpos_free() may release it — never free().
 *
 *   2. Buffers are always NUL-terminated at ptr[len], and len never counts that byte.
 *      ESC/POS output contains embedded zero bytes (GS V 66 0 ends in one), so printing
 *      output with %s truncates it — but can never read past the allocation. Use len.
 *
 *   3. On any error the out-buffer receives a human-readable UTF-8 message. Template
 *      errors carry the source line, and that text is meant for whoever edits the
 *      template — do not discard it in favour of the status code alone.
 *
 * This library performs no I/O. It produces bytes; delivering them to a printer is the
 * caller's job. Queueing, chunking, retries and status polling are all out of scope.
 */

#ifndef TK_MDPOS_H
#define TK_MDPOS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* --- status codes ---------------------------------------------------------------- */

#define TK_MDPOS_OK                   0
#define TK_MDPOS_ERR_TEMPLATE        -1  /* template rejected; out-buffer holds the message */
#define TK_MDPOS_ERR_INVALID_UTF8    -2  /* template bytes were not valid UTF-8            */
#define TK_MDPOS_ERR_NULL_ARG        -3  /* a required pointer was null                    */
#define TK_MDPOS_ERR_INVALID_PROFILE -4  /* a profile field held an undefined value        */
#define TK_MDPOS_ERR_PANIC          -99  /* caught at the boundary; this is an mdpos bug   */

/* --- types ----------------------------------------------------------------------- */

/*
 * An owned buffer produced by mdpos. Release with tk_mdpos_free().
 * Treat `cap` as opaque and do not modify any field.
 */
typedef struct {
    uint8_t *ptr;  /* bytes; NUL-terminated at ptr[len] */
    size_t   len;  /* length excluding the terminator   */
    size_t   cap;  /* opaque; required by tk_mdpos_free()  */
} TkMdposBuf;

/*
 * Printer description.
 *
 * Start from tk_mdpos_profile_epson_80mm() and adjust fields, rather than filling this in
 * from scratch — that keeps unknown-value errors impossible as the enums grow.
 */
typedef struct {
    uint8_t  dialect;               /* 0 = Epson                                   */
    uint16_t width_dots;            /* 576 = 80mm, 384 = 58mm; must be non-zero    */
    uint8_t  font;                  /* 0 = A (12 dots/char), 1 = B (9 dots/char)   */
    uint8_t  code_page;             /* 0 = CP437                                   */
    bool     supports_partial_cut;  /* whether GS V 66 is honored                  */
} TkMdposProfile;

/* --- profiles -------------------------------------------------------------------- */

/* The v0.1 default: 80mm, 576 dots, Font A, CP437, Epson. */
TkMdposProfile tk_mdpos_profile_epson_80mm(void);

/*
 * Characters per line at magnification `mag` (1..=8). Returns 0 for a null or invalid
 * profile. Needed by hosts that draw their own preview, so nothing hardcodes 48.
 */
uint16_t tk_mdpos_columns(const TkMdposProfile *profile, uint8_t mag);

/* --- rendering ------------------------------------------------------------------- */

/*
 * Render a template to ESC/POS bytes.
 *
 * `template` is UTF-8 of `template_len` bytes and need not be NUL-terminated. On success
 * `out` receives the bytes; on any error it receives a UTF-8 message. Either way `out`
 * must be released with tk_mdpos_free().
 *
 * Returns TK_MDPOS_OK or one of the TK_MDPOS_ERR_* codes.
 */
int32_t tk_mdpos_render(const uint8_t     *template,
                     size_t             template_len,
                     const TkMdposProfile *profile,
                     TkMdposBuf           *out);

/* As tk_mdpos_render(), but the buffer holds a UTF-8 monospace preview. */
int32_t tk_mdpos_preview(const uint8_t     *template,
                      size_t             template_len,
                      const TkMdposProfile *profile,
                      TkMdposBuf           *out);

/*
 * Release a buffer produced by this library. Safe on a zeroed buffer, and required even
 * after an error, since error messages come back through the same buffer.
 */
void tk_mdpos_free(TkMdposBuf buf);

/* --- versions -------------------------------------------------------------------- */

/*
 * The highest template format version this build implements. A template declaring
 * {v N} above this is rejected.
 *
 * Record this alongside stored templates: the *string* carries the compatibility
 * promise, not the library version.
 */
uint32_t tk_mdpos_format_version(void);

/* This library's version, as a static NUL-terminated string. Never free it. */
const char *tk_mdpos_version(void);

#ifdef __cplusplus
}  /* extern "C" */
#endif

#endif /* TK_MDPOS_H */
