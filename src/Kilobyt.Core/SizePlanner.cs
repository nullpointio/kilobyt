namespace Kilobyt.Core;

/// <summary>PRD §5.2.4 — fit-within planning. Never upscales.</summary>
public static class SizePlanner
{
    public static (int W, int H) Fit(int w, int h, int? maxW, int? maxH)
    {
        if (w < 1) throw new ArgumentOutOfRangeException(nameof(w), "Width must be at least 1.");
        if (h < 1) throw new ArgumentOutOfRangeException(nameof(h), "Height must be at least 1.");

        double scale = 1.0;
        if (maxW is > 0) scale = Math.Min(scale, (double)maxW.Value / w);
        if (maxH is > 0) scale = Math.Min(scale, (double)maxH.Value / h);
        scale = Math.Min(scale, 1.0);

        int newW = Clamp((int)Math.Round(w * scale, MidpointRounding.AwayFromZero), 1, w);
        int newH = Clamp((int)Math.Round(h * scale, MidpointRounding.AwayFromZero), 1, h);
        return (newW, newH);
    }

    private static int Clamp(int value, int min, int max) => value < min ? min : value > max ? max : value;
}
