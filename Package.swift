// swift-tools-version: 5.9

// Swift package for the mdpos C ABI.
//
// This manifest sits at the repository root because SwiftPM requires it there — a package
// dependency is a git URL with no subpath, so there is no way to put it in a subdirectory.
// It shares the repo with the Rust workspace deliberately: the header is hand-written, and
// keeping the manifest beside it means the checksum below always refers to an artifact
// built from this same commit. A separate Swift repo would reintroduce exactly the drift
// that tests/smoke.c exists to catch, at a level nothing checks.
//
// The binary is not in git. It is a release asset, and the URL is deterministic for a given
// tag, which is what removes the chicken-and-egg between checksumming and publishing.
//
// Both must be updated together at every release, and the zip that is uploaded must be the
// one that was checksummed — a rebuild produces different bytes and resolution then fails
// with a checksum mismatch.

import PackageDescription

let package = Package(
    name: "TkMdpos",
    // At or above the floor the binaries were actually built with (iOS 10, macOS 11), which
    // is the safe direction: declaring lower than the binary supports fails at link time.
    platforms: [
        .macOS(.v11),
        .iOS(.v13),
    ],
    products: [
        .library(name: "TkMdpos", targets: ["TkMdpos"]),
    ],
    targets: [
        // Static-library XCFramework: universal macOS, iOS device, and a universal iOS
        // simulator. Built by tk-mdpos-ffi/build-xcframework.sh, which prints this checksum.
        .binaryTarget(
            name: "TkMdpos",
            url: "https://github.com/terrakernel/tk-mdpos/releases/download/v0.3.0/TkMdpos.xcframework.zip",
            checksum: "c3d05242c505940ba255c4aecdcf8bc2c28c5aac638f23be0ad46baf26b4a2f7"
        ),
    ]
)
