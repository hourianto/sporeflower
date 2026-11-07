#!/usr/bin/env python3
"""Check real J2ME corpus sources or native project workflows; never run applications."""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import time
import tomllib
import zipfile
from pathlib import Path


def run(command: list[str], log: Path, timeout: float) -> tuple[int, float]:
    started = time.monotonic()
    with log.open("wb") as stream:
        try:
            result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT, timeout=timeout, check=False)
            return result.returncode, time.monotonic() - started
        except subprocess.TimeoutExpired:
            stream.write(b"\nCORPUS CHECK TIMEOUT\n")
            return 124, time.monotonic() - started


def artifact_paths(original: Path, classes: Path, mapping: Path, classpath: list[Path]) -> dict:
    """Retain the exact comparison inputs, independently of the run's layout."""
    return {"original": str(original.resolve()), "classes": str(classes.resolve()),
            "mapping": str(mapping.resolve()), "classpath": [str(path.resolve()) for path in classpath]}


def toml_document(config: dict, prefix: str = "") -> str:
    """Write the scalar/array/table configuration accepted by project commands."""
    lines = [f"[{prefix}]"] if prefix else []
    for key, value in config.items():
        if not isinstance(value, dict):
            lines.append(f"{json.dumps(key)} = {json.dumps(value, ensure_ascii=False)}")
    for key, value in config.items():
        if isinstance(value, dict):
            section = f"{prefix}." if prefix else ""
            lines.extend(["", toml_document(value, section + json.dumps(key))])
    return "\n".join(lines) + "\n"


def class_identity(data: bytes) -> tuple[str, int]:
    """Parse the class envelope and constant pool without loading application code."""
    position = 0

    def take(size: int) -> bytes:
        nonlocal position
        end = position + size
        if end > len(data):
            raise ValueError("truncated class file")
        value = data[position:end]
        position = end
        return value

    def u2() -> int:
        return int.from_bytes(take(2), "big")

    if take(4) != b"\xca\xfe\xba\xbe":
        raise ValueError("invalid class magic")
    take(2)
    major, count = u2(), u2()
    pool: dict[int, bytes | int] = {}
    index = 1
    sizes = {3: 4, 4: 4, 5: 8, 6: 8, 8: 2, 9: 4, 10: 4, 11: 4,
             12: 4, 15: 3, 16: 2, 17: 4, 18: 4, 19: 2, 20: 2}
    while index < count:
        tag = take(1)[0]
        if tag == 1:
            pool[index] = take(u2())
        elif tag == 7:
            pool[index] = u2()
        elif tag in sizes:
            take(sizes[tag])
        else:
            raise ValueError(f"unknown constant pool tag {tag}")
        index += 2 if tag in (5, 6) else 1
    if index != count:
        raise ValueError("invalid constant pool slot count")
    take(2)
    name = pool[pool[u2()]]
    if not isinstance(name, bytes):
        raise ValueError("invalid class identity")
    take(2)
    take(2 * u2())

    def attributes() -> None:
        for _ in range(u2()):
            if not isinstance(pool.get(u2()), bytes):
                raise ValueError("invalid attribute name")
            take(int.from_bytes(take(4), "big"))

    for _ in range(2):  # fields, methods
        for _ in range(u2()):
            take(2)
            if not all(isinstance(pool.get(u2()), bytes) for _ in range(2)):
                raise ValueError("invalid member name or descriptor")
            attributes()
    attributes()
    if position != len(data):
        raise ValueError("trailing class file bytes")
    # Class names cannot contain modified-UTF8 NUL; surrogate pairs encode names
    # outside the BMP in JVM modified UTF8.
    name = name.decode("utf-8", errors="surrogatepass")
    name = name.encode("utf-16", errors="surrogatepass").decode("utf-16")
    return name, major


def archive_check(original: Path, mapped: Path, tiny: Path) -> dict:
    """Check identities/envelopes, entry points and resources; not JVM verification."""
    names = dict(line.split("\t")[1:3] for line in tiny.read_text().splitlines()
                 if line.startswith("c\t"))
    with zipfile.ZipFile(original) as source, zipfile.ZipFile(mapped) as target:
        before = {entry.filename: source.read(entry) for entry in source.infolist() if not entry.is_dir()}
        after = {entry.filename: target.read(entry) for entry in target.infolist() if not entry.is_dir()}
        if len(after) != sum(not entry.is_dir() for entry in target.infolist()):
            raise ValueError("duplicate mapped archive entries")
    expected, classes, resources, entry_points = {}, {}, {}, []

    def manifest_points(data: bytes) -> dict[str, str]:
        points = {}
        lines = re.sub(rb"\r?\n ", b"", data).decode("utf-8", errors="replace").splitlines()
        for line in lines:
            key, _, value = line.partition(":")
            key = key.lower()
            if key in ("main-class", "midlet-class") or re.fullmatch(r"midlet-\d+", key):
                points[key] = value.rsplit(",", 1)[-1].strip().replace(".", "/")
        return points

    for path, data in before.items():
        if path.endswith(".class") and data.startswith(b"\xca\xfe\xba\xbe"):
            name, major = class_identity(data)
            expected[names[name]] = major
        elif path.upper() == "META-INF/MANIFEST.MF":
            if path not in after:
                raise ValueError("mapped manifest missing")
            points = {key: names.get(name, name) for key, name in manifest_points(data).items()}
            if manifest_points(after[path]) != points:
                raise ValueError("manifest entry points do not match mapping.tiny")
        elif re.fullmatch(r"META-INF/(SIG-[^/]*|[^/]*\.(SF|RSA|DSA|EC))", path.upper()):
            if path in after:
                raise ValueError(f"invalidated signature retained: {path}")
        else:
            if after.get(path) != data:
                raise ValueError(f"resource changed or missing: {path}")
            resources[path] = data
    preserved = len(resources)
    moves = {(old.rpartition("/")[0], new.rpartition("/")[0]) for old, new in names.items()
             if old.rpartition("/")[0] != new.rpartition("/")[0]}
    for old, new in moves:
        prefix = old + "/" if old else ""
        for path, data in list(resources.items()):
            if path in before and not path.upper().startswith("META-INF/") and path.startswith(prefix):
                alias = (new + "/" if new else "") + path[len(prefix):]
                resources[alias] = data
                if after.get(alias) != data:
                    raise ValueError(f"resource alias changed or missing: {alias}")
    for path, data in after.items():
        if any(part in ("", ".", "..") for part in path.split("/")) or any(c in path for c in "\\\x00:"):
            raise ValueError(f"unsafe mapped entry: {path!r}")
        if path.endswith(".class") and data.startswith(b"\xca\xfe\xba\xbe"):
            name, major = class_identity(data)
            if path != name + ".class" or name in classes:
                raise ValueError(f"class identity/path mismatch or duplicate: {path}")
            classes[name] = major
        elif path.upper() == "META-INF/MANIFEST.MF":
            for name in manifest_points(data).values():
                if name not in expected:
                    raise ValueError(f"missing manifest entry point: {name}")
                entry_points.append(name)
        elif resources.get(path) != data:
            raise ValueError(f"unexpected mapped resource: {path}")
    if classes != expected:
        raise ValueError("mapped class identities or bytecode versions differ from mapping.tiny")
    return {"success": True, "classes": len(classes), "resources_preserved": preserved,
            "resource_aliases": len(resources) - preserved, "entry_points": entry_points,
            "class_versions": sorted(set(classes.values()))}


def workflow(args, project: str, project_root: Path, configuration: dict, jar: Path, work: Path) -> dict:
    """Copy authored inputs only; all native command writes stay in this work project."""
    maps = sorted(path for path in (project_root / "mappings").glob("*.map") if path.is_file())
    classpath = [(project_root / path).resolve() for path in configuration.get("classpath", [])]
    classpath.extend(path.resolve() for path in args.classpath)
    for path in classpath:
        if not path.exists():
            raise ValueError(f"missing classpath: {path}")
    inputs = [project_root / "j2me.toml", jar, *maps, *classpath]
    snapshots = {path: path.stat() for path in inputs}
    configuration["jar"] = jar.name
    configuration["classpath"] = [str(path) for path in classpath]
    for key in ("jar", "compiler_jar", "java", "java_bin", "javac", "javac_bin"):
        value = configuration.get("compiler", {}).get(key)
        if value and (key in ("jar", "compiler_jar") or len(Path(value).parts) > 1):
            configuration["compiler"][key] = str((project_root / value).resolve())
    shutil.copy2(jar, work / jar.name)
    for source in maps:
        target = work / source.relative_to(project_root)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
    (work / "j2me.toml").write_text(toml_document(configuration))
    command = [str(args.binary.resolve()), "remap", "--project", str(work), "--threads", str(args.threads)]
    status, duration = run(command, work / "remap.log", args.timeout)
    result = {"project": project, "work": str(work), "workflow": True, "mappings": len(maps),
              "classpath": [str(path) for path in classpath], "remap_exit": status, "remap_seconds": duration}
    summary = work / "out/decompile.json"
    if summary.exists():
        result.update(json.loads(summary.read_text()))
    result["reconstruction_success"] = status == 0 and all(result.get(key) == 0 for key in
        ("failed_classes", "failed_methods", "fallback_methods"))
    mapped = work / "out" / (jar.stem + "-mapped.jar")
    if mapped.exists():
        try:
            result["archive_check"] = archive_check(jar, mapped, work / "out/mapping.tiny")
        except (OSError, ValueError, KeyError, zipfile.BadZipFile) as error:
            result["archive_check"] = {"success": False, "error": str(error)}
    # Even failed reconstruction can produce compilable placeholders: measure both.
    command = [str(args.binary.resolve()), "compile-stubs", "--project", str(work), "--maxerrs", "10000"]
    if args.legacy_javac:
        command += ["--compiler", "legacy", "--compiler-jar", str(args.legacy_javac.resolve())]
        command += ["--java-bin", args.java]
    elif args.javac:
        command += ["--javac-bin", args.javac]
    status, duration = run(command, work / "compile.log", args.timeout)
    result.update(compile_exit=status, compile_seconds=duration, compilation_success=status == 0)
    compiled = work / "out/compile-stubs/result.json"
    if compiled.exists():
        result["compiler"] = json.loads(compiled.read_text())
        result["artifacts"] = artifact_paths(
            work / jar.name, work / "out/compile-stubs/classes", work / "out/mapping.tiny",
            [Path(path) for path in result["compiler"]["classpath"]])
    if status == 0:
        classes = [class_identity(path.read_bytes()) for path in (work / "out/compile-stubs/classes").rglob("*.class")]
        result["compiled_classes"] = len(classes)
        result["compiled_class_versions"] = sorted({major for _, major in classes})
    log = (work / "compile.log").read_text(errors="replace")
    totals = re.findall(r"(?m)^(\d+) errors?$", log)
    result["compiler_errors"] = int(totals[-1]) if totals else sum(": error:" in line for line in log.splitlines())
    changed = [str(path) for path, old in snapshots.items()
               if (path.stat().st_size, path.stat().st_mtime_ns) != (old.st_size, old.st_mtime_ns)]
    result["inputs_unchanged"] = not changed
    if changed:
        result["changed_inputs"] = changed
    print(f"{project}: remap={result['remap_exit']}, compile={status}, "
          f"reconstruction={result['reconstruction_success']}, archive={result.get('archive_check', {}).get('success', False)}", flush=True)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("projects", nargs="*", help="Direct project directory names; otherwise select --all or --mapped")
    parser.add_argument("--root", type=Path, default=Path(os.environ.get("SPOREFLOWER_CORPUS", ".")),
                        help="Corpus root (default: SPOREFLOWER_CORPUS or current directory)")
    parser.add_argument("--binary", type=Path, default=Path("target/release/sporeflower"))
    parser.add_argument("--output", type=Path, default=Path("work/corpus"))
    java_bin = Path(os.environ["JAVA_HOME"]) / "bin" if os.environ.get("JAVA_HOME") else None
    parser.add_argument("--javac", default=os.environ.get("JAVAC") or (str(java_bin / "javac") if java_bin else None))
    parser.add_argument("--java", default=os.environ.get("JAVA") or (str(java_bin / "java") if java_bin else "java"),
                        help="Java runtime for --legacy-javac (default: JAVA, JAVA_HOME/bin/java, or PATH)")
    parser.add_argument("--legacy-javac", type=Path, help="Path to the legacy javac JAR for Java 1.3 and class version 45 output")
    parser.add_argument("--workflow", action="store_true", help="Run native remap + compile-stubs in isolated copies of authored project inputs")
    parser.add_argument("--mapped", action="store_true", help="Select every direct project containing authored mappings (requires --workflow)")
    parser.add_argument("--classpath", type=Path, action="append", default=[], help="Additional workflow classpath; repeat as needed")
    parser.add_argument("--all", action="store_true", help="Check every direct project containing j2me.toml")
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--threads", type=int, default=4)
    args = parser.parse_args()
    javac = args.javac or shutil.which("javac")
    if not (args.workflow or args.legacy_javac) and (not javac or not shutil.which(javac)):
        parser.error("javac is required; use JDK 17 via --javac, JAVAC, JAVA_HOME, or PATH")
    if min(args.timeout, args.threads) <= 0:
        parser.error("timeout and threads must be positive")
    if not args.root.is_dir():
        parser.error(f"corpus root is not a directory: {args.root}")
    if (args.mapped or args.classpath) and not args.workflow:
        parser.error("--mapped and --classpath require --workflow")
    projects = args.projects
    if args.all or args.mapped:
        if args.projects or (args.all and args.mapped):
            parser.error("choose explicit projects, --all, or --mapped")
        projects = sorted(path.parent.name for path in args.root.glob("*/j2me.toml"))
        if args.mapped:
            projects = [name for name in projects
                        if any(path.is_file() for path in (args.root / name / "mappings").glob("*.map"))]
    if not projects:
        parser.error("no projects selected; name project directories or use --all (or --workflow --mapped)")
    if len(set(projects)) != len(projects):
        parser.error("project names must be unique")
    output = args.output.resolve()
    args.root = args.root.resolve()
    if args.workflow and (output.is_relative_to(args.root) or args.root.is_relative_to(output)):
        parser.error("workflow output must be separate from the original corpus root")
    if any(Path(name).name != name or name in (".", "..") for name in projects):
        parser.error("projects must be direct project directory names")
    output.mkdir(parents=True, exist_ok=True)
    results = []
    for project in projects:
        project_root = args.root / project
        config = project_root / "j2me.toml"
        configuration = tomllib.loads(config.read_text()) if config.exists() else {}
        jar = project_root / configuration.get("jar", "game.jar")
        if not jar.exists():
            print(f"{project}: missing {jar}", flush=True)
            results.append({"project": project, "missing": str(jar)})
            continue
        # A new directory per run prevents stale source/classes from concealing failure.
        work = output / project / str(time.time_ns())
        work.mkdir(parents=True)
        if args.workflow:
            try:
                result = workflow(args, project, project_root, configuration, jar, work)
            except (OSError, ValueError, zipfile.BadZipFile) as error:
                result = {"project": project, "work": str(work), "workflow": True, "error": str(error)}
                print(f"{project}: {error}", flush=True)
            results.append(result)
            (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
            continue
        source, stubs, summary = work / "source", work / "api.jar", work / "decompile.json"
        status, duration = run([str(args.binary.resolve()), str(jar), str(source), "--stubs", str(stubs), "--json", str(summary), "--threads", str(args.threads)], work / "decompile.log", args.timeout)
        result = {"project": project, "work": str(work), "decompile_exit": status, "decompile_seconds": duration}
        if summary.exists():
            result.update(json.loads(summary.read_text()))
        java = sorted(source.rglob("*.java")) if source.exists() else []
        if java and stubs.exists():
            argfile = work / "sources.txt"
            argfile.write_text("\n".join('"' + str(p).replace("\\", "\\\\").replace('"', '\\"') + '"' for p in java) + "\n")
            classes = work / "classes"
            classes.mkdir()
            if args.legacy_javac:
                compiler = [args.java, "-cp", str(args.legacy_javac.resolve()), "j2me.thirdparty.legacyjavac.Main", "-source", "1.3", "-target", "1.1", "-bootclasspath", str(stubs)]
            else:
                compiler = [javac, "-source", "7", "-target", "7", "-Xlint:-options"]
            compiler += ["-encoding", "UTF-8", "-g:lines,source", "-Xmaxerrs", "10000", "-classpath", str(stubs), "-d", str(classes), "@" + str(argfile)]
            compile_status, compile_duration = run(compiler, work / "compile.log", args.timeout)
            result.update(compile_exit=compile_status, compile_seconds=compile_duration)
            result["artifacts"] = artifact_paths(jar, classes, source / "mapping.tiny", [stubs])
            # javac's diagnostic summary is retained verbatim in compile.log.
            log = (work / "compile.log").read_text(errors="replace")
            totals = re.findall(r"(?m)^(\d+) errors?$", log)
            result["compiler_errors"] = int(totals[-1]) if totals else sum(": error:" in line for line in log.splitlines())
        print(f"{project}: decompile={status}, compile={result.get('compile_exit', 'skipped')}, errors={result.get('compiler_errors', '-')}, failed_methods={result.get('failed_methods', '-')}, fallback_methods={result.get('fallback_methods', '-')}", flush=True)
        results.append(result)
        (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    if args.workflow:
        return 0 if results and all(r.get("reconstruction_success") and r.get("compilation_success")
            and r.get("archive_check", {}).get("success") and r.get("inputs_unchanged") for r in results) else 1
    return 0 if all(r.get("decompile_exit") == 0 and r.get("compile_exit") == 0 for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
