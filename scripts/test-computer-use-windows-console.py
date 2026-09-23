#!/usr/bin/env python3
"""Check the Windows Copilot helper's PowerShell child from a GUI process."""

import argparse
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
PATCH = ROOT / "vendor/computer-use/patches/0003-windows-console.patch"
CREATE_NO_WINDOW = 0x08000000
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
SYNCHRONIZE = 0x00100000
WAIT_TIMEOUT = 0x00000102
TH32CS_SNAPPROCESS = 0x00000002


class ProcessEntry(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD),
        ("cntUsage", wintypes.DWORD),
        ("th32ProcessID", wintypes.DWORD),
        ("th32DefaultHeapID", ctypes.c_void_p),
        ("th32ModuleID", wintypes.DWORD),
        ("cntThreads", wintypes.DWORD),
        ("th32ParentProcessID", wintypes.DWORD),
        ("pcPriClassBase", wintypes.LONG),
        ("dwFlags", wintypes.DWORD),
        ("szExeFile", wintypes.WCHAR * 260),
    ]


def windows_api():
    if os.name != "nt":
        raise RuntimeError("the console probe requires Windows")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    user = ctypes.WinDLL("user32", use_last_error=True)
    kernel.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)]
    kernel.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessEntry)]
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.AttachConsole.argtypes = [wintypes.DWORD]
    kernel.GetConsoleWindow.restype = wintypes.HWND
    user.IsWindowVisible.argtypes = [wintypes.HWND]
    return kernel, user


def child_powershell_pids(kernel, parent_pid):
    snapshot = kernel.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if snapshot == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    found = []
    try:
        entry = ProcessEntry()
        entry.dwSize = ctypes.sizeof(entry)
        ok = kernel.Process32FirstW(snapshot, ctypes.byref(entry))
        while ok:
            if entry.th32ParentProcessID == parent_pid and entry.szExeFile.lower() == "powershell.exe":
                found.append(entry.th32ProcessID)
            ok = kernel.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel.CloseHandle(snapshot)
    return found


def console_state(kernel, user, pid):
    process = kernel.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, False, pid)
    if not process:
        return None
    try:
        if kernel.WaitForSingleObject(process, 0) != WAIT_TIMEOUT:
            return None
        kernel.FreeConsole()
        if not kernel.AttachConsole(pid):
            error = ctypes.get_last_error()
            if error == 6:  # ERROR_INVALID_HANDLE: the child has no console.
                return "none"
            raise ctypes.WinError(error)
        try:
            window = kernel.GetConsoleWindow()
            return "visible" if window and user.IsWindowVisible(window) else "hidden"
        finally:
            kernel.FreeConsole()
    finally:
        kernel.CloseHandle(process)


def watch(kernel, user, process, child_of=None, timeout=35):
    states = []
    children = set()
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        pids = [process.pid] if child_of is None else child_powershell_pids(kernel, child_of)
        for pid in pids:
            state = console_state(kernel, user, pid)
            if state is not None:
                children.add(pid)
                states.append(state)
        if process.poll() is not None:
            break
        time.sleep(0.01)
    if process.poll() is None:
        process.kill()
        raise TimeoutError(f"process {process.pid} did not finish")
    if not states:
        raise AssertionError(f"no live console samples from process {process.pid}")
    return {"parent_pid": child_of, "child_pids": sorted(children), "states": sorted(set(states))}


def run_helper(kernel, user, helper, label, work):
    output = work / f"{label}.stdout"
    errors = work / f"{label}.stderr"
    with output.open("wb") as stdout, errors.open("wb") as stderr:
        process = subprocess.Popen(
            [str(helper), "call", "list_apps"],
            stdout=stdout,
            stderr=stderr,
            creationflags=CREATE_NO_WINDOW,
            env=dict(os.environ, OPEN_COMPUTER_USE_DISABLE_APP_AGENT_PROXY="1"),
        )
        observation = watch(kernel, user, process, child_of=process.pid)
        code = process.wait(timeout=5)
    if code:
        raise AssertionError(f"{label} UIA call failed: exit {code}, stderr={errors.read_text(errors='replace')[:1000]}")
    result = json.loads(output.read_text(encoding="utf-8-sig"))
    if not isinstance(result, dict) or result.get("isError") or not result.get("content"):
        raise AssertionError(f"{label} list_apps did not return a successful tool result")
    return observation


def observe(patched, baseline):
    kernel, user = windows_api()
    if kernel.GetConsoleWindow():
        raise AssertionError("observer has a console; pythonw.exe must be the GUI parent")
    with tempfile.TemporaryDirectory(prefix="biorouter-console-observer-") as temp:
        work = Path(temp)
        with (work / "control.stdout").open("wb") as stdout:
            control = subprocess.Popen(
                ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", "Start-Sleep -Seconds 3"],
                stdout=stdout,
                stderr=subprocess.STDOUT,
            )
            positive = watch(kernel, user, control, timeout=8)
            control.wait(timeout=5)
        if "visible" not in positive["states"]:
            raise AssertionError(f"unflagged PowerShell positive control was not visible: {positive}")
        before = run_helper(kernel, user, baseline, "before", work)
        after = run_helper(kernel, user, patched, "after", work)
    if "visible" not in before["states"]:
        raise AssertionError(f"pre-fix helper did not reproduce the visible child: {before}")
    if "visible" in after["states"]:
        raise AssertionError(f"patched helper still showed a PowerShell console: {after}")
    return {"positive_control": positive, "pre_fix_helper": before, "patched_helper": after}


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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, nargs="?")
    parser.add_argument("patched", type=Path, nargs="?")
    parser.add_argument("--report", type=Path)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--baseline", type=Path)
    args = parser.parse_args()
    if args.worker:
        try:
            result = observe(args.patched, args.baseline)
        except Exception as error:
            result = {"error": f"{type(error).__name__}: {error}"}
        args.report.write_text(json.dumps(result, indent=2), encoding="utf-8")
        return 1 if "error" in result else 0
    if os.name != "nt":
        parser.error("the console probe requires Windows")
    pythonw = Path(sys.executable).with_name("pythonw.exe")
    if not pythonw.is_file():
        raise FileNotFoundError(f"GUI-subsystem Python is unavailable: {pythonw}")
    args.report.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="biorouter-console-baseline-") as temp:
        baseline = Path(temp) / "ocu-before-console-fix.exe"
        build_baseline(args.source.resolve(), baseline)
        completed = subprocess.run(
            [str(pythonw), __file__, "--worker", "--baseline", str(baseline),
             "--report", str(args.report), str(args.source), str(args.patched.resolve())],
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
