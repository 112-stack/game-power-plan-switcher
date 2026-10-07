#requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
<#
.SYNOPSIS
Builds, explicitly signs, and verifies a new, versioned publisher release.
.DESCRIPTION
Uses the repository's pinned GNU-LLVM Build.ps1 (cargo --release --locked).
The output directory must not exist. All work is staged in a unique sibling
directory whose .incomplete name and marker prohibit accidental publication.
Only a completely verified set is renamed to OutputDirectory. On failure the
staging directory remains for diagnosis; no previous release is overwritten.

UseSigstore is explicit because keyless signing uses OIDC/network services and
publishes certificate identity/digest information in a public transparency log.
Acquire the trusted-root JSON independently from Sigstore's authenticated TUF
distribution; a root included with an untrusted download is not a trust anchor.
No signature, signing certificate, CA trust, or SmartScreen reputation is faked.
.EXAMPLE
.\tools\Build-Signed-Release.ps1 -LlvmMingw C:\llvm-mingw `
    -LlvmMingwRelease '20260922-ucrt' -OutputDirectory C:\releases\gpps-next `
    -Thumbprint '40_HEX_DIGITS'
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$LlvmMingw,
    [Parameter(Mandatory=$true)][string]$LlvmMingwRelease,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [string]$Thumbprint = $env:NN6_SIGN_CERT_THUMBPRINT,
    [ValidateSet('CurrentUser','LocalMachine')][string]$CertificateStore = 'CurrentUser',
    [string]$PfxPath,
    [Security.SecureString]$PfxPassword,
    [string]$PfxPasswordEnvironmentVariable = 'NN6_SIGN_PFX_PASSWORD',
    [string]$TimestampUrl = 'http://timestamp.digicert.com',
    [string]$Description = 'Game Power Plan Switcher',
    [string]$DescriptionUrl = 'https://github.com/112-stack/game-power-plan-switcher',
    [string]$SignToolPath,
    [ValidatePattern('^[0-9]+$')][string]$SourceDateEpoch,
    [ValidatePattern('^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$')][string]$BuildUuid,
    [ValidatePattern('^(unknown|[0-9A-Fa-f]{40}|[0-9A-Fa-f]{64})$')][string]$GitCommit,
    [switch]$UseSigstore,
    [string]$ExpectedIdentity,
    [string]$ExpectedIssuer,
    [string]$TrustedRootPath
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

function Get-GppsPackageVersion {
    param([string]$CargoToml)
    # Restrict parsing to [package], rather than matching dependency versions.
    $text = [IO.File]::ReadAllText($CargoToml)
    $package = [regex]::Match($text,'(?ms)^\[package\]\s*\r?\n(?<body>.*?)(?=^\[|\z)')
    $version = [regex]::Match($package.Groups['body'].Value,'(?m)^version\s*=\s*"(?<v>[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?)"\s*$')
    if (-not $package.Success -or -not $version.Success) { throw 'Cargo.toml must contain a literal semantic version in [package].' }
    return $version.Groups['v'].Value
}

function Invoke-GppsReleaseCommand {
    param([string]$Command, [string[]]$Arguments, [string]$Label)
    & $Command @Arguments | ForEach-Object { Write-Host $_ }
    if ($LASTEXITCODE -ne 0) { throw "$Label failed with exit code $LASTEXITCODE." }
}

function Write-GppsChecksums {
    param([string[]]$Paths, [string]$OutputPath)
    $lines = foreach ($path in $Paths) {
        $name = [IO.Path]::GetFileName($path)
        if ($name -match '[\r\n\\]') { throw 'Checksum filenames must be single portable basenames.' }
        '{0}  {1}' -f (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant(),$name
    }
    [IO.File]::WriteAllText($OutputPath,(($lines -join "`n") + "`n"),(New-Object Text.UTF8Encoding($false)))
}

function Invoke-GppsSignedBuild {
    [CmdletBinding()]
    param([hashtable]$Options, [string]$RepositoryRoot)
    $version = Get-GppsPackageVersion (Join-Path $RepositoryRoot 'Cargo.toml')
    if ([bool]$Options.Thumbprint -eq [bool]$Options.PfxPath) { throw 'Choose exactly one signing credential: Thumbprint or PfxPath.' }
    if (-not $Options.LlvmMingwRelease -or $Options.LlvmMingwRelease -match '[\r\n]') { throw 'Supply the exact LLVM-MinGW release tag used to build.' }
    $destination = [IO.Path]::GetFullPath($Options.OutputDirectory).TrimEnd('\','/')
    if (Test-Path -LiteralPath $destination) { throw 'OutputDirectory must not exist; previous release artifacts are never overwritten.' }
    $parent = [IO.Path]::GetDirectoryName($destination)
    if (-not $parent -or -not (Test-Path -LiteralPath $parent -PathType Container)) { throw 'Create the parent of OutputDirectory first.' }
    $signScript = Join-Path $RepositoryRoot 'tools\Sign-Release.ps1'
    # Fail early on missing SDK, before an expensive release build. Certificate
    # trust/private-key access is verified by Sign-Release after the build.
    $signAst = [Management.Automation.Language.Parser]::ParseFile($signScript,[ref]$null,[ref]$null)
    $findFunction = $signAst.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Find-GppsSignTool' },$false)
    . ([scriptblock]::Create($findFunction.Extent.Text))
    $resolvedSignTool = Find-GppsSignTool $Options.SignToolPath
    $cosign = $null
    if ($Options.UseSigstore) {
        if (-not $Options.ExpectedIdentity -or -not $Options.ExpectedIssuer -or -not $Options.TrustedRootPath) { throw 'UseSigstore requires ExpectedIdentity, ExpectedIssuer and an independently trusted TrustedRootPath.' }
        $cosign = (Get-Command cosign -CommandType Application -ErrorAction Stop).Source
        $Options.TrustedRootPath = (Resolve-Path -LiteralPath $Options.TrustedRootPath -ErrorAction Stop).ProviderPath
        $help = (& $cosign verify-blob --help 2>&1) -join "`n"
        if ($LASTEXITCODE -ne 0 -or $help -notmatch '--trusted-root') { throw 'This pipeline requires Cosign with verify-blob --trusted-root support.' }
        $cosignOffline = if ($help -match '--offline') { @('--offline') } else { @() }
    }
    $stage = Join-Path $parent ('.' + [IO.Path]::GetFileName($destination) + '.incomplete-' + [Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($stage)
    $marker = Join-Path $stage 'INCOMPLETE-DO-NOT-PUBLISH.txt'
    [IO.File]::WriteAllText($marker,"Signing pipeline has not completed. This directory is not a release.`r`n")
    $environmentNames = @('PATH','CC','CXX','AR','NN6_WINDRES','CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER','RUSTFLAGS','CARGO_INCREMENTAL','CARGO_TARGET_DIR','SOURCE_DATE_EPOCH','NN6_BUILD_UUID','NN6_GIT_COMMIT','NN6_LLVM_MINGW_VERSION','NN6_SIGN_CERT_THUMBPRINT')
    $priorEnvironment = @{}
    foreach ($name in $environmentNames) { $priorEnvironment[$name] = [Environment]::GetEnvironmentVariable($name,'Process') }
    try {
        $env:NN6_LLVM_MINGW_VERSION = $Options.LlvmMingwRelease
        $env:NN6_SIGN_CERT_THUMBPRINT = $Options.Thumbprint
        $buildOptions = @{ LlvmMingw=$Options.LlvmMingw; Action='build' }
        foreach ($key in @('SourceDateEpoch','BuildUuid','GitCommit')) { if ($Options[$key]) { $buildOptions[$key] = $Options[$key] } }
        & (Join-Path $RepositoryRoot 'Build.ps1') @buildOptions | ForEach-Object { Write-Host $_ }
        $builtExe = Join-Path $RepositoryRoot 'target\x86_64-pc-windows-gnullvm\release\Game-Power-Plan-Switcher.exe'
        if (-not (Test-Path -LiteralPath $builtExe -PathType Leaf)) { throw 'Release build did not produce the expected executable.' }
        $exe = Join-Path $stage "Game-Power-Plan-Switcher-$version.exe"
        $signOptions = @{ ExePath=$builtExe; SignedExePath=$exe; SignToolPath=$resolvedSignTool }
        foreach ($key in @('Thumbprint','CertificateStore','PfxPath','PfxPassword','PfxPasswordEnvironmentVariable','TimestampUrl','Description','DescriptionUrl')) { if ($Options[$key]) { $signOptions[$key]=$Options[$key] } }
        $observed = & $signScript @signOptions
        if (-not $observed.signed -or -not $observed.timestamp_present) { throw 'Signing did not return a verified, timestamped result.' }
        $manifestPath = Join-Path $stage 'Game-Power-Plan-Switcher-release-manifest.json'
        Invoke-GppsReleaseCommand -Command $exe -Arguments @('--write-release-manifest',$manifestPath) -Label 'Manifest generation'
        # Rust emits UTF-8 without a BOM. Windows PowerShell 5.1 Get-Content
        # defaults to the ANSI code page, which can corrupt Unicode provenance.
        $manifest = [IO.File]::ReadAllText($manifestPath,(New-Object Text.UTF8Encoding($false,$true))) | ConvertFrom-Json
        if ($manifest.schema -ne 2) { throw 'Signed release requires manifest schema 2. Rebuild the updated source.' }
        $manifest.signing = [ordered]@{
            signed=$true; signer_subject=$observed.signer_subject
            signer_thumbprint=$observed.signer_thumbprint; not_after=$observed.not_after
        }
        [IO.File]::WriteAllText($manifestPath,(($manifest | ConvertTo-Json -Depth 32) + "`n"),(New-Object Text.UTF8Encoding($false)))
        Invoke-GppsReleaseCommand -Command $exe -Arguments @('--verify-release',$manifestPath) -Label 'Built-in signed-file manifest verification'
        # The full-file hash must describe the signed bytes, not the Cargo output.
        $actualHash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($manifest.sha256 -ne $actualHash -or $observed.sha256 -ne $actualHash) { throw 'Final signed executable no longer matches the manifest/signing observation.' }
        $artifacts = @($exe,$manifestPath)
        if ($Options.UseSigstore) {
            $bundle = Join-Path $stage 'Game-Power-Plan-Switcher-release-manifest.sigstore.json'
            Write-Host 'Explicit Sigstore signing: OIDC authentication and a public transparency-log entry may be required.'
            Invoke-GppsReleaseCommand -Command $cosign -Arguments @('sign-blob',$manifestPath,'--bundle',$bundle) -Label 'Sigstore signing'
            $verifyArguments = @('verify-blob',$manifestPath,'--bundle',$bundle,'--certificate-identity',$Options.ExpectedIdentity,'--certificate-oidc-issuer',$Options.ExpectedIssuer,'--trusted-root',$Options.TrustedRootPath) + $cosignOffline
            Invoke-GppsReleaseCommand -Command $cosign -Arguments $verifyArguments -Label 'Offline Sigstore identity verification'
            $artifacts += $bundle
        }
        $checksums = Join-Path $stage 'Game-Power-Plan-Switcher-release-checksums.txt'
        Write-GppsChecksums -Paths $artifacts -OutputPath $checksums
        $artifacts += $checksums
        # Final signature recheck catches accidental mutation after initial signing.
        Invoke-GppsReleaseCommand -Command $resolvedSignTool -Arguments @('verify','/pa','/all','/v',$exe) -Label 'Final Authenticode verification'
        Remove-Item -LiteralPath $marker
        [IO.Directory]::Move($stage,$destination)
        Write-Host "PASS: signed release staged at $destination"
        foreach ($artifact in $artifacts) {
            $final = Join-Path $destination ([IO.Path]::GetFileName($artifact))
            Write-Host ("{0}  {1}" -f (Get-FileHash -LiteralPath $final -Algorithm SHA256).Hash.ToLowerInvariant(),$final)
        }
        Write-Host 'Signing binds these bytes to the observed publisher. It does not certify code safety or eliminate SmartScreen prompts.'
        return $destination
    } catch {
        if (Test-Path -LiteralPath $stage -PathType Container) {
            if (-not (Test-Path -LiteralPath $marker)) { [IO.File]::WriteAllText($marker,"Release publication failed. Do not publish this directory.`r`n") }
            Write-Warning "Incomplete artifacts retained for diagnosis: $stage"
        }
        throw
    } finally {
        foreach ($name in $environmentNames) {
            if ($null -eq $priorEnvironment[$name]) { Remove-Item -LiteralPath ("Env:" + $name) -ErrorAction SilentlyContinue }
            else { [Environment]::SetEnvironmentVariable($name,$priorEnvironment[$name],'Process') }
        }
    }
}

$options = @{
    LlvmMingw=$LlvmMingw; LlvmMingwRelease=$LlvmMingwRelease; OutputDirectory=$OutputDirectory
    Thumbprint=$Thumbprint; CertificateStore=$CertificateStore; PfxPath=$PfxPath; PfxPassword=$PfxPassword
    PfxPasswordEnvironmentVariable=$PfxPasswordEnvironmentVariable; TimestampUrl=$TimestampUrl
    Description=$Description; DescriptionUrl=$DescriptionUrl; SignToolPath=$SignToolPath
    SourceDateEpoch=$SourceDateEpoch; BuildUuid=$BuildUuid; GitCommit=$GitCommit
    UseSigstore=[bool]$UseSigstore; ExpectedIdentity=$ExpectedIdentity; ExpectedIssuer=$ExpectedIssuer; TrustedRootPath=$TrustedRootPath
}
Invoke-GppsSignedBuild -Options $options -RepositoryRoot ([IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')))
