"""Write a rights-clean synthetic test track: 96 s at 124 BPM, 44.1 kHz mono 16-bit WAV.

Layout (seconds, bars of 4 beats at 124 BPM are ~1.935 s):
  0-15.5   intro: pad, closed hats
  15.5-31  build: rising noise, snare roll that speeds up, pad filter opens
  31-58    drop:  four-on-the-floor kick, bass, hats, lead stabs
  58-73.5  break: pad and a soft lead only, no drums
  73.5-81  short build
  81-96    second drop, fades out over the last 2 s

Deterministic (fixed seed), so the same file is produced every run.
Usage: python3 make_synth_clip.py OUT.wav
"""

import sys
import wave

import numpy as np

if len(sys.argv) != 2:
    sys.exit("usage: make_synth_clip.py OUT.wav")

SR = 44100
BPM = 124.0
DURATION = 96.0
BEAT = 60.0 / BPM
BAR = 4 * BEAT


def bar_time(bars):
    return bars * BAR


INTRO_END = bar_time(8)
BUILD_END = bar_time(16)
DROP_END = bar_time(30)
BREAK_END = bar_time(38)
BUILD2_END = bar_time(42)

rng = np.random.default_rng(124)
n = int(SR * DURATION)
t = np.arange(n) / SR
out = np.zeros(n)


def add(start, signal):
    i = int(start * SR)
    j = min(n, i + len(signal))
    if j > i:
        out[i:j] += signal[: j - i]


def env(length, attack, decay):
    k = np.arange(int(length * SR)) / SR
    return np.minimum(k / max(attack, 1e-4), 1.0) * np.exp(-k / decay)


def kick():
    k = np.arange(int(0.35 * SR)) / SR
    freq = 45 + 110 * np.exp(-k / 0.03)
    return 0.9 * np.sin(2 * np.pi * np.cumsum(freq) / SR) * np.exp(-k / 0.12)


def snare(gain):
    e = env(0.18, 0.001, 0.05)
    k = np.arange(len(e)) / SR
    return gain * (rng.standard_normal(len(e)) * 0.6 + np.sin(2 * np.pi * 190 * k) * 0.4) * e


def hat(gain):
    e = env(0.05, 0.0005, 0.012)
    noise = np.diff(rng.standard_normal(len(e) + 1))
    return gain * noise * e


def tone(freq, length, gain, attack=0.005, decay=0.25, harmonics=(1.0, 0.5, 0.25)):
    e = env(length, attack, decay)
    k = np.arange(len(e)) / SR
    wave_ = sum(a * np.sin(2 * np.pi * freq * (h + 1) * k) for h, a in enumerate(harmonics))
    return gain * wave_ * e


def in_range(a, b, step):
    x = a
    while x < b - 1e-9:
        yield x
        x += step


# Pad: A minor chord, sustained over the whole track, louder in intro/break.
pad = sum(np.sin(2 * np.pi * f * t + p) for f, p in ((220.0, 0), (261.63, 1), (329.63, 2)))
pad_level = np.interp(
    t,
    [0, INTRO_END, BUILD_END, DROP_END, DROP_END + 1, BREAK_END, BUILD2_END, DURATION],
    [0.10, 0.10, 0.16, 0.05, 0.14, 0.14, 0.06, 0.05],
)
# The build opens a crude filter: more upper harmonics as it rises.
bright = np.clip((t - INTRO_END) / (BUILD_END - INTRO_END), 0, 1) * (t < BUILD_END)
pad += bright * 0.5 * sum(np.sin(2 * np.pi * f * 4 * t) for f in (220.0, 261.63, 329.63))
out += pad_level * pad

# Intro hats.
for x in in_range(0, INTRO_END, BEAT / 2):
    add(x, hat(0.12))

# Builds: riser noise and a snare roll that speeds up.
for a, b in ((INTRO_END, BUILD_END), (BREAK_END, BUILD2_END)):
    m = (t >= a) & (t < b)
    rise = np.clip((t - a) / (b - a), 0, 1)
    out += m * rise**2 * 0.25 * rng.standard_normal(n)
    x = a
    while x < b - 1e-9:
        frac = (x - a) / (b - a)
        add(x, snare(0.2 + 0.4 * frac))
        x += BEAT if frac < 0.5 else BEAT / 2 if frac < 0.75 else BEAT / 4

# Drops: kick, offbeat bass, hats, lead stabs.
bass_notes = [55.0, 55.0, 43.65, 49.0]
lead_notes = [440.0, 523.25, 659.25, 587.33]
for a, b in ((BUILD_END, DROP_END), (BUILD2_END, DURATION)):
    for i, x in enumerate(in_range(a, b, BEAT)):
        add(x, kick())
        add(x + BEAT / 2, tone(bass_notes[(i // 4) % 4], BEAT / 2, 0.35, decay=0.15))
        add(x + BEAT / 2, hat(0.18))
        if i % 2 == 1:
            add(x, snare(0.3))
        if i % 8 in (0, 3, 6):
            add(x, tone(lead_notes[(i // 8) % 4], 0.3, 0.15, decay=0.12))

# Break: soft lead melody, one note per bar.
for i, x in enumerate(in_range(DROP_END, BREAK_END, BAR)):
    add(x, tone(lead_notes[i % 4], BAR, 0.12, attack=0.2, decay=1.0))

out *= np.clip((DURATION - t) / 2.0, 0, 1)
out /= max(1e-9, np.abs(out).max()) / 0.89

with wave.open(sys.argv[1], "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes((out * 32767).astype("<i2").tobytes())
