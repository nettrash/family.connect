# Measures every Microsoft Store listing field in listing.md against Partner Center's limits, so a count is never an
# estimate. Partner Center counts characters, and a trailing newline is not one of them.
#
#   powershell -NoProfile -File win/store/count.ps1
#
# Exits non-zero when a field is over its limit or missing, so it can guard a change to the copy.
#
# The helpers carry verb-noun names on purpose: a function called `Copy` or `Section` silently loses to PowerShell's own
# alias of that name (`Copy` is Copy-Item), and the first version of this script "measured" every text field as 0.
param([string]$Listing = (Join-Path $PSScriptRoot 'listing.md'))

$text = [System.IO.File]::ReadAllText($Listing, [System.Text.Encoding]::UTF8) -replace "`r`n", "`n"
$script:failed = $false

function Get-ListingSection([string]$Heading) {
    $pattern = '(?ms)^## ' + [regex]::Escape($Heading) + '\b[^\n]*\n(.*?)(?=^## |\z)'
    $m = [regex]::Match($text, $pattern)
    if (-not $m.Success) {
        Write-Output ('{0,-34} MISSING' -f $Heading)
        $script:failed = $true
        return ''
    }
    return $m.Groups[1].Value
}

# A section's listing text, without the editor's notes: italic lines (*…*) are not copy.
function Get-ListingCopy([string]$Body) {
    $lines = $Body -split "`n" | Where-Object { $_ -notmatch '^\s*\*.*\*\s*$' }
    return ($lines -join "`n").Trim()
}

function Get-NumberedItems([string]$Body) {
    return @($Body -split "`n" | Where-Object { $_ -match '^\d+\.\s' } | ForEach-Object { ($_ -replace '^\d+\.\s+', '').Trim() })
}

function Write-Limit([string]$Name, [int]$Length, [int]$Limit, [int]$Minimum = 0) {
    $state = 'ok'
    if ($Length -gt $Limit) { $state = 'OVER'; $script:failed = $true }
    elseif ($Length -lt $Minimum) { $state = 'TOO SHORT'; $script:failed = $true }
    Write-Output ('{0,-34} {1,6} / {2,-6} {3}' -f $Name, $Length, $Limit, $state)
}

$name = (Get-ListingCopy (Get-ListingSection 'Product name')).Split("`n")[0]
Write-Limit 'Product name' $name.Length 256 1

$short = Get-ListingCopy (Get-ListingSection 'Short description')
Write-Limit 'Short description' $short.Length 1000 1
Write-Limit 'Short description (shown whole)' $short.Length 270 1

$description = Get-ListingCopy (Get-ListingSection 'Description')
Write-Limit 'Description' $description.Length 10000 200

$features = Get-NumberedItems (Get-ListingSection 'Product features')
Write-Limit 'Product features (count)' $features.Count 20 1
Write-Limit 'Product feature (longest)' (($features | Measure-Object -Property Length -Maximum).Maximum) 200 1

$captions = @((Get-ListingSection 'Screenshots') -split "`n" | Where-Object { $_ -match '^\| \d+ \|' } | ForEach-Object { ($_ -split '\|')[3].Trim() })
Write-Limit 'Screenshot captions (count)' $captions.Count 10 1
Write-Limit 'Screenshot caption (longest)' (($captions | Measure-Object -Property Length -Maximum).Maximum) 200 1

$hardware = Get-NumberedItems (Get-ListingSection 'Additional system requirements')
Write-Limit 'Recommended hardware (count)' $hardware.Count 11
Write-Limit 'Recommended hardware (longest)' (($hardware | Measure-Object -Property Length -Maximum).Maximum) 200

# Release notes. Blank is legitimate for a first submission, so this one has no minimum.
$release = Get-ListingCopy (Get-ListingSection "What's new in this version")
Write-Limit "What's new in this version" $release.Length 1500

$copyright = Get-ListingCopy (Get-ListingSection 'Copyright and trademark info')
Write-Limit 'Copyright and trademark info' $copyright.Length 200 1

$notes = Get-ListingCopy (Get-ListingSection 'Notes for certification')
Write-Limit 'Notes for certification' $notes.Length 2000 1

if ($script:failed) { exit 1 }
exit 0
