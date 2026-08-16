using System.Runtime.InteropServices;
using System.Text;

using static TerraKernel.Mdpos.NativeMethods;

namespace TerraKernel.Mdpos;

/// <summary>
/// Turns a formatted template string into ESC/POS bytes.
/// </summary>
/// <remarks>
/// <para>
/// Layout lives in a string that can sit in a database row, so changing a receipt footer
/// is a data edit rather than a redeploy.
/// </para>
/// <para>
/// This library performs no I/O. It produces bytes; delivering them to a printer is yours,
/// as are queueing, chunking, retries, status polling and job atomicity. That is deliberate
/// — the largest target hardware (Sunmi, iMin, Telpo on Android) exposes only
/// <c>sendRAWData(byte[])</c>, and producing bytes is the only thing that works everywhere.
/// </para>
/// <para>
/// Every method here is thread-safe and reentrant, and a single <see cref="PrinterProfile"/>
/// may be shared by concurrent callers. The native library holds no global state, so this
/// wrapper adds no lock.
/// </para>
/// </remarks>
public static class Mdpos
{
    private enum Backend
    {
        Escpos,
        Text,
        Html,
    }

    /// <summary>
    /// Renders a template to ESC/POS bytes, ready to send to the printer verbatim.
    /// </summary>
    /// <param name="template">The template source, in mdpos template syntax.</param>
    /// <param name="profile">The target printer.</param>
    /// <returns>
    /// A self-contained document: it begins with <c>ESC @</c> and ends with a feed and a
    /// cut, and assumes nothing about prior device state. A lost or duplicated document
    /// therefore cannot corrupt the next one.
    /// </returns>
    /// <exception cref="ArgumentNullException"><paramref name="template"/> is null.</exception>
    /// <exception cref="MdposException">The template was rejected.</exception>
    /// <remarks>
    /// The result contains embedded zero bytes — <c>GS V 66 0</c> ends in one — so it is
    /// not a C string and must not be treated as text.
    /// </remarks>
    public static byte[] Render(string template, in PrinterProfile profile) =>
        Execute(Backend.Escpos, template, profile);

    /// <summary>
    /// Renders a template to a monospace plaintext preview.
    /// </summary>
    /// <remarks>
    /// The developer's diff tool and the faster loop while editing. For showing a person
    /// what the paper will look like, prefer <see cref="PreviewHtml"/>.
    /// </remarks>
    /// <exception cref="ArgumentNullException"><paramref name="template"/> is null.</exception>
    /// <exception cref="MdposException">The template was rejected.</exception>
    public static string Preview(string template, in PrinterProfile profile) =>
        Encoding.UTF8.GetString(Execute(Backend.Text, template, profile));

    /// <summary>
    /// Renders a template to a self-contained HTML preview fragment.
    /// </summary>
    /// <returns>
    /// One <c>&lt;div&gt;</c> carrying its own scoped <c>&lt;style&gt;</c>, so it renders
    /// standalone and cannot be clobbered by host CSS. Hand it to a WebView or embed it.
    /// </returns>
    /// <exception cref="ArgumentNullException"><paramref name="template"/> is null.</exception>
    /// <exception cref="MdposException">The template was rejected.</exception>
    /// <remarks>
    /// <para>
    /// Fidelity is resemblance, not pixel accuracy: the printer's font is not available to
    /// a browser. That is sufficient because the preview is not the safety net for fit —
    /// overflow is already wrapped or rejected before this is reached, so nothing can
    /// silently run off the paper edge in a document that renders at all.
    /// </para>
    /// <para>
    /// Nothing is drawn that the library is guessing at. A QR code appears as a
    /// correctly-sized empty square with its payload on a <c>data-mdpos-qr</c> attribute,
    /// so a host with its own encoder can draw the real symbol; a plausible-looking but
    /// fake QR would invite someone to point a phone at it.
    /// </para>
    /// </remarks>
    public static string PreviewHtml(string template, in PrinterProfile profile) =>
        Encoding.UTF8.GetString(Execute(Backend.Html, template, profile));

    /// <summary>
    /// The highest template format version this build implements. A template declaring
    /// <c>{v N}</c> above this is rejected.
    /// </summary>
    /// <remarks>
    /// Record this alongside stored templates. The <em>string</em> carries the
    /// compatibility promise, not the library version: <c>{v 1}</c> is honoured forever,
    /// and any change that would alter how an existing v1 template renders is a bug rather
    /// than a release note.
    /// </remarks>
    public static int FormatVersion => (int)tk_mdpos_format_version();

    /// <summary>The native library's version.</summary>
    public static unsafe string Version =>
        Marshal.PtrToStringUTF8((IntPtr)tk_mdpos_version()) ?? string.Empty;

    /// <summary>
    /// Shared body for the three backends: marshal in, copy out, free exactly once.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The native buffer never escapes this method, which is what makes the
    /// free-exactly-once rule structural rather than something callers have to honour. A
    /// <c>SafeHandle</c> would be the reflex here and does not fit: <c>tk_mdpos_free</c>
    /// takes the buffer by value because it needs <c>ptr</c>, <c>len</c> and <c>cap</c> to
    /// rebuild the Rust allocation, while a SafeHandle tracks a single pointer. Since the
    /// buffer's lifetime is strictly bounded by this call, <c>try/finally</c> covers it
    /// completely.
    /// </para>
    /// <para>
    /// The two text backends pay one extra array copy by decoding from the returned
    /// <c>byte[]</c>. That is deliberate: a single exit path for the free obligation is
    /// worth more than a copy of a few kilobytes.
    /// </para>
    /// </remarks>
    private static unsafe byte[] Execute(Backend backend, string template, in PrinterProfile profile)
    {
        ArgumentNullException.ThrowIfNull(template);

        byte[] utf8 = Encoding.UTF8.GetBytes(template);

        // `fixed` on a zero-length array yields a null pointer, which the ABI would report
        // as a null-argument error. An empty template is legal input, so give it somewhere
        // real to point and keep the length at zero.
        byte[] pinned = utf8.Length == 0 ? new byte[1] : utf8;

        fixed (byte* src = pinned)
        fixed (PrinterProfile* prof = &profile)
        {
            TkMdposBuf buf = default;
            int status = backend switch
            {
                Backend.Escpos => tk_mdpos_render(src, (nuint)utf8.Length, prof, &buf),
                Backend.Text => tk_mdpos_preview(src, (nuint)utf8.Length, prof, &buf),
                Backend.Html => tk_mdpos_preview_html(src, (nuint)utf8.Length, prof, &buf),
                _ => throw new ArgumentOutOfRangeException(nameof(backend)),
            };

            try
            {
                // On any failure the same buffer carries a UTF-8 message instead of output.
                // Template errors name the source line, and that text is for whoever edits
                // the template, so it must not be discarded in favour of the status code.
                if (status != TK_MDPOS_OK)
                {
                    string message = buf.Ptr is null
                        ? $"mdpos failed with status {status} and no message"
                        : Marshal.PtrToStringUTF8((IntPtr)buf.Ptr, checked((int)buf.Len));

                    throw new MdposException(status, message);
                }

                // Length-based copy, never PtrToStringUTF8 on this path: ESC/POS output
                // contains embedded NULs and treating it as a C string truncates the
                // receipt at the first one.
                var result = new byte[checked((int)buf.Len)];
                if (result.Length > 0)
                {
                    Marshal.Copy((IntPtr)buf.Ptr, result, 0, result.Length);
                }

                return result;
            }
            finally
            {
                // Required even after an error, since the message came back through the
                // same buffer. Only tk_mdpos_free may release it — never free().
                tk_mdpos_free(buf);
            }
        }
    }
}
