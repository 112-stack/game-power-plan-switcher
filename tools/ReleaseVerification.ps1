#requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
# Shared, read-only verification helpers. No downloads, certificate imports or
# execution of the inspected binary. WinVerifyTrust is explicitly cache-only.
Set-StrictMode -Version Latest

function Resolve-ReleaseFile {
    param([Parameter(Mandatory=$true)][string]$Path)
    $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($item.PSIsContainer -or $item.PSProvider.Name -ne 'FileSystem') { throw "Expected a local file: $Path" }
    return $item.FullName
}

function Read-ReleaseBytes {
    param([string]$Path, [long]$Limit = 1MB)
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        if ($stream.Length -gt $Limit) { throw 'Verification metadata exceeds the 1 MiB inspection limit.' }
        $buffer = New-Object byte[] ([int]$stream.Length)
        $offset = 0
        while ($offset -lt $buffer.Length) {
            $read = $stream.Read($buffer, $offset, $buffer.Length - $offset)
            if ($read -eq 0) { throw 'Verification metadata changed while reading.' }
            $offset += $read
        }
        return ,$buffer
    } finally { $stream.Dispose() }
}

function Read-ReleaseChecksums {
    param([string]$Path)
    $file = Resolve-ReleaseFile $Path
    $utf8 = New-Object System.Text.UTF8Encoding($false, $true)
    $contents = $utf8.GetString((Read-ReleaseBytes $file)).TrimStart([char]0xFEFF)
    $result = @{}
    foreach ($line in ($contents -split '\r?\n')) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        if ($line -notmatch '^(?<digest>[A-Fa-f0-9]{64}) [ *](?<name>.+)$') { throw 'Malformed checksum line; expected SHA-256 and exact filename.' }
        $name = $Matches.name
        if ($name -ne [IO.Path]::GetFileName($name) -or $name.Contains(':') -or $name.Contains('/') -or $name.Contains('\') -or $name.Trim() -ne $name -or $name -match '[\x00-\x1F]') {
            throw 'Checksum names must be simple filenames, without directory traversal or control characters.'
        }
        if ($result.ContainsKey($name)) { throw "Duplicate checksum filename: $name" }
        $result[$name] = $Matches.digest.ToLowerInvariant()
    }
    if ($result.Count -eq 0) { throw 'Checksum file is empty.' }
    return $result
}

function Assert-ReleaseHash {
    param([string]$Path, [hashtable]$Checksums)
    $name = [IO.Path]::GetFileName($Path)
    if (-not $Checksums.ContainsKey($name)) { throw "No checksum for exact filename: $name" }
    $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -cne $Checksums[$name]) { throw "SHA-256 mismatch: $name" }
    return $hash
}

function Read-ReleaseManifest {
    param([string]$Path)
    $raw = Read-ReleaseBytes $Path
    # The framework JSON reader also exposes duplicate object members. Reject
    # them before ConvertFrom-Json can silently replace a preceding value.
    Add-Type -AssemblyName System.Runtime.Serialization
    $quotas = New-Object System.Xml.XmlDictionaryReaderQuotas
    $quotas.MaxDepth = 32
    $quotas.MaxStringContentLength = 1MB
    $quotas.MaxArrayLength = 1MB
    $quotas.MaxNameTableCharCount = 1MB
    $reader = [System.Runtime.Serialization.Json.JsonReaderWriterFactory]::CreateJsonReader($raw, $quotas)
    try {
        $xml = New-Object System.Xml.XmlDocument
        $xml.XmlResolver = $null
        $xml.Load($reader)
        if ($xml.DocumentElement.GetAttribute('type') -ne 'object') { throw 'Manifest root must be a JSON object.' }
        foreach ($object in $xml.SelectNodes('//*[@type="object"]')) {
            $names = @{}
            foreach ($child in $object.ChildNodes) {
                $key = $child.LocalName
                if ($child.HasAttribute('item')) { $key = $child.GetAttribute('item') }
                if ($names.ContainsKey($key)) { throw "Duplicate JSON property: $key" }
                $names[$key] = $true
            }
        }
    } finally { $reader.Dispose() }
    $utf8 = New-Object System.Text.UTF8Encoding($false, $true)
    $document = $utf8.GetString($raw).TrimStart([char]0xFEFF) | ConvertFrom-Json -ErrorAction Stop
    if (($document.schema -isnot [int] -and $document.schema -isnot [long]) -or $document.schema -notin @(1, 2)) { throw 'Unsupported manifest schema; expected integer 1 or 2.' }
    if ($document.product -isnot [string] -or $document.product -notin @('Game Power Plan Switcher', 'NN6 Power Plan Native')) { throw 'Unexpected manifest product.' }
    if ($document.sha256 -isnot [string] -or $document.sha256 -cnotmatch '^[a-f0-9]{64}$' -or ($document.bytes -isnot [int] -and $document.bytes -isnot [long]) -or $document.bytes -le 0) { throw 'Invalid manifest size or SHA-256.' }
    if ($document.schema -eq 2) {
        if ($document.source_tree_algorithm -isnot [string] -or $document.source_tree_algorithm -cne 'sha256-path-length-content-v1' -or $document.source_tree_sha256 -isnot [string] -or $document.source_tree_sha256 -cnotmatch '^[a-f0-9]{64}$') { throw 'Invalid source fingerprint fields.' }
        if (($document.provenance.manifest_schema -isnot [int] -and $document.provenance.manifest_schema -isnot [long]) -or $document.provenance.manifest_schema -ne 2) { throw 'Schema 2 provenance marker is required.' }
        foreach ($key in 'cargo_lock_sha256','build_inputs_sha256') {
            if ($document.toolchain.$key -isnot [string] -or $document.toolchain.$key -cnotmatch '^[a-f0-9]{64}$') { throw "Invalid toolchain field: $key" }
        }
        foreach ($key in 'rustc','rustc_commit_hash','llvm_mingw') {
            if ($document.toolchain.$key -isnot [string] -or [string]::IsNullOrWhiteSpace($document.toolchain.$key)) { throw "Invalid toolchain field: $key" }
        }
        if ($document.signing.signed -isnot [bool]) { throw 'Signing status must be a JSON boolean.' }
        if ($document.signing.signed) {
            if ($document.signing.signer_subject -isnot [string] -or [string]::IsNullOrWhiteSpace($document.signing.signer_subject) -or $document.signing.signer_thumbprint -isnot [string] -or $document.signing.signer_thumbprint -notmatch '^[a-fA-F0-9]{40}$' -or $document.signing.not_after -isnot [string] -or [string]::IsNullOrWhiteSpace($document.signing.not_after)) { throw 'Incomplete signer metadata.' }
        } else {
            foreach ($key in 'signer_subject','signer_thumbprint','not_after') {
                if ($null -ne $document.signing.$key) { throw 'Unsigned metadata must not claim a signing certificate.' }
            }
        }
    } elseif ($document.PSObject.Properties.Name -contains 'signing' -or $document.PSObject.Properties.Name -contains 'toolchain' -or $document.PSObject.Properties.Name -contains 'source_tree_sha256' -or $document.PSObject.Properties.Name -contains 'source_tree_algorithm' -or ($document.PSObject.Properties.Name -contains 'provenance' -and $document.provenance.PSObject.Properties.Name -contains 'manifest_schema')) {
        throw 'Schema 2 fields cannot be relabeled as schema 1.'
    }
    return $document
}

function Initialize-OfflineAuthenticode {
    if ('GamePowerPlan.TrustResult' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Security.Cryptography.X509Certificates;
namespace GamePowerPlan {
    public sealed class TrustResult {
        public string Status;
        public string ErrorCode;
        public string Subject;
        public string Thumbprint;
        public string NotAfter;
        public bool TimestampPresent;
    }
    public static class OfflineAuthenticode {
        [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
        struct FileInfo { public uint Size; public string Path; public IntPtr Handle; public IntPtr Subject; }
        [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
        struct TrustData {
            public uint Size; public IntPtr Policy; public IntPtr Sip;
            public uint UI; public uint Revocation; public uint UnionChoice;
            public IntPtr File; public uint StateAction; public IntPtr State;
            public IntPtr URL; public uint Flags; public uint Context; public IntPtr SignatureSettings;
        }
        [StructLayout(LayoutKind.Sequential)]
        struct Signer {
            public uint Size; public System.Runtime.InteropServices.ComTypes.FILETIME AsOf;
            public uint CertCount; public IntPtr CertChain; public uint SignerType;
            public IntPtr Info; public uint Error; public uint CounterCount;
            public IntPtr CounterSigners; public IntPtr ChainContext;
        }
        [StructLayout(LayoutKind.Sequential)]
        struct ProviderCert { public uint Size; public IntPtr Certificate; }
        [DllImport("wintrust.dll", ExactSpelling=true, CharSet=CharSet.Unicode)]
        static extern int WinVerifyTrust(IntPtr hwnd, ref Guid action, ref TrustData data);
        [DllImport("wintrust.dll", ExactSpelling=true)]
        static extern IntPtr WTHelperProvDataFromStateData(IntPtr state);
        [DllImport("wintrust.dll", ExactSpelling=true)]
        static extern IntPtr WTHelperGetProvSignerFromChain(IntPtr data, uint index, [MarshalAs(UnmanagedType.Bool)] bool counter, uint counterIndex);
        [DllImport("wintrust.dll", ExactSpelling=true)]
        static extern IntPtr WTHelperGetProvCertFromChain(IntPtr signer, uint index);
        public static TrustResult Check(string path) {
            var action = new Guid("00AAC56B-CD44-11d0-8CC2-00C04FC295EE");
            var file = new FileInfo { Size=(uint)Marshal.SizeOf(typeof(FileInfo)), Path=path };
            IntPtr buffer = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(FileInfo)));
            Marshal.StructureToPtr(file, buffer, false);
            var data = new TrustData { Size=(uint)Marshal.SizeOf(typeof(TrustData)), UI=2,
                Revocation=0, UnionChoice=1, File=buffer, StateAction=1,
                // WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT.
                // No fresh online revocation or root retrieval. Missing cache fails closed.
                Flags=0x1000 | 0x80 };
            var result = new TrustResult();
            try {
                int status = WinVerifyTrust(new IntPtr(-1), ref action, ref data);
                uint code = unchecked((uint)status);
                result.ErrorCode = "0x" + code.ToString("X8");
                result.Status = status == 0 ? "Valid" : code == 0x800B0100 ? "NotSigned" :
                    code == 0x80096010 ? "HashMismatch" : "UnknownError";
                IntPtr provider = WTHelperProvDataFromStateData(data.State);
                if (provider != IntPtr.Zero) {
                    IntPtr signerPtr = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
                    if (signerPtr != IntPtr.Zero) {
                        var signer = (Signer)Marshal.PtrToStructure(signerPtr, typeof(Signer));
                        for (uint i = 0; i < Math.Min(signer.CounterCount, 16u); i++) {
                            IntPtr counterPtr = WTHelperGetProvSignerFromChain(provider, 0, true, i);
                            if (counterPtr != IntPtr.Zero) {
                                var counter = (Signer)Marshal.PtrToStructure(counterPtr, typeof(Signer));
                                if (counter.Error == 0 && counter.CertCount > 0 && (counter.SignerType & 0x10) != 0)
                                    result.TimestampPresent = true;
                            }
                        }
                        IntPtr certPtr = WTHelperGetProvCertFromChain(signerPtr, 0);
                        if (certPtr != IntPtr.Zero) {
                            var nativeCert = (ProviderCert)Marshal.PtrToStructure(certPtr, typeof(ProviderCert));
                            using (var cert = new X509Certificate2(nativeCert.Certificate)) {
                                result.Subject = cert.Subject;
                                result.Thumbprint = cert.Thumbprint;
                                result.NotAfter = cert.NotAfter.ToUniversalTime().ToString("o");
                            }
                        }
                    }
                }
                return result;
            } finally {
                data.StateAction = 2;
                WinVerifyTrust(new IntPtr(-1), ref action, ref data);
                Marshal.DestroyStructure(buffer, typeof(FileInfo));
                Marshal.FreeHGlobal(buffer);
            }
        }
    }
}
'@
}

function Get-OfflineAuthenticode {
    param([string]$Path)
    Initialize-OfflineAuthenticode
    return [GamePowerPlan.OfflineAuthenticode]::Check($Path)
}
