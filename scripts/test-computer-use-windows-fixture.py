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
    native_doctor = subprocess.run([str(directory / "ocu.exe"), "doctor", "--json"],
                                   capture_output=True, text=True, timeout=20, check=True)
    json.loads(native_doctor.stdout)
    report.with_name(report.stem + "-native-doctor.json").write_text(native_doctor.stdout, encoding="utf-8")
    print("Native helper passive doctor: " + native_doctor.stdout, flush=True)
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
Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.IO;
using System.Windows.Forms;
public class BioRouterFixtureForm : Form {
    public Control ScrollTarget;
    public string DiagnosticsDirectory;
    protected override void WndProc(ref Message message) {
        if ((message.Msg == 0x020A || message.Msg == 0x020E) && ScrollTarget != null) {
            long packed = message.LParam.ToInt64();
            int x = (short)(packed & 0xffff), y = (short)((packed >> 16) & 0xffff);
            Rectangle expected = ScrollTarget.Parent.RectangleToScreen(ScrollTarget.Bounds);
            int delta = (short)((message.WParam.ToInt64() >> 16) & 0xffff);
            string data = String.Format("{{\"message\":{0},\"x\":{1},\"y\":{2},\"expected_x\":{3},\"expected_y\":{4},\"delta\":{5}}}",
                message.Msg, x, y, expected.Left + expected.Width / 2, expected.Top + expected.Height / 2, delta);
            File.WriteAllText(Path.Combine(DiagnosticsDirectory, "wheel.json"), data);
        }
        base.WndProc(ref message);
        // Mouse-wheel scrolling does not reliably raise WinForms' Scroll event.
        if ((message.Msg == 0x020A || message.Msg == 0x020E) && DiagnosticsDirectory != null) {
            File.WriteAllText(Path.Combine(DiagnosticsDirectory, "scroll-x.txt"), Math.Abs(AutoScrollPosition.X).ToString());
            File.WriteAllText(Path.Combine(DiagnosticsDirectory, "scroll-y.txt"), Math.Abs(AutoScrollPosition.Y).ToString());
        }
    }
}
'@
$form = New-Object BioRouterFixtureForm
$form.DiagnosticsDirectory = $env:BIOROUTER_FIXTURE_DIR
$form.Text = 'BioRouter Computer Use Fixture'
$form.Width = 480; $form.Height = 240
$form.StartPosition = 'Manual'; $form.Location = New-Object System.Drawing.Point(140, 100)
$form.AutoScroll = $true
$form.AutoScrollMinSize = New-Object System.Drawing.Size(1200, 1200)
$form.Add_Scroll({
  [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'scroll-x.txt'), [string][Math]::Abs($form.AutoScrollPosition.X))
  [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'scroll-y.txt'), [string][Math]::Abs($form.AutoScrollPosition.Y))
})
$form.KeyPreview = $true
$form.Add_KeyDown({ if ($_.KeyCode -eq 'F6') { [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'key.txt'), 'F6') } })
$text = New-Object System.Windows.Forms.TextBox
$text.AccessibleName = 'FixtureInput'; $text.Name = 'FixtureInput'
$text.Left = 20; $text.Top = 30; $text.Width = 400
$form.ScrollTarget = $text
$button = New-Object System.Windows.Forms.Button
$button.Text = 'Apply fixture'; $button.AccessibleName = 'Apply fixture'
$button.Left = 20; $button.Top = 90; $button.Width = 160
$button.Add_Click({ [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'result.txt'), $text.Text) })
$reset = New-Object System.Windows.Forms.Button
$reset.Text = 'Reset scroll'; $reset.AccessibleName = 'Reset scroll'
$reset.Left = 200; $reset.Top = 90; $reset.Width = 160
$reset.Add_Click({ $form.AutoScrollPosition = New-Object System.Drawing.Point(0, 0) })
$form.Controls.Add($text); $form.Controls.Add($button); $form.Controls.Add($reset)
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 100
$timer.Add_Tick({
  [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'scroll-x.txt'), [string][Math]::Abs($form.AutoScrollPosition.X))
  [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'scroll-y.txt'), [string][Math]::Abs($form.AutoScrollPosition.Y))
})
$form.Add_Shown({ $timer.Start(); [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'ready'), 'ready') })
[System.Windows.Forms.Application]::Run($form)
$timer.Dispose()
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
                receipt = {
                    "tool": name, "arguments": arguments, "isError": result.get("isError", False),
                    "text": [item.get("text", "") for item in result.get("content", []) if item.get("type") == "text"],
                    "image_count": sum(item.get("type") == "image" for item in result.get("content", [])),
                }
                report.with_name(f"{report.stem}-{request_id:02d}-{name}.json").write_text(json.dumps(receipt, indent=2), encoding="utf-8")
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
            def apply_and_expect(expected, phase):
                (work / "result.txt").unlink(missing_ok=True)
                snapshot = call("get_app_state", {"app": app})
                text = "\n".join(c.get("text", "") for c in snapshot["content"])
                call("click", {"app": app, "element_index": element("Apply fixture", text), "click_method": "accessibility"})
                deadline = time.monotonic() + 10
                while not (work / "result.txt").exists() and time.monotonic() < deadline:
                    time.sleep(0.1)
                actual = (work / "result.txt").read_text()
                report.with_name(f"{report.stem}-{phase}-independent-value.json").write_text(
                    json.dumps({"expected": expected, "actual": actual}, indent=2), encoding="utf-8")
                if actual != expected:
                    raise AssertionError(f"Independent {phase} value mismatch: expected {expected!r}, received {actual!r}")
            call("set_value", {"app": app, "element_index": element("FixtureInput", text), "value": "BioRouter value fixture verified"})
            apply_and_expect("BioRouter value fixture verified", "set_value")
            state = call("get_app_state", {"app": app})
            text = "\n".join(c.get("text", "") for c in state["content"])
            call("set_value", {"app": app, "element_index": element("FixtureInput", text), "value": ""})
            call("type_text", {"app": app, "text": "BioRouter typing fixture verified"})
            apply_and_expect("BioRouter typing fixture verified", "type_text")
            call("press_key", {"app": app, "key": "F6"})
            if (work / "key.txt").read_text() != "F6":
                raise AssertionError("Independent fixture did not receive the key")
            state = call("get_app_state", {"app": app})
            text = "\n".join(c.get("text", "") for c in state["content"])
            call("scroll", {"app": app, "element_index": element("FixtureInput", text), "direction": "down", "pages": 2})
            scroll_failures = []
            def wait_scroll(axis):
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    receipt = work / f"scroll-{axis}.txt"
                    if receipt.exists() and float(receipt.read_text() or "0") > 0:
                        return float(receipt.read_text())
                    time.sleep(0.1)
                wheel_path = work / "wheel.json"
                wheel_receipt = wheel_path.read_text() if wheel_path.exists() else "No wheel message received"
                report.with_name(f"{report.stem}-{axis}-wheel-failure.txt").write_text(wheel_receipt, encoding="utf-8")
                scroll_failures.append(f"Independent WinForms {axis} scroll offset did not change; wheel: {wheel_receipt}")
                return 0.0
            vertical_offset = wait_scroll("y")
            wheel = json.loads((work / "wheel.json").read_text())
            report.with_name(report.stem + "-wheel-coordinates.json").write_text(json.dumps(wheel, indent=2))
            if wheel["message"] != 0x020A or abs(wheel["x"] - wheel["expected_x"]) > 1 or abs(wheel["y"] - wheel["expected_y"]) > 1:
                scroll_failures.append(f"Mouse wheel did not carry the target's screen coordinates: {wheel}")
            state = call("get_app_state", {"app": app})
            text = "\n".join(c.get("text", "") for c in state["content"])
            call("click", {"app": app, "element_index": element("Reset scroll", text), "click_method": "accessibility"})
            state = call("get_app_state", {"app": app})
            text = "\n".join(c.get("text", "") for c in state["content"])
            (work / "wheel.json").unlink(missing_ok=True)
            call("scroll", {"app": app, "element_index": element("FixtureInput", text), "direction": "right", "pages": 2})
            horizontal_offset = wait_scroll("x")
            horizontal_wheel = json.loads((work / "wheel.json").read_text()) if (work / "wheel.json").exists() else None
            report.with_name(report.stem + "-horizontal-wheel-coordinates.json").write_text(json.dumps(horizontal_wheel, indent=2))
            if horizontal_wheel is not None and (horizontal_wheel["message"] != 0x020E or abs(horizontal_wheel["x"] - horizontal_wheel["expected_x"]) > 1 or abs(horizontal_wheel["y"] - horizontal_wheel["expected_y"]) > 1):
                scroll_failures.append(f"Horizontal mouse wheel did not carry the target's screen coordinates: {horizontal_wheel}")
            report.with_name(report.stem + "-independent-scroll.json").write_text(json.dumps({
                "down_y": vertical_offset, "right_x": horizontal_offset, "failures": scroll_failures}, indent=2))
            capture = call("screen_capture", {})
            if not any(c.get("type") == "image" and base64.b64decode(c.get("data", "")).startswith(b"\x89PNG") for c in capture["content"]):
                raise AssertionError("Native capture returned no PNG")
            if scroll_failures:
                raise AssertionError("; ".join(scroll_failures))
            result = {"status": "passed", "validated": True, "session": session.value,
                      "scroll_offsets": {"down_y": vertical_offset, "right_x": horizontal_offset},
                      "checks": ["list_apps", "get_app_state", "set_value", "type_text", "press_key", "click", "independent fixture state", "vertical and horizontal scroll offsets changed", "wheel screen coordinates", "screen_capture"],
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
