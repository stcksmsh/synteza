# Third-party licences

What each source's own page or LICENSE file says, read on 2026-11-23 (all dates below are this day). Target: synteza code under GPL-3.0-or-later (proposed). This is a reading of the licence text, not legal advice.

Verdicts: **Use** = usable under GPL-3.0-or-later as dependency or ported code. **Idea** = reference only. **No** = not usable.

| Source | URL read | Licence found | Code | Weights / data | Verdict |
|---|---|---|---|---|---|
| Shadertoy terms | https://www.shadertoy.com/terms | **Could not read.** Cloudflare returned 403 to plain fetches; the Wayback copy returned 429. | unknown | n/a | **No** until read by hand in a browser. Do not rely on "CC BY-NC-SA 3.0" from memory. |
| Vidvox ISF-Files | https://github.com/Vidvox/ISF-Files (LICENSE) | MIT, "Copyright (c) 2018" (no holder named) | MIT | n/a | **Use** the repo-level grant. Each `.fs` has its own `CREDIT` field; some may be ports of third-party work with their own terms. Check per file before copying one. |
| projectM Cream of the Crop | https://github.com/projectM-visualizer/presets-cream-of-the-crop (LICENSE.md) | None. LICENSE.md says presets were "in almost all cases, not released under any specific license" and that it is "safe to assume them to be in the public domain". GitHub reports NOASSERTION. | n/a | presets: no licence | **No** to ship or bundle. The maintainers' assumption is not a grant from the authors. |
| Playdead temporal | https://github.com/playdeadgames/temporal (LICENSE.txt) | MIT, "Copyright (c) [2015] [Playdead]" (template placeholders left in) | MIT | n/a | **Use** (port or adapt, keep the notice). |
| msdfgen | https://github.com/Chlumsky/msdfgen (LICENSE.txt, README) | MIT, "Copyright (c) 2014 - 2026 Viktor Chlumsky" | MIT | n/a | **Use.** README lists FreeType, TinyXML2, libpng and optionally Skia as dependencies; their licences were not read here. |
| Peters blue noise | http://momentsingraphics.de/BlueNoise.html | CC0 1.0 for the downloads | the page applies the same dedication to the code in the download | textures CC0 | **Use** (textures and code). |
| Real-time PLP | https://github.com/groupmm/real_time_plp (LICENSE, README) | MIT, "Copyright (c) 2024 Fraunhofer IIS" | MIT | no model weights | **Use** (port to Rust). PLAN.md does not name the repo; this is the one GitHub search returns ("Real-Time PLP ... 2024 DAFx and TISMIR Papers"). Its demo game assets are CC0 and are not needed. |
| Beat This | https://github.com/CPJKU/beat_this (README "License", LICENSE) | MIT, "Copyright (c) 2024 ... JKU Linz" | MIT | MIT: "The code and the published model weights are released under the MIT license." Caveat in the README: some training files are fully copyrighted or under limited CC licences; the user must judge impact. | **Use** code and weights. Training-data caveat stays. |
| BeatNet | https://github.com/mjhydri/BeatNet (LICENSE, README) | CC BY 4.0 (LICENSE file is the full CC BY 4.0 text; README badge) | CC BY 4.0 | README says the pretrained models are in the repo's `models` folder and states no separate weights licence, so the repo licence is the only one found | **Use** with attribution, but a surprise: CC BY 4.0 is a content licence applied to software. Compatibility with GPL-3.0-or-later and BeatNet's own Python dependencies were not checked. Prefer treating it as **idea** unless the owner accepts that. |
| Demucs | https://github.com/facebookresearch/demucs (README, LICENSE); fork https://github.com/adefossez/demucs | MIT, "Copyright (c) Meta Platforms, Inc. and affiliates" (README: "released under the MIT license") | MIT | **No separate statement found** for the pretrained weights | **Use** code. Weights: licence not stated in the files read; MIT is an assumption, not a finding. Original repo archived 2025-01-01 (read-only, unmaintained); fork carries the same MIT LICENSE. |
| BTrack | https://github.com/adamstark/BTrack (README) | GPL-3.0-or-later ("version 3 ... or (at your option) any later version"); v1.0.7; "Copyright (c) 2014 Queen Mary University of London" | GPL-3.0-or-later | n/a | **Use** only if synteza is GPL-3.0-or-later. |
| aubio | https://github.com/aubio/aubio (README, COPYING) | GPL-3.0-or-later (README text; COPYING is GPLv3) | GPL-3.0-or-later | n/a | **Use** only if synteza is GPL-3.0-or-later. |
| Mandelbulber2 | https://github.com/buddhi1980/mandelbulber2 (README, GitHub licence API) | README says only "GNU GPL v3"; GitHub detects GPL-3.0 | GPL-3.0, "or later" not stated | n/a | **Use** formula code only after confirming per-file headers. If it is GPL-3.0-only, a port makes the combined work GPL-3.0-only, which conflicts with the proposed or-later. Otherwise **idea** (write from the maths). |

## Against PLAN.md

- §4.6 "Shadertoy default licence described as CC BY-NC-SA 3.0, unverified": still unverified. The page is behind Cloudflare. Owner must read it in a browser.
- §4.6 "ISF licence name unverified": it is MIT (2018). Per-file credits remain unchecked.
- §4.6 "Cream of the Crop has no licence statement found": confirmed, with an explicit statement that most presets have no licence. Keep out.
- §4.6 Playdead, msdfgen MIT and Peters CC0 [K]: all confirmed.
- §4.6 Mandelbulber2 GPL-3.0: README states v3 with no "or later". Plan's "GPL sources usable if synteza is GPL" needs this caveat.
- §6.1 "Real-time PLP is MIT with no weights": confirmed (groupmm/real_time_plp, Fraunhofer IIS).
- §6.1 Beat This MIT code and weights: confirmed. Demucs MIT: confirmed for code only; weights licence not stated. Demucs is archived.
- §6.1 BeatNet "CC BY 4.0": confirmed, but it covers the code too, not just weights.
- §6.2 BTrack and aubio GPLv3: confirmed as GPL-3.0-or-later.
- Not checked: BeatNet's, Beat This's and Demucs's Python dependencies, and the madmom rows of §6.2.
