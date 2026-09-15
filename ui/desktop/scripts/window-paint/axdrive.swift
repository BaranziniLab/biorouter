// axdrive <pid> size|pos <x0> <y0> <x1> <y1> <steps> <interval-ms>
//
// Walks the app's main window through <steps> intermediate sizes (or positions) with
// the Accessibility API, from ONE process, so a step costs microseconds rather than an
// `osascript` launch (~150 ms, which turns a 16 ms "drag" into a slideshow). It is the
// nearest a script gets to a live edge drag; it is not one — AppKit's live-resize
// tracking loop only runs for a real pointer. Prints each size read back, because a
// resize that silently no-ops is a known impostor (window-scaling-regressions.md).
//
// Needs Accessibility permission for the terminal that runs it. macOS clamps sizes to
// the window's minimum (1048 wide) exactly as it does for a person.
// Build: swiftc -O axdrive.swift -o axdrive
import ApplicationServices
import Foundation

let a = CommandLine.arguments
guard a.count == 9, let pid = Int32(a[1]) else {
  print("usage: axdrive <pid> size|pos x0 y0 x1 y1 steps intervalMs"); exit(2)
}
let mode = a[2]
let (x0, y0, x1, y1) = (Double(a[3])!, Double(a[4])!, Double(a[5])!, Double(a[6])!)
let steps = Int(a[7])!
let interval = Double(a[8])! / 1000
let app = AXUIElementCreateApplication(pid)
var ref: CFTypeRef?
guard AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &ref) == .success,
      let windows = ref as? [AXUIElement], !windows.isEmpty else {
  print("no AX windows for pid \(pid) (Accessibility permission?)"); exit(1)
}
func size(_ w: AXUIElement) -> CGSize {
  var v: CFTypeRef?; var s = CGSize.zero
  if AXUIElementCopyAttributeValue(w, kAXSizeAttribute as CFString, &v) == .success { AXValueGetValue(v as! AXValue, .cgSize, &s) }
  return s
}
func position(_ w: AXUIElement) -> CGPoint {
  var v: CFTypeRef?; var p = CGPoint.zero
  if AXUIElementCopyAttributeValue(w, kAXPositionAttribute as CFString, &v) == .success { AXValueGetValue(v as! AXValue, .cgPoint, &p) }
  return p
}
let win = windows.max { size($0).width * size($0).height < size($1).width * size($1).height }!
let t0 = Date()
for i in 0...steps {
  let f = Double(i) / Double(max(steps, 1))
  let x = (x0 + (x1 - x0) * f).rounded(), y = (y0 + (y1 - y0) * f).rounded()
  let start = Date()
  if mode == "size" {
    var s = CGSize(width: x, height: y)
    AXUIElementSetAttributeValue(win, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &s)!)
    let r = size(win)
    print("\(Int(Date().timeIntervalSince(t0) * 1000)) size \(Int(x))x\(Int(y)) -> \(Int(r.width))x\(Int(r.height))")
  } else {
    var p = CGPoint(x: x, y: y)
    AXUIElementSetAttributeValue(win, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &p)!)
    let r = position(win)
    print("\(Int(Date().timeIntervalSince(t0) * 1000)) pos \(Int(x)),\(Int(y)) -> \(Int(r.x)),\(Int(r.y))")
  }
  let left = interval - Date().timeIntervalSince(start)
  if left > 0 { Thread.sleep(forTimeInterval: left) }
}
