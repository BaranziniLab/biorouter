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



def read_fixture_json(path, timeout=2):
    deadline = time.monotonic() + timeout
    while True:
        try:
            return json.loads(path.read_text(encoding="utf-8-sig"))
        except (FileNotFoundError, PermissionError, json.JSONDecodeError) as error:
            if time.monotonic() >= deadline:
                raise TimeoutError(f"Fixture telemetry remained unreadable: {path}") from error
            time.sleep(0.02)


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



FIXTURE_TITLE = "Biorouter Copilot Fixture"
SENTINEL_TITLE = "BioRouter Unrelated Window Sentinel"


def capture_metadata(result):
    metadata = []
    for item in result.get("content", []):
        if item.get("type") == "text":
            try:
                value = json.loads(item.get("text", ""))
            except json.JSONDecodeError:
                continue
            if isinstance(value, dict) and isinstance(value.get("windows"), list):
                metadata.append(value)
    if len(metadata) != 1:
        raise AssertionError("Capture must return exactly one windows metadata object")
    return metadata[0]


def validate_capture_scope(result, target_window, list_only=False):
    metadata = capture_metadata(result)
    windows = metadata["windows"]
    if len(windows) != 1 or windows[0] != target_window or windows[0].get("title") != FIXTURE_TITLE:
        raise AssertionError(f"Targeted capture disclosed windows outside its selected target: {windows}")
    if SENTINEL_TITLE in "\n".join(item.get("text", "") for item in result.get("content", [])):
        raise AssertionError("Targeted capture leaked unrelated sentinel metadata")
    if list_only and any(item.get("type") == "image" for item in result.get("content", [])):
        raise AssertionError("Metadata-only capture unexpectedly returned pixels")
    return metadata


def validate_drag(result):
    if not result.get("released") or result.get("down") or result.get("moves", 0) < 2 or abs(result.get("end_x", 0) - result.get("start_x", 0) - 120) > 3:
        raise AssertionError(f"Independent child drag did not complete the requested 120px gesture: {result}")


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
public class BioRouterDragSurface : Control {
    public string DiagnosticsDirectory;
    private bool down, released;
    private int moves, startX, endX;
    protected override void OnPaint(PaintEventArgs e) {
        base.OnPaint(e); e.Graphics.FillRectangle(Brushes.Blue, moves > 0 ? endX : 30, 5, 20, 30);
    }
    private void Record() {
        Invalidate();
        File.WriteAllText(Path.Combine(DiagnosticsDirectory, "drag-result.json"),
            String.Format("{{\"down\":{0},\"released\":{1},\"moves\":{2},\"start_x\":{3},\"end_x\":{4}}}",
                down.ToString().ToLowerInvariant(), released.ToString().ToLowerInvariant(), moves, startX, endX));
    }
    protected override void OnMouseDown(MouseEventArgs e) {
        base.OnMouseDown(e);
        if (e.Button != MouseButtons.Left) return;
        down = true; released = false; moves = 0; startX = endX = e.X; Capture = true; Record();
    }
    protected override void OnMouseMove(MouseEventArgs e) {
        base.OnMouseMove(e);
        if (down && (e.Button & MouseButtons.Left) != 0) { moves++; endX = e.X; Record(); }
    }
    protected override void OnMouseUp(MouseEventArgs e) {
        base.OnMouseUp(e);
        if (down && e.Button == MouseButtons.Left) { down = false; released = true; endX = e.X; Capture = false; Record(); }
    }
}
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
$form.Text = 'Biorouter Copilot Fixture'
$form.Width = 480; $form.Height = 240
$form.StartPosition = 'Manual'; $form.Location = New-Object System.Drawing.Point(140, 100)
$form.AutoScroll = $true
$form.AutoScrollMinSize = New-Object System.Drawing.Size(4000, 4000)
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
$drag = New-Object BioRouterDragSurface
$drag.DiagnosticsDirectory = $env:BIOROUTER_FIXTURE_DIR
$drag.AccessibleName = 'FixtureDrag'; $drag.Name = 'FixtureDrag'
$drag.Left = 20; $drag.Top = 130; $drag.Width = 320; $drag.Height = 40
$drag.BackColor = [System.Drawing.Color]::LightBlue
$form.Controls.Add($drag)
$form.Controls.Add($text); $form.Controls.Add($button); $form.Controls.Add($reset)
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 100
$timer.Add_Tick({
  @{x = [Math]::Abs($form.AutoScrollPosition.X); y = [Math]::Abs($form.AutoScrollPosition.Y); page_x = $form.ClientSize.Width; page_y = $form.ClientSize.Height; max_x = $form.DisplayRectangle.Width - $form.ClientSize.Width; max_y = $form.DisplayRectangle.Height - $form.ClientSize.Height; native_x = $form.HorizontalScroll.Value; native_y = $form.VerticalScroll.Value; native_page_x = $form.HorizontalScroll.LargeChange; native_page_y = $form.VerticalScroll.LargeChange; native_max_x = $form.HorizontalScroll.Maximum; native_max_y = $form.VerticalScroll.Maximum; native_step_x = $form.HorizontalScroll.SmallChange; native_step_y = $form.VerticalScroll.SmallChange} | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $env:BIOROUTER_FIXTURE_DIR 'scroll-metrics.json')

  [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'scroll-x.txt'), [string][Math]::Abs($form.AutoScrollPosition.X))
  [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'scroll-y.txt'), [string][Math]::Abs($form.AutoScrollPosition.Y))
})
$form.Add_Shown({
  $origin = $drag.PointToScreen((New-Object System.Drawing.Point(30, 20)))
  @{from_x = $origin.X - $form.Bounds.X; from_y = $origin.Y - $form.Bounds.Y; to_x = $origin.X - $form.Bounds.X + 120; to_y = $origin.Y - $form.Bounds.Y} | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $env:BIOROUTER_FIXTURE_DIR 'drag-geometry.json')
  $timer.Start(); [System.IO.File]::WriteAllText((Join-Path $env:BIOROUTER_FIXTURE_DIR 'ready'), 'ready') })
$sentinel = New-Object System.Windows.Forms.Form
$sentinel.Text = 'BioRouter Unrelated Window Sentinel'
$sentinel.Width = 220; $sentinel.Height = 100
$sentinel.StartPosition = 'Manual'; $sentinel.Location = New-Object System.Drawing.Point(750, 10)
$sentinel.Show()
[System.Windows.Forms.Application]::Run($form)
$sentinel.Dispose()
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
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                      env=dict(os.environ, OPEN_COMPUTER_USE_ALLOW_GLOBAL_POINTER_FALLBACKS="1"))
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
            def call(name, arguments, allow_error=False):
                result = request("tools/call", {"name": name, "arguments": arguments})
                receipt = {
                    "tool": name, "arguments": arguments, "isError": result.get("isError", False),
                    "text": [item.get("text", "") for item in result.get("content", []) if item.get("type") == "text"],
                    "image_count": sum(item.get("type") == "image" for item in result.get("content", [])),
                }
                report.with_name(f"{report.stem}-{request_id:02d}-{name}.json").write_text(json.dumps(receipt, indent=2), encoding="utf-8")
                if result.get("isError") and not allow_error:
                    raise RuntimeError(f"{name}: {result}")
                return result
            request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "BioRouter-fixture", "version": "1"}})
            apps = call("list_apps", {})
            if "Biorouter Copilot Fixture" not in json.dumps(apps):
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
            call("get_app_state", {"app": app})
            coordinates = read_fixture_json(work / "drag-geometry.json")
            call("drag", {"app": app, **coordinates})
            deadline = time.monotonic() + 10
            drag_result = {}
            while time.monotonic() < deadline:
                try:
                    drag_result = json.loads((work / "drag-result.json").read_text())
                except (FileNotFoundError, json.JSONDecodeError):
                    pass
                if drag_result.get("released"):
                    break
                time.sleep(0.1)
            report.with_name(report.stem + "-independent-drag.json").write_text(json.dumps(drag_result, indent=2))
            validate_drag(drag_result)
            scroll_receipts = []
            for direction, pages in [("down", 0.5), ("up", 0.5), ("down", 2.5), ("up", 2.5), ("right", 0.5), ("left", 0.5), ("right", 2.5), ("left", 2.5)]:
                state = call("get_app_state", {"app": app})
                text = "\n".join(c.get("text", "") for c in state["content"])
                before = read_fixture_json(work / "scroll-metrics.json")
                axis = "x" if direction in {"left", "right"} else "y"
                sign = -1 if direction in {"up", "left"} else 1
                expected = min(before["max_" + axis], max(0, before[axis] + sign * pages * before["page_" + axis]))
                scroll_result = call("scroll", {"app": app, "element_index": element("FixtureInput", text), "direction": direction, "pages": pages}, allow_error=True)
                time.sleep(0.2)
                actual = read_fixture_json(work / "scroll-metrics.json")
                receipt = {"direction": direction, "pages": pages, "before": before[axis], "expected": expected, "actual": actual[axis], "before_metrics": before, "after_metrics": actual, "tool_result": scroll_result, "granularity": before["native_step_" + axis]}
                scroll_receipts.append(receipt)
                report.with_name(report.stem + "-independent-scroll.json").write_text(json.dumps(scroll_receipts, indent=2))
                if scroll_result.get("isError"):
                    raise AssertionError(f"Scroll failed; independent metrics preserved: {receipt}")
                if expected != before[axis] and (actual[axis] - before[axis]) * sign <= 0:
                    raise AssertionError(f"Scroll made no direction-consistent movement: {receipt}")
                if abs(actual[axis] - expected) > before["native_step_" + axis] / 2:
                    raise AssertionError(f"Requested WinForms viewport displacement not observed: {receipt}")
            state = call("get_app_state", {"app": app})
            text = "\n".join(c.get("text", "") for c in state["content"])
            for invalid in [None, True, "2", 0, -1, 101]:
                before = read_fixture_json(work / "scroll-metrics.json")
                rejected = call("scroll", {"app": app, "element_index": element("FixtureInput", text), "direction": "down", "pages": invalid}, allow_error=True)
                if not rejected.get("isError") or read_fixture_json(work / "scroll-metrics.json") != before:
                    raise AssertionError(f"Invalid pages caused input or silently defaulted: {invalid!r}")
            inventory = capture_metadata(call("screen_capture", {"list_only": True}))
            target_windows = [window for window in inventory["windows"] if window.get("title") == FIXTURE_TITLE]
            if len(target_windows) != 1 or not any(window.get("title") == SENTINEL_TITLE for window in inventory["windows"]):
                raise AssertionError("Capture isolation fixture must expose both target and unrelated sentinel windows")
            target_window = target_windows[0]
            targeted_capture = call("screen_capture", {"window_title": FIXTURE_TITLE})
            scoped_capture = validate_capture_scope(targeted_capture, target_window)
            if not any(c.get("type") == "image" and base64.b64decode(c.get("data", "")).startswith(b"\x89PNG") for c in targeted_capture["content"]):
                raise AssertionError("Targeted native capture returned no PNG")
            scoped_list = validate_capture_scope(call("screen_capture", {"window_title": FIXTURE_TITLE, "list_only": True}), target_window, list_only=True)
            report.with_name(report.stem + "-capture-scope.json").write_text(json.dumps({"sentinel_confirmed_visible": True, "target": target_window, "capture": scoped_capture, "list_only": scoped_list}, indent=2), encoding="utf-8")
            capture = call("screen_capture", {})
            if not any(c.get("type") == "image" and base64.b64decode(c.get("data", "")).startswith(b"\x89PNG") for c in capture["content"]):
                raise AssertionError("Native capture returned no PNG")
            result = {"status": "passed", "validated": True, "session": session.value,
                      "scroll_receipts": scroll_receipts,
                      "checks": ["list_apps", "get_app_state", "set_value", "type_text", "press_key", "click", "independent fixture state", "independent child drag gesture", "fractional and multi-page viewport displacement in both axes", "screen_capture", "targeted capture and list_only metadata exclude unrelated same-process window"],
                      "not_validated": ["mixed DPI", "multiple monitors", "occluded windows", "secure desktop"]}
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
