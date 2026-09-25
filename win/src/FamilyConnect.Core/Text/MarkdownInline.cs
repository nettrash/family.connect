namespace FamilyConnect.Core;

public static partial class Markdown
{
    private enum NodeKind
    {
        Root,
        Text,
        SoftBreak,
        LineBreak,
        Code,
        Html,
        Emphasis,
        Strong,
        Strikethrough,
        Link,
        Image,
        /// <summary>Apple's <c>^[text](attributes)</c>: the text, attributes not drawn.</summary>
        Attributes,
    }

    private sealed class Node(NodeKind kind, byte[] text)
    {
        public NodeKind Kind = kind;
        /// <summary>The characters of a Text, Code or Html node.</summary>
        public byte[] Text = text;
        public byte[] Url = [];
        public byte[] Title = [];
        public int Parent = -1;
        public int Prev = -1;
        public int Next = -1;
        public int FirstChild = -1;
        public int LastChild = -1;
    }

    private enum BracketKind
    {
        Link,
        Image,
        Attributes,
    }

    /// <summary>A <c>[</c>, <c>![</c> or <c>^[</c> waiting for its <c>]</c>.</summary>
    private sealed class Bracket(BracketKind kind, int node, int position)
    {
        public readonly BracketKind Kind = kind;
        public readonly int Node = node;
        /// <summary>Just past the opener — the stack bottom for the emphasis inside it.</summary>
        public readonly int Position = position;
        public bool BracketAfter;
    }

    /// <summary>A run of <c>*</c>, <c>_</c> or <c>~</c> that may open or close.</summary>
    private sealed class Delimiter
    {
        public byte Character;
        public bool CanOpen;
        public bool CanClose;
        /// <summary>The run's ORIGINAL length: the rule of three counts what was typed.</summary>
        public int Length;
        public int Node;
        public int Position;
        public int Prev = -1;
        public int Next = -1;
    }

    /// <summary>One cmark inline parse: the subject, the node tree, and the delimiter and bracket stacks.</summary>
    private sealed class Subject(byte[] text, Dictionary<string, (byte[] Url, byte[] Title)> references)
    {
        public const int Root = 0;

        /// <summary>cmark's <c>MAXBACKTICKS</c>: longer runs never open a code span.</summary>
        private const int MaxBackticks = 1000;

        private readonly List<Node> nodes = [new Node(NodeKind.Root, [])];
        private readonly List<Delimiter> delimiters = [];
        private readonly List<Bracket> brackets = [];
        private readonly int[] backticks = new int[MaxBackticks + 1];
        private int pos;
        private int lastDelimiter = -1;
        /// <summary>cmark 0.30's <c>no_link_openers</c>: set when a link closes, cleared by the next <c>[</c> or <c>^[</c>.</summary>
        private bool noLinkOpeners = true;
        private bool scannedForBackticks;

        // ---- tree plumbing (cmark's node.c) --------------------------------------------------------------------

        private int Add(NodeKind kind, byte[]? content = null)
        {
            nodes.Add(new Node(kind, content ?? []));
            return nodes.Count - 1;
        }

        private int TextNode(ReadOnlySpan<byte> content) => Add(NodeKind.Text, content.ToArray());

        private void Unlink(int node)
        {
            var n = nodes[node];
            if (n.Prev >= 0)
            {
                nodes[n.Prev].Next = n.Next;
            }
            if (n.Next >= 0)
            {
                nodes[n.Next].Prev = n.Prev;
            }
            if (n.Parent >= 0)
            {
                if (nodes[n.Parent].FirstChild == node)
                {
                    nodes[n.Parent].FirstChild = n.Next;
                }
                if (nodes[n.Parent].LastChild == node)
                {
                    nodes[n.Parent].LastChild = n.Prev;
                }
            }
            n.Parent = n.Prev = n.Next = -1;
        }

        private void AppendChild(int parent, int child)
        {
            Unlink(child);
            var last = nodes[parent].LastChild;
            nodes[child].Parent = parent;
            nodes[child].Prev = last;
            if (last >= 0)
            {
                nodes[last].Next = child;
            }
            else
            {
                nodes[parent].FirstChild = child;
            }
            nodes[parent].LastChild = child;
        }

        private void InsertBefore(int node, int sibling)
        {
            Unlink(sibling);
            var parent = nodes[node].Parent;
            var prev = nodes[node].Prev;
            nodes[sibling].Parent = parent;
            nodes[sibling].Prev = prev;
            nodes[sibling].Next = node;
            nodes[node].Prev = sibling;
            if (prev >= 0)
            {
                nodes[prev].Next = sibling;
            }
            else if (parent >= 0)
            {
                nodes[parent].FirstChild = sibling;
            }
        }

        private void InsertAfter(int node, int sibling)
        {
            Unlink(sibling);
            var parent = nodes[node].Parent;
            var next = nodes[node].Next;
            nodes[sibling].Parent = parent;
            nodes[sibling].Prev = node;
            nodes[sibling].Next = next;
            nodes[node].Next = sibling;
            if (next >= 0)
            {
                nodes[next].Prev = sibling;
            }
            else if (parent >= 0)
            {
                nodes[parent].LastChild = sibling;
            }
        }

        /// <summary>Move every sibling after <paramref name="from"/> (up to, not including, <paramref name="until"/>) into the container.</summary>
        private void AdoptSiblings(int from, int until, int container)
        {
            var current = nodes[from].Next;
            while (current >= 0 && current != until)
            {
                var next = nodes[current].Next;
                AppendChild(container, current);
                current = next;
            }
        }

        private int TextLength(int node) => nodes[node].Kind == NodeKind.Text ? nodes[node].Text.Length : 0;

        /// <summary>Shorten a delimiter run's text node. Runs are ASCII.</summary>
        private void SetTextLength(int node, int length)
        {
            if (nodes[node].Kind == NodeKind.Text)
            {
                nodes[node].Text = nodes[node].Text[..length];
            }
        }

        // ---- reading the subject ------------------------------------------------------------------------------

        private int Peek() => pos < text.Length ? text[pos] : -1;

        private int ByteAt(int at) => at < text.Length ? text[at] : -1;

        /// <summary>The character before <paramref name="at"/>, or <c>\n</c> at the start.</summary>
        private int CharBefore(int at) => at == 0 ? '\n' : RustChar.Before(text, at).Scalar;

        /// <summary>The character at <paramref name="at"/>, or <c>\n</c> at the end.</summary>
        private int CharAt(int at) => at >= text.Length ? '\n' : RustChar.At(text, at);

        /// <summary>cmark's <c>parse_inlines</c>: read everything, then resolve emphasis.</summary>
        public void Parse()
        {
            while (pos < text.Length)
            {
                var node = ParseInline();
                if (node >= 0)
                {
                    AppendChild(Root, node);
                }
            }
            ProcessEmphasis(0);
            brackets.Clear();
            lastDelimiter = -1;
        }

        private int ParseInline()
        {
            var c = text[pos];
            switch (c)
            {
                case (byte)'\n':
                    pos++;
                    return Add(NodeKind.SoftBreak);
                case (byte)'`':
                    return HandleBackticks();
                case (byte)'\\':
                    return HandleBackslash();
                case (byte)'&':
                    return HandleEntity();
                case (byte)'<':
                    return HandlePointyBrace();
                case (byte)'*' or (byte)'_':
                    return HandleDelimiter(c);
                case (byte)'~':
                    return HandleTilde();
                case (byte)'[':
                {
                    pos++;
                    var node = TextNode("["u8);
                    PushBracket(BracketKind.Link, node);
                    return node;
                }
                case (byte)']':
                    return HandleCloseBracket();
                case (byte)'!':
                {
                    pos++;
                    // cmark-gfm keeps `![^` from opening an image: `![^a](x)` is "!" and a link.
                    if (Peek() == '[' && ByteAt(pos + 1) != '^')
                    {
                        pos++;
                        var node = TextNode("!["u8);
                        PushBracket(BracketKind.Image, node);
                        return node;
                    }
                    return TextNode("!"u8);
                }
                case (byte)'^':
                {
                    if (ByteAt(pos + 1) == '[')
                    {
                        pos += 2;
                        var node = TextNode("^["u8);
                        PushBracket(BracketKind.Attributes, node);
                        return node;
                    }
                    pos++;
                    return TextNode("^"u8);
                }
                case (byte)':':
                    return UrlMatch() is { } url ? url : TextRun();
                case (byte)'w':
                    return WwwMatch() is { } www ? www : TextRun();
                default:
                    return TextRun();
            }
        }

        /// <summary>Plain text up to the next byte something else might claim — every special byte is ASCII.</summary>
        private int TextRun()
        {
            var start = pos;
            var end = start + 1;
            while (end < text.Length && !IsSpecial(text[end]))
            {
                end++;
            }
            pos = end;
            return TextNode(text.AsSpan(start, end - start));
        }

        private int HandleBackticks()
        {
            var ticksStart = pos;
            while (Peek() == '`')
            {
                pos++;
            }
            var open = pos - ticksStart;
            var start = pos;
            if (ScanToClosingBackticks(open) is not { } end)
            {
                pos = start;
                return TextNode(text.AsSpan(ticksStart, start - ticksStart));
            }
            var code = NormalizeCode(text.AsSpan(start, end - open - start));
            pos = end;
            return Add(NodeKind.Code, code);
        }

        /// <summary>The position after a backtick run exactly <paramref name="open"/> long, or null.</summary>
        private int? ScanToClosingBackticks(int open)
        {
            if (open > MaxBackticks)
            {
                return null;
            }
            if (scannedForBackticks && backticks[open] <= pos)
            {
                return null;
            }
            while (true)
            {
                while (pos < text.Length && text[pos] != (byte)'`')
                {
                    pos++;
                }
                if (pos >= text.Length)
                {
                    break;
                }
                var count = 0;
                while (Peek() == '`')
                {
                    pos++;
                    count++;
                }
                if (count <= MaxBackticks)
                {
                    backticks[count] = pos - count;
                }
                if (count == open)
                {
                    return pos;
                }
            }
            scannedForBackticks = true;
            return null;
        }

        /// <summary><c>\</c> escapes ASCII punctuation, makes a hard break before a newline, and is itself anywhere else.</summary>
        private int HandleBackslash()
        {
            pos++;
            var c = Peek();
            if (c >= 0 && RustChar.IsAsciiPunctuation(c))
            {
                pos++;
                return TextNode(text.AsSpan(pos - 1, 1));
            }
            if (c == '\n')
            {
                pos++;
                return Add(NodeKind.LineBreak);
            }
            return TextNode("\\"u8);
        }

        private int HandleEntity()
        {
            pos++;
            if (UnescapeEntity(text.AsSpan(pos)) is { } entity)
            {
                pos += entity.Length;
                return TextNode(entity.Decoded);
            }
            return TextNode("&"u8);
        }

        /// <summary><c>&lt;</c>: an autolink, an email autolink, raw HTML, or the character.</summary>
        private int HandlePointyBrace()
        {
            pos++;
            if (ScanAutolinkUri(text, pos) is { } uri)
            {
                var contents = text.AsSpan(pos, uri - 1).ToArray();
                pos += uri;
                return MakeAutolink(contents, isEmail: false);
            }
            if (ScanAutolinkEmail(text, pos) is { } email)
            {
                var contents = text.AsSpan(pos, email - 1).ToArray();
                pos += email;
                return MakeAutolink(contents, isEmail: true);
            }
            if (ScanHtmlTag(text, pos) is { } html)
            {
                var contents = text.AsSpan(pos - 1, html + 1).ToArray();
                pos += html;
                return Add(NodeKind.Html, contents);
            }
            return TextNode("<"u8);
        }

        private int MakeAutolink(byte[] contents, bool isEmail)
        {
            var trimmed = TrimCmarkSpace(contents);
            var url = new List<byte>();
            if (isEmail && !trimmed.IsEmpty)
            {
                url.AddRange("mailto:"u8);
            }
            url.AddRange(UnescapeHtml(trimmed));
            var link = Add(NodeKind.Link);
            nodes[link].Url = [.. url];
            AppendChild(link, TextNode(UnescapeHtml(contents)));
            return link;
        }

        /// <summary>
        /// <c>*</c> and <c>_</c> runs (cmark's <c>handle_delim</c>). Tildes are TRANSPARENT to what flanks the run; a walk
        /// back that runs into a tilde at position 0 sees the start of the text.
        /// </summary>
        private int HandleDelimiter(byte c)
        {
            var start = pos;
            int before;
            if (start == 0)
            {
                before = '\n';
            }
            else
            {
                var at = start - 1;
                while (at > 0 && (RustChar.IsContinuation(text[at]) || text[at] == (byte)'~'))
                {
                    at--;
                }
                before = at == 0 && text[0] == (byte)'~' ? '\n' : RustChar.DecodeAt(text, at) ?? '\n';
            }
            while (Peek() == c)
            {
                pos++;
            }
            var after = pos;
            while (ByteAt(after) == '~')
            {
                after++;
            }
            var afterChar = after >= text.Length ? '\n' : RustChar.DecodeAt(text, after) ?? '\n';
            var (left, right) = Flanking(before, afterChar);
            var (canOpen, canClose) = c == (byte)'_'
                ? (left && (!right || IsCmarkPunctuation(before)), right && (!left || IsCmarkPunctuation(afterChar)))
                : (left, right);
            var node = TextNode(text.AsSpan(start, pos - start));
            if (canOpen || canClose)
            {
                PushDelimiter(c, canOpen, canClose, node);
            }
            return node;
        }

        /// <summary><c>~</c> runs — cmark-gfm's strikethrough: one or two tildes open and close, three or more are tildes.</summary>
        private int HandleTilde()
        {
            var before = CharBefore(pos);
            var start = pos;
            // The extension reads into a 100-byte buffer and stops at 101.
            while (Peek() == '~' && pos - start <= 100)
            {
                pos++;
            }
            var count = pos - start;
            var after = CharAt(pos);
            var (left, right) = Flanking(before, after);
            var node = TextNode(text.AsSpan(start, count));
            if ((left || right) && count is 1 or 2)
            {
                PushDelimiter((byte)'~', left, right, node);
            }
            return node;
        }

        // ---- delimiters and emphasis (cmark's inlines.c) ------------------------------------------------------

        private void PushDelimiter(byte character, bool canOpen, bool canClose, int node)
        {
            var index = delimiters.Count;
            delimiters.Add(new Delimiter
            {
                Character = character,
                CanOpen = canOpen,
                CanClose = canClose,
                Length = TextLength(node),
                Node = node,
                Position = pos,
                Prev = lastDelimiter,
            });
            if (lastDelimiter >= 0)
            {
                delimiters[lastDelimiter].Next = index;
            }
            lastDelimiter = index;
        }

        private void RemoveDelimiter(int index)
        {
            var (prev, next) = (delimiters[index].Prev, delimiters[index].Next);
            if (next >= 0)
            {
                delimiters[next].Prev = prev;
            }
            else
            {
                lastDelimiter = prev;
            }
            if (prev >= 0)
            {
                delimiters[prev].Next = next;
            }
        }

        /// <summary>CommonMark's "process emphasis", as cmark writes it.</summary>
        private void ProcessEmphasis(int stackBottom)
        {
            var openersBottom = new int[3, 3];
            for (var i = 0; i < 3; i++)
            {
                for (var j = 0; j < 3; j++)
                {
                    openersBottom[i, j] = stackBottom;
                }
            }

            var closer = -1;
            var candidate = lastDelimiter;
            while (candidate >= 0 && delimiters[candidate].Position >= stackBottom)
            {
                closer = candidate;
                candidate = delimiters[candidate].Prev;
            }

            while (closer >= 0)
            {
                var closing = delimiters[closer];
                if (!closing.CanClose)
                {
                    closer = closing.Next;
                    continue;
                }
                var (character, canOpen, length) = (closing.Character, closing.CanOpen, closing.Length);
                var slot = character switch { (byte)'*' => 0, (byte)'_' => 1, _ => 2 };
                var bottom = openersBottom[length % 3, slot];
                var opener = closing.Prev;
                var found = false;
                while (opener >= 0)
                {
                    var d = delimiters[opener];
                    if (d.Position < stackBottom || d.Position < bottom)
                    {
                        break;
                    }
                    // The rule of three.
                    if (d.CanOpen && d.Character == character && (!(canOpen || d.CanClose) || length % 3 == 0 || (d.Length + length) % 3 != 0))
                    {
                        found = true;
                        break;
                    }
                    opener = d.Prev;
                }
                var oldCloser = closer;
                closer = found
                    ? character == (byte)'~' ? InsertStrikethrough(opener, closer) : InsertEmphasis(opener, closer)
                    : closing.Next;
                if (!found)
                {
                    openersBottom[length % 3, slot] = delimiters[oldCloser].Position;
                    if (!delimiters[oldCloser].CanOpen)
                    {
                        RemoveDelimiter(oldCloser);
                    }
                }
            }

            while (lastDelimiter >= 0 && delimiters[lastDelimiter].Position >= stackBottom)
            {
                RemoveDelimiter(lastDelimiter);
            }
        }

        /// <summary>cmark's <c>S_insert_emph</c>: two characters from each side make strong, one makes emphasis.</summary>
        private int InsertEmphasis(int opener, int closer)
        {
            var openerNode = delimiters[opener].Node;
            var closerNode = delimiters[closer].Node;
            var openerChars = TextLength(openerNode);
            var closerChars = TextLength(closerNode);
            var used = closerChars >= 2 && openerChars >= 2 ? 2 : 1;
            SetTextLength(openerNode, openerChars - used);
            SetTextLength(closerNode, closerChars - used);

            var between = delimiters[closer].Prev;
            while (between >= 0 && between != opener)
            {
                var prev = delimiters[between].Prev;
                RemoveDelimiter(between);
                between = prev;
            }

            var emphasis = Add(used == 1 ? NodeKind.Emphasis : NodeKind.Strong);
            AdoptSiblings(openerNode, closerNode, emphasis);
            InsertAfter(openerNode, emphasis);

            if (openerChars == used)
            {
                Unlink(openerNode);
                RemoveDelimiter(opener);
            }
            if (closerChars == used)
            {
                Unlink(closerNode);
                var next = delimiters[closer].Next;
                RemoveDelimiter(closer);
                return next;
            }
            return closer;
        }

        /// <summary>The strikethrough extension's <c>insert</c>: only runs of the SAME length pair up; the delimiters between are spent.</summary>
        private int InsertStrikethrough(int opener, int closer)
        {
            var next = delimiters[closer].Next;
            var openerNode = delimiters[opener].Node;
            var closerNode = delimiters[closer].Node;
            if (TextLength(openerNode) == TextLength(closerNode))
            {
                nodes[openerNode].Kind = NodeKind.Strikethrough;
                nodes[openerNode].Text = [];
                AdoptSiblings(openerNode, closerNode, openerNode);
                Unlink(closerNode);
            }
            var current = closer;
            while (current >= 0 && current != opener)
            {
                var prev = delimiters[current].Prev;
                RemoveDelimiter(current);
                current = prev;
            }
            RemoveDelimiter(opener);
            return next;
        }

        // ---- brackets, links and images -----------------------------------------------------------------------

        private void PushBracket(BracketKind kind, int node)
        {
            if (brackets.Count > 0)
            {
                brackets[^1].BracketAfter = true;
            }
            // Measured: `[` and `^[` re-enable older link openers, `![` does not.
            if (kind != BracketKind.Image)
            {
                noLinkOpeners = false;
            }
            brackets.Add(new Bracket(kind, node, pos));
        }

        /// <summary>cmark's <c>handle_close_bracket</c>: a link, an image, an attribute span, or a literal <c>]</c>. -1 when nothing is appended.</summary>
        private int HandleCloseBracket()
        {
            pos++;
            var initialPos = pos;
            if (brackets.Count == 0)
            {
                return TextNode("]"u8);
            }
            var opener = brackets.Count - 1;
            var kind = brackets[opener].Kind;
            if (kind == BracketKind.Link && noLinkOpeners)
            {
                brackets.RemoveAt(opener);
                return TextNode("]"u8);
            }

            (byte[] Url, byte[] Title)? matched;
            if (kind == BracketKind.Attributes)
            {
                matched = null;
                if (Peek() == '(' && ScanAttributes(text, pos + 1) is { } length)
                {
                    pos += 1 + length + 1;
                    matched = ([], []);
                }
                // Apple's fork reads a `[label]` straight after and throws it away, span or no span.
                if (LinkLabel(text, pos) is { } label)
                {
                    pos = label.After;
                }
                if (matched is null)
                {
                    // Unlike a link, the position is NOT rewound.
                    brackets.RemoveAt(opener);
                    return TextNode("]"u8);
                }
            }
            else
            {
                matched = InlineDestination();
                if (matched is null)
                {
                    pos = initialPos;
                    matched = ReferenceDestination(opener, initialPos);
                }
            }

            if (matched is not { } destination)
            {
                brackets.RemoveAt(opener);
                pos = initialPos;
                return TextNode("]"u8);
            }

            var node = Add(kind switch
            {
                BracketKind.Link => NodeKind.Link,
                BracketKind.Image => NodeKind.Image,
                _ => NodeKind.Attributes,
            });
            nodes[node].Url = destination.Url;
            nodes[node].Title = destination.Title;
            var openerNode = brackets[opener].Node;
            InsertBefore(openerNode, node);
            AdoptSiblings(openerNode, -1, node);
            Unlink(openerNode);
            ProcessEmphasis(brackets[opener].Position);
            brackets.RemoveAt(opener);
            // Image and attribute spans leave older openers alone.
            if (kind == BracketKind.Link)
            {
                noLinkOpeners = true;
            }
            return -1;
        }

        /// <summary><c>(destination "title")</c> right after the <c>]</c>, advancing past it.</summary>
        private (byte[] Url, byte[] Title)? InlineDestination()
        {
            if (Peek() != '(')
            {
                return null;
            }
            var urlStart = pos + 1 + ScanSpacechars(text, pos + 1);
            if (ScanLinkUrl(text, urlStart) is not { } url)
            {
                return null;
            }
            var endUrl = urlStart + url.Length;
            var startTitle = endUrl + ScanSpacechars(text, endUrl);
            var endTitle = startTitle == endUrl ? startTitle : startTitle + ScanLinkTitle(text, startTitle);
            var endAll = endTitle + ScanSpacechars(text, endTitle);
            if (ByteAt(endAll) != ')')
            {
                return null;
            }
            pos = endAll + 1;
            return (CleanUrl(text.AsSpan(url.Start, url.End - url.Start)), CleanTitle(text.AsSpan(startTitle, endTitle - startTitle)));
        }

        /// <summary><c>[label]</c>, <c>[]</c> or nothing after the <c>]</c>, looked up in the definitions.</summary>
        private (byte[] Url, byte[] Title)? ReferenceDestination(int opener, int initialPos)
        {
            byte[]? label = null;
            if (LinkLabel(text, pos) is { } found)
            {
                pos = found.After;
                var raw = TrimCmarkSpace(text.AsSpan(found.Start, found.End - found.Start));
                if (!raw.IsEmpty)
                {
                    label = raw.ToArray();
                }
            }
            else
            {
                pos = initialPos;
            }
            if (label is null && !brackets[opener].BracketAfter)
            {
                // A shortcut `[a]` or a collapsed `[a][]`: the text is the label.
                var start = brackets[opener].Position;
                label = text[start..(initialPos - 1)];
            }
            if (label is null || NormalizeLabel(label) is not { } key || !references.TryGetValue(key, out var definition))
            {
                return null;
            }
            return definition;
        }

        // ---- the GFM autolink extension -----------------------------------------------------------------------

        /// <summary>cmark-gfm's <c>in_bracket</c>: a <c>[</c> or <c>![</c> still open. Apple's <c>^[</c> does not count.</summary>
        private bool InBracket() => brackets.Any(bracket => bracket.Kind != BracketKind.Attributes);

        /// <summary><c>http://</c>, <c>https://</c> and <c>ftp://</c> URLs, found at their <c>:</c>.</summary>
        private int? UrlMatch()
        {
            if (InBracket())
            {
                return null;
            }
            var maxRewind = pos;
            ReadOnlySpan<byte> data = text.AsSpan(pos);
            var size = data.Length;
            if (size < 4 || data[1] != (byte)'/' || data[2] != (byte)'/')
            {
                return null;
            }
            var rewind = 0;
            while (rewind < maxRewind && RustChar.IsAsciiLetter(text[maxRewind - rewind - 1]))
            {
                rewind++;
            }
            if (!AutolinkIsSafe(text.AsSpan(maxRewind - rewind)))
            {
                return null;
            }
            var domain = CheckDomain(data[3..], allowShort: true);
            if (domain == 0)
            {
                return null;
            }
            var linkEnd = 3 + domain;
            while (linkEnd < size && !IsCmarkSpaceByte(data[linkEnd]) && data[linkEnd] != (byte)'<')
            {
                linkEnd++;
            }
            linkEnd = AutolinkDelim(data, linkEnd);
            if (linkEnd == 0)
            {
                return null;
            }
            pos = maxRewind + linkEnd;
            Unput(rewind);
            var url = text[(maxRewind - rewind)..(maxRewind + linkEnd)];
            var link = Add(NodeKind.Link);
            nodes[link].Url = url;
            AppendChild(link, TextNode(url));
            return link;
        }

        /// <summary><c>www.</c> hosts, found at their <c>w</c>.</summary>
        private int? WwwMatch()
        {
            if (InBracket())
            {
                return null;
            }
            var maxRewind = pos;
            if (maxRewind > 0)
            {
                var before = text[maxRewind - 1];
                if (before is not ((byte)'*' or (byte)'_' or (byte)'~' or (byte)'(') && !IsCmarkSpaceByte(before))
                {
                    return null;
                }
            }
            ReadOnlySpan<byte> data = text.AsSpan(pos);
            var size = data.Length;
            if (size < 4 || !data.StartsWith("www."u8))
            {
                return null;
            }
            var linkEnd = CheckDomain(data, allowShort: false);
            if (linkEnd == 0)
            {
                return null;
            }
            while (linkEnd < size && !IsCmarkSpaceByte(data[linkEnd]) && data[linkEnd] != (byte)'<')
            {
                linkEnd++;
            }
            linkEnd = AutolinkDelim(data, linkEnd);
            if (linkEnd == 0)
            {
                return null;
            }
            pos = maxRewind + linkEnd;
            var host = text[maxRewind..(maxRewind + linkEnd)];
            var link = Add(NodeKind.Link);
            nodes[link].Url = [.. "http://"u8, .. host];
            AppendChild(link, TextNode(host));
            return link;
        }

        /// <summary>cmark's <c>cmark_node_unput</c>: the scheme letters a URL match rewound over come back off the text before it.</summary>
        private void Unput(int count)
        {
            var node = nodes[Root].LastChild;
            while (count > 0 && node >= 0 && nodes[node].Kind == NodeKind.Text)
            {
                var content = nodes[node].Text;
                if (content.Length < count)
                {
                    count -= content.Length;
                    nodes[node].Text = [];
                }
                else
                {
                    var keep = content.Length - count;
                    if (keep == content.Length || !RustChar.IsContinuation(content[keep]))
                    {
                        nodes[node].Text = content[..keep];
                    }
                    count = 0;
                }
                node = nodes[node].Prev;
            }
        }

        /// <summary>The extension's postprocess: email addresses in text not already inside a link, neighbouring text merged first.</summary>
        public void AutolinkEmails()
        {
            ConsolidateText(Root);
            var texts = new List<int>();
            CollectTextsOutsideLinks(Root, texts);
            foreach (var node in texts)
            {
                AutolinkEmailsIn(node);
            }
        }

        private void ConsolidateText(int parent)
        {
            var current = nodes[parent].FirstChild;
            while (current >= 0)
            {
                if (nodes[current].Kind == NodeKind.Text)
                {
                    while (nodes[current].Next is var next and >= 0 && nodes[next].Kind == NodeKind.Text)
                    {
                        nodes[current].Text = [.. nodes[current].Text, .. nodes[next].Text];
                        Unlink(next);
                    }
                }
                else
                {
                    ConsolidateText(current);
                }
                current = nodes[current].Next;
            }
        }

        private void CollectTextsOutsideLinks(int parent, List<int> output)
        {
            var current = nodes[parent].FirstChild;
            while (current >= 0)
            {
                switch (nodes[current].Kind)
                {
                    case NodeKind.Link:
                        break;
                    case NodeKind.Text:
                        output.Add(current);
                        break;
                    default:
                        CollectTextsOutsideLinks(current, output);
                        break;
                }
                current = nodes[current].Next;
            }
        }

        /// <summary>cmark-gfm's <c>postprocess_text</c>, index for index.</summary>
        private void AutolinkEmailsIn(int node)
        {
            if (nodes[node].Kind != NodeKind.Text || Array.IndexOf(nodes[node].Text, (byte)'@') < 0)
            {
                return;
            }
            var data = nodes[node].Text;
            var textNode = node;
            var start = 0;
            var offset = 0;
            var remaining = data.Length;

            while (offset < remaining)
            {
                var found = data.AsSpan(start + offset, remaining - offset).IndexOf((byte)'@');
                if (found < 0)
                {
                    break;
                }
                var maxRewind = found;
                // Set once per `@` the search found, NOT again when the scan restarts from a later `@`.
                var autoMailto = true;
                var isXmpp = false;
                var np = 0;
                var rewind = 0;
                var linkEnd = 0;
                var rescan = false;

                while (true)
                {
                    var at = start + offset + maxRewind;
                    rewind = 0;
                    while (rewind < maxRewind)
                    {
                        var c = data[at - rewind - 1];
                        if (RustChar.IsAsciiAlphanumeric(c) || c is (byte)'.' or (byte)'+' or (byte)'-' or (byte)'_')
                        {
                            rewind++;
                            continue;
                        }
                        if (c == (byte)':')
                        {
                            if (ValidateProtocol("mailto:"u8, data, at, rewind, maxRewind))
                            {
                                autoMailto = false;
                                rewind++;
                                continue;
                            }
                            if (ValidateProtocol("xmpp:"u8, data, at, rewind, maxRewind))
                            {
                                autoMailto = false;
                                isXmpp = true;
                                rewind++;
                                continue;
                            }
                        }
                        break;
                    }
                    if (rewind == 0)
                    {
                        offset += maxRewind + 1;
                        rescan = true;
                        break;
                    }
                    linkEnd = 1;
                    var again = false;
                    while (linkEnd < remaining - offset - maxRewind)
                    {
                        var c = data[at + linkEnd];
                        if (RustChar.IsAsciiAlphanumeric(c))
                        {
                            linkEnd++;
                            continue;
                        }
                        if (c == (byte)'@')
                        {
                            // Another `@`: start again from it.
                            offset += maxRewind + 1;
                            maxRewind = linkEnd - 1;
                            again = true;
                            break;
                        }
                        if (c == (byte)'.' && linkEnd < remaining - offset - maxRewind - 1 && RustChar.IsAsciiAlphanumeric(data[at + linkEnd + 1]))
                        {
                            np++;
                        }
                        else if (c == (byte)'/' && isXmpp)
                        {
                            // xmpp resources ride along.
                        }
                        else if (c != (byte)'-' && c != (byte)'_')
                        {
                            break;
                        }
                        linkEnd++;
                    }
                    if (!again)
                    {
                        break;
                    }
                }
                if (rescan)
                {
                    continue;
                }

                var address = start + offset + maxRewind;
                var last = data[address + linkEnd - 1];
                if (linkEnd < 2 || np == 0 || (!RustChar.IsAsciiLetter(last) && last != (byte)'.'))
                {
                    offset += maxRewind + linkEnd;
                    continue;
                }
                linkEnd = AutolinkDelim(data.AsSpan(address), linkEnd);
                if (linkEnd == 0)
                {
                    offset += maxRewind + 1;
                    continue;
                }

                var email = data[(address - rewind)..(address + linkEnd)];
                var link = Add(NodeKind.Link);
                nodes[link].Url = autoMailto ? [.. "mailto:"u8, .. email] : email;
                AppendChild(link, TextNode(email));
                InsertAfter(textNode, link);
                var post = TextNode(data.AsSpan(address + linkEnd, start + remaining - address - linkEnd));
                InsertAfter(link, post);
                nodes[textNode].Text = data[start..(address - rewind)];
                textNode = post;
                var consumed = offset + maxRewind + linkEnd;
                start += consumed;
                remaining -= consumed;
                offset = 0;
            }
        }

        // ---- Foundation's conversion into runs ----------------------------------------------------------------

        /// <summary>
        /// The tree into styled runs, the way Foundation builds its AttributedString: an inline style is SET on entering
        /// and CLEARED on leaving, never restored — so <c>*a *b* c*</c> draws " c" upright.
        /// </summary>
        public void Render(int node, ref MarkdownStyle style, MarkdownRuns output)
        {
            var n = nodes[node];
            switch (n.Kind)
            {
                case NodeKind.Root or NodeKind.Attributes:
                    RenderChildren(node, ref style, output);
                    break;
                case NodeKind.Text:
                    output.Push(n.Text, style);
                    break;
                case NodeKind.SoftBreak:
                    output.Push("\n"u8, style);
                    break;
                case NodeKind.LineBreak:
                    output.Push("\n"u8, style with { LineBreak = true });
                    break;
                case NodeKind.Code:
                    output.Push(n.Text, style with { Code = true });
                    break;
                case NodeKind.Html:
                    output.Push(n.Text, style with { Html = true });
                    break;
                case NodeKind.Emphasis:
                    style = style with { Emphasis = true };
                    RenderChildren(node, ref style, output);
                    style = style with { Emphasis = false };
                    break;
                case NodeKind.Strong:
                    style = style with { Strong = true };
                    RenderChildren(node, ref style, output);
                    style = style with { Strong = false };
                    break;
                case NodeKind.Strikethrough:
                    style = style with { Strikethrough = true };
                    RenderChildren(node, ref style, output);
                    style = style with { Strikethrough = false };
                    break;
                case NodeKind.Link:
                    // The label is FLATTENED: `[*a*](b)` is an unstyled "a".
                    output.Push([.. Flatten(node)], FoundationAcceptsUrl(n.Url) ? style with { Link = MakeLink(n.Url, n.Title) } : style);
                    break;
                case NodeKind.Image:
                {
                    var alt = Flatten(node);
                    if (alt.Count == 0)
                    {
                        alt.AddRange("￼"u8);
                    }
                    output.Push([.. alt], FoundationAcceptsUrl(n.Url) ? style with { Image = MakeLink(n.Url, n.Title) } : style);
                    break;
                }
            }
        }

        private void RenderChildren(int node, ref MarkdownStyle style, MarkdownRuns output)
        {
            var current = nodes[node].FirstChild;
            while (current >= 0)
            {
                Render(current, ref style, output);
                current = nodes[current].Next;
            }
        }

        /// <summary>A label's or alt text's plain text: soft breaks stay <c>\n</c>, a hard break vanishes.</summary>
        private List<byte> Flatten(int node)
        {
            var output = new List<byte>();
            var current = nodes[node].FirstChild;
            while (current >= 0)
            {
                switch (nodes[current].Kind)
                {
                    case NodeKind.Text or NodeKind.Code or NodeKind.Html:
                        output.AddRange(nodes[current].Text);
                        break;
                    case NodeKind.SoftBreak:
                        output.Add((byte)'\n');
                        break;
                    case NodeKind.LineBreak:
                        break;
                    default:
                        output.AddRange(Flatten(current));
                        break;
                }
                current = nodes[current].Next;
            }
            return output;
        }
    }
}
