#!/usr/bin/env python3
# Stitch the captured ./frames PNGs into biorouter-reel.mp4 with crossfades.
# Run `node capture-reel.cjs` first to produce ./frames, then `python3 encode.py`.
import subprocess, os

HERE = os.path.dirname(os.path.abspath(__file__))
FRAMES = os.path.join(HERE, "frames")
OUT = os.path.join(HERE, "biorouter-reel.mp4")

# Per-slide on-screen durations (seconds). 11 slides.
L = [3.5, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.2, 4.0, 4.8]
T = 0.6          # crossfade duration
FPS = 30
n = len(L)

files = [f"{FRAMES}/slide-{i:02d}.png" for i in range(n)]
for f in files:
    assert os.path.exists(f), f

inputs = []
for i, f in enumerate(files):
    inputs += ["-loop", "1", "-t", f"{L[i]:.3f}", "-i", f]

# Pre-normalize each input.
fc = []
for i in range(n):
    fc.append(f"[{i}:v]scale=1920:1080,format=yuv420p,fps={FPS},setsar=1[v{i}]")

# xfade chain.
prev = "v0"
cum = L[0]
for k in range(1, n):
    off = cum - T
    out = f"x{k}"
    fc.append(f"[{prev}][v{k}]xfade=transition=fade:duration={T}:offset={off:.3f}[{out}]")
    prev = out
    cum = cum + L[k] - T

filter_complex = ";".join(fc)
final_dur = cum
print(f"final duration ~= {final_dur:.2f}s")

cmd = [
    "ffmpeg", "-y", *inputs,
    "-filter_complex", filter_complex,
    "-map", f"[{prev}]",
    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20", "-preset", "slow",
    "-movflags", "+faststart", "-r", str(FPS),
    OUT,
]
r = subprocess.run(cmd, capture_output=True, text=True)
if r.returncode != 0:
    print("FFMPEG FAILED\n", r.stderr[-3000:])
    raise SystemExit(1)
print("wrote", OUT)

# Poster frame (slide 1, the first feature) for a <video> poster attribute.
POSTER = os.path.join(HERE, "biorouter-reel-poster.jpg")
subprocess.run(["ffmpeg", "-y", "-i", files[1], "-q:v", "3", POSTER], capture_output=True)
print("wrote", POSTER)
