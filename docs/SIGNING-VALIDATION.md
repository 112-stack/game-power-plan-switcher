# Signing pipeline validation — 2026-10-07

This is a **historical development record** of the signing pipeline tested locally on top of Game Power Plan Switcher 2.3.6, before publication. That development session did not replace the published v2.3.6 executable, tag or desktop installation. The feature first ships in 2.3.7; use its separate release-verification report for checks on the final 2.3.7 binary. The counts and hashes below identify only the earlier development artifacts. No publisher certificate was available during these checks, and the 2.3.7 release also remains unsigned.

| Check | Observed result | Limit |
|---|---|---|
| Rust release build | Passed with the pinned Windows GNU-LLVM toolchain | Built locally, unsigned; not a new public release |
| Rust regression suite | 134 passed; one intentional live CPU-load probe ignored | Does not replace physical hardware, UI, or long-running recovery testing |
| PowerShell signing/build fixtures | 37 checks passed in Windows PowerShell 5.1 and PowerShell 7 | Simulated tools and certificates; no real signing, timestamp server, or Sigstore service |
| Windows download verifier | 29 checks passed against each of schema 1 and schema 2 | Inspected EXEs were never executed by these tests |
| Bash verifier | 48 contract checks passed; actual schema-1 and schema-2 artifacts passed integrity-only inspection | GNU Bash on Windows; not a native macOS/Linux test; Cosign responses were simulated |
| Cached Windows trust API | Recognized the unsigned validation build and a timestamped, already-installed signed application | Read-only embedded-signature inspection; no fresh revocation or certificate acquisition |
| Provenance | Independently matched 37 source files, four build-input files, Cargo.lock, and the EXE hash | Fingerprints record build inputs; they do not prove a reproducible or independently attested build |
| Diagnostic CLI | Valid schema 2 passed; altered source/lock hashes, schema downgrade/removal, and inconsistent unsigned metadata failed; known local legacy schema 1 passed | Only explicit diagnostics ran, with failure logs isolated from normal application data |
| Runtime scope | All 36 other files under `src`, `ui`, `native`, and `assets` remain byte-identical to the published source | No new claim about RAM, full-screen GPU use, anti-cheat certification, sensor hardware, or UI testing |

The local validation EXE has SHA-256:

```text
2213ce7fcaa72dcdc344fa127711eba9750b032d6708c569d1e5338721bbe921
```

The unchanged published v2.3.6 EXE has SHA-256:

```text
67bf874cd93f736980906c3d3d40ac9f9901b184607bee2bd3275c7c7eaf95a6
```

## Repeat the affected checks

From the project directory, using the documented pinned toolchain and a trusted PowerShell session:

```powershell
.\Build.ps1 -LlvmMingw C:\Toolchains\llvm-mingw -Action test
.\tools\Test-SigningScripts.ps1
.\tools\Test-ReleaseVerification.ps1 `
  -ExePath C:\Builds\known-local-unsigned.exe `
  -ManifestPath C:\Builds\known-local-manifest.json
```

The last command requires a known local unsigned build and its matching manifest; it creates isolated copies for negative tests and does not execute the inspected EXE. Normal build, signing, and download-verification instructions are in [RELEASE-SECURITY.md](../RELEASE-SECURITY.md) and [TRUST.md](../TRUST.md).

## Remaining publisher checks

Run a real certificate-backed build/sign/verify cycle after obtaining a legitimate OV/EV certificate and installing Microsoft's SDK signing tools. Confirm certificate selection, token/PIN interaction where applicable, timestamp trust, post-signing manifest/checksums, and offline consumer verification. Test optional Sigstore with the real intended identity and independently provisioned roots before claiming that path was exercised. No certificate, private key, signature, Microsoft approval, or warning-free SmartScreen result is supplied by this source update.
