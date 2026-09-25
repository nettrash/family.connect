# Seed the fixture the MICROSOFT STORE screenshots are shot against - the Windows port of
# server/scripts/seed-store-screenshots.sh, which seeds the same family for the App Store, the Mac App Store and Google Play.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File win/store/seed-store-screenshots.ps1 `
#       -PgBin <folder with psql.exe> [-Server server\target\debug\family-connect.exe]
#
# Needs a PostgreSQL on 127.0.0.1:5432 with a superuser postgres / postgres, and a built server. It drops and recreates
# family_connect_dev, starts the server on 127.0.0.1:8091 with a throwaway config and attachments folder, and seeds one
# invented family with one of everything worth photographing.
#
# EVERY NAME AND EVERY MESSAGE HERE IS INVENTED - the same fixture as the shell script, word for word. These end up on a
# public store listing, so nothing may resemble a real person's data.
#
# The photographed account is `nora` / `password123`, the family owner.
#
# Non-ASCII text is built from code points (Get-Text below) rather than typed: Windows PowerShell 5.1 reads a script saved
# without a byte-order mark in the ANSI code page, and "...", "-" and the emoji would arrive mangled on the listing.
param(
    [Parameter(Mandatory = $true)][string]$PgBin,
    [string]$Server = '',
    [string]$PhotoDir = ''
)
$ErrorActionPreference = 'Stop'
# Defaults resolved here, not in param(): Windows PowerShell 5.1 leaves $PSScriptRoot empty inside the param block of a
# script with a mandatory parameter.
if (-not $Server) { $Server = Join-Path $PSScriptRoot '..\..\server\target\debug\family-connect.exe' }
if (-not $PhotoDir) { $PhotoDir = Join-Path $PSScriptRoot '..\..\server\scripts\screenshot-photos' }
Add-Type -AssemblyName System.Drawing

$Base = 'http://127.0.0.1:8091/api/v1'
$psql = Join-Path $PgBin 'psql.exe'
$env:PGPASSWORD = 'postgres'

# "Nora " + (Get-Text 0x2026) - a string from Unicode code points, one argument each.
function Get-Text([int[]]$CodePoints) { -join ($CodePoints | ForEach-Object { [char]::ConvertFromUtf32($_) }) }
$Ellipsis = Get-Text 0x2026
$Dash = Get-Text 0x2014
$Arrow = Get-Text 0x2192

# --- the database and the server -----------------------------------------------------------------------------------
Get-Process family-connect -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 1
& $psql -h 127.0.0.1 -U postgres -q -c 'DROP DATABASE IF EXISTS family_connect_dev WITH (FORCE);' -c 'CREATE DATABASE family_connect_dev;'
if ($LASTEXITCODE -ne 0) { throw 'could not recreate family_connect_dev - is PostgreSQL running on 127.0.0.1:5432?' }

$work = Join-Path $env:TEMP 'fc-store-screenshots'
$attachments = Join-Path $work 'attachments'
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $attachments | Out-Null
$config = Join-Path $work 'server.toml'
# Forward slashes: TOML strings treat a backslash as an escape, and the server reads either separator on Windows.
@"
[server]
bind = "127.0.0.1:8091"

[database]
host = "127.0.0.1"
port = 5432
user = "postgres"
password = "postgres"
database = "family_connect_dev"

[storage]
attachments_dir = "$($attachments -replace '\\', '/')"
"@ | Set-Content -Path $config -Encoding ASCII

$serverPath = (Resolve-Path $Server).Path
Start-Process -FilePath $serverPath -ArgumentList @('--config', $config) -WindowStyle Hidden `
    -RedirectStandardOutput (Join-Path $work 'server.log') -RedirectStandardError (Join-Path $work 'server.err.log')
$up = $false
foreach ($i in 1..120) {
    try { Invoke-RestMethod "$Base/healthz" -TimeoutSec 2 | Out-Null; $up = $true; break } catch { Start-Sleep -Seconds 1 }
}
if (-not $up) { throw "server did not come up - see $work\server.err.log" }

# --- the API ---------------------------------------------------------------------------------------------------------
function Invoke-Api([string]$Method, [string]$Path, [string]$Token, $Body = $null, [byte[]]$Raw = $null, [string]$ContentType = 'application/json; charset=utf-8') {
    $headers = @{}
    if ($Token) { $headers['Authorization'] = "Bearer $Token" }
    $payload = $null
    if ($null -ne $Raw) { $payload = $Raw }
    elseif ($null -ne $Body) { $payload = [System.Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Depth 8 -Compress)) }
    $arguments = @{ Method = $Method; Uri = "$Base$Path"; Headers = $headers; ContentType = $ContentType }
    if ($null -ne $payload) { $arguments['Body'] = $payload }
    return Invoke-RestMethod @arguments
}

function Send-Message([long]$Chat, [string]$Token, [string]$Text, [hashtable]$Extra = @{}) {
    $payload = @{ client_msg_id = [guid]::NewGuid().ToString(); body = $Text }
    foreach ($key in $Extra.Keys) { $payload[$key] = $Extra[$key] }
    return Invoke-Api POST "/chats/$Chat/messages" $Token $payload
}

# --- images: real photos when supplied, generated gradients otherwise -----------------------------------------------
function New-GradientPng([int[]]$Top, [int[]]$Bottom) {
    $bitmap = New-Object System.Drawing.Bitmap 1200, 900
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush ([System.Drawing.Point]::new(0, 0)), ([System.Drawing.Point]::new(0, 900)), `
        ([System.Drawing.Color]::FromArgb(255, $Top[0], $Top[1], $Top[2])), ([System.Drawing.Color]::FromArgb(255, $Bottom[0], $Bottom[1], $Bottom[2]))
    $graphics.FillRectangle($brush, 0, 0, 1200, 900)
    $stream = New-Object System.IO.MemoryStream
    $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    return , $stream.ToArray()
}

function Get-AlbumPhotos {
    $found = @()
    if (Test-Path $PhotoDir) { $found = @(Get-ChildItem $PhotoDir -File | Where-Object { $_.Extension -in '.jpg', '.jpeg', '.png' } | Sort-Object Name | Select-Object -First 4) }
    # Write-Host, not Write-Output: anything a function writes to the pipeline becomes part of what it returns.
    if ($found.Count -gt 0) {
        Write-Host "  album: $($found.Count) supplied photo(s) from screenshot-photos\"
        return $found | ForEach-Object { , @([System.IO.File]::ReadAllBytes($_.FullName), $(if ($_.Extension -eq '.png') { 'image/png' } else { 'image/jpeg' })) }
    }
    Write-Host "  album: no photos supplied $Dash generating gradients"
    $palettes = @(@(@(250, 214, 165), @(214, 122, 92)), @(@(186, 220, 232), @(92, 140, 176)), @(@(205, 226, 191), @(104, 152, 106)), @(@(233, 205, 222), @(150, 108, 148)))
    return $palettes | ForEach-Object { , @((New-GradientPng $_[0] $_[1]), 'image/png') }
}

# --- the family ------------------------------------------------------------------------------------------------------
function Register-Member([string]$User, [string]$Name) {
    return (Invoke-Api POST '/auth/register' $null @{ username = $User; display_name = $Name; password = 'password123' }).token
}

$nora = Register-Member 'nora' 'Nora'
$dan = Register-Member 'dan' 'Dan'
$ellie = Register-Member 'ellie' 'Ellie'
$mae = Register-Member 'mae' 'Grandma Mae'
$rob = Register-Member 'rob' 'Uncle Rob'

$family = Invoke-Api POST '/families' $nora @{ name = 'The Harpers' }
$code = $family.family.invite_code
Invoke-Api PATCH '/families/mine' $nora @{ join_policy = 'open' } | Out-Null
foreach ($token in @($dan, $ellie, $mae, $rob)) { Invoke-Api POST '/families/join' $token @{ invite_code = $code } | Out-Null }
# Back to approval, which is the default a screenshot should show.
Invoke-Api PATCH '/families/mine' $nora @{ join_policy = 'approval' } | Out-Null

$familyChat = ((Invoke-Api GET '/chats' $nora).chats | Where-Object { $_.chat.kind -eq 'family' } | Select-Object -First 1).chat.id

# Birthdays - a day and a month, never a year.
$ids = @{}
foreach ($token in @($nora, $dan, $ellie, $mae, $rob)) { $ids[$token] = (Invoke-Api GET '/me' $token).user.id }
$birthdays = @(@($dan, 3, 14), @($ellie, 7, 2), @($mae, 11, 26), @($rob, 5, 9))
foreach ($b in $birthdays) { Invoke-Api PUT '/me/birthday' $b[0] @{ month = $b[1]; day = $b[2] } | Out-Null }

# --- the thread (oldest first: the client orders by id) --------------------------------------------------------------
$thread = @(
    @($nora, 'Half day tomorrow, so I can do the big shop on the way home.'),
    @($dan, "Perfect. We're out of coffee and Ellie finished the oat milk."),
    @($ellie, 'I did not finish the oat milk'),
    @($dan, 'Ellie.'),
    @($ellie, "$($Ellipsis)I finished the oat milk"),
    @($mae, 'Put me down for a bag of those little oranges if they have them.'),
    @($nora, 'Got it. Rob, are you still coming Sunday?'),
    @($rob, "Wouldn't miss it. I'll bring the good bread.")
)
foreach ($line in $thread) { Send-Message $familyChat $line[0] $line[1] | Out-Null }

# Album - four photos in ONE message.
$albumIds = @()
foreach ($photo in (Get-AlbumPhotos)) {
    $attachment = Invoke-Api POST '/attachments?kind=photo&width=1200&height=900' $dan -Raw $photo[0] -ContentType $photo[1]
    $albumIds += $attachment.attachment.id
}
Send-Message $familyChat $dan "Sunday at the lake $(Get-Text 0x1F986)" @{ attachment_ids = $albumIds } | Out-Null
Send-Message $familyChat $mae 'Oh these are lovely. Print me the second one?' | Out-Null

# Poll, mid-vote so two options carry voter faces.
$poll = Send-Message $familyChat $nora "Sunday lunch $Dash what are we doing?" @{ poll = @{ options = @('Roast at ours', 'Everyone brings a dish', "Caf$(Get-Text 0xE9) by the park") } }
$pollId = $poll.message.id
$options = $poll.message.poll.options
foreach ($vote in @(@($nora, 0), @($mae, 0), @($dan, 1), @($ellie, 1))) {
    Invoke-Api PUT "/chats/$familyChat/messages/$pollId/vote" $vote[0] @{ option_id = $options[$vote[1]].id } | Out-Null
}

# Shared location - a public place, never a home address.
$place = [uri]::EscapeDataString('Boating lake car park')
$location = Invoke-Api POST "/attachments?kind=location&latitude=51.5290&longitude=-0.1565&accuracy_m=12&name=$place" $rob -Raw ([byte[]]@())
Send-Message $familyChat $rob '' @{ attachment_id = $location.attachment.id } | Out-Null
Send-Message $familyChat $ellie "We're by the ducks when you get here $(Get-Text 0x1F425)" | Out-Null

# Reactions, so a bubble shows chips.
$messages = (Invoke-Api GET "/chats/$familyChat/messages" $nora).messages
$album = $messages | Where-Object { $_.body -like 'Sunday at the lake*' } | Select-Object -First 1
$heart = Get-Text 0x2764, 0xFE0F
foreach ($reaction in @(@($nora, $heart), @($mae, $heart), @($ellie, (Get-Text 0x1F60D)))) {
    Invoke-Api PUT "/chats/$familyChat/messages/$($album.id)/reaction" $reaction[0] @{ emoji = $reaction[1] } | Out-Null
}

# --- board -----------------------------------------------------------------------------------------------------------
$notes = @(
    @('Bins go out Tuesday', 'yellow', 0.12, 0.10),
    @("Ellie $Dash dentist, Thu 4pm", 'blue', 0.52, 0.16),
    @("Rob's bread recipe is in the tin", 'green', 0.18, 0.46),
    @("Holiday photos $Arrow shared album", 'pink', 0.58, 0.54)
)
foreach ($note in $notes) { Invoke-Api POST '/families/mine/board/notes' $nora @{ text = $note[0]; color = $note[1]; x = $note[2]; y = $note[3] } | Out-Null }

# --- a direct chat ---------------------------------------------------------------------------------------------------
$direct = (Invoke-Api POST '/chats/direct' $nora @{ user_id = $ids[$ellie] }).chat.id
Send-Message $direct $ellie "Can I stay at Priya's on Saturday?" | Out-Null
Send-Message $direct $nora "Yes $Dash home by lunch on Sunday please." | Out-Null

# Spread the thread over the morning: every message was posted in the same second, and a listing whose every row reads
# the same minute looks staged. created_at is server-assigned, so this is the one step that reaches past the API.
& $psql -h 127.0.0.1 -U postgres -d family_connect_dev -q -c "WITH ordered AS (SELECT id, row_number() OVER (ORDER BY id DESC) AS back FROM messages) UPDATE messages m SET created_at = now() - (ordered.back * interval '37 minutes') FROM ordered WHERE m.id = ordered.id;"

Write-Output ''
Write-Output "  family 'The Harpers'  chat $familyChat  invite $code"
Write-Output '  photograph as: nora / password123   (owner)'
Write-Output '  server: http://127.0.0.1:8091'
