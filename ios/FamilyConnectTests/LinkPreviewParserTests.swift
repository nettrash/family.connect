//
//  LinkPreviewParserTests.swift
//  FamilyConnectTests
//
//  Pins how a page turns into the card under a link. The parser is a
//  cross-platform contract (Android transcribes it, with these same
//  vectors mirrored in LinkPreviewParserTest.kt), so the vectors here
//  ARE the spec: the Open Graph → Twitter → plain-HTML fallback ladder,
//  attribute forms real pages use (single quotes, reversed order,
//  self-closing), entity decoding, whitespace collapsing, length
//  clamping, relative image resolution, and the refusal to build a
//  card with no title.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Link preview parsing")
struct LinkPreviewParserTests {

    private let page = URL(string: "https://example.com/articles/1")!

    @Test("Open Graph tags win")
    func openGraph() {
        let html = """
        <html><head>
        <title>Ignored</title>
        <meta property="og:title" content="The Real Title">
        <meta property="og:site_name" content="Example News">
        <meta property="og:description" content="A short summary.">
        <meta property="og:image" content="https://cdn.example.com/a.jpg">
        </head><body>…</body></html>
        """
        let preview = LinkPreviewParser.parse(html: html, pageURL: page)
        #expect(preview?.title == "The Real Title")
        #expect(preview?.siteName == "Example News")
        #expect(preview?.description == "A short summary.")
        #expect(preview?.imageURL?.absoluteString == "https://cdn.example.com/a.jpg")
    }

    @Test("Twitter cards are the second choice, plain HTML the third")
    func fallbackLadder() {
        let twitter = """
        <head><meta name="twitter:title" content="Twitter Title">
        <meta name="twitter:description" content="Twitter summary">
        <meta name="twitter:image" content="https://cdn.example.com/t.png"></head>
        """
        let fromTwitter = LinkPreviewParser.parse(html: twitter, pageURL: page)
        #expect(fromTwitter?.title == "Twitter Title")
        #expect(fromTwitter?.description == "Twitter summary")
        #expect(fromTwitter?.imageURL?.absoluteString == "https://cdn.example.com/t.png")

        let plain = """
        <head><title>Plain Title</title>
        <meta name="description" content="Plain summary"></head>
        """
        let fromPlain = LinkPreviewParser.parse(html: plain, pageURL: page)
        #expect(fromPlain?.title == "Plain Title")
        #expect(fromPlain?.description == "Plain summary")
        #expect(fromPlain?.imageURL == nil)
    }

    @Test("no title means no card")
    func titleRequired() {
        #expect(LinkPreviewParser.parse(html: "<head></head>", pageURL: page) == nil)
        #expect(LinkPreviewParser.parse(html: "<head><title>   </title></head>", pageURL: page) == nil)
        #expect(LinkPreviewParser.parse(html: "", pageURL: page) == nil)
    }

    @Test("the site name falls back to the host, without www.")
    func siteNameFallsBackToHost() {
        let html = "<head><title>T</title></head>"
        let preview = LinkPreviewParser.parse(
            html: html, pageURL: URL(string: "https://www.example.com/x")!)
        #expect(preview?.siteName == "example.com")
    }

    @Test("attribute forms real pages use")
    func attributeForms() {
        // Single quotes, reversed order, self-closing, extra attributes.
        let html = """
        <head>
        <meta content='Reversed Order' property='og:title' />
        <meta charset="utf-8">
        <meta data-rh="true" property="og:description" content="Desc" />
        </head>
        """
        let preview = LinkPreviewParser.parse(html: html, pageURL: page)
        #expect(preview?.title == "Reversed Order")
        #expect(preview?.description == "Desc")
    }

    @Test("the first occurrence of a key wins")
    func firstKeyWins() {
        let html = """
        <head><meta property="og:title" content="First">
        <meta property="og:title" content="Second"></head>
        """
        #expect(LinkPreviewParser.parse(html: html, pageURL: page)?.title == "First")
    }

    @Test("entities are decoded and whitespace collapsed")
    func entitiesAndWhitespace() {
        let html = """
        <head><meta property="og:title"
        content="Tom &amp; Jerry&#39;s
             big   day &hellip;"></head>
        """
        #expect(LinkPreviewParser.parse(html: html, pageURL: page)?.title
            == "Tom & Jerry's big day …")
    }

    @Test("over-long text is clamped with an ellipsis")
    func clamping() {
        let long = String(repeating: "a", count: 400)
        let html = "<head><meta property=\"og:title\" content=\"\(long)\"></head>"
        let title = LinkPreviewParser.parse(html: html, pageURL: page)?.title ?? ""
        #expect(title.count == LinkPreviewParser.maxTitleLength + 1) // + the ellipsis
        #expect(title.hasSuffix("…"))
    }

    @Test("relative and protocol-relative images resolve against the page")
    func imageResolution() {
        let relative = "<head><title>T</title><meta property=\"og:image\" content=\"/img/a.png\"></head>"
        #expect(LinkPreviewParser.parse(html: relative, pageURL: page)?.imageURL?.absoluteString
            == "https://example.com/img/a.png")

        let protocolRelative = "<head><title>T</title><meta property=\"og:image\" content=\"//cdn.example.com/b.png\"></head>"
        #expect(LinkPreviewParser.parse(html: protocolRelative, pageURL: page)?.imageURL?.absoluteString
            == "https://cdn.example.com/b.png")
    }

    @Test("non-http image schemes are refused")
    func imageSchemeRestricted() {
        let html = "<head><title>T</title><meta property=\"og:image\" content=\"file:///etc/passwd\"></head>"
        #expect(LinkPreviewParser.parse(html: html, pageURL: page)?.imageURL == nil)
    }

    @Test("a tag whose name merely starts with meta is not a meta tag")
    func tagNameBoundary() {
        let html = "<head><metadata property=\"og:title\" content=\"Nope\"><title>Real</title></head>"
        #expect(LinkPreviewParser.parse(html: html, pageURL: page)?.title == "Real")
    }

    @Test("Unicode that lowercases to a different length does not corrupt or crash")
    func unicodeLengthDrift() {
        // İ (U+0130) lowercases to two scalars, ẞ to "ss", KELVIN K to
        // "k": matching on a lowercased COPY and slicing the original
        // with its indices traps here and corrupts titles on Android.
        #expect(LinkPreviewParser.parse(
            html: "<head><title>İstanbul Haber</title></head>", pageURL: page)?.title
            == "İstanbul Haber")
        #expect(LinkPreviewParser.parse(
            html: "<head><title>İİİİİİİİİİ</title></head>", pageURL: page)?.title
            == "İİİİİİİİİİ")
        #expect(LinkPreviewParser.parse(
            html: "<head><title>GROẞE STRAẞE</title></head>", pageURL: page)?.title
            == "GROẞE STRAẞE")
        #expect(LinkPreviewParser.parse(
            html: "<head><p>\u{212A}\u{212A}\u{212A}</p><title>Real</title></head>", pageURL: page)?.title
            == "Real")
        // og: tags after drifting characters must still be found.
        let afterDrift = "<head><p>İİİİİİİİ</p><meta property=\"og:title\" content=\"Found\"></head>"
        #expect(LinkPreviewParser.parse(html: afterDrift, pageURL: page)?.title == "Found")
    }

    @Test("a stray-ampersand page stays linear instead of quadratic")
    func entityScanIsBounded() {
        let hostile = "<head><title>" + String(repeating: "&", count: 30_000) + "x;</title></head>"
        let started = Date()
        _ = LinkPreviewParser.parse(html: hostile, pageURL: page)
        #expect(Date().timeIntervalSince(started) < 2.0)
    }

    @Test("a quoted > does not truncate the tag")
    func quotedAngleBracket() {
        let html = "<head><meta property=\"og:title\" content=\"A > B\"><title>Fallback</title></head>"
        #expect(LinkPreviewParser.parse(html: html, pageURL: page)?.title == "A > B")
    }

    @Test("clamping counts scalars and never splits a pair")
    func clampingIsScalarBased() {
        let emoji = String(repeating: "😀", count: 200)
        let html = "<head><meta property=\"og:title\" content=\"\(emoji)\"></head>"
        let title = LinkPreviewParser.parse(html: html, pageURL: page)?.title ?? ""
        // 140 scalars kept + the ellipsis, and every one still an emoji.
        #expect(title.unicodeScalars.count == LinkPreviewParser.maxTitleLength + 1)
        #expect(title.hasSuffix("…"))
        #expect(title.dropLast().allSatisfy { $0 == "😀" })
    }
}
