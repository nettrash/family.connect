//! What the user types, turned into the two URLs this client speaks (docs/protocol.md,
//! "Transport"). A port of `win/tests/FamilyConnect.Core.Tests/Protocol/ServerUrlTests.cs` (each
//! `[Theory]` one test over its cases), and the edges .NET's own `Uri` decided, which the C#
//! inherited without a test and this port has to write out.

use fc_core::protocol::ServerUrl;

fn normalised(typed: Option<&str>) -> Option<String> {
    ServerUrl::normalise(typed).map(|base| base.to_string())
}

#[test]
fn what_people_type_becomes_one_base() {
    for (typed, expected) in [
        ("chat.example.com", "https://chat.example.com/"),
        ("https://chat.example.com", "https://chat.example.com/"),
        ("https://chat.example.com/", "https://chat.example.com/"),
        ("  https://chat.example.com/  ", "https://chat.example.com/"),
        // A family on its own LAN types http:// and MEANS it: a bare host is guessed https, but
        // an explicit scheme is never second-guessed.
        ("http://192.168.1.10:8080", "http://192.168.1.10:8080/"),
        ("192.168.1.10:8080", "https://192.168.1.10:8080/"),
        // A pasted path is dropped: `{base}` is an origin, and a base with a path would put
        // /api/v1 under whatever happened to be on the clipboard.
        (
            "https://chat.example.com/some/page",
            "https://chat.example.com/",
        ),
    ] {
        assert_eq!(
            normalised(Some(typed)).as_deref(),
            Some(expected),
            "{typed:?}"
        );
    }
}

#[test]
fn nonsense_is_no_server() {
    for typed in [
        Some(""),
        Some("   "),
        None,
        Some("ftp://files.example.com"),
        Some("not a url"),
    ] {
        assert_eq!(normalised(typed), None, "{typed:?}");
    }
}

#[test]
fn the_rest_root_and_the_socket_follow_the_base() {
    let secure = ServerUrl::normalise(Some("chat.example.com")).expect("a base");
    assert_eq!(secure.rest(), "https://chat.example.com/api/v1");
    // `families=1` (docs/protocol.md, "Several families"): the socket's counterpart of the
    // X-FC-Families header.
    assert_eq!(
        secure.socket(),
        "wss://chat.example.com/api/v1/ws?families=1"
    );
    // ws:// for an http server, and the port rides along.
    let plain = ServerUrl::normalise(Some("http://192.168.1.10:8080")).expect("a base");
    assert_eq!(plain.rest(), "http://192.168.1.10:8080/api/v1");
    assert_eq!(
        plain.socket(),
        "ws://192.168.1.10:8080/api/v1/ws?families=1"
    );
    // And the token is nowhere in either: "a token in the query string is not a token at all" —
    // the query is the capability and nothing else.
    let socket = secure.socket();
    let query = socket.split_once('?').map(|(_, query)| query);
    assert_eq!(query, Some("families=1"));
}

/// What .NET 10's `Uri.TryCreate` + `UriBuilder` made of each of these, run on .NET itself: the
/// behaviour `win/` shipped without writing it down.
#[test]
fn the_edges_are_the_ones_dotnet_decided() {
    for (typed, expected) in [
        // Case, default ports, user info, query and fragment.
        (
            "HTTPS://Chat.Example.COM:443/x",
            Some("https://chat.example.com/"),
        ),
        ("HtTp://Example.com", Some("http://example.com/")),
        ("http://x:80", Some("http://x/")),
        ("http://192.168.1.10:80/", Some("http://192.168.1.10/")),
        (
            "https://EXAMPLE.com:8443/",
            Some("https://example.com:8443/"),
        ),
        (
            "https://user:pw@host.example/p",
            Some("https://host.example/"),
        ),
        ("https://@host", Some("https://host/")),
        ("https://a@b@c.com", None),
        ("chat.example.com?x=1", Some("https://chat.example.com/")),
        ("chat.example.com#frag", Some("https://chat.example.com/")),
        (
            "http://localhost:3000/path?x#y",
            Some("http://localhost:3000/"),
        ),
        ("localhost", Some("https://localhost/")),
        ("nas:8080", Some("https://nas:8080/")),
        // Ports.
        ("https://host:", Some("https://host/")),
        ("https://example.com:0", Some("https://example.com:0/")),
        ("https://example.com:065", Some("https://example.com:65/")),
        (
            "https://example.com:65535",
            Some("https://example.com:65535/"),
        ),
        ("https://host:99999", None),
        ("https://host:abc", None),
        ("https://host:8080:9090", None),
        ("chat.example.com:abc", None),
        // IPv6: canonical, the zone dropped, a port after the bracket.
        ("http://[::1]", Some("http://[::1]/")),
        ("http://[FD00:0:0::1]:8080", Some("http://[fd00::1]:8080/")),
        ("HTTP://[::1]:80", Some("http://[::1]/")),
        ("https://[::1]:8080/x", Some("https://[::1]:8080/")),
        (
            "https://[::ffff:192.168.1.1]",
            Some("https://[::ffff:192.168.1.1]/"),
        ),
        (
            "https://[1:2:3:4:5:6:7:8]",
            Some("https://[1:2:3:4:5:6:7:8]/"),
        ),
        ("https://[::]", Some("https://[::]/")),
        ("https://[fe80::1%25en0]", Some("https://[fe80::1]/")),
        ("https://[::1%en0]", Some("https://[::1]/")),
        ("https://[::1]:", Some("https://[::1]/")),
        ("https://[::1]x", Some("https://[::1]/")),
        ("https://[::1", None),
        ("https://[zz::1]", None),
        ("https://[v1.x]", None),
        // IPv4 in every form inet_aton reads, and what is a name instead.
        ("https://127.1", Some("https://127.0.0.1/")),
        ("https://0x7f.1", Some("https://127.0.0.1/")),
        ("https://010.0.0.1", Some("https://8.0.0.1/")),
        ("https://1.2.3", Some("https://1.2.0.3/")),
        ("https://12345", Some("https://0.0.48.57/")),
        ("https://4294967295", Some("https://255.255.255.255/")),
        ("https://0xFFFFFFFF", Some("https://255.255.255.255/")),
        ("https://1.0xFFFFFF", Some("https://1.255.255.255/")),
        ("https://1.2.0xFFFF", Some("https://1.2.255.255/")),
        ("https://01.02.03.04", Some("https://1.2.3.4/")),
        ("https://1.2.3.04", Some("https://1.2.3.4/")),
        (
            "https://00000000000000000001.2.3.4",
            Some("https://1.2.3.4/"),
        ),
        ("https://0.0.0.0", Some("https://0.0.0.0/")),
        ("https://4294967296", Some("https://4294967296/")),
        ("https://999.1.1.1", Some("https://999.1.1.1/")),
        ("https://1.2.3.256", Some("https://1.2.3.256/")),
        ("https://1.2.3.4.5", Some("https://1.2.3.4.5/")),
        ("https://1.2.3.4.", Some("https://1.2.3.4./")),
        ("https://08.1.1.1", Some("https://08.1.1.1/")),
        ("https://0x.1.1.1", Some("https://0x.1.1.1/")),
        ("https://0x100.1.1.1", Some("https://0x100.1.1.1/")),
        ("https://1.2.0x10000", Some("https://1.2.0x10000/")),
        // Names: letters, digits, `-` and `_`; no empty label but a trailing one; anything
        // non-ASCII kept as typed.
        ("https://-a.com", Some("https://-a.com/")),
        ("https://a-.com", Some("https://a-.com/")),
        ("https://under_score.com", Some("https://under_score.com/")),
        ("https://a_b-c.d-e", Some("https://a_b-c.d-e/")),
        ("https://example.com.", Some("https://example.com./")),
        ("https://Ab.Com.", Some("https://ab.com./")),
        ("https://a", Some("https://a/")),
        ("https://пример.рф", Some("https://пример.рф/")),
        ("https://ПРИМЕР.РФ", Some("https://ПРИМЕР.РФ/")),
        ("https://ÄÖÜ.de", Some("https://ÄÖÜ.de/")),
        (
            "https://xn--e1afmkfd.xn--p1ai",
            Some("https://xn--e1afmkfd.xn--p1ai/"),
        ),
        ("https://a☃b.com", Some("https://a☃b.com/")),
        ("https://a..b", None),
        ("https://.example.com", None),
        ("https://1..2.3", None),
        ("https://exa mple.com", None),
        ("https://ex\tample.com", None),
        ("https:// host.com", None),
        ("https://*.example.com", None),
        ("https://a%20b.com", None),
        ("https://ex!ample.com", None),
        ("https://a$b.com", None),
        ("https://a'b.com", None),
        ("https://a(b).com", None),
        ("https://a+b.com", None),
        ("https://a,b.com", None),
        ("https://a;b.com", None),
        ("https://a=b.com", None),
        ("https://a~b.com", None),
        ("https://a&b.com", None),
        ("https://chat.example.com\\path", None),
        // Schemes, and text that only looks like one.
        ("ws://x.com", None),
        ("https://", None),
        ("https:host.com", None),
        ("https:/host.com", Some("https://https/")),
        ("chat.example.com/?u=http://x", None),
        (
            "\thttps://tab.example.com\n",
            Some("https://tab.example.com/"),
        ),
    ] {
        assert_eq!(normalised(Some(typed)).as_deref(), expected, "{typed:?}");
    }
}

#[test]
fn an_ipv6_base_and_a_non_ascii_one_build_both_urls() {
    let six = ServerUrl::normalise(Some("http://[FD00:0:0::1]:8080")).expect("a base");
    assert_eq!(six.rest(), "http://[fd00::1]:8080/api/v1");
    assert_eq!(six.socket(), "ws://[fd00::1]:8080/api/v1/ws?families=1");
    let idn = ServerUrl::normalise(Some("https://пример.рф")).expect("a base");
    assert_eq!(idn.rest(), "https://пример.рф/api/v1");
    assert_eq!(idn.socket(), "wss://пример.рф/api/v1/ws?families=1");
    assert_eq!(idn.scheme(), "https");
    assert_eq!(idn.host(), "пример.рф");
    assert_eq!(idn.port(), None);
}
