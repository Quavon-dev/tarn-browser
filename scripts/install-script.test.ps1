$ErrorActionPreference = 'Stop'
$failed = 0

function Expect([string]$Name, [bool]$Want, [bool]$Got) {
    if ($Want -ne $Got) { Write-Host "FAIL ${Name}: expected $Want, got $Got"; $script:failed = 1 }
}

function Throws([scriptblock]$Block, [string]$Like) { try { & $Block; $false } catch { $_.Exception.Message -like $Like } }

$tmp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
$package = Join-Path $tmp 'Tarn'
[void](New-Item -ItemType Directory $package)
Set-Content -LiteralPath (Join-Path $package 'tarn.exe') -Value 'new'
Set-Content -LiteralPath (Join-Path $package 'tarn-portable') -Value ''
[void](New-Item -ItemType Directory (Join-Path $package 'locales'))
Set-Content -LiteralPath (Join-Path $package 'locales\en-US.pak') -Value 'en'
$zip = Join-Path $tmp 'Tarn-v9.9.9-windows-x64-portable-unsigned.zip'
Compress-Archive -LiteralPath $package -DestinationPath $zip
Set-Content -LiteralPath (Join-Path $tmp 'Tarn-v9.9.9-windows-x64-setup-unsigned.exe') -Value 'setup'

# The published script with its downloads served from $tmp, which Windows PowerShell reads as file:// URLs.
function Render([string]$PortableSha) {
    $base = ([Uri]$tmp).AbsoluteUri.TrimEnd('/')
    $text = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '..\packaging\windows\install.ps1')
    $values = [ordered]@{
        "'https://github.com/@REPO@/releases/download/@TAG@'" = "'$base'"
        '@TAG@' = 'v9.9.9'
        '@REPO@' = 'Quavon-dev/tarn-browser'
        '@SETUP_NAME@' = 'Tarn-v9.9.9-windows-x64-setup-unsigned.exe'
        '@SETUP_SHA256@' = '0' * 64
        '@PORTABLE_NAME@' = 'Tarn-v9.9.9-windows-x64-portable-unsigned.zip'
        '@PORTABLE_SHA256@' = $PortableSha
    }
    foreach ($key in $values.Keys) { $text = $text.Replace($key, $values[$key]) }
    $path = Join-Path $tmp "install-$([guid]::NewGuid()).ps1"
    Set-Content -LiteralPath $path -Value $text
    $path
}

$good = Render (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash
$bad = Render ('0' * 64)
$dest = Join-Path $tmp 'Portable'

& $good -Portable -Path $dest
Expect 'portable unpacked' $true ((Get-Content -LiteralPath (Join-Path $dest 'tarn.exe')) -eq 'new')
Expect 'portable marker' $true (Test-Path -LiteralPath (Join-Path $dest 'tarn-portable') -PathType Leaf)

[void](New-Item -ItemType Directory (Join-Path $dest 'Data'))
Set-Content -LiteralPath (Join-Path $dest 'Data\profile.db') -Value 'kept'
Set-Content -LiteralPath (Join-Path $dest 'old.dll') -Value 'stale'
Set-Content -LiteralPath (Join-Path $dest 'notes.txt') -Value 'mine'
Set-Content -LiteralPath (Join-Path $dest 'tarn.exe') -Value 'old'
& $good -Portable -Path "$dest\"
Expect 'update keeps Data' $true ((Get-Content -LiteralPath (Join-Path $dest 'Data\profile.db')) -eq 'kept')
Expect 'update keeps files Tarn does not ship' $true ((Get-Content -LiteralPath (Join-Path $dest 'notes.txt')) -eq 'mine')
Expect 'update replaces Tarn' $true ((Get-Content -LiteralPath (Join-Path $dest 'tarn.exe')) -eq 'new')
Expect 'update drops old runtime files' $false (Test-Path -LiteralPath (Join-Path $dest 'old.dll'))
Expect 'update keeps the marker' $true (Test-Path -LiteralPath (Join-Path $dest 'tarn-portable') -PathType Leaf)
Expect 'no staging folder left' $true (@(Get-ChildItem -LiteralPath $tmp -Filter 'Portable.new-*').Count -eq 0)

$outside = Join-Path $tmp 'Outside'
[void](New-Item -ItemType Directory $outside)
Set-Content -LiteralPath (Join-Path $outside 'keep.txt') -Value 'kept'
foreach ($link in (Join-Path $dest 'stale.dll'), (Join-Path $dest 'locales\nested')) {
    [void](New-Item -ItemType Junction -Path $link -Target $outside)
    Expect "a link in the way is refused ($link)" $true (Throws { & $good -Portable -Path $dest } '*is a link*')
    Expect "a link target is untouched ($link)" $true ((Get-Content -LiteralPath (Join-Path $outside 'keep.txt')) -eq 'kept')
    Expect "a refused update leaves Tarn in place ($link)" $true ((Get-Content -LiteralPath (Join-Path $dest 'locales\en-US.pak')) -eq 'en')
    Expect "a refused update leaves no staging folder ($link)" $true (@(Get-ChildItem -LiteralPath $tmp -Filter 'Portable.new-*').Count -eq 0)
    [IO.Directory]::Delete($link)
}

Set-Content -LiteralPath (Join-Path $dest 'tarn.exe') -Value 'before'
Expect 'wrong SHA-256 refused' $true (Throws { & $bad -Portable -Path $dest } '*does not match*')
Expect 'wrong SHA-256 changes nothing' $true ((Get-Content -LiteralPath (Join-Path $dest 'tarn.exe')) -eq 'before')

$foreign = Join-Path $tmp 'Documents'
[void](New-Item -ItemType Directory $foreign)
Set-Content -LiteralPath (Join-Path $foreign 'letter.txt') -Value 'mine'
Expect 'foreign folder refused' $true (Throws { & $good -Portable -Path $foreign } '*neither empty nor a portable Tarn*')
Expect 'foreign folder untouched' $true ((Get-Content -LiteralPath (Join-Path $foreign 'letter.txt')) -eq 'mine')
Expect 'drive refused' $true (Throws { & $good -Portable -Path ([IO.Path]::GetPathRoot($tmp)) } '*is a drive*')

Remove-Item -LiteralPath $tmp -Recurse -Force
if ($failed) { exit 1 }
Write-Host 'install-script.test.ps1: all passed'
