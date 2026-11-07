namespace Kilobyt.Core;

/// <summary>
/// PRD §5.2.3 — bake EXIF orientation 1-8 into the pixels so the output needs no
/// orientation tag. Values outside 1-8 are treated as 1 (no transform).
/// </summary>
public static class ExifOrientation
{
    public static Rgb24Image Apply(Rgb24Image src, int orientation)
    {
        if (orientation < 1 || orientation > 8) return src;
        if (orientation == 1) return src;
        if (src.Width < 1 || src.Height < 1 || src.Pixels is null)
            throw new ArgumentException("Source image has no pixels.", nameof(src));

        int w = src.Width;
        int h = src.Height;
        byte[] s = src.Pixels;
        bool swap = orientation == 5 || orientation == 6 || orientation == 7 || orientation == 8;
        int dw = swap ? h : w;
        int dh = swap ? w : h;
        byte[] d = new byte[(long)dw * dh * 3];

        for (int y = 0; y < dh; y++)
        {
            for (int x = 0; x < dw; x++)
            {
                int sx;
                int sy;
                switch (orientation)
                {
                    case 2: sx = w - 1 - x; sy = y; break;
                    case 3: sx = w - 1 - x; sy = h - 1 - y; break;
                    case 4: sx = x; sy = h - 1 - y; break;
                    case 5: sx = y; sy = x; break;
                    case 6: sx = y; sy = h - 1 - x; break;
                    case 7: sx = w - 1 - y; sy = h - 1 - x; break;
                    case 8: sx = w - 1 - y; sy = x; break;
                    default: sx = x; sy = y; break;
                }

                int si = (sy * w + sx) * 3;
                int di = (y * dw + x) * 3;
                d[di] = s[si];
                d[di + 1] = s[si + 1];
                d[di + 2] = s[si + 2];
            }
        }

        return new Rgb24Image(dw, dh, d);
    }
}
