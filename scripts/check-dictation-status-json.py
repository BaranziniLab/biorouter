#!/usr/bin/env python3
"""Validate `biorouter dictation status --format json` against schema 1 (PR-10).

Usage:
  python3 -I scripts/check-dictation-status-json.py [--expect-state S] [--expect-capture-reason R]
      [--expect-no-models] [--expect-missing-feature F]... FILE
  python3 -I scripts/check-dictation-status-json.py --self-test

FILE is the command's output (`-` reads stdin). Schema 1 has a required core, checked here
key by key and type by type; later pull requests add keys (`install`, `install_job`,
`limits`, `queue`), and readers ignore keys they do not know, so this check does too.

The package smokes run it inside both Linux containers and the Nehalem check runs it under
`qemu-x86_64 -cpu Nehalem`, where the expected answer is `unsupported_cpu` with `avx2` among
the missing features. Standard library only, and Python 3.9 compatible (debian:bullseye).
"""

import copy
import json
import sys

STATES = ("no_engine", "unsupported_cpu", "stopped", "starting", "ready", "error")
BACKENDS = ("metal", "cpu", None)
CPU_FEATURES = ("avx2", "fma", "f16c", "bmi2", "dotprod")
ALLOWANCES = ("enabled", "capture", "install", "external_engine", "dictionary", "agent_writes")


class _Missing:
    """What `.get` returns for an absent key: unlike None or a string, it satisfies no type."""

    def __repr__(self):
        return "missing"


MISSING = _Missing()


def _is_int(value):
    return isinstance(value, int) and not isinstance(value, bool)


def _is_str_or_none(value):
    return value is None or isinstance(value, str)


def problems(doc):
    """Every way `doc` departs from schema 1, as readable sentences; empty when it conforms."""
    found = []

    def need(condition, sentence):
        if not condition:
            found.append(sentence)
        return condition

    if not need(isinstance(doc, dict), "the document is not a JSON object"):
        return found
    need(_is_int(doc.get("schema")) and doc.get("schema") == 1, f"schema is {doc.get('schema')!r}, expected 1")

    engine = doc.get("engine")
    if need(isinstance(engine, dict), "engine is missing or not an object"):
        compiled = engine.get("compiled")
        need(isinstance(compiled, bool), f"engine.compiled is {compiled!r}, expected true or false")
        state = engine.get("state")
        need(state in STATES, f"engine.state is {state!r}, expected one of {', '.join(STATES)}")
        backend = engine.get("backend", MISSING)
        need(backend in BACKENDS, f"engine.backend is {backend!r}, expected \"metal\", \"cpu\" or null")
        threads = engine.get("threads")
        need(_is_int(threads) and threads >= 0, f"engine.threads is {threads!r}, expected a whole number")
        missing = engine.get("missing_cpu_features")
        if need(isinstance(missing, list), f"engine.missing_cpu_features is {missing!r}, expected a list"):
            for feature in missing:
                need(
                    feature in CPU_FEATURES,
                    f"engine.missing_cpu_features holds {feature!r}, expected lower case ids from {', '.join(CPU_FEATURES)}",
                )
        rss = engine.get("rss_bytes", MISSING)
        need(rss is None or (_is_int(rss) and rss >= 0), f"engine.rss_bytes is {rss!r}, expected a whole number or null")
        build = engine.get("build", MISSING)
        if need(build is None or isinstance(build, dict), f"engine.build is {build!r}, expected an object or null"):
            if isinstance(build, dict):
                for key in ("whisper", "ggml", "isa"):
                    need(isinstance(build.get(key), str), f"engine.build.{key} is {build.get(key)!r}, expected a string")
        # The relations section 14.2 fixes between the fields.
        if compiled is False:
            need(state == "no_engine", f"engine.compiled is false but engine.state is {state!r}, expected \"no_engine\"")
            need(build is None, "engine.compiled is false but engine.build is not null")
        if compiled is True:
            need(state != "no_engine", "engine.compiled is true but engine.state is \"no_engine\"")
            need(isinstance(build, dict), "engine.compiled is true but engine.build is null")
        if state in ("no_engine", "unsupported_cpu"):
            need(backend is None, f"engine.state is {state!r} but engine.backend is {backend!r}, expected null")

    models = doc.get("models")
    need(isinstance(models, list), f"models is {models!r}, expected a list")
    active = doc.get("active_model", MISSING)
    need(_is_str_or_none(active), f"active_model is {active!r}, expected a string or null")
    need(isinstance(doc.get("language"), str), f"language is {doc.get('language')!r}, expected a string")

    policy = doc.get("policy")
    if need(isinstance(policy, dict), "policy is missing or not an object"):
        need(isinstance(policy.get("managed"), bool), f"policy.managed is {policy.get('managed')!r}, expected true or false")
        for key in ALLOWANCES:
            allowance = policy.get(key)
            if need(isinstance(allowance, dict), f"policy.{key} is {allowance!r}, expected {{allowed, reason}}"):
                need(
                    isinstance(allowance.get("allowed"), bool),
                    f"policy.{key}.allowed is {allowance.get('allowed')!r}, expected true or false",
                )
                reason = allowance.get("reason", MISSING)
                need(_is_str_or_none(reason), f"policy.{key}.reason is {reason!r}, expected a string or null")
        allowed_models = policy.get("allowed_models", MISSING)
        if need(
            allowed_models is None or isinstance(allowed_models, list),
            f"policy.allowed_models is {allowed_models!r}, expected a list or null",
        ):
            for item in allowed_models or []:
                need(isinstance(item, str), f"policy.allowed_models holds {item!r}, expected strings")

    capture = doc.get("capture")
    if need(isinstance(capture, dict), "capture is missing or not an object"):
        need(
            isinstance(capture.get("available"), bool),
            f"capture.available is {capture.get('available')!r}, expected true or false",
        )
        reason = capture.get("reason", MISSING)
        need(_is_str_or_none(reason), f"capture.reason is {reason!r}, expected a string or null")

    for key in ("account", "host", "models_dir"):
        need(isinstance(doc.get(key), str), f"{key} is {doc.get(key)!r}, expected a string")
    return found


def expectation_problems(doc, state=None, capture_reason=None, no_models=False, missing_features=()):
    found = []
    engine = doc.get("engine") if isinstance(doc, dict) else None
    engine = engine if isinstance(engine, dict) else {}
    if state is not None and engine.get("state") != state:
        found.append(f"engine.state is {engine.get('state')!r}, this run expects {state!r}")
    if capture_reason is not None:
        capture = doc.get("capture") if isinstance(doc.get("capture"), dict) else {}
        if capture.get("reason") != capture_reason:
            found.append(f"capture.reason is {capture.get('reason')!r}, this run expects {capture_reason!r}")
    if no_models:
        if doc.get("models") != []:
            found.append(f"models is {doc.get('models')!r}, this run expects none installed")
        if doc.get("active_model") is not None:
            found.append(f"active_model is {doc.get('active_model')!r}, this run expects null")
    listed = engine.get("missing_cpu_features")
    listed = listed if isinstance(listed, list) else []
    for feature in missing_features:
        if feature not in listed:
            found.append(f"engine.missing_cpu_features is {listed!r}, this run expects it to name {feature!r}")
    return found


GOOD = {
    "schema": 1,
    "engine": {
        "compiled": True,
        "state": "stopped",
        "backend": "metal",
        "device": None,
        "threads": 8,
        "missing_cpu_features": [],
        "rss_bytes": None,
        "build": {"whisper": "4da27c41", "ggml": "0.28.0", "isa": "armv8.2-a+dotprod+fp16"},
    },
    "models": [],
    "active_model": None,
    "language": "en",
    "policy": {
        "managed": False,
        "enabled": {"allowed": True, "reason": None},
        "capture": {"allowed": True, "reason": None},
        "install": {"allowed": True, "reason": None},
        "external_engine": {"allowed": True, "reason": None},
        "dictionary": {"allowed": True, "reason": None},
        "agent_writes": {"allowed": True, "reason": None},
        "allowed_models": None,
    },
    "capture": {"available": False, "reason": "not_compiled"},
    "account": "wgu",
    "host": "example-host",
    "models_dir": "/Users/wgu/.local/share/biorouter/dictation/models",
}

_DELETE = object()


def _mutate(path, value):
    doc = copy.deepcopy(GOOD)
    target = doc
    for key in path[:-1]:
        target = target[key]
    if value is _DELETE:
        del target[path[-1]]
    else:
        target[path[-1]] = value
    return doc


def self_test():
    failures = 0
    cases = 0

    def expect(name, doc, want_problem, **expectations):
        nonlocal failures, cases
        cases += 1
        got = problems(doc) + (expectation_problems(doc, **expectations) if isinstance(doc, dict) else [])
        if want_problem is None:
            ok = not got
        else:
            ok = any(want_problem in sentence for sentence in got)
        if ok:
            print(f"ok   {name}")
        else:
            failures += 1
            print(f"FAIL {name}: want {want_problem!r}, got {got}")

    expect("the section 14.2 example conforms", GOOD, None)
    unsupported = _mutate(("engine", "state"), "unsupported_cpu")
    unsupported["engine"]["backend"] = None
    unsupported["engine"]["missing_cpu_features"] = ["avx2", "fma", "f16c", "bmi2"]
    # From PR-25 a Linux container with no sound system answers with its capture probe.
    unsupported["capture"] = {"available": False, "reason": "no_audio_system"}
    expect(
        "the Nehalem answer meets its expectations",
        unsupported,
        None,
        state="unsupported_cpu",
        capture_reason="no_audio_system",
        no_models=True,
        missing_features=("avx2",),
    )
    expect(
        "a build without capture fails where the probe is expected",
        GOOD,
        "this run expects 'no_audio_system'",
        capture_reason="no_audio_system",
    )
    no_engine = _mutate(("engine", "compiled"), False)
    no_engine["engine"].update(state="no_engine", backend=None, build=None)
    expect("a build without the engine conforms", no_engine, None)
    with_extra = _mutate(("install_job",), {"id": 3})
    with_extra["limits"] = {"max_seconds": 600}
    expect("unknown keys are allowed (schema 1 is additive)", with_extra, None)
    expect("a list is not a document", [GOOD], "not a JSON object")

    rules = [
        ("schema 2 fails", ("schema",), 2, "schema is 2"),
        ("schema true fails", ("schema",), True, "schema is True"),
        ("a missing engine fails", ("engine",), _DELETE, "engine is missing"),
        ("engine.compiled as a string fails", ("engine", "compiled"), "yes", "engine.compiled"),
        ("an unknown state fails", ("engine", "state"), "running", "engine.state is 'running'"),
        ("an unknown backend fails", ("engine", "backend"), "vulkan", "engine.backend"),
        ("a missing backend fails", ("engine", "backend"), _DELETE, "engine.backend"),
        ("threads as a string fails", ("engine", "threads"), "8", "engine.threads"),
        ("threads as a bool fails", ("engine", "threads"), True, "engine.threads"),
        ("missing features as a string fails", ("engine", "missing_cpu_features"), "avx2", "missing_cpu_features"),
        ("an upper case feature id fails", ("engine", "missing_cpu_features"), ["AVX2"], "lower case ids"),
        ("rss_bytes as a float fails", ("engine", "rss_bytes"), 1.5, "engine.rss_bytes"),
        ("a missing rss_bytes fails", ("engine", "rss_bytes"), _DELETE, "engine.rss_bytes"),
        ("build as a string fails", ("engine", "build"), "4da27c41", "engine.build is"),
        ("build without isa fails", ("engine", "build", "isa"), _DELETE, "engine.build.isa"),
        ("compiled with a null build fails", ("engine", "build"), None, "engine.build is null"),
        ("compiled with state no_engine fails", ("engine", "state"), "no_engine", "engine.compiled is true"),
        ("models as null fails", ("models",), None, "models is None"),
        ("active_model as a number fails", ("active_model",), 3, "active_model"),
        ("a missing active_model fails", ("active_model",), _DELETE, "active_model"),
        ("language as null fails", ("language",), None, "language"),
        ("a missing policy fails", ("policy",), _DELETE, "policy is missing"),
        ("policy.managed as a string fails", ("policy", "managed"), "no", "policy.managed"),
        ("allowed as a string fails", ("policy", "install", "allowed"), "true", "policy.install.allowed"),
        ("a reason as a number fails", ("policy", "dictionary", "reason"), 1, "policy.dictionary.reason"),
        ("a missing allowance reason fails", ("policy", "enabled", "reason"), _DELETE, "policy.enabled.reason is missing"),
        ("a missing capture reason fails", ("capture", "reason"), _DELETE, "capture.reason is missing"),
        ("a missing allowance fails", ("policy", "agent_writes"), _DELETE, "policy.agent_writes"),
        ("allowed_models as a string fails", ("policy", "allowed_models"), "light", "policy.allowed_models"),
        ("allowed_models holding a number fails", ("policy", "allowed_models"), [1], "policy.allowed_models holds"),
        ("capture.available as a string fails", ("capture", "available"), "no", "capture.available"),
        ("a missing capture fails", ("capture",), _DELETE, "capture is missing"),
        ("account as null fails", ("account",), None, "account"),
        ("a missing host fails", ("host",), _DELETE, "host"),
        ("models_dir as a list fails", ("models_dir",), ["/x"], "models_dir"),
    ]
    for name, path, value, want in rules:
        expect(name, _mutate(path, value), want)

    not_compiled_state = _mutate(("engine", "compiled"), False)
    not_compiled_state["engine"].update(build=None, backend=None)
    expect("not compiled but stopped fails", not_compiled_state, "expected \"no_engine\"")
    refused_backend = _mutate(("engine", "state"), "unsupported_cpu")
    expect("unsupported_cpu with a backend fails", refused_backend, "expected null")

    expect("a wrong expected state fails", GOOD, "this run expects 'unsupported_cpu'", state="unsupported_cpu")
    expect("a wrong capture reason fails", GOOD, "this run expects 'no_device'", capture_reason="no_device")
    expect(
        "an installed model fails --expect-no-models",
        _mutate(("models",), [{"id": "light"}]),
        "this run expects none installed",
        no_models=True,
    )
    expect(
        "an active model fails --expect-no-models",
        _mutate(("active_model",), "light"),
        "active_model is 'light'",
        no_models=True,
    )
    expect("an absent missing feature fails", GOOD, "expects it to name 'avx2'", missing_features=("avx2",))

    if failures:
        print(f"::error::{failures} of {cases} check-dictation-status-json self test case(s) did not behave as expected")
        return 1
    print(f"check-dictation-status-json self test: {cases} cases behaved as expected")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    state = capture_reason = path = None
    no_models = False
    features = []
    i = 0
    usage = (
        "usage: check-dictation-status-json.py [--expect-state S] [--expect-capture-reason R] "
        "[--expect-no-models] [--expect-missing-feature F]... FILE | --self-test"
    )
    while i < len(argv):
        arg = argv[i]
        if arg in ("--expect-state", "--expect-capture-reason", "--expect-missing-feature"):
            if i + 1 >= len(argv):
                print(usage, file=sys.stderr)
                return 2
            value = argv[i + 1]
            if arg == "--expect-state":
                if value not in STATES:
                    print(f"--expect-state must be one of {', '.join(STATES)}", file=sys.stderr)
                    return 2
                state = value
            elif arg == "--expect-capture-reason":
                capture_reason = value
            else:
                features.append(value)
            i += 2
        elif arg == "--expect-no-models":
            no_models = True
            i += 1
        elif arg.startswith("--") or path is not None:
            print(usage, file=sys.stderr)
            return 2
        else:
            path = arg
            i += 1
    if path is None:
        print(usage, file=sys.stderr)
        return 2
    try:
        if path == "-":
            text = sys.stdin.read()
        else:
            with open(path, encoding="utf-8") as handle:
                text = handle.read()
        doc = json.loads(text)
    except (OSError, ValueError) as error:
        print(f"::error::{path}: not readable as JSON: {error}")
        return 1
    found = problems(doc)
    if isinstance(doc, dict):
        found += expectation_problems(doc, state, capture_reason, no_models, features)
    if found:
        for sentence in found:
            print(f"::error::{path}: {sentence}")
        return 1
    engine = doc["engine"]
    print(
        f"OK: {path}: dictation status schema 1; engine {engine['state']}"
        f" (backend {engine['backend']}, missing {engine['missing_cpu_features'] or 'nothing'});"
        f" {len(doc['models'])} model(s); capture {doc['capture']['reason'] or 'available'}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
