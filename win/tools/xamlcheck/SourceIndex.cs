using System.Text;
using System.Text.RegularExpressions;

namespace XamlCheck;

/// <summary>
/// The app's (and Core's) *.cs, searched with regexes: partial classes, class declarations, handler
/// methods. Deliberately not a C# parser. Every search runs on <see cref="SourceFile.Code"/>, the
/// text with comments and string/char literal contents blanked (same length, same line breaks), so
/// a handler that survives only in a comment or a string is not mistaken for a declaration.
/// </summary>
sealed class SourceIndex
{
    public sealed record SourceFile(string Path, string Text, string Code);
    public enum ClassMatch { None, NamespaceDiffers, Exact }

    public IReadOnlyList<SourceFile> Files { get; }

    SourceIndex(List<SourceFile> files) => Files = files;

    public static SourceIndex Load(params string?[] dirs)
    {
        var files = new List<SourceFile>();
        foreach (var dir in dirs)
        {
            if (dir is null || !Directory.Exists(dir)) continue;
            foreach (var p in Directory.EnumerateFiles(dir, "*.cs", SearchOption.AllDirectories).Where(p => !IsBuildOutput(p, dir)).Order(StringComparer.Ordinal))
            {
                var text = File.ReadAllText(p);
                files.Add(new SourceFile(p, text, StripCommentsAndLiterals(text)));
            }
        }
        return new SourceIndex(files);
    }

    /// <summary>True for anything under a bin/ or obj/ directory below <paramref name="root"/>.</summary>
    public static bool IsBuildOutput(string path, string root)
    {
        var segments = Path.GetRelativePath(root, path).Split(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
        return segments.Take(segments.Length - 1).Any(s => s is "bin" or "obj");
    }

    /// <summary>Files holding <c>partial class Name</c> — the code-behind of an x:Class plus any other partial of it.</summary>
    public IReadOnlyList<SourceFile> PartialsOf(string className)
    {
        var rx = new Regex($@"\bpartial\s+class\s+{Regex.Escape(className)}\b");
        return Files.Where(f => rx.IsMatch(f.Code)).ToList();
    }

    /// <summary>Files declaring <c>class Name</c> / <c>record Name</c> in any namespace.</summary>
    public IReadOnlyList<SourceFile> ClassFiles(string className)
    {
        var rx = new Regex($@"\b(class|record)\s+{Regex.Escape(className)}\b");
        return Files.Where(f => rx.IsMatch(f.Code)).ToList();
    }

    /// <summary>
    /// <paramref name="files"/> plus, transitively, the files declaring the textual base class of
    /// <paramref name="className"/> (<c>class X : Base</c>) when that base is declared in the sources.
    /// The XAML compiler wires <c>this.Handler</c>, so a handler inherited from an app-declared
    /// base class is legal; one inherited from a WinUI type is not a case worth modelling.
    /// </summary>
    public IReadOnlyList<SourceFile> WithAncestors(IReadOnlyList<SourceFile> files, string className)
    {
        var result = new List<SourceFile>(files);
        var seen = new HashSet<string> { className };
        var frontier = new Queue<(IReadOnlyList<SourceFile> Files, string Name)>();
        frontier.Enqueue((files, className));
        while (frontier.Count > 0 && seen.Count < 16)
        {
            var (fs, name) = frontier.Dequeue();
            var rx = new Regex($@"\bclass\s+{Regex.Escape(name)}\b\s*(?:<[^<>{{]*>)?\s*:\s*([\w.]+)");
            foreach (var f in fs)
            {
                var m = rx.Match(f.Code);
                if (!m.Success) continue;
                var baseName = m.Groups[1].Value;
                var simple = baseName[(baseName.LastIndexOf('.') + 1)..];
                if (!seen.Add(simple)) continue;
                var baseFiles = ClassFiles(simple);
                result.AddRange(baseFiles.Where(b => !result.Contains(b)));
                frontier.Enqueue((baseFiles, simple));
            }
        }
        return result;
    }

    public static bool DeclaresNamespace(SourceFile file, string ns) =>
        ns.Length == 0 || Regex.IsMatch(file.Code, $@"\bnamespace\s+{Regex.Escape(ns)}\s*[;{{]");

    public ClassMatch FindClass(string ns, string name)
    {
        var rx = new Regex($@"\b(class|record|struct)\s+{Regex.Escape(name)}\b");
        var hits = Files.Where(f => rx.IsMatch(f.Code)).ToList();
        if (hits.Count == 0) return ClassMatch.None;
        return hits.Any(f => DeclaresNamespace(f, ns)) ? ClassMatch.Exact : ClassMatch.NamespaceDiffers;
    }

    // Tokens that can precede a *call* `Name(` and could otherwise pass for a return type.
    const string NotAReturnType =
        "return|await|new|throw|yield|else|in|is|as|not|and|or|default|checked|unchecked|goto|case|when|with|using|lock|" +
        "typeof|sizeof|nameof|stackalloc|ref|out|params|this|base|do|if|while|for|foreach|switch|try|catch|finally|" +
        "from|where|select|let|join|on|equals|into|orderby|ascending|descending|group|by";

    /// <summary>
    /// A method <em>declaration</em> named <paramref name="name"/>: a return type (<c>void</c>,
    /// <c>Task</c>, <c>Foo&lt;T&gt;</c>, <c>int[]</c>, <c>string?</c>, possibly namespace-qualified)
    /// followed by the name and <c>(</c>. A call — <c>=&gt; Name(</c>, <c>return Name(</c>,
    /// <c>await Name(</c>, <c>x = Name(</c>, <c>.Name(</c> — is not a declaration.
    /// </summary>
    public static bool HasMethod(IEnumerable<SourceFile> files, string name)
    {
        var rx = new Regex($@"\b(?!(?:{NotAReturnType})\b)\w+(?:<[^()]*>|\[\s*(?:,\s*)*\])*\??\s+{Regex.Escape(name)}\s*\(");
        return files.Any(f => rx.IsMatch(f.Code));
    }

    public static bool Mentions(IEnumerable<SourceFile> files, string pattern) => files.Any(f => Regex.IsMatch(f.Code, pattern));

    public static int LineOf(string text, string needle)
    {
        var at = text.IndexOf(needle, StringComparison.Ordinal);
        return at < 0 ? 1 : text.AsSpan(0, at).Count('\n') + 1;
    }

    /// <summary>
    /// Blanks // and /* */ comments and the contents of string and character literals (regular,
    /// verbatim, interpolated, raw), keeping every line break and the overall length so line
    /// numbers computed on the result hold for the original text. Interpolation holes are blanked
    /// with the rest of the string; a nested quote inside one only shortens the blanked span.
    /// </summary>
    internal static string StripCommentsAndLiterals(string s)
    {
        var sb = new StringBuilder(s.Length);
        int i = 0, n = s.Length;
        void Blank(int count) { for (var k = 0; k < count; k++) { sb.Append(s[i] == '\n' ? '\n' : ' '); i++; } }
        while (i < n)
        {
            var c = s[i];
            if (c == '/' && i + 1 < n && s[i + 1] == '/')
            {
                while (i < n && s[i] != '\n') Blank(1);
                continue;
            }
            if (c == '/' && i + 1 < n && s[i + 1] == '*')
            {
                Blank(2);
                while (i < n && !(s[i] == '*' && i + 1 < n && s[i + 1] == '/')) Blank(1);
                if (i < n) Blank(2);
                continue;
            }
            if (c == '"')
            {
                if (i + 2 < n && s[i + 1] == '"' && s[i + 2] == '"')
                {
                    // Raw string literal: closed by at least as many quotes as opened it.
                    var opening = 0;
                    while (i < n && s[i] == '"') { sb.Append('"'); i++; opening++; }
                    while (i < n)
                    {
                        if (s[i] != '"') { Blank(1); continue; }
                        var run = 0;
                        while (i + run < n && s[i + run] == '"') run++;
                        for (var k = 0; k < run; k++) { sb.Append('"'); i++; }
                        if (run >= opening) break;
                    }
                    continue;
                }
                var verbatim = (i > 0 && s[i - 1] == '@') || (i > 1 && s[i - 1] == '$' && s[i - 2] == '@');
                sb.Append('"'); i++;
                while (i < n)
                {
                    if (verbatim)
                    {
                        if (s[i] == '"')
                        {
                            if (i + 1 < n && s[i + 1] == '"') { Blank(2); continue; }
                            break;
                        }
                    }
                    else
                    {
                        if (s[i] == '\\' && i + 1 < n) { Blank(2); continue; }
                        if (s[i] == '"' || s[i] == '\n') break;
                    }
                    Blank(1);
                }
                if (i < n && s[i] == '"') { sb.Append('"'); i++; }
                continue;
            }
            if (c == '\'')
            {
                sb.Append('\''); i++;
                while (i < n && s[i] != '\'' && s[i] != '\n')
                {
                    if (s[i] == '\\' && i + 1 < n) { Blank(2); continue; }
                    Blank(1);
                }
                if (i < n && s[i] == '\'') { sb.Append('\''); i++; }
                continue;
            }
            sb.Append(c); i++;
        }
        return sb.ToString();
    }
}
