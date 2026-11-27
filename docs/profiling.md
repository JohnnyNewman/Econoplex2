# Profiling the 3D view

Where each frame's time goes, per system, render pass and simulation tick.
It uses Bevy's own tracing spans; nothing is recorded unless the binary is
built with one of the two features.

| Feature | What it does |
| --- | --- |
| `profile` | Writes every span to a Chrome trace file (`bevy/trace_chrome`). |
| `profile-tracy` | Streams the same spans to the [Tracy](https://github.com/wolfpld/tracy) profiler (`bevy/trace_tracy`). Start Tracy first and connect. |

Always profile release builds.

## Record a trace

```sh
cargo build --release -p game3d --features profile
TRACE_CHROME=trace.json target/release/econoplex3d --fps --quit-after 15 2> run.log
```

- `--quit-after S` quits cleanly after S seconds, which writes out the trace.
  If the app is killed instead, the file lacks its closing bracket; the
  analyzer repairs that.
- `--fps` turns off vsync and logs frame times and GPU time per render pass
  (timestamp queries) into `run.log`.
- The trace grows by about 150 MB per second: Bevy records each system of
  every schedule on every thread, roughly 1,700 spans per frame. Keep runs short.
- Profiling costs 1 to 3 ms per frame, and the laptop GPU and CPU throttle when
  warm. Compare runs from the same build, and alternate them.

## Summarize it

```sh
python tools/profile/analyze.py trace.json --log run.log --clip clip.json --clip-at 8
```

It reads the trace one line at a time (standard library only) and prints:

- frame times (from one `main app` span to the next), mean and p95;
- self time (a span minus its children) per system, in ms per frame summed
  over all threads, split into main world, render world, extract and the
  simulation thread. Both worlds share the task-pool threads, so a span's
  world is taken from which world was running when it began;
- how busy each thread is, the simulation's ticks and snapshots;
- with `--log`, GPU ms per render pass.

It also writes `profile-summary.json` with all of this plus a frame-time series
and a 0.25 s timeline of all threads. `--clip` writes a short slice of the
trace (0.5 s by default, at `--clip-at` seconds) without file and line fields,
small enough to open in <https://ui.perfetto.dev>.

## How a frame runs

Bevy uses pipelined rendering: the main thread runs the game's schedules
(`PreUpdate`, `Update`, `PostUpdate`, ...) and then the extract step, while the
render thread renders the previous frame (`Render` schedule, then the
`RenderGraph`). The task-pool workers run systems of both. The frame rate is
set by whichever of the two threads is slower; in the measurements of
2026-11-13 that was the render thread (about 9 ms of a 10 ms frame), with the
GPU busy only about 2 ms.

Named spans of our own: `sim tick` and `sim snapshot` on the simulation thread
(`live.rs`).

## Measurements of 2026-11-27

After the look work (water, grounding, fields, smoke, cloud shadows,
grading, people, sheep) the frame is GPU-bound: the render thread spends
about 3 ms a frame in `prepare_windows`, waiting for the next swapchain
image. GPU per frame was about 6 ms, of which:

| pass | ms |
| --- | --- |
| main opaque pass | 4.3 |
| sun shadows (all cascades) | 0.3 |
| clustering | 0.3 |
| tonemapping | 0.26 |

So caching the shadow map would save at most 0.3 ms; it was dropped. The
main pass shaded about 9 million fragments for 3.2 million pixels: each
pixel was shaded almost three times, under houses, trees and grass. A
**depth prepass** (`DepthPrepass` on the camera) cut the main pass to 2.0 ms
for 0.25 ms of prepass. Materials whose vertex shader moves the geometry
(grass, swaying trees, people, sheep) opt out (`enable_prepass` returns
false). Otherwise the default prepass would write their depth where they
stand still, and the main pass would lose the parts that moved (pale
patches in the tree crowns). The terrain has its own prepass vertex shader,
so it takes part and gains the most.

Frame rates in these runs swing between about 85 and 150 fps from one run to
the next, as the laptop throttles. Compare GPU pass times, not fps.


## Dynamic resolution

The render scale now follows the frame time by default (`render_scale.rs`,
`AutoScale`). Frames clearly slower than 60 a second step the 3D view down by
0.05 a second, to 0.6 at the least. Frames that keep up try a step up after a
few seconds, waiting twice as long after a try that didn't hold. It only
watches while the window has the focus: a covered window on vsync was slowed
to 5 to 10 fps by the system in a test, which would have dragged the scale to
its floor. It's off with `--render-scale F` (a fixed scale), `--no-auto-scale`,
`--shot` and `--fps`, so screenshots and measurements stay at full resolution.
Each change logs `render scale …`.
