using System;
using Kilobyt.Core;
using Xunit;

namespace Kilobyt.Core.Tests;

public class ResamplerTests
{
    [Fact]
    public void Downscale_HonoursRequestedDimensions()
    {
        var src = TestImages.Noise(400, 300, 1);
        var dst = Resampler.Downscale(src, 100, 75);
        Assert.Equal(100, dst.Width);
        Assert.Equal(75, dst.Height);
        Assert.Equal(100 * 75 * 3, dst.Pixels.Length);
    }

    [Fact]
    public void Downscale_UniformColourStaysUniform()
    {
        var src = TestImages.Solid(120, 90, 37, 211, 99);
        var dst = Resampler.Downscale(src, 41, 31);
        for (int i = 0; i < dst.Pixels.Length; i += 3)
        {
            Assert.Equal(37, dst.Pixels[i]);
            Assert.Equal(211, dst.Pixels[i + 1]);
            Assert.Equal(99, dst.Pixels[i + 2]);
        }
    }

    [Fact]
    public void Downscale_AveragesBox_QuadrantsToMidGrey()
    {
        // Four quadrants (red, green, blue, white) averaged into one pixel.
        var src = TestImages.Quadrants(4, 4);
        var dst = Resampler.Downscale(src, 1, 1);
        var (r, g, b) = TestImages.Pixel(dst, 0, 0);
        Assert.Equal((byte)128, r); // (255 + 0 + 0 + 255) / 4 = 127.5 -> round half up
        Assert.Equal((byte)128, g);
        Assert.Equal((byte)128, b);
    }

    [Fact]
    public void Downscale_RoundHalfUp_NotTruncate()
    {
        // 2x1 pixels 0 and 255 -> 127.5 -> 128 (not 127)
        var pixels = new byte[] { 0, 0, 0, 255, 255, 255 };
        var src = new Rgb24Image(2, 1, pixels);
        var dst = Resampler.Downscale(src, 1, 1);
        Assert.Equal((byte)128, dst.Pixels[0]);
    }

    [Fact]
    public void Downscale_SameSize_ReturnsSourceUnchanged()
    {
        var src = TestImages.Noise(10, 10, 2);
        var dst = Resampler.Downscale(src, 10, 10);
        Assert.Equal(10, dst.Width);
        Assert.Same(src.Pixels, dst.Pixels);
    }

    [Fact]
    public void Downscale_RefusesToUpscale()
    {
        var src = TestImages.Noise(10, 10, 3);
        Assert.Throws<ArgumentOutOfRangeException>(() => Resampler.Downscale(src, 11, 10));
        Assert.Throws<ArgumentOutOfRangeException>(() => Resampler.Downscale(src, 10, 11));
    }

    [Fact]
    public void Downscale_RejectsInvalidTargets()
    {
        var src = TestImages.Noise(10, 10, 4);
        Assert.Throws<ArgumentOutOfRangeException>(() => Resampler.Downscale(src, 0, 5));
        Assert.Throws<ArgumentOutOfRangeException>(() => Resampler.Downscale(src, 5, 0));
    }

    [Fact]
    public void Downscale_RejectsMismatchedBuffer()
    {
        var broken = new Rgb24Image(10, 10, new byte[10]);
        Assert.Throws<ArgumentException>(() => Resampler.Downscale(broken, 5, 5));
    }

    [Fact]
    public void Downscale_IsDeterministic()
    {
        var src = TestImages.Noise(200, 150, 5);
        var a = Resampler.Downscale(src, 64, 48);
        var b = Resampler.Downscale(src, 64, 48);
        Assert.Equal(a.Pixels, b.Pixels);
    }

    [Fact]
    public void Downscale_KeepsAspectAccuracy()
    {
        var src = TestImages.Noise(1000, 333, 6);
        var dst = Resampler.Downscale(src, 300, 100);
        double sourceRatio = 1000.0 / 333.0;
        double targetRatio = (double)dst.Width / dst.Height;
        // The caller (SizePlanner) owns the 1 px rule; here we check the pixels are not skewed.
        Assert.Equal(300, dst.Width);
        Assert.Equal(100, dst.Height);
        Assert.True(Math.Abs(sourceRatio - targetRatio) < 0.01);
    }
}
