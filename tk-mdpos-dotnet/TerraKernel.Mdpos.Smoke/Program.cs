using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;

using TerraKernel.Mdpos;

// Deliberately a console program rather than a test framework, mirroring
// tk-mdpos-ffi/tests/smoke.c. Two reasons:
//
//   - it needs no package restore, so it runs in a clean container;
//   - the same source can be pointed at either the ProjectReference or the packed
//     .nupkg, and consuming the packed package from a throwaway project is the only
//     thing that actually proves runtimes/{rid}/native/ resolves. `dotnet pack`
//     succeeding proves nothing.

int failures = 0;

void Check(string what, bool ok, string? detail = null)
{
    Console.WriteLine(ok ? $"ok    {what}" : $"FAIL  {what}");
    if (!ok)
    {
        failures++;
        if (detail is not null) Console.WriteLine($"      {detail}");
    }
}

// --- the ABI struct layout is load-bearing ---------------------------------------------
// PrinterProfile is passed straight across the boundary with no marshalling step, so its
// size has to match the C struct: uint8, uint16 (aligned to 2), uint8, uint8, bool.

Check("PrinterProfile is 8 bytes", Marshal.SizeOf<PrinterProfile>() == 8,
    $"got {Marshal.SizeOf<PrinterProfile>()}");

// --- profile ---------------------------------------------------------------------------

var profile = PrinterProfile.Epson80mm;

Check("80mm profile is 48 columns", profile.ColumnsAt(1) == 48, $"got {profile.ColumnsAt(1)}");
Check("magnification halves the grid", profile.ColumnsAt(2) == 24, $"got {profile.ColumnsAt(2)}");
Check("58mm profile is 32 columns", profile.WithWidthDots(384).ColumnsAt(1) == 32);
Check("font B is 64 columns", profile.WithFont(PrinterFont.B).ColumnsAt(1) == 64);
Check("profile round-trips its fields",
    profile is { WidthDots: 576, Font: PrinterFont.A, CodePage: CodePage.Cp437, Dialect: Dialect.Epson }
    && profile.SupportsPartialCut);

Check("format version is 1", Mdpos.FormatVersion == 1, $"got {Mdpos.FormatVersion}");
Check("version string present", !string.IsNullOrEmpty(Mdpos.Version));
Console.WriteLine($"      native version: {Mdpos.Version}");

// --- rendering ---------------------------------------------------------------------------

const string Template = """
    {cols 20,10:r,12:r}
    Nasi Goreng | 2 x 25.000 | 50.000
    {cut}
    """;

byte[] bytes = Mdpos.Render(Template, profile);

Check("bytes returned", bytes.Length > 0);
Check("document starts with ESC @", bytes.Length >= 2 && bytes[0] == 0x1B && bytes[1] == 0x40);
Check("document ends with a partial cut",
    bytes.Length >= 4
    && bytes[^4] == 0x1D && bytes[^3] == 0x56 && bytes[^2] == 0x42 && bytes[^1] == 0x00);

// This is the trap the wrapper exists to avoid: ESC/POS output contains embedded NULs, so
// marshalling it as a C string truncates the receipt at the first GS V 66 0. If this
// assertion holds, Render used a length-based copy.
Check("output contains embedded NULs", Array.IndexOf(bytes, (byte)0) >= 0);

string preview = Mdpos.Preview(Template, profile);
Check("preview contains the item", preview.Contains("Nasi Goreng"));
// The row is 20 + 10 + 12 = 42 columns wide, not the paper's full 48: a {cols} spec need
// not fill the line. Right-alignment lands the price flush at the end of its own column.
Check("preview right-aligns the price at the column edge",
    preview.Split('\n').Any(l => l.TrimEnd() is { Length: 42 } line && line.EndsWith("50.000")));

string html = Mdpos.PreviewHtml(Template, profile);
Check("html preview is a scoped fragment", html.Contains("<style") && html.TrimStart().StartsWith("<div"));
Check("html preview has the item", html.Contains("Nasi Goreng"));

// --- errors -------------------------------------------------------------------------------
// The message is the useful half of a rejection: it names the source line and is written
// for whoever edits the template. A bare status code would discard it.

try
{
    Mdpos.Render("{cols 20,6:r}\nItem | 1.250.000", profile);
    Check("overflowing right column is rejected", false, "it rendered instead");
}
catch (MdposException e)
{
    Check("overflowing right column is rejected", true);
    Check("error is categorised as a template error", e.Error == MdposError.Template, $"got {e.Error}");
    Check("message carries the line number", e.Message.Contains("line 2"), e.Message);
    Console.WriteLine($"      message: {e.Message}");
}

try
{
    Mdpos.Render("HI", profile.WithFont((PrinterFont)9));
    Check("unknown font is rejected", false, "it rendered instead");
}
catch (MdposException e)
{
    Check("unknown font is rejected", e.Error == MdposError.InvalidProfile, $"got {e.Error}");
}

try
{
    Mdpos.Render(null!, profile);
    Check("null template throws ArgumentNullException", false, "it rendered instead");
}
catch (ArgumentNullException)
{
    Check("null template throws ArgumentNullException", true);
}

// An empty template is legal input. It is worth pinning because `fixed` on a zero-length
// array yields a null pointer, which the ABI would otherwise report as a null argument.
try
{
    byte[] empty = Mdpos.Render("", profile);
    Check("empty template renders", empty.Length > 0);
}
catch (Exception e)
{
    Check("empty template renders", false, e.Message);
}

// --- thread safety --------------------------------------------------------------------------
// The ABI documents that entry points are reentrant and one profile may be shared by
// concurrent callers. The wrapper adds no lock, so that promise is the thing under test.

try
{
    byte[] want = Mdpos.Render(Template, profile);
    bool consistent = true;

    Parallel.For(0, 200, _ =>
    {
        byte[] got = Mdpos.Render(Template, profile);
        if (!got.AsSpan().SequenceEqual(want)) Volatile.Write(ref consistent, false);
    });

    Check("concurrent renders agree", consistent);
}
catch (Exception e)
{
    Check("concurrent renders agree", false, e.Message);
}

// --- the conformance corpus ---------------------------------------------------------------
// The strongest check available: render the golden fixtures through the whole stack —
// C# wrapper, P/Invoke, C ABI, Rust engine — and compare against bytes that a real Epson
// TM-T82X has printed correctly. Anything wrong in the marshalling shows up here as a
// byte difference rather than as a plausible-looking receipt.

string? repoRoot = Assembly.GetExecutingAssembly()
    .GetCustomAttributes<AssemblyMetadataAttribute>()
    .FirstOrDefault(a => a.Key == "RepoRoot")?.Value;

string corpus = Path.Combine(repoRoot ?? ".", "tests", "golden");

if (!Directory.Exists(corpus))
{
    Console.WriteLine($"\nskip  golden corpus not found at {corpus} (expected when run against a packed package)");
}
else
{
    Console.WriteLine();
    foreach (string dir in Directory.GetDirectories(corpus).OrderBy(d => d))
    {
        string name = Path.GetFileName(dir);
        string templatePath = Path.Combine(dir, "input.tmpl");
        if (!File.Exists(templatePath)) continue;

        // Fixtures with a profile.ron use a non-default profile. Parsing RON from C# is
        // not worth it here; the Rust golden test already covers those, and this harness
        // is checking the marshalling rather than the engine.
        if (File.Exists(Path.Combine(dir, "profile.ron")))
        {
            Console.WriteLine($"skip  {name} (non-default profile)");
            continue;
        }

        string src = File.ReadAllText(templatePath);
        string errPath = Path.Combine(dir, "expected.err");

        if (File.Exists(errPath))
        {
            string wantErr = File.ReadAllText(errPath).Trim();
            try
            {
                Mdpos.Render(src, profile);
                Check($"{name}: rejected", false, "it rendered instead");
            }
            catch (MdposException e)
            {
                Check($"{name}: rejection matches", e.Message.Trim() == wantErr,
                    $"expected: {wantErr}\n      actual:   {e.Message}");
            }
            continue;
        }

        byte[] wantBytes = File.ReadAllBytes(Path.Combine(dir, "expected.bin"));
        byte[] gotBytes = Mdpos.Render(src, profile);
        Check($"{name}: bytes match the corpus", wantBytes.AsSpan().SequenceEqual(gotBytes),
            $"expected {wantBytes.Length} bytes, got {gotBytes.Length}");

        // The fixtures are stored with LF. Compare against LF so the check does not depend
        // on how the repository happened to be checked out.
        string wantText = File.ReadAllText(Path.Combine(dir, "expected.txt")).Replace("\r\n", "\n");
        Check($"{name}: preview matches the corpus",
            Mdpos.Preview(src, profile).Replace("\r\n", "\n") == wantText);

        string wantHtml = File.ReadAllText(Path.Combine(dir, "expected.html")).Replace("\r\n", "\n");
        Check($"{name}: html matches the corpus",
            Mdpos.PreviewHtml(src, profile).Replace("\r\n", "\n") == wantHtml);
    }
}

Console.WriteLine();
if (failures == 0)
{
    Console.WriteLine("all checks passed");
    return 0;
}

Console.WriteLine($"{failures} check(s) failed");
return 1;
