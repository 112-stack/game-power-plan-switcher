#requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
<#
.SYNOPSIS
Signs a fresh COPY of an executable, then verifies Authenticode and timestamp.
.DESCRIPTION
This explicit publisher step is never called by cargo/build.rs. Install the
Windows SDK and a CA-issued OV/EV code-signing certificate first. Hardware-token
and cloud KSP/CSP certificates should be exposed by their provider in the chosen
Windows certificate store; their provider may prompt for a PIN. No certificate
is generated, imported, or added to a trusted-root store by this script.

PFX fallback: set NN6_SIGN_PFX_PASSWORD in this process or pass a SecureString
from Read-Host -AsSecureString. Microsoft SignTool needs /p in its process
arguments, so a local process inspector can see a PFX password during signing.
Prefer certificate-store/token signing. The script never prints the password.

Signing/timestamping and Windows chain verification can contact CA servers.
The original file is untouched; the destination is committed only on success.
.EXAMPLE
.\tools\Sign-Release.ps1 -ExePath .\target\app.exe -Thumbprint '40_HEX_DIGITS' `
    -SignedExePath .\release\Game-Power-Plan-Switcher.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$ExePath,
    [string]$SignedExePath,
    [string]$Thumbprint = $env:NN6_SIGN_CERT_THUMBPRINT,
    [ValidateSet('CurrentUser','LocalMachine')][string]$CertificateStore = 'CurrentUser',
    [string]$PfxPath,
    [Security.SecureString]$PfxPassword,
    [string]$PfxPasswordEnvironmentVariable = 'NN6_SIGN_PFX_PASSWORD',
    [string]$TimestampUrl = 'http://timestamp.digicert.com',
    [string]$Description = 'Game Power Plan Switcher',
    [string]$DescriptionUrl = 'https://github.com/112-stack/game-power-plan-switcher',
    [string]$SignToolPath
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

function Find-GppsSignTool {
    param([string]$ExplicitPath)
    if ($ExplicitPath) {
        $candidate = (Resolve-Path -LiteralPath $ExplicitPath -ErrorAction Stop).ProviderPath
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf) -or [IO.Path]::GetExtension($candidate) -ne '.exe') { throw 'SignToolPath must identify signtool.exe from the Windows SDK.' }
        return $candidate
    }
    # Respect WindowsSdkDir first. Prefer x64 tools and newest SDK versions.
    $roots = @($env:WindowsSdkDir)
    foreach ($programFiles in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if ($programFiles) { $roots += (Join-Path $programFiles 'Windows Kits\10'); $roots += (Join-Path $programFiles 'Windows Kits\8.1') }
    }
    foreach ($sdkRoot in @($roots | Where-Object { $_ } | Select-Object -Unique)) {
        $bin = Join-Path $sdkRoot 'bin'
        if (-not (Test-Path -LiteralPath $bin -PathType Container)) { continue }
        $versions = @(Get-ChildItem -LiteralPath $bin -Directory -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '^\d+(\.\d+)+$' } | Sort-Object { [version]$_.Name } -Descending)
        foreach ($folder in @($versions | ForEach-Object { $_.FullName }) + @($bin)) {
            foreach ($arch in @('x64','x86','arm64')) {
                $candidate = Join-Path $folder "$arch\signtool.exe"
                if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
            }
        }
    }
    throw 'SignTool was not found. Install the Windows SDK signing tools, set WindowsSdkDir, or provide -SignToolPath.'
}

function Invoke-GppsSignTool {
    param([string]$Tool, [string[]]$Arguments, [string]$SensitiveValue)
    # Capture tool output rather than echoing the command (which could contain /p).
    $toolOutput = @(& $Tool @Arguments 2>&1)
    $toolExit = $LASTEXITCODE
    foreach ($line in $toolOutput) {
        $safeLine = [string]$line
        if ($SensitiveValue) { $safeLine = $safeLine.Replace($SensitiveValue, '[REDACTED]') }
        Write-Host $safeLine
    }
    if ($toolExit -ne 0) { throw "SignTool failed with exit code $toolExit. No signed release was committed." }
}

function Invoke-GppsSigning {
    [CmdletBinding()]
    param(
        [string]$ExePath, [string]$SignedExePath, [string]$Thumbprint,
        [string]$CertificateStore = 'CurrentUser', [string]$PfxPath,
        [Security.SecureString]$PfxPassword,
        [string]$PfxPasswordEnvironmentVariable = 'NN6_SIGN_PFX_PASSWORD',
        [string]$TimestampUrl = 'http://timestamp.digicert.com',
        [string]$Description = 'Game Power Plan Switcher',
        [string]$DescriptionUrl = 'https://github.com/112-stack/game-power-plan-switcher',
        [string]$SignToolPath
    )
    if ([bool]$Thumbprint -eq [bool]$PfxPath) { throw 'Choose exactly one signing credential: -Thumbprint (or NN6_SIGN_CERT_THUMBPRINT), or -PfxPath.' }
    if ($PfxPassword -and -not $PfxPath) { throw 'PfxPassword is only applicable with PfxPath.' }
    if ($CertificateStore -notin @('CurrentUser','LocalMachine')) { throw 'CertificateStore must be CurrentUser or LocalMachine.' }
    foreach ($url in @($TimestampUrl,$DescriptionUrl)) {
        $uri = $null
        if (-not [Uri]::TryCreate($url,[UriKind]::Absolute,[ref]$uri) -or $uri.Scheme -notin @('http','https') -or $uri.UserInfo) { throw 'TimestampUrl and DescriptionUrl must be absolute HTTP(S) URLs without credentials.' }
    }
    if (-not $Description -or $Description -match '[\r\n]') { throw 'Description must be a nonempty, single-line product name.' }
    $source = (Resolve-Path -LiteralPath $ExePath -ErrorAction Stop).ProviderPath
    if (-not (Test-Path -LiteralPath $source -PathType Leaf) -or [IO.Path]::GetExtension($source) -ne '.exe') { throw 'ExePath must be an existing Windows .exe file.' }
    if (-not $SignedExePath) { $SignedExePath = Join-Path ([IO.Path]::GetDirectoryName($source)) ([IO.Path]::GetFileNameWithoutExtension($source) + '.signed.exe') }
    $destination = [IO.Path]::GetFullPath($SignedExePath)
    if ([IO.Path]::GetExtension($destination) -ne '.exe') { throw 'SignedExePath must have an .exe extension.' }
    if ([string]::Equals($source,$destination,[StringComparison]::OrdinalIgnoreCase) -or (Test-Path -LiteralPath $destination)) { throw 'SignedExePath must be a new file, distinct from ExePath. Existing releases are never overwritten.' }
    $parent = [IO.Path]::GetDirectoryName($destination)
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) { throw 'Create the destination directory before signing.' }
    $tool = Find-GppsSignTool $SignToolPath
    $arguments = @('sign','/fd','SHA256','/tr',$TimestampUrl,'/td','SHA256','/d',$Description,'/du',$DescriptionUrl)
    if ($Thumbprint) {
        $Thumbprint = ($Thumbprint -replace '\s','').ToUpperInvariant()
        if ($Thumbprint -notmatch '^[A-F0-9]{40}$') { throw 'Thumbprint must contain exactly 40 hexadecimal characters (SHA-1 certificate identifier; file digest remains SHA-256).' }
        $certificate = Get-Item -LiteralPath "Cert:\$CertificateStore\My\$Thumbprint" -ErrorAction Stop
        if (-not $certificate.HasPrivateKey -or $certificate.NotBefore -gt (Get-Date) -or $certificate.NotAfter -le (Get-Date)) { throw 'Selected certificate has no available private key or is not currently valid.' }
        $eku = @($certificate.EnhancedKeyUsageList | ForEach-Object { $_.ObjectId.Value })
        if ($eku -notcontains '1.3.6.1.5.5.7.3.3') { throw 'Selected certificate does not include the Code Signing extended key usage.' }
        $arguments += @('/sha1',$Thumbprint,'/s','My')
        if ($CertificateStore -eq 'LocalMachine') { $arguments += '/sm' }
    } else {
        $pfx = (Resolve-Path -LiteralPath $PfxPath -ErrorAction Stop).ProviderPath
        if (-not (Test-Path -LiteralPath $pfx -PathType Leaf)) { throw 'PfxPath must be an existing PFX file.' }
        $arguments += @('/f',$pfx)
    }
    $temporary = Join-Path $parent ('.signing-' + [Guid]::NewGuid().ToString('N') + '.exe')
    $plainPassword = $null
    $passwordPointer = [IntPtr]::Zero
    $ownsTemporary = $false
    try {
        [IO.File]::Copy($source,$temporary,$false)
        $ownsTemporary = $true
        if ($PfxPath) {
            if ($PfxPassword) {
                $passwordPointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($PfxPassword)
                $plainPassword = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($passwordPointer)
            } else {
                $plainPassword = [Environment]::GetEnvironmentVariable($PfxPasswordEnvironmentVariable, 'Process')
            }
            if ($null -ne $plainPassword -and $plainPassword.Length -gt 0) { $arguments += @('/p',$plainPassword) }
        }
        $arguments += $temporary
        Invoke-GppsSignTool -Tool $tool -Arguments $arguments -SensitiveValue $plainPassword
        Invoke-GppsSignTool -Tool $tool -Arguments @('verify','/pa','/all','/v',$temporary)
        $signature = Get-AuthenticodeSignature -LiteralPath $temporary
        if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate) { throw "Authenticode status is $($signature.Status); refusing to publish the copy." }
        if (-not $signature.TimeStamperCertificate) { throw 'No trusted timestamp was observed; refusing to publish an untimestamped copy.' }
        if ($Thumbprint -and $signature.SignerCertificate.Thumbprint -ne $Thumbprint) { throw 'Observed signer does not match the selected certificate.' }
        # File.Move fails rather than replacing a destination created during signing.
        [IO.File]::Move($temporary,$destination)
        $result = [pscustomobject]@{
            Path = $destination
            signed = $true
            signer_subject = $signature.SignerCertificate.Subject
            signer_thumbprint = $signature.SignerCertificate.Thumbprint.ToUpperInvariant()
            not_after = $signature.SignerCertificate.NotAfter.ToUniversalTime().ToString('o')
            timestamp_present = $true
            sha256 = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        Write-Host "Signed: $destination"
        Write-Host "Signer: $($result.signer_subject)"
        Write-Host "Thumbprint: $($result.signer_thumbprint)"
        Write-Host "Certificate NotAfter (UTC): $($result.not_after)"
        return $result
    } finally {
        if ($passwordPointer -ne [IntPtr]::Zero) { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($passwordPointer) }
        $plainPassword = $null
        $arguments = $null
        # Delete only this invocation's exact staging file, never a directory.
        if ($ownsTemporary -and (Test-Path -LiteralPath $temporary -PathType Leaf)) { Remove-Item -LiteralPath $temporary -Force }
    }
}

Invoke-GppsSigning -ExePath $ExePath -SignedExePath $SignedExePath -Thumbprint $Thumbprint `
    -CertificateStore $CertificateStore -PfxPath $PfxPath -PfxPassword $PfxPassword `
    -PfxPasswordEnvironmentVariable $PfxPasswordEnvironmentVariable -TimestampUrl $TimestampUrl `
    -Description $Description -DescriptionUrl $DescriptionUrl -SignToolPath $SignToolPath
