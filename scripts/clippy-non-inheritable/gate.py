#!/usr/bin/env python3
"""The socket gate: no production code may open a socket a Windows child inherits.

Run it through `scripts/check-non-inheritable-sockets.sh`, which checks the
Python version first; that script's header is the user-facing description.
This file is the implementation, in Python because it reads three structured
formats (the gate's TOML, `cargo metadata` and cargo's JSON diagnostics) and
the Windows cross container has no `jq`.

Threat model: this gate stops MISTAKES (a developer or a model writing an
inheritable socket call into production code), not a determined evader. It
closes every bypass that is cheap to close and names the rest; the list is in
the header of `scripts/check-non-inheritable-sockets.sh`.

Exit codes (a higher-priority failure wins when several happen at once, and
all of them are printed):

    0  clean: clippy linted every production crate and found no site
    2  BROKEN: the check did not complete (compile error, a clippy
       configuration clippy cannot parse, cargo failing, a crate that was
       never linted, a clippy that does not know the lint by name), so it
       proves nothing about what it could not reach
    3  CONFIGURATION: the gate would guard less than it says: its clippy.toml
       (an entry in a form the gate does not read, a crate no production
       code links, a path clippy cannot resolve to a function), or source
       that hides code from clippy (a `clippy` cfg predicate, build-time
       code that reads clippy's environment)
    1  SITES: production code calls a function the configuration forbids
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

EXIT_OK = 0
EXIT_SITES = 1
EXIT_BROKEN = 2
EXIT_CONFIG = 3

LINT = "clippy::disallowed_methods"

CONF_DIR = Path(__file__).resolve().parent
ROOT = CONF_DIR.parent.parent
CONF = CONF_DIR / "clippy.toml"

# The keys clippy 1.92 reads on a `disallowed-methods` entry, minus
# `allow-invalid`, which exists to silence the "does not refer to a reachable
# function" warning that step 4 below turns into a failure.
ENTRY_KEYS = {"path", "reason", "replacement"}
# Only whether an entry is a path at all. Whether it names a FUNCTION is
# clippy's to say, and the gate fails on its answer (step 4): clippy 1.92 warns
# "expected a function, found a struct" (or "a module", "an associated
# constant"), and does not fail on it. The case of the last segment says
# nothing either way: `windows_sys::…::WinSock::WSASocketW` is a function, and a
# module is as snake_case as one.
PATH_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)+")

# The lint a "this lint is unknown / renamed / removed" message is about: the
# first one it names. rustc 1.92 writes them as "unknown lint: `X`", "lint `X`
# has been renamed to `Y`" and "lint `X` has been removed: …".
LINT_SUBJECT_RE = re.compile(r"(?:unknown lint: |lint )`([^`]+)`")

# rustc's own tally at the end of a crate, which is not a warning of its own.
SUMMARY_RE = re.compile(r"(\d+|`.*`) (generated|emitted) \d+ warnings?.*|\d+ warnings? emitted")

# The target kinds `--lib --bins` lints.
LINTED_KINDS = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro", "bin"}


def say(*lines: str) -> None:
    for line in lines:
        print(line, file=sys.stderr)


def error(message: str) -> None:
    # `::error::` is a GitHub Actions annotation; a terminal shows it as text.
    say(f"::error::{message}")


# ── 1. The configuration, read by a real TOML parser. ─────────────────────────


class Unreadable(Exception):
    """The configuration cannot be read at all, so clippy could not run on it."""


def read_config() -> tuple[list[str], list[str]]:
    """The paths the configuration forbids, and why it cannot be trusted."""
    problems: list[str] = []
    if not CONF.is_file():
        raise Unreadable(f"{CONF} is missing; this gate would check nothing")
    # clippy prefers `.clippy.toml` to `clippy.toml` in the same directory, so
    # a second file here would silently replace the one this gate reads.
    for other in (".clippy.toml",):
        if (CONF_DIR / other).exists():
            problems.append(
                f"{CONF_DIR / other} exists; clippy would read it instead of clippy.toml"
            )
    try:
        data = tomllib.loads(CONF.read_text(encoding="utf-8"))
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as e:
        raise Unreadable(f"{CONF} does not parse, so clippy could not read it either: {e}")

    for key in sorted(set(data) - {"disallowed-methods"}):
        problems.append(
            f"top-level key `{key}`: this configuration holds `disallowed-methods` and "
            "nothing else, and the gate checks nothing else. Extend gate.py before adding one."
        )
    entries = data.get("disallowed-methods")
    if not isinstance(entries, list) or not entries:
        problems.append("`disallowed-methods` is missing, empty, or not an array")
        return [], problems

    paths: list[str] = []
    for n, entry in enumerate(entries, 1):
        # clippy accepts a bare string as well as a table; so does the gate.
        if isinstance(entry, str):
            path = entry
        elif isinstance(entry, dict):
            unknown = sorted(set(entry) - ENTRY_KEYS)
            if unknown:
                problems.append(
                    f"entry {n}: key(s) {', '.join(unknown)} are not ones the gate accepts "
                    f"({', '.join(sorted(ENTRY_KEYS))}); `allow-invalid` in particular would "
                    "hide a path clippy cannot resolve"
                )
            for key in ("reason", "replacement"):
                if key in entry and not isinstance(entry[key], str):
                    problems.append(f"entry {n}: `{key}` is not a string")
            path = entry.get("path")
            if not isinstance(path, str):
                problems.append(f"entry {n}: no string `path`")
                continue
        else:
            problems.append(f"entry {n}: neither a string nor a table: {entry!r}")
            continue
        if not PATH_RE.fullmatch(path):
            problems.append(f"entry {n}: `{path}` is not a `crate::…::item` path")
            continue
        if path in paths:
            problems.append(f"entry {n}: `{path}` is listed twice")
        paths.append(path)
    return paths, problems


# ── 2. Every crate the configuration names is one production code links. ─────


def cargo_metadata() -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        text=True,
    )
    if out.returncode != 0:
        raise SystemExit(broken("`cargo metadata` failed (its error is above)"))
    return json.loads(out.stdout)


def production_graph(meta: dict) -> tuple[set[str], set[tuple[str, str]]]:
    """The crate names linked by production code, and the workspace targets
    `--lib --bins` must lint.

    Reachable from a workspace member over NORMAL dependency edges only, on any
    platform: a crate reached only through a dev- or build-dependency is not in
    anything that ships. Read from `cargo metadata`'s resolve graph rather than
    `cargo tree -i <name>`, which refuses with "ambiguous" whenever two versions
    of a crate are locked (socket2 and hyper are today), and which the previous
    script turned into a false "no production code links it"."""
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = meta["workspace_members"]
    seen: set[str] = set()
    stack = list(members)
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    crates = set()
    for pid in seen:
        for target in packages[pid]["targets"]:
            if set(target["kind"]) & (LINTED_KINDS - {"bin"}):
                # A path names the crate by its crate name, which is the lib
                # target's name with `-` as `_`.
                crates.add(target["name"].replace("-", "_"))
    expected = set()
    for pid in members:
        package = packages[pid]
        for target in package["targets"]:
            if set(target["kind"]) & LINTED_KINDS and not target.get("required-features"):
                expected.add((package["name"], target["name"]))
    return crates, expected


# ── 3. No source hides code from clippy. ─────────────────────────────────────
#
# clippy-driver compiles with `--cfg clippy`, so `#[cfg(not(clippy))]` code
# ships in every binary and never reaches the lint, and build-time code (a
# build script, a proc macro) that reads clippy's environment can emit such a
# cfg, or different code, for clippy alone. rustc and cargo accept all of it
# without a word, and `--force-warn` cannot reach code that was never compiled.

LITERAL_START = re.compile(r"//|/\*|(?<!\w)[bc]?r(#*)\"|(?<!\w)[bc]\"|\"|(?<!\w)b'|'")
BARE_CLIPPY = re.compile(r"(?<!\w)clippy(?!\w)")
CLIPPY_ENV = re.compile(r"\w*CLIPPY\w*|RUSTC_WORKSPACE_WRAPPER")
PATH_AFTER = re.compile(r"\s*::")
PATH_BEFORE = re.compile(r"::\s*(r#)?$")


def rust_code(text: str, keep_strings: bool = False) -> str:
    """`text` with its comments blanked, and its string and char literals too
    unless `keep_strings`. Every newline is kept, so a line number still names
    the line it did."""

    def blank(s: str) -> str:
        return re.sub(r"[^\n]", " ", s)

    out: list[str] = []
    i, n = 0, len(text)
    while m := LITERAL_START.search(text, i):
        out.append(text[i : m.start()])
        start, tok = m.start(), m.group(0)
        if tok == "//":
            end = text.find("\n", start)
            end = n if end < 0 else end
        elif tok == "/*":
            depth, end = 0, start
            while end < n:
                if text.startswith("/*", end):
                    depth, end = depth + 1, end + 2
                elif text.startswith("*/", end):
                    depth, end = depth - 1, end + 2
                    if depth == 0:
                        break
                else:
                    end += 1
        elif tok.endswith('"') and "r" in tok:
            close = text.find('"' + m.group(1), m.end())
            end = n if close < 0 else close + 1 + len(m.group(1))
        elif tok.endswith('"'):
            end = m.end()
            while end < n and text[end] != '"':
                end += 2 if text[end] == "\\" else 1
            end = min(end + 1, n)
        else:
            k = m.end()
            if text.startswith("\\", k):
                close = text.find("'", k + 2)
                end = n if close < 0 else close + 1
            elif text.startswith("'", k + 1):
                end = k + 2
            else:
                # A lifetime or a label (`'a`, `'outer:`), not a literal.
                out.append(tok)
                i = m.end()
                continue
        comment = tok in ("//", "/*")
        out.append(text[start:end] if keep_strings and not comment else blank(text[start:end]))
        i = end
    out.append(text[i:])
    return "".join(out)


def hidden_from_clippy(meta: dict) -> tuple[list[str], int, int]:
    """Where the source hides code from clippy, with how many files and
    build-time files were read.

    A bare `clippy` is refused wherever it stands, not only inside `cfg(…)`:
    a macro can take the predicate as an argument (`gated!(not(clippy), …)`)
    and text cannot tell that from any other use. Nothing in the tree names
    anything `clippy`, so the only cost is renaming something that does."""
    crates = ROOT / "crates"
    members = set(meta["workspace_members"])
    build_time = {p.resolve() for p in crates.rglob("build.rs")}
    for package in meta["packages"]:
        if package["id"] not in members:
            continue
        for target in package["targets"]:
            if "custom-build" in target["kind"]:
                build_time.add(Path(target["src_path"]).resolve())
            if "proc-macro" in target["kind"]:
                crate_dir = Path(package["manifest_path"]).parent
                build_time.update(p.resolve() for p in crate_dir.rglob("*.rs"))
    files = sorted(p.resolve() for p in crates.rglob("*.rs"))
    problems: list[str] = []
    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        rel = path.relative_to(ROOT)
        code = rust_code(text)
        for m in BARE_CLIPPY.finditer(code):
            before = code[max(0, m.start() - 40) : m.start()]
            if PATH_BEFORE.search(before) or PATH_AFTER.match(code, m.end()):
                continue
            line = code.count("\n", 0, m.start()) + 1
            problems.append(
                f"{rel}:{line}: `clippy` as a name, not a `clippy::` path: a cfg predicate "
                "(`cfg(not(clippy))`, `cfg_attr(clippy, …)`, `cfg!(clippy)`), or an argument a "
                "macro makes one of. clippy compiles with `--cfg clippy`, so the code it gates "
                "either ships unlinted or is linted and never ships."
            )
        if path in build_time:
            code = rust_code(text, keep_strings=True)
            for m in CLIPPY_ENV.finditer(code):
                line = code.count("\n", 0, m.start()) + 1
                problems.append(
                    f"{rel}:{line}: build-time code names `{m.group(0)}`, which is clippy's "
                    "environment (`cargo clippy` sets it), so it can build different code for "
                    "clippy than for the binary that ships."
                )
    return problems, len(files), len(build_time)


# ── 4. The lint, with every allow in the tree ignored. ───────────────────────


def lint_unknown() -> str | None:
    """Why this clippy does not know LINT, or None when it does.

    A `--force-warn` of a lint clippy does not know (a toolchain bump renamed
    or removed it) is only a warning, and every run after it would lint nothing
    and pass. `--explain` knows only the current name of a live lint, so a
    renamed one fails here too, not only a removed one."""
    name = LINT.split("::", 1)[1]
    out = subprocess.run(
        ["cargo", "clippy", "--explain", name],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if out.returncode != 0:
        return f"`cargo clippy --explain {name}` exited {out.returncode}: {out.stdout.strip()}"
    return None


def says_our_lint_is_unknown(diag: dict) -> bool:
    """Whether `diag` says this clippy does not know LINT by that name.

    Asked of the lint the message is ABOUT, not of every lint it names. An
    `#[allow(clippy::disallowed_method)]` in the source draws "lint
    `clippy::disallowed_method` has been renamed to `clippy::disallowed_methods`":
    it names LINT, as the lint an OLD name now means, and says nothing against
    it; the force-warned lint still reports the call under that allow. Matching
    LINT anywhere in the text read that as BROKEN, which blamed the gate for a
    site it had found."""
    code = (diag.get("code") or {}).get("code")
    if code not in (
        "E0602",  # "unknown lint", when the lint is named on the command line
        "unknown_lints",
        "renamed_and_removed_lints",
    ):
        return False
    subject = LINT_SUBJECT_RE.match(diag["message"])
    return subject is not None and subject.group(1) == LINT


def broken(message: str) -> int:
    error(message)
    say(
        "The gate did NOT complete, so it proves nothing about the code it could not reach.",
        "This is not a socket finding: fix what is reported above and run it again.",
    )
    return EXIT_BROKEN


def location(span: dict) -> str:
    here = f"{span['file_name']}:{span['line_start']}:{span['column_start']}"
    # A site inside a macro: name where the macro was called, outermost last.
    calls = []
    expansion = span.get("expansion")
    while expansion:
        call = expansion["span"]
        calls.append(f"{expansion['macro_decl_name']} at {call['file_name']}:{call['line_start']}")
        expansion = call.get("expansion")
    return here + "".join(f" (expanded from {c})" for c in calls)


def run_clippy(cargo_args: list[str], names: dict[str, str]) -> dict:
    # `--force-warn`, not `-D`. rustc applies `-D` like any other level, so an
    # `#[allow]` in the source beats it, in every spelling the text scan this
    # replaced never read: the renamed `clippy::disallowed_method`, a raw
    # `clippy::r#disallowed_methods`, `clippy:: disallowed_methods`, an allow a
    # macro emits, `#![allow]` in a `#[path]` module or an `include!`d file
    # outside `crates/*/src`, and the `#[allow(clippy::style)]` clap_derive
    # puts around a `default_value_t` expression. `--force-warn` is the one
    # level no attribute can lower (nor `#[expect]` fulfil), so every call
    # site is reported, and it is this script, not rustc's exit status, that
    # fails on one.
    #
    # `--release`: the profile that ships. `cfg(not(debug_assertions))` code is
    # compiled only there, and `cfg(debug_assertions)` code never ships.
    #
    # `--keep-going`: lint every crate that can be linted, not only up to the
    # first that fails.
    cmd = [
        "cargo", "clippy", "--workspace", "--lib", "--bins", "--locked", "--release",
        "--keep-going", "--message-format=json", *cargo_args,
        "--", "-A", "clippy::all", "--force-warn", LINT,
    ]
    env = dict(os.environ, CLIPPY_CONF_DIR=str(CONF_DIR))
    say("+ CLIPPY_CONF_DIR=" + str(CONF_DIR.relative_to(ROOT)) + " " + " ".join(cmd))
    result = {
        "sites": {},  # (file, line, col, message) -> where, as printed
        "unresolved": [],
        "unknown_lint": [],
        "errors": [],
        "warnings": 0,
        "linted": set(),
        "status": None,
    }
    proc = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, text=True)
    assert proc.stdout is not None
    for line in proc.stdout:
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            sys.stderr.write(line)
            continue
        reason = msg.get("reason")
        if reason == "compiler-artifact" and msg["package_id"].startswith("path+"):
            name = names.get(msg["package_id"], msg["package_id"])
            result["linted"].add((name, msg["target"]["name"]))
            continue
        if reason != "compiler-message":
            continue
        diag = msg["message"]
        code = (diag.get("code") or {}).get("code")
        rendered = diag.get("rendered") or diag["message"]
        if code == LINT:
            span = next((s for s in diag["spans"] if s["is_primary"]), None)
            if span:
                key = (span["file_name"], span["line_start"], span["column_start"], diag["message"])
                where = location(span)
            else:
                key, where = ("", 0, 0, diag["message"]), "(no location)"
            if key not in result["sites"]:
                result["sites"][key] = where
                sys.stderr.write(rendered)
        elif "does not refer to a reachable function" in diag["message"] or diag[
            "message"
        ].startswith("expected a function, found"):
            if rendered not in result["unresolved"]:
                result["unresolved"].append(rendered)
        elif says_our_lint_is_unknown(diag):
            if rendered not in result["unknown_lint"]:
                result["unknown_lint"].append(rendered)
        elif diag["level"].startswith("error"):
            result["errors"].append(rendered)
            sys.stderr.write(rendered)
        elif diag["level"] == "warning" and not SUMMARY_RE.fullmatch(diag["message"]):
            result["warnings"] += 1
    result["status"] = proc.wait()
    return result


def main(argv: list[str]) -> int:
    if "--" in argv:
        return broken("pass cargo arguments only (`--target <triple>`); the lint flags are the gate's")

    # 1.
    try:
        paths, problems = read_config()
    except Unreadable as e:
        return broken(str(e))
    if problems:
        for problem in problems:
            error(f"{CONF.relative_to(ROOT)}: {problem}")
        say("The configuration is not in a form the gate can vouch for, so it was not run.")
        return EXIT_CONFIG
    say(f"The gate forbids {len(paths)} function(s):", *(f"  {p}" for p in paths))

    # 2.
    meta = cargo_metadata()
    linked, expected = production_graph(meta)
    unlinked = sorted({p.split("::", 1)[0] for p in paths} - linked)
    if unlinked:
        error(
            f"{CONF.relative_to(ROOT)} names crates no production code links: {', '.join(unlinked)}."
        )
        say(
            "clippy ignores such an entry without a word, so it guards nothing.",
            "Delete it, or correct the crate name (a path uses the crate name, `_` not `-`).",
        )
        return EXIT_CONFIG
    crates = sorted({p.split("::", 1)[0] for p in paths})
    say(f"Every crate it names is linked by production code: {' '.join(crates)}")

    # 3.
    hidden, files, build_time = hidden_from_clippy(meta)
    if not files:
        return broken(f"found no .rs file under {ROOT / 'crates'}, so it read nothing")
    if hidden:
        for problem in hidden:
            error(problem)
        say(
            "Code clippy never compiles is code this gate never checks, so it was not run.",
            "Delete the predicate, or the build-time code that looks for clippy.",
        )
        return EXIT_CONFIG
    say(
        f"No source hides code from clippy: {files} .rs file(s) read, no `clippy` cfg "
        f"predicate, and {build_time} build-time file(s) read none of clippy's environment."
    )

    # 4.
    unknown = lint_unknown()
    if unknown:
        return broken(
            f"this clippy does not know `{LINT}`, so a run would lint nothing and pass: {unknown}"
        )
    result = run_clippy(argv, {p["id"]: p["name"] for p in meta["packages"]})
    code = EXIT_OK
    never_linted = sorted(expected - result["linted"])
    say("")
    if result["status"] != 0 or result["errors"]:
        code = broken(
            f"clippy itself failed (exit {result['status']}, {len(result['errors'])} error(s)): "
            "a compile error, a clippy configuration it cannot parse, or cargo failing."
        )
    elif result["unknown_lint"]:
        code = broken(
            f"clippy did not recognise `{LINT}` by that name, so it linted nothing:\n"
            + "".join(result["unknown_lint"])
        )
    elif never_linted:
        code = broken(
            "clippy finished, but never linted "
            + ", ".join(f"{pkg}/{target}" for pkg, target in never_linted)
            + "; a crate it did not lint is a crate it did not check"
        )
    if result["unresolved"]:
        error(f"an entry in {CONF.relative_to(ROOT)} names no function clippy can find:")
        for rendered in result["unresolved"]:
            sys.stderr.write(rendered)
        say(
            "clippy only warns about this (a typo, an upstream rename, or a path to a type or",
            "a module), so the entry was guarding nothing. Fix the path.",
        )
        code = code if code == EXIT_BROKEN else EXIT_CONFIG
    if result["sites"]:
        error(
            f"production code opens a socket that a Windows child process inherits "
            f"({len(result['sites'])} site(s)):"
        )
        for key in sorted(result["sites"]):
            say(f"  {result['sites'][key]}  {key[3]}")
        say(
            "",
            "Bind with biorouter::net::bind_non_inheritable, connect with",
            "biorouter::net::connect_non_inheritable; the reason is under each site above.",
            "Test code is exempt and needs no attribute. An #[allow] does not exempt a site:",
            "the gate reports every call regardless.",
        )
        code = code if code in (EXIT_BROKEN, EXIT_CONFIG) else EXIT_SITES
    if code == EXIT_OK:
        say(
            f"OK: clippy linted {len(result['linted'] & expected)} production targets "
            f"({result['warnings']} ordinary warning(s), not this gate's concern), "
            "and no production socket is inheritable by a child process."
        )
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
