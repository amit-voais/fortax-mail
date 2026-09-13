#!/usr/bin/env python3
"""Sample a launched Linux process without adding a sampler to Flectar's RAM."""
import argparse
import csv
import os
from pathlib import Path
import subprocess
import sys
import time


def sample(pid):
    root = Path(f"/proc/{pid}")
    status = {}
    for line in (root / "status").read_text().splitlines():
        key, _, value = line.partition(":")
        if key in ("VmRSS", "VmHWM"):
            status[key] = int(value.split()[0])
    # The command name can contain spaces and parentheses.
    fields = (root / "stat").read_text().rsplit(")", 1)[1].split()
    ticks = int(fields[11]) + int(fields[12])
    return status.get("VmRSS", 0), status.get("VmHWM", 0), ticks


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", choices=("cpu", "gpu"), default="cpu")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--interval", type=float, default=0.1)
    parser.add_argument("--disable-sync", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command or not sys.platform.startswith("linux") or not 0.05 <= args.interval <= 10:
        parser.error("Linux, a command after --, and an interval of 0.05–10 seconds are required")
    env = os.environ.copy()
    env["FLECTAR_RENDERER"] = args.renderer
    env["FLECTAR_RENDER_TIMINGS"] = "1"
    env.pop("FLECTAR_GPU_FALLBACK", None)
    if args.disable_sync:
        env["FLECTAR_BENCHMARK_DISABLE_SYNC"] = "1"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    ticks_per_second = os.sysconf("SC_CLK_TCK")
    started = time.monotonic()
    previous = None
    peak = 0
    with args.output.open("w", newline="") as output:
        writer = csv.writer(output)
        writer.writerow(["elapsed_seconds", "requested_renderer", "rss_kib", "process_peak_rss_kib", "cpu_percent"])
        process = subprocess.Popen(command, env=env)
        print(f"Sampling PID {process.pid}; check FLECTAR_RENDERER logs for the actual active mode.", file=sys.stderr)
        try:
            while process.poll() is None:
                now = time.monotonic()
                try:
                    rss, high_water, ticks = sample(process.pid)
                except (FileNotFoundError, ProcessLookupError):
                    break
                cpu = 0 if previous is None else 100 * (ticks - previous[1]) / ticks_per_second / (now - previous[0])
                previous = (now, ticks)
                peak = max(peak, high_water)
                writer.writerow([f"{now - started:.3f}", args.renderer, rss, high_water, f"{cpu:.2f}"])
                output.flush()
                time.sleep(args.interval)
        except KeyboardInterrupt:
            process.terminate()
        finally:
            code = process.wait()
    print(f"Peak process RSS: {peak} KiB; samples: {args.output}", file=sys.stderr)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
