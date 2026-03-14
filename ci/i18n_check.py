#!/usr/bin/env python3
import json
import os
import re
import sys
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
I18N_DIR = ROOT / os.environ.get("I18N_DIR", "i18n")
LOCALES_PATH = ROOT / os.environ.get("LOCALES_PATH", str(Path("i18n") / "locales.json"))
EN_PATH = ROOT / os.environ.get("EN_PATH", str(Path("i18n") / "en.json"))

PLACEHOLDER_RE = re.compile(r"\{[A-Za-z0-9_.-]+\}")
BACKTICK_RE = re.compile(r"`[^`]*`")


def load_json(path: Path):
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def placeholders(text: str):
    return PLACEHOLDER_RE.findall(text)


def same_placeholders(lhs: str, rhs: str):
    return Counter(placeholders(lhs)) == Counter(placeholders(rhs))


def backticks(text: str):
    return BACKTICK_RE.findall(text)


def literal_backticks(text: str):
    spans = []
    for span in backticks(text):
        inner = span[1:-1]
        if PLACEHOLDER_RE.fullmatch(inner):
            continue
        spans.append(span)
    return spans


def same_literal_backticks(lhs: str, rhs: str):
    return Counter(literal_backticks(lhs)) == Counter(literal_backticks(rhs))


def validate():
    locales = load_json(LOCALES_PATH)
    english = load_json(EN_PATH)
    ok = True

    for locale in locales:
        path = I18N_DIR / f"{locale}.json"
        if not path.exists():
            print(f"missing locale file: {path}")
            ok = False
            continue
        catalog = load_json(path)
        missing = sorted(set(english) - set(catalog))
        extra = sorted(set(catalog) - set(english))
        if missing:
            print(f"{locale}: missing keys: {', '.join(missing)}")
            ok = False
        if extra:
            print(f"{locale}: stale keys: {', '.join(extra)}")
            ok = False
        for key, source in english.items():
            target = catalog.get(key)
            if not isinstance(target, str):
                print(f"{locale}: key {key} must map to a string")
                ok = False
                continue
            if not same_placeholders(source, target):
                print(f"{locale}: placeholder mismatch for {key}")
                ok = False
            if source.count("\n") != target.count("\n"):
                print(f"{locale}: newline mismatch for {key}")
                ok = False
            if not same_literal_backticks(source, target):
                print(f"{locale}: backtick span mismatch for {key}")
                ok = False

    return 0 if ok else 1


def status():
    locales = load_json(LOCALES_PATH)
    english = load_json(EN_PATH)
    dirty = False

    for locale in locales:
        path = I18N_DIR / f"{locale}.json"
        if not path.exists():
            print(f"{locale}: missing file")
            dirty = True
            continue
        catalog = load_json(path)
        missing = len(set(english) - set(catalog))
        extra = len(set(catalog) - set(english))
        if missing or extra:
            dirty = True
        print(f"{locale}: missing={missing} stale={extra} keys={len(catalog)}")

    return 1 if dirty else 0


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in {"validate", "status"}:
        print("usage: ci/i18n_check.py [validate|status]", file=sys.stderr)
        return 2
    if not EN_PATH.exists():
        print(f"missing English source file: {EN_PATH}", file=sys.stderr)
        return 2
    if not LOCALES_PATH.exists():
        print(f"missing locale list: {LOCALES_PATH}", file=sys.stderr)
        return 2
    if sys.argv[1] == "validate":
        return validate()
    return status()


if __name__ == "__main__":
    raise SystemExit(main())
