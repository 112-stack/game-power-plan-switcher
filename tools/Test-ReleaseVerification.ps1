#requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
<# Offline regression checks. ExePath must be a known local unsigned build.
   The inspected executable is NEVER run. Only isolated copies are modified.
   A process-local execution policy permits the trusted verifier test script;
   no user/machine policy, certificate store or Windows protection is changed. #>
param(
    [Parameter(Mandatory=$true)][string]$ExePath,
    [Parameter(Mandatory=$true)][string]$ManifestPath,
    [string]$ReportPath
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'ReleaseVerification.ps1')
$original = Resolve-ReleaseFile $ExePath
$sourceManifest = Resolve-ReleaseFile $ManifestPath
$sourceHash = (Get-FileHash -LiteralPath $original -Algorithm SHA256).Hash
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('gpps-verification-' + [guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testRoot
$exe = Join-Path $testRoot 'utility with spaces.exe'
$manifest = Join-Path $testRoot 'release manifest.json'
$checksums = Join-Path $testRoot 'checksums.txt'
$utf8 = New-Object Text.UTF8Encoding($false)
$rows = New-Object 'System.Collections.Generic.List[object]'
function Save-Checksums {
    $lines = foreach ($path in @($exe,$manifest)) { '{0}  {1}' -f (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant(),[IO.Path]::GetFileName($path) }
    [IO.File]::WriteAllText($checksums, (($lines -join "`r`n") + "`r`n"), $utf8)
}
function Check-Command {
    param([string]$Name,[bool]$ShouldPass,[string[]]$Additional=@(),[string]$Pattern)
    $shell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $log = @(& $shell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'Verify-Download.ps1') -ExePath $exe -ManifestPath $manifest -ChecksumPath $checksums @Additional 2>&1)
    $exitCode = $LASTEXITCODE
    $text = $log -join "`n"
    if (($exitCode -eq 0) -ne $ShouldPass -or ($Pattern -and $text -notmatch $Pattern)) { throw "Case $Name failed (exit $exitCode): $text" }
    $rows.Add([pscustomobject]@{name=$Name;passed=$true;exit=$exitCode})
}
function Check-Reject {
    param([string]$Name,[scriptblock]$Action)
    $rejected = $false
    try { $null = & $Action } catch { $rejected = $true }
    if (-not $rejected) { throw "Case $Name was unexpectedly accepted." }
    $rows.Add([pscustomobject]@{name=$Name;passed=$true})
}
try {
    [IO.File]::Copy($original,$exe,$false)
    [IO.File]::Copy($sourceManifest,$manifest,$false)
    Save-Checksums
    Check-Command 'unsigned default fails' $false @() 'Authenticode trust failed'
    Check-Command 'explicit integrity-only with spaces and CRLF' $true @('-AllowUnsigned') 'PASS \(INTEGRITY ONLY\)'
    Check-Command 'never execute unsigned' $false @('-AllowUnsigned','-RunManifestCheck') 'Refusing to execute'
    Check-Command 'unsigned cannot match a publisher pin' $false @('-AllowUnsigned','-ExpectedSignerThumbprint',('a' * 40)) 'signer pin'
    Check-Command 'missing Sigstore bundle is not skipped' $false @('-AllowUnsigned','-ExpectedIdentity','publisher','-ExpectedIssuer','issuer') 'no bundle was found'
    $bytes = [IO.File]::ReadAllBytes($exe)
    $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 1
    [IO.File]::WriteAllBytes($exe,$bytes)
    Check-Command 'one changed EXE byte fails' $false @('-AllowUnsigned') 'SHA-256 mismatch'
    [IO.File]::Copy($original,$exe,$true)
    [IO.File]::AppendAllText($manifest,"`n",$utf8)
    Check-Command 'changed manifest bytes fail before parsing' $false @('-AllowUnsigned') 'SHA-256 mismatch'
    [IO.File]::Copy($sourceManifest,$manifest,$true)
    Save-Checksums
    [IO.File]::AppendAllText($checksums, ('a' * 64) + '  utility with spaces.exe' + "`n",$utf8)
    Check-Command 'duplicate checksum cannot shadow a match' $false @('-AllowUnsigned') 'Duplicate checksum'
    [IO.File]::WriteAllText($checksums,('a' * 64) + '  ../utility.exe' + "`n",$utf8)
    Check-Command 'checksum traversal rejected' $false @('-AllowUnsigned') 'simple filenames'
    [IO.File]::WriteAllText($checksums,('a' * 63) + '  utility with spaces.exe' + "`n",$utf8)
    Check-Command 'malformed checksum rejected' $false @('-AllowUnsigned') 'Malformed checksum'
    foreach ($jsonCase in @(
        @('duplicate JSON fields','{"schema":1,"schema":1}'),
        @('root array','[]'),
        @('string schema','{"schema":"1"}'),
        @('fractional schema','{"schema":1.5}'),
        @('unsupported schema','{"schema":99}'),
        @('string byte size','{"schema":1,"product":"Game Power Plan Switcher","sha256":"' + ('a'*64) + '","bytes":"5"}'),
        @('fractional byte size','{"schema":1,"product":"Game Power Plan Switcher","sha256":"' + ('a'*64) + '","bytes":1.5}')
    )) {
        [IO.File]::WriteAllText($manifest,$jsonCase[1],$utf8)
        Check-Reject $jsonCase[0] { Read-ReleaseManifest $manifest }
    }
    $schema2 = @{
        schema=2; product='Game Power Plan Switcher'; bytes=5; sha256=('a'*64)
        source_tree_algorithm='sha256-path-length-content-v1'; source_tree_sha256=('b'*64)
        provenance=@{manifest_schema=2}
        toolchain=@{cargo_lock_sha256=('c'*64);build_inputs_sha256=('d'*64);rustc='rustc fixture';rustc_commit_hash='unknown';llvm_mingw='unknown'}
        signing=@{signed=$false;signer_subject=$null;signer_thumbprint=$null;not_after=$null}
    }
    $validFixture = $schema2 | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText($manifest,$validFixture,$utf8)
    $null = Read-ReleaseManifest $manifest
    $rows.Add([pscustomobject]@{name='schema 2 structural fixture accepted';passed=$true})
    foreach ($case in @(
        @('array source digest', { param($d) $d.source_tree_sha256 = @('b'*64) }),
        @('array toolchain digest', { param($d) $d.toolchain.cargo_lock_sha256 = @('c'*64) }),
        @('array compiler identity', { param($d) $d.toolchain.rustc = @('rustc fixture') }),
        @('string schema provenance marker', { param($d) $d.provenance.manifest_schema = '2' }),
        @('missing schema provenance marker', { param($d) $d.provenance.PSObject.Properties.Remove('manifest_schema') }),
        @('schema downgrade fields', { param($d) $d.schema = 1 }),
        @('schema downgrade marker', { param($d) $d.schema = 1; foreach ($p in 'signing','toolchain','source_tree_sha256','source_tree_algorithm') { $d.PSObject.Properties.Remove($p) } })
    )) {
        $fixture = $validFixture | ConvertFrom-Json
        & $case[1] $fixture
        [IO.File]::WriteAllText($manifest,($fixture | ConvertTo-Json -Depth 8),$utf8)
        Check-Reject $case[0] { Read-ReleaseManifest $manifest }
    }
    [IO.File]::WriteAllText($manifest, ('{"x":' * 40) + '0' + ('}' * 40),$utf8)
    Check-Reject 'bounded JSON nesting depth' { Read-ReleaseManifest $manifest }
    [IO.File]::WriteAllText($manifest, ('x' * (1MB + 1)),$utf8)
    Check-Reject 'bounded manifest read' { Read-ReleaseManifest $manifest }
    [IO.File]::WriteAllText($checksums, ('x' * (1MB + 1)),$utf8)
    Check-Reject 'bounded checksum read' { Read-ReleaseChecksums $checksums }
    [IO.File]::Copy($sourceManifest,$manifest,$true)
    Save-Checksums
    $bundle = [IO.Path]::ChangeExtension($manifest,'.sigstore.json')
    [IO.File]::WriteAllText($bundle,'{}',$utf8)
    Check-Command 'discovered bundle requires independent identity and root' $false @('-AllowUnsigned') 'requires ExpectedIdentity'
    if ((Get-FileHash -LiteralPath $original -Algorithm SHA256).Hash -ne $sourceHash) { throw 'Original executable changed.' }
    $result = [pscustomobject]@{passed=$true;checks=$rows.Count;inspected_executable_executed=$false;source_unchanged=$true;cases=@($rows.ToArray())}
    if ($ReportPath) { [IO.File]::WriteAllText([IO.Path]::GetFullPath($ReportPath),($result | ConvertTo-Json -Depth 8),$utf8) }
    Write-Host "PASS: $($rows.Count) offline verifier checks; inspected executable never run."
} finally {
    # Only exact direct child files created in this unique disposable directory.
    foreach ($file in @(Get-ChildItem -LiteralPath $testRoot -File)) {
        if ($file.Directory.FullName -ne $testRoot) { throw 'Test cleanup path escaped its root.' }
        Remove-Item -LiteralPath $file.FullName -Force
    }
    [IO.Directory]::Delete($testRoot,$false)
}
