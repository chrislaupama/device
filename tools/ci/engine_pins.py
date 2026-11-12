#!/usr/bin/env python3
"""Report engine third-party pins that are behind upstream or whose tag moved.

Reads the pins from engine/CMakeLists.txt (the DSTUDIO_*_TAG, *_TAG_COMMIT and
*_COMMIT variables and the FetchContent_Declare blocks that use them) and asks
each upstream repository, with `git ls-remote`, for its tags and default
branch. Prints a Markdown report. Exit status: 0 everything current, 1
something to do, 2 the check itself failed.

  tools/ci/engine_pins.py [--cmake engine/CMakeLists.txt]

Needs git and network access to the upstream repositories, nothing else.
"""
import argparse
import re
import subprocess
import sys
import tempfile

# Tags that count as releases, per tag pin. A commit pin (a repository
# without usable release tags) lists the files we compile instead: it is
# behind when the default branch changed one of them since the pinned commit.
RELEASE_TAGS = {
    "DSTUDIO_CLAP_TAG": r"^\d+\.\d+\.\d+$",
    "DSTUDIO_VST3SDK_TAG": r"^v\d+\.\d+\.\d+_build_\d+$",
    "DSTUDIO_STRETCH_TAG": r"^\d+\.\d+\.\d+$",
    "DSTUDIO_LINEAR_TAG": r"^\d+\.\d+\.\d+$",
}
COMMIT_PIN_FILES = {
    "DSTUDIO_DR_LIBS_COMMIT": ["dr_mp3.h", "dr_flac.h", "dr_wav.h"],
    "DSTUDIO_STB_COMMIT": ["stb_vorbis.c"],
}


def parse_pins(text):
    """[(variable, repository URL, value, recorded tag commit or None)], in file order."""
    variables = dict(re.findall(r"^set\((DSTUDIO_\w+)\s+([^\s)]+)\)", text, re.M))
    pins = []
    for block in re.findall(r"FetchContent_Declare\((.*?)\)", text, re.S):
        repo = re.search(r"GIT_REPOSITORY\s+(\S+)", block)
        var = re.search(r"GIT_TAG\s+\$\{(\w+)\}", block)
        if not repo or not var:
            continue
        name = var.group(1)
        if name not in variables:
            raise ValueError(f"{name} is used by FetchContent_Declare but never set")
        pins.append((name, repo.group(1), variables[name], variables.get(name + "_COMMIT") if name.endswith("_TAG") else None))
    return pins


def version_key(tag):
    return [int(n) for n in re.findall(r"\d+", tag)]


def newest(tags, pattern):
    matching = [t for t in tags if re.match(pattern, t)]
    return max(matching, key=version_key) if matching else None


def ls_remote(repo):
    """({tag: commit}, default-branch commit). Annotated tags resolve to their commit."""
    out = subprocess.run(["git", "ls-remote", repo], check=True, capture_output=True, text=True, timeout=120).stdout
    tags, head = {}, None
    for line in out.splitlines():
        sha, ref = line.split("\t")
        if ref == "HEAD":
            head = sha
        elif ref.startswith("refs/tags/"):
            name = ref[len("refs/tags/"):]
            if name.endswith("^{}"):
                tags[name[:-3]] = sha  # the commit an annotated tag points at
            else:
                tags.setdefault(name, sha)
    return tags, head


def changed_files(repo, base, head):
    """Files that differ between two commits of a repository, or None if unknown.

    Uses a blobless clone (commits and trees only), so it needs nothing but git.
    """
    with tempfile.TemporaryDirectory() as tmp:
        try:
            subprocess.run(["git", "clone", "--quiet", "--filter=blob:none", "--no-checkout", repo, tmp], check=True, capture_output=True, timeout=600)
            out = subprocess.run(["git", "-C", tmp, "diff", "--name-only", base, head], check=True, capture_output=True, text=True, timeout=120)
        except subprocess.SubprocessError:
            return None
    return out.stdout.split("\n")


def check(pins, remote=ls_remote, diff=changed_files):
    """(findings, table rows). `remote` and `diff` are replaceable for the self-test."""
    findings, rows = [], []
    for name, repo, value, recorded in pins:
        tags, head = remote(repo)
        page = repo.removesuffix(".git")
        if name in RELEASE_TAGS:
            latest = newest(tags, RELEASE_TAGS[name])
            status = "current"
            if value not in tags:
                status = "pinned tag is gone upstream"
                findings.append(f"`{name}`: tag `{value}` no longer exists in {page}.")
            elif recorded is None:
                status = "no recorded commit"
                findings.append(f"`{name}`: add `{name}_COMMIT {tags[value]}` next to it.")
            elif tags[value] != recorded:
                status = "TAG MOVED"
                findings.append(f"`{name}`: tag `{value}` now points at `{tags[value]}`, not the recorded `{recorded}`. Review the change before trusting it.")
            if latest and latest != value and version_key(latest) > version_key(value):
                status = f"behind: {latest}" if status == "current" else status
                findings.append(f"`{name}`: `{value}` is behind `{latest}` ({page}/releases).")
            rows.append((name, value, latest or "?", status))
        elif name in COMMIT_PIN_FILES:
            status = "current"
            if head is not None and head != value:
                files = diff(repo, value, head)
                compare = f"{page}/compare/{value}...{head}"
                if files is None:
                    status = "default branch moved"
                    findings.append(f"`{name}`: the default branch moved and the changed files could not be listed; check {compare}.")
                else:
                    ours = sorted(set(files) & set(COMMIT_PIN_FILES[name]))
                    if ours:
                        status = "behind: " + ", ".join(ours)
                        findings.append(f"`{name}`: {', '.join(f'`{f}`' for f in ours)} changed upstream since the pinned commit ({compare}).")
            rows.append((name, value[:12], (head or "?")[:12], status))
        else:
            findings.append(f"`{name}` is not known to tools/ci/engine_pins.py: add it to RELEASE_TAGS or COMMIT_PIN_FILES.")
            rows.append((name, value, "?", "unknown pin"))
    return findings, rows


def report(findings, rows):
    lines = ["| Pin | Pinned | Upstream | Status |", "|---|---|---|---|"]
    lines += [f"| `{n}` | `{v}` | `{u}` | {s} |" for n, v, u, s in rows]
    lines.append("")
    if findings:
        lines += ["To do:", ""] + [f"- {f}" for f in findings]
        lines += ["", "How to update a pin: docs/building.md, \"Engine dependencies\"."]
    else:
        lines.append("Every engine pin is current.")
    return "\n".join(lines)


def self_test():
    cmake = """
set(DSTUDIO_CLAP_TAG 1.2.10)
set(DSTUDIO_CLAP_TAG_COMMIT aaa)
set(DSTUDIO_LINEAR_TAG 0.6.4)
set(DSTUDIO_LINEAR_TAG_COMMIT bbb)
set(DSTUDIO_DR_LIBS_COMMIT ccc)
set(DSTUDIO_STB_COMMIT ddd)
FetchContent_Declare(a GIT_REPOSITORY https://example.org/clap.git GIT_TAG ${DSTUDIO_CLAP_TAG} GIT_SHALLOW TRUE)
FetchContent_Declare(b GIT_REPOSITORY https://example.org/linear.git GIT_TAG ${DSTUDIO_LINEAR_TAG})
FetchContent_Declare(c GIT_REPOSITORY https://example.org/dr_libs.git GIT_TAG ${DSTUDIO_DR_LIBS_COMMIT})
FetchContent_Declare(d GIT_REPOSITORY https://example.org/stb.git GIT_TAG ${DSTUDIO_STB_COMMIT})
"""
    pins = parse_pins(cmake)
    assert [p[0] for p in pins] == ["DSTUDIO_CLAP_TAG", "DSTUDIO_LINEAR_TAG", "DSTUDIO_DR_LIBS_COMMIT", "DSTUDIO_STB_COMMIT"], pins
    assert pins[0][3] == "aaa" and pins[2][3] is None
    current = {
        "https://example.org/clap.git": ({"1.2.9": "x", "1.2.10": "aaa", "1.2.10-rc1": "y"}, "h"),
        "https://example.org/linear.git": ({"0.6.4": "bbb"}, "h"),
        "https://example.org/dr_libs.git": ({"wav-0.14.6": "ccc"}, "ccc"),
        "https://example.org/stb.git": ({}, "eee"),  # moved, but not in a file we use
    }
    diffs = {"https://example.org/stb.git": ["stb_image.h"], "https://example.org/dr_libs.git": ["dr_mp3.h", "README.md"]}
    findings, rows = check(pins, current.__getitem__, lambda repo, a, b: diffs[repo])
    assert not findings and all(r[3] == "current" for r in rows), (findings, rows)
    stale = dict(current)
    stale["https://example.org/clap.git"] = ({"1.2.10": "moved", "1.10.0": "z"}, "h")  # moved tag, and 1.10 > 1.2.10
    stale["https://example.org/dr_libs.git"] = ({}, "new")
    findings, rows = check(pins, stale.__getitem__, lambda repo, a, b: diffs[repo])
    text = report(findings, rows)
    assert len(findings) == 3, findings
    assert "TAG MOVED" in text and "`1.10.0`" in text and "`dr_mp3.h` changed upstream" in text, text
    findings, _ = check(pins, current.__getitem__, lambda repo, a, b: None)
    assert len(findings) == 1 and "could not be listed" in findings[0], findings
    print("engine_pins self-test ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--cmake", default="engine/CMakeLists.txt")
    ap.add_argument("--self-test", action="store_true", help="check the parser and the comparison offline")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    try:
        with open(args.cmake, encoding="utf-8") as f:
            pins = parse_pins(f.read())
        if not pins:
            raise ValueError(f"no pins found in {args.cmake}")
        findings, rows = check(pins)
    except (OSError, ValueError, subprocess.SubprocessError) as e:
        print(f"engine_pins: {e}", file=sys.stderr)
        return 2
    print(report(findings, rows))
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
