"""Check the release rules, then build the extension bundle.

Writes dist/<id>.brxt, dist/<id>-X.Y.Z.brxt (the same bytes) and dist/SHA256SUMS.

    uv run python scripts/build_brxt.py                  # check and build
    uv run python scripts/build_brxt.py --check          # check only
    uv run python scripts/build_brxt.py --previous old.brxt

It refuses unless:

* manifest.json, pyproject.toml, src/<package>/__init__.py and the README's
  "Current version" line carry the same X.Y.Z;
* manifest.json, README.md, LICENSE, NOTICE, pyproject.toml, uv.lock and
  src/<package>/ are present, and uv.lock pins fastmcp 4 and this version of the
  project;
* every environment name adapter.json reads is declared in manifest.json, every
  credential is declared secret, no secret has a default, and a tombstone
  declares none at all;
* the vendored kit files match the hashes in _kit/vendored.json;
* the bundled skills are exactly the ones the manifest declares, and every
  skill path in --previous is still present (an in place update never deletes a
  file, so a dropped skill would keep its old text forever);
* the server starts in process and lists exactly manifest.tools_count tools;
* with --previous, the name and repository are unchanged and the version moved
  forward.

The bundle is built from an explicit allowlist with fixed timestamps, so the
same tree always gives the same bytes.
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import importlib
import json
import re
import sys
import tomllib
import zipfile
from pathlib import Path
from typing import Any, Optional

ROOT = Path(__file__).resolve().parents[1]
REQUIRED_FILES = ("manifest.json", "README.md", "LICENSE", "NOTICE", "pyproject.toml", "uv.lock")
NAME = re.compile(r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*")
VERSION = re.compile(r"\d+\.\d+\.\d+")
ENV = re.compile(r"[A-Z][A-Z0-9_]{0,127}")
README_VERSION = re.compile(r"Current version: \*\*(\d+\.\d+\.\d+)\*\*")
INIT_VERSION = re.compile(r'(?m)^__version__ = "([^"]+)"')
ZIP_TIME = (2020, 1, 1, 0, 0, 0)


class Refused(SystemExit):
    def __init__(self, message: str) -> None:
        super().__init__(f"build_brxt: refused: {message}")
        self.message = message


def _version_tuple(text: str) -> tuple[int, ...]:
    return tuple(int(part) for part in text.split("."))


def _frontmatter_name(text: str) -> Optional[str]:
    match = re.match(r"---\r?\n(.*?)\r?\n---\r?\n", text, re.S)
    if not match:
        return None
    for line in match.group(1).splitlines():
        if line.startswith("name:"):
            return line.split(":", 1)[1].strip().strip("'\"")
    return None


def _count_tools(root: Path, package: str) -> list[str]:
    sys.path.insert(0, str(root / "src"))
    try:
        for cached in [m for m in sys.modules if m == package or m.startswith(package + ".")]:
            del sys.modules[cached]
        server = importlib.import_module(f"{package}.server")
        mcp = server.create_server()
        from fastmcp import Client

        async def names() -> list[str]:
            async with Client(mcp) as client:
                return sorted(t.name for t in await client.list_tools())

        return asyncio.run(names())
    finally:
        sys.path.remove(str(root / "src"))


def check(root: Path, previous: Optional[Path] = None, *, count_tools: bool = True) -> tuple[dict[str, Any], list[Path]]:
    """Run every rule and return the manifest and the files to bundle."""

    for filename in REQUIRED_FILES:
        if not (root / filename).is_file():
            raise Refused(f"{filename} is missing.")
    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    project = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8")).get("project", {})
    for key in ("name", "display_name", "description", "version", "entry_point", "repository"):
        if not isinstance(manifest.get(key), str) or not manifest[key].strip():
            raise Refused(f"manifest.json needs {key}.")
    name, version = manifest["name"], manifest["version"]
    if not NAME.fullmatch(name):
        raise Refused(f"manifest name {name!r} is not a valid extension name.")
    if not VERSION.fullmatch(version):
        raise Refused(f"manifest version {version!r} is not X.Y.Z.")
    package = name.replace("-", "_")
    source = root / "src" / package
    if not source.is_dir():
        raise Refused(f"src/{package}/ is missing.")

    init_text = (source / "__init__.py").read_text(encoding="utf-8") if (source / "__init__.py").is_file() else ""
    init_match = INIT_VERSION.search(init_text)
    readme_match = README_VERSION.search((root / "README.md").read_text(encoding="utf-8"))
    versions = {
        "manifest.json": version,
        "pyproject.toml": project.get("version"),
        f"src/{package}/__init__.py": init_match.group(1) if init_match else None,
        "README.md (Current version line)": readme_match.group(1) if readme_match else None,
    }
    if len(set(versions.values())) != 1:
        raise Refused(f"the versions disagree: {versions}. Run scripts/version.py.")
    if project.get("name") != name:
        raise Refused("pyproject project.name must equal the manifest name.")
    if manifest["entry_point"] not in (project.get("scripts") or {}):
        raise Refused("the manifest entry_point is not a pyproject script.")
    dependencies = project.get("dependencies") or []
    fastmcp = [d for d in dependencies if re.match(r"fastmcp\b", d)]
    if not fastmcp or not any(">=4" in d and "<5" in d for d in fastmcp):
        raise Refused("pyproject must depend on fastmcp>=4.0,<5.")

    lock = tomllib.loads((root / "uv.lock").read_text(encoding="utf-8"))
    packages = lock.get("package") or []
    locked_fastmcp = [p.get("version", "") for p in packages if p.get("name") == "fastmcp"]
    if not locked_fastmcp or not all(v.split(".")[0] == "4" for v in locked_fastmcp):
        raise Refused(f"uv.lock must pin fastmcp 4 (found {locked_fastmcp or 'none'}).")
    own = [p for p in packages if p.get("name") == name]
    if not own or own[0].get("version") != version:
        raise Refused("uv.lock was made for another version of this project. Run uv lock.")

    env_vars = manifest.get("env_vars")
    if not isinstance(env_vars, list):
        raise Refused("manifest env_vars must be a list.")
    declared: dict[str, dict[str, Any]] = {}
    for item in env_vars:
        key = item.get("key", "") if isinstance(item, dict) else ""
        if not ENV.fullmatch(key):
            raise Refused(f"env var {key!r} is not an upper case name.")
        if key in declared:
            raise Refused(f"env var {key} is declared twice.")
        if item.get("secret") and item.get("default"):
            raise Refused(f"secret {key} must not have a default.")
        if item.get("secret") and item.get("auto_propagate"):
            raise Refused(f"secret {key} must not be auto propagated from the machine.")
        declared[key] = item

    sys.path.insert(0, str(root / "src"))
    try:
        for cached in [m for m in sys.modules if m == package or m.startswith(package + ".")]:
            del sys.modules[cached]
        spec_module = importlib.import_module(f"{package}._kit.spec")
        kit_module = importlib.import_module(f"{package}._kit")
        spec = spec_module.load_adapter_spec((source / "adapter.json").read_text(encoding="utf-8"))
    finally:
        sys.path.remove(str(root / "src"))
    if spec.id != name or spec.display_name != manifest["display_name"]:
        raise Refused("adapter.json id and display_name must equal the manifest's.")
    credentials = set(spec.credential_envs())
    for env in spec.environment_names():
        if env not in declared:
            raise Refused(f"adapter.json reads {env}, which manifest.json does not declare.")
    for env in credentials:
        if not declared[env].get("secret"):
            raise Refused(f"{env} is a credential and must be declared secret.")
    secrets = set(spec.secret_envs())
    for key, item in declared.items():
        if item.get("secret") and key not in secrets:
            raise Refused(f"secret {key} is neither a target credential nor an extra_secrets entry in adapter.json.")
    for env in secrets - credentials:
        if env not in declared or not declared[env].get("secret"):
            raise Refused(f"extra secret {env} must be declared secret in manifest.json.")
    if spec.log_level_env and spec.kind != "tombstone" and spec.log_level_env not in declared:
        raise Refused(f"log_level_env {spec.log_level_env} is not declared in manifest.json.")
    if spec.kind == "tombstone" and env_vars:
        raise Refused("a tombstone declares no env_vars: it reads no credential and no setting.")

    vendored_record = source / "_kit" / "vendored.json"
    if not vendored_record.is_file():
        raise Refused("src/<package>/_kit/vendored.json is missing. Run the kit's bin/vendor_kit.py.")
    record = json.loads(vendored_record.read_text(encoding="utf-8"))
    if record.get("kit_version") != kit_module.KIT_VERSION:
        raise Refused("the vendored kit version does not match _kit/vendored.json.")
    for relative, digest in sorted((record.get("files") or {}).items()):
        path = root / relative
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise Refused(f"vendored kit file {relative} was changed or removed. Vendor the kit again.")

    skills_dir = root / "skills"
    found = sorted(p.parent.name for p in skills_dir.glob("*/SKILL.md")) if skills_dir.is_dir() else []
    declared_skills = sorted(s["name"] for s in manifest.get("skills") or [])
    if found != declared_skills:
        raise Refused(f"bundled skills {found} differ from the manifest's {declared_skills}.")
    for skill in found:
        if _frontmatter_name((skills_dir / skill / "SKILL.md").read_text(encoding="utf-8")) != skill:
            raise Refused(f"skills/{skill}/SKILL.md must name itself {skill} in its frontmatter.")

    if previous is not None:
        with zipfile.ZipFile(previous) as archive:
            previous_names = set(archive.namelist())
            previous_manifest = json.loads(archive.read("manifest.json"))
        if previous_manifest.get("name") != name:
            raise Refused("the manifest name changed since the previous release.")
        if previous_manifest.get("repository") != manifest["repository"]:
            raise Refused("the manifest repository changed since the previous release.")
        previous_version = str(previous_manifest.get("version", "0.0.0"))
        previous_core = re.match(r"\d+\.\d+\.\d+", previous_version)
        if previous_core and _version_tuple(version) <= _version_tuple(previous_core.group(0)):
            raise Refused(f"version {version} does not move forward from {previous_version}.")
        previous_skills = {n for n in previous_names if n.startswith("skills/") and n.endswith("/SKILL.md")}
        current_skills = {f"skills/{s}/SKILL.md" for s in found}
        missing = sorted(previous_skills - current_skills)
        if missing:
            raise Refused(
                f"the previous release bundled {missing}. Keep every bundled skill path "
                "(rewrite it as a moved notice if it is no longer needed)."
            )

    if count_tools:
        tools = _count_tools(root, package)
        if manifest.get("tools_count") != len(tools):
            raise Refused(f"manifest tools_count is {manifest.get('tools_count')}, the server lists {len(tools)}: {tools}.")
        missing_kit_tools = sorted(set(spec.tool_names()) - set(tools))
        if missing_kit_tools:
            raise Refused(f"the server does not list the kit tools {missing_kit_tools}.")

    files = [root / f for f in REQUIRED_FILES]
    files += sorted(p for p in source.rglob("*.py") if "__pycache__" not in p.parts)
    files += [source / "adapter.json", vendored_record]
    files += [skills_dir / s / "SKILL.md" for s in found]
    for path in files:
        if path.is_symlink():
            raise Refused(f"{path.relative_to(root)} is a symbolic link.")
        relative = path.relative_to(root).as_posix()
        if any(part.startswith(".") for part in Path(relative).parts):
            raise Refused(f"{relative} is a hidden path.")
    return manifest, files


def build(root: Path, out: Path, files: list[Path], manifest: dict[str, Any]) -> tuple[Path, Path, str]:
    name, version = manifest["name"], manifest["version"]
    out.mkdir(parents=True, exist_ok=True)
    stable = out / f"{name}.brxt"
    versioned = out / f"{name}-{version}.brxt"
    with zipfile.ZipFile(stable, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(files, key=lambda p: p.relative_to(root).as_posix()):
            info = zipfile.ZipInfo(path.relative_to(root).as_posix(), date_time=ZIP_TIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            archive.writestr(info, path.read_bytes())
    data = stable.read_bytes()
    versioned.write_bytes(data)
    digest = hashlib.sha256(data).hexdigest()
    (out / "SHA256SUMS").write_text(f"{digest}  {stable.name}\n{digest}  {versioned.name}\n", encoding="utf-8")
    return stable, versioned, digest


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--previous", type=Path, help="the previous release's <id>.brxt, for the skill path and identity checks")
    parser.add_argument("--out", type=Path, default=ROOT / "dist")
    parser.add_argument("--check", action="store_true", help="run the checks and write nothing")
    args = parser.parse_args(argv)
    manifest, files = check(ROOT, args.previous)
    if args.check:
        print(f"{manifest['name']} {manifest['version']}: every release check passed ({len(files)} files).")
        return 0
    stable, versioned, digest = build(ROOT, args.out, files, manifest)
    print(f"{stable.name} and {versioned.name}: {len(files)} files, {stable.stat().st_size} bytes, SHA-256 {digest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
