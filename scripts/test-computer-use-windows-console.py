#!/usr/bin/env python3
"""Compare Windows Copilot helper child-console behavior with its pre-fix build."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import traceback


ROOT = Path(__file__).resolve().parents[1]
PATCH = ROOT / "vendor/computer-use/patches/0003-windows-console.patch"
CREATE_NO_WINDOW = 0x08000000

# A GUI observer cannot reliably attach to another process's console on CI.
# This child reports its own console handle before forwarding to PowerShell.
PROBE_GO = r'''
package main

import (
    "encoding/json"
    "fmt"
    "os"
    "os/exec"
    "syscall"
)

func consoleVerdict() string {
    kernel := syscall.NewLazyDLL("kernel32.dll")
    user := syscall.NewLazyDLL("user32.dll")
    window, _, _ := kernel.NewProc("GetConsoleWindow").Call()
    if window == 0 {
        return "none"
    }
    visible, _, _ := user.NewProc("IsWindowVisible").Call(window)
    if visible != 0 {
        return "visible"
    }
    return "hidden"
}

func main() {
    report := os.Getenv("BIOROUTER_CONSOLE_PROBE_FILE")
    if report == "" {
        fmt.Fprintln(os.Stderr, "missing console probe report path")
        os.Exit(2)
    }
    result, err := json.Marshal(map[string]any{
        "verdict": consoleVerdict(),
        "pid": os.Getpid(),
    })
    if err == nil {
        err = os.WriteFile(report, append(result, '\n'), 0600)
    }
    if err != nil {
        fmt.Fprintln(os.Stderr, err)
        os.Exit(2)
    }
    if os.Getenv("BIOROUTER_CONSOLE_PROBE_CONTROL") == "1" {
        return
    }

    real := os.Getenv("BIOROUTER_REAL_POWERSHELL")
    if real == "" {
        fmt.Fprintln(os.Stderr, "missing real PowerShell path")
        os.Exit(2)
    }
    child := exec.Command(real, os.Args[1:]...)
    child.SysProcAttr = &syscall.SysProcAttr{
        CreationFlags: 0x08000000,
        HideWindow: true,
    }
    child.Stdout = os.Stdout
    child.Stderr = os.Stderr
    if err := child.Run(); err != nil {
        if exit, ok := err.(*exec.ExitError); ok {
            os.Exit(exit.ExitCode())
        }
        fmt.Fprintln(os.Stderr, err)
        os.Exit(2)
    }
}
'''


def build_baseline(source, destination):
    with tempfile.TemporaryDirectory(prefix="biorouter-pre-fix-helper-") as temp:
        root = Path(temp)
        app = root / "apps/OpenComputerUseWindows"
        shutil.copytree(source / "apps/OpenComputerUseWindows", app)
        subprocess.run(["git", "init", "--quiet"], cwd=root, check=True)
        subprocess.run(["git", "apply", "--reverse", "--check", str(PATCH)], cwd=root, check=True)
        subprocess.run(["git", "apply", "--reverse", str(PATCH)], cwd=root, check=True)
        subprocess.run(
            ["go", "build", "-trimpath", "-buildvcs=false", "-o", str(destination), "."],
            cwd=app,
            check=True,
        )


def build_probe(destination):
    source = destination.with_suffix(".go")
    source.write_text(PROBE_GO, encoding="utf-8")
    subprocess.run(
        ["go", "build", "-trimpath", "-buildvcs=false", "-o", str(destination), str(source)],
        check=True,
    )


def probe_environment(wrapper, report, real_powershell, control=False):
    env = dict(os.environ)
    env["PATH"] = str(wrapper.parent) + os.pathsep + env.get("PATH", "")
    env["BIOROUTER_CONSOLE_PROBE_FILE"] = str(report)
    env["BIOROUTER_REAL_POWERSHELL"] = str(real_powershell)
    env["OPEN_COMPUTER_USE_DISABLE_APP_AGENT_PROXY"] = "1"
    if control:
        env["BIOROUTER_CONSOLE_PROBE_CONTROL"] = "1"
    else:
        env.pop("BIOROUTER_CONSOLE_PROBE_CONTROL", None)
    return env


def read_probe(path, label):
    if not path.is_file():
        raise AssertionError(f"{label}: PowerShell child did not write a console verdict")
    result = json.loads(path.read_text(encoding="utf-8"))
    if result.get("verdict") not in {"visible", "hidden", "none"} or not result.get("pid"):
        raise AssertionError(f"{label}: invalid console verdict: {result}")
    return result


def run_control(wrapper, work, real_powershell):
    report = work / "control-console.json"
    completed = subprocess.run(
        [str(wrapper)],
        cwd=work,
        env=probe_environment(wrapper, report, real_powershell, control=True),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        timeout=15,
        check=False,
    )
    if completed.returncode:
        raise AssertionError(f"unflagged control failed: {completed.stderr[:1000]!r}")
    result = read_probe(report, "unflagged control")
    if result["verdict"] != "visible":
        raise AssertionError(f"unflagged control did not show a console: {result}")
    return result


def run_helper(helper, wrapper, real_powershell, work, label):
    report = work / f"{label}-console.json"
    output = work / f"{label}.stdout"
    errors = work / f"{label}.stderr"
    with output.open("wb") as stdout, errors.open("wb") as stderr:
        process = subprocess.Popen(
            [str(helper), "call", "list_apps"],
            cwd=work,
            stdout=stdout,
            stderr=stderr,
            creationflags=CREATE_NO_WINDOW,
            env=probe_environment(wrapper, report, real_powershell),
        )
        try:
            code = process.wait(timeout=45)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
            raise AssertionError(f"{label}: helper timed out")
    verdict = read_probe(report, label)
    if code:
        raise AssertionError(
            f"{label}: list_apps failed with exit {code}: "
            f"{errors.read_text(errors='replace')[:1000]}"
        )
    result = json.loads(output.read_text(encoding="utf-8-sig"))
    if not isinstance(result, dict) or result.get("isError") or not result.get("content"):
        raise AssertionError(f"{label}: list_apps did not return a successful tool result")
    return {"helper_pid": process.pid, **verdict}


def observe(patched, baseline, wrapper, real_powershell):
    with tempfile.TemporaryDirectory(prefix="biorouter-console-observer-") as temp:
        work = Path(temp)
        positive = run_control(wrapper, work, real_powershell)
        before = run_helper(baseline, wrapper, real_powershell, work, "before")
        after = run_helper(patched, wrapper, real_powershell, work, "after")
    if before["verdict"] != "visible":
        raise AssertionError(f"pre-fix helper did not show its PowerShell child: {before}")
    if after["verdict"] == "visible":
        raise AssertionError(f"patched helper showed its PowerShell child: {after}")
    return {"positive_control": positive, "pre_fix_helper": before, "patched_helper": after}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, nargs="?")
    parser.add_argument("patched", type=Path, nargs="?")
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--wrapper", type=Path)
    parser.add_argument("--real-powershell", type=Path)
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("the console probe requires Windows")
    args.report.parent.mkdir(parents=True, exist_ok=True)

    if args.worker:
        try:
            result = observe(args.patched, args.baseline, args.wrapper, args.real_powershell)
        except Exception as error:
            result = {
                "error": f"{type(error).__name__}: {error}",
                "traceback": traceback.format_exc(),
            }
        args.report.write_text(json.dumps(result, indent=2), encoding="utf-8")
        return 1 if "error" in result else 0

    pythonw = Path(sys.executable).with_name("pythonw.exe")
    if not pythonw.is_file():
        raise FileNotFoundError(f"GUI-subsystem Python is unavailable: {pythonw}")
    real_powershell = shutil.which("powershell.exe")
    if not real_powershell:
        raise FileNotFoundError("Windows PowerShell is unavailable")
    with tempfile.TemporaryDirectory(prefix="biorouter-console-baseline-") as temp:
        root = Path(temp)
        baseline = root / "ocu-before-console-fix.exe"
        wrapper = root / "powershell.exe"
        build_baseline(args.source.resolve(), baseline)
        build_probe(wrapper)
        completed = subprocess.run(
            [
                str(pythonw), __file__, "--worker", "--baseline", str(baseline),
                "--wrapper", str(wrapper), "--real-powershell", real_powershell,
                "--report", str(args.report), str(args.source), str(args.patched.resolve()),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=120,
            check=False,
        )
        if not args.report.is_file():
            raise AssertionError(f"GUI observer wrote no report (exit {completed.returncode})")
        result = json.loads(args.report.read_text(encoding="utf-8"))
        if completed.returncode or "error" in result:
            raise AssertionError(f"Windows helper console probe failed: {result}")
        print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
