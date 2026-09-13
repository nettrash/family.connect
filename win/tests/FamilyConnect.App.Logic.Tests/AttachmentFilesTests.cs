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
