#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
# Offline file inspection for macOS/Linux. NEVER starts the Windows executable.
set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'HELP'
Usage: bash tools/Verify-Download.sh --exe FILE --manifest FILE --checksums FILE
       [--sigstore-bundle FILE --expected-identity ID --expected-issuer URL
        --trusted-root FILE] [--allow-unsigned]

Requires Python 3 plus sha256sum or shasum. Every supplied/discovered Sigstore
bundle must verify using an independently trusted exact identity, issuer and
local TrustedRoot JSON with Cosign 3+. No downloads or executable launch occur.
Without a verified Sigstore bundle, use --allow-unsigned ONLY for an explicitly
labelled INTEGRITY-ONLY result. It is not publisher authentication.
HELP
}
exe='' manifest='' checksums='' bundle='' identity='' issuer='' trusted_root=''
allow_unsigned=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --exe|--manifest|--checksums|--sigstore-bundle|--expected-identity|--expected-issuer|--trusted-root)
            [[ $# -ge 2 && -n "$2" ]] || fail "Missing value for $1"
            case "$1" in
                --exe) exe=$2 ;; --manifest) manifest=$2 ;; --checksums) checksums=$2 ;;
                --sigstore-bundle) bundle=$2 ;; --expected-identity) identity=$2 ;;
                --expected-issuer) issuer=$2 ;; --trusted-root) trusted_root=$2 ;;
            esac
            shift 2 ;;
        --allow-unsigned) allow_unsigned=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) fail "Unknown argument: $1" ;;
    esac
done
[[ -n "$exe" && -n "$manifest" && -n "$checksums" ]] || { usage >&2; fail 'Three artifact paths are required.'; }
for path in "$exe" "$manifest" "$checksums"; do
    [[ -f "$path" && -r "$path" ]] || fail "Not a readable regular file: $path"
done
command -v python3 >/dev/null 2>&1 || fail 'Python 3 is required for strict JSON and checksum parsing.'
if command -v sha256sum >/dev/null 2>&1; then
    sha_file() { sha256sum < "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    sha_file() { shasum -a 256 < "$1" | cut -d ' ' -f 1; }
else
    fail 'Install sha256sum or shasum from a trusted source.'
fi

exe_hash=$(sha_file "$exe")
manifest_hash=$(sha_file "$manifest")
# Parse data as data. Exact base names, unique records and bounded input avoid
# ambiguous checksum matches; whitespace within a filename is preserved.
python3 - "$exe" "$manifest" "$checksums" "$exe_hash" "$manifest_hash" <<'PY' || fail 'Artifact integrity or manifest validation failed.'
import datetime, hashlib, json, os, re, sys, unicodedata
exe, manifest, checksums, exe_hash, manifest_hash = sys.argv[1:]
def need(condition, message):
    if not condition:
        raise ValueError(message)
def bounded(path):
    with open(path, 'rb') as stream:
        raw = stream.read(1024 * 1024 + 1)
    need(len(raw) <= 1024 * 1024, 'Metadata exceeds 1 MiB inspection limit')
    return raw.decode('utf-8-sig')
def unique_object(pairs):
    obj = {}
    for key, value in pairs:
        need(key not in obj, 'Duplicate JSON property: ' + key)
        obj[key] = value
    return obj
def exact_keys(obj, keys, label):
    need(isinstance(obj, dict) and set(obj) == set(keys), 'Invalid or missing fields in ' + label)
def sha256_text(value):
    return isinstance(value, str) and re.fullmatch(r'[0-9a-fA-F]{64}', value) is not None
def bounded_depth(value):
    # Bound nested untrusted structures even in extensible provenance fields.
    pending = [(value, 1)]
    while pending:
        node, depth = pending.pop()
        need(depth <= 32, 'Manifest exceeds 32 levels of nesting')
        if isinstance(node, dict):
            pending.extend((child, depth + 1) for child in node.values())
        elif isinstance(node, list):
            pending.extend((child, depth + 1) for child in node)
try:
    names = [os.path.basename(exe), os.path.basename(manifest)]
    need(names[0].casefold() != names[1].casefold(), 'Artifact base names must differ')
    entries, folded = {}, set()
    for number, line in enumerate(bounded(checksums).splitlines(), 1):
        if not line:
            continue
        match = re.fullmatch(r'([0-9a-fA-F]{64}) [ *](.+)', line)
        need(match is not None, f'Malformed checksum record at line {number}')
        digest, name = match.groups()
        need('/' not in name and '\\' not in name and not any(ord(c) < 32 for c in name), 'Checksums must use plain file base names')
        need(name.casefold() not in folded, 'Duplicate checksum filename: ' + name)
        folded.add(name.casefold())
        entries[name] = digest.lower()
    for name, actual in zip(names, [exe_hash, manifest_hash]):
        need(entries.get(name) == actual.lower(), 'Missing or mismatched SHA-256: ' + name)
        print('PASS: checksum ' + name)
    value = json.loads(bounded(manifest), object_pairs_hook=unique_object,
                       parse_constant=lambda value: (_ for _ in ()).throw(ValueError('Non-finite JSON number')))
    bounded_depth(value)
    need(isinstance(value, dict), 'Manifest must be an object')
    need(type(value.get('schema')) is int and value['schema'] in (1, 2), 'Only manifest schema 1 and 2 are supported')
    common = ('schema', 'product', 'version', 'bytes', 'sha256', 'sections', 'flags', 'provenance')
    modern = ('toolchain', 'source_tree_sha256', 'source_tree_algorithm', 'signing')
    exact_keys(value, common + (modern if value['schema'] == 2 else ()), 'manifest')
    need(isinstance(value.get('sha256'), str) and value['sha256'].lower() == exe_hash.lower(), 'Manifest EXE SHA-256 mismatch')
    need(type(value.get('bytes')) is int and 0 < value['bytes'] <= 512 * 1024 * 1024 and value['bytes'] == os.path.getsize(exe), 'Manifest EXE size mismatch or exceeds 512 MiB')
    need(value.get('product') in ('Game Power Plan Switcher', 'NN6 Power Plan Native'), 'Unexpected manifest product')
    need(isinstance(value.get('version'), str) and bool(value['version']), 'Missing manifest version')
    need(isinstance(value['provenance'], dict), 'Manifest provenance must be an object')
    exact_keys(value['flags'], ('dynamic_base', 'high_entropy_va', 'nx_compatible', 'guard_cf_header'), 'PE flags')
    need(all(type(flag) is bool for flag in value['flags'].values()), 'PE flags must be booleans')
    need(isinstance(value['sections'], list) and len(value['sections']) == 2, 'Expected text and rdata section records')
    seen = set()
    ranges = []
    with open(exe, 'rb') as stream:
        for section in value['sections']:
            exact_keys(section, ('name', 'raw_offset', 'raw_size', 'sha256'), 'section digest')
            name, start, size = section['name'], section['raw_offset'], section['raw_size']
            need(isinstance(name, str) and name in ('.text', '.rdata') and name not in seen, 'Invalid or duplicate section name')
            need(type(start) is int and type(size) is int and start >= 0 and size > 0 and start + size <= value['bytes'], 'Invalid section range')
            need(not any(start < old_end and old_start < start + size for old_start, old_end in ranges), 'Overlapping section ranges')
            need(sha256_text(section['sha256']), 'Malformed section SHA-256')
            stream.seek(start)
            remaining, digest = size, hashlib.sha256()
            while remaining:
                chunk = stream.read(min(remaining, 65536))
                need(bool(chunk), 'Truncated section')
                digest.update(chunk)
                remaining -= len(chunk)
            need(digest.hexdigest() == section['sha256'].lower(), 'Section SHA-256 mismatch')
            seen.add(name)
            ranges.append((start, start + size))
    if value['schema'] == 1:
        need('manifest_schema' not in value['provenance'], 'Legacy schema 1 cannot contain a manifest_schema marker')
    else:
        need(type(value['provenance'].get('manifest_schema')) is int and value['provenance']['manifest_schema'] == 2, 'Schema 2 requires the integer manifest_schema: 2 marker')
        tc = value.get('toolchain')
        exact_keys(tc, ('rustc', 'rustc_commit_hash', 'llvm_mingw', 'cargo_lock_sha256', 'build_inputs_sha256'), 'toolchain')
        for key in ('rustc', 'rustc_commit_hash', 'llvm_mingw'):
            need(isinstance(tc.get(key), str) and bool(tc[key].strip()), 'Missing toolchain field: ' + key)
        need(len(tc['rustc'].encode('utf-8')) <= 16384, 'Rust compiler identity is too long')
        need(re.fullmatch(r'[A-Za-z0-9._+-]{1,128}', tc['llvm_mingw']) is not None, 'Invalid LLVM-MinGW release tag')
        need(tc['rustc_commit_hash'] == 'unknown' or re.fullmatch(r'(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})', tc['rustc_commit_hash']) is not None, 'Invalid Rust compiler commit')
        for key in ('cargo_lock_sha256', 'build_inputs_sha256'):
            need(sha256_text(tc[key]), 'Invalid toolchain hash: ' + key)
        need(('commit-hash: ' + tc['rustc_commit_hash']) in tc['rustc'].splitlines(), 'Rust compiler commit fields disagree')
        need(sha256_text(value['source_tree_sha256']), 'Invalid source-tree hash')
        need(value.get('source_tree_algorithm') == 'sha256-path-length-content-v1', 'Unknown source-tree hash algorithm')
        signing = value.get('signing')
        exact_keys(signing, ('signed', 'signer_subject', 'signer_thumbprint', 'not_after'), 'signing audit metadata')
        need(type(signing['signed']) is bool, 'Signing state must be boolean')
        if signing['signed']:
            subject = signing['signer_subject']
            need(isinstance(subject, str) and bool(subject.strip()) and len(subject.encode('utf-8')) <= 4096 and not any(unicodedata.category(c) == 'Cc' for c in subject), 'Invalid signer subject')
            need(isinstance(signing['signer_thumbprint'], str) and re.fullmatch(r'[0-9a-fA-F]{40}', signing['signer_thumbprint']) is not None, 'Invalid signer thumbprint')
            expiry = signing['not_after']
            need(isinstance(expiry, str) and re.fullmatch(r'[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,9})?(?:Z|\+00:00)', expiry) is not None, 'Certificate expiry must use UTC RFC3339')
            datetime.datetime.fromisoformat(expiry.replace('Z', '+00:00'))
        else:
            need(all(signing[key] is None for key in ('signer_subject', 'signer_thumbprint', 'not_after')), 'Unsigned metadata cannot claim a signer')
    print(f"PASS: schema {value['schema']} manifest EXE SHA-256 and size")
except (OSError, UnicodeError, ValueError, TypeError, RecursionError) as error:
    print('FAIL: ' + str(error), file=sys.stderr)
    sys.exit(1)
PY

[[ "$(sha_file "$manifest")" == "$manifest_hash" && "$(sha_file "$exe")" == "$exe_hash" ]] || fail 'An artifact changed during integrity verification.'

# A bundle beside the manifest is never silently ignored. Missing verification
# material is a failure, even if the caller selected integrity-only fallback.
if [[ -z "$bundle" && -f "${manifest%.*}.sigstore.json" ]]; then
    bundle="${manifest%.*}.sigstore.json"
fi
if [[ -z "$bundle" && ( -n "$identity" || -n "$issuer" || -n "$trusted_root" ) ]]; then
    fail 'Sigstore identity/issuer/root supplied without a bundle.'
fi
if [[ -n "$bundle" ]]; then
    [[ -f "$bundle" && -f "$trusted_root" && -n "$identity" && -n "$issuer" ]] || fail 'A bundle requires ExpectedIdentity, ExpectedIssuer and an independently trusted local TrustedRoot file.'
    command -v cosign >/dev/null 2>&1 || fail 'Cosign is required for the supplied/discovered bundle.'
    version=$(cosign version 2>&1) || fail 'Cannot inspect Cosign version.'
    [[ "$version" =~ GitVersion:[[:space:]]*v([0-9]+)\. ]] || fail 'Cannot determine Cosign major version.'
    [[ "${BASH_REMATCH[1]}" -ge 3 ]] || fail 'Cosign 3 or later is required for this bundle workflow.'
    help=$(cosign verify-blob --help 2>&1) || fail 'Cannot inspect Cosign verification options.'
    [[ "$help" == *'--trusted-root'* ]] || fail 'Cosign does not support local trusted roots.'
    args=(verify-blob --bundle "$bundle" --certificate-identity "$identity"
          --certificate-oidc-issuer "$issuer" --trusted-root "$trusted_root")
    # New Cosign bundle verification uses local material; older supported builds
    # can additionally expose --offline. Do not pass removed flags blindly.
    if [[ "$help" == *'--offline'* ]]; then args+=(--offline); fi
    cosign "${args[@]}" -- "$manifest" || fail 'Sigstore signature or publisher identity did not verify.'
    [[ "$(sha_file "$manifest")" == "$manifest_hash" && "$(sha_file "$exe")" == "$exe_hash" ]] || fail 'An artifact changed during verification.'
    printf '%s\n' 'PASS: integrity and Sigstore publisher identity verified.'
    printf '%s\n' 'The signed manifest binds these EXE bytes to the expected identity; it does not prove malware-free behavior or a reproducible build.'
elif [[ "$allow_unsigned" -eq 1 ]]; then
    printf '%s\n' 'INTEGRITY-ONLY: checksums and manifest match. Publisher identity was NOT verified.'
    printf '%s\n' 'A replaced EXE, manifest and checksum file can agree. Authenticate the reference through a separate trusted channel.'
else
    fail 'Publisher verification unavailable: supply a Sigstore bundle and trusted identity/root, or explicitly request --allow-unsigned for INTEGRITY-ONLY inspection.'
fi
printf '%s\n' 'Authenticode was NOT checked. Use tools/Verify-Download.ps1 on Windows, or a trusted osslsigncode installation:'
printf '  osslsigncode verify -in %q\n' "$exe"
printf '%s\n' 'osslsigncode may need -CAfile and -TSA-CAfile with trusted publisher/timestamp roots; its policy is not identical to Windows. No Windows executable was run.'
