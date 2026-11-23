"""Contact sheet: one PNG of a video at fixed instants, each tile labelled with its time.

Two renders compared at the same instants should use the same --times (or the same
--count and --span), so tile N of both sheets shows the same moment.

Usage:
  python3 contact_sheet.py IN.mp4 OUT.png [--count 12] [--cols 4] [--span 96]
                                          [--times 0,8,16,...] [--tile-width 400]

By default the times are --count instants evenly spaced over --span seconds
(default: the video's duration), at the middle of each slice.
"""

import argparse
import io
import json
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFont


def duration_of(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "json", path],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return float(json.loads(out)["format"]["duration"])


def frame_at(path, seconds):
    png = subprocess.run(
        ["ffmpeg", "-v", "error", "-ss", f"{seconds:.3f}", "-i", path,
         "-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"],
        check=True,
        capture_output=True,
    ).stdout
    if not png:
        raise ValueError(f"no frame at {seconds:.3f} s in {path}")
    return Image.open(io.BytesIO(png)).convert("RGB")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("video")
    p.add_argument("out")
    p.add_argument("--count", type=int, default=12)
    p.add_argument("--cols", type=int, default=4)
    p.add_argument("--span", type=float, help="seconds to spread the instants over")
    p.add_argument("--times", help="comma-separated seconds; overrides --count/--span")
    p.add_argument("--tile-width", type=int, default=400)
    a = p.parse_args()

    duration = duration_of(a.video)
    if a.times:
        times = [float(x) for x in a.times.split(",")]
    else:
        if a.count < 1:
            sys.exit("--count must be at least 1")
        span = a.span if a.span else duration
        times = [span * (i + 0.5) / a.count for i in range(a.count)]
    late = [x for x in times if not 0 <= x < duration]
    if late:
        sys.exit(f"times outside the video's 0-{duration:.2f} s: {late}")
    if a.cols < 1 or a.tile_width < 16:
        sys.exit("--cols must be >= 1 and --tile-width >= 16")

    frames = [frame_at(a.video, x) for x in times]
    tw = a.tile_width
    th = round(frames[0].height * tw / frames[0].width)
    label_h = 28
    cols = min(a.cols, len(frames))
    rows = -(-len(frames) // cols)
    sheet = Image.new("RGB", (cols * tw, rows * (th + label_h)), (16, 16, 16))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=18)
    for i, (x, im) in enumerate(zip(times, frames, strict=True)):
        cx, cy = (i % cols) * tw, (i // cols) * (th + label_h)
        sheet.paste(im.resize((tw, th), Image.LANCZOS), (cx, cy))
        m, s = divmod(x, 60)
        draw.text((cx + 6, cy + th + 4), f"{int(m)}:{s:05.2f}", fill=(230, 230, 230), font=font)
    sheet.save(a.out)


if __name__ == "__main__":
    main()
