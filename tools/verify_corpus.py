#!/usr/bin/env python3
"""Symbolically compare retained corpus rebuilds; never execute corpus application code.

Inputs are tools/corpus.py results.json files. MATCH is conditional on the verifier's
model and the recorded API declarations; successful compilation is not a proof.
Exit 0 means all selected method and artifact checks matched under that model;
1 means completed checks have findings or limits; 2 means input/runner failures
or incomplete comparisons. Inspect the retained method-level findings.
"""
from __future__ import annotations

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
import json
import os
from pathlib import Path
import signal
import shutil
import subprocess
import time


def compare(row: dict, args: argparse.Namespace, output: Path) -> dict:
    project = row["project"]
    work = Path(row["work"])
    result = {
        "project": project,
        "manifest": row["manifest"],
        "input": row.get("input"),
        "work": str(work),
        "decompile_exit": row.get("decompile_exit"),
        "compile_exit": row.get("compile_exit"),
        "failed_methods": row.get("failed_methods"),
        "decompile_diagnostics": row.get("diagnostics", []),
    }
    if row.get("compile_exit") != 0:
        return {**result, "state": "NO_REBUILD"}
    artifacts = row.get("artifacts")
    if artifacts is None:
        # Older retained runs predate explicit artifact paths. Normalize their
        # two historical layouts here; new producers own their actual paths.
        if not row.get("input"):
            return {**result, "state": "NO_REBUILD"}
        workflow = row.get("workflow", False)
        artifacts = {
            "original": row["input"],
            "classes": str(work / ("out/compile-stubs/classes" if workflow else "classes")),
            "mapping": str(work / ("out/mapping.tiny" if workflow else "source/mapping.tiny")),
            "classpath": row.get("compiler", {}).get("classpath", [
                str(work / ("out/api-stubs.jar" if workflow else "api.jar"))]),
        }
    original, classes, mapping = [Path(artifacts[key]) for key in ("original", "classes", "mapping")]
    classpath = [Path(path) for path in artifacts["classpath"]]
    result["inputs"] = artifacts
    missing = [str(path) for path in [original, classes, mapping, *classpath] if not path.exists()]
    if missing:
        return {**result, "state": "INPUT_ERROR", "missing": missing}
    report = output / f"{project}.json"
    command = [
        str(args.verifier), str(original), str(classes),
        "--mapping", str(mapping), "--classpath", os.pathsep.join(map(str, classpath)),
        "--json", str(report), "--timeout", str(args.timeout),
        "--method-ms", str(args.method_ms), "--jobs", str(args.method_jobs),
        "--max-states", str(args.max_states),
    ]
    if args.details:
        command += ["--report-dir", str(output / project)]
    if args.method:
        command += ["--method", args.method]
    if args.no_smt:
        command += ["--no-smt"]
    result["command"] = command
    started = time.monotonic()
    environment = os.environ.copy()
    environment.pop("JAVA_OPTS", None)
    environment["J2ME_VERIFIER_OPTS"] = f"-Xmx{args.heap_mb}m -XX:ActiveProcessorCount={args.method_jobs}"
    with (output / f"{project}.log").open("wb") as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT,
                                   env=environment, start_new_session=True)
        try:
            exit_code = process.wait(timeout=args.timeout + 30)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            exit_code = 124
    result.update(exit_code=exit_code, seconds=round(time.monotonic() - started, 3))
    if not report.is_file():
        return {**result, "state": "NO_REPORT"}
    data = json.loads(report.read_text())
    methods = data["methods"]
    result.update(
        state="COMPLETE" if exit_code in (0, 1, 2) and not data.get("runErrors") and not data.get("inputErrors") else "INCOMPLETE",
        verifier=data.get("verifier"),
        counts=data.get("counts", {}),
        match_diagnostics=dict(Counter(m["diagnostic"] for m in methods if m["status"] == "MATCH")),
        relational_models=dict(Counter(m.get("model", "ordered") for m in methods
                                       if m["status"] == "MATCH" and m["diagnostic"] == "RELATIONAL_MATCH")),
        findings=dict(Counter(m["diagnostic"] for m in methods if m["status"] != "MATCH")),
        input_errors=data.get("inputErrors", []),
        run_errors=data.get("runErrors", []),
        artifacts=data.get("artifactDifferences", []),
        scope=data.get("scope"),
    )
    return result


def summarize(rows: list[dict], output: Path) -> None:
    counts = Counter()
    findings = Counter()
    matches = Counter()
    models = Counter()
    for row in rows:
        counts.update(row.get("counts", {}))
        findings.update(row.get("findings", {}))
        matches.update(row.get("match_diagnostics", {}))
        models.update(row.get("relational_models", {}))
    summary = {
        "projects": len(rows),
        "states": dict(Counter(row["state"] for row in rows)),
        "counts": dict(counts),
        "match_diagnostics": dict(matches),
        "relational_models": dict(models),
        "findings": dict(findings),
        "projects_with_artifact_differences": sum(bool(row.get("artifacts")) for row in rows),
        "scope": "Code-only comparisons against retained legacy rebuilds and generated compilation API declarations; no application code executed. Model-specific MATCH is not whole-program or resource equivalence.",
        "rows": sorted(rows, key=lambda row: row["project"]),
    }
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifests", nargs="+", type=Path)
    parser.add_argument("--verifier", default=os.environ.get("J2ME_VERIFIER", "j2me-verifier"),
                        help="External verifier executable (default: J2ME_VERIFIER or j2me-verifier on PATH; not bundled)")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--project", action="append", default=[])
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--method-jobs", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--method-ms", type=int, default=2500)
    parser.add_argument("--max-states", type=int, default=30000)
    parser.add_argument("--heap-mb", type=int, default=1536)
    parser.add_argument("--details", action="store_true")
    parser.add_argument("--method")
    parser.add_argument("--no-smt", action="store_true")
    args = parser.parse_args()
    verifier = shutil.which(args.verifier)
    if not verifier:
        parser.error("external j2me-verifier is not bundled; set --verifier or J2ME_VERIFIER to an executable, or put it on PATH")
    args.verifier = Path(verifier).resolve()
    if min(args.jobs, args.method_jobs, args.timeout, args.method_ms, args.max_states, args.heap_mb) <= 0:
        parser.error("budgets and worker counts must be positive")
    rows = []
    try:
        for manifest in args.manifests:
            entries = json.loads(manifest.read_text())
            if not isinstance(entries, list) or not all(
                isinstance(row, dict) and isinstance(row.get("project"), str) and row["project"]
                for row in entries
            ):
                parser.error(f"expected a list of project rows in {manifest}")
            rows += [{**row, "manifest": str(manifest.resolve())} for row in entries]
    except (OSError, ValueError) as error:
        parser.error(str(error))
    if args.project:
        requested = set(args.project)
        rows = [row for row in rows if row["project"] in requested]
        missing = requested - {row["project"] for row in rows}
        if missing:
            parser.error(f"unknown projects: {sorted(missing)}")
    projects = [row["project"] for row in rows]
    if not projects:
        parser.error("no projects selected")
    if len(set(projects)) != len(projects):
        parser.error("project names must be unique across the selected manifests")
    if any(Path(name).name != name or name in (".", "..") for name in projects):
        parser.error("project names must be direct directory names")
    output = args.out.resolve()
    try:
        output.mkdir(parents=True, exist_ok=False)
    except OSError as error:
        parser.error(str(error))
    completed = []
    with ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures = {executor.submit(compare, row, args, output): row["project"] for row in rows}
        for future in as_completed(futures):
            try:
                result = future.result()
            except Exception as error:
                result = {"project": futures[future], "state": "RUNNER_ERROR", "error": repr(error)}
            completed.append(result)
            summarize(completed, output)
            print(f"{result['project']}: {result['state']} {result.get('counts', {})} ({result.get('seconds', 0)}s)", flush=True)
    if any(row["state"] != "COMPLETE" for row in completed):
        return 2
    return int(any(row.get("artifacts") or any(
        count for status, count in row.get("counts", {}).items() if status != "MATCH"
    ) for row in completed))


if __name__ == "__main__":
    raise SystemExit(main())
