//
//  MessageMarkdown.swift
//  FamilyConnect
//
//  Markdown in a chat bubble: the small subset people actually type.
//
//  **bold**, *italic*, `code`, ~~strike~~, [label](url), \escapes, and
//  ```fenced``` blocks. Nothing else. Headings, lists, quotes and tables
//  are deliberately absent — they turn a balloon into a column of blocks,
//  and a column of blocks is what would break the two things this bubble
//  depends on being ONE `Text`: the `\.openURL` arbitration that lets a
//  double-tap over a link land the quick heart, and the single laid-out
//  string the link hit test resolves against.
//
//  THE WHOLE OUTPUT IS ONE ATTRIBUTED STRING, and that is the constraint,
//  not a simplification. A fenced block is therefore a monospaced RUN
//  rather than a scrolling box: it reads as code, it wraps like text, and
//  it costs the bubble nothing structurally.
//
//  This is a RENDERING convention, not a wire format (docs/protocol.md
//  states it explicitly): `body` is plain text on the wire, the server
//  neither parses nor validates any of it, the 4000-character limit counts
//  what was typed, and a client that renders none of this shows the source
//  — which is still exactly what was written.
//
//  The inline half leans on Foundation's own parser, exactly as the `md`
//  apps do (md/md/MarkdownInline.swift): it is correct, ships in the SDK,
//  and hand-rolling a tokenizer to match CommonMark's emphasis rules is a
//  well-known way to get `snake_case` wrong. Android has no equivalent and
//  carries a hand-written one; the SUBSET is what the two share, not the
//  implementation.
//

import Foundation
import SwiftUI

nonisolated enum MessageMarkdown {

    /// Render a message body.
    ///
    /// Fenced blocks are lifted out FIRST and never inline-parsed: the
    /// whole point of a code fence is that what is inside it is not markup.
    static func render(_ body: String) -> AttributedString {
        var out = AttributedString()
        for segment in segments(of: body) {
            switch segment {
            case .text(let text):
                out.append(inline(text))
            case .code(let code):
                var run = AttributedString(code)
                run.font = .system(.body, design: .monospaced)
                out.append(run)
            }
        }
        return out
    }

    /// True when rendering would change anything at all.
    ///
    /// Lets a caller skip the work — and, more importantly, lets tests say
    /// "this ordinary sentence is left completely alone", which is the
    /// property that matters most: most messages are not markdown, and a
    /// renderer that quietly rewrites them is worse than no renderer.
    static func isMarkdown(_ body: String) -> Bool {
        String(render(body).characters) != body
    }

    // MARK: - Fences

    private enum Segment {
        case text(String)
        case code(String)
    }

    /// The fence marker. Three backticks, at the start of a line.
    private static let fence = "```"

    /// Split a body into alternating plain and fenced-code segments.
    ///
    /// A fence must OPEN at the start of a line and CLOSE at the start of a
    /// line, which is what stops a stray triple-backtick in the middle of a
    /// sentence swallowing the rest of the message. An unclosed fence is
    /// not a fence at all: the text stays as it was typed, because somebody
    /// mid-way through writing one should not watch their message turn into
    /// a code block as they type the third backtick.
    private static func segments(of body: String) -> [Segment] {
        guard body.contains(fence) else { return [.text(body)] }
        let lines = body.components(separatedBy: "\n")
        var result: [Segment] = []
        var plain: [String] = []
        var code: [String] = []
        var inFence = false
        for line in lines {
            let opensOrCloses = line.hasPrefix(fence)
            if !inFence, opensOrCloses {
                // The language tag on an opening fence is dropped: nothing
                // here highlights syntax, so it would only be noise.
                inFence = true
                continue
            }
            if inFence, opensOrCloses {
                inFence = false
                result.append(.text(plain.joined(separator: "\n")))
                plain = []
                result.append(.code(code.joined(separator: "\n")))
                code = []
                // Keep the newline that ended the fence, so text after a
                // block does not run onto its last line.
                plain.append("")
                continue
            }
            if inFence {
                code.append(line)
            } else {
                plain.append(line)
            }
        }
        if inFence {
            // Never closed — put it back exactly as it was typed.
            return [.text(body)]
        }
        result.append(.text(plain.joined(separator: "\n")))
        return result.filter { segment in
            if case .text(let text) = segment { return !text.isEmpty }
            return true
        }
    }

    // MARK: - Inline

    /// Foundation's parser, held to inline syntax only.
    ///
    /// `.inlineOnlyPreservingWhitespace` is what keeps line breaks and stops
    /// it re-discovering block structure — without it a message beginning
    /// "# " would silently become a heading and a line starting "- " a list
    /// item, neither of which this bubble is prepared to lay out.
    private static func inline(_ text: String) -> AttributedString {
        var options = AttributedString.MarkdownParsingOptions()
        options.interpretedSyntax = .inlineOnlyPreservingWhitespace
        options.allowsExtendedAttributes = true
        // A half-typed `[label](` must render as the characters it is, not
        // vanish.
        options.failurePolicy = .returnPartiallyParsedIfPossible
        guard let parsed = try? AttributedString(markdown: text, options: options) else {
            return AttributedString(text)
        }
        return parsed
    }
}
