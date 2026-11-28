"""Measure executable lines in the JMAP network and storage modules on loopback."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

import test_stalwart

ROOT = Path(__file__).resolve().parents[1]


def check_source_coverage(report, sources):
    # DA records identify a physical source line. Generic functions and nested
    # closures may also contribute separate line counts to LLVM's summary.
    measured = set()
    line_count = function_count = 0
    misses = []
    for record in report.split("end_of_record"):
        if not record.strip():
            continue
        rows = record.splitlines()
        path = Path(next(row[3:] for row in rows if row.startswith("SF:"))).resolve()
        measured.add(path)
        hits = [row[3:].split(",") for row in rows if row.startswith("DA:")]
        missed = [number for number, count, *_ in hits if int(count) == 0]
        functions = int(next(row[4:] for row in rows if row.startswith("FNF:")))
        called = int(next(row[4:] for row in rows if row.startswith("FNH:")))
        if missed or called != functions or not hits:
            misses.append(f"{path.relative_to(ROOT)}: missed lines {','.join(missed)}; "
                          f"executed {called}/{functions} functions")
        line_count += len(hits)
        function_count += functions
    if not sources or measured != set(sources):
        raise SystemExit("Coverage report differs from the JMAP production source inventory")
    if misses:
        raise SystemExit("\n".join(misses))
    print(f"Covered {line_count}/{line_count} executable source lines and "
          f"{function_count}/{function_count} functions in {len(measured)} JMAP modules.")


def coverage(temp):
    host = subprocess.check_output(["rustc", "-vV"], text=True).split("host: ")[1].splitlines()[0]
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    tools = Path(os.environ.get("LLVM_TOOLS_DIR", sysroot / "lib/rustlib" / host / "bin"))
    if not (tools / "llvm-cov").exists():
        raise SystemExit("Install rustup component llvm-tools-preview or set LLVM_TOOLS_DIR")
    env = {**os.environ,
           "RUSTC_WORKSPACE_WRAPPER": str(ROOT / "scripts/jmap-coverage-rustc.sh"),
           "LLVM_PROFILE_FILE": str(temp / "%m-%p.profraw")}
    build = subprocess.run(["cargo", "test", "--locked", "-p", "moonmail-network",
                            "--lib", "--test", "jmap", "--test", "stalwart", "--no-run",
                            "--message-format=json-render-diagnostics"],
                           cwd=ROOT, env=env, stdout=subprocess.PIPE, text=True, check=True)
    tests = {event["target"]["name"]: event["executable"] for line in build.stdout.splitlines()
             if (event := json.loads(line)).get("executable") and event.get("profile", {}).get("test")}
    subprocess.run([tests["moonmail_network"], "jmap"], env=env, check=True)
    subprocess.run([tests["jmap"]], env=env, check=True)
    server = temp / "server"
    server.mkdir()
    test_stalwart.run(server, executable_test=tests["stalwart"], test_env=env)
    profile = temp / "jmap.profdata"
    subprocess.run([tools / "llvm-profdata", "merge", "-sparse", "-o", profile,
                    *temp.glob("*.profraw")], check=True)
    sources = []
    for crate in ("network", "store"):
        src = ROOT / f"crates/moonmail-{crate}/src"
        sources.extend(sorted(p for p in src.rglob("*.rs")
                              if p.relative_to(src).parts[0].startswith("jmap")
                              and p.name != "tests.rs" and "tests" not in p.relative_to(src).parts))
    objects = [item for name in ("jmap", "stalwart") for item in ("-object", tests[name])]
    args = [tests["moonmail_network"], *objects, f"-instr-profile={profile}", *sources]
    subprocess.run([tools / "llvm-cov", "report", *args], check=True)
    report = subprocess.check_output([tools / "llvm-cov", "export", "-format=lcov", *args], text=True)
    if output := os.environ.get("JMAP_COVERAGE_LCOV"):
        Path(output).write_text(report)
    check_source_coverage(report, sources)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="moonmail-jmap-coverage-") as tmp:
        coverage(Path(tmp))
