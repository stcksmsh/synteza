"""Read a Reaper .RPP project: tempo map, grid, markers/regions and per-track note lanes.

Times returned here are project seconds. Shifting them onto the rendered wav is the caller's job.
"""
import bisect
import re
from dataclasses import dataclass, field

_TOKEN = re.compile(r'"([^"]*)"|\'([^\']*)\'|`([^`]*)`|(\S+)')


def tokenize(line):
    return [next(g for g in m.groups() if g is not None) for m in _TOKEN.finditer(line)]


@dataclass
class Node:
    name: str
    args: list
    body: list = field(default_factory=list)  # in file order: token lists and child Nodes

    def lines(self, key):
        return [e for e in self.body if isinstance(e, list) and e and e[0] == key]

    def first(self, key):
        found = self.lines(key)
        return found[0] if found else None

    def children(self, name):
        return [e for e in self.body if isinstance(e, Node) and e.name == name]


def parse(text):
    top = Node('', [])
    stack = [top]
    for raw in text.splitlines():
        s = raw.strip()
        if s.startswith('<'):
            toks = tokenize(s[1:])
            node = Node(toks[0] if toks else '', toks[1:])
            stack[-1].body.append(node)
            stack.append(node)
        elif s == '>':
            if len(stack) == 1:
                raise ValueError('unbalanced ">" in project')
            stack.pop()
        elif s:
            stack[-1].body.append(tokenize(s))
    root = top.body[0] if top.body and isinstance(top.body[0], Node) else None
    if root is None or root.name != 'REAPER_PROJECT':
        raise ValueError('not a REAPER project')
    if len(stack) > 1:
        raise ValueError('project ends inside an open block')
    return root


def _num(tok, what):
    try:
        return float(tok)
    except (ValueError, TypeError):
        raise ValueError(f'cannot read {what}: {tok!r}') from None


class TempoMap:
    """Square tempo segments plus time-signature changes. Beats are 1/den notes; bars restart at each change."""

    def __init__(self, bpm, num, den, points=()):
        # Each point is time in seconds, bpm, an optional (num, den) and the envelope shape.
        self.t0 = [0.0]
        self.q0 = [0.0]
        self.bpm = [float(bpm)]
        sigs = [(0.0, int(num), int(den))]
        pts = sorted(points, key=lambda p: p[0])
        for i, (t, b, sig, shape) in enumerate(pts):
            nxt = pts[i + 1][1] if i + 1 < len(pts) else b
            if shape != 1 and nxt != b:
                raise ValueError(f'tempo ramp at {t} s (shape {shape}) is not supported')
            q = self.qn_at(t)
            if t <= 0:
                self.bpm[0] = float(b)
            else:
                self.t0.append(float(t))
                self.q0.append(q)
                self.bpm.append(float(b))
            if sig:
                sigs = [s for s in sigs if s[0] < q] + [(q, sig[0], sig[1])]
        self.sigs = sigs

    def bpm_at(self, t):
        return self.bpm[max(bisect.bisect_right(self.t0, t) - 1, 0)]

    def qn_at(self, t):
        i = max(bisect.bisect_right(self.t0, t) - 1, 0)
        return self.q0[i] + (t - self.t0[i]) * self.bpm[i] / 60.0

    def time_at(self, q):
        i = max(bisect.bisect_right(self.q0, q) - 1, 0)
        return self.t0[i] + (q - self.q0[i]) * 60.0 / self.bpm[i]

    def grid(self, t_end):
        """(beat times, downbeat times) from project 0 up to t_end."""
        q_end = self.qn_at(t_end)
        beats, downbeats = [], []
        for j, (qs, num, den) in enumerate(self.sigs):
            stop = min(self.sigs[j + 1][0] if j + 1 < len(self.sigs) else q_end, q_end)
            step = 4.0 / den
            k = 0
            while qs + k * step < stop - 1e-9:
                t = self.time_at(qs + k * step)
                beats.append(t)
                if k % num == 0:
                    downbeats.append(t)
                k += 1
        return beats, downbeats


def tempo_map(root):
    tempo = root.first('TEMPO')
    if tempo is None:
        raise ValueError('project has no TEMPO line')
    bpm, num, den = _num(tempo[1], 'tempo'), int(tempo[2]), int(tempo[3])
    rate = root.first('PLAYRATE')
    if rate is not None and _num(rate[1], 'PLAYRATE') != 1:
        raise ValueError(f'project play rate {rate[1]} is not supported')
    for env in root.children('MASTERPLAYSPEEDENV'):
        if env.lines('PT'):
            raise ValueError('play-speed envelope points are not supported')
    points = []
    for env in root.children('TEMPOENVEX'):
        for pt in env.lines('PT'):
            sig = None
            if len(pt) > 4 and int(pt[4]) > 0:
                v = int(pt[4])  # REAPER packs it as num + den * 65536
                sig = (v & 0xFFFF, v >> 16)
            points.append((_num(pt[1], 'PT time'), _num(pt[2], 'PT bpm'), sig, int(pt[3]) if len(pt) > 3 else 1))
    return TempoMap(bpm, num, den, points)


def markers(root):
    """[(name, start, end)] in project seconds; a point marker has end == start."""
    out, open_regions = [], {}
    for m in root.lines('MARKER'):
        idx, t, name = m[1], _num(m[2], 'marker time'), m[3] if len(m) > 3 else ''
        flags = int(m[4]) if len(m) > 4 else 0
        if flags & 1:
            if idx in open_regions:
                name0, start = open_regions.pop(idx)
                out.append((name0, start, t))
            else:
                open_regions[idx] = (name, t)
        else:
            out.append((name, t, t))
    if open_regions:
        raise ValueError(f'regions without an end: {sorted(open_regions)}')
    return sorted(out, key=lambda m: (m[1], m[2], m[0]))


def _midi_notes(src):
    """Notes in source ticks (start, end, pitch, velocity), ticks per QN, and the source length in ticks."""
    has = src.first('HASDATA')
    if has is None or len(has) < 4 or has[3] != 'QN':
        raise ValueError(f'MIDI source with unsupported HASDATA {has}')
    ppq = int(has[2])
    pos = 0
    open_notes, notes = {}, []
    for e in src.body:
        if isinstance(e, Node):
            pos += int(e.args[0]) if e.name in ('X', 'x') else 0
            continue
        if e and e[0] in ('X', 'x'):
            pos += int(e[1])
        if not e or e[0] not in ('E', 'e'):
            continue
        pos += int(e[1])
        status, d1, d2 = int(e[2], 16), int(e[3], 16), int(e[4], 16)
        kind, ch = status & 0xF0, status & 0x0F
        if kind == 0x90 and d2 > 0:
            open_notes.setdefault((ch, d1), []).append((pos, d2))
        elif kind == 0x80 or (kind == 0x90 and d2 == 0):
            stack = open_notes.get((ch, d1))
            if stack:
                start, vel = stack.pop(0)
                notes.append((start, pos, d1, vel))
    for (_ch, pitch), stack in open_notes.items():
        notes.extend((start, pos, pitch, vel) for start, vel in stack)
    notes.sort()
    return notes, ppq, pos


@dataclass
class Lane:
    name: str
    kind: str  # "midi" | "items"
    events: list  # (t, pitch, velocity, length) in project seconds


def _unroll_midi(item, src, tmap):
    pos, length = _num(item.first('POSITION')[1], 'POSITION'), _num(item.first('LENGTH')[1], 'LENGTH')
    end = pos + length
    loop = item.first('LOOP')
    looped = loop is not None and loop[1] == '1'
    soffs = _num(item.first('SOFFS')[1], 'SOFFS') if item.first('SOFFS') else 0.0
    rate_line = item.first('PLAYRATE')
    rate = _num(rate_line[1], 'PLAYRATE') if rate_line else 1.0
    notes, ppq, src_len = _midi_notes(src)
    ign = src.first('IGNTEMPO')
    fixed = ign is not None and ign[1] == '1'
    q_pos = tmap.qn_at(pos)
    # SOFFS is source seconds; at the tempo where the item starts that is a QN distance.
    soffs_q = soffs * tmap.bpm_at(pos) / 60.0

    def to_project(tick):
        if fixed:  # ticks run at the source's own tempo, not the project's tempo map
            return pos + (tick * 60.0 / (_num(ign[2], 'IGNTEMPO bpm') * ppq) - soffs) / rate
        return tmap.time_at(q_pos + (tick / ppq - soffs_q) / rate)

    out = []
    if src_len <= 0:
        return out
    k = 0
    while to_project(k * src_len) < end - 1e-9:
        base = k * src_len
        for s, e, pitch, vel in notes:
            ts = to_project(base + s)
            if pos - 1e-9 <= ts < end - 1e-9:
                out.append((ts, pitch, vel, min(to_project(base + e), end) - ts))
        if not looped:
            break
        k += 1
    return out


def _track_events(track, name, tmap):
    midi, items = [], []
    for item in track.children('ITEM'):
        if item.lines('TAKE'):
            raise ValueError(f'track {name!r}: items with several takes are not supported')
        mute = item.first('MUTE')
        if mute is not None and mute[1] == '1':
            continue
        for src in item.children('SOURCE'):
            kind = src.args[0] if src.args else ''
            if kind == 'MIDI':
                midi.extend(_unroll_midi(item, src, tmap))
            elif kind != 'EMPTY':
                pos = _num(item.first('POSITION')[1], 'POSITION')
                items.append((pos, None, None, _num(item.first('LENGTH')[1], 'LENGTH')))
    if midi and items:
        raise ValueError(f'track {name!r} has both MIDI and audio items')
    return midi, items


def lanes(root, tmap):
    out = []
    folder_mutes = []  # effective mute of each open folder
    seen = {}
    for track in root.children('TRACK'):
        name_line = track.first('NAME')
        name = name_line[1] if name_line and len(name_line) > 1 else ''
        ms = track.first('MUTESOLO')
        muted = (ms is not None and ms[1] == '1') or any(folder_mutes)
        midi, items = ([], []) if muted else _track_events(track, name, tmap)
        if midi or items:
            seen[name] = seen.get(name, 0) + 1
            if seen[name] > 1:  # several tracks share a name; keep lanes apart in project order
                name = f'{name} #{seen[name]}'
            out.append(Lane(name, 'midi' if midi else 'items', sorted(midi or items, key=lambda e: (e[0], e[1] or 0))))
        bus = track.first('ISBUS')
        delta = int(bus[2]) if bus is not None and len(bus) > 2 else 0
        if delta > 0:
            folder_mutes.append(muted)
        elif delta < 0:
            del folder_mutes[len(folder_mutes) + delta:]
    return out


def renders_whole_project(root):
    """True when the render bounds are the entire project, so the wav starts at project 0 (or later, when the
    render trimmed leading silence)."""
    rr = root.first('RENDER_RANGE')
    return rr is not None and len(rr) > 1 and rr[1] == '1'


def read(path):
    """(tempo map, markers, lanes, whether the render covers the entire project)."""
    with open(path, encoding='utf-8', errors='replace') as f:
        root = parse(f.read())
    tmap = tempo_map(root)
    return tmap, markers(root), lanes(root, tmap), renders_whole_project(root)
