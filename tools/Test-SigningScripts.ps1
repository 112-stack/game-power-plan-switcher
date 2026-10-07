#requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
<# Offline contract/failure tests; no real certificate, signing, network or build.
   Functions are loaded from their AST without executing publisher entrypoints.
   A successful mock does NOT establish Authenticode validity for any release. #>
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
$script:Passed = 0
function Assert-True([bool]$Condition,[string]$Label) {
    if (-not $Condition) { throw "FAIL: $Label" }
    $script:Passed++
    Write-Host "PASS: $Label"
}
function Assert-Fails([scriptblock]$Code,[string]$Pattern,[string]$Label) {
    try { & $Code | Out-Null } catch { Assert-True ($_.Exception.Message -match $Pattern) $Label; return }
    throw "FAIL: $Label (unexpected success)"
}

foreach ($scriptName in @('Sign-Release.ps1','Build-Signed-Release.ps1')) {
    $tokens=$null; $parseErrors=$null
    $ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $scriptName),[ref]$tokens,[ref]$parseErrors)
    Assert-True ($parseErrors.Count -eq 0) "$scriptName parses"
    foreach ($definition in $ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst]},$false)) {
        . ([scriptblock]::Create($definition.Extent.Text))
    }
}
$scratch=Join-Path ([IO.Path]::GetTempPath()) ('gpps-signing-tests-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($scratch)
$originalSdk=$env:WindowsSdkDir
$originalPassword=[Environment]::GetEnvironmentVariable('GPPS_TEST_PFX_PASSWORD','Process')
try {
    $sdk=Join-Path $scratch 'sdk'
    foreach ($sdkVersion in @('10.0.9999.0','10.0.10000.0')) {
        $sdkBin=Join-Path $sdk "bin\$sdkVersion\x64"
        [void][IO.Directory]::CreateDirectory($sdkBin)
        [IO.File]::WriteAllText((Join-Path $sdkBin 'signtool.exe'),'test placeholder, never executed')
    }
    $env:WindowsSdkDir=$sdk
    Assert-True ((Find-GppsSignTool) -like '*10.0.10000.0*') 'SDK discovery chooses semantic newest version'
    $env:WindowsSdkDir=$originalSdk
    $source=Join-Path $scratch 'input.exe'
    $pfx=Join-Path $scratch 'dummy.pfx'
    [IO.File]::WriteAllText($source,'unsigned test bytes, not executable')
    [IO.File]::WriteAllText($pfx,'test placeholder, never a certificate')
    $originalHash=(Get-FileHash -LiteralPath $source).Hash
    $script:SignCalls=@()
    $script:ToolFailure=0
    $script:SignatureStatus='Valid'
    $script:TimestampPresent=$true
    function Find-GppsSignTool { param([string]$ExplicitPath) return 'MOCK-SignTool.exe' }
    function Invoke-GppsSignTool {
        param([string]$Tool,[string[]]$Arguments,[string]$SensitiveValue)
        $script:SignCalls += ,$Arguments
        if ($script:ToolFailure -eq $script:SignCalls.Count) { throw 'SignTool failed with exit code 7. No signed release was committed.' }
        if ($Arguments[0] -eq 'sign') { [IO.File]::AppendAllText($Arguments[-1],' MOCK SIGNATURE') }
    }
    function Get-AuthenticodeSignature {
        param([string]$LiteralPath)
        [pscustomobject]@{
            Status=$script:SignatureStatus
            SignerCertificate=[pscustomobject]@{Subject='CN=TEST ONLY';Thumbprint=('A'*40);NotAfter=[datetime]'2030-01-01T00:00:00Z'}
            TimeStamperCertificate=$(if($script:TimestampPresent){[pscustomobject]@{Subject='CN=TEST TSA'}}else{$null})
        }
    }
    Assert-Fails { Invoke-GppsSigning -ExePath $source } 'exactly one' 'Missing credential fails closed'
    Assert-Fails { Invoke-GppsSigning -ExePath $source -Thumbprint ('A'*40) -PfxPath $pfx } 'exactly one' 'Ambiguous credential fails closed'
    Assert-Fails { Invoke-GppsSigning -ExePath $source -PfxPath $pfx -SignedExePath $source } 'new file' 'Original EXE cannot be selected as destination'
    Assert-Fails { Invoke-GppsSigning -ExePath $source -PfxPath $pfx -TimestampUrl 'file:///secret' } 'HTTP' 'Non-HTTP timestamp URL rejected'
    Assert-Fails { Invoke-GppsSigning -ExePath $source -PfxPath $pfx -DescriptionUrl 'https://user:pass@example.test' } 'credentials' 'URLs containing credentials rejected'
    $signed=Join-Path $scratch 'signed.exe'
    [Environment]::SetEnvironmentVariable('GPPS_TEST_PFX_PASSWORD','test-only-password','Process')
    $result=Invoke-GppsSigning -ExePath $source -SignedExePath $signed -PfxPath $pfx -PfxPasswordEnvironmentVariable GPPS_TEST_PFX_PASSWORD
    Assert-True ((Get-FileHash -LiteralPath $source).Hash -eq $originalHash) 'Successful signing preserves original EXE'
    Assert-True ($result.signed -and $result.timestamp_present -and $result.signer_subject -eq 'CN=TEST ONLY') 'Observed signer metadata returned'
    Assert-True (($script:SignCalls[0] -join '|') -match '/fd\|SHA256.*?/tr\|http://timestamp.digicert.com\|/td\|SHA256') 'SignTool requests SHA-256 and RFC3161 timestamp'
    Assert-True (($script:SignCalls[1] -join '|') -match '^verify\|/pa\|/all\|/v\|') 'SignTool all-signatures Authenticode verification follows signing'
    Assert-True (($script:SignCalls[0] -join '|') -match '/p\|test-only-password') 'PFX environment password passed only to signing tool'
    Assert-Fails { Invoke-GppsSigning -ExePath $source -SignedExePath $signed -PfxPath $pfx } 'new file' 'Existing signed destination is never overwritten'
    foreach ($failure in @(1,2)) {
        $script:SignCalls=@(); $script:ToolFailure=$failure
        $failed=Join-Path $scratch "failed-$failure.exe"
        Assert-Fails { Invoke-GppsSigning -ExePath $source -SignedExePath $failed -PfxPath $pfx } 'exit code 7' "SignTool failure at step $failure aborts"
        Assert-True (-not (Test-Path -LiteralPath $failed)) "SignTool failure $failure cannot produce final EXE"
    }
    $script:ToolFailure=0; $script:SignCalls=@(); $script:SignatureStatus='HashMismatch'
    Assert-Fails { Invoke-GppsSigning -ExePath $source -SignedExePath (Join-Path $scratch 'bad-signature.exe') -PfxPath $pfx } 'HashMismatch' 'Signature validation failure aborts'
    $script:SignatureStatus='Valid'; $script:TimestampPresent=$false
    Assert-Fails { Invoke-GppsSigning -ExePath $source -SignedExePath (Join-Path $scratch 'no-timestamp.exe') -PfxPath $pfx } 'timestamp' 'Missing timestamp aborts'
    Assert-True (@(Get-ChildItem -LiteralPath $scratch -Filter '.signing-*').Count -eq 0) 'Failed and successful signing remove exact temporary files'
    Assert-True ((Get-FileHash -LiteralPath $source).Hash -eq $originalHash) 'All failure cases preserve source bytes'

    $cargo=Join-Path $scratch 'Cargo.toml'
    [IO.File]::WriteAllText($cargo,"[package]`nname = `"test`"`nversion = `"2.3.7-rc.1`"`n[dependencies]`nversion = `"99.0.0`"`n")
    Assert-True ((Get-GppsPackageVersion $cargo) -eq '2.3.7-rc.1') 'Package version excludes dependency section'
    [IO.File]::WriteAllText($cargo,"[dependencies]`nversion = `"99.0.0`"`n")
    Assert-Fails { Get-GppsPackageVersion $cargo } 'literal semantic version' 'Missing package version fails closed'
    $checksum=Join-Path $scratch 'checksums.txt'
    Write-GppsChecksums -Paths @($source,$signed) -OutputPath $checksum
    $lines=@(Get-Content -LiteralPath $checksum)
    Assert-True ($lines.Count -eq 2 -and $lines[0] -eq ($originalHash.ToLowerInvariant() + '  input.exe')) 'Checksums hash actual files and use portable basenames'
    Assert-True ($lines[1] -eq ((Get-FileHash -LiteralPath $signed).Hash.ToLowerInvariant() + '  signed.exe')) 'Signed hash differs from source hash and is captured after signing'
    Assert-True (-not ([IO.File]::ReadAllBytes($checksum)[0] -eq 0xEF)) 'Checksum file has no UTF-8 BOM'

    # Exercise the release transaction end to end with disposable build/sign
    # fixtures. Neither the real compiler nor a PE file is executed here.
    $fixture=Join-Path $scratch 'fixture'
    [void][IO.Directory]::CreateDirectory((Join-Path $fixture 'tools'))
    [IO.File]::WriteAllText((Join-Path $fixture 'Cargo.toml'),"[package]`nversion = `"2.3.7`"`n")
    [IO.File]::WriteAllText((Join-Path $fixture 'Build.ps1'),@'
param([string]$LlvmMingw,[string]$Action)
$bin=Join-Path $PSScriptRoot 'target\x86_64-pc-windows-gnullvm\release'
[void][IO.Directory]::CreateDirectory($bin)
[IO.File]::WriteAllText((Join-Path $bin 'Game-Power-Plan-Switcher.exe'),'MOCK CARGO OUTPUT')
$env:NN6_GIT_COMMIT='fixture changed environment'
'MOCK BUILD COMPLETE'
'@)
    [IO.File]::WriteAllText((Join-Path $fixture 'tools\Sign-Release.ps1'),@'
param([string]$ExePath,[string]$SignedExePath,[string]$SignToolPath,[string]$Thumbprint)
function Find-GppsSignTool { param([string]$ExplicitPath) return 'MOCK-SignTool.exe' }
[IO.File]::WriteAllText($SignedExePath,([IO.File]::ReadAllText($ExePath)+' MOCK SIGNATURE'))
[pscustomobject]@{signed=$true;timestamp_present=$true;signer_subject='CN=TEST ONLY';signer_thumbprint=('A'*40);not_after='2030-01-01T00:00:00Z';sha256=(Get-FileHash -LiteralPath $SignedExePath).Hash.ToLowerInvariant()}
'@)
    $script:BuildCommands=@()
    $script:FailManifest=$false
    function Invoke-GppsReleaseCommand {
        param([string]$Command,[string[]]$Arguments,[string]$Label)
        $script:BuildCommands += $Label
        if ($Arguments[0] -eq '--write-release-manifest') {
            if ($script:FailManifest) { throw 'Mock manifest failure' }
            $manifest=[ordered]@{schema=2;sha256=(Get-FileHash -LiteralPath $Command).Hash.ToLowerInvariant();signing=$null;provenance=[ordered]@{label=('caf'+[char]0x00E9)}}
            [IO.File]::WriteAllText($Arguments[1],($manifest | ConvertTo-Json))
        }
        if ($Arguments[0] -eq '--verify-release') {
            $manifest=Get-Content -LiteralPath $Arguments[1] -Raw | ConvertFrom-Json
            if (-not $manifest.signing.signed) { throw 'Signing metadata was not injected before CLI verification.' }
        }
    }
    $buildOptions=@{Thumbprint=('A'*40);PfxPath=$null;LlvmMingw='fixture';LlvmMingwRelease='fixture-ucrt';OutputDirectory=(Join-Path $scratch 'release-success');SignToolPath=$null;UseSigstore=$false}
    $beforeGit=$env:NN6_GIT_COMMIT
    $beforeLlvm=$env:NN6_LLVM_MINGW_VERSION
    $output=Invoke-GppsSignedBuild -Options $buildOptions -RepositoryRoot $fixture
    Assert-True ((Test-Path -LiteralPath $output -PathType Container) -and -not (Test-Path -LiteralPath (Join-Path $output 'INCOMPLETE-DO-NOT-PUBLISH.txt'))) 'Complete release is committed without incomplete marker'
    $builtManifest=[IO.File]::ReadAllText((Join-Path $output 'Game-Power-Plan-Switcher-release-manifest.json'),(New-Object Text.UTF8Encoding($false,$true))) | ConvertFrom-Json
    Assert-True ($builtManifest.signing.signed -and $builtManifest.signing.signer_subject -eq 'CN=TEST ONLY') 'Release manifest records observed signer'
    Assert-True ($builtManifest.provenance.label -ceq ('caf'+[char]0x00E9)) 'UTF-8 provenance survives signing metadata injection on PowerShell 5.1'
    Assert-True (($script:BuildCommands -join '|') -eq 'Manifest generation|Built-in signed-file manifest verification|Final Authenticode verification') 'Manifest/signature verification transaction order'
    Assert-True ($env:NN6_GIT_COMMIT -eq $beforeGit -and $env:NN6_LLVM_MINGW_VERSION -eq $beforeLlvm) 'Build restores caller environment'
    Assert-Fails { Invoke-GppsSignedBuild -Options $buildOptions -RepositoryRoot $fixture } 'must not exist' 'Build refuses to overwrite existing release directory'
    $buildOptions.OutputDirectory=Join-Path $scratch 'release-failed'
    $script:FailManifest=$true
    Assert-Fails { Invoke-GppsSignedBuild -Options $buildOptions -RepositoryRoot $fixture } 'Mock manifest failure' 'Manifest failure aborts release transaction'
    Assert-True (-not (Test-Path -LiteralPath $buildOptions.OutputDirectory)) 'Failed build never creates publishable final directory'
    $staging=@(Get-ChildItem -LiteralPath $scratch -Directory -Filter '.release-failed.incomplete-*')
    Assert-True ($staging.Count -eq 1 -and (Test-Path -LiteralPath (Join-Path $staging[0].FullName 'INCOMPLETE-DO-NOT-PUBLISH.txt'))) 'Failed artifacts retain an explicit incomplete marker'
    Assert-True ($env:NN6_GIT_COMMIT -eq $beforeGit -and $env:NN6_LLVM_MINGW_VERSION -eq $beforeLlvm) 'Failed build also restores caller environment'
    Write-Host "PASS: $script:Passed offline script checks. No real certificate/signing/network/build was exercised."
} finally {
    $env:WindowsSdkDir=$originalSdk
    [Environment]::SetEnvironmentVariable('GPPS_TEST_PFX_PASSWORD',$originalPassword,'Process')
    # Resolve and validate our exact disposable workspace before recursive cleanup.
    $fullScratch=[IO.Path]::GetFullPath($scratch)
    $tempRoot=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
    if (-not $fullScratch.StartsWith($tempRoot,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($fullScratch) -notlike 'gpps-signing-tests-*') { throw 'Test cleanup path failed containment validation.' }
    Remove-Item -LiteralPath $fullScratch -Recurse -Force
}
