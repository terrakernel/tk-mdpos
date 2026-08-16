//! Command-line renderer.
//!
//! All file and stream I/O lives in this crate. The core library stays sans-IO, so this
//! binary is the only place allowed to know that files exist.
//!
//! ```text
//! mdpos template.txt > out.bin
//! mdpos --preview template.txt
//! mdpos --html template.txt > preview.html
//! mdpos --html --watch template.txt -o preview.html
//! ```

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use tk_mdpos::Profile;

const USAGE: &str = "\
mdpos — render a receipt template to ESC/POS bytes

USAGE:
    mdpos <template.txt>            write ESC/POS bytes to stdout
    mdpos --preview <template.txt>  write a monospace preview to stdout
    mdpos --html <template.txt>     write an HTML preview to stdout

OPTIONS:
    -o, --out <file>   write to a file instead of stdout
    -w, --watch        re-render whenever the template changes; Ctrl-C to stop
    -h, --help         this text

Use - as the filename to read the template from stdin.

--preview is the faster loop while editing a template; --html shows emphasis and
magnification at their real size, for approving a layout or showing someone who is
not at a terminal.

    mdpos --html --watch receipt.tmpl -o preview.html

Open that file in a browser and leave it open: it reloads itself, so editing the
template updates the receipt with no rebuild. A template that fails to render leaves
the error on the page rather than blanking it.

Bytes go to stdout unredirected, which will spray control codes across your
terminal. Redirect to a file, use -o, or pipe them to a printer. On PowerShell prefer
-o: its redirection re-encodes the stream and corrupts binary output.
";

/// How often the watcher re-stats the template.
///
/// Polling rather than a filesystem-notification crate, which is a deviation from the
/// original estimate of "one file-watcher dependency". Two reasons: this keeps the CLI at
/// zero dependencies, and re-stat'ing the path by name is *more* robust for this exact
/// case — editors that save atomically write a temporary file and rename it over the
/// original, which breaks a watch registered against the original inode.
const POLL: Duration = Duration::from_millis(200);

/// How often the generated page reloads itself, in seconds.
///
/// A `meta refresh` rather than a websocket or an injected live-reload client, because
/// those need a server and running one is not a decision this tool gets to make on its
/// own. The page is small and local, so a periodic reload is cheap.
const RELOAD_SECS: u32 = 1;

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
    let mut output = Output::Bytes;
    let mut path: Option<String> = None;
    let mut out: Option<String> = None;
    let mut watch = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--preview" | "-p" => output = Output::Preview,
            "--html" => output = Output::Html,
            "--watch" | "-w" => watch = true,
            "--out" | "-o" => {
                let Some(v) = args.next() else {
                    return Err(format!("{arg} needs a filename\n\n{USAGE}").into());
                };
                out = Some(v);
            }
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

    // v0.1 has exactly one profile. A registry and TOML loading are out of scope until
    // the layout engine has proven itself.
    let profile = Profile::epson_80mm();

    if !watch {
        return render_once(&path, output, out.as_deref(), &profile);
    }

    // --- watch mode -------------------------------------------------------------------
    //
    // Rejected up front rather than discovered later: there is nothing to re-read from a
    // pipe, and rewriting a binary document in place helps nobody.
    if path == "-" {
        return Err("--watch needs a file to watch, not stdin".into());
    }
    match output {
        Output::Bytes => {
            return Err("--watch applies to --preview or --html, not to byte output".into());
        }
        Output::Html if out.is_none() => {
            return Err(
                "--html --watch needs -o <file>, so a browser has something to keep open\n\n\
                 e.g. mdpos --html --watch receipt.tmpl -o preview.html"
                    .into(),
            );
        }
        _ => {}
    }

    watch_loop(&path, output, out.as_deref(), &profile)
}

/// Render once and exit. The behaviour the CLI has always had.
fn render_once(
    path: &str,
    output: Output,
    out: Option<&str>,
    profile: &Profile,
) -> Result<(), Box<dyn std::error::Error>> {
    let template = read_template(path)?;

    let bytes = match output {
        Output::Bytes => tk_mdpos::render(&template, profile)?,
        Output::Preview => tk_mdpos::preview(&template, profile)?.into_bytes(),
        Output::Html => tk_mdpos::preview_html(&template, profile)?.into_bytes(),
    };

    match out {
        Some(file) => std::fs::write(file, &bytes).map_err(|e| format!("{file}: {e}"))?,
        None => {
            let mut stdout = io::stdout().lock();
            stdout.write_all(&bytes)?;
            stdout.flush()?;
        }
    }

    Ok(())
}

/// Re-render on every change until interrupted.
///
/// A render failure is reported and the loop continues: a template being edited is
/// invalid half the time, and exiting on the first unbalanced brace would make the mode
/// useless.
fn watch_loop(
    path: &str,
    output: Output,
    out: Option<&str>,
    profile: &Profile,
) -> Result<(), Box<dyn std::error::Error>> {
    // Fail fast if the path is wrong, rather than polling a typo forever.
    std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;

    match out {
        Some(file) => eprintln!("mdpos: watching {path} -> {file}; open it and leave it open"),
        None => eprintln!("mdpos: watching {path}"),
    }

    let mut last: Option<SystemTime> = None;
    loop {
        // A failed stat is the gap during an atomic save, not an error. Skipping keeps
        // `last` untouched, so the write that follows the rename still registers.
        if let Ok(stamp) = std::fs::metadata(path).and_then(|m| m.modified()) {
            if last != Some(stamp) {
                last = Some(stamp);
                rerender(path, output, out, profile);
            }
        }
        std::thread::sleep(POLL);
    }
}

/// One pass of the watch loop. Never fails the process — it reports and returns.
fn rerender(path: &str, output: Output, out: Option<&str>, profile: &Profile) {
    let rendered = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|template| match output {
            Output::Preview => tk_mdpos::preview(&template, profile).map_err(|e| e.to_string()),
            // Wrapped here, not in the core: `preview_html` returns a clean fragment and
            // has no business knowing about reloading. Framing is this binary's job.
            Output::Html => tk_mdpos::preview_html(&template, profile)
                .map(|f| document(&f, path))
                .map_err(|e| e.to_string()),
            Output::Bytes => unreachable!("rejected before the loop starts"),
        });

    match (rendered, out) {
        (Ok(text), Some(file)) => match std::fs::write(file, &text) {
            Ok(()) => eprintln!("mdpos: {} rendered", stamp()),
            Err(e) => eprintln!("mdpos: {file}: {e}"),
        },
        (Ok(text), None) => {
            // No screen clearing: ANSI is not honoured on every Windows console and
            // printing raw escapes into someone's terminal is worse than scrolling.
            // Scrollback also lets successive renders be compared, which suits the
            // monospace preview's job as a diff tool.
            println!("\n=== {} — {path} {}", stamp(), "=".repeat(20));
            print!("{text}");
            let _ = io::stdout().flush();
        }
        (Err(message), Some(file)) => {
            eprintln!("mdpos: {message}");
            // Leave the reason on the page. Blanking it, or leaving the last good render
            // up, both suggest the edit was fine.
            let _ = std::fs::write(file, error_document(&message, path));
        }
        (Err(message), None) => {
            println!("\n=== {} — {path} {}", stamp(), "=".repeat(20));
            println!("{message}");
        }
    }
}

/// `HH:MM:SS` from the wall clock, without pulling in a date library.
fn stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60)
}

/// Wrap a preview fragment in a self-reloading document.
fn document(fragment: &str, path: &str) -> String {
    format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta http-equiv=\"refresh\" content=\"{RELOAD_SECS}\">\n\
         <title>{title} — mdpos</title>\n\
         <style>{STYLE}</style>\n\
         </head>\n\
         <body>\n\
         {fragment}\n\
         <p class=\"mdpos-watch-note\">watching {title} — reloads every {RELOAD_SECS}s</p>\n\
         <script>{SCROLL}</script>\n\
         </body>\n\
         </html>\n",
        title = escape(path),
    )
}

/// The same document, reporting why the template did not render.
fn error_document(message: &str, path: &str) -> String {
    format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta http-equiv=\"refresh\" content=\"{RELOAD_SECS}\">\n\
         <title>error — mdpos</title>\n\
         <style>{STYLE}</style>\n\
         </head>\n\
         <body>\n\
         <pre class=\"mdpos-watch-error\">{message}</pre>\n\
         <p class=\"mdpos-watch-note\">watching {title} — reloads every {RELOAD_SECS}s</p>\n\
         <script>{SCROLL}</script>\n\
         </body>\n\
         </html>\n",
        message = escape(message),
        title = escape(path),
    )
}

/// Page chrome only. The fragment carries its own scoped styles and is not touched.
const STYLE: &str = "\
body{margin:0;padding:2rem;background:#f4f4f5;display:flex;flex-direction:column;\
align-items:center;gap:1rem;font-family:system-ui,sans-serif}\
.mdpos-watch-note{margin:0;color:#71717a;font-size:.8rem}\
.mdpos-watch-error{margin:0;padding:1rem 1.25rem;max-width:60ch;background:#fef2f2;\
color:#991b1b;border-left:3px solid #dc2626;border-radius:3px;font-size:.85rem;\
white-space:pre-wrap;overflow-wrap:anywhere}";

/// A meta refresh resets the scroll position, which is irritating on a long receipt.
const SCROLL: &str = "\
addEventListener('beforeunload',()=>sessionStorage.setItem('mdpos-y',scrollY));\
const y=sessionStorage.getItem('mdpos-y');if(y)scrollTo(0,+y);";

/// Minimal HTML escaping for text this binary injects: a template path and an error
/// message, both of which can legitimately contain `<`, `&` or a quote.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn read_template(path: &str) -> Result<String, Box<dyn std::error::Error>> {
    if path == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        return Ok(buf);
    }
    Ok(std::fs::read_to_string(Path::new(path)).map_err(|e| format!("{path}: {e}"))?)
}

/// Which backend to run. The last flag on the command line wins, which is the ordinary
/// shell convention and avoids inventing an error for `--preview --html`.
#[derive(Clone, Copy)]
enum Output {
    Bytes,
    Preview,
    Html,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_the_characters_that_break_markup() {
        assert_eq!(escape("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
        assert_eq!(escape("plain"), "plain");
    }

    /// The wrapper must not disturb the fragment: the two preview backends are asserted
    /// against each other by the golden fixtures, and a document that mangled the
    /// fragment would slip past those entirely.
    #[test]
    fn the_document_embeds_the_fragment_verbatim() {
        let fragment = tk_mdpos::preview_html("{center}\nHI\n{cut}", &Profile::epson_80mm())
            .expect("fixture template renders");
        let page = document(&fragment, "receipt.tmpl");

        assert!(page.contains(&fragment), "fragment was altered");
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("http-equiv=\"refresh\""));
    }

    /// An error message carries the offending line and is written for whoever edits the
    /// template, so it has to survive into the page rather than being swallowed.
    #[test]
    fn the_error_document_carries_the_message() {
        let err = tk_mdpos::render("{cols 20,6:r}\nItem | 1.250.000", &Profile::epson_80mm())
            .expect_err("right-column overflow is rejected");
        let page = error_document(&err.to_string(), "receipt.tmpl");

        assert!(page.contains("line 2"));
        assert!(page.contains("overflows right-aligned column"));
        assert!(page.contains("http-equiv=\"refresh\""), "must keep retrying");
    }

    #[test]
    fn a_path_containing_markup_cannot_break_out_of_the_page() {
        let page = document("<div></div>", "<script>alert(1)</script>.tmpl");
        assert!(!page.contains("<script>alert(1)</script>"));
        assert!(page.contains("&lt;script&gt;"));
    }

    #[test]
    fn the_clock_is_wall_time_of_day() {
        let s = stamp();
        assert_eq!(s.len(), 8, "{s}");
        assert!(s.as_bytes()[2] == b':' && s.as_bytes()[5] == b':', "{s}");
    }
}
