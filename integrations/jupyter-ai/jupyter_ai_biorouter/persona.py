import os
import shutil
from pathlib import Path

from jupyter_ai_acp_client.base_acp_persona import BaseAcpPersona
from jupyter_ai_persona_manager import PersonaDefaults, PersonaRequirementsUnmet


def resolve_biorouter_executable() -> str:
    configured = os.environ.get("BIOROUTER_EXECUTABLE")
    if configured:
        candidate = Path(configured).expanduser()
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate.resolve())

        resolved = shutil.which(configured)
        if resolved:
            return resolved

        raise PersonaRequirementsUnmet(
            f"BIOROUTER_EXECUTABLE does not point to an executable: {configured}"
        )

    resolved = shutil.which("biorouter")
    if resolved:
        return resolved

    raise PersonaRequirementsUnmet(
        "This persona requires the `biorouter` CLI on PATH. Install Biorouter "
        "or set BIOROUTER_EXECUTABLE to its absolute path, then restart JupyterLab."
    )


BIOROUTER_EXECUTABLE = resolve_biorouter_executable()


class BioRouterAcpPersona(BaseAcpPersona):
    def __init__(self, *args, **kwargs):
        super().__init__(
            *args,
            executable=[BIOROUTER_EXECUTABLE, "acp"],
            **kwargs,
        )

    async def _init_agent_subprocess(self, env=None):
        child_env = os.environ.copy() if env is None else dict(env)
        child_env["BIOROUTER_ACP_CLIENT"] = "jupyter-ai"
        return await super()._init_agent_subprocess(env=child_env)

    @property
    def defaults(self) -> PersonaDefaults:
        avatar_path = Path(__file__).parent / "static" / "biorouter.svg"
        return PersonaDefaults(
            name="Biorouter",
            description=(
                "Biorouter over ACP (Agent Client Protocol), with access to its "
                "configured extensions, skills, workflows, and biomedical agents."
            ),
            avatar_path=str(avatar_path.resolve()),
            system_prompt="unused",
        )
