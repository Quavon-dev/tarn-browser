$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'set-exe-icon.ps1')
$failed = 0

function Expect([string]$Name, [bool]$Want, [bool]$Got) {
    if ($Want -ne $Got) { Write-Host "FAIL ${Name}: expected $Want, got $Got"; $script:failed = 1 }
}

function Throws([scriptblock]$Block, [string]$Like) { try { & $Block; $false } catch { $_.Exception.Message -like $Like } }

function Same([byte[]]$A, [byte[]]$B) { [Convert]::ToBase64String($A) -eq [Convert]::ToBase64String($B) }

$tmp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
[void](New-Item -ItemType Directory $tmp)
$exe = Join-Path $tmp 'app.exe'
$sample = if ($env:CEF_PATH) { Join-Path $env:CEF_PATH 'bootstrap.exe' } else { (Get-Command powershell).Source }
Copy-Item -LiteralPath $sample -Destination $exe
$ico = (Resolve-Path (Join-Path $PSScriptRoot '..\packaging\windows\tarn.ico')).ProviderPath
$group = $null
$images = [TarnIcon]::Images([IO.File]::ReadAllBytes($ico), [ref]$group)
Expect 'tarn.ico holds 7 images' $true ($images.Count -eq 7)

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class IconSample {
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] public static extern IntPtr BeginUpdateResource(string file, bool deleteExisting);
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] public static extern bool UpdateResource(IntPtr update, IntPtr type, string name, ushort language, byte[] data, uint size);
    [DllImport("kernel32", SetLastError = true)] public static extern bool EndUpdateResource(IntPtr update, bool discard);
}
'@
Expect 'the sample has icons' $true ([TarnIcon]::Groups($exe).Count -gt 0)
$empty = [byte[]](0, 0, 1, 0, 0, 0)
$update = [IconSample]::BeginUpdateResource($exe, $false)
Expect 'the sample opens for update' $true ($update -ne [IntPtr]::Zero)
$added = [IconSample]::UpdateResource($update, [IntPtr]14, 'TARNTEST', 1033, $empty, $empty.Length)
Expect 'a named group icon is added to the sample' $true ([IconSample]::EndUpdateResource($update, -not $added) -and $added)
$before = [TarnIcon]::Groups($exe)
Expect 'a named group icon is listed by its name' $true (@($before | Where-Object { $_.Name -is [string] -and $_.Name -eq 'TARNTEST' -and $_.Language -eq 1033 }).Count -eq 1)
[TarnIcon]::Set($exe, $ico)
$after = [TarnIcon]::Groups($exe)
Expect 'group icon 1 exists' $true (@($after | Where-Object { $_.Name -is [int] -and $_.Name -eq 1 }).Count -ge 1)
Expect 'no group icon is lost' $true ($after.Count -ge $before.Count)
foreach ($g in $after) {
    Expect "group icon $($g.Name) ($($g.Language)) is Tarn's" $true (Same ([TarnIcon]::Read($exe, 14, $g)) $group)
}
$first = New-Object TarnIcon+Group
$first.Name = 30001
$first.Language = 0
Expect 'the first image is written' $true (Same ([TarnIcon]::Read($exe, 3, $first)) $images[0])

Expect 'a file that is no icon is refused' $true (Throws { [TarnIcon]::Images([byte[]](1, 2, 3, 4, 5, 6), [ref]$group) } '*not an .ico file*')
$short = [IO.File]::ReadAllBytes($ico)[0..199]
Expect 'a cut-off icon is refused' $true (Throws { [TarnIcon]::Images([byte[]]$short, [ref]$group) } '*outside the .ico file*')

Remove-Item -LiteralPath $tmp -Recurse -Force
if ($failed) { exit 1 }
Write-Host 'set-exe-icon.test.ps1: all passed'
