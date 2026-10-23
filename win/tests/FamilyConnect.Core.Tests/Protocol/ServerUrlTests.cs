using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// What the user types, turned into the two URLs this client speaks (docs/protocol.md,
/// "Transport").
/// </summary>
public class ServerUrlTests
{
    [Theory]
    [InlineData("chat.example.com", "https://chat.example.com/")]
    [InlineData("https://chat.example.com", "https://chat.example.com/")]
    [InlineData("https://chat.example.com/", "https://chat.example.com/")]
    [InlineData("  https://chat.example.com/  ", "https://chat.example.com/")]
    // A family on its own LAN types http:// and MEANS it: a bare host is guessed https, but an
    // explicit scheme is never second-guessed.
    [InlineData("http://192.168.1.10:8080", "http://192.168.1.10:8080/")]
    [InlineData("192.168.1.10:8080", "https://192.168.1.10:8080/")]
    // A pasted path is dropped: `{base}` is an origin, and a base with a path would put /api/v1
    // under whatever happened to be on the clipboard.
    [InlineData("https://chat.example.com/some/page", "https://chat.example.com/")]
    public void WhatPeopleTypeBecomesOneBase(string typed, string expected) =>
        Assert.Equal(expected, ServerUrl.Normalise(typed)?.ToString());

    [Theory]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData(null)]
    [InlineData("ftp://files.example.com")]
    [InlineData("not a url")]
    public void NonsenseIsNoServer(string? typed) => Assert.Null(ServerUrl.Normalise(typed));

    [Fact]
    public void TheRestRootAndTheSocketFollowTheBase()
    {
        var secure = ServerUrl.Normalise("chat.example.com")!;
        Assert.Equal("https://chat.example.com/api/v1", ServerUrl.Rest(secure).ToString());
        // `families=1` (docs/protocol.md, "Several families"): the socket's counterpart of the X-FC-Families header.
        Assert.Equal("wss://chat.example.com/api/v1/ws?families=1", ServerUrl.Socket(secure).ToString());
        // ws:// for an http server, and the port rides along.
        var plain = ServerUrl.Normalise("http://192.168.1.10:8080")!;
        Assert.Equal("http://192.168.1.10:8080/api/v1", ServerUrl.Rest(plain).ToString());
        Assert.Equal("ws://192.168.1.10:8080/api/v1/ws?families=1", ServerUrl.Socket(plain).ToString());
        // And the token is nowhere in either: "a token in the query string is not a token at all" — the query is the
        // capability and nothing else.
        Assert.Equal("?families=1", ServerUrl.Socket(secure).Query);
    }
}
