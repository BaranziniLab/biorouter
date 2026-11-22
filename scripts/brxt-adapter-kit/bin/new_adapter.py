"""Render an adapter, sunset or tombstone repository from one kit spec file.

    uv run python bin/new_adapter.py SPEC.json OUT_DIR [--force] [--lock] [--previous OLD.brxt]

SPEC.json holds the manifest fields, the adapter.json content and the bundled
skills (the format is in docs/extensions/brxt-adapter-kit.md). OUT_DIR may be a
new folder or an existing adapter repository. The command writes only the files
the kit owns; with an existing file that differs it stops unless --force is
given, and it never deletes a file it did not write. It then lists the files in
OUT_DIR the kit did not write, so a converted repository's leftovers (the old
build_brxt.sh, for example) are reviewed by a person.

--previous checks that every skill the old bundle shipped is still in the spec.
--lock runs `uv lock` in OUT_DIR, which needs network access the first time.
"""

from __future__ import annotations

import argparse
import asyncio
import datetime
import json
import re
import subprocess
import sys
import zipfile
from pathlib import Path
from typing import Any, Optional

KIT_ROOT = Path(__file__).resolve().parents[1]
TEMPLATES = KIT_ROOT / "templates"
sys.path.insert(0, str(KIT_ROOT))

from brxt_kit import KIT_VERSION  # noqa: E402
from brxt_kit.spec import AdapterSpec, load_adapter_spec  # noqa: E402
from brxt_kit.tools import build_server  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent))
import vendor_kit  # noqa: E402

UV_VERSION = "0.13.9"
DEFAULT_COPYRIGHT = "Baranzini Lab, University of California, San Francisco"
_TOP_KEYS = {"kit_spec", "version", "authors", "copyright_holder", "manifest", "adapter", "skills", "readme_intro"}
_MANIFEST_KEYS = {"name", "display_name", "description", "repository", "license", "env_vars"}
_SKILL_KEYS = {"name", "description", "form", "method_skill", "moved_to", "moved_body", "title"}
_PLACEHOLDER = re.compile(r"@@[A-Z_]+@@")
_SKILL_NAME = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")


class GeneratorError(SystemExit):
    def __init__(self, message: str) -> None:
        super().__init__(f"new_adapter: {message}")


def _reject_unknown(kind: str, data: dict[str, Any], allowed: set[str]) -> None:
    unknown = sorted(set(data) - allowed)
    if unknown:
        raise GeneratorError(f"{kind} has unknown keys: {', '.join(unknown)}")


def render(text: str, values: dict[str, str]) -> str:
    for key, value in values.items():
        text = text.replace(f"@@{key}@@", value)
    left = sorted(set(_PLACEHOLDER.findall(text)))
    if left:
        raise GeneratorError(f"template placeholders left unfilled: {', '.join(left)}")
    return text


def _toml_string(text: str) -> str:
    return json.dumps(text, ensure_ascii=False)


def _authors_toml(authors: list[dict[str, str]]) -> str:
    items = []
    for author in authors:
        fields = [f"name = {_toml_string(author['name'])}"]
        if author.get("email"):
            fields.append(f"email = {_toml_string(author['email'])}")
        items.append("{" + ", ".join(fields) + "}")
    return "[" + ", ".join(items) + "]"


def _tool_count(spec: AdapterSpec, version: str) -> list[str]:
    from fastmcp import Client

    server = build_server(spec, version=version)

    async def names() -> list[str]:
        async with Client(server) as client:
            return sorted(t.name for t in await client.list_tools())

    return asyncio.run(names())


def tools_list(spec: AdapterSpec) -> str:
    svc = spec.service
    if spec.kind == "tombstone":
        return f"- `{svc}_status`: shows why {spec.display_name} was retired and what to use instead. It calls no service."
    lines = [
        f"- `{svc}_status`: shows which targets are configured, their routes and which settings are present. "
        "It never shows a value and makes no network call.",
        f"- `{svc}_get`: reads from a target. It sends `GET`, or `POST` on routes declared as reads, and is "
        "marked read only, so Biorouter's Smart approval runs it without asking.",
    ]
    if spec.has_writes:
        lines.append(
            f"- `{svc}_write`: changes data on a declared write route. It is marked destructive, so Biorouter "
            "asks you before every call."
        )
    return "\n".join(lines)


def targets_table(spec: AdapterSpec) -> str:
    if not spec.targets:
        return "None. This release calls no service."
    rows = ["| Target | Base URL | Credential | Reads | Writes |", "| --- | --- | --- | --- | --- |"]
    for target in spec.targets:
        base = target.base_url or "(set by you)"
        if target.base_url_env:
            base += f", or `{target.base_url_env}`"
        if target.credential:
            credential = f"`{target.credential.env}` ({target.credential.scheme}"
            credential += ", optional)" if not target.credential.required else ")"
        else:
            credential = "none"
        reads = ", ".join(f"GET `{p}`" for p in target.get_routes)
        if target.read_post_routes:
            reads += (", " if reads else "") + ", ".join(f"POST `{p}`" for p in target.read_post_routes)
        writes = ", ".join(f"{r.method} `{r.pattern}`" for r in target.write_routes) or "none"
        rows.append(f"| `{target.name}` | `{base}` | {credential} | {reads or 'none'} | {writes} |")
    return "\n".join(rows)


def settings_table(env_vars: list[dict[str, Any]]) -> str:
    if not env_vars:
        return "None. This release reads no setting."
    rows = ["| Variable | Required | Secret | Purpose |", "| --- | --- | --- | --- |"]
    for item in env_vars:
        rows.append(
            f"| `{item['key']}` | {'yes' if item.get('required') else 'no'} | {'yes' if item.get('secret') else 'no'} "
            f"| {item.get('description', '')} |"
        )
    return "\n".join(rows)


def _skill_text(skill: dict[str, Any], spec: AdapterSpec) -> str:
    form = skill.get("form", "tools")
    name = skill["name"]
    if form == "moved":
        moved_to = skill.get("moved_to", "")
        body = skill.get("moved_body") or (
            f"This skill was bundled with {spec.display_name}. Its content now lives in "
            f"`{moved_to}`. Install `{moved_to}` from BAAM in Biorouter and use it instead."
            if moved_to
            else f"This skill was bundled with {spec.display_name}, which no longer uses it."
        )
        if spec.notice is not None:
            body += "\n\n" + spec.notice.sentence(spec.display_name)
        description = skill.get("description") or (
            f"Moved: use {moved_to} instead. This skill no longer holds instructions." if moved_to else "Moved. This skill no longer holds instructions."
        )
        return render(
            (TEMPLATES / "skills" / "moved.SKILL.md.tmpl").read_text(encoding="utf-8"),
            {"SKILL_NAME": name, "SKILL_DESCRIPTION_JSON": json.dumps(description), "MOVED_BODY": body},
        )
    method_skill = skill.get("method_skill")
    method_sentence = (
        f"For the method itself, use the BAAM skill `{method_skill}`." if method_skill else ""
    )
    example = next((e for t in spec.targets for e in t.examples if e.startswith("/")), "/")
    write_rule = (
        f"- `{spec.service}_write` changes data. Use it only for what the person asked for; Biorouter asks them "
        "to approve every call."
        if spec.has_writes
        else "- This extension is read only: it has no write tool."
    )
    return render(
        (TEMPLATES / "skills" / "tools.SKILL.md.tmpl").read_text(encoding="utf-8"),
        {
            "SKILL_NAME": name,
            "SKILL_DESCRIPTION_JSON": json.dumps(skill["description"]),
            "SKILL_TITLE": skill.get("title") or f"Calling {spec.display_name}",
            "DISPLAY_NAME": spec.display_name,
            "METHOD_SENTENCE": method_sentence,
            "TOOLS_LIST": tools_list(spec),
            "TARGETS_TABLE": targets_table(spec),
            "SERVICE": spec.service,
            "EXAMPLE_PATH": example,
            "WRITE_RULE": write_rule,
        },
    ).rstrip() + "\n"


def load_source(path: Path) -> dict[str, Any]:
    source = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(source, dict):
        raise GeneratorError("the spec must be one JSON object")
    _reject_unknown("spec", source, _TOP_KEYS)
    if source.get("kit_spec") != 1:
        raise GeneratorError("the spec must declare kit_spec 1")
    manifest = source.get("manifest") or {}
    _reject_unknown("manifest", manifest, _MANIFEST_KEYS)
    for key in ("name", "display_name", "description", "repository"):
        if not isinstance(manifest.get(key), str) or not manifest[key].strip():
            raise GeneratorError(f"manifest.{key} is required")
    if not re.fullmatch(r"\d+\.\d+\.\d+", str(source.get("version", ""))):
        raise GeneratorError("version must be X.Y.Z")
    for skill in source.get("skills") or []:
        _reject_unknown(f"skill {skill.get('name')}", skill, _SKILL_KEYS)
        if not _SKILL_NAME.fullmatch(skill.get("name", "")):
            raise GeneratorError(f"skill name {skill.get('name')!r} must be lower case words joined by hyphens")
        if skill.get("form", "tools") not in {"tools", "moved"}:
            raise GeneratorError(f"skill {skill['name']}: form is tools or moved")
        if skill.get("form", "tools") == "tools" and not skill.get("description"):
            raise GeneratorError(f"skill {skill['name']}: a tools skill needs a description")
    return source


def plan(source: dict[str, Any], previous: Optional[Path] = None) -> tuple[AdapterSpec, dict[str, str]]:
    """Return the validated spec and every file to write, as relative path to text."""

    manifest_in = source["manifest"]
    version = source["version"]
    adapter = dict(source.get("adapter") or {})
    for key in ("id", "display_name"):
        manifest_key = "name" if key == "id" else key
        if key in adapter and adapter[key] != manifest_in[manifest_key]:
            raise GeneratorError(f"adapter.{key} differs from manifest.{manifest_key}")
        adapter[key] = manifest_in[manifest_key]
    adapter.setdefault("kit_spec", 1)
    spec = load_adapter_spec(adapter)
    env_vars = list(manifest_in.get("env_vars") or [])
    skills = list(source.get("skills") or [])
    if spec.kind in {"sunset", "tombstone"}:
        bad = [s["name"] for s in skills if s.get("form", "tools") != "moved"]
        if bad:
            raise GeneratorError(f"a {spec.kind} release rewrites every bundled skill as a moved notice: {bad}")
    if spec.kind == "tombstone" and env_vars:
        raise GeneratorError("a tombstone declares no env_vars")
    if previous is not None:
        with zipfile.ZipFile(previous) as archive:
            old = sorted(
                n.split("/")[1] for n in archive.namelist() if n.startswith("skills/") and n.endswith("/SKILL.md")
            )
        missing = sorted(set(old) - {s["name"] for s in skills})
        if missing:
            raise GeneratorError(f"the previous bundle shipped skills {missing}; keep each one (as a moved notice if needed)")

    package = spec.package
    tools = _tool_count(spec, version)
    manifest = {
        "name": spec.id,
        "display_name": spec.display_name,
        "description": manifest_in["description"],
        "version": version,
        "entry_point": spec.id,
        "repository": manifest_in["repository"],
        "license": manifest_in.get("license", "Apache-2.0"),
        "tools_count": len(tools),
        "env_vars": env_vars,
        "skills": [
            {"name": s["name"], "description": s.get("description") or f"Moved: use {s.get('moved_to', 'its replacement')} instead."}
            for s in skills
        ],
    }
    notice_block = ""
    if spec.notice is not None:
        label = "Retired." if spec.kind == "tombstone" else "Sunset."
        notice_block = f"> **{label}** {spec.notice.sentence(spec.display_name)}\n\n"
    intro = source.get("readme_intro", "").strip()
    values = {
        "ID": spec.id,
        "PKG": package,
        "DISPLAY_NAME": spec.display_name,
        "SERVICE": spec.service,
        "VERSION": version,
        "DESCRIPTION": manifest_in["description"],
        "DESCRIPTION_TOML": _toml_string(manifest_in["description"]),
        "LICENSE": manifest["license"],
        "AUTHORS_TOML": _authors_toml(source.get("authors") or [{"name": "Baranzini Lab"}]),
        "LOG_ENV": spec.log_level_env or "",
        "KIT_VERSION": KIT_VERSION,
        "UV_VERSION": UV_VERSION,
        "YEAR": str(datetime.date.today().year),
        "COPYRIGHT_HOLDER": source.get("copyright_holder") or DEFAULT_COPYRIGHT,
        "NOTICE_BLOCK": notice_block,
        "INTRO": f"{intro}\n\n" if intro else "",
        "TOOLS_LIST": tools_list(spec),
        "TARGETS_TABLE": targets_table(spec),
        "SETTINGS_TABLE": settings_table(env_vars),
    }
    files: dict[str, str] = {}
    template_root = TEMPLATES / "adapter"
    override_root = TEMPLATES / spec.kind
    for path in sorted(template_root.rglob("*")):
        if not path.is_file() or "__pycache__" in path.parts:
            continue
        override = override_root / path.relative_to(template_root)
        if spec.kind != "adapter" and override.is_file():
            path = override
        relative = override.relative_to(override_root).as_posix().replace("__PKG__", package)
        if relative.endswith(".tmpl"):
            files[relative[: -len(".tmpl")]] = render(path.read_text(encoding="utf-8"), values)
        else:
            files[relative] = path.read_text(encoding="utf-8")
    files["manifest.json"] = json.dumps(manifest, indent=2, ensure_ascii=False) + "\n"
    files[f"src/{package}/adapter.json"] = json.dumps(adapter, indent=2, ensure_ascii=False) + "\n"
    for skill in skills:
        files[f"skills/{skill['name']}/SKILL.md"] = _skill_text(skill, spec)
    return spec, files


def write(out: Path, files: dict[str, str], *, force: bool) -> list[str]:
    conflicts = []
    for relative, text in files.items():
        target = out / relative
        if target.exists() and target.read_text(encoding="utf-8", errors="replace") != text and not force:
            conflicts.append(relative)
    if conflicts:
        raise GeneratorError("these files exist and differ (pass --force to overwrite): " + ", ".join(conflicts))
    for relative, text in files.items():
        target = out / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
    vendor_kit.vendor(out)
    written = set(files) | set(vendor_kit.planned_files(out))
    package = json.loads(files["manifest.json"])["name"].replace("-", "_")
    written.add(f"src/{package}/_kit/vendored.json")
    others = []
    for path in sorted(out.rglob("*")):
        relative = path.relative_to(out).as_posix()
        if path.is_dir() or relative.startswith((".git/", ".venv/", "dist/", "previous/")) or "__pycache__" in path.parts:
            continue
        if relative not in written and relative != "uv.lock":
            others.append(relative)
    return others


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("spec", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--force", action="store_true", help="overwrite kit owned files that differ")
    parser.add_argument("--lock", action="store_true", help="run uv lock in OUT_DIR afterwards")
    parser.add_argument("--previous", type=Path, help="the previous release bundle, to keep its skill paths")
    args = parser.parse_args()
    source = load_source(args.spec)
    spec, files = plan(source, args.previous)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    others = write(out, files, force=args.force)
    print(f"Rendered {spec.kind} {spec.id} {source['version']} into {out} ({len(files)} files plus the vendored kit).")
    if others:
        print("Files the kit did not write (review them; remove what the kit replaced):")
        for relative in others:
            print(f"  {relative}")
    if args.lock:
        subprocess.run(["uv", "lock"], cwd=out, check=True)
    else:
        print("Next: run `uv lock` in the adapter, then its tests and scripts/build_brxt.py.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
