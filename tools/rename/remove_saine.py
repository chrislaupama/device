#!/usr/bin/env python3
"""Rename the old internal name "saine" to "dstudio" across the repository.

Device Studio was called Saine Studio until November 2026. This script does the
internal part of that rename: folders and files, crate and package names, the
C++ namespace, the engine header, CMake targets and options, environment
variables and test plugin ids. It is idempotent, so a branch that started
before the rename can run it and then merge main with far fewer conflicts:

    python3 tools/rename/remove_saine.py          # rewrite the working tree
    python3 tools/rename/remove_saine.py --check  # list what is still left

Mapping: saine -> dstudio, Saine -> DStudio, SAINE -> DSTUDIO, and the product
name Saine Studio -> Device Studio. The strings that let the app open projects
and settings saved under the old name are kept (see KEEP), as are links to the
GitHub repository, which is still called sainestudio.
"""
import os
import re
import subprocess
import sys

ROOT = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip()

# Kept verbatim: legacy project/settings names that old files on disk still use,
# history notes about the rename, and the GitHub repository's own name.
KEEP = [
    # persist.rs: the format tag and folder extension old projects carry.
    '"saine-project"',
    '"saineproj"',
    "`Name.saineproj/`",
    "`.saineproj` / `saine-project`",
    "`saine-project`",
    'let old = dir.path().join("Song.saineproj")',
    # Built-in device vendor written into old projects.
    'Some("Saine Studio")',
    '\\"vendor\\": \\"Saine Studio\\"',
    'VENDOR, "Saine Studio")',
    # files.rs: the old projects folder and extension, still listed.
    'docs.join("Saine Studio")',
    "`~/Documents/Saine Studio`",
    "(and `.saineproj` ones saved before the rename)",
    # prefs.rs: the old support folder, moved on first launch.
    'support_dir_named("Saine Studio", "saine-studio")',
    'root.join("Saine Studio")',
    "`Saine Studio` (`saine-studio` on Linux)",
    "`Saine Studio` support folder",
    "Saine Studio folder",
    # History notes and the GitHub repository's own name.
    "called Saine Studio",
    "formerly Saine Studio",
    "chrislaupama/sainestudio",
    "cd sainestudio",
    "remove_saine.py",
    "`saine`",
    "`sainestudio`",
]
SELF = "tools/rename/remove_saine.py"

WORDS = [
    ("com.saine.studio", "com.devicestudio.app"),
    ("saineproj", "dsproj"),
    ("SAINE STUDIO", "DEVICE STUDIO"),
    ("Saine Studio", "Device Studio"),
    ("SaineStudio", "DeviceStudio"),
    ("sainestudio", "devicestudio"),
    ("saine-studio", "device-studio"),
    ("SAINE", "DSTUDIO"),
    ("Saine", "DStudio"),
    ("saine", "dstudio"),
]
WORD_RE = re.compile("|".join(re.escape(a) for a, _ in WORDS))
WORD_MAP = dict(WORDS)


def rewrite(text: str) -> str:
    holds = {}
    for i, k in enumerate(KEEP):
        token = f"\x00KEEP{i}\x00"
        if k in text:
            holds[token] = k
            text = text.replace(k, token)
    text = WORD_RE.sub(lambda m: WORD_MAP[m.group(0)], text)
    for token, k in holds.items():
        text = text.replace(token, k)
    return text


def tracked():
    out = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True).stdout
    return [p for p in out.decode().split("\0") if p]


def is_text(path: str) -> bool:
    with open(path, "rb") as f:
        return b"\0" not in f.read(8192)


def main() -> int:
    check = "--check" in sys.argv
    left = []
    files = tracked()
    for rel in files:
        if rel == SELF:
            continue
        path = os.path.join(ROOT, rel)
        if not os.path.isfile(path) or not is_text(path):
            continue
        with open(path, encoding="utf-8", errors="surrogateescape") as f:
            text = f.read()
        new = rewrite(text)
        if new != text:
            if check:
                left.append(rel)
            else:
                with open(path, "w", encoding="utf-8", errors="surrogateescape", newline="") as f:
                    f.write(new)
    # Paths: rename deepest first so parent folders move last.
    moves = sorted({rel for rel in files if rel != SELF and rewrite(rel) != rel}, key=lambda p: -p.count("/"))
    for rel in moves:
        if check:
            left.append(rel + " (path)")
            continue
        dst = rewrite(rel)
        os.makedirs(os.path.join(ROOT, os.path.dirname(dst)), exist_ok=True)
        subprocess.run(["git", "mv", "-k", rel, dst], cwd=ROOT, check=True)
    # Drop the old folders the moves emptied (untracked build output keeps them).
    for d in sorted({os.path.dirname(rel) for rel in moves}, key=lambda p: -p.count("/")):
        while d:
            try:
                os.rmdir(os.path.join(ROOT, d))
            except OSError:
                break
            d = os.path.dirname(d)
    if check:
        print("\n".join(left) if left else "no saine names left (outside KEEP)")
        return 1 if left else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
