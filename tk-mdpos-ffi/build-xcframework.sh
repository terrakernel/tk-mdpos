#!/usr/bin/env bash
#
# Build TkMdpos.xcframework for the Apple platforms.
#
# Three slices, which is the set Xcode needs to build and run an app without the consumer
# choosing an architecture by hand:
#
#   macos-arm64_x86_64          universal, for a Mac app or a command-line host
#   ios-arm64                   device
#   ios-arm64_x86_64-simulator  universal, so the simulator works on both Mac generations
#
# staticlib rather than cdylib: iOS will not load an arbitrary .dylib, and a static archive
# is one artifact with no runtime lookup. The library exports eight `tk_mdpos_*` symbols and
# has no dependencies beyond libSystem, so there is nothing to bundle alongside it.
#
# This is deliberately not wired into `cargo build`. It shells out to lipo and xcodebuild,
# runs only on macOS, and producing a distributable framework is a release step rather than
# part of the ordinary edit-compile loop.
#
# Requires Xcode (not just the Command Line Tools) for xcodebuild -create-xcframework.

set -euo pipefail

cd "$(dirname "$0")/.."

readonly OUT="target/TkMdpos.xcframework"
readonly STAGE="target/xcframework-stage"

readonly MACOS_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
readonly IOS_TARGETS=(aarch64-apple-ios)
readonly SIM_TARGETS=(aarch64-apple-ios-sim x86_64-apple-ios)
readonly ALL_TARGETS=("${MACOS_TARGETS[@]}" "${IOS_TARGETS[@]}" "${SIM_TARGETS[@]}")

# Fail with the fix rather than with a linker error three steps later.
missing=()
installed="$(rustup target list --installed)"
for t in "${ALL_TARGETS[@]}"; do
    grep -qx "$t" <<<"$installed" || missing+=("$t")
done
if (( ${#missing[@]} )); then
    echo "missing rust targets: ${missing[*]}" >&2
    echo "install with: rustup target add ${missing[*]}" >&2
    exit 1
fi

for t in "${ALL_TARGETS[@]}"; do
    echo "building $t"
    cargo build --release -p tk-mdpos-ffi --target "$t"
done

rm -rf "$STAGE" "$OUT"
mkdir -p "$STAGE"/{macos,ios,sim,Headers}

# Every slice carries the same headers. The modulemap is what lets Swift say
# `import TkMdpos` instead of dropping to a bridging header.
cp tk-mdpos-ffi/include/tk_mdpos.h tk-mdpos-ffi/include/module.modulemap "$STAGE/Headers/"

# Combine the per-architecture archives, then drop debug symbols.
#
# The `strip = "debuginfo"` in the workspace profile does *not* reach a staticlib: cargo
# implements it as a link-time rustc flag and an ar archive is never linked. The DWARF is
# std's, bundled in from its precompiled objects, and it is roughly a third of the payload:
# the zipped xcframework measured 28 MB before this and 20 MB after. `-S` removes debug
# symbols only; stripping
# globals would take the tk_mdpos_* entry points with them and the archive would link to
# nothing. The smoke-test link at the end of this script is what catches that if it is ever
# changed to a broader flag.
fatten() {
    local dest="$1"; shift
    local inputs=()
    for t in "$@"; do inputs+=("target/$t/release/libtk_mdpos.a"); done
    lipo -create "${inputs[@]}" -output "$dest"
    strip -S "$dest"
}

fatten "$STAGE/macos/libtk_mdpos.a" "${MACOS_TARGETS[@]}"
fatten "$STAGE/ios/libtk_mdpos.a"   "${IOS_TARGETS[@]}"
fatten "$STAGE/sim/libtk_mdpos.a"   "${SIM_TARGETS[@]}"

xcodebuild -create-xcframework \
    -library "$STAGE/macos/libtk_mdpos.a" -headers "$STAGE/Headers" \
    -library "$STAGE/ios/libtk_mdpos.a"   -headers "$STAGE/Headers" \
    -library "$STAGE/sim/libtk_mdpos.a"   -headers "$STAGE/Headers" \
    -output "$OUT"

# A framework that merely assembled is not a framework that links. Building the existing C
# smoke test against the macOS slice is what proves the header and the archive inside the
# bundle actually work together — the other two slices cannot be run here, and xcodebuild's
# own platform classification is the evidence for those.
echo
echo "verifying the macOS slice by linking tests/smoke.c against it"
cc -Wall -Wextra -I"$OUT/macos-arm64_x86_64/Headers" \
    "$OUT/macos-arm64_x86_64/libtk_mdpos.a" \
    tk-mdpos-ffi/tests/smoke.c -o "$STAGE/smoke"
"$STAGE/smoke" >/dev/null

# Zip and checksum here rather than by hand, because both details bite quietly.
#
# `ditto --sequesterRsrc` is what Apple's docs show, but it writes a __MACOSX/ directory
# beside the framework; a static-library xcframework has no resource forks or symlinks for
# it to preserve, so it is pure noise. Plain `zip -r` from inside target/ puts the bundle at
# the archive root, which is where binaryTarget expects it.
#
# The checksum belongs to *these bytes*. Rebuilding after computing it produces a different
# archive and Package.swift silently stops matching, so the two steps stay adjacent and the
# zip that is uploaded must be this one.
rm -f "$OUT.zip"
( cd "$(dirname "$OUT")" && zip -qr "$(basename "$OUT").zip" "$(basename "$OUT")" )

echo
echo "wrote $OUT"
for d in "$OUT"/*/; do
    printf '  %-30s %s\n' "$(basename "$d")" "$(lipo -archs "$d/libtk_mdpos.a")"
done
echo
echo "wrote $OUT.zip ($(du -h "$OUT.zip" | cut -f1))"
echo "  checksum $(swift package compute-checksum "$OUT.zip")"
echo
echo "Upload THIS zip. A rebuild invalidates the checksum above."
