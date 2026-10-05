#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
"""The render-layer import rule: `src/render` and `src/model` must not name `http`, `provider` or
`cache` (step 12's render-path audit; the CI `gates` job runs this from the repository root).

A line-based `grep` is defeatable by ordinary Rust: `use crate::{\\n cache::Cache, …}` and
`crate :: cache::Cache` (spaces around the path separator) both pass one. This is a small
token-aware scan instead. It first blanks comment and string-literal text (so a rustdoc link such as
`crate::provider::Capabilities`, or an error message that merely names a module, is documentation
rather than a dependency) while keeping every byte offset, then looks for

* a `crate`/`super` path segment naming `http`, `provider` or `cache` — whatever the spacing around
  `::`, so `crate :: cache` is one path;
* the names inside a grouped import, whose braces are matched by balance so a nested group
  (`crate::{model::{…}, cache}`) cannot hide one.

Nothing but the interpreter the runner already ships is needed. Exits 1 and prints `path:line` for
every offence.
"""

import pathlib
import re
import sys

ROOTS = ("src/render", "src/model")


def blanked(src: str) -> str:
    """Comment and string-literal text replaced by spaces, offsets and newlines kept."""
    out = list(src)
    n = len(src)

    def blank(a: int, b: int) -> None:
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    i = 0
    while i < n:
        c = src[i]
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = src.find("\n", i)
            j = n if j == -1 else j
            blank(i, j)
            i = j
        elif c == "/" and i + 1 < n and src[i + 1] == "*":
            depth, j = 0, i
            while j < n:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                    if depth == 0:
                        break
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == '"':
            j = i + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            blank(i, j)
            i = j
        else:
            i += 1
    return "".join(out)


PATH = re.compile(
    r"(?<![A-Za-z0-9_])(?:crate|super)\s*::\s*(http|provider|cache)(?![A-Za-z0-9_])"
)
GROUP = re.compile(r"(?<![A-Za-z0-9_])(?:crate|super)\s*::\s*\{")
NAME = re.compile(r"(?<![A-Za-z0-9_])(http|provider|cache)(?![A-Za-z0-9_])")


def offences(text: str) -> list[int]:
    """Offsets of every offending module name in the blanked source."""
    hits = []
    for match in PATH.finditer(text):
        hits.append(match.start(1))
    for match in GROUP.finditer(text):
        depth, j = 0, match.end() - 1
        while j < len(text):
            if text[j] == "{":
                depth += 1
            elif text[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        for name in NAME.finditer(text, match.end(), j):
            hits.append(name.start(1))
    return hits


def main() -> int:
    found = False
    scanned = 0
    for root in ROOTS:
        if not pathlib.Path(root).is_dir():
            print(f"::error::{root} does not exist — run this from the repository root")
            return 2
    for root in ROOTS:
        for source in sorted(pathlib.Path(root).rglob("*.rs")):
            scanned += 1
            text = blanked(source.read_text())
            for offset in sorted(offences(text)):
                line = text.count("\n", 0, offset) + 1
                print(f"{source}:{line}")
                found = True
    if found:
        print("::error::src/render and src/model must not reference http, provider or cache")
        return 1
    print(f"ok: {scanned} files under {' and '.join(ROOTS)} name no http, provider or cache")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
