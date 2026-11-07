using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using Kilobyt.Core;

namespace Kilobyt.App;

/// <summary>
/// PRD §5.8 platform codec, backed by System.Drawing (Windows-only, risk R1).
/// Decoding flattens alpha over white and reads EXIF orientation/DPI; encoding paints
/// into a brand-new 24 bpp bitmap so no source metadata can be inherited, and the
/// resulting JPEG is scrubbed segment-by-segment as a second guarantee (§5.2.13).
/// </summary>
public sealed class SystemDrawingJpegCodec : IJpegCodec
{
    internal const int ExifOrientationTag = 0x0112;
    private const int MinQuality = 30;
    private const int MaxQuality = 95;
    private const int DefaultDpi = 96;
    private const int MaxDpi = 1200;

    public Rgb24Image Decode(byte[] file, out ImageMeta meta)
    {
        if (file is null || file.Length == 0)
            throw new UnsupportedImageException(ErrorMessages.Decode);

        try
        {
            // The stream must outlive the Image: GDI+ reads it lazily.
            using var stream = new MemoryStream(file, writable: false);
            using Image source = Image.FromStream(stream, useEmbeddedColorManagement: false, validateImageData: true);

            int width = source.Width;
            int height = source.Height;
            if (width < 1 || height < 1)
                throw new UnsupportedImageException(ErrorMessages.Decode);

            meta = new ImageMeta(ReadDpi(source.HorizontalResolution), ReadDpi(source.VerticalResolution), ReadOrientation(source));

            using var canvas = new Bitmap(width, height, PixelFormat.Format32bppArgb);
            using (Graphics g = Graphics.FromImage(canvas))
            {
                g.CompositingMode = CompositingMode.SourceOver;
                g.CompositingQuality = CompositingQuality.HighSpeed;
                g.InterpolationMode = InterpolationMode.NearestNeighbor;
                g.PixelOffsetMode = PixelOffsetMode.Half;
                g.SmoothingMode = SmoothingMode.None;
                g.Clear(Color.White);
                g.DrawImage(source, new Rectangle(0, 0, width, height), 0, 0, width, height, GraphicsUnit.Pixel);
            }

            byte[] pixels = ReadRgb24(canvas, width, height);
            return new Rgb24Image(width, height, pixels);
        }
        catch (UnsupportedImageException)
        {
            throw;
        }
        catch (Exception ex)
        {
            // Includes OutOfMemoryException surfaces from GDI+ on absurd inputs.
            throw new UnsupportedImageException(ErrorMessages.Decode, ex);
        }
    }

    public byte[] Encode(Rgb24Image image, int quality, int dpi)
    {
        if (image.Pixels is null)
            throw new ArgumentException("Image has no pixels.", nameof(image));

        int width = image.Width;
        int height = image.Height;
        if (width < 1 || height < 1 || image.Pixels.LongLength != (long)width * height * 3)
            throw new ArgumentException("Pixel buffer does not match the image dimensions.", nameof(image));

        int q = Math.Clamp(quality, MinQuality, MaxQuality);
        int density = dpi is >= 1 and <= MaxDpi ? dpi : DefaultDpi;

        try
        {
            using var bitmap = new Bitmap(width, height, PixelFormat.Format24bppRgb);
            WriteRgb24(bitmap, image.Pixels, width, height);
            bitmap.SetResolution(density, density);

            ImageCodecInfo jpeg = FindJpegEncoder();
            using var parameters = new EncoderParameters(1);
            parameters.Param[0] = new EncoderParameter(Encoder.Quality, (long)q);

            using var buffer = new MemoryStream(600_000);
            bitmap.Save(buffer, jpeg, parameters);
            return JpegSegmentScrubber.StripMetadata(buffer.ToArray());
        }
        catch (Exception ex) when (ex is ArgumentException or ExternalException or IOException or OutOfMemoryException)
        {
            throw new UnsupportedImageException(ErrorMessages.Decode, ex);
        }
    }

    private static ImageCodecInfo FindJpegEncoder()
    {
        ImageCodecInfo[] encoders = ImageCodecInfo.GetImageEncoders();
        foreach (ImageCodecInfo candidate in encoders)
        {
            if (candidate.FormatID == ImageFormat.Jpeg.Guid) return candidate;
        }
        throw new UnsupportedImageException("No JPEG encoder is available on this system.");
    }

    private static byte[] ReadRgb24(Bitmap source, int width, int height)
    {
        var destination = new byte[(long)width * height * 3];
        Rectangle rect = new(0, 0, width, height);
        BitmapData data = source.LockBits(rect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        try
        {
            int stride = data.Stride;
            byte[] row = new byte[Math.Abs(stride)];
            for (int y = 0; y < height; y++)
            {
                Marshal.Copy(IntPtr.Add(data.Scan0, y * stride), row, 0, row.Length);
                int destinationIndex = y * width * 3;
                for (int x = 0; x < width; x++)
                {
                    int i = x * 4;
                    int b = row[i];
                    int g = row[i + 1];
                    int r = row[i + 2];
                    int a = row[i + 3];

                    if (a != 255)
                    {
                        // Composite over white, round-half-up (§5.2.2).
                        r = (r * a + 255 * (255 - a) + 127) / 255;
                        g = (g * a + 255 * (255 - a) + 127) / 255;
                        b = (b * a + 255 * (255 - a) + 127) / 255;
                    }

                    destination[destinationIndex] = (byte)b;
                    destination[destinationIndex + 1] = (byte)g;
                    destination[destinationIndex + 2] = (byte)r;
                    destinationIndex += 3;
                }
            }
        }
        finally
        {
            source.UnlockBits(data);
        }

        return destination;
    }

    private static void WriteRgb24(Bitmap destination, byte[] pixels, int width, int height)
    {
        Rectangle rect = new(0, 0, width, height);
        BitmapData data = destination.LockBits(rect, ImageLockMode.WriteOnly, PixelFormat.Format24bppRgb);
        try
        {
            int stride = data.Stride;
            byte[] row = new byte[Math.Abs(stride)];
            for (int y = 0; y < height; y++)
            {
                Array.Clear(row);
                int sourceIndex = y * width * 3;
                for (int x = 0; x < width; x++)
                {
                    int i = x * 3;
                    int o = x * 3;
                    row[o] = pixels[sourceIndex + i];
                    row[o + 1] = pixels[sourceIndex + i + 1];
                    row[o + 2] = pixels[sourceIndex + i + 2];
                }

                Marshal.Copy(row, 0, IntPtr.Add(data.Scan0, y * stride), row.Length);
            }
        }
        finally
        {
            destination.UnlockBits(data);
        }
    }

    private static int ReadDpi(float resolution)
    {
        if (float.IsNaN(resolution) || float.IsInfinity(resolution) || resolution <= 0f) return 0;
        double rounded = Math.Round(resolution, MidpointRounding.AwayFromZero);
        if (rounded < 1) return 1;
        if (rounded > MaxDpi) return MaxDpi;
        return (int)rounded;
    }

    private static int ReadOrientation(Image image)
    {
        try
        {
            if (!image.PropertyIdList.AsSpan().Contains(ExifOrientationTag)) return 1;

            PropertyItem? item = image.GetPropertyItem(ExifOrientationTag);
            if (item?.Value is not { Length: >= 2 } value) return 1;

            int orientation = BitConverter.ToUInt16(value, 0);
            return orientation is >= 1 and <= 8 ? orientation : 1;
        }
        catch (ArgumentException)
        {
            // Missing or malformed orientation property: treat as upright.
            return 1;
        }
        catch (ExternalException)
        {
            return 1;
        }
    }
}

/// <summary>
/// PRD §5.2.13 — keeps SOI, the frame tables, APP0 (JFIF density) and the entropy-coded
/// data; drops every other APPn (EXIF/XMP/GPS/ICC) and any COM segment.
/// </summary>
internal static class JpegSegmentScrubber
{
    private const byte MarkerSoi = 0xD8;
    private const byte MarkerEoi = 0xD9;
    private const byte MarkerSos = 0xDA;
    private const byte MarkerCom = 0xFE;
    private const byte MarkerApp0 = 0xE0;

    public static byte[] StripMetadata(byte[] jpeg)
    {
        if (jpeg is null || jpeg.Length < 4 || jpeg[0] != 0xFF || jpeg[1] != MarkerSoi)
            return jpeg ?? Array.Empty<byte>();

        using var output = new MemoryStream(jpeg.Length);
        output.WriteByte(0xFF);
        output.WriteByte(MarkerSoi);

        int i = 2;
        while (i + 1 < jpeg.Length)
        {
            if (jpeg[i] != 0xFF) return jpeg; // not a marker: leave the original bytes alone

            int markerStart = i;
            while (i < jpeg.Length && jpeg[i] == 0xFF) i++;
            if (i >= jpeg.Length) return jpeg;

            byte marker = jpeg[i++];

            if (marker == MarkerEoi || marker == MarkerSos)
            {
                output.Write(jpeg, markerStart, jpeg.Length - markerStart);
                return output.ToArray();
            }

            if (marker == 0x01 || (marker >= 0xD0 && marker <= 0xD7))
            {
                output.Write(jpeg, markerStart, i - markerStart);
                continue;
            }

            if (i + 1 >= jpeg.Length) return jpeg;
            int length = (jpeg[i] << 8) | jpeg[i + 1];
            if (length < 2 || i + length > jpeg.Length) return jpeg;

            bool drop = (marker >= 0xE1 && marker <= 0xEF) || marker == MarkerCom;
            if (!drop || marker == MarkerApp0)
                output.Write(jpeg, markerStart, i + length - markerStart);

            i += length;
        }

        return output.ToArray();
    }
}
