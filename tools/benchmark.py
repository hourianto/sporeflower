#!/usr/bin/env python3
"""Compare fresh decompiler processes; never execute corpus application code."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import statistics
import subprocess
import threading
import time
import tomllib


def measure(command: list[str], stream, timeout: int) -> tuple[int, float]:
    """Block in waitpid, avoiding Popen.wait(timeout)'s polling latency floor."""
    started = time.perf_counter()
    process = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT,
                               start_new_session=os.name == "posix")
    expired = threading.Event()

    def terminate() -> None:
        if process.poll() is not None:
            return
        expired.set()
        try:
            if os.name == "posix":
                # The timer wraps /usr/bin/time, whose decompiler child must
                # also stop on timeout instead of contaminating later runs.
                os.killpg(process.pid, signal.SIGKILL)
            else:
                process.kill()
        except ProcessLookupError:
            pass

    timer = threading.Timer(timeout, terminate)
    timer.start()
    try:
        status = process.wait()
        elapsed = time.perf_counter() - started
    finally:
        timer.cancel()
        timer.join()
    return (124 if expired.is_set() else status), elapsed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("projects", nargs="+")
    parser.add_argument("--root", type=Path, default=Path(os.environ.get("SPOREFLOWER_CORPUS", ".")),
                        help="Corpus root (default: SPOREFLOWER_CORPUS or current directory)")
    parser.add_argument("--native", type=Path, default=Path("target/release/sporeflower"))
    parser.add_argument("--baseline-jar", type=Path, required=True,
                        help="Executable Sporeflower/Vineflower-compatible CLI JAR (not bundled)")
    java_bin = Path(os.environ["JAVA_HOME"]) / "bin/java" if os.environ.get("JAVA_HOME") else "java"
    parser.add_argument("--java", default=os.environ.get("JAVA", str(java_bin)),
                        help="Baseline runtime (default: JAVA, JAVA_HOME/bin/java, or PATH)")
    parser.add_argument("--output", type=Path, default=Path("work/benchmark"))
    parser.add_argument("--threads", type=int, default=2)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args()
    if min(args.threads, args.repetitions, args.timeout) <= 0:
        parser.error("threads, repetitions and timeout must be positive")
    if any(Path(name).name != name or name in (".", "..") for name in args.projects):
        parser.error("projects must be direct project directory names")
    if len(set(args.projects)) != len(args.projects):
        parser.error("project names must be unique")
    if platform.system() not in ("Darwin", "Linux") or not Path("/usr/bin/time").is_file():
        parser.error("benchmark requires macOS /usr/bin/time or GNU /usr/bin/time (install the time package on Linux)")
    if not shutil.which(args.java):
        parser.error("java is required; set --java, JAVA, JAVA_HOME, or PATH")
    if not args.baseline_jar.is_file():
        parser.error(f"baseline JAR does not exist: {args.baseline_jar}")
    args.root = args.root.resolve()
    if not args.root.is_dir():
        parser.error(f"corpus root is not a directory: {args.root}")
    output = args.output.resolve() / str(time.time_ns())
    output.mkdir(parents=True)
    timer = ["/usr/bin/time", "-l" if platform.system() == "Darwin" else "-v"]
    rows = []
    for project in args.projects:
        project_root = args.root / project
        config = tomllib.loads((project_root / "j2me.toml").read_text())
        jar = project_root / config.get("jar", "game.jar")
        for repetition in range(args.repetitions):
            # Alternate the order so background activity/cache warmth does not
            # consistently favor either implementation.
            engines = ["rust", "java"] if repetition % 2 == 0 else ["java", "rust"]
            for engine in engines:
                work = output / project / f"{engine}-{repetition + 1}"
                work.mkdir(parents=True)
                source, log = work / "source", work / "run.log"
                if engine == "rust":
                    command = [str(args.native.resolve()), str(jar), str(source),
                               "--threads", str(args.threads), "--no-mapping", "--quiet",
                               "--json", str(work / "decompile.json")]
                else:
                    command = [args.java, "-jar", str(args.baseline_jar.resolve()),
                               "--folder", "--silent", "--skip-extra-files=true",
                               f"--thread-count={args.threads}", str(jar), str(source)]
                with log.open("wb") as stream:
                    status, elapsed = measure(timer + command, stream, args.timeout)
                text = log.read_text(errors="replace")
                if platform.system() == "Darwin":
                    match = re.search(r"(\d+)\s+maximum resident set size", text)
                    peak = int(match[1]) if match else None
                else:
                    match = re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", text)
                    peak = int(match[1]) * 1024 if match else None
                row = dict(project=project, engine=engine, repetition=repetition + 1,
                           seconds=elapsed, peak_rss_bytes=peak, exit=status,
                           command=command, work=str(work))
                summary = work / "decompile.json"
                if summary.exists():
                    native = json.loads(summary.read_text())
                    row.update(methods=native["methods"], failed_methods=native["failed_methods"],
                               fallback_methods=native["fallback_methods"])
                rows.append(row)
                (output / "results.json").write_text(json.dumps(rows, indent=2) + "\n")
                print(f"{project} {engine} run{repetition+1}: {elapsed:.3f}s, "
                      f"RSS {peak / 1048576:.1f}MiB" if peak else f"{project} {engine}: RSS unavailable",
                      f"exit={status}", flush=True)
        for engine in ["rust", "java"]:
            measurements = [row for row in rows if row["project"] == project and row["engine"] == engine]
            print(f"{project} {engine} median: {statistics.median(row['seconds'] for row in measurements):.3f}s", flush=True)
    print(output / "results.json", flush=True)
    return 0 if all(row["exit"] == 0 for row in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
