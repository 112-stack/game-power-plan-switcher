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

Treat hashes, signatures and build provenance as separate evidence. A matching checksum alone establishes byte equality with its reference, not publisher identity or absence of bugs. A signature is not an antivirus approval. The release is published from 112-stack/game-power-plan-switcher without an Authenticode certificate or signing attestation; see the exact release verification report for observed signing status.

Keep Windows Defender enabled. Do not add exclusions, disable scanning, disguise the executable or pack it to suppress a detection. The owner can submit a suspected false positive using Microsoft's software-developer submission path, with the precise binary and explanation. Review uploads before sending; no sample has been submitted automatically. [Microsoft submission guidance](https://learn.microsoft.com/en-us/defender-xdr/submission-guide).

See [RELEASE-SECURITY.md](RELEASE-SECURITY.md) for release preparation, optional signing and all Part 4 requirement dispositions. This is engineering guidance, not a penetration-test certificate or legal opinion.
