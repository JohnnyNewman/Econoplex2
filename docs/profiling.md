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
