#!/usr/bin/env python3
"""Summarize a Chrome trace from `econoplex3d` built with `--features profile`.

    python tools/profile/analyze.py trace.json [--log run.log] [--skip 3]
        [--out summary.json] [--clip clip.json --clip-at 15 --clip-len 0.5]

The trace is large (about 150 MB per second of running: Bevy records a span
for every system of every schedule, on every thread), so it is read one line
at a time and only sums are kept. Only the standard library is needed.

It reports:
- frame times over the run (start of one `main app` span to the next), so
  thermal throttling shows as a drift;
- self time (a span's time minus its children's) per system or span, in ms per
  frame, by where it ran: the main world (main thread and task-pool workers),
  the render world (render thread and workers), the simulation thread;
- the simulation thread's ticks and snapshots;
- with `--log` (the stderr of a run with `--fps`), GPU time per render pass;
- with `--clip`, a short window of the trace without file/line fields, small
  enough to open in https://ui.perfetto.dev.
"""

import argparse
import json
import re
import statistics
import sys
from collections import defaultdict

FIELDS = re.compile(r'"\.file":"(?:[^"\\]|\\.)*",|"\.line":\d+,')


def short(name):
    """`system: name="bevy_pbr::render::light::prepare_lights"` -> (kind,
    `bevy_pbr`, `bevy_pbr::light::prepare_lights`)."""
    m = re.match(r'^([\w ]+?): (?:name|system)="(.+)"$', name)
    if m:
        kind, full = m.group(1), m.group(2)
    else:
        kind, full = name.split(":")[0], name.rstrip(": ")
    full = re.sub(r"<.*>", "<..>", full)
    parts = full.split("::")
    crate = parts[0] if len(parts) > 1 else "(bevy internals)"
    label = full if len(parts) <= 3 else f"{parts[0]}::{parts[-2]}::{parts[-1]}"
    if kind not in ("system", ""):
        label = f"{kind}: {label}" if kind != label else label
    return kind, crate, label


def gpu_passes(log_path):
    """GPU ms per pass from LogDiagnosticsPlugin lines such as
    `render/main_opaque_pass_3d/elapsed_gpu : 1.23ms (avg 1.20ms)`."""
    out = defaultdict(list)
    cpu = defaultdict(list)
    frame = []
    pat = re.compile(r"render/(\S+?)/elapsed_(gpu|cpu)\s*:\s*([\d.]+)ms")
    fpat = re.compile(r"frame_time\s*:\s*([\d.]+)ms")
    for line in open(log_path, encoding="utf-8", errors="replace"):
        line = re.sub(r"\x1b\[[0-9;]*m", "", line)
        m = pat.search(line)
        if m:
            (out if m.group(2) == "gpu" else cpu)[m.group(1)].append(float(m.group(3)))
        m = fpat.search(line)
        if m:
            frame.append(float(m.group(1)))

    def mean(d):
        return {k: statistics.mean(v[len(v) // 4:] or v) for k, v in d.items()}

    return mean(out), mean(cpu), frame


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser()
    ap.add_argument("trace")
    ap.add_argument("--log")
    ap.add_argument("--skip", type=float, default=3.0, help="seconds of start-up to leave out")
    ap.add_argument("--out", default="profile-summary.json")
    ap.add_argument("--top", type=int, default=25)
    ap.add_argument("--clip", help="write a short Perfetto-sized window of the trace here")
    ap.add_argument("--clip-at", type=float, default=15.0, help="seconds into the run")
    ap.add_argument("--clip-len", type=float, default=0.5)
    a = ap.parse_args()

    names = {}
    stacks = defaultdict(list)  # tid -> [[name, ts, child_us]]
    self_us = defaultdict(float)  # (tid, name) -> us
    calls = defaultdict(int)
    frames = []
    sched = defaultdict(list)  # (tid, schedule) -> durations
    top = defaultdict(list)  # name of a top-level span on main/render -> durations
    sim = {"tick": [], "snapshot": []}
    t_first = None
    clip_lo = clip_hi = None
    clip = open(a.clip, "w", encoding="utf-8") if a.clip else None
    timeline = []
    window = []
    # Which world was running when a span began: the main app's schedules
    # (`main app`), the render world's (`schedule: Render`), or the extract
    # step. Systems of both worlds share the task-pool threads (and the main
    # and render threads help out while they wait), so a span's thread does
    # not say which world it belongs to. Spans that begin while only one world
    # runs say it; each name then takes the world seen most often.
    active = {"main app": 0, "schedule: name=Render": 0, "schedule: name=ExtractSchedule": 0}
    hinted = defaultdict(lambda: defaultdict(float))  # name -> hint -> us

    if clip:
        clip.write("[\n")
    first_clip = True
    for line in open(a.trace, encoding="utf-8", errors="replace"):
        line = line.strip().rstrip(",")
        if not line.startswith("{"):
            continue
        try:
            e = json.loads(line)
        except json.JSONDecodeError:
            continue
        ph = e.get("ph")
        tid = e.get("tid", 0)
        if ph == "M":
            if e.get("name") == "thread_name":
                names[tid] = e["args"]["name"]
            if clip:
                clip.write(("" if first_clip else ",\n") + line)
                first_clip = False
            continue
        ts = e["ts"]
        # Spans without fields are named like `main app: `.
        if e["name"].endswith(": "):
            e["name"] = e["name"][:-2]
        if t_first is None:
            t_first = ts
            clip_lo = t_first + a.clip_at * 1e6
            clip_hi = clip_lo + a.clip_len * 1e6
        if clip and clip_lo <= ts <= clip_hi:
            clip.write(("" if first_clip else ",\n") + FIELDS.sub("", line))
            first_clip = False
        if clip_lo <= ts <= clip_lo + 0.25e6:
            window.append((tid, e["name"], ph, ts))
        if ph == "B":
            if active["schedule: name=ExtractSchedule"]:
                hint = "extract"
            elif active["main app"] and active["schedule: name=Render"]:
                hint = "both"
            elif active["main app"]:
                hint = "main world"
            elif active["schedule: name=Render"]:
                hint = "render world"
            else:
                hint = "none"
            stacks[tid].append([e["name"], ts, 0.0, hint])
            if e["name"] in active:
                active[e["name"]] += 1
            if e["name"] == "main app":
                frames.append(ts)
        elif ph == "E" and stacks[tid]:
            name, b, child, hint = stacks[tid].pop()
            if name in active:
                active[name] -= 1
            dur = ts - b
            if ts - t_first < a.skip * 1e6:
                continue
            self_us[(tid, name)] += dur - child
            hinted[name][hint] += dur - child
            calls[(tid, name)] += 1
            if stacks[tid]:
                stacks[tid][-1][2] += dur
            elif tid in (0,) or names.get(tid) == "Render thread":
                top[(tid, name)].append(dur)
            if name.startswith("schedule: "):
                sched[(tid, name[len("schedule: name="):])].append(dur)
            if name == "sim tick":
                sim["tick"].append(dur)
            elif name == "sim snapshot":
                sim["snapshot"].append(dur)
    if clip:
        clip.write("\n]\n")
        clip.close()

    start = t_first + a.skip * 1e6
    frames = [f for f in frames if f >= start]
    end = frames[-1]
    n = len(frames) - 1
    secs = (end - start) / 1e6
    frame_ms = [(y - x) / 1000 for x, y in zip(frames, frames[1:])]

    main_tid = 0
    render_tid = next((t for t, v in names.items() if v == "Render thread"), None)
    sim_tid = next((t for t, v in names.items() if v == "simulation"), None)

    def world(tid, name):
        if tid == sim_tid:
            return "simulation"
        if name == "main app" or name.startswith("schedule: name=") and tid == main_tid and name != "schedule: name=ExtractSchedule":
            return "main world"
        if name.startswith("sub app: name=RenderApp") or name.startswith(("schedule: name=Render", "schedule: name=RenderGraph")):
            return "render world"
        h = hinted[name]
        if h.get("extract", 0) > 0.5 * sum(h.values()):
            return "extract"
        sure = {k: v for k, v in h.items() if k in ("main world", "render world")}
        if sure:
            return max(sure, key=sure.get)
        # Only ever seen while both worlds ran: go by Bevy's naming, where the
        # render world's systems prepare, queue, specialize and write buffers.
        fn = short(name)[2].split("::")[-1]
        if re.match(r"(prepare|queue|specialize|write|batch|collect|upload|sort|finish|prepared|clear|reset)_?", fn):
            return "render world"
        return "main world"

    by = defaultdict(lambda: defaultdict(float))
    by_crate = defaultdict(lambda: defaultdict(float))
    ncalls = defaultdict(lambda: defaultdict(float))
    per_thread = defaultdict(float)
    for (tid, name), us in self_us.items():
        if name in ("main app", "render app", "frame", "update") or name.startswith(
            ("schedule: ", "multithreaded executor", "sub app")
        ):
            # Containers: their self time is the executor's own bookkeeping
            # and waiting for workers; counted under "(scheduling)".
            label, crate = "(scheduling and waiting)", "(scheduling)"
        else:
            _, crate, label = short(name)
        w = world(tid, name)
        by[w][label] += us / 1000 / n
        by_crate[w][crate] += us / 1000 / n
        ncalls[w][label] += calls[(tid, name)] / n
        per_thread[names.get(tid, str(tid))] += us / 1000 / secs  # ms busy per s

    def ranked(d):
        return dict(sorted(d.items(), key=lambda kv: -kv[1]))

    # A timeline of 0.25 s of all threads, for the frame chart.
    st = defaultdict(list)
    for tid, name, ph, ts in window:
        if ph == "B":
            st[tid].append((name, ts))
        elif ph == "E" and st[tid]:
            nm, b = st[tid].pop()
            if ts - b >= 30:
                timeline.append({"tid": tid, "thread": names.get(tid, str(tid)), "name": short(nm)[2],
                                 "ts": round((b - clip_lo) / 1000, 3), "dur": round((ts - b) / 1000, 3),
                                 "depth": len(st[tid])})

    def stats(v):
        v = sorted(x / 1000 for x in v)
        return {"mean": statistics.mean(v), "p50": v[len(v) // 2], "p95": v[int(len(v) * 0.95)],
                "count_per_frame": len(v) / n} if v else None

    summary = {
        "seconds": secs,
        "frames": n,
        "fps": n / secs,
        "frame_ms": {"mean": statistics.mean(frame_ms), "p50": statistics.median(frame_ms),
                     "p95": sorted(frame_ms)[int(n * 0.95)], "max": max(frame_ms)},
        "frame_series": [[round((f - t_first) / 1e6, 3), round(ms, 3)] for f, ms in zip(frames, frame_ms)],
        "threads": names,
        "top_level": {f"{names.get(t, t)} / {k}": stats(v) for (t, k), v in top.items()},
        "schedules": {f"{names.get(t, t)} / {k}": stats(v) for (t, k), v in sched.items()
                      if t in (main_tid, render_tid, sim_tid)},
        "self_ms_per_frame": {w: ranked(d) for w, d in by.items()},
        "crate_ms_per_frame": {w: ranked(d) for w, d in by_crate.items()},
        "calls_per_frame": {w: dict(d) for w, d in ncalls.items()},
        "thread_busy_ms_per_s": ranked(per_thread),
        "sim": {
            "ticks_per_s": len(sim["tick"]) / secs,
            "tick_ms": stats(sim["tick"]),
            "snapshots_per_s": len(sim["snapshot"]) / secs,
            "snapshot_ms": stats(sim["snapshot"]),
        },
        "timeline_at_s": a.clip_at,
        "timeline": timeline,
    }
    if a.log:
        summary["gpu_ms_per_pass"], summary["cpu_ms_per_pass"], summary["log_frame_ms"] = gpu_passes(a.log)
    json.dump(summary, open(a.out, "w"), indent=1)

    print(f"{n} frames in {secs:.1f} s: {summary['fps']:.0f} fps, frame "
          f"{summary['frame_ms']['mean']:.2f} ms mean, {summary['frame_ms']['p95']:.2f} ms p95")
    for w, d in summary["self_ms_per_frame"].items():
        print(f"\n== {w}: {sum(d.values()):.2f} ms of CPU per frame (all threads)")
        for k, v in list(d.items())[: a.top]:
            print(f"  {v:7.3f}  {k}")
    print("\n== busy ms per second, by thread")
    for k, v in summary["thread_busy_ms_per_s"].items():
        print(f"  {v:7.1f}  {k}")
    print("\n== sim", json.dumps(summary["sim"]))
    if a.log:
        print("\n== GPU ms per pass")
        for k, v in sorted(summary["gpu_ms_per_pass"].items(), key=lambda kv: -kv[1]):
            print(f"  {v:7.3f}  {k}")
    print(f"\nwrote {a.out}")


if __name__ == "__main__":
    main()
