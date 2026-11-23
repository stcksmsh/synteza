# scripts/look — judging renders by eye

Tools for comparing two versions of the visualizer's look. Python 3 with numpy and
Pillow, plus ffmpeg/ffprobe on `PATH`.

## Contact sheet

One PNG of a video at fixed instants, each tile labelled with its time:

```
python3 scripts/look/contact_sheet.py clip.mp4 sheet.png            # 12 tiles, 4 columns
python3 scripts/look/contact_sheet.py clip.mp4 sheet.png --span 96  # instants spread over 0-96 s
python3 scripts/look/contact_sheet.py clip.mp4 sheet.png --times 4,32,60
```

The default instants are the middles of `--count` equal slices of `--span` seconds
(default: the clip's duration). Give both versions the same `--span` (or `--times`) and
tile N of each sheet shows the same moment.

## Blind A/B

`ab.html` shows two renders of the same audio side by side, swapped left/right at random
on every trial, and plays both in step with one audio clock. The owner picks left, right
or same, with an optional note. The log records the unblinded answer.

```
python3 scripts/look/serve.py /secondary/hyst-data/synteza/before
# open http://127.0.0.1:8770/ab.html?a=one.mp4&b=two.mp4&audio=track.wav&trials=3&log=my-test
```

- `a`, `b`: the two clips, relative to the served directory.
- `audio`: what to play; defaults to `a` (clips from `render_before` below carry the track).
- `trials`: how many blinded presentations (default 3). `log`: the log's file name.

`serve.py` is needed for seeking: Python's `http.server` ignores Range requests, so a
browser cannot seek video it serves. It listens on 127.0.0.1 only. After each verdict the
page writes `<log>.json` into the served directory (and keeps a copy in the browser, so a
reload resumes the run; "Download log" saves it too). Each verdict has `trial`, `left`,
`right`, `choice` (`left`/`right`/`same`), `preferred` (the file chosen, or `same`),
`note` and `timestamp`.

## "Before" renders

`make_synth_clip.py OUT.wav` writes a rights-clean 96 s test track at 124 BPM: intro,
build, drop at 31 s, break 58-74 s, a short build, second drop. It is deterministic.

The Julia renderer (`crates/hyst-render/examples/audio_driven_render.rs`) takes its
size and length from `JULIA_WIDTH`, `JULIA_HEIGHT` (width a multiple of 64) and
`JULIA_SECONDS`, and writes to `JULIA_OUT_DIR`. The "before" clips were made with:

```
cargo build --release -p hyst-render --example audio_driven_render
JULIA_OUT_DIR=<dir> JULIA_WIDTH=1280 JULIA_HEIGHT=720 JULIA_SECONDS=96 \
  target/release/examples/audio_driven_render track.wav
ffmpeg -i <dir>/audio_driven_render.mp4 -i track.wav -map 0:v -map 1:a -c:v copy -c:a aac -shortest clip.mp4
```

at 1280x720 and 576x1024 (9:16; 720 is not a multiple of 64), on the synthetic track and
on `hysteresis.wav`. Clips live in `/secondary/hyst-data/synteza/before/`, outside git.
Their contact sheets (`--span 96`) are in `docs/plan-evidence/before/`.
