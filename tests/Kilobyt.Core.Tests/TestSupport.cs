using System;
using System.Collections.Generic;
using Kilobyt.Core;

namespace Kilobyt.Core.Tests;

/// <summary>Deterministic fixtures — no binary files checked in (PRD §6).</summary>
public static class TestImages
{
    public static Rgb24Image Noise(int w, int h, int seed)
    {
        var pixels = new byte[(long)w * h * 3];
        var rng = new Random(seed);
        rng.NextBytes(pixels);
        return new Rgb24Image(w, h, pixels);
    }

    public static Rgb24Image Solid(int w, int h, byte r, byte g, byte b)
    {
        var pixels = new byte[(long)w * h * 3];
        for (int i = 0; i < pixels.Length; i += 3)
        {
            pixels[i] = r;
            pixels[i + 1] = g;
            pixels[i + 2] = b;
        }
        return new Rgb24Image(w, h, pixels);
    }

    /// <summary>Four distinct quadrants: TL red, TR green, BL blue, BR white.</summary>
    public static Rgb24Image Quadrants(int w, int h)
    {
        var pixels = new byte[(long)w * h * 3];
        for (int y = 0; y < h; y++)
        {
            for (int x = 0; x < w; x++)
            {
                int i = (y * w + x) * 3;
                bool left = x < w / 2;
                bool top = y < h / 2;
                byte r = 0, g = 0, b = 0;
                if (top && left) { r = 255; }
                else if (top) { g = 255; }
                else if (left) { b = 255; }
                else { r = 255; g = 255; b = 255; }
                pixels[i] = r;
                pixels[i + 1] = g;
                pixels[i + 2] = b;
            }
        }
        return new Rgb24Image(w, h, pixels);
    }

    public static (byte R, byte G, byte B) Pixel(Rgb24Image img, int x, int y)
    {
        int i = (y * img.Width + x) * 3;
        return (img.Pixels[i], img.Pixels[i + 1], img.Pixels[i + 2]);
    }
}

/// <summary>
/// Fake codec: encoded length is a pure function of pixel count and quality
/// (pixels * 3 * q / 100 * SizeFactor + Header), so every probe is reproducible.
/// </summary>
public sealed class FakeJpegCodec : IJpegCodec
{
    private readonly Rgb24Image _image;
    private readonly ImageMeta _meta;
    private int _encodeCalls;

    public FakeJpegCodec(Rgb24Image image, ImageMeta? meta = null)
    {
        _image = image;
        _meta = meta ?? new ImageMeta(96, 96, 1);
    }

    public double SizeFactor { get; init; } = 1.0;
    public int Header { get; init; } = 512;
    public Action<int>? OnEncode { get; init; }
    public bool ThrowUnsupported { get; init; }
    public bool ThrowGeneric { get; init; }

    public int EncodeCalls => _encodeCalls;
    public List<(int W, int H, int Q, int Dpi)> Calls { get; } = new();

    public Rgb24Image Decode(byte[] file, out ImageMeta meta)
    {
        if (ThrowUnsupported) throw new UnsupportedImageException("nope");
        if (ThrowGeneric) throw new InvalidOperationException("codec exploded");
        meta = _meta;
        return _image;
    }

    public byte[] Encode(Rgb24Image image, int quality, int dpi)
    {
        if (quality < 30 || quality > 95)
            throw new InvalidOperationException($"quality {quality} outside [30,95]");

        _encodeCalls++;
        Calls.Add((image.Width, image.Height, quality, dpi));
        OnEncode?.Invoke(_encodeCalls);

        int size = Size(image, quality);
        var buffer = new byte[size];
        for (int i = 0; i < size; i++)
        {
            buffer[i] = (byte)((i * 31) + (quality * 7) + image.Width + image.Height + dpi);
        }
        return buffer;
    }

    public int Size(Rgb24Image image, int quality)
    {
        double raw = (double)image.Width * image.Height * 3 * quality / 100.0 * SizeFactor;
        int size = Header + (int)Math.Round(raw, MidpointRounding.AwayFromZero);
        return Math.Max(1, size);
    }
}
