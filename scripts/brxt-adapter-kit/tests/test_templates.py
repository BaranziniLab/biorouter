"""The generator, the vendoring check, version.py and build_brxt.py's release rules.

These tests render into temporary folders and write a minimal uv.lock (fastmcp 4
plus the project at its version) instead of running `uv lock`, so they need no
network. tests/test_integration.py runs the real `uv lock`, `uv sync` and
`uv run` path when BRXT_KIT_INTEGRATION=1.
"""

import hashlib
import importlib.util
import json
import os
import re
import shutil
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

from support import load_source
import new_adapter
import vendor_kit


def fake_lock(root: Path, name: str, version: str, fastmcp: str = "4.2.3") -> None:
    (root / "uv.lock").write_text(
        "version = 1\nrevision = 5\nrequires-python = \">=3.11\"\n\n"
        f"[[package]]\nname = \"fastmcp\"\nversion = \"{fastmcp}\"\nsource = {{ registry = \"https://pypi.org/simple\" }}\n\n"
        f"[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = {{ editable = \".\" }}\n",
        encoding="utf-8",
    )


def load_script(root: Path, name: str):
    spec = importlib.util.spec_from_file_location(f"{name}_{hashlib.sha1(str(root).encode()).hexdigest()[:8]}", root / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Rendered(unittest.TestCase):
    fixture = "sample-adapter.json"

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="brxt-render-"))
        self.source = load_source(self.fixture)
        self.root = self.tmp / self.source["manifest"]["name"]
        spec, files = new_adapter.plan(self.source)
        new_adapter.write(self.root, files, force=False)
        self.spec = spec
        self.manifest = json.loads((self.root / "manifest.json").read_text(encoding="utf-8"))
        fake_lock(self.root, self.manifest["name"], self.manifest["version"])
        self.build = load_script(self.root, "build_brxt")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)
        for name in [m for m in sys.modules if m.split(".")[0] in {"sampleagent", "oldsearchagent", "retiredagent", "singleagent"}]:
            del sys.modules[name]

    def check(self, previous=None):
        return self.build.check(self.root, previous)

    def assertRefused(self, fragment, previous=None):
        with self.assertRaises(SystemExit) as caught:
            self.check(previous)
        self.assertIn(fragment, str(caught.exception))

    def edit(self, relative, old, new):
        path = self.root / relative
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1), encoding="utf-8")

    def edit_manifest(self, change):
        manifest = json.loads((self.root / "manifest.json").read_text(encoding="utf-8"))
        change(manifest)
        (self.root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


class BuildRules(Rendered):
    def test_a_rendered_adapter_builds_reproducibly(self):
        manifest, files = self.check()
        out = self.tmp / "dist"
        stable, versioned, digest = self.build.build(self.root, out, files, manifest)
        self.assertEqual(stable.name, "sampleagent.brxt")
        self.assertEqual(versioned.name, "sampleagent-0.2.0.brxt")
        self.assertEqual(stable.read_bytes(), versioned.read_bytes())
        self.assertEqual(
            (out / "SHA256SUMS").read_text(encoding="utf-8"),
            f"{digest}  sampleagent.brxt\n{digest}  sampleagent-0.2.0.brxt\n",
        )
        first = stable.read_bytes()
        manifest, files = self.check()
        self.build.build(self.root, out, files, manifest)
        self.assertEqual(stable.read_bytes(), first)
        with zipfile.ZipFile(stable) as archive:
            names = set(archive.namelist())
        for required in ("manifest.json", "README.md", "LICENSE", "NOTICE", "pyproject.toml", "uv.lock",
                         "src/sampleagent/server.py", "src/sampleagent/adapter.json", "src/sampleagent/_kit/safe_http.py",
                         "src/sampleagent/_kit/vendored.json", "skills/sample-tools/SKILL.md"):
            self.assertIn(required, names)
        self.assertFalse([n for n in names if n.startswith(("tests/", "scripts/", ".github/", ".")) or "__pycache__" in n])

    def test_the_bundle_leaves_out_stray_files(self):
        (self.root / "src" / "sampleagent" / "secrets.env").write_text("TOKEN=private", encoding="utf-8")
        (self.root / "src" / "sampleagent" / "cache.sqlite3").write_text("private", encoding="utf-8")
        (self.root / "skills" / "sample-tools" / "notes.csv").write_text("private", encoding="utf-8")
        manifest, files = self.check()
        names = {f.relative_to(self.root).as_posix() for f in files}
        self.assertFalse([n for n in names if n.endswith((".env", ".sqlite3", ".csv"))])

    def test_versions_must_agree(self):
        self.edit("README.md", "Current version: **0.2.0**", "Current version: **0.1.9**")
        self.assertRefused("versions disagree")

    def test_versions_must_agree_in_init(self):
        self.edit("src/sampleagent/__init__.py", '__version__ = "0.2.0"', '__version__ = "0.2.1"')
        self.assertRefused("versions disagree")

    def test_required_files(self):
        for name in ("NOTICE", "LICENSE", "uv.lock", "README.md"):
            with self.subTest(name=name):
                moved = self.root / name
                backup = self.tmp / f"{name}.bak"
                shutil.move(moved, backup)
                try:
                    self.assertRefused(f"{name} is missing")
                finally:
                    shutil.move(backup, moved)

    def test_lock_must_pin_fastmcp_4_and_this_version(self):
        fake_lock(self.root, "sampleagent", "0.2.0", fastmcp="3.4.2")
        self.assertRefused("fastmcp 4")
        fake_lock(self.root, "sampleagent", "0.1.0")
        self.assertRefused("another version")

    def test_pyproject_must_bound_fastmcp(self):
        self.edit("pyproject.toml", '"fastmcp>=4.0,<5"', '"fastmcp>=2.11.2"')
        self.assertRefused("fastmcp>=4.0,<5")

    def test_secrets_have_no_default_and_are_not_propagated(self):
        self.edit_manifest(lambda m: m["env_vars"][0].update(default="abc"))
        self.assertRefused("must not have a default")

    def test_secret_auto_propagation_refused(self):
        self.edit_manifest(lambda m: m["env_vars"][0].update(auto_propagate=True))
        self.assertRefused("auto propagated")

    def test_every_environment_name_is_declared(self):
        self.edit_manifest(lambda m: m.update(env_vars=[e for e in m["env_vars"] if e["key"] != "GAMMA_KEY"]))
        self.assertRefused("GAMMA_KEY")

    def test_credentials_are_declared_secret(self):
        self.edit_manifest(lambda m: [e.update(secret=False) for e in m["env_vars"] if e["key"] == "ALPHA_TOKEN"])
        self.assertRefused("must be declared secret")

    def test_a_secret_must_be_bound(self):
        self.edit_manifest(lambda m: m["env_vars"].append({"key": "STRAY_SECRET", "required": False, "auto_propagate": False, "secret": True}))
        self.assertRefused("STRAY_SECRET")

    def test_vendored_kit_must_be_unchanged(self):
        self.edit("src/sampleagent/_kit/safe_http.py", "MAX_REDIRECTS = 3", "MAX_REDIRECTS = 30")
        self.assertRefused("was changed or removed")
        self.assertTrue(vendor_kit.check(self.root))

    def test_skills_match_the_manifest(self):
        (self.root / "skills" / "undeclared").mkdir()
        (self.root / "skills" / "undeclared" / "SKILL.md").write_text("---\nname: undeclared\ndescription: x\n---\n", encoding="utf-8")
        self.assertRefused("differ from the manifest")

    def test_skill_frontmatter_names_itself(self):
        self.edit("skills/sample-tools/SKILL.md", "name: sample-tools", "name: other-name")
        self.assertRefused("frontmatter")

    def test_tools_count_must_match_the_server(self):
        self.edit_manifest(lambda m: m.update(tools_count=6))
        self.assertRefused("tools_count")

    def test_symbolic_links_are_refused(self):
        if os.name == "nt":
            self.skipTest("symbolic links need privileges on Windows")
        (self.root / "src" / "sampleagent" / "linked.py").symlink_to(self.root / "src" / "sampleagent" / "cli.py")
        self.assertRefused("symbolic link")

    def make_previous(self, *, skills=("sample-tools",), version="0.1.0", repository=None, name="sampleagent"):
        path = self.tmp / "previous.brxt"
        manifest = {"name": name, "version": version, "repository": repository or self.manifest["repository"]}
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr("manifest.json", json.dumps(manifest))
            for skill in skills:
                archive.writestr(f"skills/{skill}/SKILL.md", "---\nname: x\n---\n")
        return path

    def test_previous_release_skill_paths_are_kept(self):
        self.check(self.make_previous())
        with self.assertRaises(SystemExit) as caught:
            self.check(self.make_previous(skills=("sample-tools", "dropped-skill")))
        self.assertIn("skills/dropped-skill/SKILL.md", str(caught.exception))

    def test_identity_strings_never_move(self):
        self.assertRefused("repository changed", self.make_previous(repository="https://github.com/Someone/Else"))
        self.assertRefused("name changed", self.make_previous(name="otheragent"))

    def test_version_moves_forward(self):
        self.assertRefused("does not move forward", self.make_previous(version="0.2.0"))
        self.assertRefused("does not move forward", self.make_previous(version="0.3.0"))
        self.check(self.make_previous(version="0.1.0-brxt"))

    def test_command_line(self):
        build_main = self.build.main
        self.build.ROOT = self.root
        self.assertEqual(build_main(["--check"]), 0)
        self.assertEqual(build_main(["--out", str(self.tmp / "cli-dist")]), 0)
        self.assertTrue((self.tmp / "cli-dist" / "SHA256SUMS").is_file())


class VersionScript(Rendered):
    def test_moves_every_version_together(self):
        version = load_script(self.root, "version")
        old = version.bump(self.root, "0.3.0")
        self.assertEqual(old, "0.2.0")
        fake_lock(self.root, "sampleagent", "0.3.0")
        manifest, _ = self.check()
        self.assertEqual(manifest["version"], "0.3.0")
        self.assertIn('version = "0.3.0"', (self.root / "pyproject.toml").read_text(encoding="utf-8"))
        self.assertIn("Current version: **0.3.0**", (self.root / "README.md").read_text(encoding="utf-8"))

    def test_refuses_to_move_backwards_or_malformed(self):
        version = load_script(self.root, "version")
        for value in ("0.2.0", "0.1.9", "1.0", "v0.3.0"):
            with self.subTest(value=value), self.assertRaises(SystemExit):
                version.bump(self.root, value)


class Vendoring(Rendered):
    def test_vendored_copy_matches_the_kit(self):
        self.assertEqual(vendor_kit.check(self.root), [])
        record = json.loads((self.root / "src" / "sampleagent" / "_kit" / "vendored.json").read_text(encoding="utf-8"))
        self.assertIn("src/sampleagent/_kit/safe_http.py", record["files"])
        self.assertIn("tests/kit_testing/leak_suite.py", record["files"])

    def test_vendoring_again_removes_stale_kit_files(self):
        (self.root / "src" / "sampleagent" / "_kit" / "old_module.py").write_text("x = 1\n", encoding="utf-8")
        vendor_kit.vendor(self.root)
        self.assertFalse((self.root / "src" / "sampleagent" / "_kit" / "old_module.py").exists())
        self.assertEqual(vendor_kit.check(self.root), [])


class SunsetBuild(Rendered):
    fixture = "sunset-adapter.json"

    def test_sunset_builds_with_moved_skills(self):
        manifest, files = self.check()
        self.assertEqual(manifest["tools_count"], 2)
        skill = (self.root / "skills" / "old-search-evidence" / "SKILL.md").read_text(encoding="utf-8")
        self.assertIn("name: old-search-evidence", skill)
        self.assertIn("literatureagent", skill)
        self.assertIn("Sunset.", (self.root / "README.md").read_text(encoding="utf-8"))


class TombstoneBuild(Rendered):
    fixture = "tombstone-adapter.json"

    def test_tombstone_builds_and_declares_nothing(self):
        manifest, files = self.check()
        self.assertEqual(manifest["env_vars"], [])
        self.assertEqual(manifest["tools_count"], 1)
        readme = (self.root / "README.md").read_text(encoding="utf-8")
        self.assertIn("Retired.", readme)
        self.assertIn("holds no", readme)
        self.assertNotIn("Reads use", readme)

    def test_tombstone_with_a_setting_is_refused(self):
        self.edit_manifest(lambda m: m["env_vars"].append({"key": "RETIRED_LOG_LEVEL", "secret": False}))
        self.assertRefused("declares no env_vars")


class GeneratorRules(unittest.TestCase):
    def test_sunset_and_tombstone_skills_must_be_moved_notices(self):
        source = load_source("sunset-adapter.json")
        source["skills"] = [{"name": "old-search-evidence", "description": "still here", "form": "tools"}]
        with self.assertRaises(SystemExit):
            new_adapter.plan(source)

    def test_a_dropped_previous_skill_is_refused(self):
        tmp = Path(tempfile.mkdtemp(prefix="brxt-gen-"))
        try:
            previous = tmp / "old.brxt"
            with zipfile.ZipFile(previous, "w") as archive:
                archive.writestr("skills/sample-tools/SKILL.md", "x")
                archive.writestr("skills/legacy-skill/SKILL.md", "x")
            with self.assertRaises(SystemExit) as caught:
                new_adapter.plan(load_source("sample-adapter.json"), previous)
            self.assertIn("legacy-skill", str(caught.exception))
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    def test_unknown_keys_are_refused(self):
        tmp = Path(tempfile.mkdtemp(prefix="brxt-gen-"))
        try:
            source = load_source("single-adapter.json")
            source["manifest"]["extra"] = True
            path = tmp / "spec.json"
            path.write_text(json.dumps(source), encoding="utf-8")
            with self.assertRaises(SystemExit):
                new_adapter.load_source(path)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    def test_existing_files_are_not_overwritten_without_force(self):
        tmp = Path(tempfile.mkdtemp(prefix="brxt-gen-"))
        try:
            source = load_source("single-adapter.json")
            _, files = new_adapter.plan(source)
            new_adapter.write(tmp, files, force=False)
            (tmp / "README.md").write_text("hand edited\n", encoding="utf-8")
            (tmp / "scripts" / "build_brxt.sh").write_text("#!/bin/sh\n", encoding="utf-8")
            with self.assertRaises(SystemExit):
                new_adapter.write(tmp, files, force=False)
            others = new_adapter.write(tmp, files, force=True)
            self.assertIn("scripts/build_brxt.sh", others)
            self.assertNotEqual((tmp / "README.md").read_text(encoding="utf-8"), "hand edited\n")
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    def test_rendered_files_hold_no_placeholder(self):
        for fixture in ("sample-adapter.json", "single-adapter.json", "sunset-adapter.json", "tombstone-adapter.json"):
            _, files = new_adapter.plan(load_source(fixture))
            for relative, text in files.items():
                self.assertNotRegex(text, r"@@[A-Z_]+@@", f"{fixture}: {relative}")

    def test_rendered_workflows_pin_every_action_by_commit(self):
        # An action named by a tag runs whatever the tag points at on the day the job
        # starts, and the release workflow holds contents: write. Every action is a full
        # commit SHA followed by the release it carries.
        pinned = re.compile(r"^\s*-?\s*uses:\s*[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[0-9a-f]{40} # v\d+\.\d+\.\d+$")
        for fixture in ("sample-adapter.json", "single-adapter.json", "sunset-adapter.json", "tombstone-adapter.json"):
            _, files = new_adapter.plan(load_source(fixture))
            workflows = {relative: text for relative, text in files.items() if relative.startswith(".github/workflows/")}
            self.assertEqual(sorted(workflows), [".github/workflows/ci.yml", ".github/workflows/release.yml"], fixture)
            for relative, text in workflows.items():
                uses = [line for line in text.splitlines() if re.match(r"^\s*-?\s*uses:", line)]
                self.assertTrue(uses, f"{fixture}: {relative}")
                for line in uses:
                    self.assertRegex(line, pinned, f"{fixture}: {relative}")


if __name__ == "__main__":
    unittest.main()
