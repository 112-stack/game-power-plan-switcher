# Security and vulnerability reporting

For a suspected Game Power Plan Switcher security issue, contact the developer through the verified [nn6 profile](https://discord.com/users/176078095957098497). For ordinary bugs, use the repository issue tracker. No dedicated security mailbox or response-time commitment has been configured. Contact availability and privacy depend on that service; avoid sending credentials or unredacted personal logs. If private vulnerability reporting is enabled on this repository, prefer its verified reporting channel.

Include the app version, SHA-256, Windows version, relevant setting, reproduction steps and expected/actual behavior. Use an isolated disposable profile and stop before any test would affect another person's computer or data. Diagnostic exports can contain account, executable and plan names; inspect them before sharing. A malware-detection report should include the exact detection name and affected file hash.

## Trust boundaries

- A per-user engine owns serialized power changes through a shared lock and recovery journal. Normal monitoring runs without elevation. A user-requested privileged power-plan file operation can offer a separate Windows approval prompt after denial.
- The app reads process identities and Windows CPU observations. Optional scheduling changes use normal Windows process APIs. It does not inject into game memory or certify anti-cheat compatibility.
- File-integrity comparison is advisory and explicit. A mutable manifest and verifier cannot authenticate themselves against an attacker controlling both. Obtain a trusted release digest or verified publisher/provenance signature independently.
- No debugger/VM detection, fake measurements, silent punitive shutdown, activation key, machine fingerprint or periodic tamper scan is part of this GPL release. Recovery is never gated behind a license or integrity decision.
- GPU/OS/runtime dependencies and external hardware-sensor providers remain separate trust inputs. The app cannot make a compromised local administrator or operating system trustworthy.

Close normally or use Exit to restore state. The guardian attempts recovery after an engine crash; simultaneous termination, system power loss, deleted plans or denied Windows APIs can delay restoration. Do not deliberately kill the guardian to test an integrity warning.

## Release handling

Treat hashes, signatures and build provenance as separate evidence. A matching checksum alone establishes byte equality with its reference, not publisher identity or absence of bugs. A signature is not an antivirus approval. The 2.3.7 release remains unsigned; adding signing scripts does not retroactively sign it. Use the exact release verification report and independently inspect the downloaded file.

## Why the binary is signed — publisher workflow

Future signed releases use Authenticode to bind the signed executable content to a certificate-backed publisher identity. The signing pipeline requires a real code-signing certificate and verifies the result before producing release metadata. No such certificate has been configured for this project yet. A self-signed test certificate is not public trust.

Authenticode does not guarantee the absence of bugs or malware, prove that a binary matches public source, or guarantee favorable antivirus/SmartScreen decisions. The manifest's `signing` fields are audit data written after signing, not evidence of a valid signature by themselves. Check the actual EXE and obtain the expected publisher identity through an independent trusted channel.

Use [tools/Verify-Download.ps1](tools/Verify-Download.ps1) for independent file inspection. It checks EXE and manifest hashes first and uses cache-only Windows trust verification by default; missing or stale revocation/chain evidence can prevent offline validation. It does not fetch current revocation state. An optional `-ExpectedSignerThumbprint` pins a separately trusted certificate. The verifier never executes the downloaded app by default. `-AllowUnsigned` reports integrity only unless a separately verified Sigstore bundle authenticates the manifest's expected signer; that does not make the EXE Authenticode-signed. `-RunManifestCheck` requires both a valid Authenticode signature and either a separately obtained matching `-ExpectedSignerThumbprint` or a verified Sigstore identity, plus that explicit request to execute the file. A file signed by an unrelated trusted publisher does not meet this execution gate.

Optional Sigstore verification requires the signed bundle, exact expected identity and issuer, and pre-provisioned trusted roots. The app itself makes no network calls for signing, identity validation or release verification. See [TRUST.md](TRUST.md) for scope and [RELEASE-SECURITY.md](RELEASE-SECURITY.md) for publisher commands.

Keep Windows Defender enabled. Do not add exclusions, disable scanning, disguise the executable or pack it to suppress a detection. The owner can submit a suspected false positive using Microsoft's software-developer submission path, with the precise binary and explanation. Review uploads before sending; no sample has been submitted automatically. [Microsoft submission guidance](https://learn.microsoft.com/en-us/defender-xdr/submission-guide).

See [RELEASE-SECURITY.md](RELEASE-SECURITY.md) for release preparation, optional signing and all Part 4 requirement dispositions. This is engineering guidance, not a penetration-test certificate or legal opinion.
