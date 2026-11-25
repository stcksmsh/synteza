#!/usr/bin/env python3
"""A check clip for a DAW sidecar: the track's audio under a thin beat strip (orange on downbeats) and one labelled
row per lane showing what the lane plays: a note is lit for its real length, in a stripe of its own while others
sound (a chord is several stripes, at most MAX_STRIPES), brighter for louder, with a white cap for the first
START_FRAMES frames of every note so legato and repeated pitches still show each start.

checkclip.py sidecar.json track.wav out.mp4 [--start S] [--seconds 60] [--drums REGEX] [--lead-ms 83]

The window starts at --start, or by default at the downbeat at or before the first event of a lane whose name
matches --drums. Frame k shows wav time start + k/FPS; a note first lights the frame ceil((t - start) * FPS) and
stays lit while t <= frame time < t + length, for at least one frame. Notes already sounding at the window start are
drawn sustaining, without a cap. Stripes beyond MAX_STRIPES share the last stripe, drawn in OVERFLOW_RGB.
--lead-ms draws everything that much earlier, rounded to whole frames; the audio is untouched.
Frames are drawn with numpy and piped to ffmpeg, which adds the labels and the audio.
"""
import argparse
import bisect
import json
import math
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

W, H, FPS = 960, 540, 60
BEAT_RGB = (0, 90, 255)
DOWNBEAT_RGB = (255, 70, 0)
NOTE_RGB, OVERFLOW_RGB, CAP_RGB, EMPTY_RGB = (0, 200, 255), (255, 200, 0), (255, 255, 255), (30, 30, 30)
FONT = '/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf'
LABEL_W, BOX_W, CAP_W = 330, 500, 40
STRIP_H, ROWS_TOP, ROW_GAP = 24, 40, 12
MAX_STRIPES, START_FRAMES = 6, 4
DEFAULT_LEAD_MS = 83.0


def lead_frames(lead_ms):
    return round(lead_ms * FPS / 1000)


def beat_lit(times, frame_times):
    """For each frame, whether a beat started within 60 ms before it."""
    t = np.asarray(sorted(times))
    i = np.searchsorted(t, frame_times, side='right') - 1
    ok = i >= 0
    return ok & (frame_times - t[np.clip(i, 0, None)] < 0.06) if len(t) else np.zeros(len(frame_times), bool)


def lane_states(events, start, n, lead):
    """(level, cap, over): level[s, k] is 0..255 brightness of stripe s at frame k (0 = dark),
    cap[s, k] the start mark, over[k] whether a note had no free stripe at frame k and was drawn on the last one.
    A note takes the lowest stripe free at its first frame, so overlapping notes never share one."""
    level = np.zeros((MAX_STRIPES, n), np.uint8)
    cap = np.zeros((MAX_STRIPES, n), bool)
    over = np.zeros(n, bool)
    free = [-math.inf] * MAX_STRIPES  # first frame each stripe is free again
    for e in sorted(events, key=lambda e: e['t']):
        f0 = math.ceil((e['t'] - start) * FPS - 1e-9) - lead
        f1 = max(f0 + 1, math.ceil((e['t'] + e.get('length', 0) - start) * FPS - 1e-9) - lead)
        s = next((i for i in range(MAX_STRIPES) if free[i] <= f0), None)
        clamped = s is None
        s = MAX_STRIPES - 1 if clamped else s
        free[s] = max(free[s], f1)
        if f1 <= 0 or f0 >= n:
            continue
        a, b = max(f0, 0), min(f1, n)
        v = 80 + 175 * min(e.get('velocity', 127), 127) // 127
        over[a:b] |= clamped
        level[s, a:b] = np.maximum(level[s, a:b], v)
        if f0 >= 0:
            cap[s, f0:min(f0 + START_FRAMES, n)] = True
    return level, cap, over


def states(side, start, n, lead_ms=DEFAULT_LEAD_MS):
    """Per frame: downbeat flash, beat flash, and a (level, cap, over) triple per lane, all drawn lead_ms early."""
    lead = lead_frames(lead_ms)
    ft = start + (np.arange(n) + lead) / FPS
    downbeat = beat_lit(side['downbeats'], ft)
    beat = beat_lit(side['beats'], ft) & ~downbeat
    return downbeat, beat, [lane_states(lane['events'], start, n, lead) for lane in side['lanes']]


def visible(side, rows):
    """Only lanes with a note sounding in the window, each cut to as many stripes as its most simultaneous notes."""
    out = []
    for lane, (level, cap, over) in zip(side['lanes'], rows, strict=True):
        used = np.flatnonzero(level.any(axis=1))
        if len(used):
            k = used[-1] + 1
            out.append((lane['name'], level[:k], cap[:k], over))
    return out


def default_start(side, drums):
    firsts = [lane['events'][0]['t'] for lane in side['lanes'] if re.search(drums, lane['name'])]
    if not firsts:
        raise ValueError(f'no lane matches {drums!r}')
    downbeats = side['downbeats']
    return downbeats[max(bisect.bisect_right(downbeats, min(firsts)) - 1, 0)]


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('sidecar')
    ap.add_argument('wav')
    ap.add_argument('out')
    ap.add_argument('--start', type=float)
    ap.add_argument('--seconds', type=float, default=60.0)
    ap.add_argument('--drums', default='(?i)kick|clap|hat|perc|snare')
    ap.add_argument('--lead-ms', type=float, default=DEFAULT_LEAD_MS)
    a = ap.parse_args(argv)
    side = json.loads(Path(a.sidecar).read_text(encoding='utf-8'))
    try:
        start = a.start if a.start is not None else default_start(side, a.drums)
    except ValueError as e:
        print(f'error: {e}', file=sys.stderr)
        return 1
    n = round(a.seconds * FPS)
    downbeat, beat, rows = states(side, start, n, a.lead_ms)
    lanes = visible(side, rows)
    if not lanes:
        print('error: no lane has a note in the window', file=sys.stderr)
        return 1
    row_h = (H - ROWS_TOP - 20) // len(lanes)
    bar_h = row_h - ROW_GAP
    if bar_h < MAX_STRIPES * 2:
        print(f'error: {len(lanes)} lanes do not fit', file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as tmp:
        filters = []
        for i, (name, *_) in enumerate(lanes):  # label text goes through files so no name needs escaping
            f = Path(tmp) / f'label{i}.txt'
            f.write_text(name, encoding='utf-8')
            filters.append(f"drawtext=fontfile={FONT}:textfile={f}:fontsize=20:fontcolor=white:"
                           f"box=1:boxcolor=black:boxborderw=4:x=20:y={ROWS_TOP + i * row_h + bar_h // 2 - 12}")
        cmd = ['ffmpeg', '-v', 'error', '-y',
               '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', f'{W}x{H}', '-r', str(FPS), '-i', '-',
               '-ss', f'{start:.6f}', '-t', f'{a.seconds:.6f}', '-i', a.wav,
               '-vf', ','.join(filters), '-map', '0:v', '-map', '1:a',
               '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '23', '-pix_fmt', 'yuv420p',
               '-c:a', 'aac', '-b:a', '192k', '-shortest', a.out]
        proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)
        frame = np.zeros((H, W, 3), np.uint8)
        for k in range(n):
            frame[:] = 0
            frame[:STRIP_H] = DOWNBEAT_RGB if downbeat[k] else BEAT_RGB if beat[k] else 0
            for i, (_, level, cap, over) in enumerate(lanes):
                stripe_h = bar_h // len(level)
                for s in range(len(level)):
                    y = ROWS_TOP + i * row_h + s * stripe_h
                    y1 = y + stripe_h - (2 if stripe_h > 4 else 1)
                    box = frame[y:y1, LABEL_W:LABEL_W + BOX_W]
                    box[:] = EMPTY_RGB
                    if level[s, k]:
                        rgb = OVERFLOW_RGB if s == MAX_STRIPES - 1 and over[k] else NOTE_RGB
                        box[:, CAP_W:] = np.array(rgb) * level[s, k] // 255
                    if cap[s, k]:
                        box[:, :CAP_W] = CAP_RGB
            proc.stdin.write(frame.tobytes())
        proc.stdin.close()
        code = proc.wait()
    if code:
        print(f'error: ffmpeg exited {code}', file=sys.stderr)
        return code
    lead = lead_frames(a.lead_ms)
    print(f'{a.out}: {start:.3f}-{start + a.seconds:.3f} s of the wav, {len(lanes)} lanes, '
          f'lead {lead} frames ({lead * 1000 / FPS:.1f} ms)')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
