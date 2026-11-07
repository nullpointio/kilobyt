using Kilobyt.Core;
using Xunit;

namespace Kilobyt.Core.Tests;

public class ExifOrientationTests
{
    [Fact]
    public void Orientation1_IsIdentity()
    {
        var src = TestImages.Quadrants(4, 2);
        var dst = ExifOrientation.Apply(src, 1);
        Assert.Equal(4, dst.Width);
        Assert.Equal(2, dst.Height);
        Assert.Same(src.Pixels, dst.Pixels);
    }

    [Theory]
    [InlineData(0)]
    [InlineData(9)]
    [InlineData(-3)]
    public void UnknownOrientation_IsIdentity(int orientation)
    {
        var src = TestImages.Quadrants(4, 2);
        var dst = ExifOrientation.Apply(src, orientation);
        Assert.Equal(4, dst.Width);
        Assert.Equal(2, dst.Height);
    }

    [Theory]
    [InlineData(5)]
    [InlineData(6)]
    [InlineData(7)]
    [InlineData(8)]
    public void QuarterTurns_SwapDimensions(int orientation)
    {
        var src = TestImages.Quadrants(400, 300);
        var dst = ExifOrientation.Apply(src, orientation);
        Assert.Equal(300, dst.Width);
        Assert.Equal(400, dst.Height);
    }

    [Fact]
    public void Orientation3_Rotates180()
    {
        var pixels = new byte[] { 1, 0, 0, 2, 0, 0, 3, 0, 0, 4, 0, 0 };
        var src = new Rgb24Image(2, 2, pixels);
        var dst = ExifOrientation.Apply(src, 3);
        Assert.Equal(4, dst.Pixels[0]);   // top-left becomes bottom-right
        Assert.Equal(1, dst.Pixels[9]);   // and vice versa
    }

    [Fact]
    public void Orientation6_RotatesClockwise()
    {
        // src 3x2, top-left pixel value 1 moves to the top-right of a 2x3 result.
        var pixels = new byte[3 * 2 * 3];
        pixels[0] = 1;
        var src = new Rgb24Image(3, 2, pixels);
        var dst = ExifOrientation.Apply(src, 6);
        Assert.Equal(2, dst.Width);
        Assert.Equal(3, dst.Height);
        int i = (0 * dst.Width + (dst.Width - 1)) * 3;
        Assert.Equal(1, dst.Pixels[i]);
    }

    [Fact]
    public void Orientation2_MirrorsHorizontally()
    {
        var pixels = new byte[] { 1, 0, 0, 2, 0, 0 };
        var src = new Rgb24Image(2, 1, pixels);
        var dst = ExifOrientation.Apply(src, 2);
        Assert.Equal(2, dst.Pixels[0]);
        Assert.Equal(1, dst.Pixels[3]);
    }

    [Fact]
    public void PortraitWithOrientation6_BecomesTaller() // AC-4.3 shape rule
    {
        var src = TestImages.Noise(400, 300, 7);
        var dst = ExifOrientation.Apply(src, 6);
        Assert.True(dst.Width < dst.Height);
    }
}
