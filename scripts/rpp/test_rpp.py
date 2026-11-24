"""Synthetic checks for the .RPP reader, the sidecar builder and the offset measurement.

Run: /secondary/hyst-env/analyze-py/bin/python -m pytest scripts/rpp -q
(pytest puts scripts/ on the path because scripts/rpp is a package.)
"""
import importlib.util
import json
import wave
from pathlib import Path

import numpy as np
import pytest

from rpp import offset as off
from rpp.project import parse, read

CLI = Path(__file__).resolve().parents[1] / 'rpp.py'

# 120 bpm 4/4 until 4 s, then 90 bpm 3/4 (REAPER packs the signature as num + den * 65536).
# One marker, one region, a looped 1-QN MIDI item that runs across the tempo change, and a muted track.
FIXTURE = """<REAPER_PROJECT 0.1 "7.0" 0
  TEMPO 120 4 4 0
  PLAYRATE 1 0 0.25 4
  <TEMPOENVEX
    PT 0 120 1
    PT 4 90 1 262147
  >
  MARKER 1 1 intro 0
  MARKER 2 2 verse 1
  MARKER 2 5 "" 1
  <TRACK
    NAME Lead
    MUTESOLO 0 0 0
    ISBUS 0 0
    <ITEM
      POSITION 3
      LOOP 1
      LENGTH 2
      MUTE 0 0
      <SOURCE MIDI
        HASDATA 1 960 QN
        E 0 90 3c 60
        E 240 80 3c 00
        E 240 90 40 50
        E 240 80 40 00
        E 240 b0 7b 00
      >
    >
  >
  <TRACK
    NAME Muted
    MUTESOLO 1 0 0
    ISBUS 0 0
    <ITEM
      POSITION 0
      LOOP 0
      LENGTH 1
      <SOURCE MIDI
        HASDATA 1 960 QN
        E 0 90 24 60
        E 480 80 24 00
        E 480 b0 7b 00
      >
    >
  >
>
"""


@pytest.fixture
def project(tmp_path):
    p = tmp_path / 'fixture.RPP'
    p.write_text(FIXTURE)
    return p


def _cli():
    spec = importlib.util.spec_from_file_location('rpp_cli', CLI)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_grid_follows_tempo_points_and_time_signature(project):
    tmap, _, _ = read(project)
    beats, downbeats = tmap.grid(7.0)
    third = 60 / 90
    expected = [k * 0.5 for k in range(8)] + [4 + k * third for k in range(5)]
    assert beats == pytest.approx(expected)
    assert downbeats == pytest.approx([0, 2, 4, 4 + 3 * third])


def test_markers_and_regions(project):
    _, markers, _ = read(project)
    assert markers == [('intro', 1.0, 1.0), ('verse', 2.0, 5.0)]


def test_looped_midi_is_unrolled_across_the_tempo_change_and_muted_tracks_are_left_out(project):
    _, _, lanes = read(project)
    assert [lane.name for lane in lanes] == ['Lead']
    third = 60 / 90
    # Item 3..5 s is QN 6..9.5; the source is one QN: pitch 60 on the beat, pitch 64 on the off-beat.
    expected = [(3.0, 60, 96, 0.125), (3.25, 64, 80, 0.125), (3.5, 60, 96, 0.125), (3.75, 64, 80, 0.125),
                (4.0, 60, 96, third / 4), (4 + third / 2, 64, 80, third / 4), (4 + third, 60, 96, third / 4)]
    got = lanes[0].events
    assert [(p, v) for _, p, v, _ in got] == [(p, v) for _, p, v, _ in expected]
    assert [t for t, *_ in got] == pytest.approx([t for t, *_ in expected])
    assert [ln for *_, ln in got] == pytest.approx([ln for *_, ln in expected])


def test_parse_rejects_unbalanced_blocks():
    with pytest.raises(ValueError, match='open block'):
        parse('<REAPER_PROJECT\n<TRACK\n>\n')


def test_build_changes_no_base_key_but_beats_and_tempo(project):
    tmap, markers, lanes = read(project)
    base = {'schema': 3, 'duration': 6, 'tempo': 117.4, 'beats': [0.1, 0.6], 'sections': [{'start': 0}],
            'events': [], 'onsets': [0.3], 'energyEnvelope': [0.5, 0.25], 'envelopeRate': 50}
    out = _cli().build(base, tmap, markers, lanes, 0.5, 6.0)
    for key, value in base.items():
        if key not in ('beats', 'tempo'):
            assert out[key] == value, key
    assert out['tempo'] == 120
    assert out['source'] == 'daw'
    assert out['beats'][:3] == [0.0, 0.5, 1.0]  # project 0.5 s is wav 0
    assert out['downbeats'][:2] == [1.5, 3.5]
    assert out['markers'] == [{'name': 'intro', 'start': 0.5, 'end': 0.5},
                              {'name': 'verse', 'start': 1.5, 'end': 4.5}]
    assert out['lanes'][0]['events'][0] == {'t': 2.5, 'pitch': 60, 'velocity': 96, 'length': 0.125}


SR = 48000


def _clicks(times, offset, duration, doubled=False, preroll=0.0):
    """A mono track with a short noise burst at each project time, shifted onto the wav by offset."""
    rng = np.random.default_rng(0)
    n = round(0.005 * SR)
    burst = rng.standard_normal(n) * np.exp(-np.arange(n) / 40.0) * 0.5
    x = np.zeros(round(duration * SR))
    starts = [t - offset for t in times] + ([t - offset + 0.032 for t in times] if doubled else [])
    for t in starts:
        i = round(t * SR)
        x[i:i + n] += burst
    if preroll:  # a quiet sound just before the first click, so the start-of-audio anchor is off
        first = round((min(times) - offset - preroll) * SR)
        x[first:first + round(preroll * SR)] += 0.001 * rng.standard_normal(round(preroll * SR))
    return x


def _arrangement():
    """Two lanes in a loop that repeats every beat at 120 bpm: plain beat correlation cannot place it."""
    return [[1.0 + 0.5 * k for k in range(40)], [1.25 + 0.5 * k for k in range(40)]]


def test_offset_of_shifted_click_track_is_within_5_ms():
    lanes = _arrangement()
    mono = _clicks(lanes[0] + lanes[1], 0.2371, 25.0, preroll=0.03)
    offset, residual, anchor, ratio, matched, total, _ = off.measure(lanes, mono, SR)
    assert abs(anchor - 0.2671) < 0.002  # the preroll moves the anchor 30 ms; the onset peak must correct it
    assert abs(offset - 0.2371) < 0.005
    assert residual < 2
    assert ratio >= off.MIN_RATIO
    assert matched == total == 80


def test_two_equally_good_peaks_are_ambiguous():
    lanes = _arrangement()
    mono = _clicks(lanes[0] + lanes[1], 0.2, 25.0, doubled=True)
    with pytest.raises(off.OffsetError, match='ambiguous'):
        off.measure(lanes, mono, SR)


def test_cli_exits_non_zero_with_a_message_on_an_ambiguous_offset(project, tmp_path, capsys):
    _, _, lanes = read(project)
    mono = _clicks([e[0] for e in lanes[0].events], 0.0, 7.0, doubled=True)
    wav = tmp_path / 'fixture.wav'
    with wave.open(str(wav), 'wb') as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes((np.clip(mono, -1, 1) * 32767).astype('<i2').tobytes())
    base = tmp_path / 'base.json'
    base.write_text(json.dumps({'schema': 3, 'duration': 7, 'tempo': 120, 'beats': []}))
    out = tmp_path / 'out.json'
    assert _cli().main([str(project), str(base), str(out), '--wav', str(wav)]) == 2
    assert 'ambiguous offset' in capsys.readouterr().err
    assert not out.exists()
