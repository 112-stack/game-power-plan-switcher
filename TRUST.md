# Trust your download

**Current status:** the Game Power Plan Switcher v2.3.7 executable is unsigned. This source adds a signing pipeline for future releases; no publisher certificate or Sigstore release identity has been configured. Do not treat documentation, a badge or the manifest's signer fields as proof of signing.

```text
Reviewed source + pinned tools
        │ build (no certificate required)
        ▼
Unsigned EXE ── explicit publisher signing + RFC 3161 timestamp ──► Signed EXE
        │                                                        │
        └── final manifest + SHA-256 checksums ◄──────────────────┘
                         │ optional explicit Sigstore signature
                         ▼
              Publish EXE, source and trust artifacts
                         │ independent trusted reference
                         ▼
            Verify hashes → publisher → optional diagnostics
```

| Artifact/check | What it establishes | What it does not establish |
|---|---|---|
| EXE and manifest SHA-256 | These files match the supplied reference bytes. | Who supplied that reference, or whether the code is safe. |
| Authenticode + timestamp | Windows trust policy accepts signed content and certificate/time evidence available to the verifier. | Source correspondence, no vulnerabilities, antivirus approval or current online revocation status during offline checking. |
| Sigstore manifest bundle | The manifest was signed by the exact expected identity/issuer under independently trusted roots, with verifiable log evidence. | Safety of the source or Authenticode validity. |
| Schema-2 toolchain/source digests | The build records compiler identity, lockfile hash and a defined source-tree digest. | A third-party build attestation or demonstrated bit-for-bit reproducibility. |
| Version-specific verification report | Tests and limitations recorded for that exact EXE hash. | Tests on every CPU, driver, game or Windows policy. |

## Verify without running the download

Obtain the script itself from trusted source. On Windows:

```powershell
.\tools\Verify-Download.ps1 -ExePath .\Game-Power-Plan-Switcher-2.3.7.exe `
  -ManifestPath .\Game-Power-Plan-Switcher-2.3.7-release-manifest.json `
  -ChecksumPath .\Game-Power-Plan-Switcher-2.3.7-checksums.txt -AllowUnsigned
```

`-AllowUnsigned` is appropriate for the current unsigned release or a deliberate local build. Without a separately verified Sigstore bundle, success means **INTEGRITY-ONLY**, not publisher authentication. If a bundle verifies against the independently trusted expected identity, the result reports integrity and Sigstore identity verification while clearly retaining the EXE's Authenticode-unsigned status. Omit this switch for Authenticode-signed releases; a required check that fails or lacks evidence returns nonzero. Default Windows checks use local trust/revocation caches without fetching new evidence. `-OnlineAuthenticodeReport` explicitly permits the additional PowerShell signature report to consult Windows trust services. `-ExpectedSignerThumbprint` can pin a certificate obtained independently. `-RunManifestCheck` explicitly runs the EXE's internal diagnostic only with a valid Authenticode signature **plus** a matching, independently obtained `-ExpectedSignerThumbprint` or verified Sigstore identity. It never runs an unsigned file or executes a file merely because some trusted publisher signed it.

On macOS/Linux, Python 3 and `sha256sum` or `shasum` are required:

```bash
bash tools/Verify-Download.sh --exe ./Game-Power-Plan-Switcher-2.3.7.exe \
  --manifest ./Game-Power-Plan-Switcher-2.3.7-release-manifest.json \
  --checksums ./Game-Power-Plan-Switcher-2.3.7-checksums.txt --allow-unsigned
```

Bash checks schema-1/2 EXE hash and size without executing Windows code. Publisher verification there requires a Sigstore bundle; Authenticode is a separate Windows or `osslsigncode verify -in <exe>` check. `osslsigncode` can require independently trusted publisher and timestamp CA bundles (`-CAfile`, `-TSA-CAfile`); its policy is not identical to Windows.

## Optional Sigstore verification

Both scripts detect a bundle named after the manifest, replacing `.json` with `.sigstore.json`. A present or explicitly supplied bundle must verify; missing Cosign or trust inputs is a failure, never a silent skip. Supply **Cosign 3+**, exact expected identity/issuer, and a local trusted-root JSON obtained independently of the download. PowerShell parameters are `-SigstoreBundle`, `-ExpectedIdentity`, `-ExpectedIssuer`, `-TrustedRootPath`; Bash equivalents are `--sigstore-bundle`, `--expected-identity`, `--expected-issuer`, `--trusted-root`. No identity has been invented for this project.

The scripts use local bundle/root material. Provision verified tools and roots before going offline; offline results cannot promise fresh revocation or trust-policy updates. Never derive the expected publisher solely from the file you are trying to authenticate. See the [publisher workflow](RELEASE-SECURITY.md#code-signing-and-smartscreen) and [security reporting](SECURITY.md). Report trust issues with the exact file hash to [nn6](https://discord.com/users/176078095957098497); do not send private keys or credentials.
