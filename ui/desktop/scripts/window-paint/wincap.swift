// wincap <electron-pid> <out-dir> <duration-ms> <canvas R,G,B>
//
// Measures the part of ONE Biorouter window that carries no app pixels, as fast as
// the window server hands out frames (~50/s). It reads only that window's own
// surface — never the screen — so nothing else on the operator's display is recorded.
//
// Every frame is judged in-process for an UNPAINTED band:
//   rightBand   columns counted in from the right edge where >= 95% of rows sampled
//               in [60, h-30] sit within 8 of that column's modal colour, and that
//               colour is more than 12 away from the canvas in some channel
//   bottomBand  the same for rows counted up from the bottom edge, sampled across the
//               content area (x in [0.3w, w-20]) so the sidebar is not a band
// A pixel with alpha < 250 is outside the window's surface (a frame that raced a
// shrink, or a rounded corner) and can never form a band.
//
// The canvas is the colour the CAPTURE reports for --background-app: the display
// profile is applied, so #131312 reads as 20,20,19 and #ffffff as 255,255,255.
// Read it off a settled capture if the display differs.
//
// Writes frames.tsv (ms, w, h, rightBand, bottomBand, colour) and PNGs of the first
// frame of each banded run, the widest banded frame, and every 60th frame.
// Build: swiftc -O wincap.swift -o wincap  (macOS 14+; see measure.sh)
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

// CGWindowListCreateImage is compile-time unavailable in the macOS 15+ SDK but still
// exported, and it is the only synchronous single-window capture; bind it by name.
typealias CreateImageFn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
guard let sym = dlsym(dlopen("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics", RTLD_NOW), "CGWindowListCreateImage") else {
  print("CGWindowListCreateImage is not exported on this macOS"); exit(3)
}
let createImage = unsafeBitCast(sym, to: CreateImageFn.self)

let a = CommandLine.arguments
guard a.count == 5, let pid = Int32(a[1]), let dur = Double(a[3]) else {
  print("usage: wincap <electron-pid> <out-dir> <duration-ms> <canvas R,G,B>"); exit(2)
}
let out = a[2]
let canvas = a[4].split(separator: ",").map { Int($0)! }
try? FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)

// The app's window: the largest normal-layer window the process owns.
let infos = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
let mine = infos.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
guard let win = mine.max(by: {
  let s = { (w: [String: Any]) -> Double in let b = w[kCGWindowBounds as String] as! [String: Double]; return b["Width"]! * b["Height"]! }
  return s($0) < s($1)
}), let wid = win[kCGWindowNumber as String] as? UInt32 else { print("no window for pid \(pid)"); exit(1) }

func save(_ img: CGImage, _ name: String) {
  let url = URL(fileURLWithPath: "\(out)/\(name).png")
  guard let dst = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else { return }
  CGImageDestinationAddImage(dst, img, nil)
  CGImageDestinationFinalize(dst)
}

typealias RGB = (Int, Int, Int)
let cv: RGB = (canvas[0], canvas[1], canvas[2])
func near(_ p: RGB, _ q: RGB, _ t: Int) -> Bool { abs(p.0 - q.0) <= t && abs(p.1 - q.1) <= t && abs(p.2 - q.2) <= t }

func measure(_ img: CGImage) -> (right: Int, bottom: Int, colour: String) {
  let w = img.width, h = img.height
  guard let cs = img.colorSpace,
        let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: cs,
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return (0, 0, "-") }
  ctx.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
  guard let data = ctx.data?.assumingMemoryBound(to: UInt8.self) else { return (0, 0, "-") }
  func px(_ x: Int, _ y: Int) -> RGB? {
    let o = (y * w + x) * 4
    return data[o + 3] < 250 ? nil : (Int(data[o]), Int(data[o + 1]), Int(data[o + 2]))
  }
  func foreign(_ s: [RGB?]) -> RGB? {
    var counts: [Int: Int] = [:]
    for p in s { counts[p.map { ($0.0 << 16) | ($0.1 << 8) | $0.2 } ?? -1, default: 0] += 1 }
    guard let k = counts.max(by: { $0.value < $1.value })?.key, k >= 0 else { return nil }
    let mode: RGB = (k >> 16, (k >> 8) & 255, k & 255)
    if near(mode, cv, 12) { return nil }
    let n = s.filter { $0.map { near($0, mode, 8) } ?? false }.count
    return Double(n) >= 0.95 * Double(s.count) ? mode : nil
  }
  var right = 0, bottom = 0
  var rc = "-", bc = "-"
  let ys = Array(stride(from: 60, to: h - 30, by: 5))
  var x = w - 8
  while x > 0, ys.count > 4, let m = foreign(ys.map { px(x, $0) }) { right = w - x + 8; rc = "\(m.0),\(m.1),\(m.2)"; x -= 2 }
  let xs = Array(stride(from: Int(0.3 * Double(w)), to: w - 20, by: 5))
  var y = h - 8
  while y > 30, xs.count > 4, let m = foreign(xs.map { px($0, y) }) { bottom = h - y + 8; bc = "\(m.0),\(m.1),\(m.2)"; y -= 2 }
  return (right, bottom, bottom > right ? bc : rc)
}

var tsv = "ms\tw\th\trightBand\tbottomBand\tcolour\n"
let t0 = Date()
var k = 0, banded = 0, widest = 0
var prevBanded = false
var widestFrame: (CGImage, String)? = nil
while Date().timeIntervalSince(t0) * 1000 < dur {
  let ms = Int(Date().timeIntervalSince(t0) * 1000)
  guard let img = createImage(.null, CGWindowListOption.optionIncludingWindow.rawValue, wid,
                              CGWindowImageOption([.boundsIgnoreFraming, .bestResolution]).rawValue)?.takeRetainedValue() else { continue }
  k += 1
  let b = measure(img)
  tsv += "\(ms)\t\(img.width)\t\(img.height)\t\(b.right)\t\(b.bottom)\t\(b.colour)\n"
  let name = String(format: "%04d-%dms-%dx%d-r%d-b%d", k, ms, img.width, img.height, b.right, b.bottom)
  let isBanded = b.right > 0 || b.bottom > 0
  if isBanded { banded += 1 }
  if isBanded && !prevBanded { save(img, name + "-bandstart") }
  if b.right + b.bottom > widest { widest = b.right + b.bottom; widestFrame = (img, name + "-widest") }
  if k % 60 == 1 { save(img, name) }
  prevBanded = isBanded
}
if let (img, name) = widestFrame { save(img, name) }
try? tsv.write(toFile: "\(out)/frames.tsv", atomically: true, encoding: .utf8)
print("frames=\(k) banded=\(banded) widestBand=\(widest) fps=\(Int(Double(k) / (dur / 1000)))")
