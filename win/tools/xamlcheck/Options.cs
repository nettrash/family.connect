namespace XamlCheck;

/// <summary>Command line. One positional argument, the repo root; defaults to the nearest ancestor of the cwd holding FamilyConnect.slnx.</summary>
sealed class Options
{
    public string RepoRoot { get; private set; } = "";
    public string AppDir { get; private set; } = "";
    public string CoreCsproj { get; private set; } = "";
    public string ShadowDir { get; private set; } = "";
    public string Configuration { get; private set; } = "Debug";
    public string Platform { get; private set; } = "x64";
    public string Rid { get; private set; } = "win-x64";
    public bool Build { get; private set; } = true;
    public bool Verbose { get; private set; }

    const string Usage = """
        usage: xamlcheck [<repo-root>] [options]

          --app <dir>            app project directory, relative to the root   (src/FamilyConnect.App)
          --core <csproj>        Core project the app references; skipped if absent (src/FamilyConnect.Core/FamilyConnect.Core.csproj)
          --shadow-dir <dir>     where the shadow project is written; never inside the repo
                                 ($XAMLCHECK_SHADOW_DIR, else <tmp>/xamlcheck-shadow)
          -c, --configuration    Debug (default) or Release
          --platform <p>         x64 (default) or ARM64      --rid <r>   win-x64 (default) or win-arm64
          --no-build             lint and write the stubs only
          -v, --verbose          show the dotnet invocations, the resolved assemblies and every x:Name

        exit code: 0 clean, 1 lint or build errors, 2 usage / environment problem
        """;

    public static Options? Parse(string[] args)
    {
        var o = new Options();
        string? root = null;
        string app = "src/FamilyConnect.App", core = "src/FamilyConnect.Core/FamilyConnect.Core.csproj";
        string? shadow = Environment.GetEnvironmentVariable("XAMLCHECK_SHADOW_DIR");
        try
        {
            for (int i = 0; i < args.Length; i++)
            {
                string Next() => i + 1 < args.Length ? args[++i] : throw new ArgumentException($"{args[i]} needs a value");
                switch (args[i])
                {
                    case "--app": app = Next(); break;
                    case "--core": core = Next(); break;
                    case "--shadow-dir": shadow = Next(); break;
                    case "-c": case "--configuration": o.Configuration = Next(); break;
                    case "--platform": o.Platform = Next(); break;
                    case "--rid": o.Rid = Next(); break;
                    case "--no-build": o.Build = false; break;
                    case "-v": case "--verbose": o.Verbose = true; break;
                    case "-h": case "--help": Console.WriteLine(Usage); return null;
                    default:
                        if (args[i].StartsWith('-') || root is not null) throw new ArgumentException($"unexpected argument '{args[i]}'");
                        root = args[i];
                        break;
                }
            }
        }
        catch (ArgumentException e)
        {
            Console.Error.WriteLine($"xamlcheck: {e.Message}");
            Console.Error.WriteLine(Usage);
            return null;
        }

        root ??= FindRoot(Directory.GetCurrentDirectory());
        if (root is null)
        {
            Console.Error.WriteLine("xamlcheck: no repo root given and no FamilyConnect.slnx above the current directory");
            return null;
        }
        o.RepoRoot = Path.GetFullPath(root);
        o.AppDir = Path.GetFullPath(Path.Combine(o.RepoRoot, app));
        o.CoreCsproj = Path.GetFullPath(Path.Combine(o.RepoRoot, core));
        o.ShadowDir = Path.GetFullPath(string.IsNullOrEmpty(shadow) ? Path.Combine(Path.GetTempPath(), "xamlcheck-shadow") : shadow);
        return o;
    }

    static string? FindRoot(string start)
    {
        for (var d = new DirectoryInfo(start); d is not null; d = d.Parent)
            if (File.Exists(Path.Combine(d.FullName, "FamilyConnect.slnx"))) return d.FullName;
        return null;
    }
}

sealed class Log(bool verbose)
{
    public void Info(string message) { if (verbose) Console.Error.WriteLine($"xamlcheck: {message}"); }
    public void Warn(string message) => Console.Error.WriteLine($"xamlcheck: warning: {message}");
    public void Fatal(string message) => Console.Error.WriteLine($"xamlcheck: error: {message}");
}
