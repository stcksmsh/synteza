"""Find the offset between project time and a rendered wav by matching note times to the wav.

wav time = project time - offset.

Loop-based music repeats every beat and every eighth, so note times line up with the wav's onsets nearly as
well half a beat off as at the true offset (on panicspiral the kick lane alone prefers the half-beat-off lag,
because the off-beat bass makes the larger onsets). Three steps therefore:
1. anchor: the first event against the first audible sample of the wav. This is coarse but cannot be off by
   whole or half beats.
2. peak: every lane's events against the wav's onset envelope, for lags within ANCHOR_S of the anchor. The best
   lag must clearly beat every lag more than PEAK_GAP_S away from it, or the offset is ambiguous.
3. refine: each event is matched to its nearest onset peak (sub-frame); the median gap corrects the offset and
   the median deviation from it is the residual.
"""
import numpy as np

FRAME = 256
HOP = 64
AUDIBLE = 1e-4  # -80 dBFS: the first sample above this is where the music starts
ANCHOR_S = 0.06  # how far the peak search may move from the anchor (under a sixteenth at 120 bpm)
PEAK_GAP_S = 0.015  # the runner-up is the best lag at least this far from the best one
MATCH_S = 0.030  # half-width of the window an event looks for its onset in
MIN_RATIO = 1.2  # best score over the runner-up, both measured above the median score


class OffsetError(ValueError):
    pass


def onset_envelope(mono):
    """Log spectral flux on short frames; element k is the frame centred at (k * HOP + FRAME / 2) / sr."""
    x = np.asarray(mono, np.float32)
    if len(x) < FRAME:
        raise OffsetError('audio shorter than one analysis frame')
    win = np.hanning(FRAME).astype(np.float32)
    frames = np.lib.stride_tricks.sliding_window_view(x, FRAME)[::HOP]
    flux = np.zeros(len(frames), np.float32)
    prev = None
    for a in range(0, len(frames), 8192):  # chunked to keep memory flat on long tracks
        mag = np.log1p(100.0 * np.abs(np.fft.rfft(frames[a:a + 8192] * win, axis=1))).astype(np.float32)
        lead = mag[:1] if prev is None else prev
        d = np.diff(np.concatenate([lead, mag]), axis=0)
        flux[a:a + len(mag)] = np.maximum(d, 0).sum(axis=1)
        prev = mag[-1:]
    return flux


def _lag_scores(times, env, hop_s, t0, lags):
    """score[i] = sum over events of env at the frame of (event - lags[i] * hop_s)."""
    idx = np.round((np.asarray(times) - t0) / hop_s).astype(int)
    idx = idx[idx >= 0]
    if len(idx) == 0:
        raise OffsetError('no reference events')
    train = np.zeros(idx.max() + 1)
    np.add.at(train, idx, 1.0)
    n = 1 << int(np.ceil(np.log2(len(train) + len(env))))
    corr = np.fft.irfft(np.fft.rfft(env, n).conj() * np.fft.rfft(train, n), n)
    return corr[lags % n]


def _ratio(scores, best, others):
    base = np.median(scores)
    runner = others[int(np.argmax(scores[others]))]
    return float((scores[best] - base) / max(scores[runner] - base, 1e-12)), runner


def measure(lanes, mono, sr):
    """lanes: one list of event times per lane, project seconds.

    Returns (offset_s, residual_ms, anchor_s, peak_ratio, matched, total, per-lane offset in ms or None);
    raises OffsetError when the offset is ambiguous.
    """
    x = np.asarray(mono)
    loud = np.flatnonzero(np.abs(x) > AUDIBLE)
    times = [np.unique(np.round(lane, 4)) for lane in lanes if len(lane)]
    if len(loud) == 0 or not times:
        raise OffsetError('silent wav or no events')
    anchor = min(float(t[0]) for t in times) - float(loud[0]) / sr

    env = onset_envelope(x)
    hop_s = HOP / sr
    centre = FRAME / 2 / sr
    smooth = env.copy()
    for s in (1, 2):  # widen onsets a frame or two so slight timing spread between lanes still adds up
        smooth[s:] = np.maximum(smooth[s:], env[:-s])
        smooth[:-s] = np.maximum(smooth[:-s], env[s:])
    a = round(anchor / hop_s)
    span = round(ANCHOR_S / hop_s)
    lags = np.arange(a - span, a + span + 1)
    scores = np.zeros(len(lags))
    for t in times:  # normalised so each lane weighs the same
        sc = _lag_scores(t, smooth, hop_s, centre, lags)
        scores += (sc - np.median(sc)) / len(t)
    best = int(np.argmax(scores))
    far = np.flatnonzero(np.abs(lags - lags[best]) * hop_s >= PEAK_GAP_S)
    ratio, runner = _ratio(scores, best, far)
    if ratio < MIN_RATIO:
        raise OffsetError(f'ambiguous offset: anchor {anchor * 1000:.1f} ms, best {lags[best] * hop_s * 1000:.1f} ms, '
                          f'runner-up {lags[runner] * hop_s * 1000:.1f} ms, peak ratio {ratio:.3f} < {MIN_RATIO}')
    coarse = float(lags[best] * hop_s)

    w = round(MATCH_S / hop_s)
    all_d, per_lane = [], []
    for lane in lanes:
        deltas = []
        for t in np.unique(np.round(lane, 4)):
            k = round((t - coarse - centre) / hop_s)
            if k - w < 1 or k + w + 1 >= len(env):
                continue
            j = k - w + int(np.argmax(env[k - w:k + w + 1]))
            if k - w < j < k + w:  # a peak at the window edge is a neighbour's onset, not this event's
                y0, y1, y2 = env[j - 1], env[j], env[j + 1]
                curve = y0 - 2 * y1 + y2
                frac = 0.5 * (y0 - y2) / curve if curve < 0 else 0.0  # parabolic sub-frame peak
                deltas.append((j + frac) * hop_s + centre - (t - coarse))
        per_lane.append(round((coarse - float(np.median(deltas))) * 1000, 1) if deltas else None)
        all_d += deltas
    if not all_d:
        raise OffsetError('no event matched an onset')
    all_d = np.asarray(all_d)
    d = float(np.median(all_d))
    residual = float(np.median(np.abs(all_d - d))) * 1000
    return coarse - d, residual, anchor, ratio, len(all_d), sum(len(t) for t in times), per_lane
