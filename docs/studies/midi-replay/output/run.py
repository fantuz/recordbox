#!/usr/bin/env python3
"""Run the two release-mode probes serially and retain raw measurements."""
import argparse
import csv
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess

BASE = Path(__file__).resolve().parent
LABELS = ["fantuz", "adamsardo"]
CASES = [(case, state) for case in ["unmapped", "small", "broad", "fanout", "tagged"]
         for state in ["stable", "changed"] if case != "unmapped" or state == "stable"]


def invoke(label, *args):
    suffix = ".exe" if os.name == "nt" else ""
    return subprocess.check_output([BASE / "build" / "target" / "release" / (label + suffix), *map(str, args)],
                                   text=True)


def measure(label, case, state, ticks):
    response = invoke(label, case, state, ticks).strip().split(",")
    assert response[:3] == [case, state, str(ticks)]
    return int(response[3]), int(response[4])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cpu", type=int, help="pin the entire run to one permitted logical CPU (Linux)")
    parser.add_argument("--out", type=Path, required=True, help="new or empty results directory")
    parser.add_argument("--rounds", type=int, default=8)
    parser.add_argument("--sample-ms", type=int, default=40, help="minimum calibrated target among the two revisions")
    args = parser.parse_args()
    if args.rounds < 2 or args.rounds % 2 or args.sample_ms < 1:
        parser.error("rounds must be even and >=2, with sample-ms >=1")
    if args.cpu is not None:
        os.sched_setaffinity(0, {args.cpu})
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()):
        parser.error("output directory already contains files")
    oracle = invoke(LABELS[0], "oracle")
    assert oracle == invoke(LABELS[1], "oracle"), "wire packet streams differ"
    (out / "packets.csv").write_text(oracle)
    env = [platform.platform(), f"cpu={args.cpu}, rounds={args.rounds}, target_sample_ms={args.sample_ms}",
           subprocess.check_output(["rustc", "-Vv"], text=True)]
    if Path("/proc/cpuinfo").exists():
        env.extend(next(line for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")) for _ in [0])
    if Path("/sys/fs/cgroup/cpu.max").exists():
        env.append("cpu.max: " + Path("/sys/fs/cgroup/cpu.max").read_text().strip())
    (out / "environment.txt").write_text("\n".join(env) + "\n")
    workload = {}
    for case, state in CASES:
        pilots = [measure(label, case, state, 2048)[1] / 2048 for label in LABELS]
        workload[(case,state)] = max(512, round(args.sample_ms * 1_000_000 / min(pilots)))
    results = {}
    with (out / "raw.csv").open("w", newline="") as file:
        writer = csv.writer(file)
        writer.writerow(["round", "order_in_pair", "case", "state", "revision", "ticks", "packets", "ns", "ns_per_tick"])
        for n in range(args.rounds):
            # AB, BA, AB, BA for each cell; shuffle cells across rounds with a repeatable seed.
            order = CASES.copy()
            random.Random(17838+n).shuffle(order)
            for case,state in order:
                labels = LABELS if (n + CASES.index((case,state))) % 2 == 0 else LABELS[::-1]
                counts = []
                for sequence, label in enumerate(labels, 1):
                    ticks = workload[(case,state)]
                    packets, ns = measure(label, case, state, ticks)
                    counts.append(packets)
                    results.setdefault((case,state,label), []).append(ns / ticks)
                    writer.writerow([n+1,sequence,case,state,label,ticks,packets,ns,f"{ns/ticks:.5f}"])
                assert counts[0] == counts[1]
                file.flush()
    with (out / "summary.csv").open("w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["case","state","fantuz_median_ns","adamsardo_median_ns","median_paired_speed_ratio",
                         "paired_ratio_min","paired_ratio_max"])
        for c,s in CASES:
            fork, up = (results[c,s,label] for label in LABELS)
            ratios = [b/a for a,b in zip(fork,up)]
            row = [c,s,*[round(x,3) for x in (statistics.median(fork), statistics.median(up),
                    statistics.median(ratios),min(ratios),max(ratios))]]
            writer.writerow(row)
            print(*row)


if __name__ == "__main__":
    main()
