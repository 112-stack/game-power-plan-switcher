#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (c) 2026 NN6. See LICENSE.txt and NOTICE.txt.
"""Check translation contracts and literal UI message coverage (Python stdlib).

Run: python tools/Check-Translations.py --self-test
The optional self-tests run before checking the current repository. This is a
static literal-call check, not a Rust/Slint parser or a linguistic certification.
Dynamic messages, OS plan names, and runtime diagnostics are intentionally not
inferred. Test-only Rust items are excluded from the coverage scan.
"""

from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import re
import sys
from typing import Any


LANGUAGES = ("en", "ar", "es", "pt-BR", "fr", "de", "ru", "zh-CN")
SOURCE_FILES = (
    "ui/app.slint", "src/ui.rs", "src/ui_cpu.rs", "src/ui_pro.rs",
    "src/pro_view.rs", "src/localization.rs",
)
PLACEHOLDER = re.compile(r"\{(?:0|[1-9][0-9]*)\}")
RUST_CALL = re.compile(
    r"(?<![\w:.])(?:(?:crate::)?localization::(?:tr|format)|trf|tr)\s*\("
)
SLINT_CALL = re.compile(r"@tr\s*\(")


class CatalogError(ValueError):
    """A malformed catalog or message contract."""


def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise CatalogError(f"duplicate JSON key: {key!r}")
        result[key] = value
    return result


def parse_catalog(raw: bytes) -> dict[str, str]:
    if raw.startswith(b"\xef\xbb\xbf"):
        raise CatalogError("use UTF-8 without a BOM")
    try:
        data = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise CatalogError(str(error)) from error
    if not isinstance(data, dict) or not data:
        raise CatalogError("catalog must be a nonempty JSON object")
    for key, value in data.items():
        if not isinstance(key, str) or not key.strip():
            raise CatalogError("message keys must be nonempty strings")
        if not isinstance(value, str) or not value.strip():
            raise CatalogError(f"translation must be a nonempty string: {key!r}")
    return data


def placeholders(message: str) -> Counter[str]:
    # This project's interpolation syntax accepts indexed arguments only.
    # Detect accidental {name}, unmatched braces, {{0}}, and changed indices.
    remaining = PLACEHOLDER.sub("", message)
    if "{" in remaining or "}" in remaining:
        raise CatalogError(f"invalid placeholder/braces in {message!r}")
    return Counter(PLACEHOLDER.findall(message))


def validate_catalog(
    catalog: dict[str, str], english: dict[str, str], language: str
) -> list[str]:
    errors: list[str] = []
    missing = english.keys() - catalog.keys()
    extra = catalog.keys() - english.keys()
    for key in sorted(missing):
        errors.append(f"{language}: missing key {key!r}")
    for key in sorted(extra):
        errors.append(f"{language}: unknown key {key!r}")
    for key, value in catalog.items():
        if language == "en" and key != value:
            errors.append(f"en: value must equal its English key: {key!r}")
        try:
            if placeholders(key) != placeholders(value):
                errors.append(f"{language}: placeholder multiset differs: {key!r}")
        except CatalogError as error:
            errors.append(f"{language}: {error}")
    return errors


def blank(text: str) -> str:
    """Preserve newlines and offsets while excluding text from code matching."""
    return "".join("\n" if character == "\n" else " " for character in text)


def literal_end(source: str, start: int) -> tuple[int, str] | None:
    """Return a normal/raw string's end and decoded contents, if present."""
    raw = re.match(r'r(#{0,255})"', source[start:])
    if raw:
        content_start = start + raw.end()
        terminator = '"' + raw.group(1)
        end = source.find(terminator, content_start)
        if end < 0:
            raise CatalogError("unterminated raw string literal")
        return end + len(terminator), source[content_start:end]
    if start >= len(source) or source[start] != '"':
        return None
    index = start + 1
    output: list[str] = []
    escapes = {"n": "\n", "r": "\r", "t": "\t", "0": "\0", '"': '"', "\\": "\\", "'": "'"}
    while index < len(source):
        character = source[index]
        if character == '"':
            return index + 1, "".join(output)
        if character != "\\":
            output.append(character)
            index += 1
            continue
        index += 1
        if index >= len(source):
            break
        escape = source[index]
        if escape in escapes:
            output.append(escapes[escape])
            index += 1
        elif escape in "\r\n":
            # Rust source continuation discards the newline and next indent.
            while index < len(source) and source[index].isspace():
                index += 1
        elif escape == "u":
            match = re.match(r"u\{([0-9a-fA-F_]+)\}", source[index:])
            if not match:
                raise CatalogError("invalid Unicode string escape")
            output.append(chr(int(match.group(1).replace("_", ""), 16)))
            index += match.end()
        elif escape == "x" and re.match(r"x[0-9a-fA-F]{2}", source[index:]):
            output.append(chr(int(source[index + 1:index + 3], 16)))
            index += 3
        else:
            raise CatalogError(f"unsupported string escape: \\{escape}")
    raise CatalogError("unterminated string literal")


def code_mask(source: str) -> str:
    """Mask comments/literals, including nested Rust block comments."""
    chunks: list[str] = []
    index = 0
    while index < len(source):
        start = index
        if source.startswith("//", index):
            end = source.find("\n", index)
            index = len(source) if end < 0 else end
        elif source.startswith("/*", index):
            index += 2
            depth = 1
            while index < len(source) and depth:
                if source.startswith("/*", index):
                    depth += 1
                    index += 2
                elif source.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
        else:
            # Avoid treating a Rust lifetime ('a) as a character literal.
            char = re.match(r"'(?:\\u\{[0-9a-fA-F_]+\}|\\x[0-9a-fA-F]{2}|\\.|[^'\\\n])'", source[index:])
            literal = literal_end(source, index) if source[index] == '"' or re.match(r'r#*"', source[index:]) else None
            if char:
                index += char.end()
            elif literal:
                index = literal[0]
            else:
                chunks.append(source[index])
                index += 1
                continue
        chunks.append(blank(source[start:index]))
    return "".join(chunks)


def exclude_test_items(mask: str) -> str:
    """Exclude #[cfg(test)] items and #[test] functions without truncating files."""
    attribute = re.compile(r"#\s*\[\s*(?:cfg\s*\(\s*test\s*\)|test)\s*\]")
    spans: list[tuple[int, int]] = []
    for match in attribute.finditer(mask):
        if any(start <= match.start() < end for start, end in spans):
            continue
        item = re.search(r"[;{]", mask[match.end():])
        if not item:
            raise CatalogError("test attribute has no following item")
        start = match.end() + item.start()
        end = start + 1
        if mask[start] == "{":
            depth = 1
            while end < len(mask) and depth:
                depth += (mask[end] == "{") - (mask[end] == "}")
                end += 1
            if depth:
                raise CatalogError("unbalanced test item")
        spans.append((match.start(), end))
    for start, end in reversed(spans):
        mask = mask[:start] + blank(mask[start:end]) + mask[end:]
    return mask


def literal_calls(source: str, slint: bool = False) -> list[tuple[int, str]]:
    mask = code_mask(source)
    if not slint:
        mask = exclude_test_items(mask)
    result: list[tuple[int, str]] = []
    for match in (SLINT_CALL if slint else RUST_CALL).finditer(mask):
        index = match.end()
        # Skip whitespace, not arbitrary expressions or dynamic arguments.
        while index < len(source) and source[index].isspace():
            index += 1
        literal = literal_end(source, index)
        if literal:
            result.append((source.count("\n", 0, index) + 1, literal[1]))
    return result


def self_test() -> int:
    count = 0

    def check(condition: bool, name: str) -> None:
        nonlocal count
        if not condition:
            raise AssertionError(f"self-test failed: {name}")
        count += 1

    def rejects(call: Any, name: str) -> None:
        try:
            call()
        except (CatalogError, ValueError):
            check(True, name)
        else:
            check(False, name)

    sample = {"State {0}": "State {0}", "Go": "Go"}
    check(parse_catalog(json.dumps(sample).encode()) == sample, "valid catalog")
    for raw, label in (
        (b'{"Go":"One","Go":"Two"}', "duplicate JSON key"),
        (b'{"Go":""}', "empty translation"),
        (b'{"Go":"   "}', "whitespace translation"),
        (b'{"Go":false}', "non-string translation"),
        (b'[]', "non-object root"),
        (b'{}', "empty root"),
        (b'\xef\xbb\xbf{"Go":"Go"}', "BOM"),
        (b'{"Go":"\xff"}', "invalid UTF-8"),
    ):
        rejects(lambda raw=raw: parse_catalog(raw), label)
    check(bool(validate_catalog({"Go": "Aller"}, sample, "fr")), "missing key")
    check(bool(validate_catalog(sample | {"Extra": "Extra"}, sample, "fr")), "extra key")
    check(bool(validate_catalog({"State {0}": "État", "Go": "Aller"}, sample, "fr")), "placeholder lost")
    check(bool(validate_catalog({"State {0}": "État {1}", "Go": "Aller"}, sample, "fr")), "placeholder changed")
    check(bool(validate_catalog({"State {0}": "État {0} {0}", "Go": "Aller"}, sample, "fr")), "placeholder duplicated")
    check(bool(validate_catalog({"State {0}": "State {0}", "Go": "Different"}, sample, "en")), "English source changed")
    check(placeholders("{1} {0} {1}") == placeholders("{1} {1} {0}"), "reordering allowed")
    for text in ("{0", "0}", "{name}", "{{0}}", "{01}"):
        rejects(lambda text=text: placeholders(text), "invalid braces " + text)
    rust = '''// tr("comment")
/* tr("block") /* tr("nested") */ */
fn live() { tr("Go"); trf("State {0}", &["x"]); }
#[cfg(test)] fn fixture() { tr("test fixture"); }
fn later() { localization::tr("Later"); crate::localization::format("Line\\n{1}", &[]); }
#[cfg(test)] mod tests { fn nested() { tr("nested fixture"); } }
fn last() { let unrelated = "tr(\\\"string\\\")"; tr(r#"Raw {0}"#); tr(dynamic); }
'''
    check([value for _, value in literal_calls(rust)] == ["Go", "State {0}", "Later", "Line\n{1}", "Raw {0}"], "runtime literal coverage excludes tests/comments/dynamic arguments")
    check(literal_calls('Text { text: @tr("Go"); } // @tr("comment")', True) == [(1, "Go")], "Slint literal coverage")
    check(literal_end('"\\u{4e2d}\\x41"', 0)[1] == "中A", "Unicode and hex literal decoding")
    check(literal_calls('fn live() { tr("Go"); }\n#[test] fn test() { tr("fixture"); }') == [(1, "Go")], "standalone test function excluded")
    return count


def validate_project(root: Path) -> tuple[list[str], dict[str, int]]:
    errors: list[str] = []
    folder = root / "ui" / "translations"
    expected = {language + ".json" for language in LANGUAGES}
    observed = {path.name for path in folder.glob("*.json")}
    for name in sorted(expected - observed):
        errors.append(f"missing catalog file: ui/translations/{name}")
    for name in sorted(observed - expected):
        errors.append(f"unexpected catalog file: ui/translations/{name}")
    catalogs: dict[str, dict[str, str]] = {}
    for language in LANGUAGES:
        try:
            catalogs[language] = parse_catalog((folder / (language + ".json")).read_bytes())
        except (OSError, CatalogError) as error:
            errors.append(f"{language}: {error}")
    english = catalogs.get("en", {})
    if english:
        for language, catalog in catalogs.items():
            errors.extend(validate_catalog(catalog, english, language))
    used: set[str] = set()
    total_calls = 0
    for relative in SOURCE_FILES:
        try:
            source = (root / relative).read_text(encoding="utf-8")
            calls = literal_calls(source, relative.endswith(".slint"))
            for line, message in calls:
                total_calls += 1
                used.add(message)
                if message not in english:
                    errors.append(f"{relative}:{line}: literal message absent from English catalog: {message!r}")
        except (OSError, UnicodeError, CatalogError) as error:
            errors.append(f"{relative}: {error}")
    return errors, {
        "catalogs": len(catalogs), "messages": len(english),
        "literal_calls": total_calls, "unique_used_messages": len(used),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1], help="project source root")
    parser.add_argument("--self-test", action="store_true", help="run negative/positive fixtures before repository checks")
    args = parser.parse_args()
    try:
        if args.self_test:
            print(f"PASS: {self_test()} validator self-tests")
        errors, summary = validate_project(args.root.resolve())
    except (AssertionError, CatalogError, ValueError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        print(f"FAIL: {len(errors)} translation contract/coverage issue(s)", file=sys.stderr)
        return 1
    print(f"PASS: {summary['catalogs']} catalogs, {summary['messages']} messages each; exact keys, UTF-8, nonempty values and placeholder multisets")
    print(f"PASS: {summary['literal_calls']} runtime literal calls, {summary['unique_used_messages']} distinct messages covered")
    print("Scope: static literals only; dynamic strings and linguistic/layout quality need separate review.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
