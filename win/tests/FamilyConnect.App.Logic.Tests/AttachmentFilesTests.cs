using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The web client's attachment rules, with the web client's own vectors.</summary>
public class AttachmentFilesTests
{
    [Fact]
    public void ALongNameGivesUpItsMiddle()
    {
        Assert.Equal("short.pdf", AttachmentFiles.MiddleTruncate("short.pdf", 40));
        var cut = AttachmentFiles.MiddleTruncate(string.Concat(Enumerable.Repeat("Quarterly report ", 5)) + ".pdf", 20);
        Assert.Equal(20, cut.EnumerateRunes().Count());
        Assert.StartsWith("Quarterly re", cut);
        Assert.EndsWith("t .pdf", cut);
        Assert.Contains("…", cut);
        // Too little room to say anything useful: the name as it is.
        Assert.Equal("abcdefgh", AttachmentFiles.MiddleTruncate("abcdefgh", 4));
    }

    [Fact]
    public void ACutNeverSplitsAnEmoji()
    {
        var face = char.ConvertFromUtf32(0x1F600);
        var name = string.Concat(Enumerable.Repeat(face, 30)) + ".png";
        var cut = AttachmentFiles.MiddleTruncate(name, 12);
        Assert.Equal(12, cut.EnumerateRunes().Count());
        Assert.DoesNotContain(cut, char.IsSurrogate(cut, 0) && !char.IsSurrogatePair(cut, 0) ? "x" : "\0");
        foreach (var rune in cut.EnumerateRunes())
        {
            Assert.NotEqual(System.Text.Rune.ReplacementChar, rune);
        }
    }

    /// <summary>
    /// The name is the sender's, and the server only checks its length, so everything Windows
    /// refuses has to be caught here.
    /// </summary>
    [Fact]
    public void ASavedNameIsNeverAPathAndNeverACharacterWindowsRefuses()
    {
        Assert.Equal("receipts.pdf", AttachmentFiles.SafeFileName("receipts.pdf"));
        // A path is reduced to its last part, in either slash.
        Assert.Equal("receipts.pdf", AttachmentFiles.SafeFileName(@"C:\Users\nora\receipts.pdf"));
        Assert.Equal("receipts.pdf", AttachmentFiles.SafeFileName("../../etc/receipts.pdf"));
        // Trailing dots and spaces are not a name Windows will keep.
        Assert.Equal("receipts", AttachmentFiles.SafeFileName("  receipts.  "));
        // Nothing usable left: something has to be written somewhere.
        Assert.Equal("attachment.bin", AttachmentFiles.SafeFileName("   "));
        Assert.Equal("attachment.bin", AttachmentFiles.SafeFileName("/"));
    }

    /// <summary>
    /// Win32 resolves its device names in EVERY directory and with ANY extension, so an attachment
    /// called "CON.txt" would be written to the console rather than to a file — the write goes to a
    /// device or fails, opening it afterwards cannot work, and the reader is told only "Something
    /// went wrong". Anyone in the family can name a file that, so the name must stop being a device.
    /// </summary>
    [Fact]
    public void ADeviceNameIsNotLeftAsOne()
    {
        Assert.Equal("_CON", AttachmentFiles.SafeFileName("CON"));
        Assert.Equal("_con.txt", AttachmentFiles.SafeFileName("con.txt"));
        Assert.Equal("_NUL.pdf", AttachmentFiles.SafeFileName("NUL.pdf"));
        Assert.Equal("_COM1.jpg", AttachmentFiles.SafeFileName("COM1.jpg"));
        Assert.Equal("_LPT9", AttachmentFiles.SafeFileName("LPT9"));
        Assert.Equal("_PRN.doc", AttachmentFiles.SafeFileName("PRN.doc"));
        Assert.Equal("_AUX", AttachmentFiles.SafeFileName("AUX."));
        // Not devices: only the exact stems are reserved.
        Assert.Equal("CONtract.pdf", AttachmentFiles.SafeFileName("CONtract.pdf"));
        Assert.Equal("COM10.txt", AttachmentFiles.SafeFileName("COM10.txt"));
        Assert.Equal("nulls.csv", AttachmentFiles.SafeFileName("nulls.csv"));
        Assert.Equal(".con", AttachmentFiles.SafeFileName(".con"));
    }

    [Fact]
    public void ADownloadIsNamedForWhatItIs()
    {
        Assert.Equal("receipts.pdf", AttachmentFiles.FileName(new AttachmentDto(9, "file", Name: "receipts.pdf")));
        Assert.Equal("photo-34.jpg", AttachmentFiles.FileName(new AttachmentDto(34, "photo", Mime: "image/jpeg")));
        Assert.Equal("video-35.mov", AttachmentFiles.FileName(new AttachmentDto(35, "video", Mime: "video/quicktime")));
        Assert.Equal("audio-36.m4a", AttachmentFiles.FileName(new AttachmentDto(36, "audio", Mime: "audio/mp4; codecs=mp4a.40.2")));
        Assert.Equal("file-37.bin", AttachmentFiles.FileName(new AttachmentDto(37, "file", Name: "")));
    }

    /// <summary>Which bytes a tile asks for — never a whole video.</summary>
    [Fact]
    public void ATileNeverDownloadsAVideoToDrawItself()
    {
        Assert.Equal(AttachmentFiles.TileSource.Preview, AttachmentFiles.SourceFor(new AttachmentDto(1, "photo", HasPreview: true)));
        Assert.Equal(AttachmentFiles.TileSource.Original, AttachmentFiles.SourceFor(new AttachmentDto(1, "photo")));
        Assert.Equal(AttachmentFiles.TileSource.Preview, AttachmentFiles.SourceFor(new AttachmentDto(1, "video", HasPreview: true)));
        Assert.Equal(AttachmentFiles.TileSource.None, AttachmentFiles.SourceFor(new AttachmentDto(1, "video")));
    }
}
