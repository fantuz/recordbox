"""Warm, sequential, CPU-pinned A/B runs. Alternates the starting branch between order-balanced blocks."""
import argparse
import datetime
import json
import os
import platform
import shlex
import statistics
import subprocess
import sys
from pathlib import Path

ap = argparse.ArgumentParser()
ap.add_argument("--cpu", type=int, required=True)
ap.add_argument("--bulk", type=int, default=14_000_000)
ap.add_argument("--latency", type=int, default=150_000)
ap.add_argument("--blocks", type=int, default=4)
ap.add_argument("--out", type=Path, default=Path("results.json"))
args = ap.parse_args()
if args.cpu not in os.sched_getaffinity(0) or min(args.blocks, args.bulk, args.latency) < 1:
    ap.error("Pick an allowed CPU and strictly positive budgets.")
os.sched_setaffinity(0, {args.cpu})
root = Path(__file__).resolve().parent
bindir = root / "target" / "release"
def run(key, *params):
    return json.loads(subprocess.check_output([str(bindir / key), *params], text=True))

def optional_text(file):
    try:
        return Path(file).read_text().strip()
    except OSError:
        return None

def quartiles(values):
    points = statistics.quantiles(values, n=4, method="inclusive")
    return [round(value, 4) for value in points]

results = {
    "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "command": shlex.join(sys.argv),
    "platform": platform.platform(),
    "model": next((s.split(":", 1)[-1].strip() for s in Path("/proc/cpuinfo").read_text().splitlines() if s.startswith("model name")), None),
    "pinned_cpu": sorted(os.sched_getaffinity(0)),
    "cgroup_cpu_quota": optional_text("/sys/fs/cgroup/cpu.max"),
    "governor": optional_text(f"/sys/devices/system/cpu/cpu{args.cpu}/cpufreq/scaling_governor"),
    "provenance": json.loads((root/"provenance.json").read_text()),
    "oracle": {key:run(key, "verify") for key in ["fork","upstream"]},
    "runs": [],
}
for mode, budget in [("bulk", args.bulk), ("latency", args.latency)]:
    for block in range(args.blocks):
        order = ("fork", "upstream", "upstream", "fork")
        if block % 2:
            order = tuple(("fork" if key == "upstream" else "upstream") for key in order)
        for slot, key in enumerate(order):
            row = {"mode":mode, "block":block+1, "slot":slot+1, "branch":key}
            row.update(run(key, mode, str(budget)))
            results["runs"].append(row)
summary = {}
for key in ("fork", "upstream"):
    rows = [r for r in results["runs"] if r["branch"]==key]
    bulk = [r["ns_per_packet"] for r in rows if r["mode"]=="bulk"]
    lat = [r for r in rows if r["mode"]=="latency"]
    med_ns = statistics.median(bulk)
    summary[key] = {
        "per_mode_runs": len(bulk),
        "bulk_ns_per_packet_q1_median_q3": quartiles(bulk),
        "bulk_million_packets_per_second": round(1000/med_ns,3),
        "map_single_call_ns_p50_p95_p99_median_over_runs":
            [statistics.median(r["map_ns_p50_p95_p99"][i] for r in lat) for i in range(3)],
        "timer_blank_ns_p50_p95_p99_median_over_runs":
            [statistics.median(r["blank_ns_p50_p95_p99"][i] for r in lat) for i in range(3)],
    }
results["summary"] = summary
args.out.write_text(json.dumps(results, indent=2) + "\n")
print(json.dumps(summary, indent=2))
