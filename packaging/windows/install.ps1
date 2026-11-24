<#
.SYNOPSIS
Installs Tarn @TAG@ for the current user, or with -Portable unpacks the build that runs without
installing. Each download must match the SHA-256 this script was published with.

.EXAMPLE
irm https://github.com/@REPO@/releases/latest/download/install.ps1 | iex

.EXAMPLE
& ([scriptblock]::Create((irm https://github.com/@REPO@/releases/latest/download/install.ps1))) -Portable -Path D:\Tarn
#>
[CmdletBinding()]
param(
    [switch]$Portable,
    [string]$Path
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$Tag = '@TAG@'
$Base = 'https://github.com/@REPO@/releases/download/@TAG@'
$SetupFile = @{ Name = '@SETUP_NAME@'; Sha256 = '@SETUP_SHA256@' }
$PortableFile = @{ Name = '@PORTABLE_NAME@'; Sha256 = '@PORTABLE_SHA256@' }

function Get-Verified($File) {
    $target = Join-Path ([IO.Path]::GetTempPath()) "tarn-$([guid]::NewGuid())-$($File.Name)"
    Invoke-WebRequest -UseBasicParsing -Uri "$Base/$($File.Name)" -OutFile $target
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $target).Hash -ne $File.Sha256) {
        Remove-Item -LiteralPath $target -Force
        throw "$($File.Name) does not match the SHA-256 published with this script; nothing was installed."
    }
    $target
}

function Assert-NoLink($Item) {
    if ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "$($Item.FullName) is a link; remove it, then run this again." }
    if ($Item.PSIsContainer) { Get-ChildItem -LiteralPath $Item.FullName -Force | ForEach-Object { Assert-NoLink $_ } }
}

function Remove-Tree($Item) {
    if ($Item.PSIsContainer -and -not ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { Get-ChildItem -LiteralPath $Item.FullName -Force | ForEach-Object { Remove-Tree $_ } }
    Remove-Item -LiteralPath $Item.FullName -Force
}

function Install-Portable([string]$Target) {
    if (-not $Target) { $Target = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Tarn Portable' }
    $full = [IO.Path]::GetFullPath($Target)
    if ([IO.Path]::GetPathRoot($full) -eq $full) { throw "$full is a drive; choose a folder on it with -Path." }
    $folder = $full.TrimEnd('\', '/')
    if (Test-Path -LiteralPath $folder -PathType Leaf) { throw "$folder is a file." }
    $existing = Test-Path -LiteralPath (Join-Path $folder 'tarn-portable') -PathType Leaf
    if (-not $existing -and (Test-Path -LiteralPath $folder) -and (Get-ChildItem -LiteralPath $folder -Force | Select-Object -First 1)) {
        throw "$folder is neither empty nor a portable Tarn; choose another folder with -Path."
    }
    if (Get-Process -Name tarn -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith("$folder\", [StringComparison]::OrdinalIgnoreCase) }) {
        throw "Quit the portable Tarn in $folder, then run this again."
    }
    $zip = Get-Verified $PortableFile
    $stage = "$folder.new-$([guid]::NewGuid())"
    try {
        Expand-Archive -LiteralPath $zip -DestinationPath $stage
        $package = Join-Path $stage 'Tarn'
        $names = @(Get-ChildItem -LiteralPath $package -Force | ForEach-Object Name)
        New-Item -ItemType Directory -Force -Path $folder | Out-Null
        $old = @(Get-ChildItem -LiteralPath $folder -Force | Where-Object {
            $_.Name -ne 'tarn-portable' -and ($names -contains $_.Name -or $_.Name -like '*.dll' -or $_.Name -like '*.pak' -or $_.Name -like '*.bin')
        })
        $old | ForEach-Object { Assert-NoLink $_ }
        $old | ForEach-Object { Remove-Tree $_ }
        Get-ChildItem -LiteralPath $package -Force | Where-Object { -not (Test-Path -LiteralPath (Join-Path $folder $_.Name)) } |
            Move-Item -Destination $folder
    } finally {
        Remove-Item -LiteralPath $zip, $stage -Recurse -Force -ErrorAction SilentlyContinue
    }
    Write-Host "Portable Tarn $Tag is in $folder. Start tarn.exe there; your data stays in $folder\Data."
}

function Install-Tarn {
    $setup = Get-Verified $SetupFile
    try {
        $process = Start-Process -FilePath $setup -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "The Tarn installer stopped with exit code $($process.ExitCode)." }
    } finally {
        Remove-Item -LiteralPath $setup -Force -ErrorAction SilentlyContinue
    }
    Write-Host "Tarn $Tag is installed. Open it from the Start menu; it keeps itself up to date."
}

if (-not [Environment]::Is64BitOperatingSystem) { throw 'Tarn needs 64-bit Windows 10 or later.' }
if ($Portable) { Install-Portable $Path } else { Install-Tarn }
