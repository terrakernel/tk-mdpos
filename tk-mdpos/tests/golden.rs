//! Golden-file tests.
//!
//! Both backends snapshot from the same fixture, so the preview cannot drift away from
//! the bytes without a test noticing.
//!
//! Fixtures live at the repository root rather than under this crate: the directory is
//! the seed of the conformance corpus and of the customer-facing compatibility test, and
//! it is structured as if it will be published even though publishing is out of v0.1
//! scope.
//!
//! To regenerate after an intentional change:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test --test golden
//! ```
//!
//! Then read the diff. A changed `.bin` is a v1 compatibility break until you have
//! convinced yourself otherwise — deployed templates must render identically forever.

use std::path::{Path, PathBuf};

use tk_mdpos::Profile;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate is nested in the workspace")
        .join("tests/golden")
}

fn updating() -> bool {
    std::env::var_os("UPDATE_GOLDEN").is_some()
}

#[test]
fn golden_fixtures() {
    let dir = corpus_dir();
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();

    let mut failures = Vec::new();

    for case in &cases {
        let name = case.file_name().unwrap().to_string_lossy().to_string();

        let template = std::fs::read_to_string(case.join("input.tmpl"))
            .unwrap_or_else(|e| panic!("{name}: input.tmpl: {e}"));

        // A fixture without profile.ron uses the v0.1 default, which keeps the common
        // 80mm case from repeating itself in every directory.
        let profile = match std::fs::read_to_string(case.join("profile.ron")) {
            Ok(src) => ron::from_str::<Profile>(&src)
                .unwrap_or_else(|e| panic!("{name}: profile.ron: {e}")),
            Err(_) => Profile::epson_80mm(),
        };

        // A fixture with expected.err asserts a rejection instead of an output. Refusing
        // a template is a feature — a wrapped total is worse than a failed render — so
        // the corpus has to be able to pin the refusals too.
        let expected_err = case.join("expected.err");
        if expected_err.exists() {
            let want = std::fs::read_to_string(&expected_err).unwrap();
            match tk_mdpos::render(&template, &profile) {
                Ok(_) => failures.push(format!(
                    "{name}: expected the template to be rejected, but it rendered\n  wanted: {}",
                    want.trim()
                )),
                Err(e) if e.to_string().trim() != want.trim() => failures.push(format!(
                    "{name}: wrong rejection\n  expected: {}\n  actual:   {e}",
                    want.trim()
                )),
                Err(_) => {}
            }
            continue;
        }

        let bytes = tk_mdpos::render(&template, &profile)
            .unwrap_or_else(|e| panic!("{name}: render failed: {e}"));
        let text = tk_mdpos::preview(&template, &profile)
            .unwrap_or_else(|e| panic!("{name}: preview failed: {e}"));

        if updating() {
            std::fs::write(case.join("expected.bin"), &bytes).unwrap();
            std::fs::write(case.join("expected.txt"), &text).unwrap();
            continue;
        }

        match std::fs::read(case.join("expected.bin")) {
            Ok(want) if want != bytes => failures.push(format!(
                "{name}: bytes differ\n  expected {} bytes: {}\n  actual   {} bytes: {}",
                want.len(),
                hex(&want),
                bytes.len(),
                hex(&bytes),
            )),
            Err(e) => failures.push(format!("{name}: expected.bin: {e}")),
            _ => {}
        }

        match std::fs::read_to_string(case.join("expected.txt")) {
            Ok(want) if want != text => failures.push(format!(
                "{name}: preview differs\n--- expected ---\n{want}--- actual ---\n{text}"
            )),
            Err(e) => failures.push(format!("{name}: expected.txt: {e}")),
            _ => {}
        }
    }

    assert!(
        failures.is_empty(),
        "{} golden fixture(s) failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );

    if updating() {
        eprintln!("rewrote {} fixture(s) — review the diff", cases.len());
    }
}

/// Hex dump for failure output. Reading raw bytes off a panic message is bad enough
/// without them being escaped as a Rust byte-string literal.
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
