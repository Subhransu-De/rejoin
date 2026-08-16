[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^v\d+\.\d+\.\d+$')]
    [string] $Tag,

    [string] $Repository = 'Subhransu-De/rejoin',

    [string] $SigningKey
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-Checked {
    param(
        [Parameter(Mandatory)]
        [string] $Program,

        [Parameter(ValueFromRemainingArguments)]
        [string[]] $Arguments
    )

    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program failed with exit code $LASTEXITCODE."
    }
}

if (-not $SigningKey) {
    $SigningKey = (git config --get user.signingkey).Trim()
}
if (-not $SigningKey) {
    throw 'No signing key was provided and git user.signingkey is empty.'
}
$expectedFingerprint = '039EED8E5BFCC20392DBDFD97D0D1D64441ECACF'
$fingerprintRecord = & gpg --batch --with-colons --fingerprint $SigningKey |
    Where-Object { $_ -like 'fpr:*' } |
    Select-Object -First 1
if ($LASTEXITCODE -ne 0 -or -not $fingerprintRecord) {
    throw "Could not resolve GPG signing key $SigningKey."
}
$actualFingerprint = $fingerprintRecord.Split(':')[9]
if ($actualFingerprint -ne $expectedFingerprint) {
    throw "Signing key fingerprint $actualFingerprint does not match the documented release key."
}

$release = gh release view $Tag --repo $Repository --json isDraft,url | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) {
    throw "Could not read release $Tag."
}
if (-not $release.isDraft) {
    throw "Release $Tag is already published; refusing to replace its signatures."
}

Invoke-Checked git tag --verify $Tag

$temporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) "rejoin-$Tag-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null

try {
    Invoke-Checked gh release download $Tag --repo $Repository --dir $temporaryDirectory

    $manifestPath = Join-Path $temporaryDirectory 'SHA256SUMS'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw 'The draft release does not contain SHA256SUMS.'
    }

    $entries = @{}
    foreach ($line in Get-Content -LiteralPath $manifestPath) {
        if ($line -notmatch '^(?<hash>[0-9a-f]{64}) [ *](?<name>.+)$') {
            throw "Invalid SHA256SUMS line: $line"
        }
        $entries[$Matches.name] = $Matches.hash
    }
    if ($entries.Count -eq 0) {
        throw 'SHA256SUMS is empty.'
    }

    $version = $Tag.Substring(1)
    $expectedAssets = @(
        "rejoin-$Tag-linux-x64.tar.gz"
        "rejoin-$Tag-windows-x64.exe"
        "rejoin-$Tag-windows-x64.msi"
        "rejoin-$Tag-windows-x64.zip"
        "rejoin-$version-1.x86_64.rpm"
        "rejoin_$($version)_amd64.deb"
    )
    $missingAssets = $expectedAssets | Where-Object { -not $entries.ContainsKey($_) }
    $unexpectedAssets = $entries.Keys | Where-Object { $_ -notin $expectedAssets }
    if ($missingAssets) {
        throw "Expected assets are missing from SHA256SUMS: $($missingAssets -join ', ')"
    }
    if ($unexpectedAssets) {
        throw "Unexpected assets are present in SHA256SUMS: $($unexpectedAssets -join ', ')"
    }

    foreach ($entry in $entries.GetEnumerator()) {
        $assetPath = Join-Path $temporaryDirectory $entry.Key
        if (-not (Test-Path -LiteralPath $assetPath -PathType Leaf)) {
            throw "SHA256SUMS references a missing asset: $($entry.Key)"
        }
        $actualHash = (Get-FileHash -LiteralPath $assetPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualHash -ne $entry.Value) {
            throw "Checksum mismatch for $($entry.Key)."
        }
    }

    $downloadedAssets = Get-ChildItem -LiteralPath $temporaryDirectory -File |
        Where-Object Name -NotIn @('SHA256SUMS', 'SHA256SUMS.asc', 'rejoin-release-key.asc')
    $unlistedAssets = $downloadedAssets | Where-Object Name -NotIn $entries.Keys
    if ($unlistedAssets) {
        throw "Unsigned assets are missing from SHA256SUMS: $($unlistedAssets.Name -join ', ')"
    }

    $signaturePath = Join-Path $temporaryDirectory 'SHA256SUMS.asc'
    $publicKeyPath = Join-Path $temporaryDirectory 'rejoin-release-key.asc'
    Invoke-Checked gpg --yes --armor --local-user $SigningKey --output $signaturePath --detach-sign $manifestPath
    Invoke-Checked gpg --yes --armor --output $publicKeyPath --export $SigningKey
    Invoke-Checked gpg --verify $signaturePath $manifestPath

    Invoke-Checked gh release upload $Tag $signaturePath $publicKeyPath --repo $Repository --clobber
    Invoke-Checked gh release edit $Tag --repo $Repository --draft=false --latest

    Write-Output "Published $($release.url) with a GPG-signed checksum manifest."
}
finally {
    if (Test-Path -LiteralPath $temporaryDirectory) {
        Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force
    }
}
