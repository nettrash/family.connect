namespace FamilyConnect.App.Logic;

/// <summary>
/// An animation's canvas, frame after frame, as a viewer would show it (docs/protocol.md, "Animations": "Each frame is
/// COMPOSITED — drawn as a viewer would show it at that moment, its disposal and blending applied").
/// </summary>
/// <remarks>
/// <para>
/// <b>ONE COMPOSITOR FOR ALL THREE FORMATS.</b> A GIF image, an APNG frame and a WebP frame are each a rectangle of pixels with
/// a place on the canvas, a way of meeting what is there (<see cref="FrameBlend"/>) and something to do once its time is up
/// (<see cref="FrameDisposal"/>); the formats differ only in how they spell those, which their readers translate.
/// </para>
/// <para>
/// <b>STRAIGHT ALPHA, AND TRANSPARENT TO BEGIN WITH.</b> The canvas starts empty — every browser ignores a GIF's background
/// colour and a WebP's <c>ANIM</c> one — and "restore to background" clears a rectangle to empty again. White is put
/// under it only at the end (<see cref="AnimationRender.OnWhite"/>), so a half-transparent APNG edge drawn over another is
/// blended as the browsers blend it before anything is flattened.
/// </para>
/// <para>
/// A disposal is applied when the NEXT frame is drawn, as the formats define it: what <see cref="Pixels"/> holds after a
/// <see cref="Draw"/> is exactly what shows while that frame stands.
/// </para>
/// </remarks>
public sealed class FrameCanvas
{
    private AnimationFrame? standing;
    private byte[]? before;

    public FrameCanvas(int width, int height)
    {
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(width);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(height);
        Width = width;
        Height = height;
        Pixels = new byte[checked(width * height * 4)];
    }

    public int Width { get; }

    public int Height { get; }

    /// <summary>The canvas as straight BGRA, <c>Width × Height × 4</c>.</summary>
    public byte[] Pixels { get; }

    /// <summary>
    /// The frame before this one disposed of as it asked, then <paramref name="frame"/> drawn — its
    /// <c>Width × Height × 4</c> straight-BGRA <paramref name="pixels"/>, clipped to the canvas.
    /// </summary>
    public void Draw(AnimationFrame frame, ReadOnlySpan<byte> pixels)
    {
        if (pixels.Length != frame.Width * frame.Height * 4)
        {
            throw new ArgumentException("a frame's pixels are not the size of its rectangle", nameof(pixels));
        }
        if (standing is { } last)
        {
            switch (last.Disposal)
            {
                case FrameDisposal.Background:
                    Clear(last);
                    break;
                case FrameDisposal.Previous when before is not null:
                    Restore(last, before);
                    break;
            }
        }
        before = frame.Disposal == FrameDisposal.Previous ? Save(frame) : null;
        var (left, top, right, bottom) = Clip(frame);
        for (var y = top; y < bottom; y++)
        {
            var source = (((y - frame.Y) * frame.Width) + (left - frame.X)) * 4;
            var target = ((y * Width) + left) * 4;
            for (var x = left; x < right; x++, source += 4, target += 4)
            {
                if (frame.Blend == FrameBlend.Source)
                {
                    Pixels[target] = pixels[source];
                    Pixels[target + 1] = pixels[source + 1];
                    Pixels[target + 2] = pixels[source + 2];
                    Pixels[target + 3] = pixels[source + 3];
                    continue;
                }
                Over(pixels.Slice(source, 4), Pixels.AsSpan(target, 4));
            }
        }
        standing = frame;
    }

    /// <summary>
    /// <paramref name="source"/> drawn over <paramref name="target"/>, both straight BGRA: Porter-Duff OVER, rounded to the
    /// nearest. An opaque source replaces, an empty one changes nothing.
    /// </summary>
    public static void Over(ReadOnlySpan<byte> source, Span<byte> target)
    {
        int sa = source[3];
        if (sa == 255)
        {
            source.CopyTo(target);
            return;
        }
        if (sa == 0)
        {
            return;
        }
        int da = target[3];
        // Alpha out, times 255: sa·255 + da·(255 − sa).
        var weight = (sa * 255) + (da * (255 - sa));
        for (var channel = 0; channel < 3; channel++)
        {
            var mixed = (source[channel] * sa * 255) + (target[channel] * da * (255 - sa));
            target[channel] = (byte)((mixed + (weight / 2)) / weight);
        }
        target[3] = (byte)((weight + 127) / 255);
    }

    private (int Left, int Top, int Right, int Bottom) Clip(AnimationFrame frame) =>
        (Math.Clamp(frame.X, 0, Width),
         Math.Clamp(frame.Y, 0, Height),
         (int)Math.Clamp((long)frame.X + frame.Width, 0, Width),
         (int)Math.Clamp((long)frame.Y + frame.Height, 0, Height));

    private void Clear(AnimationFrame frame)
    {
        var (left, top, right, bottom) = Clip(frame);
        for (var y = top; y < bottom; y++)
        {
            Pixels.AsSpan(((y * Width) + left) * 4, (right - left) * 4).Clear();
        }
    }

    private byte[] Save(AnimationFrame frame)
    {
        var (left, top, right, bottom) = Clip(frame);
        var row = (right - left) * 4;
        var saved = new byte[row * (bottom - top)];
        for (var y = top; y < bottom; y++)
        {
            Pixels.AsSpan(((y * Width) + left) * 4, row).CopyTo(saved.AsSpan((y - top) * row));
        }
        return saved;
    }

    private void Restore(AnimationFrame frame, byte[] saved)
    {
        var (left, top, right, bottom) = Clip(frame);
        var row = (right - left) * 4;
        for (var y = top; y < bottom; y++)
        {
            saved.AsSpan((y - top) * row, row).CopyTo(Pixels.AsSpan(((y * Width) + left) * 4));
        }
    }
}

/// <summary>What a composited canvas becomes on its way into the encoder: on white, at the video's size, in its pixel format.</summary>
public static class AnimationRender
{
    /// <summary>Straight BGRA drawn onto <see cref="AnimationTiming.BackgroundRgb"/> white: every pixel opaque after.</summary>
    public static byte[] OnWhite(ReadOnlySpan<byte> straight)
    {
        var opaque = new byte[straight.Length];
        for (var at = 0; at + 3 < straight.Length; at += 4)
        {
            int alpha = straight[at + 3];
            var uncovered = 255 * (255 - alpha);
            opaque[at] = (byte)(((straight[at] * alpha) + uncovered + 127) / 255);
            opaque[at + 1] = (byte)(((straight[at + 1] * alpha) + uncovered + 127) / 255);
            opaque[at + 2] = (byte)(((straight[at + 2] * alpha) + uncovered + 127) / 255);
            opaque[at + 3] = 255;
        }
        return opaque;
    }

    /// <summary>
    /// Opaque BGRA from <paramref name="width"/> × <paramref name="height"/> to <paramref name="toWidth"/> ×
    /// <paramref name="toHeight"/> by AREA AVERAGING — each target pixel the mean of the source area it covers, fractions
    /// of a pixel weighed as such — which is what a downscale of a picture with hard pixel-art edges wants, and the plan
    /// never scales up. The same size comes back as a copy.
    /// </summary>
    public static byte[] Scale(ReadOnlySpan<byte> bgra, int width, int height, int toWidth, int toHeight)
    {
        if (width == toWidth && height == toHeight)
        {
            return bgra.ToArray();
        }
        var across = Weights(width, toWidth);
        var down = Weights(height, toHeight);
        // Across first: every source row to the target's width, three channels as floats.
        var rows = new float[height * toWidth * 3];
        for (var y = 0; y < height; y++)
        {
            for (var x = 0; x < toWidth; x++)
            {
                float b = 0, g = 0, r = 0;
                foreach (var (from, weight) in across[x])
                {
                    var at = ((y * width) + from) * 4;
                    b += bgra[at] * weight;
                    g += bgra[at + 1] * weight;
                    r += bgra[at + 2] * weight;
                }
                var into = ((y * toWidth) + x) * 3;
                rows[into] = b;
                rows[into + 1] = g;
                rows[into + 2] = r;
            }
        }
        var scaled = new byte[toWidth * toHeight * 4];
        for (var y = 0; y < toHeight; y++)
        {
            for (var x = 0; x < toWidth; x++)
            {
                float b = 0, g = 0, r = 0;
                foreach (var (from, weight) in down[y])
                {
                    var at = ((from * toWidth) + x) * 3;
                    b += rows[at] * weight;
                    g += rows[at + 1] * weight;
                    r += rows[at + 2] * weight;
                }
                var into = ((y * toWidth) + x) * 4;
                scaled[into] = Channel(b);
                scaled[into + 1] = Channel(g);
                scaled[into + 2] = Channel(r);
                scaled[into + 3] = 255;
            }
        }
        return scaled;

        static byte Channel(float value) => (byte)Math.Clamp((int)MathF.Round(value), 0, 255);
    }

    /// <summary>For each of <paramref name="to"/> target pixels, the source pixels it covers and how much of it each makes up.</summary>
    private static (int From, float Weight)[][] Weights(int from, int to)
    {
        var weights = new (int, float)[to][];
        var step = (double)from / to;
        for (var target = 0; target < to; target++)
        {
            var start = target * step;
            var end = (target + 1) * step;
            var list = new List<(int, float)>();
            for (var source = (int)Math.Floor(start); source < Math.Min(from, (int)Math.Ceiling(end)); source++)
            {
                var covered = Math.Min(end, source + 1) - Math.Max(start, source);
                if (covered > 0)
                {
                    list.Add((source, (float)(covered / step)));
                }
            }
            weights[target] = [.. list];
        }
        return weights;
    }

    /// <summary>
    /// Opaque BGRA as NV12 — a full-size Y plane, then interleaved Cb Cr at half size each way — in BT.709 LIMITED range, the
    /// profile's "8-bit SDR BT.709". Converted here rather than by Media Foundation's video processor, which picks the
    /// matrix by resolution and would write BT.601 under its idea of HD. <paramref name="width"/> and
    /// <paramref name="height"/> are even, as the plan makes them.
    /// </summary>
    public static byte[] Nv12(ReadOnlySpan<byte> bgra, int width, int height)
    {
        var luma = width * height;
        var nv12 = new byte[luma + (luma / 2)];
        for (var at = 0; at < luma; at++)
        {
            nv12[at] = Y(bgra[(at * 4) + 2], bgra[(at * 4) + 1], bgra[at * 4]);
        }
        var chroma = luma;
        for (var y = 0; y < height; y += 2)
        {
            for (var x = 0; x < width; x += 2)
            {
                int r = 0, g = 0, b = 0;
                foreach (var (dx, dy) in Quad)
                {
                    var at = (((y + dy) * width) + x + dx) * 4;
                    b += bgra[at];
                    g += bgra[at + 1];
                    r += bgra[at + 2];
                }
                var (cb, cr) = CbCr(r, g, b, 4);
                nv12[chroma++] = cb;
                nv12[chroma++] = cr;
            }
        }
        return nv12;
    }

    private static readonly (int, int)[] Quad = [(0, 0), (1, 0), (0, 1), (1, 1)];

    // BT.709: Kr 0.2126, Kb 0.0722. Limited range: Y 16–235 (219 steps), Cb Cr 16–240 (224) around 128. Fixed point at 2^16,
    // each chroma row's three weights summing to exactly zero so a grey is exactly 128.
    private const int YR = 11966;  // 0.2126 × 219/255 × 65536
    private const int YG = 40254;  // 0.7152 × 219/255 × 65536
    private const int YB = 4064;   // 0.0722 × 219/255 × 65536
    private const int CbR = -6596;  // −0.2126/1.8556 × 224/255 × 65536
    private const int CbG = -22189; // −0.7152/1.8556 × 224/255 × 65536
    private const int CbB = 28785;  // 0.5 × 224/255 × 65536, less the rounding of the other two
    private const int CrR = 28785;
    private const int CrG = -26145; // −0.7152/1.5748 × 224/255 × 65536
    private const int CrB = -2640;  // the rest, so the row sums to zero

    /// <summary>One pixel's BT.709 limited-range luma.</summary>
    public static byte Y(int r, int g, int b) => (byte)Math.Clamp(16 + (((YR * r) + (YG * g) + (YB * b) + 32768) >> 16), 16, 235);

    /// <summary>The BT.709 limited-range chroma of <paramref name="count"/> pixels' summed channels.</summary>
    public static (byte Cb, byte Cr) CbCr(int r, int g, int b, int count)
    {
        var scale = 65536L * count;
        var cb = 128 + Rounded((CbR * (long)r) + (CbG * (long)g) + (CbB * (long)b), scale);
        var cr = 128 + Rounded((CrR * (long)r) + (CrG * (long)g) + (CrB * (long)b), scale);
        return ((byte)Math.Clamp(cb, 16, 240), (byte)Math.Clamp(cr, 16, 240));

        static long Rounded(long value, long scale) =>
            value >= 0 ? (value + (scale / 2)) / scale : -((-value + (scale / 2)) / scale);
    }
}

/// <summary>
/// The animation's video frames, one after another: each source frame composited in turn (<see cref="FrameCanvas"/>), and
/// the canvas taken at every one the plan samples — on white, at the plan's size.
/// </summary>
/// <remarks>
/// <b>EVERY FRAME IS DRAWN, SAMPLED OR NOT.</b> A frame no tick lands on is not in the video, but what it left on the canvas
/// is: the next frame is drawn over it. So the source is walked in order, start to end, once — and only one canvas, plus
/// what a "previous" disposal keeps, is ever held, whatever the animation's length.
/// </remarks>
/// <param name="pixelsOf">
/// Source frame <c>n</c>'s own rectangle as straight BGRA — the platform's decoder, or <see cref="GifPicture.Pixels"/>. Null
/// is a frame that could not be decoded, and ends the animation as a failure (rule C).
/// </param>
public sealed class AnimationFrames(
    AnimationLayout layout,
    AnimationPlan plan,
    Func<int, CancellationToken, Task<byte[]?>> pixelsOf)
{
    private readonly FrameCanvas canvas = new(layout.Width, layout.Height);
    private int drawn = -1;
    private int next;

    /// <summary>How many video frames have been given out.</summary>
    public int Given => next;

    /// <summary>
    /// The next video frame as opaque BGRA at the plan's size, or null once every sample has been given.
    /// </summary>
    /// <exception cref="InvalidDataException">A source frame could not be decoded, or decoded at the wrong size.</exception>
    public async Task<byte[]?> NextAsync(CancellationToken cancel = default)
    {
        if (next >= plan.Samples.Count)
        {
            return null;
        }
        var wanted = plan.Samples[next].Frame;
        while (drawn < wanted)
        {
            cancel.ThrowIfCancellationRequested();
            drawn++;
            var frame = layout.Frames[drawn];
            var pixels = await pixelsOf(drawn, cancel).ConfigureAwait(false);
            if (pixels is null || pixels.Length != frame.Width * frame.Height * 4)
            {
                throw new InvalidDataException($"frame {drawn} of an animation could not be decoded at {frame.Width}x{frame.Height}");
            }
            canvas.Draw(frame, pixels);
        }
        next++;
        return AnimationRender.Scale(AnimationRender.OnWhite(canvas.Pixels), canvas.Width, canvas.Height, plan.Width, plan.Height);
    }
}
