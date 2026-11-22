"""Shared helpers for the kit's own tests.

Run the suite from scripts/brxt-adapter-kit/:

    uv run python -m unittest discover -s tests -v
"""

from __future__ import annotations

import json
import logging
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any, Mapping, Optional

KIT_ROOT = Path(__file__).resolve().parents[1]
FIXTURES = Path(__file__).resolve().parent / "fixtures"
for entry in (str(KIT_ROOT), str(KIT_ROOT / "bin")):
    if entry not in sys.path:
        sys.path.insert(0, entry)

# FastMCP logs every refused call; the tests make thousands. Import it first,
# because it sets its own logger level when it is imported.
import fastmcp  # noqa: E402,F401

logging.getLogger("fastmcp").setLevel(logging.CRITICAL)

from brxt_kit import safe_http  # noqa: E402
from brxt_kit_testing.echo_server import EchoFleet  # noqa: E402


def load_source(name: str) -> dict[str, Any]:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def adapter_data(name: str) -> dict[str, Any]:
    """The adapter.json content of a fixture, as new_adapter.py would write it."""

    source = load_source(name)
    data = dict(source["adapter"])
    data["id"] = source["manifest"]["name"]
    data["display_name"] = source["manifest"]["display_name"]
    return data


SECRETS = {
    "A_TOKEN": "alpha-token-7f3c9a1e5b",
    "B_KEY": "beta-key-2d8e4f6a0c",
    "C_KEY": "gamma-key-9b1d3f5e7a",
    "D_KEY": "delta-key-4c6e8a0b2d",
    "E_PASSWORD": "epsilon-pass-1a3c5e7b9d",
}


class EchoTestCase(unittest.TestCase):
    """Starts four echo servers once per class: a, b (on localhost), c and an outsider."""

    fleet: EchoFleet
    workdir: Path

    @classmethod
    def setUpClass(cls) -> None:
        cls.workdir = Path(tempfile.mkdtemp(prefix="brxt-kit-test-"))
        cls.fleet = EchoFleet(cls.workdir, ["a", "b", "c", "out"], hosts={"b": "localhost"})

    @classmethod
    def tearDownClass(cls) -> None:
        cls.fleet.stop()
        shutil.rmtree(cls.workdir, ignore_errors=True)

    def setUp(self) -> None:
        self.fleet.log.clear()

    def origin(self, name: str) -> str:
        return self.fleet[name].origin

    def client(self, targets: list[safe_http.Target], env: Optional[Mapping[str, str]] = None) -> safe_http.SafeHttpClient:
        return safe_http.SafeHttpClient(
            targets,
            user_agent="brxt-kit-tests/1",
            environ=dict(SECRETS if env is None else env),
            verify=self.fleet.ca.client_context(),
            trust_env=False,
        )

    def sent(self, server: Optional[str] = None) -> list[Any]:
        return self.fleet.log.at(server) if server else self.fleet.log.snapshot()

    def assertNothingSent(self) -> None:
        self.assertEqual([(r.server, r.method, r.path) for r in self.sent()], [])

    def assertRefused(self, call: Any, *args: Any, **kwargs: Any) -> safe_http.KitError:
        self.fleet.log.clear()
        with self.assertRaises(safe_http.KitError) as caught:
            call(*args, **kwargs)
        self.assertNothingSent()
        return caught.exception
