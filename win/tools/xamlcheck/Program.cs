// xamlcheck — see README.md. Five steps: shadow project → restore + resolved references →
// XAML lint against the metadata → stubs → shadow build. Exit 1 on any lint or build error.
using System.Diagnostics;
using XamlCheck;

var options = Options.Parse(args);
if (options is null) return 2;
var log = new Log(options.Verbose);
var clock = Stopwatch.StartNew();

if (!Directory.Exists(options.AppDir)) { log.Fatal($"app directory not found: {options.AppDir}"); return 2; }
var appCsproj = Directory.EnumerateFiles(options.AppDir, "*.csproj").Order(StringComparer.Ordinal).FirstOrDefault();
if (appCsproj is null) { log.Fatal($"no .csproj in {options.AppDir}"); return 2; }

var shadow = new ShadowProject(options, appCsproj, log);
shadow.Write();
log.Info($"shadow project {shadow.ProjectPath}");

// The exact reference set csc will see comes from the restored shadow project; the NuGet cache is the fallback.
var restored = shadow.Restore();
var references = restored ? shadow.ResolveReferencePaths() : [];
if (references.Count == 0)
{
    log.Warn("could not resolve the shadow project's references; using the newest assemblies in the NuGet cache instead");
    references = shadow.FallbackReferencePaths();
}
if (options.Verbose) foreach (var r in references.Where(r => !r.Contains("Microsoft.NETCore.App.Ref"))) log.Info($"reference {r}");

using var types = TypeIndex.Load(references, log);
if (types.Find("Microsoft.UI.Xaml.Controls.Grid") is null)
{
    log.Fatal("Microsoft.WinUI.dll is not among the references: has Microsoft.WindowsAppSDK ever been restored on this machine?");
    return 2;
}
log.Info($"{types.TypeCount} public types from {types.AssemblyCount} assemblies");

var sources = SourceIndex.Load(options.AppDir, Path.GetDirectoryName(options.CoreCsproj));
var lint = new XamlLint(types, sources, options, log).Run();
foreach (var d in lint.Diagnostics.OrderBy(d => d.File, StringComparer.Ordinal).ThenBy(d => d.Line))
    Console.WriteLine(d.Format(options.RepoRoot));
if (options.Verbose)
    foreach (var c in lint.Classes)
        foreach (var f in c.Fields) log.Info($"{c.FullName}.{f.Name}: {f.TypeFullName}");

StubWriter.Write(shadow.StubsPath, lint.Classes, options.RepoRoot);
log.Info($"stubs {shadow.StubsPath}");

var errors = lint.Diagnostics.Count(d => d.IsError);
var warnings = lint.Diagnostics.Count - errors;
var buildExit = 0;
string buildState;
if (!options.Build) buildState = "skipped";
else if (!restored) { buildState = "not run (restore failed)"; buildExit = 1; }
else { buildExit = shadow.Build(); buildState = buildExit == 0 ? "ok" : $"failed ({buildExit})"; }

Console.WriteLine($"xamlcheck: {lint.FileCount} XAML file(s), {lint.Classes.Count} x:Class, {lint.Classes.Sum(c => c.Fields.Count)} named element(s); " +
                  $"{errors} error(s), {warnings} warning(s); shadow build {buildState}; {clock.Elapsed.TotalSeconds:F1}s");
return errors > 0 || buildExit != 0 ? 1 : 0;
