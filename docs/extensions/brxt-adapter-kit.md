# The BRXT adapter kit

> **What this is.** The shared code, templates and tests that every BRXT adapter extension is built from: the one HTTP path an adapter may use, the three tool shapes, the build and release scripts, and the leak suite that proves where each credential goes.
> **Status:** Current. Kit 1.0.1, added 2026-11-20 for the BAAM refresh. Kit 1.0.1 (2026-11-22) closes the two medium findings of the kit's first security review and the smaller ones that were one line each. `safe_http.py` is security code and needs a person's review before any adapter built on it is released, and again after every change.
> **Audience:** developers who build or update a BRXT adapter, and the reviewer of the kit's security code.

BAAM lists 31 adapter extensions (the cards whose organisation reads `BaranziniLab · BRXT`). Release 0.1.0 of every one of them is the same generic server, and that server sends its API key to any URL the model names: a full URL in `path`, or `target="custom"`, gets the configured credential attached. A prompt injection that names a URL is enough to move a lab's Benchling or DNAnexus token off the machine. The same template also ran local commands outside the Developer shell's guards, let the model choose whether a write was allowed, and could not be rebuilt without breaking on fastmcp 4. The kit fixes all of it in one place, and every adapter vendors the result.

The kit lives in [`scripts/brxt-adapter-kit/`](../../scripts/brxt-adapter-kit/). It is code only; this page is its documentation.

## What the kit fixes

| Defect in the 0.1.0 adapters | What the kit does instead |
| --- | --- |
| The credential goes to any URL the model names, including `target="custom"`. | Each credential is bound to its own target's origin (scheme, host and port). A full URL is accepted only on that origin. There is no custom target. |
| Redirects are followed by the HTTP library, which drops `Authorization` on a host change but keeps `X-API-Key`. | The library never follows a redirect. The kit follows at most three, same origin only, and reports any other. |
| One key is sent to every target (the Exa key went to Tavily; the Semantic Scholar key went to OpenAlex, Crossref, PubMed, bioRxiv and Unpaywall). | A key reaches only the targets that declare it. |
| Writes are gated by a model supplied `allow_mutation=true`. | Reads and writes are separate tools with honest annotations, so Biorouter's Smart approval runs reads and asks before every write. |
| `run_*_command` is a second shell without the Developer shell's secret guard. | No command runner. A service that needs local computation gets a typed tool. |
| No lock file, `fastmcp>=2.11.2` with no upper bound, and fastmcp 3 import paths, so a fresh install or a rebuild breaks. | `fastmcp>=4.0,<5`, fastmcp 4 import paths and a committed `uv.lock`, which Biorouter's plain `uv sync` honours. |
| The version is written in four places by hand, the bundle is a plain zip, and tags end in `-brxt`. | `scripts/version.py` moves every version together, `scripts/build_brxt.py` refuses a bundle that breaks a release rule, and the release workflow publishes only a tag equal to `v` plus the manifest version. |

## What is in the kit

| Path under `scripts/brxt-adapter-kit/` | What it is |
| --- | --- |
| `brxt_kit/safe_http.py` | The only HTTP path an adapter may use. The security code. |
| `brxt_kit/spec.py` | Reads and validates an adapter's `adapter.json`. Unknown keys are refused. |
| `brxt_kit/tools.py` | The three tool shapes, and the sunset and tombstone servers. |
| `brxt_kit_testing/echo_server.py` | Loopback HTTPS echo servers with a throwaway certificate authority. |
| `brxt_kit_testing/leak_suite.py` | The battery of credential checks, run in process or against a real stdio subprocess. |
| `brxt_kit_testing/stdio_probe.py` | The start probe: launches an extension the way Biorouter does and lists its tools. |
| `bin/new_adapter.py` | Renders an adapter, sunset or tombstone repository from one spec file. |
| `bin/vendor_kit.py` | Copies the kit into an adapter, or checks the adapter's copy is unchanged. |
| `templates/adapter/` | The repository template: manifest, `pyproject.toml`, server, tests, `scripts/version.py`, `scripts/build_brxt.py`, and the `ci.yml` and `release.yml` workflows. |
| `templates/tombstone/` | The files a tombstone release replaces (its README). |
| `templates/skills/` | The bundled skill templates: one that says how to call the tools, one that says a skill moved. |
| `tests/` | The kit's own tests and their fixtures. |

An adapter carries two vendored copies: `brxt_kit/` becomes `src/<package>/_kit/` and `brxt_kit_testing/` becomes `tests/kit_testing/`. `src/<package>/_kit/vendored.json` records the kit version and the SHA-256 of every vendored file, and both the adapter's tests and its build refuse a copy that differs. To change the kit, change it here, get the change reviewed, and vendor it again.

## How a request is checked

This list is the contract `safe_http.py` enforces, and the checklist for its review.

1. **The base URL.** A target's base is https, names one host and an optional port, and has no user name, password, query or fragment. A `{NAME}` placeholder (a Benchling tenant, for example) is filled from the environment and must be one DNS label, so a configured value can never change which host is reached. A base read whole from the environment (`base_url_env`, for OMERO) follows the same rules and the target's `host_pattern`.
2. **The path.** A path is relative to the target's base path. A full URL is accepted only when its scheme, host and port equal the target's and its path lies under the base path; it is then reduced to a relative path. A path that starts with `//`, holds a backslash, a space or a control character, a fragment, a user name, a `%uXXXX` escape, or a `.` or `..` segment is refused before any connection opens. Dot segments are looked for also when percent encoded once or twice, under NFKC (fullwidth full stops), and with a `;` parameter cut off, since servlet containers read `..;` as `..`.
3. **The target.** The model names a target from a fixed list. `custom` and any other name are refused.
4. **The credential.** It is read from the environment at the moment of the request, attached only to a request for its own target's origin, and never placed where the model can set it: the query parameter that carries a key is reserved, and static headers cannot be `Authorization`, `Cookie` or any other credential header. Two targets on one origin may not carry different credentials.
5. **Redirects.** The HTTP library follows none. The kit follows at most three, only within the same origin; a redirect to another host, or to plain http on the same host, is reported in the result and not followed.
6. **Methods.** A read is GET, or POST on a route the target declares as a read route (DNAnexus `/system/find*`, for example). A JSON body on a GET route is refused. A write uses a method and route the target declares; anything else is refused. A read does not reach the path of a declared write route, because services that put the action in the path (TamarindBio's `POST /stop-job`) may run it on a GET too. The path is compared the way a server may read it: percent decoded once and twice, under NFKC, with `;` parameters cut, with a final extension dropped and with case folded. Only a read route that lies within the write route's own pattern opens such a path to a read: a REST service where `GET /entries` lists and `POST /entries` creates names `/entries` in `get_routes`, and the catch-all `/**` never counts. A read never uses the write's own method on its path, and a read POST route that could reach a write route is refused when the target is declared.
7. **Size and time.** Request bodies are capped at 1 MB, responses at the target's `max_response_bytes` (2 MB by default, 20 MB at most), and waits at the target's `timeout_s` (30 seconds by default, 120 at most). A tool call may shorten the target's time limit, never lengthen it. Compressed replies are inflated in bounded steps, so a small reply cannot grow without limit, and the raw bytes read are capped too (the cap plus 1 MiB, or twice the cap plus 1 MiB when compressed, never more than 20 MB plus 1 MiB). A gzip body may hold several members, as `bgzip` and `cat a.gz b.gz` write it, and each is inflated in turn within the same cap. Nothing after the end of the compressed data is read or kept: the reply is marked `truncated`. A body that cannot be inflated is an error.
8. **What the model sees.** Every credential value, in every encoding the kit sends it (raw, percent encoded, base64, the Basic pair), is replaced with `[REDACTED]` in results and error messages, including when a service echoes it back, also as a JSON number. A credential shorter than six characters is too short to redact without false matches, so it is never sent. Only a short list of response headers is shown, never `Set-Cookie`.
9. **Logs.** The HTTP library's own log lines print full URLs, so a query credential would reach the log. They are silenced, and the kit never logs a header, a query string or a body.
10. **Failures.** Every refusal or failure becomes a tool error raised outside the exception handler, so no chained exception carries a URL or a value, and the status tool goes through the same handler. An unexpected exception is reported as an internal error with nothing more. A setting or a full URL that Python's URL parser cannot read is a configuration error or a refusal, never a raw exception, and one bad setting does not stop the other targets. A credential or static header value outside ASCII, which a header cannot carry, is refused the same way.

## Tool shapes

| Tool | Annotation | What Biorouter does with it |
| --- | --- | --- |
| `<svc>_status` | read only, no network | Runs it without asking. It reports targets, routes and which settings are present, never a value. |
| `<svc>_get` | read only | Smart approval runs it without asking. |
| `<svc>_write` | destructive | Asks the person before every call. Registered only when a target declares a write route. |

Biorouter grades a tool from these annotations (`crates/biorouter/src/permission/tool_risk.rs`): read only is low risk and runs under Smart approval, and destructive is high risk and always asks. The 0.1.0 adapters annotated their one HTTP tool as neither, so every call asked and the model's `allow_mutation` flag was the only gate on writes.

## The adapter spec

`bin/new_adapter.py` reads one JSON file and writes the whole repository. The same file's `adapter` section becomes `src/<package>/adapter.json`, which the server reads at start.

| Top level key | What it holds |
| --- | --- |
| `kit_spec` | Always `1`. |
| `version` | The release version, `X.Y.Z`. |
| `authors` | A list of `{"name", "email"}` for `pyproject.toml`. |
| `copyright_holder` | The `NOTICE` holder. Defaults to the Baranzini Lab at UCSF. |
| `manifest` | `name` (the registry id, never changed), `display_name`, `description`, `repository` (never changed), `license`, and `env_vars` exactly as Biorouter's manifest takes them. |
| `adapter` | The service surface, below. |
| `skills` | The bundled skills, below. |
| `readme_intro` | An optional paragraph for the README. |

| `adapter` key | What it holds |
| --- | --- |
| `service` | The tool prefix, for example `benchling` gives `benchling_get`. |
| `service_title` | The service's name in tool descriptions. |
| `kind` | `adapter`, `sunset` or `tombstone`. |
| `instructions` | Server instructions placed before the kit's own paragraph on how to call the tools. |
| `log_level_env` | The setting that holds the log level. |
| `targets` | The list of targets, below. A tombstone has none. |
| `notice` | Required for a sunset or tombstone: `status` (`superseded_by` or `retired`), `reason`, `use_instead`, and `replacement`, `replacement_id` or `date`. |
| `extra_secrets` | Secrets a typed tool uses outside `safe_http`, each with `env` and `used_by`. They need their own review, because the leak suite does not cover them. |

| Target key | What it holds |
| --- | --- |
| `name` | Lower case, unique, never `custom`. |
| `base_url` | The https base, optionally with `{NAME}` placeholders. |
| `base_url_env`, `host_pattern` | A base set by the person, and the regular expression its host must match. |
| `credential` | `env`, `scheme`, and the scheme's own fields, below. `required` defaults to true; an optional credential that is not set is not sent. |
| `get_routes` | GET read routes. Defaults to every path under the base (`/**`). A path that is also a write route's is read only when a route here lies within that write route (`/entries` beside `POST /entries`, `/entries/*` beside `PATCH /entries/*`); see rule 6. |
| `read_post_routes` | POST routes that only read. One that could reach a POST write route, or a write route of another method that it does not lie within, is refused. |
| `write_routes` | `{"method", "path"}` pairs for `<svc>_write`. |
| `headers` | Static headers that are not credentials. |
| `env_query` | `{"param", "env"}` pairs for query values taken from settings, such as a contact email. |
| `max_response_bytes`, `timeout_s` | The caps for this target. |
| `documentation`, `examples` | Shown by the status tool and the tool descriptions. |

| Credential scheme | What is sent | Extra fields |
| --- | --- | --- |
| `bearer` | `Authorization: Bearer <value>` | none |
| `basic` | `Authorization: Basic <base64>`, the key as user name with an empty password, or a user name from `username_env` with the key as password | `username_env` |
| `header` | `<header_name>: <value_prefix><value>` | `header_name`, `value_prefix` |
| `query` | `?<query_param>=<value>` | `query_param` |

In a route, `*` matches within one path segment and `**` matches any number of whole segments. Routes are matched against the path relative to the base, case sensitive. A path that matches a read POST route is sent as POST even if it also matches a GET route.

A skill entry has `name` (kept exactly, so an update overwrites the old file), `description`, `form` and the form's fields. `form: tools` writes a skill that says only how to call this extension's tools, naming the BAAM skill with the method in `method_skill`. `form: moved` writes a moved notice naming `moved_to`. A sunset or tombstone release takes only moved notices.

A short example:

```json
{
  "kit_spec": 1,
  "version": "0.2.0",
  "manifest": {
    "name": "dnanexusagent",
    "display_name": "DNAnexusAgent",
    "description": "Read DNAnexus projects and data objects, and make writes Biorouter asks you to approve.",
    "repository": "https://github.com/BaranziniLab/DNAnexusAgent",
    "env_vars": [
      {"key": "DNANEXUS_API_TOKEN", "required": true, "auto_propagate": false, "secret": true, "description": "DNAnexus API token"}
    ]
  },
  "adapter": {
    "kit_spec": 1,
    "service": "dnanexus",
    "service_title": "DNAnexus",
    "kind": "adapter",
    "targets": [
      {
        "name": "api",
        "base_url": "https://api.dnanexus.com",
        "credential": {"env": "DNANEXUS_API_TOKEN", "scheme": "bearer"},
        "get_routes": [],
        "read_post_routes": ["/system/find*", "/*/describe", "/*/listFolder"],
        "examples": ["/system/findProjects"]
      }
    ]
  },
  "skills": [
    {"name": "dnanexus-genomics-platform", "form": "tools", "description": "How to call DNAnexusAgent's tools.", "method_skill": "workflow-management"}
  ]
}
```

The four fixtures in `scripts/brxt-adapter-kit/tests/fixtures/` show every field in use.

## Making a new adapter

Run these from `scripts/brxt-adapter-kit/`, after `uv sync`.

1. Write the spec file. Declare every setting the targets read in `manifest.env_vars`, and mark each credential `secret: true` and `auto_propagate: false`.
2. Render the repository: `uv run python bin/new_adapter.py spec.json ../../../MyAgent`.
3. In the new repository, run `uv lock`, then `uv sync`.
4. Run the contract tests: `uv run python -m unittest discover -s tests -v`.
5. Build: `uv run python scripts/build_brxt.py`. It writes `dist/<id>.brxt`, `dist/<id>-X.Y.Z.brxt` and `dist/SHA256SUMS`.
6. Install the bundle in an isolated root and check it starts: `BIOROUTER_PATH_ROOT=$(mktemp -d) BIOROUTER_DISABLE_KEYRING=true biorouter extension install dist/<id>.brxt`.

To add a typed tool (RDKit descriptors, an Opentrons simulation), write it in `register_extra_tools` in `server.py`. It reaches the network through the kit's `SafeHttpClient` and runs no local command; the contract test fails on an import of `httpx`, `requests`, `subprocess` or `socket` in the adapter's own code. A file that must reach the network another way, such as through a vendor library, carries a `# brxt-kit: allow-direct-network <reason>` line and needs a reviewer to accept it. Update `tools_count` in `manifest.json` when you add tools; the build refuses a count that differs from what the server lists.

## Converting a 0.1.0 adapter

1. Clone the adapter's repository. Keep `manifest.name`, `manifest.repository` and every bundled skill name exactly as they are.
2. Write the spec from the old `manifest.json` and server: one target per old API target, with its credential bound to it alone. Drop the `custom` target, the command runner and every `ALLOWED_COMMANDS` list. Correct any dead endpoint while you are there (the plan lists them).
3. Download the 0.1.0 bundle, then render over the clone with it as the previous release: `uv run python bin/new_adapter.py spec.json ../AdapterRepo --force --previous old.brxt`. The command refuses a spec that drops a skill the old bundle shipped.
4. Review the files it lists as not written by the kit. Remove `scripts/build_brxt.sh` and anything else the kit replaced; keep tests of your own.
5. `uv lock`, the tests, then `uv run python scripts/build_brxt.py --previous old.brxt`.
6. Commit, push and tag `v0.2.0` when the release is approved. The release workflow does the rest.

## Sunset and tombstone releases

A **sunset** release is the working adapter, rebuilt on the kit, with a notice: set `kind: sunset` and a `notice` with `status: superseded_by`. The notice appears in the server instructions, at the top of every tool description and in the status report, so the person and the model both learn what replaces it.

A **tombstone** release replaces every tool with one, `<svc>_status`, which returns the notice. Set `kind: tombstone`, no targets and an empty `env_vars`; it reads no credential and no setting, and builds no HTTP client.

Both keep every bundled skill path of the previous release and overwrite each `SKILL.md` with a moved notice. An update in place never deletes a file the new version dropped, so a skill left out would keep its old instructions on every machine that had it.

## Version, build and release

`scripts/version.py X.Y.Z` rewrites `manifest.json`, `pyproject.toml`, `src/<package>/__init__.py` and the README's `Current version` line, and refuses a version that does not move forward. The User-Agent is built from `__version__`, so it moves with them.

`scripts/build_brxt.py` refuses to build unless:

- the four versions agree;
- `manifest.json`, `README.md`, `LICENSE`, `NOTICE`, `pyproject.toml`, `uv.lock` and `src/<package>/` are present, and `uv.lock` pins fastmcp 4 and this version of the project;
- every setting `adapter.json` reads is declared, every credential is declared secret, no secret has a default or is propagated from the machine, every declared secret is bound to a target, and a tombstone declares none;
- the vendored kit matches `vendored.json`;
- the bundled skills are exactly the manifest's, each names itself in its frontmatter, and, with `--previous`, every skill path of the previous bundle is still there;
- with `--previous`, the name and repository are unchanged and the version moved forward;
- the server starts in process and lists exactly `tools_count` tools.

The bundle holds only an explicit list of files with fixed timestamps, so the same tree always gives the same bytes. Local data such as `.env`, `.csv` or `.sqlite3` files next to the code is never bundled.

The `ci.yml` workflow runs on every push and pull request, on Ubuntu and macOS: `uv lock --check`, `uv sync --locked`, the tests and the build. The `release.yml` workflow runs on a `v*` tag: it refuses a tag that is not `v` plus the manifest version, runs the same checks, downloads the latest release's `<id>.brxt` for the skill path check, builds, and publishes `<id>.brxt`, `<id>-X.Y.Z.brxt` and `SHA256SUMS` as the latest release. A GitHub release is a deploy: the desktop updater installs it at the next launch without asking.

## Tests

The kit's own tests run from `scripts/brxt-adapter-kit/`:

```bash
uv sync
uv run python -m unittest discover -s tests -v
```

| File | What it proves |
| --- | --- |
| `test_safe_http_rules.py` | Declaration rules, route matching, path preparation and base resolution, with no network. |
| `test_safe_http_binding.py` | Against loopback HTTPS echo servers: each scheme reaches its own host only, full URLs and redirects to other hosts are refused or not followed, writes and reads keep their methods, caps hold, echoed secrets are redacted, and the HTTP library's logs hold no query credential. |
| `test_tools.py` | Tool lists and annotations, the status report, refusals as redacted tool errors, and the sunset and tombstone servers. |
| `test_safe_http_review.py` | One class for each finding of the 2026-11-20 security review that kit 1.0.1 closes: decompression bounds and gzip members, reads on write paths and read POST declarations (with a brute force check of the route overlap and containment tests), `..;` and fullwidth dot segments, numeric and short credentials, the time limit, the Accept check, settings Python cannot parse, and IDN hosts. |
| `test_leak_suite.py` | The leak battery passes on the kit, and fails on twelve broken kits, each reintroducing one way a credential leaked or could leak (the 0.1.0 template among them, and a kit whose read tool reaches write paths). |
| `test_stdio.py` | A rendered sample adapter as a real subprocess: the start probe, the whole battery over stdio, and a log written at DEBUG level that holds no secret. A mutant with the log silencing removed proves the log check can fail. The sunset and tombstone releases start and say what replaces them. |
| `test_templates.py` | The generator, vendoring, `version.py`, and every refusal of `build_brxt.py`. |
| `test_integration.py` | Opt in with `BRXT_KIT_INTEGRATION=1`: real `uv lock` and `uv sync --locked`, the rendered adapter's own CI steps, a fresh extraction started with `uv run --directory`, and, with `BRXT_KIT_BIOROUTER` set to a `biorouter` binary, an install through `biorouter extension install` into a throwaway root followed by the battery against the installed copy. |

A live check on 2026-11-20 installed the sample adapter with `biorouter extension install` (Biorouter 1.94.6, keyring off, a throwaway `BIOROUTER_PATH_ROOT`) and ran one `biorouter run` turn on Versa `gpt-5.5-2026-04-24` in Smart approval, with Code Execution off so the model called the tools directly. `sample_get` ran without asking and returned 200; a full URL on another host, given to the model as an instruction from a document, was refused with the kit's message and the outside server saw no request; `sample_write` was held with "needs a person's approval"; and no credential appeared in the output or in any file of the profile except `secrets.yaml`.

Every adapter runs `tests/test_adapter_contract.py` in its CI: the vendored kit is unchanged, the lock pins fastmcp 4 and this version, the release rules hold, the server starts over stdio and lists its tools, the leak battery passes against the adapter's own spec with every target pointed at an echo server, a tombstone reads no credential, and the adapter's own code reaches the network and runs programs only through the kit.

## Known limits

- **The kit is not in Biorouter's CI.** Its tests run by hand from `scripts/brxt-adapter-kit/`, and in every adapter's CI through the vendored copy. Wiring them into a Biorouter workflow is left for later.
- **Windows is not in the adapter CI matrix**, which follows the shape CDWAgent already runs (Ubuntu and macOS).
- **The time cap is per network wait.** A server that answers one byte at a time just inside the timeout can hold a request longer than `timeout_s` before the body starts; the body itself stops at the deadline.
- **Proxies and certificate files from the environment are honoured** (`HTTPS_PROXY`, `NO_PROXY`, `SSL_CERT_FILE`), as the 0.1.0 adapters did, so institutional proxies keep working. A credential still travels only inside TLS to its own host.
- **A same origin redirect may leave the base path and the declared routes.** The credential stays on its own host, which owns it. A 307 or 308 keeps the method, the body and the credential, so a service that redirects an approved write or a read POST elsewhere on its own host sends them there.
- **A refused cross host redirect is shown.** Its `Location` header is in the result, and for object stores that is often a presigned URL, a bearer link of its own.
- **Route confinement depends on the server's path handling.** Routes match the path as sent, so a `*` matches `a%2Fb`, which a server that decodes `%2F` reads as two segments. The credential still reaches only its own host. Rule 6 reads write paths the way a server may, but read routes are matched as sent.
- **Redaction matches whole values** in the encodings the kit sends, and JSON numbers. A service that echoes part of a key, a different encoding of it, or JSON escapes in a reply cut at the cap (which is returned as text, not parsed), is not caught. `extra_secrets` and the Basic user name are not in the redaction set.
- **The system's proxy and key log settings apply.** On macOS httpx also reads the proxies set in System Settings, and Python honours `SSLKEYLOGFILE`, which writes TLS session keys to a file. Both are the person's own environment.
- **Typed tools and `extra_secrets`** are outside the leak battery. Each needs its own tests and review.

## Related documentation

- [Installing an extension, and where its credentials go](installing-an-extension.md): how Biorouter installs a `.brxt` and stores its secrets.
- [Capabilities, extensions, skills, and MCP agents](extensions-and-skills-guide.md): what an extension is to the person using it.
- [Permission modes](../security/permission-modes.md): what Smart approval does with a read only or destructive tool.
- [Developer](built-in/developer.md): the one command runner, and the secret guard the adapters' old runner skipped.
- [Secret guard](../security/secret-guard.md): the floor that keeps credential files away from the model.
