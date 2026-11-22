#!/usr/bin/env python3
"""
Benchmark comparison script: ZeroRec vs. FFmpeg
Measures CPU utilization, memory (RSS), and frame pacing across identical video workloads on macOS.
"""

import os
import sys
import time
import subprocess
import json
import statistics
import tempfile
from pathlib import Path

ZEROREC = Path(__file__).resolve().parent.parent / "target" / "release" / "zerorec"
FFMPEG = "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg"
FFPROBE = "/opt/homebrew/opt/ffmpeg-full/bin/ffprobe"

def sample_process(pid, stop_event, interval_sec=0.1):
    cpu_samples = []
    rss_samples = []
    history = []
    start_time = time.perf_counter()
    
    while not stop_event["stop"]:
        try:
            # Query the target process and any child processes
            pids = [str(pid)]
            try:
                children = subprocess.check_output(["pgrep", "-P", str(pid)], stderr=subprocess.DEVNULL).decode().split()
                pids.extend(children)
            except Exception:
                pass

            out = subprocess.check_output(["ps", "-o", "%cpu,rss", "-p", ",".join(pids)], stderr=subprocess.DEVNULL).decode()
            lines = out.strip().split("\n")
            total_cpu = 0.0
            total_rss = 0.0
            for line in lines[1:]:
                parts = line.strip().split()
                if len(parts) >= 2:
                    total_cpu += float(parts[0])
                    total_rss += float(parts[1]) / 1024.0  # KB to MB
            
            if len(lines) > 1:
                cpu_samples.append(total_cpu)
                rss_samples.append(total_rss)
                history.append({
                    "t": round(time.perf_counter() - start_time, 2),
                    "cpu": round(total_cpu, 1),
                    "rss": round(total_rss, 1)
                })
        except Exception:
            break
        time.sleep(interval_sec)
    return cpu_samples, rss_samples, history

def analyze_file(filepath):
    """Inspect timestamps and frame count using ffprobe."""
    if not os.path.exists(filepath) or os.path.getsize(filepath) == 0:
        return {"frames": 0, "size_mb": 0.0, "duration": 0.0, "status": "Failed/Empty"}
    
    size_mb = os.path.getsize(filepath) / (1024 * 1024)
    cmd = [
        FFPROBE, "-v", "error", "-select_streams", "v:0",
        "-show_entries", "stream=nb_frames,duration,avg_frame_rate",
        "-of", "json", filepath
    ]
    try:
        res = subprocess.check_output(cmd).decode()
        data = json.loads(res)
        stream = data.get("streams", [{}])[0]
        frames = int(stream.get("nb_frames") or 0)
        duration = float(stream.get("duration") or 0.0)
        return {
            "frames": frames,
            "size_mb": size_mb,
            "duration": duration,
            "status": "OK"
        }
    except Exception as e:
        return {"frames": 0, "size_mb": size_mb, "duration": 0.0, "status": f"Probe error: {e}"}

def run_test(name, cmd, outfile, duration_sec, extra_env=None):
    import threading
    stop_event = {"stop": False}
    result_holder = {}

    env = dict(os.environ)
    env["REC_NO_DISCLAIM"] = "1"
    if extra_env:
        env.update(extra_env)

    start_t = time.perf_counter()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)

    def monitor():
        cpu, rss, hist = sample_process(p.pid, stop_event)
        result_holder["cpu"] = cpu
        result_holder["rss"] = rss
        result_holder["history"] = hist

    t = threading.Thread(target=monitor)
    t.start()

    try:
        stdout, stderr = p.communicate(timeout=duration_sec + 10)
    except subprocess.TimeoutExpired:
        p.kill()
        stdout, stderr = p.communicate()
    finally:
        stop_event["stop"] = True
        t.join()

    elapsed = time.perf_counter() - start_t
    cpus = result_holder.get("cpu", [])
    rsss = result_holder.get("rss", [])
    history = result_holder.get("history", [])

    mean_cpu = statistics.mean(cpus) if cpus else 0.0
    peak_cpu = max(cpus) if cpus else 0.0
    max_rss = max(rsss) if rsss else 0.0

    meta = analyze_file(outfile)
    return {
        "name": name,
        "mean_cpu": mean_cpu,
        "peak_cpu": peak_cpu,
        "max_rss": max_rss,
        "elapsed": elapsed,
        "file_mb": meta["size_mb"],
        "frames": meta["frames"],
        "status": meta["status"],
        "history": history
    }

def main():
    print("=" * 70)
    print("BENCHMARK: ZeroRec vs. FFmpeg on macOS (Apple Silicon)")
    print("=" * 70)

    duration = 10
    tmpdir = tempfile.gettempdir()

    tests = [
        {
            "category": "1080p60 H.264 (Test Pattern)",
            "runs": [
                {
                    "name": "ZeroRec (1080p60 H.264)",
                    "outfile": f"{tmpdir}/zerorec_1080p.mp4",
                    "cmd": [
                        str(ZEROREC), "record", "--test-pattern",
                        "-r", "1080p", "--fps", "60",
                        "--codec", "h264", "--no-mic",
                        "--duration", str(duration),
                        "-o", f"{tmpdir}/zerorec_1080p.mp4",
                        "--quiet"
                    ]
                },
                {
                    "name": "FFmpeg (1080p60 H.264)",
                    "outfile": f"{tmpdir}/ffmpeg_1080p.mp4",
                    "cmd": [
                        FFMPEG, "-y", "-re", "-f", "lavfi",
                        "-i", "testsrc=size=1920x1080:rate=60",
                        "-t", str(duration),
                        "-c:v", "h264_videotoolbox",
                        "-b:v", "15M",
                        "-pix_fmt", "nv12",
                        f"{tmpdir}/ffmpeg_1080p.mp4"
                    ]
                }
            ]
        },
        {
            "category": "4K60 HEVC (Test Pattern)",
            "runs": [
                {
                    "name": "ZeroRec (4K60 HEVC)",
                    "outfile": f"{tmpdir}/zerorec_4k.mp4",
                    "cmd": [
                        str(ZEROREC), "record", "--test-pattern",
                        "-r", "4k", "--fps", "60",
                        "--codec", "hevc", "--no-mic",
                        "--duration", str(duration),
                        "-o", f"{tmpdir}/zerorec_4k.mp4",
                        "--quiet"
                    ]
                },
                {
                    "name": "FFmpeg (4K60 HEVC)",
                    "outfile": f"{tmpdir}/ffmpeg_4k.mp4",
                    "cmd": [
                        FFMPEG, "-y", "-re", "-f", "lavfi",
                        "-i", "testsrc=size=3840x2160:rate=60",
                        "-t", str(duration),
                        "-c:v", "hevc_videotoolbox",
                        "-b:v", "40M",
                        "-pix_fmt", "nv12",
                        f"{tmpdir}/ffmpeg_4k.mp4"
                    ]
                }
            ]
        }
    ]

    results = []

    for suite in tests:
        print(f"\n--- Running: {suite['category']} ---")
        for r in suite["runs"]:
            print(f"Running {r['name']} for {duration}s...")
            # Clean old file
            if os.path.exists(r["outfile"]):
                os.remove(r["outfile"])
            res = run_test(r["name"], r["cmd"], r["outfile"], duration)
            res["category"] = suite["category"]
            results.append(res)
            print(f"  -> Mean CPU: {res['mean_cpu']:.1f}%, Peak CPU: {res['peak_cpu']:.1f}%, Max RSS: {res['max_rss']:.1f} MB, Frames: {res['frames']}")

    print("\n" + "=" * 80)
    print("FINAL BENCHMARK RESULTS")
    print("=" * 80)
    print(f"{'Engine':<28} | {'Mean CPU':<10} | {'Peak CPU':<10} | {'Max RAM':<10} | {'Frames':<8} | {'File Size'}")
    print("-" * 80)
    for res in results:
        print(f"{res['name']:<28} | {res['mean_cpu']:>7.1f} % | {res['peak_cpu']:>7.1f} % | {res['max_rss']:>7.1f} MB | {res['frames']:>8} | {res['file_mb']:.1f} MB")
    print("=" * 80)

    # Save results json
    out_json = Path(__file__).resolve().parent.parent / "bench_results.json"
    with open(out_json, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nSaved raw benchmark data to {out_json}")

if __name__ == "__main__":
    main()
