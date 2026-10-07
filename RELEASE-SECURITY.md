> Current product: Game Power Plan Switcher 2.3.7 (2026-10-07). Use the version-specific verification report attached to the 2.3.7 release for current evidence. IMPLEMENTATION.txt, RELEASE-VERIFICATION.txt and older benchmarks are retained historical records; they do not describe new checks on this binary.

# Release integrity and security

## Code Signing and SmartScreen

**The 2.3.7 executable is unsigned.** The new signing workflow is an explicit publisher step for a future release. No legitimate publisher certificate, private key or Sigstore signing identity is configured here, and no signature or Microsoft approval is implied. An unsigned local build remains supported.

Authenticode signing requires a publicly trusted code-signing certificate issued for the real publisher. OV and EV certificates work through their provider's supported Windows certificate store/KSP/CSP, including hardware-backed tokens. Follow the certificate provider's identity and key-protection requirements; do not assume a new public certificate can be exported as a PFX. A self-signed test certificate is not public trust. Never commit keys, PFX files or passwords.

A valid signature identifies the publisher; it does not guarantee a warning-free first download. SmartScreen considers file and publisher reputation, and a newly signed binary may be **unrecognized while showing a verified publisher**. This is different from **Unknown Publisher**. EV certificates no longer bypass reputation checks. Do not remove Windows protections, claim a guaranteed time-to-reputation, or describe antivirus submission as a consumer SmartScreen reputation whitelist. [Microsoft's current SmartScreen guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).

### Build, sign, verify, then package

1. Review the source and locked dependencies, run the affected checks, and record the actual toolchain. Install the Windows SDK signing tools from Microsoft. Keep the existing published release intact.
2. Select the intended code-signing certificate in `Cert:\CurrentUser\My` (or `LocalMachine` with `-CertificateStore LocalMachine`). The thumbprint selects a certificate; it is not a SHA-1 file signature.
3. Run the script below with the actual LLVM-MinGW installation and release tag. `OutputDirectory` must be new and its parent must exist. The script uses the pinned `Build.ps1` release/locked build, creates a separate copy to sign, verifies it, then writes the manifest/checksums from the **signed bytes**. Failed work stays in a clearly marked `.incomplete-*` directory; never publish that directory.

```powershell
Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
  Select-Object Subject, Thumbprint, NotAfter, HasPrivateKey

.\tools\Build-Signed-Release.ps1 `
  -LlvmMingw C:\Toolchains\llvm-mingw `
  -LlvmMingwRelease '20260922-ucrt' `
  -OutputDirectory C:\Releases\gpps-next `
  -Thumbprint 'REPLACE_WITH_REAL_CERTIFICATE_THUMBPRINT'
```

The release tag above is the project's pinned LLVM-MinGW UCRT release, not a guess about an arbitrary installation. Supply the tag of the exact toolchain you actually use. The output uses `Game-Power-Plan-Switcher-<CargoVersion>.exe`, `Game-Power-Plan-Switcher-release-manifest.json` and `Game-Power-Plan-Switcher-release-checksums.txt`. Choose and review the next application version before publication; these scripts neither bump versions nor upload or overwrite GitHub assets.

To sign a reviewed prebuilt file instead:

```powershell
.\tools\Sign-Release.ps1 -ExePath C:\Builds\Game-Power-Plan-Switcher.exe `
  -SignedExePath C:\Releases\Game-Power-Plan-Switcher.signed.exe `
  -Thumbprint 'REPLACE_WITH_REAL_CERTIFICATE_THUMBPRINT' `
  -TimestampUrl 'http://timestamp.digicert.com' `
  -Description 'Game Power Plan Switcher' `
  -DescriptionUrl 'https://github.com/112-stack/game-power-plan-switcher'
```

`Sign-Release.ps1` locates SignTool under `WindowsSdkDir`, then standard SDK paths, or accepts `-SignToolPath`. It signs a new copy, uses SHA-256 for the file and RFC 3161 timestamp digest, and treats SignTool warnings/nonzero exits as failure. It verifies every embedded signature with `verify /pa /all /v`, requires a timestamp, and reports the actual signer and certificate expiry. The original EXE remains unchanged. Use the timestamp endpoint allowed by your certificate provider. [SignTool reference](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool), [timestamping guidance](https://learn.microsoft.com/en-us/windows/win32/seccrypto/time-stamping-authenticode-signatures).

For an eligible existing PFX, pass `-PfxPath` instead of `-Thumbprint`; supply `-PfxPassword (Read-Host -AsSecureString)` or the process environment variable `NN6_SIGN_PFX_PASSWORD` from a secret manager. Do not type passwords into shell history or source files. SignTool's PFX interface temporarily exposes `/p` to local process inspection even when the wrapper never logs it; prefer certificate-store/hardware signing. The scripts do not import certificates or install trust roots.

### Optional detached Sigstore signing

Add `-UseSigstore -ExpectedIdentity <exact-identity> -ExpectedIssuer <exact-issuer> -TrustedRootPath <trusted-root.json>` to the build script only when intentionally signing the manifest. Merely installing Cosign does not enable it. Keyless signing can use network/OIDC authentication and publish the certificate identity and artifact digest in a public transparency log; it is not an offline signing action. No project identity is invented or automatically approved. [Sigstore blob signing](https://docs.sigstore.dev/cosign/signing/signing_with_blobs/).

After Authenticode, the script creates the manifest, adds the observed signer metadata, runs `--verify-release`, optionally signs and verifies the manifest bundle, and finally writes checksums. Both download verifiers use local bundles and explicitly supplied trusted roots for offline Sigstore verification; provision a verified Cosign 3+ and trusted-root JSON independently before disconnecting. Exact identity and issuer must come from an authenticated channel, never only from an untrusted bundle. Required signature checks may not be skipped because a tool or root is absent. [Cosign verification reference](https://github.com/sigstore/cosign/blob/main/doc/cosign_verify-blob.md), [trusted-root configuration](https://docs.sigstore.dev/cosign/system_config/custom_components/).

### Provenance and reproducibility limits

`cargo build` never signs or contacts signing services. It records `rustc -vV`, compiler commit, the supplied `NN6_LLVM_MINGW_VERSION`, and the SHA-256 of Cargo.lock. Missing local LLVM-MinGW identity is marked unknown rather than guessed; the signed-release script requires it explicitly. `NN6_SIGN_CERT_THUMBPRINT` only enables a release-build reminder to run the separate signing script.

Schema 2 adds `toolchain`, `source_tree_sha256`, `source_tree_algorithm` and `signing`. The source-tree algorithm, `sha256-path-length-content-v1`, hashes the domain `GamePowerPlanSwitcher/source-tree/v1\0`, followed by all regular files under `src/`, `ui/`, `native/` and `assets/`, sorted by UTF-8 relative path bytes with `/` separators. Each record contains little-endian 64-bit path length, path bytes, little-endian 64-bit file length, and the raw 32-byte content SHA-256. Symlinks/reparse points are rejected. A separate build-input fingerprint also covers the build scripts and configuration; the exact algorithm is in `build_support.rs`.

The binary initially emits `signing.signed=false` with null signer fields. Only the publisher script replaces those fields with observations after signing. The built-in verifier checks bytes and embedded provenance and accepts the legacy schema-1 format where appropriate; it does not authenticate the manifest's `signing` declaration. Independent Windows/Sigstore verification supplies that evidence. A schema-2 manifest cannot become legacy just by changing its schema number. An old schema-1-only executable cannot read a new schema-2 manifest.

This is a repeatable workflow, not a claim that timestamped signatures are byte-for-byte deterministic. Reproducibility experiments must also control toolchains, paths, environment, build timestamps and optional UUIDs, then compare the unsigned build outputs. Signing and timestamps intentionally change bytes. Publish final hashes after signing; do not modify resources or metadata inside the signed EXE afterward.

### Submit a suspected Microsoft false positive

1. Record the exact released EXE's SHA-256, detection name, Windows/security-product versions and reproducible behavior. Investigate whether the report identifies a real problem first.
2. Open [Microsoft Security Intelligence file submission](https://www.microsoft.com/en-us/wdsi/filesubmission), sign in, and select the **software developer** submission path for an incorrectly detected file.
3. Upload the exact affected file, choose the affected Microsoft security product, and describe its legitimate behavior, detection, hash and project URL. Review the upload because it sends the binary to Microsoft.
4. Keep the submission ID, follow the determination in submission history, and use the contact option in the final result if disputing it. No favorable result or permanent allowlisting is guaranteed. [Microsoft's submission procedure](https://learn.microsoft.com/en-us/defender-xdr/submission-guide).

Keep security protections enabled. Submission, signing and publication are explicit publisher actions; the app does none of them at runtime. See [TRUST.md](TRUST.md) for user-facing verification and [SECURITY.md](SECURITY.md) for reporting issues.

## Historical 2.3.3 engineering assessment

The sections below preserve earlier implementation decisions. The current workflow above supersedes their illustrative signing commands and schema-1-only description; historical hashes and measurements are not new release claims.

The final licensing decision is **GPL-3.0-or-later, freely runnable**. This release uses transparent source notices, build metadata and explicit offline artifact comparison. It does not add DRM or hide different behavior from debuggers, virtual machines or users. Documentation is not evidence that a certificate was purchased, a file signed, a public repository created or a sample submitted.

No publisher certificate, private signing key or verified public release identity was supplied for this work. The signing and publishing commands below are preparation instructions, not completed actions. Consult the version-specific verification report and actual signature status for the delivered binary. These are engineering and license-summary notes, not legal advice.

## 2.3.3 local lifecycle and sensor boundaries

The GUI singleton runs before Slint initialization on normal GUI launches only. Its Global mutex name hashes the actual user SID, Windows session, dry-run mode and isolated test directory; the explicit DACL grants access only to that user. Session identity prevents a launch in one desktop session from targeting an invisible GUI in another. Daemon, guardian and diagnostic paths do not acquire this GUI lock. Engine ownership and restoration retain their existing separate lock and journal.

The local named pipe rejects remote clients, uses overlapped bounded I/O and accepts only a fixed SHOW message. It carries no file path, settings or executable command. The duplicate verifies the server's session, requests foreground permission, waits for acceptance, and exits. Early requests coalesce until the UI handler is installed. Startup retries are bounded; failure reports an error without starting another GUI. Mutex abandonment allows a later launch to recover ownership after a crash. These objects prevent accidental duplication and cross-user access; they do not authenticate trusted software against arbitrary malicious code already running as the same user. Older versions without this protocol are not silently terminated.

FemtoVG OpenGL replaces the prior WGPU renderer. Hidden-main-window refresh deferral and dropping a disabled HUD reduce unnecessary presentation work, but are not a no-overhead guarantee. Saved Expanded/Compact preferences remain intact. GPU metrics use installed NVIDIA NVML or supported AMD ADL Overdrive8 PMLog interfaces with restricted absolute System32 DLL loading, nominally every two seconds while a view/HUD is visible. No proprietary driver is bundled or installed. CPU/fallback readings use an existing LHM/OHM WMI provider on a separate ten-second worker; Retry is explicit. No undocumented LHM shared-memory protocol or sysinfo GPU capability is assumed. Missing metrics remain unavailable, and local poll timestamps are not hardware acquisition timestamps.

The isolated singleton probe/harness tests the actual kernel ownership and pipe protocol without initializing Slint or an engine. It can terminate only the disposable owner process it created to exercise abandonment. Its result is separate from live window focus, renderer, power-plan or hardware verification; consult the final version-specific report for completed checks.

## Part 4 requirement disposition

Every requested item is accounted for here. “Implemented” describes the named source behavior; the final release report separately records tests against the packaged EXE.

| Item | Disposition and practical limit |
|---|---|
| **4.1 — Release profile, symbols, LTO and overflow** | Root release profile uses fat Rust LTO, one codegen unit, size optimization, symbol stripping, aborting panics, no debug assertions/info, no incremental build and disabled implicit overflow checks. Security-sensitive parsing still uses explicit checked bounds. This is optimization, not resistance to modification. Cargo package overrides cannot set `lto`; `/DEBUG:NONE` is not blindly passed to this GNU-style linker. Rust LTO does not imply C++ cross-language LTO. See the [Cargo profile reference](https://doc.rust-lang.org/cargo/reference/profiles.html). |
| **4.2 — Obfuscate names, GUIDs, URLs, notices and resources** | Not implemented. Public plan identifiers, profile links, copyright notices, icon references and RC metadata remain inspectable. Encoding strings would not keep secrets from someone who can inspect the executable, and hiding notices would undermine attribution. No secret should be placed in the binary. |
| **4.3 — `.text`/`.rdata` hashes, silent 4E36, periodic scan, two-pass build** | Replaced with explicit `--write-release-manifest` and `--verify-release` commands. They compare the final on-disk EXE's full SHA-256, raw `.text`/`.rdata` ranges and hashes, size, PE flags and provenance with an external schema-1 manifest. No startup enforcement, 60-second scan, silent kill or 4E36 punishment. The engine/guardian remains available for recovery. The manifest itself needs independent authentication. |
| **4.4 — Debug APIs/PEB checks and corrupted metrics** | Omitted. Debugging and instrumentation remain supported; CPU metrics are never deliberately falsified. No debugger detection, anti-analysis timing trap or telemetry sabotage was added. |
| **4.5 — VM detection and altered timing** | Omitted. VMs receive the same logic and truthful unavailable states as physical machines. No VM fingerprinting, artificial delay or altered measurement path is included. |
| **4.6 — Commercial packers/protectors and antivirus mitigation** | VMProtect, Themida, Enigma and other packer/protector integration are not recommended or included. Protection-product and “RustAegis” claims are not adopted without evidence. Use explainable behavior, reviewed source, verified signing and legitimate vendor false-positive review. EV signing is not a guarantee against SmartScreen/Defender warnings or anti-cheat action. |
| **4.7 — Signed offline HWID licenses, expiry, 4E37, issuer CLI** | Omitted by the user's final GPL choice. No key activation, expiry, MachineGuid/volume/CPU fingerprint, entitlement verification or issuer executable is required. Signing a release artifact is distinct from licensing users to run it. |
| **4.8 — Hidden PE signatures/watermarks, Sigstore/Rekor, historical hashes** | Build identity is transparent metadata, not a covert watermark or proof of authorship. No `sigpack` dependency or certificate-padding payload is added. Detached Sigstore signing and independently verified provenance are documented below but not performed. Existing historical file hashes are measured below; no invented timestamp or signature is attached to them. |
| **4.9 — SPDX, branding, README, citation and trademark** | GPL source identifiers, copyright notices, README, NOTICE, CITATION and separate trademark guidance identify NN6 accurately. GPL does not impose a blanket no-modification/no-rebranding rule or make citation mandatory. Earlier MIT permissions remain intact. Dependency/asset rights remain separate. |
| **4.10 — Silent exit/nag, opt-in logs, three-event cooldown** | No punitive exit, nag cycle or counter-based cooldown. Explicit verification returns success/failure and a readable error through the normal diagnostic error path. Existing operational/error logging remains for power restoration and troubleshooting; a new “opt-in only” rule does not erase recovery evidence. No periodic verification means no new tamper-event telemetry. |
| **4.11 — Offline, no driver/elevation/encryption/aggressive VM checks** | Integrity verification is offline and read-only; no kernel driver, whole-binary encryption or VM detection is added. Normal monitoring stays `asInvoker`. Existing user-requested privileged `.pow` operations can separately offer Windows approval after a permission failure; this is not hidden elevation by the integrity feature. Optional signing/provenance publication uses network services only when the publisher deliberately runs it. |
| **4.12 — Requested protection dependencies** | The integrity feature uses the SHA-256/encoding dependencies `sha2` and `hex`. Other operational dependencies, including the GPU sensor wrapper introduced in 2.3.2 and retained in 2.3.3, are independently recorded in the lockfile/notices. `obfstr`, `obfuse`, `licenz`, `astraguard`, `memmap2` and `sigpack` are not added merely because they appeared in a proposed list; their requested versions/features are not certified by this release. No unsupported “package = product protection” claim is made. The exact build dependency graph is in Cargo.lock and the notices. |
| **4.13 — Self-hash build, commit/time stamp, optional SignTool** | Build metadata records available version/authorship/commit/dirty-state/build-time/UUID information. Unknown source identity is labeled unknown, not guessed. No circular whole-file self-hash or rebuild-to-fixed-point claim. Build once, optionally sign/timestamp, then produce the final external manifest/checksums. Signing changes file bytes, so regenerate post-signing artifacts. |
| **4.14 — Manifest, CFG, ASLR and DEP** | The Windows manifest requests `asInvoker`, per-monitor DPI awareness and long-path awareness. Release inspection reports dynamic-base, high-entropy-VA, NX and Guard-CF header bits. A set header bit alone is not evidence of complete CFG instrumentation across Rust, C++ and every dependency; do not claim verified CFG coverage without the compiler/linker and binary evidence. ASLR/DEP flags are mitigation observations, not a security certification. |

## What an integrity check proves

An external manifest avoids embedding a hash of data that changes when that same hash is written. Even hashing `.rdata` becomes circular if the expected digest is embedded there. A two-pass rebuild also changes metadata and code layout; it is not a general solution to that dependency. A deliberate excluded slot could support another design, but this implementation uses an external manifest instead.

An attacker who can replace both EXE and manifest can make a new matching pair. A modified verifier can lie. Therefore first compare a downloaded file with an independently authenticated reference using a trusted external tool; do not launch an untrusted EXE just to ask it whether it is trustworthy. The built-in command is useful for a known local build and release regression checks. Section digests are for identifying changes, not proof of a publisher or a tamper-proof in-memory process.

Authenticode is not a plain whole-file SHA-256: PE signing excludes specific checksum/certificate-related fields. Appending arbitrary “watermark” data to a signed image is not an authenticated authorship mechanism. Keep the full-file checksum alongside signature verification and do not modify a signed release. [Microsoft's executable-signing explanation](https://learn.microsoft.com/en-us/windows/win32/secbp/understanding-pe-signatures).

## Local manifest commands

Run these in the directory containing a **trusted** build. This GUI-subsystem executable is launched with `Start-Process -Wait` so PowerShell can reliably inspect its exit code.

```powershell
$releaseExe = (Resolve-Path -LiteralPath '.\Game-Power-Plan-Switcher-2.3.7.exe').Path
$releaseManifest = Join-Path (Split-Path $releaseExe) 'NN6-release-manifest.json'
$job = Start-Process -FilePath $releaseExe -ArgumentList @(
    '--write-release-manifest', ('"' + $releaseManifest + '"')
) -WindowStyle Hidden -Wait -PassThru
if ($job.ExitCode -ne 0) { throw 'Release manifest creation failed.' }

$job = Start-Process -FilePath $releaseExe -ArgumentList @(
    '--verify-release', ('"' + $releaseManifest + '"')
) -WindowStyle Hidden -Wait -PassThru
if ($job.ExitCode -ne 0) { throw 'Release verification failed.' }
Get-FileHash -LiteralPath $releaseExe -Algorithm SHA256
Get-AuthenticodeSignature -LiteralPath $releaseExe |
    Select-Object Status, StatusMessage, SignerCertificate
```

The verifier compares its own current executable against the supplied manifest. It does not start monitoring or select a power plan. Exit code 0 is a comparison success; 1 is failure, with the normal startup-error diagnostic recorded under the app data directory. A manifest freshly generated from a modified binary proves only that binary's current contents. Keep the published reference immutable and separately authenticated.

## Publisher workflow and signing

1. Review source/dependency notices and final license choice. Run required tests and record the exact toolchain, source commit/dirty state and build inputs. Keep private keys outside the source tree and archive.
2. Build and inspect the final unsigned EXE. Optional signing occurs now. Do not post-process resources or executable bytes afterward.
3. Generate `NN6-release-manifest.json` and checksums from the final signed-or-unsigned artifact. Run the comparison and record signature status separately.
4. Create the matching source/archive artifacts and their digests. Do not put a hash of an archive inside the same archive unless you have an explicitly excluded representation. Detached checksum/manifest files avoid that cycle.
5. Optionally sign the manifest/archive digest with a real trusted identity, then publish all corresponding artifacts only after release authorization. Hashes do not replace source delivery, and none of these steps establishes absence of vulnerabilities.

### Authenticode: requires an actual publisher certificate

Install Microsoft's Windows SDK signing tools from their official distribution. Import/select the publisher's legitimate code-signing certificate using its provider's instructions, preferably with a hardware-backed key. Do not put a PFX password into scripts, shell history or source control. Replace the placeholder with the intended code-signing certificate's thumbprint; `/sha1` selects that certificate and is not the file digest algorithm.

```powershell
Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
    Select-Object Subject, Thumbprint, NotAfter, HasPrivateKey
$publisherThumbprint = 'REPLACE_WITH_REAL_CERTIFICATE_THUMBPRINT'
signtool sign /sha1 $publisherThumbprint /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 .\Game-Power-Plan-Switcher-2.3.7.exe
if ($LASTEXITCODE -ne 0) { throw 'Signing failed.' }
signtool verify /pa /all /v .\Game-Power-Plan-Switcher-2.3.7.exe
if ($LASTEXITCODE -ne 0) { throw 'Authenticode verification failed.' }
```

Use the RFC 3161 timestamp service permitted by your certificate provider; the URL shown is Microsoft's documented example, not a claim that NN6 has an account/certificate there. Verify the intended publisher identity as well as the trust result. A self-signed test certificate is not public publisher trust. [Microsoft SignTool reference](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool).

### Detached Sigstore/Cosign signatures

Acquire Cosign from its official verified distribution. The following commands are examples to be run intentionally by the publisher: keyless signing authenticates an identity and can create a public transparency-log record. It is not an offline app activation system. No such signing or log publication has been performed by these instructions. [Cosign installation](https://docs.sigstore.dev/cosign/system_config/installation/) and [blob signing](https://docs.sigstore.dev/cosign/signing/signing_with_blobs/).

```powershell
cosign sign-blob .\NN6-release-manifest.json --bundle .\NN6-release-manifest.sigstore.json
if ($LASTEXITCODE -ne 0) { throw 'Manifest signing failed.' }

$expectedIdentity = 'REPLACE_WITH_INDEPENDENTLY_PUBLISHED_SIGNER_IDENTITY'
$expectedIssuer = 'REPLACE_WITH_THE_EXPECTED_OIDC_ISSUER_URL'
cosign verify-blob .\NN6-release-manifest.json --bundle .\NN6-release-manifest.sigstore.json --certificate-identity $expectedIdentity --certificate-oidc-issuer $expectedIssuer
if ($LASTEXITCODE -ne 0) { throw 'Manifest signer verification failed.' }
```

Consumers must obtain the expected identity/issuer through a trusted channel; do not accept values solely from the untrusted download. For an established public-key workflow, use `cosign verify-blob ... --bundle ... --key <trusted-public-key>` instead. A bundle carries signature verification material; air-gapped verification also needs a vetted Cosign version and pre-provisioned trusted roots. Do not bypass transparency/certificate checks with insecure-ignore flags. [Cosign blob verification](https://github.com/sigstore/cosign/blob/main/doc/cosign_verify-blob.md).

### GitHub build provenance, if a repository is established

No repository URL or publisher workflow identity is invented here. Once an authorized repository builds the artifact, its workflow can generate an attestation after the final artifact exists:

```yaml
permissions:
  contents: read
  id-token: write
  attestations: write
# A reviewed build/signing step must run before this step.
steps:
  - name: Attest final release artifact
    uses: actions/attest@v4
    with:
      subject-path: 'dist/Game-Power-Plan-Switcher-2.3.7.exe'
```

This is a fragment, not an installed publishing workflow. Pin the action to a reviewed full commit SHA in production. Verify a published artifact with `gh attestation verify .\Game-Power-Plan-Switcher-2.3.7.exe -R OWNER/REPOSITORY`, substituting the independently trusted repository. Provenance identifies a build context; it does not audit the source or replace Authenticode. [GitHub artifact-attestation guidance](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations).

## False-positive review and external actions

This engineering risk assessment describes behaviors that **may** complicate review or trigger a heuristic/policy warning; it does not claim that any product will detect a particular binary. Microsoft's published criteria specifically discuss evasive behavior, and its developer guidance favors understandable, signed software. [Detection criteria](https://learn.microsoft.com/en-us/unified-secops/criteria), [software-developer guidance](https://learn.microsoft.com/en-us/defender-xdr/developer-faq).

| Technique or property | Possible concern | Release approach |
|---|---|---|
| Packer/protector or executable encryption | Concealed code and loader behavior may increase scrutiny. | Ship the normal reviewed native binary; remove unnecessary protectors. |
| Broad string/control-flow obfuscation | Can hide intent and complicate false-positive analysis. | Retain readable notices, public identifiers and explainable behavior. |
| Self-modification or frequent self-scanning | Runtime writes/scans can resemble protection or evasion behavior and add overhead. | Use explicit read-only file comparison, never executable self-modification. |
| Anti-debug/anti-VM branches | Different behavior under analysis can look evasive and harms legitimate users. | Use the same behavior on physical machines, VMs and debug sessions. |
| HWID collection/activation | Device identifiers create privacy, support and trust questions. | No HWID or activation mechanism in this GPL release. |
| Process/power/scheduling APIs | Legitimate administration can still be restricted by enterprise or game policy. | Use only documented scope, show errors, retain reversible changes and explain process tuning. |
| UAC/elevated helper | An unexpected prompt can alarm users or exceed the intended scope. | Separate explicit retry for the selected privileged file operation; monitoring stays unelevated. |
| New/unsigned executable or new signing identity | Reputation may be absent even when behavior is benign. | Verify exact bytes and signer, retain publisher consistency, submit a precise disputed sample if necessary. |
| Hidden PE/certificate watermark | Custom post-signing bytes can undermine clear verification semantics. | Publish transparent build metadata and detached manifests; do not alter a signed artifact. |
| EV certificate | Identity vetting is not a cleanliness, reputation or game-compatibility guarantee. | Make no EV/SmartScreen/Defender bypass promise; follow actual vendor determinations. |

If a precise shipped file is flagged, retain its SHA-256, detection name, engine/signature version and reproducible behavior. Review it for an actual defect first. The developer can submit that exact file through [Microsoft Security Intelligence](https://www.microsoft.com/en-us/wdsi/filesubmission) as an incorrectly detected software-developer sample, then retain the submission ID and result. The portal requires sign-in; no submission or guaranteed favorable determination is claimed. [Microsoft's submission procedure](https://learn.microsoft.com/en-us/defender-xdr/submission-guide).

Do not disable antivirus, add exclusions, strip readable evidence, encrypt the executable or install a packer to evade a detection. Review any upload because it sends the binary outside the local machine. Public releases, signing-service authentication and transparency publication require an authorized target/identity; this document alone does not perform or authorize those actions.

## Historical local hashes actually measured

The following SHA-256 values were computed on October 4, 2026 from the retained local files. They are historical byte identifiers, not third-party attestations, earlier publication evidence or the current 2.3.3 digest. A verifier should recompute them from the actual retained files.

| Retained artifact | SHA-256 |
|---|---|
| `outputs/previous-2.1-review/NN6-PowerPlan-Native.exe` | `F54C44678CBBAD10B3568147C7615B7DC40A9DC9C6AFF655E39AF91910CF812A` |
| `outputs/previous-2.2-review/NN6-PowerPlan-Native.exe` | `D75EAF8D993A5DBAFCCD5A38A0ABEA1A627B0FEED9E97FA76378740689C5D2F0` |
| `outputs/previous-2.3-review/NN6-PowerPlan-Native.exe` | `4C42E1329148691C3655428F6410E3DD052AD4A3DF0ED7EA317569DC2014FDB0` |

Current-release digests belong in the final generated manifest/checksum report after all build/signing steps. No 2.3.3 digest is guessed in this guide.

## Optional Windows icon refresh

The new release is `Game-Power-Plan-Switcher-2.3.7.exe`. Older review artifacts may use `NN6-PowerPlan-Native.exe`; their presence does not establish which version is currently running. Check a shortcut's target and IconLocation before assuming a stale image belongs to the new file. To attempt a non-destructive shell icon refresh on Windows where the utility exists:

```powershell
& "$env:SystemRoot\System32\ie4uinit.exe" -show
```

Treat this as an optional refresh attempt, not a guaranteed full icon-cache rebuild or proof of embedded icon correctness. It is not a request to delete cache files, terminate Explorer, stop the game or close the running monitor. Inspect the new file/shortcut again afterward; the package's resource verification is separate from Windows shell caching.

## Licensing and attribution boundaries

### Dependency and toolchain source inventory

[DEPENDENCY-SOURCES.json](DEPENDENCY-SOURCES.json) records the supplied Cargo.lock SHA-256, locked registry packages, exact versioned crate-archive URLs and lockfile checksums. Refer to its generated counts for this release's hashed cached archives and indexed but missing source packages. Cache presence is evidence of available source bytes, not proof that every cached crate was linked into the final executable. Preserve the matching source/build scripts, notices and dependency archive alongside the binary when distributing this release.

The companion dependency-source ZIP covers those cached Windows-build source archives. It is not a configured `cargo vendor` tree, a complete cross-platform offline dependency closure or a claim of reproducible binary output. The source index also identifies Rust 1.99.0's compiler source by full commit and records dated official rust-src, rustc and rust-std component URLs and SHA-256 values from the installed rustup manifest. `rust-src` contains standard-library source; full Rust compiler source and pinned submodules may require separate retrieval. Archive hashes identify bytes; obtain the index and toolchain provenance through an independently trusted channel before relying on them.

NN6 code in this release is GPL-3.0-or-later; the complete unchanged GPL text is in LICENSE.txt. Preserve notices, identify changes and provide the corresponding source as the license requires when conveying covered binaries. It does not forbid a GPL-compliant fork, modification, commercial redistribution or rebranding. Optional citation metadata and truthful-identity guidance add no usage restriction. Prior MIT grants are preserved rather than withdrawn. [GNU license FAQ](https://www.gnu.org/licenses/gpl-faq.html).

The pinned Slint 1.18.1 packages explicitly offer GPL-3.0-only as an alternative, and this release selects it. The combined distribution is conveyed under GPL version 3; NN6's own source grant remains GPL-3.0-or-later. AboutSlint stays available. The upstream Royalty-Free 2.0 alternative is not relied on for GPL compatibility in this release; its text remains in the upstream license inventory. Third-party dependencies and artwork remain separately licensed. Review shipped notices and ownership before public distribution; these notes are not a legal certification. [Slint's upstream license choices](https://github.com/slint-ui/slint/blob/master/LICENSE.md).
