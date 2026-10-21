namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// An animated sticker's frames as this client draws them in a chat (<see cref="StickerFrames"/>): composited from the file's
/// own frames, kept transparent, premultiplied, at the size drawn. Before this, Windows Imaging was asked for the frames and
/// handed out one, so an animated sticker was drawn still.
/// </summary>
public sealed class StickerFramesTests
{
    // Straight BGRA, one pixel each.
    private static readonly byte[] Red = [0, 0, 255, 255];
    private static readonly byte[] Blue = [255, 0, 0, 255];
    private static readonly byte[] Clear = [0, 0, 0, 0];

    private static byte[] Pixels(params byte[][] each) => [.. each.SelectMany(pixel => pixel)];

    private static Func<int, CancellationToken, Task<byte[]?>> From(params byte[]?[] frames) =>
        (index, _) => Task.FromResult(frames[index]);

    /// <summary>Opaque stays itself, transparent stays transparent, and half-covered is premultiplied.</summary>
    [Fact]
    public void PixelsArePremultipliedAndTransparencyIsKept()
    {
        var halfRed = new byte[] { 0, 0, 255, 128 };
        var drawn = StickerFrames.Premultiplied(Pixels(Red, Clear, halfRed), 3, 1, 3, 1);
        Assert.Equal(Pixels(Red, Clear, [0, 0, 128, 128]), drawn);
    }

    /// <summary>
    /// A transparent pixel's colour does not bleed into the edge when the sticker is scaled down: the green of an invisible
    /// pixel next to red would come out a muddy dark edge if straight pixels were averaged.
    /// </summary>
    [Fact]
    public void ScalingDownBlendsNoInvisibleColourIntoTheEdge()
    {
        var invisibleGreen = new byte[] { 0, 255, 0, 0 };
        var drawn = StickerFrames.Premultiplied(Pixels(Red, invisibleGreen), 2, 1, 1, 1);
        Assert.Equal(0, drawn[1]);                  // no green
        Assert.Equal(128, drawn[2]);                // half the red
        Assert.Equal(128, drawn[3]);                // half covered
    }

    /// <summary>Premultiplied BGRA never holds a colour above its alpha, whatever the rounding did.</summary>
    [Fact]
    public void NoColourIsLeftAboveItsAlpha()
    {
        var faint = new byte[] { 255, 255, 255, 1 };
        var drawn = StickerFrames.Premultiplied(Pixels(faint, faint, faint), 3, 1, 2, 1);
        for (var at = 0; at < drawn.Length; at += 4)
        {
            Assert.True(drawn[at] <= drawn[at + 3] && drawn[at + 1] <= drawn[at + 3] && drawn[at + 2] <= drawn[at + 3]);
        }
    }

    /// <summary>
    /// Every frame is the CANVAS at that moment, not the file's rectangle: a second frame that only paints the right pixel
    /// leaves the first frame's left pixel standing.
    /// </summary>
    [Fact]
    public async Task EachFrameIsTheWholeCanvasAsAViewerShowsIt()
    {
        var layout = new AnimationLayout(AnimationFormat.Apng, 2, 1,
        [
            new AnimationFrame(0, 0, 2, 1, 100, FrameBlend.Source, FrameDisposal.None),
            new AnimationFrame(1, 0, 1, 1, 100, FrameBlend.Over, FrameDisposal.None),
        ]);
        var drawn = await StickerFrames.ComposeAsync(layout, From(Pixels(Red, Clear), Blue), 2, 1);
        Assert.NotNull(drawn);
        Assert.Equal(2, drawn.Count);
        Assert.Equal(Pixels(Red, Clear), drawn[0]);
        Assert.Equal(Pixels(Red, Blue), drawn[1]);
    }

    /// <summary>"Restore to background" clears the frame's rectangle to transparent before the next is drawn.</summary>
    [Fact]
    public async Task ADisposedFrameLeavesTransparencyNotWhite()
    {
        var layout = new AnimationLayout(AnimationFormat.WebP, 2, 1,
        [
            new AnimationFrame(0, 0, 1, 1, 100, FrameBlend.Source, FrameDisposal.Background),
            new AnimationFrame(1, 0, 1, 1, 100, FrameBlend.Source, FrameDisposal.None),
        ]);
        var drawn = await StickerFrames.ComposeAsync(layout, From(Red, Blue), 2, 1);
        Assert.Equal(Pixels(Clear, Blue), drawn![1]);
    }

    /// <summary>A frame that will not decode is not drawn around: the whole animation is answered null and drawn by the old way.</summary>
    [Fact]
    public async Task AFrameThatWillNotDecodeGivesNoFrames()
    {
        var layout = new AnimationLayout(AnimationFormat.Apng, 1, 1,
        [
            new AnimationFrame(0, 0, 1, 1, 100, FrameBlend.Source, FrameDisposal.None),
            new AnimationFrame(0, 0, 1, 1, 100, FrameBlend.Source, FrameDisposal.None),
        ]);
        Assert.Null(await StickerFrames.ComposeAsync(layout, From(Red, null), 1, 1));
        Assert.Null(await StickerFrames.ComposeAsync(layout, From(Red, Pixels(Red, Red)), 1, 1));
    }

    /// <summary>Frames come out at the size the sticker is drawn, not the file's.</summary>
    [Fact]
    public async Task FramesComeOutAtTheDrawnSize()
    {
        var layout = new AnimationLayout(AnimationFormat.Apng, 4, 4,
        [
            new AnimationFrame(0, 0, 4, 4, 100, FrameBlend.Source, FrameDisposal.None),
            new AnimationFrame(0, 0, 4, 4, 100, FrameBlend.Source, FrameDisposal.None),
        ]);
        var full = Pixels([.. Enumerable.Repeat(Red, 16)]);
        var drawn = await StickerFrames.ComposeAsync(layout, From(full, full), 2, 2);
        Assert.All(drawn!, frame => Assert.Equal(2 * 2 * 4, frame.Length));
        Assert.Equal(Pixels(Red, Red, Red, Red), drawn![0]);
    }
}
