using System.Runtime.InteropServices;

namespace TerraKernel.Mdpos;

/// <summary>Command dialect. Only Epson is implemented.</summary>
/// <remarks>
/// "Standard ESC/POS" is not a real standard — it is Epson's proprietary command set,
/// copied to varying degrees, with no spec body and no certification. This library targets
/// genuine Epson and does not chase clones; a vendor quirk is the caller's problem, and
/// <c>{raw HEX}</c> is the escape hatch for it.
/// </remarks>
public enum Dialect : byte
{
    /// <summary>Epson ESC/POS.</summary>
    Epson = 0,
}

/// <summary>Device font, which fixes the character cell and therefore the column grid.</summary>
public enum PrinterFont : byte
{
    /// <summary>Font A — 12 dots per character. 48 columns on 80mm paper.</summary>
    A = 0,

    /// <summary>Font B — 9 dots per character. 64 columns on 80mm paper.</summary>
    B = 1,
}

/// <summary>Character encoding for the printer's text mode.</summary>
public enum CodePage : byte
{
    /// <summary>CP437.</summary>
    Cp437 = 0,
}

/// <summary>
/// Describes the target printer. Layout is resolved against this, so the same template
/// renders correctly on 58mm and 80mm without being rewritten.
/// </summary>
/// <remarks>
/// <para>
/// This type is the native <c>TkMdposProfile</c> struct: it is blittable and passed
/// straight across the ABI with no marshalling step. Field order and widths are load
/// bearing and pinned by a test.
/// </para>
/// <para>
/// Start from <see cref="Epson80mm"/> and adjust with the <c>With…</c> methods rather than
/// constructing one field by field. That keeps unknown-value errors impossible as the
/// enums grow.
/// </para>
/// <para>
/// A single profile may be shared by concurrent callers. It is read-only input and the
/// native library never writes through it.
/// </para>
/// </remarks>
[StructLayout(LayoutKind.Sequential)]
public readonly struct PrinterProfile
{
    // Explicit backing fields rather than auto-properties: the runtime only guarantees
    // declared order for fields, and this struct's layout has to match the C header.
    private readonly byte _dialect;
    private readonly ushort _widthDots;
    private readonly byte _font;
    private readonly byte _codePage;
    private readonly byte _supportsPartialCut;

    private PrinterProfile(
        byte dialect, ushort widthDots, byte font, byte codePage, byte supportsPartialCut)
    {
        _dialect = dialect;
        _widthDots = widthDots;
        _font = font;
        _codePage = codePage;
        _supportsPartialCut = supportsPartialCut;
    }

    /// <summary>The default profile: 80mm, 576 dots, Font A, CP437, Epson, partial cut.</summary>
    public static PrinterProfile Epson80mm => NativeMethods.tk_mdpos_profile_epson_80mm();

    /// <summary>Command dialect.</summary>
    public Dialect Dialect => (Dialect)_dialect;

    /// <summary>Printable width in dots. 576 = 80mm, 384 = 58mm.</summary>
    public ushort WidthDots => _widthDots;

    /// <summary>Device font.</summary>
    public PrinterFont Font => (PrinterFont)_font;

    /// <summary>Text-mode code page.</summary>
    public CodePage CodePage => (CodePage)_codePage;

    /// <summary>
    /// Whether the mechanism honours <c>GS V 66</c>. When false, a partial cut falls back
    /// to a full cut rather than emitting a command the mechanism would ignore.
    /// </summary>
    public bool SupportsPartialCut => _supportsPartialCut != 0;

    /// <summary>Returns a copy with a different printable width in dots.</summary>
    public PrinterProfile WithWidthDots(ushort widthDots) =>
        new(_dialect, widthDots, _font, _codePage, _supportsPartialCut);

    /// <summary>Returns a copy using a different device font.</summary>
    public PrinterProfile WithFont(PrinterFont font) =>
        new(_dialect, _widthDots, (byte)font, _codePage, _supportsPartialCut);

    /// <summary>Returns a copy using a different code page.</summary>
    public PrinterProfile WithCodePage(CodePage codePage) =>
        new(_dialect, _widthDots, _font, (byte)codePage, _supportsPartialCut);

    /// <summary>Returns a copy with the partial-cut capability set.</summary>
    public PrinterProfile WithPartialCut(bool supported) =>
        new(_dialect, _widthDots, _font, _codePage, (byte)(supported ? 1 : 0));

    /// <summary>
    /// Characters per line at the given magnification, so nothing has to hardcode 48.
    /// </summary>
    /// <param name="magnification">Character magnification, 1 to 8.</param>
    /// <returns>
    /// The column count, or 0 if this profile holds a value the native library does not
    /// define.
    /// </returns>
    /// <remarks>
    /// Magnification mutates the grid: <c>{size 2x2}</c> halves 48 columns to 24. Any host
    /// drawing its own preview must track the current width rather than a document
    /// constant.
    /// </remarks>
    public unsafe ushort ColumnsAt(byte magnification)
    {
        PrinterProfile copy = this;
        return NativeMethods.tk_mdpos_columns(&copy, magnification);
    }
}
