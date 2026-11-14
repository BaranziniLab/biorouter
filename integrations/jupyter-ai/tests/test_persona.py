import asyncio
import os
import sys
import tomllib
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

from jupyter_ai_acp_client.base_acp_persona import BaseAcpPersona
from jupyter_ai_persona_manager import PersonaRequirementsUnmet

from jupyter_ai_biorouter.persona import (
    BIOROUTER_EXECUTABLE,
    BioRouterAcpPersona,
    resolve_biorouter_executable,
)


class BioRouterPersonaTest(unittest.TestCase):
    def test_defaults(self):
        persona = object.__new__(BioRouterAcpPersona)
        defaults = persona.defaults

        self.assertEqual(defaults.name, "Biorouter")
        self.assertTrue(Path(defaults.avatar_path).is_file())

    def test_explicit_executable(self):
        with patch.dict(
            os.environ, {"BIOROUTER_EXECUTABLE": sys.executable}, clear=False
        ):
            self.assertEqual(
                resolve_biorouter_executable(), str(Path(sys.executable).resolve())
            )

    def test_invalid_explicit_executable(self):
        with patch.dict(
            os.environ,
            {"BIOROUTER_EXECUTABLE": "/missing/biorouter"},
            clear=False,
        ):
            with self.assertRaises(PersonaRequirementsUnmet):
                resolve_biorouter_executable()

    def test_launches_biorouter_acp(self):
        with patch.object(BaseAcpPersona, "__init__", return_value=None) as base_init:
            BioRouterAcpPersona(example="value")

        base_init.assert_called_once_with(
            executable=[BIOROUTER_EXECUTABLE, "acp"],
            example="value",
        )

    def test_child_identifies_jupyter_without_changing_parent_environment(self):
        persona = object.__new__(BioRouterAcpPersona)
        with patch.dict(os.environ, {"BIOROUTER_TEST_PARENT": "kept"}, clear=False):
            parent_identity = os.environ.get("BIOROUTER_ACP_CLIENT")
            with patch.object(
                BaseAcpPersona, "_init_agent_subprocess", new_callable=AsyncMock
            ) as launch:
                asyncio.run(persona._init_agent_subprocess())
            child_env = launch.await_args.kwargs["env"]
            self.assertEqual(child_env["BIOROUTER_TEST_PARENT"], "kept")
            self.assertEqual(child_env["BIOROUTER_ACP_CLIENT"], "jupyter-ai")
            self.assertEqual(os.environ.get("BIOROUTER_ACP_CLIENT"), parent_identity)

    def test_child_preserves_explicit_environment(self):
        persona = object.__new__(BioRouterAcpPersona)
        with patch.object(
            BaseAcpPersona, "_init_agent_subprocess", new_callable=AsyncMock
        ) as launch:
            asyncio.run(persona._init_agent_subprocess(env={"MARKER": "kept"}))
        child_env = launch.await_args.kwargs["env"]
        self.assertEqual(child_env["MARKER"], "kept")
        self.assertEqual(child_env["BIOROUTER_ACP_CLIENT"], "jupyter-ai")

    def test_the_real_base_launch_takes_the_child_environment(self):
        # The two tests above mock the base method, so they cannot show that it
        # takes `env`. jupyter-ai-acp-client 0.1.5 declares
        # `_init_agent_subprocess(self, env=None)` and hands `env` to
        # `asyncio.create_subprocess_exec`; a version without it would start the
        # child without the persona's name, and Jupyter AI would list /dictate.
        import inspect

        parameters = inspect.signature(BaseAcpPersona._init_agent_subprocess).parameters
        self.assertIn("env", parameters)
        self.assertIn(
            parameters["env"].kind,
            (inspect.Parameter.POSITIONAL_OR_KEYWORD, inspect.Parameter.KEYWORD_ONLY),
        )
        self.assertIn("env", inspect.getsource(BaseAcpPersona._init_agent_subprocess))

    def test_manifest_registers_persona(self):
        manifest = Path(__file__).parents[1] / "pyproject.toml"
        project = tomllib.loads(manifest.read_text())["project"]

        self.assertEqual(
            project["entry-points"]["jupyter_ai.personas"]["biorouter-acp"],
            "jupyter_ai_biorouter.persona:BioRouterAcpPersona",
        )


if __name__ == "__main__":
    unittest.main()
