using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;

namespace Kilobyt.App.Tests;

/// <summary>Deterministic fixtures generated in code (PRD §6: no checked-in binaries).</summary>
internal static class TestImages
{
    /// <summary>Deterministic PRNG noise: high entropy, so the encoder has real work to do.</summary>
    public static Bitmap Noise(int width, int height, int seed)
    {
        var bitmap = new Bitmap(width, height, PixelFormat.Format24bppRgb);
        var random = new Random(seed);
        var buffer = new byte[width * height * 3];
        random.NextBytes(buffer);

        BitmapData data = bitmap.LockBits(new Rectangle(0, 0, width, height), ImageLockMode.WriteOnly, PixelFormat.Format24bppRgb);
        try
        {
            // The Random() stream is per-row-aligned only by luck; copy row by row.
            int stride = data.Stride;
            var row = new byte[Math.Abs(stride)];
            for (int y = 0; y < height; y++)
            {
                Array.Copy(buffer, y * width * 3, row, 0, width * 3);
                Marshal.Copy(row, 0, IntPtr.Add(data.Scan0, y * stride), row.Length);
            }
        }
        finally
        {
            bitmap.UnlockBits(data);
        }

        return bitmap;
    }

    /// <summary>
    /// A synthetic "photo": smooth gradient scene plus hard-edged shapes. Compressible
    /// enough that a real encoder reaches a 100 KB target well inside the probe budget,
    /// unlike uniform noise, which is close to incompressible.
    /// </summary>
    public static Bitmap PhotoLike(int width, int height, int seed)
    {
        var bitmap = new Bitmap(width, height, PixelFormat.Format24bppRgb);
        using (Graphics g = Graphics.FromImage(bitmap))
        {
            g.SmoothingMode = SmoothingMode.AntiAlias;
            var bounds = new Rectangle(0, 0, width, height);

            using (var sky = new LinearGradientBrush(bounds, Color.FromArgb(64, 118, 188), Color.FromArgb(238, 226, 200), LinearGradientMode.Vertical))
            {
                g.FillRectangle(sky, bounds);
            }

            using (var sun = new SolidBrush(Color.FromArgb(255, 246, 220)))
            {
                g.FillEllipse(sun, width * 0.60f, height * 0.10f, width * 0.16f, height * 0.16f);
            }

            using (var hill = new SolidBrush(Color.FromArgb(56, 96, 62)))
            {
                g.FillEllipse(hill, -width * 0.30f, height * 0.58f, width * 1.20f, height * 0.80f);
            }

            using (var water = new SolidBrush(Color.FromArgb(44, 78, 118)))
            {
                int top = (int)(height * 0.84f);
                g.FillRectangle(water, 0, top, width, height - top);
            }

            var random = new Random(seed);
            for (int i = 0; i < 24; i++)
            {
                int w = Math.Max(4, (int)(width * (0.02 + (random.NextDouble() * 0.06))));
                int h = Math.Max(4, (int)(height * (0.02 + (random.NextDouble() * 0.05))));
                int x = random.Next(0, Math.Max(1, width - w));
                int y = random.Next(0, Math.Max(1, (int)(height * 0.80)));
                using var brush = new SolidBrush(Color.FromArgb(random.Next(30, 230), random.Next(30, 230), random.Next(30, 230)));
                g.FillRectangle(brush, x, y, w, h);
            }
        }

        return bitmap;
    }

    public static Bitmap Solid(int width, int height, Color color)
    {
        var bitmap = new Bitmap(width, height, PixelFormat.Format24bppRgb);
        using Graphics g = Graphics.FromImage(bitmap);
        g.Clear(color);
        return bitmap;
    }

    public static Bitmap Transparent(int width, int height)
    {
        var bitmap = new Bitmap(width, height, PixelFormat.Format32bppArgb);
        using Graphics g = Graphics.FromImage(bitmap);
        g.Clear(Color.Transparent);
        return bitmap;
    }

    public static byte[] JpegBytes(Bitmap bitmap, int quality = 90)
    {
        ImageCodecInfo jpeg = FindJpegEncoder();
        using var parameters = new EncoderParameters(1);
        parameters.Param[0] = new EncoderParameter(System.Drawing.Imaging.Encoder.Quality, (long)quality);
        using var buffer = new MemoryStream();
        bitmap.Save(buffer, jpeg, parameters);
        return buffer.ToArray();
    }

    public static byte[] PngBytes(Bitmap bitmap)
    {
        using var buffer = new MemoryStream();
        bitmap.Save(buffer, ImageFormat.Png);
        return buffer.ToArray();
    }

    public static byte[] BmpBytes(Bitmap bitmap)
    {
        using var buffer = new MemoryStream();
        bitmap.Save(buffer, ImageFormat.Bmp);
        return buffer.ToArray();
    }

    public static ImageCodecInfo FindJpegEncoder()
    {
        foreach (ImageCodecInfo encoder in ImageCodecInfo.GetImageEncoders())
        {
            if (encoder.FormatID == ImageFormat.Jpeg.Guid) return encoder;
        }

        throw new InvalidOperationException("No JPEG encoder on this machine.");
    }

    public static Bitmap DecodeOrThrow(byte[] bytes)
    {
        using var stream = new MemoryStream(bytes, writable: false);
        using Image image = Image.FromStream(stream, useEmbeddedColorManagement: false, validateImageData: true);
        return new Bitmap(image);
    }
}

/// <summary>Builds a minimal but valid EXIF APP1 segment (GPS + make + serial + date).</summary>
internal static class ExifInjector
{
    public const string MakeSentinel = "KILOBYT-CAMERA-9";
    public const string SerialSentinel = "SN-KB-424242";

    private const ushort TypeByte = 1;
    private const ushort TypeAscii = 2;
    private const ushort TypeShort = 3;
    private const ushort TypeLong = 4;
    private const ushort TypeRational = 5;

    /// <summary>Returns <paramref name="jpeg"/> with an APP1 EXIF segment inserted after APP0 (or after SOI).</summary>
    public static byte[] WithExif(byte[] jpeg, ushort orientation = 1, bool gps = true)
    {
        byte[] app1 = BuildApp1(orientation, gps);

        // Conventional order: SOI, APP0 (JFIF), then the other APPn segments.
        int insertAt = 2;
        if (jpeg.Length > 6 && jpeg[2] == 0xFF && jpeg[3] == 0xE0)
        {
            int app0Length = (jpeg[4] << 8) | jpeg[5];
            if (app0Length >= 2 && 4 + app0Length <= jpeg.Length) insertAt = 4 + app0Length;
        }

        var output = new byte[jpeg.Length + app1.Length];
        Buffer.BlockCopy(jpeg, 0, output, 0, insertAt);
        Buffer.BlockCopy(app1, 0, output, insertAt, app1.Length);
        Buffer.BlockCopy(jpeg, insertAt, output, insertAt + app1.Length, jpeg.Length - insertAt);
        return output;
    }

    private static byte[] BuildApp1(ushort orientation, bool gps)
    {
        const int ifd0EntryCount = 6;
        const int tiffHeaderSize = 8;

        var ifd0 = new List<Entry>
        {
            Entry.Ascii(0x010F, MakeSentinel),
            Entry.Ascii(0x0110, "Kilobyt Test Body"),
            Entry.Inline(0x0112, TypeShort, 1, BitConverter.GetBytes(orientation)),
            Entry.Ascii(0x0131, "Kilobyt.Tests 1.0"),
            Entry.Ascii(0x0132, "2026:11:07 12:34:56"),
            Entry.Inline(0x8825, TypeLong, 1, new byte[4]),
        };

        // GPS IFD lives after IFD0's header *and* its out-of-line values, so measure first.
        byte[] measure = BuildIfd(ifd0, tiffHeaderSize);
        uint gpsIfdOffset = (uint)(tiffHeaderSize + measure.Length);
        ifd0[ifd0EntryCount - 1] = Entry.Inline(0x8825, TypeLong, 1, BitConverter.GetBytes(gpsIfdOffset));

        byte[] ifd0Bytes = BuildIfd(ifd0, tiffHeaderSize);
        if (ifd0Bytes.Length != measure.Length)
            throw new InvalidOperationException("IFD0 size changed while resolving the GPS pointer.");
        byte[] gpsBytes = gps ? BuildIfd(GpsEntries(), (int)gpsIfdOffset) : Array.Empty<byte>();

        using var tiff = new MemoryStream();
        using var writer = new BinaryWriter(tiff);
        writer.Write((byte)'I');
        writer.Write((byte)'I');
        writer.Write((ushort)0x002A);
        writer.Write(0x00000008u);
        writer.Write(ifd0Bytes);
        if (gps) writer.Write(gpsBytes);
        writer.Flush();

        byte[] payload = tiff.ToArray();
        int segmentLength = 2 + 6 + payload.Length;

        using var segment = new MemoryStream();
        using var seg = new BinaryWriter(segment);
        seg.Write((byte)0xFF);
        seg.Write((byte)0xE1);
        // JPEG segment lengths are BIG-endian; BinaryWriter would write them little-endian and
        // GDI+ then reads a bogus length and refuses the file ("JPEG datastream contains no image").
        seg.Write((byte)(segmentLength >> 8));
        seg.Write((byte)(segmentLength & 0xFF));
        seg.Write(Encoding.ASCII.GetBytes("Exif\0\0"));
        seg.Write(payload);
        seg.Flush();
        return segment.ToArray();
    }

    private static List<Entry> GpsEntries()
    {
        return new List<Entry>
        {
            Entry.Inline(0x0000, TypeByte, 4, new byte[] { 2, 3, 0, 0 }),
            Entry.Ascii(0x0001, "N"),
            Entry.Inline(0x0002, TypeRational, 3, Rationals(51, 1, 30, 1, 1234, 100)),
            Entry.Ascii(0x0003, "E"),
            Entry.Inline(0x0004, TypeRational, 3, Rationals(0, 1, 7, 1, 9876, 100)),
        };
    }

    private static byte[] Rationals(params int[] pairs)
    {
        using var stream = new MemoryStream();
        using var writer = new BinaryWriter(stream);
        for (int i = 0; i + 1 < pairs.Length; i += 2)
        {
            writer.Write((uint)pairs[i]);
            writer.Write((uint)pairs[i + 1]);
        }

        writer.Flush();
        return stream.ToArray();
    }

    private static byte[] BuildIfd(List<Entry> entries, int ifdOffset)
    {
        int headerSize = 2 + (entries.Count * 12) + 4;
        int dataCursor = ifdOffset + headerSize;

        using var head = new MemoryStream();
        using var body = new MemoryStream();
        using var headWriter = new BinaryWriter(head);
        using var bodyWriter = new BinaryWriter(body);

        headWriter.Write((ushort)entries.Count);

        foreach (Entry entry in entries)
        {
            headWriter.Write(entry.Tag);
            headWriter.Write(entry.Type);
            headWriter.Write(entry.Count);

            if (entry.Data.Length <= 4)
            {
                var padded = new byte[4];
                Buffer.BlockCopy(entry.Data, 0, padded, 0, entry.Data.Length);
                headWriter.Write(padded);
            }
            else
            {
                headWriter.Write((uint)dataCursor);
                bodyWriter.Write(entry.Data);
                if (entry.Data.Length % 2 == 1) bodyWriter.Write((byte)0);
                dataCursor += entry.Data.Length + (entry.Data.Length % 2);
            }
        }

        headWriter.Write(0u);
        headWriter.Flush();
        bodyWriter.Flush();

        byte[] headBytes = head.ToArray();
        byte[] bodyBytes = body.ToArray();
        var result = new byte[headBytes.Length + bodyBytes.Length];
        Buffer.BlockCopy(headBytes, 0, result, 0, headBytes.Length);
        Buffer.BlockCopy(bodyBytes, 0, result, headBytes.Length, bodyBytes.Length);

        // The IFD must be exactly headerSize bytes before the external data.
        if (headBytes.Length != headerSize)
            throw new InvalidOperationException($"IFD header {headBytes.Length} != {headerSize}.");

        return result;
    }

    private sealed class Entry
    {
        public ushort Tag;
        public ushort Type;
        public uint Count;
        public byte[] Data = Array.Empty<byte>();

        public static Entry Ascii(ushort tag, string value)
        {
            byte[] text = Encoding.ASCII.GetBytes(value + "\0");
            return new Entry { Tag = tag, Type = TypeAscii, Count = (uint)text.Length, Data = text };
        }

        public static Entry Inline(ushort tag, ushort type, uint count, byte[] data)
            => new() { Tag = tag, Type = type, Count = count, Data = data };
    }
}

/// <summary>Minimal JPEG marker reader — used to prove no APP1/COM/ICC segment survived.</summary>
internal static class JpegInspector
{
    public static IReadOnlyList<byte> Markers(byte[] jpeg)
    {
        var markers = new List<byte>();
        if (jpeg is null || jpeg.Length < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8) return markers;

        int i = 2;
        while (i + 1 < jpeg.Length)
        {
            if (jpeg[i] != 0xFF) break;
            while (i < jpeg.Length && jpeg[i] == 0xFF) i++;
            if (i >= jpeg.Length) break;
            byte marker = jpeg[i++];
            markers.Add(marker);

            if (marker == 0xD9 || marker == 0xDA) break;
            if (marker == 0x01 || (marker >= 0xD0 && marker <= 0xD7)) continue;
            if (i + 1 >= jpeg.Length) break;

            int length = (jpeg[i] << 8) | jpeg[i + 1];
            if (length < 2 || i + length > jpeg.Length) break;
            i += length;
        }

        return markers;
    }

    public static bool HasApp1(byte[] jpeg) => Markers(jpeg).Contains((byte)0xE1);

    public static int AppSegmentCount(byte[] jpeg) => Markers(jpeg).Count(m => m >= 0xE0 && m <= 0xEF);

    public static bool ContainsAscii(byte[] haystack, string needle)
    {
        byte[] pattern = Encoding.ASCII.GetBytes(needle);
        return haystack.AsSpan().IndexOf(pattern) >= 0;
    }
}

/// <summary>Temp folder that cleans up after itself.</summary>
internal sealed class TempDir : IDisposable
{
    public TempDir(string prefix = "kilobyt-tests")
    {
        Path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), $"{prefix}-{Guid.NewGuid():N}");
        Directory.CreateDirectory(Path);
    }

    public string Path { get; }

    public string File(string name) => System.IO.Path.Combine(Path, name);

    public void Dispose()
    {
        try
        {
            if (Directory.Exists(Path)) Directory.Delete(Path, recursive: true);
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }
    }
}
