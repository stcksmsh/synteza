"""Sampler tracks: one lane per pitch, named from the preset the plugin state points at.

Only DecentSampler's state is read: it holds the preset's file URL, and the preset (.dspreset, XML, read-only here)
maps notes to groups and samples. Any other sampler gets pitch numbers.
"""
import base64
import re
import xml.etree.ElementTree as ET
from pathlib import Path
from urllib.parse import unquote, urlparse

SAMPLERS = ('DecentSampler', 'ReaSamplOmatic5000')
_URL = re.compile(rb'_libraryUrl="([^"]*)"')


def _first_instrument(track):
    for fx in track.children('FXCHAIN'):
        for vst in fx.children('VST'):
            if vst.args and re.match(r'(VST|AU|CLAP)\w*i:', vst.args[0]):
                return vst
    return None


def sampler_plugin(track):
    """The track's first instrument plugin when it is a known sampler, else None."""
    vst = _first_instrument(track)
    return vst if vst is not None and any(s in vst.args[0] for s in SAMPLERS) else None


def _preset_path(vst):
    if 'DecentSampler' not in vst.args[0]:
        return None
    b64 = ''.join(t[0] for t in vst.body if isinstance(t, list) and len(t) == 1)
    try:
        state = base64.b64decode(b64 + '=' * (-len(b64) % 4))
    except ValueError:
        return None
    m = _URL.search(state)
    if m is None:
        return None
    url = urlparse(unquote(m.group(1).decode('utf-8', 'replace')))
    return Path(url.path) if url.scheme == 'file' else None


def _int(el, key, default):
    try:
        return int(el.get(key))
    except (TypeError, ValueError):
        return default


def note_names(vst):
    """{pitch: sound name}, or {} when the preset cannot be found or read. A zone over several notes gives
    "<name> <pitch>" so names stay unique."""
    path = _preset_path(vst)
    try:
        root = ET.parse(path).getroot() if path else None
    except (OSError, ET.ParseError):
        return {}
    if root is None:
        return {}
    zones = []
    for group in root.iter('group'):
        for s in group.iter('sample'):
            lo = _int(s, 'loNote', _int(group, 'loNote', _int(s, 'rootNote', _int(group, 'rootNote', 0))))
            hi = _int(s, 'hiNote', _int(group, 'hiNote', lo))
            name = group.get('name') or Path(s.get('path') or '').stem
            if name:
                zones.append((lo, hi, name))
    out = {}
    for lo, hi, name in zones:
        for p in range(lo, hi + 1):
            out.setdefault(p, name if lo == hi else f'{name} {p}')
    return out
