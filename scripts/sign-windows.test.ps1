$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'sign-windows.ps1')
$failed = 0

function Expect([string]$Name, [bool]$Want, [bool]$Got) {
    if ($Want -ne $Got) { Write-Host "FAIL ${Name}: expected $Want, got $Got"; $script:failed = 1 }
}

function Hex([string]$Text) { , [byte[]]($Text -split '(..)' -ne '' | ForEach-Object { [Convert]::ToByte($_, 16) }) }

function Verify([string]$Key, [byte[]]$Message, [string]$Signature) {
    Test-Ed25519 (Hex $Key) (New-Object System.IO.MemoryStream(, $Message)) (Hex $Signature)
}

$key1 = 'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a'
$sig1 = 'e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b'
$key2 = '3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c'
$sig2 = '92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00'

Expect 'rfc8032 test 1' $true (Verify $key1 @() $sig1)
Expect 'rfc8032 test 2' $true (Verify $key2 @(0x72) $sig2)
Expect 'other message' $false (Verify $key2 @(0x73) $sig2)
Expect 'other key' $false (Verify $key1 @(0x72) $sig2)
Expect 'flipped signature' $false (Verify $key1 @() ($sig1.Substring(0, 126) + '0a'))
Expect 'short signature' $false (Verify $key1 @() $sig1.Substring(0, 126))

$sig1Text = [Convert]::ToBase64String((Hex $sig1))
Expect 'base64 signature' $true ((ConvertFrom-Signature $sig1Text).Length -eq 64)
Expect 'signature with a newline' $true ($(try { [void](ConvertFrom-Signature "$sig1Text`n"); $false } catch { $true }))

$json = Get-FeedJson '0.4.0' $false ('ab' * 32) 123 'SIG'
Expect 'feed format' $true ($json -ceq '{"version":"0.4.0","critical":false,"installer":{"url":"https://github.com/Quavon-dev/tarn-browser/releases/download/v0.4.0/Tarn-v0.4.0-windows-x64-setup.exe","sha256":"abababababababababababababababababababababababababababababababab","length":123,"signature":"SIG"}}')
Expect 'critical feed' $true ((Get-FeedJson '1.0' $true 'x' 1 'y') -clike '*"critical":true,*')

Expect 'microsoft subject' $true (Test-RedistributorSubject 'CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US')
Expect 'google subject' $true (Test-RedistributorSubject 'CN=Google LLC, O=Google LLC, L=Mountain View, S=California, C=US, SERIALNUMBER=1')
Expect 'spoofed subject' $false (Test-RedistributorSubject 'CN="Evil, O=Microsoft Corporation, L=Redmond, x", O=Evil Inc, C=US')
Expect 'other publisher' $false (Test-RedistributorSubject 'CN=Evil, O=Evil Inc, L=Redmond, C=US')

if ($failed) { exit 1 }
Write-Host 'sign-windows.test.ps1: all passed'
