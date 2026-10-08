// The screen kept on while somebody records (docs/audio-video-messages-2026-10-04.md, S1.7): an idle timer that turned the
// display off halfway through a long story would lock the session, and a lock stops the recording.
using System.Runtime.InteropServices;
using Windows.System.Display;

namespace FamilyConnect.App.Services;

/// <summary>
/// Keeps the display — and with it the computer — awake while held: Windows' <c>DisplayRequest</c>, and where that throws,
/// the window thread's own execution state.
/// </summary>
/// <remarks>
/// <para>
/// <b>GUARDED, BECAUSE IT HAS THROWN HERE BEFORE.</b> <c>DisplayRequest.RequestActive</c> once threw a <c>COMException</c>
/// under the Windows App SDK (microsoft/WindowsAppSDK #3002, closed as fixed in 2025). Should it still, the request is
/// made with <c>SetThreadExecutionState</c> instead — continuous on THIS thread, which is why both halves must be called
/// on the window's thread — and the failure is written down. Neither has been run: trial T7.
/// </para>
/// <para>
/// <b>BALANCED.</b> A release with nothing held does nothing, and a second hold is the same hold: <c>RequestRelease</c>
/// without a matching request throws.
/// </para>
/// </remarks>
internal sealed class KeepAwake
{
    private const uint EsContinuous = 0x80000000;
    private const uint EsSystemRequired = 0x00000001;
    private const uint EsDisplayRequired = 0x00000002;

    private DisplayRequest? request;
    private bool held;
    private bool byExecutionState;

    /// <summary>Keep the screen on until <see cref="Release"/>.</summary>
    public void Hold()
    {
        if (held)
        {
            return;
        }
        try
        {
            request ??= new DisplayRequest();
            request.RequestActive();
            held = true;
            return;
        }
        catch (Exception e)
        {
            Diagnostics.Write($"keeping the screen on: DisplayRequest {e.GetType().Name} 0x{e.HResult:X8}");
        }
        if (SetThreadExecutionState(EsContinuous | EsSystemRequired | EsDisplayRequired) != 0)
        {
            held = true;
            byExecutionState = true;
        }
        else
        {
            Diagnostics.Write("keeping the screen on: the execution state was refused too");
        }
    }

    /// <summary>Let the screen sleep again.</summary>
    public void Release()
    {
        if (!held)
        {
            return;
        }
        held = false;
        if (byExecutionState)
        {
            byExecutionState = false;
            SetThreadExecutionState(EsContinuous);
            return;
        }
        try
        {
            request?.RequestRelease();
        }
        catch (Exception e)
        {
            Diagnostics.Write($"letting the screen sleep: {e.GetType().Name} 0x{e.HResult:X8}");
        }
    }

    [DllImport("kernel32.dll")]
    private static extern uint SetThreadExecutionState(uint flags);
}
