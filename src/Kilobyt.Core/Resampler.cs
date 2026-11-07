namespace Kilobyt.Core;

/// <summary>
/// PRD §5.2.5 — area-average (box) downscale. Each destination pixel averages the source
/// rectangle it covers; integer accumulation, round-half-up. No bicubic, no sharpening.
/// </summary>
public static class Resampler
{
    public static Rgb24Image Downscale(Rgb24Image src, int w, int h)
    {
        if (src.Width < 1 || src.Height < 1)
            throw new ArgumentException("Source image has no pixels.", nameof(src));
        if (src.Pixels is null || src.Pixels.LongLength != (long)src.Width * src.Height * 3)
            throw new ArgumentException("Source pixel buffer does not match its dimensions.", nameof(src));
        if (w < 1) throw new ArgumentOutOfRangeException(nameof(w), "Width must be at least 1.");
        if (h < 1) throw new ArgumentOutOfRangeException(nameof(h), "Height must be at least 1.");
        if (w > src.Width || h > src.Height)
            throw new ArgumentOutOfRangeException(nameof(w), "Downscale never upscales (PRD §5.2.4).");

        if (w == src.Width && h == src.Height) return src;

        int sw = src.Width, sh = src.Height;
        byte[] s = src.Pixels!;
        byte[] d = new byte[w * h * 3];

        // Source column/row boundaries: destination pixel i covers [xs[i], xs[i+1]).
        int[] xs = new int[w + 1];
        int[] ys = new int[h + 1];
        for (int i = 0; i <= w; i++) xs[i] = (int)((long)i * sw / w);
        for (int j = 0; j <= h; j++) ys[j] = (int)((long)j * sh / h);
        xs[w] = sw;
        ys[h] = sh;

        for (int y = 0; y < h; y++)
        {
            int y0 = ys[y], y1 = ys[y + 1];
            if (y1 <= y0) y1 = y0 + 1;
            for (int x = 0; x < w; x++)
            {
                int x0 = xs[x], x1 = xs[x + 1];
                if (x1 <= x0) x1 = x0 + 1;

                long r = 0, g = 0, b = 0;
                long count = 0;
                for (int sy = y0; sy < y1; sy++)
                {
                    int row = sy * sw * 3;
                    for (int sx = x0; sx < x1; sx++)
                    {
                        int idx = row + sx * 3;
                        r += s[idx];
                        g += s[idx + 1];
                        b += s[idx + 2];
                        count++;
                    }
                }

                int o = (y * w + x) * 3;
                d[o] = (byte)((r + count / 2) / count);
                d[o + 1] = (byte)((g + count / 2) / count);
                d[o + 2] = (byte)((b + count / 2) / count);
            }
        }

        return new Rgb24Image(w, h, d);
    }
}
