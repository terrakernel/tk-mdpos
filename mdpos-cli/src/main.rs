//! Command-line renderer.
//!
//! All file and stream I/O lives in this crate. The core library stays sans-IO, so this
//! binary is the only place allowed to know that files exist.
//!
//! ```text
//! mdpos template.txt > out.bin
//! mdpos --preview template.txt
//! ```

use std::io::{self, Read, Write};
use std::process::ExitCode;

use mdpos::Profile;

const USAGE: &str = "\
mdpos — render a receipt template to ESC/POS bytes

USAGE:
    mdpos <template.txt>            write ESC/POS bytes to stdout
    mdpos --preview <template.txt>  write a monospace preview to stdout

Use - as the filename to read the template from stdin.

Bytes go to stdout unredirected, which will spray control codes across your
terminal. Redirect to a file or pipe them to a printer.
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mdpos: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut preview = false;
    let mut path: Option<String> = None;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--preview" | "-p" => preview = true,
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(());
            }
            a if a.starts_with('-') && a != "-" => {
                return Err(format!("unknown option `{a}`\n\n{USAGE}").into());
            }
            a => {
                if path.replace(a.to_string()).is_some() {
                    return Err(format!("expected one template file\n\n{USAGE}").into());
                }
            }
        }
    }

    let Some(path) = path else {
        return Err(format!("no template given\n\n{USAGE}").into());
    };

    let template = if path == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?
    };

    // v0.1 has exactly one profile. A registry and TOML loading are out of scope until
    // the layout engine has proven itself.
    let profile = Profile::epson_80mm();

    let mut stdout = io::stdout().lock();
    if preview {
        stdout.write_all(mdpos::preview(&template, &profile)?.as_bytes())?;
    } else {
        stdout.write_all(&mdpos::render(&template, &profile)?)?;
    }
    stdout.flush()?;

    Ok(())
}
