#!/usr/bin/env python3
"""A check clip for a DAW sidecar: the track's audio under a full-frame flash on every beat (another colour on
downbeats) and one labelled row per lane whose tick lights at each of the lane's events.

checkclip.py sidecar.json track.wav out.mp4 [--start S] [--seconds 60] [--drums REGEX]

The window starts at --start, or by default at the downbeat at or before the first event of a lane whose name
matches --drums. Frames are drawn with numpy and piped to ffmpeg, which adds the labels and the audio.
"""
import argparse
import bisect
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

W, H, FPS = 960, 540, 60
FLASH_S = 0.06
BEAT_RGB = (0, 90, 255)
DOWNBEAT_RGB = (255, 70, 0)
TICK_ON, TICK_OFF = (255, 255, 255), (45, 45, 45)
FONT = '/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf'
LABEL_W, ROW_H, TICK_W = 330, 36, 120


def lit(times, frame_times):
    """For each frame, whether an event started within FLASH_S before it."""
    t = np.asarray(sorted(times))
    i = np.searchsorted(t, frame_times, side='right') - 1
    ok = i >= 0
    return ok & (frame_times - t[np.clip(i, 0, None)] < FLASH_S) if len(t) else np.zeros(len(frame_times), bool)


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
    a = ap.parse_args(argv)
    side = json.loads(Path(a.sidecar).read_text(encoding='utf-8'))
    try:
        start = a.start if a.start is not None else default_start(side, a.drums)
    except ValueError as e:
        print(f'error: {e}', file=sys.stderr)
        return 1
    n = round(a.seconds * FPS)
    ft = start + np.arange(n) / FPS
    downbeat = lit(side['downbeats'], ft)
    beat = lit(side['beats'], ft) & ~downbeat
    lanes = side['lanes']
    rows = [lit([e['t'] for e in lane['events']], ft) for lane in lanes]
    top = H - 20 - ROW_H * len(lanes)
    if top < 20:
        print(f'error: {len(lanes)} lanes do not fit', file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as tmp:
        filters = []
        for i, lane in enumerate(lanes):  # label text goes through files so no name needs escaping
            f = Path(tmp) / f'label{i}.txt'
            f.write_text(lane['name'], encoding='utf-8')
            filters.append(f"drawtext=fontfile={FONT}:textfile={f}:fontsize=20:fontcolor=white:"
                           f"box=1:boxcolor=black:boxborderw=4:x=20:y={top + i * ROW_H + 6}")
        cmd = ['ffmpeg', '-v', 'error', '-y',
               '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', f'{W}x{H}', '-r', str(FPS), '-i', '-',
               '-ss', f'{start:.6f}', '-t', f'{a.seconds:.6f}', '-i', a.wav,
               '-vf', ','.join(filters), '-map', '0:v', '-map', '1:a',
               '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '23', '-pix_fmt', 'yuv420p',
               '-c:a', 'aac', '-b:a', '192k', '-shortest', a.out]
        proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)
        frame = np.zeros((H, W, 3), np.uint8)
        for k in range(n):
            frame[:] = DOWNBEAT_RGB if downbeat[k] else BEAT_RGB if beat[k] else (0, 0, 0)
            for i, row in enumerate(rows):
                y = top + i * ROW_H + 4
                frame[y:y + ROW_H - 8, LABEL_W:LABEL_W + TICK_W] = TICK_ON if row[k] else TICK_OFF
            proc.stdin.write(frame.tobytes())
        proc.stdin.close()
        code = proc.wait()
    if code:
        print(f'error: ffmpeg exited {code}', file=sys.stderr)
        return code
    print(f'{a.out}: {start:.3f}-{start + a.seconds:.3f} s of the wav, {len(lanes)} lanes')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
