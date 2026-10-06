#requires -Version 5.1
<# x64 Windows GNU-LLVM build. See README.txt for prerequisites.
   Dependencies are pinned in Cargo.lock. No PS2EXE or PowerShell runtime is
   needed to run the resulting native executable.

   Public provenance is generated in OUT_DIR/build_info.rs. Set a fixed
   SOURCE_DATE_EPOCH, NN6_BUILD_UUID and NN6_GIT_COMMIT (or parameters below)
   to reproduce those metadata inputs. This is not a guarantee of byte-for-byte
   reproducibility across different toolchains, paths or dependency versions.

   Release LTO/panic/incremental settings belong in [profile.release], not
   [profile.release.package."*"], where Cargo does not support those keys.
   Rust fat LTO does not include the separately compiled C++ bridge. Build.rs
   requests GNU-lld --dynamicbase, --nxcompat and --high-entropy-va, and omits
   the PE timestamp. CFG is not enabled: whole-program Rust/std/C++ coverage
   is not established. Stripping symbols is neither encryption nor DRM.
   No packer, anti-debugging, VM detection or security-product evasion is used. #>
param(
    [Parameter(Mandatory=$true)][string]$LlvmMingw,
    [ValidateSet('build','test','check')][string]$Action='build',
    [switch]$DebugBuild,
    [ValidatePattern('^[0-9]+$')][string]$SourceDateEpoch,
    [ValidatePattern('^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$')][string]$BuildUuid,
    [ValidatePattern('^(unknown|[0-9A-Fa-f]{40}|[0-9A-Fa-f]{64})$')][string]$GitCommit
)
$ErrorActionPreference='Stop'
if($PSBoundParameters.ContainsKey('SourceDateEpoch')){$env:SOURCE_DATE_EPOCH=$SourceDateEpoch}
if($PSBoundParameters.ContainsKey('BuildUuid')){$env:NN6_BUILD_UUID=$BuildUuid.ToLowerInvariant()}
if($PSBoundParameters.ContainsKey('GitCommit')){$env:NN6_GIT_COMMIT=$GitCommit.ToLowerInvariant()}
$bin=Join-Path ([IO.Path]::GetFullPath($LlvmMingw)) 'bin'
foreach($file in 'x86_64-w64-mingw32-clang.exe','x86_64-w64-mingw32-clang++.exe','x86_64-w64-mingw32-windres.exe','llvm-ar.exe'){
    if(-not(Test-Path -LiteralPath (Join-Path $bin $file))){throw "Missing LLVM-MinGW tool: $file"}
}
$env:PATH="$bin;$env:PATH"
$env:CC=Join-Path $bin 'x86_64-w64-mingw32-clang.exe'
$env:CXX=Join-Path $bin 'x86_64-w64-mingw32-clang++.exe'
$env:AR=Join-Path $bin 'llvm-ar.exe'
$env:NN6_WINDRES=Join-Path $bin 'x86_64-w64-mingw32-windres.exe'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=$env:CC
$env:RUSTFLAGS='-C target-feature=+crt-static -C symbol-mangling-version=v0'
if(-not $DebugBuild){$env:CARGO_INCREMENTAL='0'}
$env:CARGO_TARGET_DIR=Join-Path $PSScriptRoot 'target'
$buildArguments=@('+1.99.0-x86_64-pc-windows-gnullvm',$Action,'--locked','--target','x86_64-pc-windows-gnullvm','--manifest-path',(Join-Path $PSScriptRoot 'Cargo.toml'))
if(-not $DebugBuild){$buildArguments+='--release'}
& cargo @buildArguments
if($LASTEXITCODE -ne 0){throw "Cargo failed with exit code $LASTEXITCODE"}
if($Action -eq 'build'){
    $profile=if($DebugBuild){'debug'}else{'release'}
    Write-Output (Join-Path $env:CARGO_TARGET_DIR "x86_64-pc-windows-gnullvm\$profile\Game-Power-Plan-Switcher.exe")
}
