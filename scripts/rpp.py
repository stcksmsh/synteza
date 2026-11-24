#!/usr/bin/env python3
"""Exact grid and note lanes from a Reaper project, added to an existing sidecar.

rpp.py project.RPP base.sidecar.json out.json [--wav W]

The wav defaults to the .wav beside the project with the same name. The offset between project and wav time
is measured from the wav (see rpp/offset.py) and printed; an ambiguous offset or a residual over 20 ms exits 2.
"""
import argparse
import json
import sys
from pathlib import Path

import numpy as np
from analyze.jsjson import dumps
from analyze.wav import decode_wav_file
from rpp import offset as off
from rpp.project import read


def build(base, tmap, markers, lanes, offset, duration):
    """The base sidecar with an exact grid, lanes and markers, all in wav seconds."""

    def wav(t):
        return round(t - offset, 6)

    def keep(t):
        return 0 <= t - offset < duration

    beats, downbeats = tmap.grid(duration + offset)
    out = dict(base)
    out['tempo'] = tmap.bpm[0]
    out['beats'] = [wav(t) for t in beats if keep(t)]
    out['source'] = 'daw'
    out['downbeats'] = [wav(t) for t in downbeats if keep(t)]
    out['lanes'] = []
    for lane in lanes:
        events = [{'t': wav(t), 'pitch': p, 'velocity': v, 'length': round(ln, 6)}
                  for t, p, v, ln in lane.events if keep(t)]
        if events:
            out['lanes'].append({'name': lane.name, 'kind': lane.kind, 'events': events})
    out['markers'] = [{'name': n, 'start': wav(s), 'end': wav(e)} for n, s, e in markers if keep(s)]
    return out


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('project')
    ap.add_argument('base')
    ap.add_argument('out')
    ap.add_argument('--wav')
    a = ap.parse_args(argv)
    project = Path(a.project)
    wav = Path(a.wav) if a.wav else project.with_suffix('.wav')
    for p in (project, Path(a.base), wav):
        if not p.is_file():
            print(f'error: {p} not found', file=sys.stderr)
            return 1
    try:
        tmap, markers, lanes = read(project)
    except ValueError as e:
        print(f'error: {project}: {e}', file=sys.stderr)
        return 1
    w = decode_wav_file(str(wav))
    mono = np.mean(w.channels, axis=0)
    try:
        offset, residual, anchor, ratio, matched, total, per_lane = off.measure(
            [[e[0] for e in lane.events] for lane in lanes], mono, w.sample_rate)
    except off.OffsetError as e:
        print(f'error: {wav.name}: {e}', file=sys.stderr)
        return 2
    print(f'{wav.name}: offset {offset * 1000:.1f} ms, residual {residual:.1f} ms (anchor {anchor * 1000:.1f} ms, '
          f'peak ratio {ratio:.2f}, {matched}/{total} events matched an onset)')
    # Whole milliseconds: the measurement resolves about one analysis hop (1.3 ms at 48 kHz), and a round
    # offset keeps the grid's spacing exact.
    offset = round(offset, 3) + 0.0
    print(f'  applied offset {offset * 1000:.0f} ms')
    if residual > 20:
        print(f'error: residual {residual:.1f} ms is over 20 ms', file=sys.stderr)
        return 2
    base = json.loads(Path(a.base).read_text(encoding='utf-8'))
    out = build(base, tmap, markers, lanes, offset, w.duration)
    Path(a.out).write_text(dumps(out), encoding='utf-8')
    lane_offset = {lane.name: ms for lane, ms in zip(lanes, per_lane, strict=True)}
    for lane in out['lanes']:
        ms = lane_offset[lane['name']]
        own = 'no onsets matched' if ms is None else f'own offset {ms:.1f} ms'
        print(f"  {lane['kind']:5} {len(lane['events']):5}  {lane['name']}  ({own})")
    print(f"  {len(out['beats'])} beats, {len(out['downbeats'])} downbeats, {len(out['markers'])} markers")
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
