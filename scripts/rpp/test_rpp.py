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

from rpp import checkclip
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
    tmap, _, _, _ = read(project)
    beats, downbeats = tmap.grid(7.0)
    third = 60 / 90
    expected = [k * 0.5 for k in range(8)] + [4 + k * third for k in range(5)]
    assert beats == pytest.approx(expected)
    assert downbeats == pytest.approx([0, 2, 4, 4 + 3 * third])


def test_markers_and_regions(project):
    _, markers, _, _ = read(project)
    assert markers == [('intro', 1.0, 1.0), ('verse', 2.0, 5.0)]


def test_looped_midi_is_unrolled_across_the_tempo_change_and_muted_tracks_are_left_out(project):
    _, _, lanes, _ = read(project)
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
    tmap, markers, lanes, _ = read(project)
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


def _project(tmp_path, item='', source=None, head='', track=''):
    """A one-track 120 bpm 4/4 project; the track holds one item at 1 s, 1 s long, with the given extra lines."""
    if source is None:
        source = """E 0 90 3c 60
        E 240 80 3c 00
        E 240 90 40 50
        E 240 80 40 00
        E 240 b0 7b 00"""
    if not source.lstrip().startswith('HASDATA'):
        source = 'HASDATA 1 960 QN\n' + source
    p = tmp_path / 'one.RPP'
    p.write_text(f"""<REAPER_PROJECT 0.1 "7.0" 0
  TEMPO 120 4 4 0
  {head}
  <TRACK
    NAME One
    {track}
    <ITEM
      POSITION 1
      LENGTH 1
      LOOP 0
      {item}
      <SOURCE MIDI
        {source}
      >
    >
  >
>
""")
    return p


def _notes(path):
    _, _, lanes, _ = read(path)
    return [(round(t, 6), pitch, vel, round(ln, 6)) for t, pitch, vel, ln in lanes[0].events]


def test_plain_item(tmp_path):
    # The reference for the item tests below: notes at ticks 0 and 480 of a 960-PPQ, 1-QN source.
    assert _notes(_project(tmp_path)) == [(1.0, 60, 96, 0.125), (1.25, 64, 80, 0.125)]


def test_soffs_starts_the_item_inside_its_source(tmp_path):
    # 0.25 s of source offset is half a QN: the note at tick 0 falls before the item, the one at 480 plays at 1 s.
    assert _notes(_project(tmp_path, item='SOFFS 0.25')) == [(1.0, 64, 80, 0.125)]


def test_item_playrate_compresses_the_source(tmp_path):
    assert _notes(_project(tmp_path, item='PLAYRATE 2 1 0 -1 0 0.0025')) == [(1.0, 60, 96, 0.0625),
                                                                            (1.125, 64, 80, 0.0625)]


def test_ticks_per_quarter_note_other_than_960(tmp_path):
    source = """HASDATA 1 480 QN
        E 0 90 3c 60
        E 120 80 3c 00
        E 120 90 40 50
        E 120 80 40 00
        E 120 b0 7b 00"""
    assert _notes(_project(tmp_path, source=source)) == [(1.0, 60, 96, 0.125), (1.25, 64, 80, 0.125)]


def test_overlapping_same_pitch_notes_pair_first_in_first_out(tmp_path):
    source = """E 0 90 3c 60
        E 240 90 3c 50
        E 240 80 3c 00
        E 240 80 3c 00
        E 240 b0 7b 00"""
    assert _notes(_project(tmp_path, source=source)) == [(1.0, 60, 96, 0.25), (1.125, 60, 80, 0.25)]


def test_note_still_open_at_the_end_of_the_source_lasts_to_the_end(tmp_path):
    source = """E 0 90 3c 60
        E 480 b0 7b 00
        E 480 b0 7b 00"""
    assert _notes(_project(tmp_path, source=source)) == [(1.0, 60, 96, 0.5)]


@pytest.mark.parametrize(('head', 'item', 'message'), [
    ('<TEMPOENVEX\n    PT 0 120 0\n    PT 4 90 1\n  >', '', 'tempo ramp'),
    ('PLAYRATE 1.5 0 0.25 4', '', 'play rate'),
    ('<MASTERPLAYSPEEDENV\n    PT 0 1 0\n  >', '', 'play-speed envelope'),
    ('', 'TAKE', 'several takes'),
])
def test_refusals(tmp_path, head, item, message):
    with pytest.raises(ValueError, match=message):
        read(_project(tmp_path, item=item, head=head))


def test_render_range_entire_project_is_read(tmp_path):
    assert read(_project(tmp_path, head='RENDER_RANGE 1 0 0 0 1000'))[3] is True
    assert read(_project(tmp_path, head='RENDER_RANGE 2 0 0 0 1000'))[3] is False


SR = 48000


def _clicks(times, offset, duration, doubled=False, lead=0.0, floor=False, extra=()):
    """A mono track with a short noise burst at each project time, shifted onto the wav by offset.

    lead: seconds of quiet noise before the first click; floor: quiet noise over the whole track, so the wav is
    audible from its first sample; extra: wav times of clicks no event explains.
    """
    rng = np.random.default_rng(0)
    n = round(0.005 * SR)
    burst = rng.standard_normal(n) * np.exp(-np.arange(n) / 40.0) * 0.5
    x = np.zeros(round(duration * SR))
    starts = [t - offset for t in times] + ([t - offset + 0.032 for t in times] if doubled else []) + list(extra)
    for t in starts:
        i = round(t * SR)
        x[i:i + n] += burst
    if lead:
        first = round((min(times) - offset - lead) * SR)
        x[first:first + round(lead * SR)] += 0.001 * rng.standard_normal(round(lead * SR))
    if floor:
        x += 0.001 * rng.standard_normal(len(x))
    return x


def _arrangement():
    """Two lanes in a loop that repeats every beat at 120 bpm: plain beat correlation cannot place it."""
    return [[1.0 + 0.5 * k for k in range(40)], [1.25 + 0.5 * k for k in range(40)]]


def _irregular():
    """Two lanes at irregular times, so only the true lag lines every event up."""
    rng = np.random.default_rng(1)
    t = np.cumsum(rng.uniform(0.15, 0.6, 80)) + 1.0
    return [sorted(t[::2].tolist()), sorted(t[1::2].tolist())]


def test_search_finds_a_shift_the_anchor_cannot_give():
    # The noise floor makes the wav audible from sample 0, so the anchor is a whole second off and untrusted;
    # the search itself has to find the 237.1 ms shift.
    lanes = _irregular()
    mono = _clicks(lanes[0] + lanes[1], 0.2371, 40.0, floor=True)
    offset, residual, anchor, how, ratio, matched, total, _ = off.measure(lanes, mono, SR)
    assert abs(anchor - 0.2371) > 0.5
    assert how.startswith('search')
    assert abs(offset - 0.2371) < 0.005
    assert residual < 2
    assert ratio >= off.MIN_RATIO
    assert matched == total == 80


def test_sharp_start_places_a_loop_that_repeats_every_beat():
    lanes = _arrangement()
    mono = _clicks(lanes[0] + lanes[1], 0.2371, 25.0)
    offset, *_ = off.measure(lanes, mono, SR)
    assert abs(offset - 0.2371) < 0.005


def test_quiet_lead_in_far_from_the_offset_is_never_a_confident_wrong_value():
    # One second of quiet noise before the first click moves the anchor two beats off, where the loop fits too.
    lanes = _arrangement()
    mono = _clicks(lanes[0] + lanes[1], -0.5, 25.0, lead=1.0)
    try:
        offset, *_ = off.measure(lanes, mono, SR)
    except off.OffsetError as e:
        assert 'ambiguous' in str(e)
    else:
        assert abs(offset + 0.5) < 0.005


def test_render_bounds_place_a_loop_with_no_sharp_start():
    # Like afterimage: audible from sample 0, the first event at project 0, the render starting at project 0.
    lanes = [[0.5 * k for k in range(40)], [0.25 + 0.5 * k for k in range(40)]]
    mono = _clicks(lanes[0] + lanes[1], 0.0, 25.0, floor=True)
    offset, *_ = off.measure(lanes, mono, SR, bounds=(0.0, 0.0))
    assert abs(offset) < 0.005
    with pytest.raises(off.OffsetError, match='outside the offsets the project allows'):
        off.measure(lanes, mono, SR, bounds=(1.0, 1.2))


def test_sharp_start_that_contradicts_a_decisive_search_is_refused():
    # A loud click half a second before the first event's sound starts the wav sharply but in the wrong place.
    lanes = _irregular()
    first = min(lanes[0][0], lanes[1][0])
    mono = _clicks(lanes[0] + lanes[1], 0.2, 40.0, extra=[first - 0.2 - 0.5])
    with pytest.raises(off.OffsetError, match='sharp start'):
        off.measure(lanes, mono, SR)


def test_two_equally_good_peaks_are_ambiguous():
    lanes = _arrangement()
    mono = _clicks(lanes[0] + lanes[1], 0.2, 25.0, doubled=True)
    with pytest.raises(off.OffsetError, match='ambiguous'):
        off.measure(lanes, mono, SR)


def test_cli_exits_non_zero_with_a_message_on_an_ambiguous_offset(project, tmp_path, capsys):
    _, _, lanes, _ = read(project)
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


def _starts(on):
    return np.flatnonzero(on & ~np.r_[False, on[:-1]]).tolist()


@pytest.mark.parametrize('lead_ms', [0, 30, 60, 90, 120])
def test_check_clip_lead_moves_every_flash_and_tick_earlier(lead_ms):
    side = {'beats': [1.0 + 0.5 * k for k in range(8)], 'downbeats': [1.0, 3.0],
            'lanes': [{'name': 'Kick', 'events': [{'t': 1.25 + 0.5 * k} for k in range(6)]}]}
    shift = round(lead_ms * checkclip.FPS / 1000)
    plain = checkclip.states(side, 0.5, 300)
    led = checkclip.states(side, 0.5, 300, lead_ms)
    for a, b in zip([plain[0], plain[1], *plain[2]], [led[0], led[1], *led[2]], strict=True):
        assert _starts(b) == [k - shift for k in _starts(a)]
    # Without lead an event lights the first frame at or after it: ceil, never earlier.
    assert _starts(plain[0]) == [30, 150]
    assert _starts(plain[2][0]) == [45 + 30 * k for k in range(6)]
