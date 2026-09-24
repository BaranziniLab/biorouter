#!/usr/bin/env python3
"""Measure a packaged CLI's Rust-to-Go Copilot PowerShell launch on Windows."""

import argparse
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import traceback


CREATE_NO_WINDOW = 0x08000000
PROBE_GO = r'''
package main

import (
    "encoding/json"
    "fmt"
    "os"
    "os/exec"
    "path/filepath"
    "strings"
    "syscall"
    "unsafe"
)

func consoleVerdict() string {
    window, _, _ := syscall.NewLazyDLL("kernel32.dll").NewProc("GetConsoleWindow").Call()
    if window == 0 {
        return "none"
    }
    visible, _, _ := syscall.NewLazyDLL("user32.dll").NewProc("IsWindowVisible").Call(window)
    if visible != 0 {
        return "visible"
    }
    return "hidden"
}

func ancestry() (int, string, int, string) {
    parentPID := os.Getppid()
    snapshot, err := syscall.CreateToolhelp32Snapshot(syscall.TH32CS_SNAPPROCESS, 0)
    if err != nil {
        return parentPID, "", 0, ""
    }
    defer syscall.CloseHandle(snapshot)
    var entry syscall.ProcessEntry32
    entry.Size = uint32(unsafe.Sizeof(entry))
    if err := syscall.Process32First(snapshot, &entry); err != nil {
        return parentPID, "", 0, ""
    }
    processes := map[uint32]syscall.ProcessEntry32{}
    for {
        processes[entry.ProcessID] = entry
        if err := syscall.Process32Next(snapshot, &entry); err != nil {
            break
        }
    }
    parent, found := processes[uint32(parentPID)]
    if !found {
        return parentPID, "", 0, ""
    }
    grandparent := processes[parent.ParentProcessID]
    return parentPID, syscall.UTF16ToString(parent.ExeFile[:]),
        int(parent.ParentProcessID), syscall.UTF16ToString(grandparent.ExeFile[:])
}

func main() {
    executable, err := os.Executable()
    if err != nil {
        fmt.Fprintln(os.Stderr, err)
        os.Exit(2)
    }
    directory := filepath.Dir(executable)
    parentPID, parentImage, grandparentPID, grandparentImage := ancestry()
    control := len(os.Args) == 2 && os.Args[1] == "--probe-only"
    record, err := json.Marshal(map[string]any{
        "pid": os.Getpid(), "parent_pid": parentPID,
        "parent_image": parentImage, "grandparent_pid": grandparentPID,
        "grandparent_image": grandparentImage,
        "mode": map[bool]string{true: "control", false: "forward"}[control],
        "verdict": consoleVerdict(),
    })
    if err == nil {
        err = os.WriteFile(filepath.Join(directory, fmt.Sprintf("console-%d.json", os.Getpid())), record, 0600)
    }
    if err != nil {
        fmt.Fprintln(os.Stderr, err)
        os.Exit(2)
    }
    if control {
        return
    }
    real, err := os.ReadFile(filepath.Join(directory, "real-powershell.txt"))
    if err != nil {
        fmt.Fprintln(os.Stderr, err)
        os.Exit(2)
    }
    child := exec.Command(strings.TrimSpace(string(real)), os.Args[1:]...)
    child.SysProcAttr = &syscall.SysProcAttr{CreationFlags: 0x08000000, HideWindow: true}
    child.Stdout, child.Stderr = os.Stdout, os.Stderr
    if err := child.Run(); err != nil {
        if exit, ok := err.(*exec.ExitError); ok {
            os.Exit(exit.ExitCode())
        }
        fmt.Fprintln(os.Stderr, err)
        os.Exit(2)
    }
}
'''


def records(directory, mode):
    found = [json.loads(path.read_text(encoding="utf-8"))
             for path in directory.glob("console-*.json")]
    return [item for item in found if item.get("mode") == mode]


def observe(cli, wrapper):
    directory = wrapper.parent
    control = subprocess.run([str(wrapper), "--probe-only"], cwd=directory,
                             stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                             timeout=15, check=False)
    positive = records(directory, "control")
    if control.returncode or len(positive) != 1 or positive[0].get("verdict") != "visible":
        raise AssertionError(f"GUI positive control did not show a console: {positive}, exit={control.returncode}")

    env = dict(os.environ)
    env.pop("BIOROUTER_COMPUTER_USE_DIR", None)
    env["PATH"] = str(directory) + os.pathsep + env.get("PATH", "")
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "Biorouter-package-console-check", "version": "1"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "_meta": {"biorouter-session-id": "packaged-console-fixture",
                      "computer_use_generation": "packaged-console-fixture"},
            "name": "list_apps", "arguments": {}}},
    ]
    errors_path = directory / "packaged-cli.stderr"
    errors_file = errors_path.open("w", encoding="utf-8")
    process = subprocess.Popen([str(cli), "mcp", "computercontroller"], cwd=directory,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors_file,
                               text=True, encoding="utf-8", errors="replace",
                               creationflags=CREATE_NO_WINDOW, env=env)
    lines = queue.Queue()
    def read_lines():
        for line in process.stdout:
            lines.put(line)
        lines.put(None)
    threading.Thread(target=read_lines, daemon=True).start()
    replies = {}
    try:
        for message in messages:
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()
            if "id" not in message:
                continue
            deadline = time.monotonic() + 90
            while message["id"] not in replies:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise AssertionError(f"packaged MCP request {message['id']} timed out")
                try:
                    line = lines.get(timeout=remaining)
                except queue.Empty as error:
                    raise AssertionError(f"packaged MCP request {message['id']} timed out") from error
                if line is None:
                    raise AssertionError(f"packaged MCP exited before response {message['id']}")
                reply = json.loads(line)
                if "id" in reply:
                    replies[reply["id"]] = reply
        process.stdin.close()
        process.wait(timeout=15)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        errors_file.close()
    errors = errors_path.read_text(encoding="utf-8", errors="replace")
    if process.returncode or not replies.get(1, {}).get("result", {}).get("serverInfo"):
        raise AssertionError(f"packaged MCP initialization failed: exit={process.returncode}, stderr={errors[:1000]!r}")
    result = replies.get(2, {}).get("result", {})
    if replies.get(2, {}).get("error") or result.get("isError") or not result.get("content"):
        raise AssertionError(f"packaged list_apps failed: reply={replies.get(2)}, stderr={errors[:1000]!r}")

    children = records(directory, "forward")
    if not children:
        raise AssertionError("packaged Rust-to-Go call launched no instrumented PowerShell child")
    for child in children:
        if (child.get("parent_image", "").lower() != "ocu.exe"
                or child.get("grandparent_image", "").lower() != "biorouter.exe"
                or child.get("grandparent_pid") != process.pid):
            raise AssertionError(f"PowerShell child did not descend from the packaged CLI and helper: {child}")
        if child.get("verdict") == "visible":
            raise AssertionError(f"packaged Copilot PowerShell child showed a console: {child}")
        if child.get("verdict") not in {"none", "hidden"}:
            raise AssertionError(f"PowerShell child did not report a console state: {child}")
    return {"positive_control": positive[0], "packaged_cli_pid": process.pid,
            "native_power_shell_children": children, "list_apps_success": True,
            "coverage": "packaged CLI -> Rust MCP -> packaged Go helper -> PowerShell child"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--wrapper", type=Path)
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("the packaged console probe requires Windows")
    report = args.report.resolve()
    report.parent.mkdir(parents=True, exist_ok=True)
    if args.worker:
        try:
            result = observe(args.cli.resolve(), args.wrapper.resolve())
        except Exception as error:
            result = {"error": f"{type(error).__name__}: {error}", "traceback": traceback.format_exc()}
        report.write_text(json.dumps(result, indent=2), encoding="utf-8")
        return 1 if "error" in result else 0

    try:
        pythonw = Path(sys.executable).with_name("pythonw.exe")
        if not pythonw.is_file():
            raise FileNotFoundError(f"GUI-subsystem Python is unavailable: {pythonw}")
        real_powershell = shutil.which("powershell.exe")
        if not real_powershell:
            raise FileNotFoundError("Windows PowerShell is unavailable")
        with tempfile.TemporaryDirectory(prefix="biorouter-packaged-console-") as temp:
            directory = Path(temp)
            wrapper = directory / "powershell.exe"
            source = directory / "console_probe.go"
            source.write_text(PROBE_GO, encoding="utf-8")
            (directory / "real-powershell.txt").write_text(real_powershell, encoding="utf-8")
            subprocess.run(["go", "build", "-trimpath", "-buildvcs=false", "-o", str(wrapper), str(source)],
                           check=True, timeout=90)
            completed = subprocess.run(
                [str(pythonw), str(Path(__file__).resolve()), "--worker", "--cli", str(args.cli.resolve()),
                 "--wrapper", str(wrapper), "--report", str(report)],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120, check=False)
            if not report.is_file():
                raise AssertionError(f"GUI observer wrote no report (exit {completed.returncode})")
            result = json.loads(report.read_text(encoding="utf-8"))
            if completed.returncode or "error" in result:
                raise AssertionError(f"packaged Windows console check failed: {result}")
            print(json.dumps(result, sort_keys=True))
    except Exception as error:
        if not report.is_file():
            report.write_text(json.dumps({"error": f"{type(error).__name__}: {error}",
                                          "traceback": traceback.format_exc()}, indent=2), encoding="utf-8")
        raise
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
