# Capture the Microsoft Store screenshots from the running Windows app, signed in to the invented demo family that
# seed-store-screenshots.ps1 builds on a local server.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File win/store/capture-screenshots.ps1            # capture all six
#   powershell -NoProfile -ExecutionPolicy Bypass -File win/store/capture-screenshots.ps1 -Restore   # put the reader's server back
#
# THE APP IS THE INSTALLED DEV PACKAGE, and pointing it at the demo server replaces its saved server. The setting it had is
# kept beside it (server.txt.before-screenshots) and -Restore writes it back. The session token is stored per server, so
# nobody is signed out; the local history cache for the real server is downloaded again on its next launch.
#
# 3200 x 1800 physical pixels: 16:9, well above the Store's 1366 x 768 minimum, and the window's effective 1600 x 900 at
# 200% is wide enough for the family's two-column layout. A capture that comes back mostly black - a sleeping display, a
# locked session - is refused rather than saved, because Windows then hands back black pixels for every window.
param(
    [string]$Server = 'http://127.0.0.1:8091/',
    [string]$User = 'nora',
    [string]$Password = 'password123',
    [string]$OutDir = '',
    [switch]$Restore
)
$ErrorActionPreference = 'Stop'
# Resolved here, not in param(): Windows PowerShell 5.1 can leave $PSScriptRoot empty inside a param block.
if (-not $OutDir) { $OutDir = Join-Path $PSScriptRoot 'images\screenshots' }

Add-Type @'
using System; using System.Runtime.InteropServices;
public static class StoreShots {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
  public struct Rect { public int Left, Top, Right, Bottom; }
}
'@
[StoreShots]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing, System.Windows.Forms

# The installed package, found rather than written down: its family name follows the identity in Package.appxmanifest,
# and a build under an older identity may still be installed beside it.
$package = @(Get-AppxPackage -Name 'nttrsh.FamilyConnect') + @(Get-AppxPackage -Name '*.FamilyConnect') | Where-Object { $_ } | Select-Object -First 1
if (-not $package) { throw 'Family Connect is not installed - run it once with dotnet run first' }
$App = "shell:AppsFolder\$($package.PackageFamilyName)!App"
$Data = Join-Path $env:LOCALAPPDATA "Packages\$($package.PackageFamilyName)\LocalCache\Local\FamilyConnect"
$Setting = Join-Path $Data 'server.txt'
$Kept = "$Setting.before-screenshots"

function Stop-App { Get-Process FamilyConnect -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Seconds 2 }

function Start-App {
    Start-Process $App
    for ($i = 0; $i -lt 60; $i++) {
        $process = Get-Process FamilyConnect -ErrorAction SilentlyContinue | Where-Object MainWindowHandle -ne 0 | Select-Object -First 1
        if ($process) { Start-Sleep -Seconds 3; return $process }
        Start-Sleep -Milliseconds 500
    }
    throw 'the app window never appeared'
}

if ($Restore) {
    Stop-App
    if (Test-Path $Kept) { Move-Item $Kept $Setting -Force; Write-Host "restored the saved server: $(Get-Content $Setting)" }
    else { Write-Host 'nothing to restore' }
    Start-App | Out-Null
    return
}

# --- point the app at the demo server -------------------------------------------------------------------------------
Stop-App
if ((Test-Path $Setting) -and -not (Test-Path $Kept)) { Copy-Item $Setting $Kept }
Set-Content -Path $Setting -Value $Server -NoNewline -Encoding ASCII
$process = Start-App
$window = $process.MainWindowHandle
$root = [System.Windows.Automation.AutomationElement]::FromHandle($window)

function Find-ById([string]$Id, [int]$Seconds = 20) {
    # ::new, never New-Object: New-Object hands the value over wrapped in a PSObject, which UI Automation refuses.
    $condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::AutomationIdProperty, $Id)
    for ($i = 0; $i -lt $Seconds * 2; $i++) {
        $found = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
        if ($found) { return $found }
        Start-Sleep -Milliseconds 500
    }
    return $null
}

function Find-ByName([string]$Name, [int]$Seconds = 20) {
    $condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, $Name)
    for ($i = 0; $i -lt $Seconds * 2; $i++) {
        $found = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
        if ($found) { return $found }
        Start-Sleep -Milliseconds 500
    }
    return $null
}

function Enter-Text($Element, [string]$Text) {
    [StoreShots]::SetForegroundWindow($window) | Out-Null
    $Element.SetFocus()
    Start-Sleep -Milliseconds 300
    [System.Windows.Forms.SendKeys]::SendWait($Text)
    Start-Sleep -Milliseconds 300
}

# --- sign in ---------------------------------------------------------------------------------------------------------
$username = Find-ById 'UsernameBox' 30
if ($username) {
    Enter-Text $username $User
    Enter-Text (Find-ById 'PasswordField') $Password
    (Find-ById 'SubmitButton').GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    Write-Host "signed in as $User"
}
if (-not (Find-ByName 'Chats' 40)) { throw 'the chats never appeared - is the demo server seeded and running?' }
Start-Sleep -Seconds 4

# --- the window: 3200 x 1800, top-left of the work area --------------------------------------------------------------
[StoreShots]::ShowWindow($window, 9) | Out-Null
# TOPMOST while the six are taken: a screen capture is whatever is on top, and the first run photographed a terminal
# that came forward over the board. Released again at the end.
[StoreShots]::SetWindowPos($window, [IntPtr](-1), 0, 0, 3200, 1800, 0x0040) | Out-Null
Start-Sleep -Seconds 2

New-Item -ItemType Directory -Force $OutDir | Out-Null

function Save-Shot([string]$Name) {
    [StoreShots]::SetForegroundWindow($window) | Out-Null
    Start-Sleep -Seconds 2
    $rect = New-Object StoreShots+Rect
    [StoreShots]::GetWindowRect($window, [ref]$rect) | Out-Null
    $width = $rect.Right - $rect.Left; $height = $rect.Bottom - $rect.Top
    $bitmap = New-Object System.Drawing.Bitmap $width, $height, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $dark = 0; $samples = 0
    for ($x = 40; $x -lt $width; $x += 97) { for ($y = 40; $y -lt $height; $y += 89) { $c = $bitmap.GetPixel($x, $y); $samples++; if ($c.R -lt 8 -and $c.G -lt 8 -and $c.B -lt 8) { $dark++ } } }
    if ($dark -gt $samples * 0.6) {
        $graphics.Dispose(); $bitmap.Dispose()
        throw "$Name came back black ($dark of $samples samples) - is the display on and the session unlocked?"
    }
    $path = Join-Path $OutDir "$Name.png"
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    Write-Host "  $path  (${width}x$height)"
}

function Select-Rail([string]$Name) {
    $item = Find-ByName $Name
    $item.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Seconds 3
}

function Set-Scroll([string]$Id, [double]$Percent) {
    $scroller = Find-ById $Id 10
    if (-not $scroller) { Write-Host "  (no $Id to scroll)"; return }
    $scroller.GetCurrentPattern([System.Windows.Automation.ScrollPattern]::Pattern).SetScrollPercent(-1, $Percent)
    Start-Sleep -Seconds 2
}

# --- the six screens -------------------------------------------------------------------------------------------------
Select-Rail 'Chats'
$list = Find-ById 'ChatList'
$first = $list.FindFirst([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::ListItem))
$first.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
Start-Sleep -Seconds 5

Set-Scroll 'MessageScroller' 0
Save-Shot '01-family-chat'
Set-Scroll 'MessageScroller' 66
Save-Shot '02-photos-and-poll'
Set-Scroll 'MessageScroller' 100
# The map's tiles arrive after the bubble does.
Start-Sleep -Seconds 5
Save-Shot '03-location'

Select-Rail 'Board'
Save-Shot '04-board'

Select-Rail 'Family'
Start-Sleep -Seconds 2
Save-Shot '05-family'

Select-Rail 'Settings'
$scrollers = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::IsScrollPatternAvailableProperty, $true))
foreach ($scroller in $scrollers) {
    $pattern = $scroller.GetCurrentPattern([System.Windows.Automation.ScrollPattern]::Pattern)
    if ($pattern.Current.VerticallyScrollable) { $pattern.SetScrollPercent(-1, 62); break }
}
Save-Shot '06-settings'

# No longer on top of everything.
[StoreShots]::SetWindowPos($window, [IntPtr](-2), 0, 0, 0, 0, 0x0003) | Out-Null

Write-Host ''
Write-Host "done. Put the saved server back with: capture-screenshots.ps1 -Restore"
