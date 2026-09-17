#!/usr/bin/env python3
"""Exercise UI Automation on an isolated WinForms fixture in an interactive session."""
import argparse
import base64
import ctypes
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import tempfile
import threading
import time


def diagnostics(fixture_pid, report, snapshot):
    report.with_name(report.stem + "-native-snapshot.txt").write_text(
        "\n".join(item.get("text", "") for item in snapshot.get("content", [])), encoding="utf-8")
    script = r'''
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$process = Get-Process -Id ([int]$env:BIOROUTER_FIXTURE_PID)
$condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, [int]$env:BIOROUTER_FIXTURE_PID)
$windows = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $condition)
$rows = @()
foreach ($window in $windows) {
  $elements = $window.FindAll([System.Windows.Automation.TreeScope]::Subtree, [System.Windows.Automation.Condition]::TrueCondition)
  foreach ($element in ($elements | Select-Object -First 100)) {
    try {
      $provider = 'Not exposed by this managed UIA assembly'
      try { $provider = [string]$element.GetCurrentPropertyValue([System.Windows.Automation.AutomationElement]::ProviderDescriptionProperty) } catch {}
      $rows += [pscustomobject]@{
        name = $element.Current.Name; automationId = $element.Current.AutomationId
        framework = $element.Current.FrameworkId; className = $element.Current.ClassName
        controlType = $element.Current.ControlType.ProgrammaticName
        hwnd = $element.Current.NativeWindowHandle; offscreen = $element.Current.IsOffscreen
        providerDescription = $provider
        patterns = @($element.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName })
      }
    } catch { $rows += [pscustomobject]@{ error = $_.Exception.Message } }
  }
}
[pscustomobject]@{ processId = $process.Id; processMainWindowHandle = $process.MainWindowHandle.ToInt64(); processMainWindowTitle = $process.MainWindowTitle; apartment = [Threading.Thread]::CurrentThread.ApartmentState.ToString(); windows = $windows.Count; elements = $rows } | ConvertTo-Json -Depth 8
'''
    try:
        result = subprocess.run(["powershell.exe", "-NoProfile", "-MTA", "-EncodedCommand",
                                 base64.b64encode(script.encode("utf-16-le")).decode()],
                                env=dict(os.environ, BIOROUTER_FIXTURE_PID=str(fixture_pid)),
                                capture_output=True, text=True, timeout=20)
        report.with_name(report.stem + "-independent-uia.json").write_text(result.stdout, encoding="utf-8")
        report.with_name(report.stem + "-independent-uia-stderr.txt").write_text(result.stderr, encoding="utf-8")
        print("Independent MTA UIA diagnostics: " + result.stdout, flush=True)
    except Exception as error:
        report.with_name(report.stem + "-diagnostics-error.txt").write_text(str(error), encoding="utf-8")


def main(directory, report):
    session = ctypes.c_ulong()
    if not ctypes.windll.kernel32.ProcessIdToSessionId(os.getpid(), ctypes.byref(session)):
        raise OSError("Cannot identify Windows session")
    if session.value == 0:
        result = {"status": "desktop_unavailable", "reason": "Windows session 0 has no interactive desktop", "validated": False}
        report.write_text(json.dumps(result, indent=2))
        print("::warning::Windows interactive fixture unavailable in session 0; UI Automation was not validated")
        raise SystemExit(77)
    with tempfile.TemporaryDirectory(prefix="biorouter-computer-use-fixture-") as temp:
        work = Path(temp)
        env = dict(os.environ, BIOROUTER_FIXTURE_DIR=str(work))
        script = r'''
Add-Type -AssemblyName System.Windows.Forms
$form = New-Object System.Windows.Forms.Form
$form.Text = 'BioRouter Computer Use Fixture'
$form.Width = 480; $form.Height = 240
$form.AutoScroll = $true
$form.AutoScrollMinSize = New-Object System.Drawing.Size(450, 1200)
$form.Add_Scroll({ [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'scroll.txt'), [string][Math]::Abs($form.AutoScrollPosition.Y)) })
$form.KeyPreview = $true
$form.Add_KeyDown({ if ($_.KeyCode -eq 'F6') { [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'key.txt'), 'F6') } })
$text = New-Object System.Windows.Forms.TextBox
$text.AccessibleName = 'FixtureInput'; $text.Name = 'FixtureInput'
$text.Left = 20; $text.Top = 30; $text.Width = 400
$button = New-Object System.Windows.Forms.Button
$button.Text = 'Apply fixture'; $button.AccessibleName = 'Apply fixture'
$button.Left = 20; $button.Top = 90; $button.Width = 160
$button.Add_Click({ [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'result.txt'), $text.Text) })
$form.Controls.Add($text); $form.Controls.Add($button)
$form.Add_Shown({ [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'ready'), 'ready') })
[System.Windows.Forms.Application]::Run($form)
'''
        fixture = subprocess.Popen(["powershell.exe", "-NoProfile", "-STA", "-EncodedCommand",
                                    base64.b64encode(script.encode("utf-16-le")).decode()], env=env)
        helper = None
        try:
            deadline = time.monotonic() + 30
            while not (work / "ready").exists():
                if fixture.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("WinForms fixture did not open in the interactive session")
                time.sleep(0.1)
            helper = subprocess.Popen([str(directory / "ocu.exe"), "mcp"], stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            lines = queue.Queue()
            def reader():
                for line in helper.stdout:
                    lines.put(line)
            threading.Thread(target=reader, daemon=True).start()
            request_id = 0
            def request(method, params):
                nonlocal request_id
                request_id += 1
                helper.stdin.write(json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}) + "\n")
                helper.stdin.flush()
                deadline = time.monotonic() + 45
                while time.monotonic() < deadline:
                    reply = json.loads(lines.get(timeout=max(0.1, deadline - time.monotonic())))
                    if reply.get("id") == request_id:
                        if "error" in reply:
                            raise RuntimeError(reply["error"])
                        return reply["result"]
                raise TimeoutError(method)
            def call(name, arguments):
                result = request("tools/call", {"name": name, "arguments": arguments})
                if result.get("isError"):
                    raise RuntimeError(f"{name}: {result}")
                return result
            request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "BioRouter-fixture", "version": "1"}})
            apps = call("list_apps", {})
            if "BioRouter Computer Use Fixture" not in json.dumps(apps):
                raise AssertionError("Fixture absent from native app discovery")
            app = str(fixture.pid)
            state = call("get_app_state", {"app": app})
            diagnostics(fixture.pid, report, state)
            text = "\n".join(c.get("text", "") for c in state["content"])
            def element(label, snapshot):
                match = re.search(r"^\s*(\d+)\s+.*" + re.escape(label), snapshot, re.MULTILINE)
                if not match:
                    raise AssertionError(f"No accessibility element for {label}: {snapshot}")
                return match.group(1)
            call("set_value", {"app": app, "element_index": element("FixtureInput", text), "value": "BioRouter native fixture verified"})
            call("type_text", {"app": app, "text": " by typing"})
            call("press_key", {"app": app, "key": "F6"})
            state = call("get_app_state", {"app": app})
            text = "\n".join(c.get("text", "") for c in state["content"])
            call("click", {"app": app, "element_index": element("Apply fixture", text), "click_method": "accessibility"})
            deadline = time.monotonic() + 10
            while not (work / "result.txt").exists() and time.monotonic() < deadline:
                time.sleep(0.1)
            if (work / "result.txt").read_text() != "BioRouter native fixture verified by typing":
                raise AssertionError("Independent fixture result did not match typed value")
            if (work / "key.txt").read_text() != "F6":
                raise AssertionError("Independent fixture did not receive the key")
            state = call("get_app_state", {"app": app})
            text = "\n".join(c.get("text", "") for c in state["content"])
            call("scroll", {"app": app, "element_index": element("FixtureInput", text), "direction": "down", "pages": 2})
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if (work / "scroll.txt").exists() and float((work / "scroll.txt").read_text() or "0") > 0:
                    break
                time.sleep(0.1)
            else:
                raise AssertionError("Independent WinForms scroll offset did not change")
            capture = call("screen_capture", {})
            if not any(c.get("type") == "image" and base64.b64decode(c.get("data", "")).startswith(b"\x89PNG") for c in capture["content"]):
                raise AssertionError("Native capture returned no PNG")
            result = {"status": "passed", "validated": True, "session": session.value,
                      "checks": ["list_apps", "get_app_state", "set_value", "type_text", "press_key", "click", "independent fixture state", "scroll offset changed", "screen_capture"],
                      "not_validated": ["drag", "mixed DPI", "multiple monitors", "occluded windows", "secure desktop"]}
            report.write_text(json.dumps(result, indent=2))
            print(json.dumps(result))
        finally:
            for process in (helper, fixture):
                if process and process.poll() is None:
                    subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], check=False, capture_output=True)
                    process.wait(timeout=10)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--report", type=Path, default=Path("computer-use-windows-fixture.json"))
    args = parser.parse_args()
    main(args.directory.resolve(), args.report)
