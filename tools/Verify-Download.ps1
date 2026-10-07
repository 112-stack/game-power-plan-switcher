#requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
<# Read-only and offline by default. Obtain this script, checksum file and any
   identity/thumbprint pins from a trusted channel. Matching attacker-supplied
   hashes cannot prove authenticity. An inspected EXE is never run implicitly.
   -AllowUnsigned permits a clearly labeled integrity-only check of local builds.
   -RunManifestCheck explicitly runs a trusted, signed EXE's diagnostic command.
   -OnlineAuthenticodeReport opts into the PowerShell cmdlet, which may retrieve
    certificate-chain/revocation material online. No Windows settings are changed.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$ExePath,
    [Parameter(Mandatory=$true)][string]$ManifestPath,
    [Parameter(Mandatory=$true)][string]$ChecksumPath,
    [string]$SigstoreBundle,
    [string]$ExpectedIdentity,
    [string]$ExpectedIssuer,
    [string]$TrustedRootPath,
    [ValidatePattern('^[a-fA-F0-9]{40}$')][string]$ExpectedSignerThumbprint,
    [switch]$RunManifestCheck,
    [switch]$AllowUnsigned,
    [switch]$OnlineAuthenticodeReport
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ReleaseVerification.ps1')
$exeLock = $null
$manifestLock = $null
try {
    $exe = Resolve-ReleaseFile $ExePath
    $manifestFile = Resolve-ReleaseFile $ManifestPath
    # Keep these file identities stable for all checks and any explicitly
    # requested diagnostic. Read-sharing allows verification, not replacement.
    $exeLock = [IO.File]::Open($exe, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $manifestLock = [IO.File]::Open($manifestFile, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $sums = Read-ReleaseChecksums $ChecksumPath
    $exeHash = Assert-ReleaseHash $exe $sums
    $manifestHash = Assert-ReleaseHash $manifestFile $sums
    $manifest = Read-ReleaseManifest $manifestFile
    if ($manifest.sha256 -cne $exeHash -or $manifest.bytes -ne (Get-Item -LiteralPath $exe).Length) { throw 'Manifest does not describe these executable bytes.' }
    Write-Host 'PASS: EXE and manifest match the checksum file; manifest hash/size match the EXE.'
    $signature = Get-OfflineAuthenticode $exe
    Write-Host "Authenticode (offline cached trust): $($signature.Status) [$($signature.ErrorCode)]"
    Write-Host "Signer subject: $($signature.Subject)"
    Write-Host "Signer thumbprint: $($signature.Thumbprint)"
    Write-Host "Certificate NotAfter (UTC): $($signature.NotAfter)"
    Write-Host "Timestamp present: $($signature.TimestampPresent)"
    if ($OnlineAuthenticodeReport) {
        $online = Get-AuthenticodeSignature -LiteralPath $exe
        Write-Host "Get-AuthenticodeSignature status (online retrieval allowed): $($online.Status)"
        if ($online.Status -notin @('Valid','NotSigned')) { throw "Windows signature report: $($online.Status)" }
    }
    $trustedSignature = $signature.Status -eq 'Valid'
    $publisherMatched = $false
    if ($signature.Status -eq 'NotSigned' -and $AllowUnsigned) {
        if ($ExpectedSignerThumbprint) { throw 'A signer pin was supplied but the file is unsigned.' }
        if ($RunManifestCheck) { throw 'Refusing to execute an unsigned download. Run diagnostics yourself only after separately establishing trust.' }
        Write-Host 'UNSIGNED: integrity only; no Authenticode publisher identity was established.'
    } elseif (-not $trustedSignature) { throw 'Authenticode trust failed. Offline caches may be incomplete; this is not a PASS. Do not disable Windows protections.' }
    if ($trustedSignature) {
        if (-not $signature.TimestampPresent) { throw 'A signed release requires a verified timestamp.' }
        if ($ExpectedSignerThumbprint -and $signature.Thumbprint -ine $ExpectedSignerThumbprint) { throw 'The signer does not match the independently supplied thumbprint.' }
        if ($ExpectedSignerThumbprint) { $publisherMatched = $true }
        if (-not $ExpectedSignerThumbprint) { Write-Host 'No publisher pin supplied: confirm the displayed subject against an independent trusted source.' }
    }
    if ($manifest.schema -eq 2) {
        if ($manifest.signing.signed -ne $trustedSignature) { throw 'Manifest signing claim disagrees with observed Authenticode trust.' }
        if ($trustedSignature) {
            if ($manifest.signing.signer_subject -cne $signature.Subject -or $manifest.signing.signer_thumbprint -ine $signature.Thumbprint -or [DateTimeOffset]::Parse($manifest.signing.not_after).UtcDateTime -ne [DateTimeOffset]::Parse($signature.NotAfter).UtcDateTime) { throw 'Manifest signer metadata does not match the certificate.' }
        }
    }
    if (-not $SigstoreBundle) {
        $candidate = [IO.Path]::ChangeExtension($manifestFile, '.sigstore.json')
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { $SigstoreBundle = $candidate }
    }
    if ($SigstoreBundle) {
        if (-not $ExpectedIdentity -or -not $ExpectedIssuer -or -not $TrustedRootPath) { throw 'Bundle verification requires ExpectedIdentity, ExpectedIssuer and a separately trusted local TrustedRootPath.' }
        $bundle = Resolve-ReleaseFile $SigstoreBundle
        $trustRoot = Resolve-ReleaseFile $TrustedRootPath
        $cosign = (Get-Command cosign -CommandType Application -ErrorAction Stop).Source
        $helpText = (& $cosign verify-blob --help 2>&1 | Out-String)
        if ($LASTEXITCODE -ne 0 -or $helpText -notmatch '--trusted-root') { throw 'A Cosign version supporting local --trusted-root verification is required.' }
        $arguments = @('verify-blob', $manifestFile, '--bundle', $bundle, '--certificate-identity', $ExpectedIdentity, '--certificate-oidc-issuer', $ExpectedIssuer, '--trusted-root', $trustRoot)
        if ($helpText -match '--offline') { $arguments += '--offline' }
        & $cosign @arguments
        if ($LASTEXITCODE -ne 0) { throw 'Sigstore identity, signature or inclusion proof verification failed.' }
        $publisherMatched = $true
        Write-Host 'PASS: Sigstore authenticated this manifest for the exact expected identity and issuer.'
    } elseif ($ExpectedIdentity -or $ExpectedIssuer -or $TrustedRootPath) { throw 'Sigstore parameters were supplied but no bundle was found.' }
    if ($RunManifestCheck) {
        if (-not $publisherMatched) { throw 'Executing the diagnostic requires an independent ExpectedSignerThumbprint or a verified Sigstore identity, in addition to valid Authenticode.' }
        # Recheck after external tools before the explicit opt-in execution.
        $null = Assert-ReleaseHash $exe $sums
        $null = Assert-ReleaseHash $manifestFile $sums
        $process = Start-Process -FilePath $exe -ArgumentList @('--verify-release', ('"' + $manifestFile + '"')) -WindowStyle Hidden -PassThru
        if (-not $process.WaitForExit(30000)) { $process.Kill(); throw 'Manifest diagnostic timed out.' }
        if ($process.ExitCode -ne 0) { throw "Built-in manifest diagnostic failed ($($process.ExitCode))." }
        Write-Host 'PASS: the explicitly launched diagnostic agrees with the manifest.'
    } else { Write-Host 'NOT RUN: built-in EXE diagnostic (use -RunManifestCheck only when you trust this executable).' }
    Write-Host 'Limits: hashes show byte agreement, not safety. Authenticode shows cached Windows trust and file integrity, not bug-free code, fresh revocation status or guaranteed SmartScreen reputation.'
    if ($trustedSignature) { Write-Host 'PASS: requested integrity and signature checks passed.' }
    elseif ($publisherMatched) { Write-Host 'PASS: integrity and Sigstore identity verified; the executable remains Authenticode-unsigned.' }
    else { Write-Host 'PASS (INTEGRITY ONLY): unsigned file matches supplied hashes; publisher identity is unverified.' }
    exit 0
} catch {
    Write-Host ('FAIL: ' + $_.Exception.Message) -ForegroundColor Red
    exit 1
} finally {
    if ($null -ne $manifestLock) { $manifestLock.Dispose() }
    if ($null -ne $exeLock) { $exeLock.Dispose() }
}
