using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;

namespace XamlCheck;

/// <summary>
/// The shadow project: a net10.0-windows class library outside the repo that compiles the app's
/// *.cs plus the stubs against the same Windows App SDK packages the real csproj references.
/// Also drives dotnet for restore, the resolved-reference query and the build.
/// </summary>
sealed class ShadowProject(Options options, string appCsproj, Log log)
{
    public string ProjectPath => Path.Combine(options.ShadowDir, "FamilyConnect.App.Shadow.csproj");
    public string StubsPath => Path.Combine(options.ShadowDir, "Xaml.Stubs.g.cs");

    static string Dotnet => Environment.GetEnvironmentVariable("DOTNET_HOST_PATH") is { Length: > 0 } host ? host : "dotnet";

    public void Write()
    {
        Directory.CreateDirectory(options.ShadowDir);
        var app = XDocument.Load(appCsproj);
        string Prop(string name, string fallback) => app.Descendants(name).FirstOrDefault()?.Value ?? fallback;
        var tfm = Prop("TargetFramework", "net10.0-windows10.0.26100.0");
        var minVersion = Prop("TargetPlatformMinVersion", "10.0.22000.0");
        var osVersion = Prop("SupportedOSPlatformVersion", minVersion);
        var rootNamespace = Prop("RootNamespace", "FamilyConnect.App");
        // Compile switches the code-behind depends on: LibraryImport (Interop/NativeMethods.cs) needs unsafe.
        var allowUnsafe = Prop("AllowUnsafeBlocks", "false");
        var packages = app.Descendants("PackageReference")
            .Select(p => (Id: p.Attribute("Include")?.Value, Version: p.Attribute("Version")?.Value ?? p.Element("Version")?.Value))
            .Where(p => p.Id is not null).ToList();
        var buildProps = Path.Combine(options.RepoRoot, "Directory.Build.props");
        // The app's ProjectReferences (FamilyConnect.Core, FamilyConnect.App.Logic), resolved from the app directory: the
        // code-behind implements Logic's seams, so the shadow must see the same assemblies csc would.
        var projectReferences = app.Descendants("ProjectReference")
            .Select(r => r.Attribute("Include")?.Value)
            .Where(v => !string.IsNullOrWhiteSpace(v))
            .Select(v => Path.GetFullPath(Path.Combine(options.AppDir, v!.Replace('\\', Path.DirectorySeparatorChar))))
            .Where(File.Exists)
            .ToList();
        if (projectReferences.Count == 0 && File.Exists(options.CoreCsproj)) projectReferences.Add(options.CoreCsproj);

        var sb = new StringBuilder();
        sb.AppendLine($$"""
            <Project Sdk="Microsoft.NET.Sdk">
              <!--
                FamilyConnect.App.Shadow — written by tools/xamlcheck on every run; do not edit. Compiles the app's
                *.cs against the real WinUI 3 projection with Xaml.Stubs.g.cs standing in for the XAML
                compiler's generated partials, so the code-behind is type-checked off Windows.
                TargetFramework, minimum version and package versions are copied from
                {{appCsproj}}.
              -->
              <Import Project="{{buildProps}}" Condition="Exists('{{buildProps}}')" />
              <PropertyGroup>
                <!-- A library: no Main, and nothing runs. -->
                <OutputType>Library</OutputType>
                <TargetFramework>{{tfm}}</TargetFramework>
                <TargetPlatformMinVersion>{{minVersion}}</TargetPlatformMinVersion>
                <SupportedOSPlatformVersion>{{osVersion}}</SupportedOSPlatformVersion>
                <RootNamespace>{{rootNamespace}}</RootNamespace>
                <AssemblyName>FamilyConnect.App.Shadow</AssemblyName>
                <Platforms>x64;ARM64</Platforms>
                <!-- Set here, not on the command line: global -p:Platform / -p:RuntimeIdentifier would flow
                     into the FamilyConnect.Core project reference and re-restore it for win-x64 (measured). -->
                <Platform Condition="'$(Platform)' == ''">{{options.Platform}}</Platform>
                <RuntimeIdentifier Condition="'$(RuntimeIdentifier)' == ''">{{options.Rid}}</RuntimeIdentifier>
                <UseWinUI>true</UseWinUI>
                <WinUISDKReferences>false</WinUISDKReferences>
                <EnableWindowsTargeting>true</EnableWindowsTargeting>
                <!-- MRT Core's PRI step runs MakePri.exe, a Windows binary, after the compile; a shadow library needs no PRI. -->
                <EnableCoreMrtTooling>false</EnableCoreMrtTooling>
                <EnableDefaultCompileItems>false</EnableDefaultCompileItems>
                <EnableDefaultPageItems>false</EnableDefaultPageItems>
                <EnableDefaultApplicationDefinition>false</EnableDefaultApplicationDefinition>
                <Nullable>enable</Nullable>
                <ImplicitUsings>enable</ImplicitUsings>
                <AllowUnsafeBlocks>{{allowUnsafe}}</AllowUnsafeBlocks>
                <DefineConstants>$(DefineConstants);DISABLE_XAML_GENERATED_MAIN</DefineConstants>
                <GenerateDocumentationFile>false</GenerateDocumentationFile>
              </PropertyGroup>

              <ItemGroup>
                <Compile Include="{{options.AppDir}}/**/*.cs" Exclude="{{options.AppDir}}/bin/**;{{options.AppDir}}/obj/**" />
                <Compile Include="Xaml.Stubs.g.cs" />
              </ItemGroup>

              <ItemGroup>
            """);
        foreach (var reference in projectReferences)
            sb.AppendLine($"""    <ProjectReference Include="{reference}" />""");
        sb.AppendLine("  </ItemGroup>");
        sb.AppendLine();
        sb.AppendLine("  <ItemGroup>");
        foreach (var (id, version) in packages)
            sb.AppendLine($"""    <PackageReference Include="{id}" Version="{version}" />""");
        sb.AppendLine("  </ItemGroup>");
        sb.AppendLine("</Project>");
        File.WriteAllText(ProjectPath, sb.ToString());
        if (!File.Exists(StubsPath)) File.WriteAllText(StubsPath, "// placeholder; xamlcheck rewrites this file after linting\n");
    }

    public bool Restore()
    {
        var (exit, output) = Run(["restore", ProjectPath, $"-p:Configuration={options.Configuration}", "-nologo", "-v", "minimal", "-tl:off"], echo: options.Verbose);
        if (exit != 0)
        {
            if (!options.Verbose) foreach (var line in output) Console.Error.WriteLine(line);
            log.Warn($"dotnet restore of the shadow project failed ({exit})");
        }
        return exit == 0;
    }

    /// <summary>The compile references csc will see, from MSBuild itself (-getItem:ReferencePath after ResolveAssemblyReferences).</summary>
    public List<string> ResolveReferencePaths()
    {
        var (exit, output) = Run(["msbuild", ProjectPath, "-getItem:ReferencePath", "-t:ResolveAssemblyReferences",
            "-p:BuildProjectReferences=false", $"-p:Configuration={options.Configuration}", "-nologo", "-tl:off"], echo: false);
        var text = string.Join('\n', output);
        var start = text.IndexOf('{');
        if (exit != 0 || start < 0)
        {
            log.Warn($"reference query failed ({exit}): {text}");
            return [];
        }
        try
        {
            using var doc = JsonDocument.Parse(text[start..]);
            return doc.RootElement.GetProperty("Items").GetProperty("ReferencePath").EnumerateArray()
                .Select(i => i.GetProperty("Identity").GetString()!).Where(File.Exists).ToList();
        }
        catch (Exception e) when (e is JsonException or KeyNotFoundException or InvalidOperationException)
        {
            log.Warn($"reference query returned unexpected output: {e.Message}");
            return [];
        }
    }

    /// <summary>Without a restore: the newest projection assemblies in the NuGet cache plus the running runtime's reference pack.</summary>
    public List<string> FallbackReferencePaths()
    {
        var refs = new List<string>();
        var runtimeDir = System.Runtime.InteropServices.RuntimeEnvironment.GetRuntimeDirectory();
        var refPack = Path.GetFullPath(Path.Combine(runtimeDir, "..", "..", "..", "packs", "Microsoft.NETCore.App.Ref", Environment.Version.ToString(), "ref", $"net{Environment.Version.Major}.0"));
        refs.AddRange(Directory.EnumerateFiles(Directory.Exists(refPack) ? refPack : runtimeDir, "*.dll"));

        var nuget = Environment.GetEnvironmentVariable("NUGET_PACKAGES") is { Length: > 0 } n ? n
            : Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".nuget", "packages");
        foreach (var (package, lib) in new[]
        {
            ("microsoft.windowsappsdk.winui", "lib"), ("microsoft.windows.sdk.net.ref", "lib"), ("microsoft.web.webview2", "lib_manual"),
            ("microsoft.windowsappsdk.interactiveexperiences", "lib"), ("microsoft.windowsappsdk.foundation", "lib"),
        })
        {
            var dir = Path.Combine(nuget, package);
            if (!Directory.Exists(dir)) { log.Warn($"fallback: {dir} is not in the NuGet cache"); continue; }
            var newest = Directory.EnumerateDirectories(dir).OrderByDescending(d => Version.TryParse(Path.GetFileName(d).Split('-')[0], out var v) ? v : new Version(0, 0)).First();
            var libDir = Path.Combine(newest, lib);
            if (!Directory.Exists(libDir)) continue;
            var tfmDir = Directory.EnumerateDirectories(libDir).Where(d => Path.GetFileName(d).StartsWith("net", StringComparison.Ordinal)).OrderByDescending(d => d, StringComparer.Ordinal).FirstOrDefault();
            if (tfmDir is not null) refs.AddRange(Directory.EnumerateFiles(tfmDir, "*.dll"));
        }
        return refs;
    }

    public int Build()
    {
        Console.WriteLine($"xamlcheck: dotnet build {ProjectPath} ({options.Configuration}, {options.Platform}, {options.Rid})");
        var (exit, _) = Run(["build", ProjectPath, "--no-restore", $"-p:Configuration={options.Configuration}", "-nologo", "-v", "minimal", "-tl:off"], echo: true);
        return exit;
    }

    (int Exit, List<string> Output) Run(string[] arguments, bool echo)
    {
        var psi = new ProcessStartInfo(Dotnet)
        {
            RedirectStandardOutput = true, RedirectStandardError = true, UseShellExecute = false, WorkingDirectory = options.ShadowDir,
        };
        foreach (var a in arguments) psi.ArgumentList.Add(a);
        psi.Environment["DOTNET_NOLOGO"] = "1";
        psi.Environment["DOTNET_CLI_TELEMETRY_OPTOUT"] = "1";
        psi.Environment["MSBUILDTERMINALLOGGER"] = "off";
        log.Info($"dotnet {string.Join(' ', arguments)}");

        var output = new List<string>();
        using var p = Process.Start(psi) ?? throw new InvalidOperationException("could not start dotnet");
        p.OutputDataReceived += (_, e) => { if (e.Data is null) return; lock (output) output.Add(e.Data); if (echo) Console.WriteLine(e.Data); };
        p.ErrorDataReceived += (_, e) => { if (e.Data is null) return; lock (output) output.Add(e.Data); if (echo) Console.Error.WriteLine(e.Data); };
        p.BeginOutputReadLine();
        p.BeginErrorReadLine();
        p.WaitForExit();
        return (p.ExitCode, output);
    }
}
