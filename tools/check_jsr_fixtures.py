"""Compare a baseline and candidate decompiler on fixtures compiled by JDK 8 javac."""
import argparse
import json
import re
import subprocess
from pathlib import Path
from tempfile import TemporaryDirectory


def measure(binary, classes, output, preserve):
    report = output.with_suffix(".json")
    command = [binary, classes, output, "--json", report, "--no-mapping", "-q", "-j1"]
    if preserve:
        command.append("--preserve-bytecode")
    run = subprocess.run(command, capture_output=True, text=True, check=False)
    if run.returncode not in (0, 2) or not report.is_file():
        raise RuntimeError(f"{binary} failed on {classes}\n{run.stderr}")
    summary = json.loads(report.read_text(encoding="utf-8"))
    counts = {key: summary[key] for key in
              ("failed_classes", "failed_methods", "fallback_methods", "inspection_fallback_methods")}
    for source in output.rglob("*.java"):
        # Count declarations, so a label with several breaks still counts once.
        text = source.read_text(encoding="utf-8")
        name = source.relative_to(output)
        counts[f"{name} labels"] = len(re.findall(r"^\s*label\d+:", text, re.MULTILINE))
        counts[f"{name} catches"] = len(re.findall(r"^\s*\}\s*catch \(", text, re.MULTILINE))
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--javac", required=True, help="path to JDK 8 javac")
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", type=Path, default=Path("target/release/sporeflower"))
    parser.add_argument("fixtures", nargs="+", type=Path,
                        help="Java source file or directory of sources compiled together")
    args = parser.parse_args()
    regressions = 0
    for fixture in args.fixtures:
        sources = sorted(fixture.glob("*.java")) if fixture.is_dir() else [fixture]
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            classes = root / "classes"
            classes.mkdir()
            subprocess.run([args.javac, "-nowarn", "-encoding", "UTF-8", "-source", "1.4", "-target", "1.4", "-XDjsrlimit=0",
                            "-classpath", classes, "-d", classes, *sources], check=True)
            for mode, preserve in [("default", False), ("preserve", True)]:
                before = measure(args.baseline.resolve(), classes, root / f"before-{mode}", preserve)
                after = measure(args.candidate.resolve(), classes, root / f"after-{mode}", preserve)
                for key, count in after.items():
                    previous = before.get(key, 0)
                    if count > previous:
                        print(f"{fixture} ({mode}) {key}: {previous} -> {count}", flush=True)
                        regressions += 1
    print(f"{len(args.fixtures)} fixtures in both modes, {regressions} increases")
    return int(regressions != 0)


if __name__ == "__main__":
    raise SystemExit(main())
