#!/usr/bin/env python3
"""Check the agent skills in .agents/skills/ and the agent guide AGENTS.md.

Skills describe the codebase, so they go stale when code moves. This catches the mechanical part:

  * every skill is .agents/skills/<name>/SKILL.md with front matter giving `name` (equal to the
    folder name) and a one-line `description`;
  * every repository path a skill or AGENTS.md names in backticks still exists. A path is any
    backticked token with a slash in it (written from the repository root), or a bare name of a
    current top-level entry or of a root document such as `PROGRESS.md`. Tokens with placeholders
    such as <id>, Rust paths (`a::b`), git refs (`origin/main`) and relative paths are skipped;
  * every skill AGENTS.md mentions exists, and every skill is mentioned in AGENTS.md.

Usage: python3 tools/agents/check_skills.py     (exit status 1 on any problem)
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SKILLS = ROOT / ".agents" / "skills"
GUIDE = ROOT / "AGENTS.md"
TICKED = re.compile(r"`([^`\n]+)`")
# A root document named in capitals, such as README.md or PROGRESS.md. SKILL.md is the name of
# every skill file, not a root document.
ROOT_DOC = re.compile(r"[A-Z][A-Z0-9_-]*\.md")


def front_matter(text):
    """Return the front matter fields of a skill file as a dict, or None if it has none."""
    if not text.startswith("---\n"):
        return None
    end = text.find("\n---\n", 4)
    if end < 0:
        return None
    fields = {}
    for line in text[4:end].splitlines():
        key, sep, value = line.partition(":")
        if sep:
            fields[key.strip()] = value.strip()
    return fields


def named_paths(text, top_level):
    """Yield every backticked token in text that names a repository path."""
    for token in TICKED.findall(text):
        token = token.strip().rstrip(".,;:")
        if any(c in token for c in "<>*{}$ ") or "::" in token:
            continue
        if token.startswith(("-", "/", "~", ".", "http", "origin/")):
            continue
        if "/" in token:
            yield token
        elif token in top_level and "." in token or ROOT_DOC.fullmatch(token) and token != "SKILL.md":
            yield token


def main():
    """Check every skill and AGENTS.md; print the problems and return the exit status."""
    problems = []
    top_level = {p.name for p in ROOT.iterdir() if p.name != ".git"}
    names = []
    if not SKILLS.is_dir():
        print(f"no skills folder at {SKILLS.relative_to(ROOT)}")
        return 1
    for folder in sorted(p for p in SKILLS.iterdir() if p.is_dir()):
        skill = folder / "SKILL.md"
        where = skill.relative_to(ROOT)
        if not skill.is_file():
            problems.append(f"{folder.relative_to(ROOT)}: missing SKILL.md")
            continue
        names.append(folder.name)
        text = skill.read_text(encoding="utf-8")
        fields = front_matter(text)
        if fields is None:
            problems.append(f"{where}: no front matter (--- name / description ---)")
        else:
            if fields.get("name") != folder.name:
                problems.append(f"{where}: name is {fields.get('name')!r}, folder is {folder.name!r}")
            description = fields.get("description", "")
            if not description:
                problems.append(f"{where}: empty description")
            elif len(description) > 1024:
                problems.append(f"{where}: description is {len(description)} characters (max 1024)")
        for path in named_paths(text, top_level):
            if not (ROOT / path).exists():
                problems.append(f"{where}: names `{path}`, which does not exist")

    if GUIDE.is_file():
        guide = GUIDE.read_text(encoding="utf-8")
        for path in named_paths(guide, top_level):
            if not (ROOT / path).exists():
                problems.append(f"AGENTS.md: names `{path}`, which does not exist")
        for name in names:
            if f"`{name}`" not in guide:
                problems.append(f"AGENTS.md: skill `{name}` is not mentioned")
        for token in TICKED.findall(guide):
            if re.fullmatch(r"(spec|bug|dstudio)-[a-z-]+", token) and token not in names:
                problems.append(f"AGENTS.md: mentions skill `{token}`, which does not exist")
    else:
        problems.append("AGENTS.md is missing")

    for problem in problems:
        print(problem)
    if problems:
        return 1
    print(f"{len(names)} skills ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
