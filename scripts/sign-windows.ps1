<#
.SYNOPSIS
Signs a Windows release of Tarn on the maintainer's machine and checks its update feed (docs/release.md).

.DESCRIPTION
Sign the binaries of the unsigned release zip, build the installer from them and sign it
(certificate in the user's store by SHA-1 thumbprint, or Azure Trusted Signing):
  sign-windows.ps1 -Package Tarn-v1.2.3-windows-x64-unsigned.zip -Version 1.2.3 -Thumbprint <SHA-1>
  sign-windows.ps1 -Package <zip> -Version 1.2.3 -Dlib <Azure.CodeSigning.Dlib.dll> -Metadata <metadata.json>
It then prints the commands that sign the installer and the feed with the update key.

Write windows-update.json for a signed installer whose Ed25519 signature was made elsewhere:
  sign-windows.ps1 -Installer Tarn-v1.2.3-windows-x64-setup.exe -InstallerSignature <base64> [-Critical]

Check windows-update.json and windows-update.json.sig (and the installer, if given) against the
public key in packaging/updates.conf:
  sign-windows.ps1 -Check windows-update.json [-Installer Tarn-v1.2.3-windows-x64-setup.exe]
#>
[CmdletBinding(DefaultParameterSetName = 'None')]
param(
    [Parameter(ParameterSetName = 'Thumbprint', Mandatory)]
    [Parameter(ParameterSetName = 'Trusted', Mandatory)]
    [string]$Package,
    [Parameter(ParameterSetName = 'Thumbprint', Mandatory)]
    [Parameter(ParameterSetName = 'Trusted', Mandatory)]
    [string]$Version,
    [Parameter(ParameterSetName = 'Thumbprint', Mandatory)]
    [string]$Thumbprint,
    [Parameter(ParameterSetName = 'Trusted', Mandatory)]
    [string]$Dlib,
    [Parameter(ParameterSetName = 'Trusted', Mandatory)]
    [string]$Metadata,
    [Parameter(ParameterSetName = 'Thumbprint')]
    [Parameter(ParameterSetName = 'Trusted')]
    [string]$TimestampUrl,
    [Parameter(ParameterSetName = 'Thumbprint')]
    [Parameter(ParameterSetName = 'Trusted')]
    [string]$SignTool,
    [Parameter(ParameterSetName = 'Thumbprint')]
    [Parameter(ParameterSetName = 'Trusted')]
    [string]$Iscc,
    [Parameter(ParameterSetName = 'Thumbprint')]
    [Parameter(ParameterSetName = 'Trusted')]
    [Parameter(ParameterSetName = 'Feed')]
    [string]$OutDir = 'dist',
    [Parameter(ParameterSetName = 'Thumbprint')]
    [Parameter(ParameterSetName = 'Trusted')]
    [Parameter(ParameterSetName = 'Feed')]
    [switch]$Critical,
    [Parameter(ParameterSetName = 'Feed', Mandatory)]
    [Parameter(ParameterSetName = 'Check')]
    [string]$Installer,
    [Parameter(ParameterSetName = 'Feed', Mandatory)]
    [string]$InstallerSignature,
    [Parameter(ParameterSetName = 'Check', Mandatory)]
    [string]$Check
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Numerics

$Repo = 'Quavon-dev/tarn-browser'
$UpdatesConf = Join-Path $PSScriptRoot '..\packaging\updates.conf'
$MaxFeed = 64KB

# Microsoft's and Google's own code signing subjects; a quoted or comma-holding CN cannot pass.
function Test-RedistributorSubject([string]$Subject) {
    $Subject -match '^CN=[^",]+, O=(Microsoft Corporation|Google LLC), L=(Redmond|Mountain View), '
}
$MaxInstaller = 512MB

function Get-FeedJson([string]$Version, [bool]$Critical, [string]$Sha256, [long]$Length, [string]$Signature, [switch]$Unsigned) {
    $suffix = if ($Unsigned) { '-unsigned' } else { '' }
    $url = "https://github.com/$Repo/releases/download/v$Version/Tarn-v$Version-windows-x64-setup$suffix.exe"
    $flag = if ($Critical) { 'true' } else { 'false' }
    '{"version":"' + $Version + '","critical":' + $flag + ',"installer":{"url":"' + $url + '","sha256":"' + $Sha256 + '","length":' + $Length + ',"signature":"' + $Signature + '"}}'
}

function Get-PublicKey {
    $line = Get-Content -LiteralPath $UpdatesConf | Where-Object { $_ -like 'public_ed_key=*' } | Select-Object -First 1
    if (-not $line) { throw "$UpdatesConf has no public_ed_key" }
    $key = [Convert]::FromBase64String($line.Substring('public_ed_key='.Length).Trim())
    if ($key.Length -ne 32) { throw "public_ed_key in $UpdatesConf is not 32 bytes" }
    , $key
}

function ConvertFrom-Signature([string]$Text) {
    if ($Text -notmatch '^[A-Za-z0-9+/]{86}==\z') { throw "'$Text' is not a base64 Ed25519 signature" }
    , [Convert]::FromBase64String($Text)
}

#region Ed25519 verification (RFC 8032 section 5.1.7); PowerShell 5.1 has no Ed25519.
function ConvertFrom-LittleEndian([byte[]]$Bytes) {
    New-Object System.Numerics.BigInteger -ArgumentList (, [byte[]]($Bytes + [byte]0))
}

function Get-Mod($X, $M) {
    $r = [System.Numerics.BigInteger]::Remainder($X, $M)
    if ($r.Sign -lt 0) { $r = $r + $M }
    $r
}

$EdP = [System.Numerics.BigInteger]::Pow(2, 255) - 19
$EdL = [System.Numerics.BigInteger]::Parse('7237005577332262213973186563042994240857116359379907606001950938285454250989')
$EdD = Get-Mod ([System.Numerics.BigInteger]::new(-121665) * [System.Numerics.BigInteger]::ModPow(121666, $EdP - 2, $EdP)) $EdP
$EdI = [System.Numerics.BigInteger]::ModPow(2, [System.Numerics.BigInteger]::Divide($EdP - 1, 4), $EdP)

function Add-EdPoint($A, $B) {
    $p = $EdP
    $a1 = Get-Mod (($A[1] - $A[0]) * ($B[1] - $B[0])) $p
    $b1 = Get-Mod (($A[1] + $A[0]) * ($B[1] + $B[0])) $p
    $c1 = Get-Mod ($A[3] * $B[3] * 2 * $EdD) $p
    $d1 = Get-Mod ($A[2] * $B[2] * 2) $p
    $e = $b1 - $a1; $f = $d1 - $c1; $g = $d1 + $c1; $h = $b1 + $a1
    , @((Get-Mod ($e * $f) $p), (Get-Mod ($g * $h) $p), (Get-Mod ($f * $g) $p), (Get-Mod ($e * $h) $p))
}

function Get-EdMultiple($Point, $Scalar) {
    $q = @([System.Numerics.BigInteger]::Zero, [System.Numerics.BigInteger]::One, [System.Numerics.BigInteger]::One, [System.Numerics.BigInteger]::Zero)
    foreach ($byte in $Scalar.ToByteArray()) {
        for ($bit = 0; $bit -lt 8; $bit++) {
            if (($byte -shr $bit) -band 1) { $q = Add-EdPoint $q $Point }
            $Point = Add-EdPoint $Point $Point
        }
    }
    , $q
}

function ConvertFrom-EdPoint([byte[]]$Bytes) {
    $p = $EdP
    $b = [byte[]]$Bytes.Clone()
    $sign = $b[31] -shr 7
    $b[31] = $b[31] -band 0x7f
    $y = ConvertFrom-LittleEndian $b
    if ($y -ge $p) { return $null }
    $x2 = Get-Mod (($y * $y - 1) * [System.Numerics.BigInteger]::ModPow($EdD * $y * $y + 1, $p - 2, $p)) $p
    $x = [System.Numerics.BigInteger]::ModPow($x2, [System.Numerics.BigInteger]::Divide($p + 3, 8), $p)
    if (-not (Get-Mod ($x * $x - $x2) $p).IsZero) { $x = Get-Mod ($x * $EdI) $p }
    if (-not (Get-Mod ($x * $x - $x2) $p).IsZero) { return $null }
    if ($x.IsZero -and $sign -eq 1) { return $null }
    if ([int](-not $x.IsEven) -ne $sign) { $x = $p - $x }
    , @($x, $y, [System.Numerics.BigInteger]::One, (Get-Mod ($x * $y) $p))
}

$EdBase = ConvertFrom-EdPoint ([byte[]](@(0x58) + @(0x66) * 31))

function Test-Ed25519([byte[]]$PublicKey, [System.IO.Stream]$Message, [byte[]]$Signature) {
    if ($PublicKey.Length -ne 32 -or $Signature.Length -ne 64) { return $false }
    $a = ConvertFrom-EdPoint $PublicKey
    $r = ConvertFrom-EdPoint $Signature[0..31]
    $s = ConvertFrom-LittleEndian $Signature[32..63]
    if ($null -eq $a -or $null -eq $r -or $s -ge $EdL) { return $false }
    $sha = [System.Security.Cryptography.SHA512]::Create()
    [void]$sha.TransformBlock($Signature, 0, 32, $null, 0)
    [void]$sha.TransformBlock($PublicKey, 0, 32, $null, 0)
    $buffer = New-Object byte[] 1MB
    while (($n = $Message.Read($buffer, 0, $buffer.Length)) -gt 0) { [void]$sha.TransformBlock($buffer, 0, $n, $null, 0) }
    [void]$sha.TransformFinalBlock($buffer, 0, 0)
    $k = Get-Mod (ConvertFrom-LittleEndian $sha.Hash) $EdL
    $left = Get-EdMultiple $EdBase $s
    $right = Add-EdPoint $r (Get-EdMultiple $a $k)
    $p = $EdP
    (Get-Mod ($left[0] * $right[2] - $right[0] * $left[2]) $p).IsZero -and (Get-Mod ($left[1] * $right[2] - $right[1] * $left[2]) $p).IsZero
}
#endregion

function Test-FileSignature([byte[]]$PublicKey, [string]$Path, [byte[]]$Signature) {
    $stream = [System.IO.File]::OpenRead($Path)
    try { Test-Ed25519 $PublicKey $stream $Signature } finally { $stream.Dispose() }
}

function Get-Sha256([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }

function Test-Installer([string]$Path, $Feed) {
    $key = Get-PublicKey
    $file = Get-Item -LiteralPath $Path
    if ($file.Name -cne ($Feed.installer.url -split '/')[-1]) { throw "$Path is not the installer the feed names" }
    if ($file.Length -ne $Feed.installer.length) { throw "$Path is $($file.Length) bytes, the feed says $($Feed.installer.length)" }
    if ((Get-Sha256 $Path) -ne $Feed.installer.sha256) { throw "$Path does not have the feed's SHA-256" }
    if (-not (Test-FileSignature $key $Path (ConvertFrom-Signature $Feed.installer.signature))) { throw "The installer signature does not verify with the key in $UpdatesConf" }
}

function Invoke-Check([string]$FeedPath, [string]$InstallerPath) {
    $bytes = [System.IO.File]::ReadAllBytes($FeedPath)
    if ($bytes.Length -gt $MaxFeed) { throw "$FeedPath is larger than 64 KiB" }
    $text = (New-Object System.Text.UTF8Encoding($false, $true)).GetString($bytes)
    $sigText = [System.IO.File]::ReadAllText("$FeedPath.sig")
    if (-not (Test-Ed25519 (Get-PublicKey) (New-Object System.IO.MemoryStream(, $bytes)) (ConvertFrom-Signature $sigText))) {
        throw "$FeedPath.sig does not verify $FeedPath with the key in $UpdatesConf"
    }
    $feed = $text | ConvertFrom-Json
    if ($feed.version -isnot [string] -or $feed.version -notmatch '^\d+(\.\d+)*$') { throw "Feed version '$($feed.version)' is not dot-separated numbers" }
    if ($feed.installer.sha256 -isnot [string] -or $feed.installer.sha256 -notmatch '^[0-9a-f]{64}$') { throw 'Feed sha256 is not 64 lowercase hex digits' }
    if ($feed.installer.length -le 0 -or $feed.installer.length -gt $MaxInstaller) { throw 'Feed length is not between 1 byte and 512 MiB' }
    [void](ConvertFrom-Signature $feed.installer.signature)
    $unsigned = $feed.installer.url -is [string] -and $feed.installer.url.EndsWith('-setup-unsigned.exe')
    $canonical = Get-FeedJson $feed.version ($feed.critical -eq $true) $feed.installer.sha256 $feed.installer.length $feed.installer.signature -Unsigned:$unsigned
    if ($text -cne $canonical) { throw "$FeedPath is not in the exact feed format:`n  $text`nexpected`n  $canonical" }
    if ($InstallerPath) { Test-Installer $InstallerPath $feed }
    $what = if ($InstallerPath) { ', the installer matches it' } else { '' }
    Write-Host "OK: $FeedPath is signed with the update key, version $($feed.version), critical $($feed.critical)$what."
}

function Write-Feed([string]$InstallerPath, [string]$Signature, [bool]$IsCritical, [string]$Dir) {
    $name = Split-Path -Leaf $InstallerPath
    if ($name -cnotmatch '^Tarn-v(\d+(\.\d+)*)-windows-x64-setup(-unsigned)?\.exe$') { throw "$name is not named Tarn-v<version>-windows-x64-setup[-unsigned].exe" }
    $json = Get-FeedJson $Matches[1] $IsCritical (Get-Sha256 $InstallerPath) (Get-Item -LiteralPath $InstallerPath).Length $Signature -Unsigned:([bool]$Matches[3])
    Test-Installer $InstallerPath ($json | ConvertFrom-Json)
    [void](New-Item -ItemType Directory -Force $Dir)
    $feedPath = Join-Path $Dir 'windows-update.json'
    [System.IO.File]::WriteAllBytes($feedPath, [System.Text.Encoding]::UTF8.GetBytes($json))
    Write-Host "Wrote $feedPath. Sign it on the Mac that holds the update key, then run -Check:"
    Write-Host "  printf '%s' `"`$(sign_update --account dev.tarn.browser windows-update.json | sed -n 's/.*edSignature=`"\([^`"]*\)`".*/\1/p')`" > windows-update.json.sig"
}

function Find-Tool([string]$Given, [string[]]$Candidates, [string]$Name) {
    $found = @($Given) + $Candidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
    if (-not $found) { throw "$Name not found; pass its path" }
    $found
}

function Invoke-Sign {
    if ($Version -notmatch '^\d+(\.\d+){0,3}$') { throw "Version '$Version' is not dot-separated numbers" }
    $out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutDir)
    $kits = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | ForEach-Object FullName
    $signer = Find-Tool $SignTool @($kits) 'signtool.exe (Windows SDK)'
    $compiler = Find-Tool $Iscc @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") 'ISCC.exe (Inno Setup 6)'

    $work = Join-Path $out 'Tarn'
    if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
    [void](New-Item -ItemType Directory -Force $out)
    if (Test-Path -LiteralPath $Package -PathType Container) {
        Copy-Item -LiteralPath $Package -Destination $work -Recurse
    } else {
        $unpacked = Join-Path $out 'unpacked'
        if (Test-Path -LiteralPath $unpacked) { Remove-Item -LiteralPath $unpacked -Recurse -Force }
        Expand-Archive -LiteralPath $Package -DestinationPath $unpacked
        Move-Item -LiteralPath (Join-Path $unpacked 'Tarn') -Destination $work
        Remove-Item -LiteralPath $unpacked -Recurse -Force
    }
    if (-not (Test-Path -LiteralPath (Join-Path $work 'tarn.exe'))) { throw "$Package holds no Tarn\tarn.exe" }

    $common = @('/fd', 'SHA256', '/td', 'SHA256', '/d', 'Tarn', '/du', "https://github.com/$Repo")
    if ($PSCmdlet.ParameterSetName -eq 'Thumbprint') {
        $how = @('/sha1', $Thumbprint, '/tr', $(if ($TimestampUrl) { $TimestampUrl } else { 'http://timestamp.digicert.com' }))
    } else {
        $how = @('/dlib', $Dlib, '/dmdf', $Metadata, '/tr', $(if ($TimestampUrl) { $TimestampUrl } else { 'http://timestamp.acs.microsoft.com' }))
    }
    $signArgs = @('sign') + $common + $how

    $binaries = Get-ChildItem -LiteralPath $work -Recurse -File | Where-Object { $_.Extension -in '.exe', '.dll' }
    # Only Microsoft's and Google's own signatures are kept (redistributed runtime files); anything
    # else, unsigned or signed by another publisher, gets Tarn's signature.
    $trusted = { param($sig) $sig.Status -eq 'Valid' -and (Test-RedistributorSubject $sig.SignerCertificate.Subject) }
    # tarn.exe (CEF's launcher) and tarn.dll must carry the same certificate, or the launcher refuses the DLL.
    $unsigned = @($binaries | Where-Object { $_.Name -in 'tarn.exe', 'tarn.dll' -or -not (& $trusted (Get-AuthenticodeSignature -LiteralPath $_.FullName)) } | ForEach-Object FullName)
    Write-Host "Signing $($unsigned.Count) of $(@($binaries).Count) binaries (the rest carry Microsoft's or Google's signature)."
    if ($unsigned.Count -gt 0) {
        & $signer @signArgs @unsigned
        if ($LASTEXITCODE -ne 0) { throw "signtool sign failed ($LASTEXITCODE)" }
    }

    $quote = { param($a) if ($a -match '\s') { '$q' + $a + '$q' } else { $a } }
    $command = (@('$q' + $signer + '$q') + ($signArgs | ForEach-Object { & $quote $_ }) + '$f') -join ' '
    & $compiler /Q "/DAppVersion=$Version" "/DPackageDir=$work" "/DOutputDir=$out" /DSign "/Starn=$command" (Join-Path $PSScriptRoot '..\packaging\windows\tarn.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
    $setup = Join-Path $out "Tarn-v$Version-windows-x64-setup.exe"

    $ours = (Get-AuthenticodeSignature -LiteralPath (Join-Path $work 'tarn.exe')).SignerCertificate.Thumbprint
    foreach ($file in @($binaries | ForEach-Object FullName) + $setup) {
        & $signer verify /pa /q $file
        if ($LASTEXITCODE -ne 0) { throw "signtool verify /pa rejects $file" }
        $sig = Get-AuthenticodeSignature -LiteralPath $file
        if ($null -eq $sig.TimeStamperCertificate) { throw "$file has no timestamp" }
        if ($sig.SignerCertificate.Thumbprint -ne $ours -and -not (& $trusted $sig)) { throw "$file is signed by $($sig.SignerCertificate.Subject), not by Tarn's certificate" }
    }
    Remove-Item -LiteralPath $work -Recurse -Force

    $placeholder = '@SIGNATURE@'
    $parts = (Get-FeedJson $Version $Critical.IsPresent (Get-Sha256 $setup) (Get-Item -LiteralPath $setup).Length $placeholder) -split $placeholder
    $setupName = Split-Path -Leaf $setup
    Write-Host "Signed and verified $setup."
    Write-Host ''
    Write-Host "Copy $setupName to the Mac that holds the update key and run there (bash):"
    Write-Host "  sig=`"`$(sign_update --account dev.tarn.browser $setupName | sed -n 's/.*edSignature=`"\([^`"]*\)`".*/\1/p')`""
    Write-Host "  printf '%s%s%s' '$($parts[0])' `"`$sig`" '$($parts[1])' > windows-update.json"
    Write-Host "  printf '%s' `"`$(sign_update --account dev.tarn.browser windows-update.json | sed -n 's/.*edSignature=`"\([^`"]*\)`".*/\1/p')`" > windows-update.json.sig"
    Write-Host ''
    Write-Host 'Bring windows-update.json and windows-update.json.sig back next to the installer and check them:'
    Write-Host "  scripts\sign-windows.ps1 -Check windows-update.json -Installer $setupName"
}

if ($MyInvocation.InvocationName -eq '.') { return }
switch ($PSCmdlet.ParameterSetName) {
    'Check' { Invoke-Check $Check $Installer }
    'Feed' { Write-Feed $Installer $InstallerSignature $Critical.IsPresent $OutDir }
    'None' { Get-Help $PSCommandPath -Detailed; exit 2 }
    default { Invoke-Sign }
}
