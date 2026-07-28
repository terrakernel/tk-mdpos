/*
 * C smoke test. This is the only thing that verifies include/mdpos.h actually matches
 * the compiled ABI — the Rust tests exercise the contract, but they cannot catch a
 * header that has drifted from it.
 *
 * Not part of `cargo test` (that would mean a build script and a `cc` dependency for
 * everyone). Run it by hand:
 *
 *     cargo build -p mdpos-ffi
 *     cc -Wall -Wextra -Imdpos-ffi/include mdpos-ffi/tests/smoke.c \
 *        target/debug/libmdpos.a -o /tmp/mdpos-smoke && /tmp/mdpos-smoke
 */

#include <stdio.h>
#include <string.h>

#include "mdpos.h"

static int failures = 0;

static void check(int ok, const char *what) {
    printf("%s  %s\n", ok ? "ok  " : "FAIL", what);
    if (!ok) failures++;
}

int main(void) {
    MdposProfile profile = mdpos_profile_epson_80mm();

    check(mdpos_columns(&profile, 1) == 48, "80mm profile is 48 columns");
    check(mdpos_columns(&profile, 2) == 24, "magnification halves the grid");
    check(mdpos_format_version() == 1, "format version is 1");
    check(mdpos_version() != NULL && mdpos_version()[0] != '\0', "version string present");

    /* --- rendering --- */
    const char *tmpl =
        "{v 1}\n"
        "{center}\n"
        "{size 2x2}TOKO MAJU\n"
        "{size 1x1}\n"
        "{left}\n"
        "{cols 20,10:r,12:r}\n"
        "Nasi Goreng | 2 x 25.000 | 50.000\n"
        "{cut}\n";

    MdposBuf out;
    int32_t code = mdpos_render((const uint8_t *)tmpl, strlen(tmpl), &profile, &out);

    check(code == MDPOS_OK, "template renders");
    check(out.ptr != NULL && out.len > 0, "bytes returned");
    check(out.ptr[0] == 0x1B && out.ptr[1] == 0x40, "document starts with ESC @");
    check(out.ptr[out.len] == 0, "buffer is NUL-terminated past len");
    /* GS V 66 0 — the trailing zero is why output is not a C string. */
    check(out.len >= 4 && out.ptr[out.len - 4] == 0x1D && out.ptr[out.len - 3] == 0x56 &&
              out.ptr[out.len - 2] == 0x42 && out.ptr[out.len - 1] == 0x00,
          "document ends with a partial cut");
    check(strlen((const char *)out.ptr) < out.len, "output contains embedded NULs");
    mdpos_free(out);

    /* --- preview --- */
    code = mdpos_preview((const uint8_t *)tmpl, strlen(tmpl), &profile, &out);
    check(code == MDPOS_OK, "template previews");
    check(strstr((const char *)out.ptr, "TOKO MAJU") != NULL, "preview contains the header");
    check(strstr((const char *)out.ptr, "50.000") != NULL, "preview contains the price");
    mdpos_free(out);

    /* --- a rejected template must explain itself --- */
    const char *bad = "{cols 20,6:r}\nItem | 1.250.000\n";
    code = mdpos_render((const uint8_t *)bad, strlen(bad), &profile, &out);
    check(code == MDPOS_ERR_TEMPLATE, "overflowing right column is rejected");
    check(out.ptr != NULL, "rejection still returns a buffer");
    check(strstr((const char *)out.ptr, "line 2") != NULL, "message carries the line number");
    printf("      message: %s\n", (const char *)out.ptr);
    mdpos_free(out);

    /* --- invalid inputs --- */
    MdposProfile broken = mdpos_profile_epson_80mm();
    broken.font = 9;
    code = mdpos_render((const uint8_t *)tmpl, strlen(tmpl), &broken, &out);
    check(code == MDPOS_ERR_INVALID_PROFILE, "unknown font is rejected");
    mdpos_free(out);

    code = mdpos_render(NULL, 0, &profile, &out);
    check(code == MDPOS_ERR_NULL_ARG, "null template is rejected");
    mdpos_free(out);

    /* Freeing a zeroed buffer must be safe — callers will do this after an early exit. */
    MdposBuf zeroed = {0};
    mdpos_free(zeroed);
    check(1, "freeing a zeroed buffer is safe");

    printf("\n%s\n", failures ? "FAILURES" : "all checks passed");
    return failures != 0;
}
