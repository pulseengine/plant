#!/usr/bin/env python3
"""ci_controls — prove each CI check can fail, by running it against planted bad input.

Reference implementation for the `gate-potency` skill (Part A, step 1: the negative
control). A repo drops a `ci-controls.toml` at its root; this runs every control in a
scratch copy of HEAD and reports which checks went red when they should, and which
failed open. A green check nobody has seen red is the vacuous-gate class; this makes
"seen red" a re-runnable fact instead of a one-off.

Usage:  ci_controls.py [ci-controls.toml]      (default: ./ci-controls.toml)
        ci_controls.py --format json
        ci_controls.py --only ID               (repeatable; run just those controls)
        ci_controls.py --self-test             (throwaway repos with tiny fake checks)
Exit:   0 = every control passed · 1 = a control failed (or the manifest has none)
        2 = could not run (bad manifest, git missing, no commit)

Manifest (stdlib TOML; the repo root is the manifest's directory):

    [[control]]
    id      = "empty-artifact-list"                 # unique
    check   = "Verification gate"                   # the CI check it protects (reporting)
    run     = "python3 tools/run_verification.py"   # run from the root of a scratch copy
    plant   = "ci-controls/empty-artifact-list"     # dir overlaid onto the scratch copy
    expect  = "fail"                                # the only verdict: must exit non-zero
    stderr_or_stdout_matches = "regex"              # optional: the red output must match
    timeout  = 300                                  # optional, seconds
    baseline = true                                 # optional; false skips the green half
    [control.env]                                   # optional extra environment
    KEY = "value"

How a control runs: `git archive HEAD` into a temp dir (the working tree is never
touched), then

  1. baseline: the command on the unplanted copy must exit 0 (the real input stays green),
     unless `baseline = false`, which is reported as skipped;
  2. plant: every file under the plant dir is copied over the copy at the same relative
     path; an optional file named `delete` in the plant dir lists paths to remove (one
     per line, `#` comments); the command must now exit non-zero, and its combined
     output must match the regex when one is given.

A control FAILS when the planted run exits 0 (fail-open), when the baseline is red, when
either run times out, when the command cannot start (exit 126/127 is not "red"), or when
the plant dir is missing or holds nothing to apply. Plants are read from the working
tree, so a new control can be tried before it is committed; the scratch copy is HEAD, so
commit (or stash) edits to the checks themselves before relying on a verdict.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time

try:
    import tomllib  # Python 3.11+
except ImportError:  # pragma: no cover - the macOS floor is 3.9
    try:
        import tomli as tomllib  # type: ignore
    except ImportError:
        print("ci_controls: needs Python 3.11+ or the tomli package", file=sys.stderr)
        sys.exit(2)

DEFAULT_TIMEOUT = 300
DELETE_FILE = "delete"
KNOWN_KEYS = {"id", "check", "run", "plant", "expect", "stderr_or_stdout_matches",
              "timeout", "baseline", "env"}
TAIL_LINES = 6


class ManifestError(Exception):
    pass


# ── manifest ────────────────────────────────────────────────────────────────

def load_manifest(path: str) -> list:
    try:
        with open(path, "rb") as fh:
            doc = tomllib.load(fh)
    except OSError as e:
        raise ManifestError("cannot read %s: %s" % (path, e))
    except Exception as e:  # noqa: BLE001 - any parse failure is a bad manifest
        raise ManifestError("cannot parse %s: %s" % (path, e))
    controls = doc.get("control", [])
    if not isinstance(controls, list):
        raise ManifestError("`control` must be an array of tables ([[control]])")
    seen = set()
    for i, c in enumerate(controls):
        if not isinstance(c, dict):
            raise ManifestError("control #%d is not a table" % (i + 1))
        cid = c.get("id")
        where = "control #%d" % (i + 1) if not isinstance(cid, str) else "control %r" % cid
        for k in ("id", "check", "run", "plant"):
            if not isinstance(c.get(k), str) or not c[k].strip():
                raise ManifestError("%s: `%s` is missing or not a non-empty string" % (where, k))
        if cid in seen:
            raise ManifestError("%s: duplicate id" % where)
        seen.add(cid)
        unknown = sorted(set(c) - KNOWN_KEYS)
        if unknown:
            raise ManifestError("%s: unknown key(s) %s" % (where, ", ".join(unknown)))
        if c.get("expect", "fail") != "fail":
            raise ManifestError("%s: `expect` must be \"fail\" (the only verdict)" % where)
        rx = c.get("stderr_or_stdout_matches")
        if rx is not None:
            if not isinstance(rx, str) or not rx:
                raise ManifestError("%s: `stderr_or_stdout_matches` must be a non-empty string" % where)
            try:
                re.compile(rx)
            except re.error as e:
                raise ManifestError("%s: bad regex: %s" % (where, e))
        t = c.get("timeout", DEFAULT_TIMEOUT)
        if isinstance(t, bool) or not isinstance(t, (int, float)) or t <= 0:
            raise ManifestError("%s: `timeout` must be a positive number of seconds" % where)
        if not isinstance(c.get("baseline", True), bool):
            raise ManifestError("%s: `baseline` must be true or false" % where)
        env = c.get("env", {})
        if not isinstance(env, dict) or not all(isinstance(v, str) for v in env.values()):
            raise ManifestError("%s: `env` must be a table of strings" % where)
    return controls


# ── scratch copy ────────────────────────────────────────────────────────────

def git(root: str, *args: str) -> str:
    try:
        p = subprocess.run(["git", "-C", root] + list(args), capture_output=True, text=True)
    except OSError as e:
        raise ManifestError("git is not runnable: %s" % e)
    if p.returncode != 0:
        raise ManifestError("git %s failed: %s" % (" ".join(args), p.stderr.strip()))
    return p.stdout


def export_head(root: str, dest: str) -> None:
    """Extract `git archive HEAD` into dest. Nothing in `root` is written."""
    git(root, "rev-parse", "--verify", "HEAD")  # a repo with a commit
    tar_path = os.path.join(dest, "..", os.path.basename(dest) + ".tar")
    with open(tar_path, "wb") as out:
        p = subprocess.run(["git", "-C", root, "archive", "--format=tar", "HEAD"],
                           stdout=out, stderr=subprocess.PIPE)
    if p.returncode != 0:
        raise ManifestError("git archive failed: %s" % p.stderr.decode(errors="replace").strip())
    os.makedirs(dest)
    with tarfile.open(tar_path) as tf:
        tf.extractall(dest)
    os.unlink(tar_path)


def _inside(base: str, rel: str) -> str:
    """Join rel under base, refusing anything that escapes it."""
    if os.path.isabs(rel):
        raise ValueError("absolute path %r" % rel)
    full = os.path.normpath(os.path.join(base, rel))
    b = os.path.normpath(base)
    if full != b and not full.startswith(b + os.sep):
        raise ValueError("path %r escapes the repo root" % rel)
    return full


def plant_files(plant_dir: str) -> tuple:
    """(relative file paths to overlay, paths to delete)."""
    files = []
    for dirpath, _dirs, names in os.walk(plant_dir):
        for n in names:
            rel = os.path.relpath(os.path.join(dirpath, n), plant_dir)
            if rel != DELETE_FILE:
                files.append(rel)
    deletes = []
    dfile = os.path.join(plant_dir, DELETE_FILE)
    if os.path.isfile(dfile):
        with open(dfile, encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if line and not line.startswith("#"):
                    deletes.append(line)
    return sorted(files), deletes


def apply_plant(scratch: str, plant_dir: str, files: list, deletes: list) -> None:
    for rel in files:
        dst = _inside(scratch, rel)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copy2(os.path.join(plant_dir, rel), dst)
    for rel in deletes:
        target = _inside(scratch, rel)
        if not os.path.lexists(target):
            raise ValueError("delete lists %r, which is not in the scratch copy" % rel)
        if os.path.isdir(target) and not os.path.islink(target):
            shutil.rmtree(target)
        else:
            os.unlink(target)


# ── running one command ─────────────────────────────────────────────────────

def run_command(cmd: str, cwd: str, env: dict, timeout: float) -> tuple:
    """(returncode or None on timeout, combined output)."""
    full_env = dict(os.environ)
    full_env.update(env)
    p = subprocess.Popen(cmd, shell=True, cwd=cwd, env=full_env, stdin=subprocess.DEVNULL,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                         start_new_session=True)
    try:
        out, _ = p.communicate(timeout=timeout)
        return p.returncode, out.decode("utf-8", errors="replace")
    except subprocess.TimeoutExpired:
        try:
            os.killpg(p.pid, signal.SIGKILL)  # the shell and anything it started
        except OSError:
            pass
        out, _ = p.communicate()
        return None, out.decode("utf-8", errors="replace")


def tail(text: str) -> str:
    lines = [l for l in text.strip().splitlines() if l.strip()]
    return "\n".join(lines[-TAIL_LINES:])


def run_control(c: dict, root: str, base_copy: str, work: str) -> dict:
    res = {"id": c["id"], "check": c["check"], "status": "FAIL", "reason": "",
           "baseline": "skipped" if c.get("baseline", True) is False else "not run",
           "planted_exit": None, "output_tail": ""}
    timeout = c.get("timeout", DEFAULT_TIMEOUT)
    env = c.get("env", {})
    started = time.time()

    plant_dir = os.path.normpath(os.path.join(root, c["plant"]))
    rootn = os.path.normpath(root)
    if os.path.isabs(c["plant"]) or not (plant_dir == rootn or plant_dir.startswith(rootn + os.sep)):
        res["reason"] = "plant path %r is not inside the repo" % c["plant"]
        return res
    if not os.path.isdir(plant_dir):
        res["reason"] = "plant dir %s is missing, so nothing was planted" % c["plant"]
        return res
    files, deletes = plant_files(plant_dir)
    if not files and not deletes:
        res["reason"] = "plant dir %s holds no files and no deletes, so it plants nothing" % c["plant"]
        return res

    scratch = os.path.join(work, c["id"].replace(os.sep, "_"))
    shutil.copytree(base_copy, scratch, symlinks=True)

    if c.get("baseline", True) is not False:
        rc, out = run_command(c["run"], scratch, env, timeout)
        if rc is None:
            res["baseline"] = "timeout"
            res["reason"] = "baseline timed out after %ss" % timeout
            res["output_tail"] = tail(out)
            return res
        if rc != 0:
            res["baseline"] = "red (exit %d)" % rc
            res["reason"] = "baseline is red (exit %d) before anything is planted" % rc
            res["output_tail"] = tail(out)
            return res
        res["baseline"] = "green"
        # the baseline run may have written files (caches, reports); start the plant clean
        shutil.rmtree(scratch)
        shutil.copytree(base_copy, scratch, symlinks=True)

    try:
        apply_plant(scratch, plant_dir, files, deletes)
    except (ValueError, OSError) as e:
        res["reason"] = "cannot apply plant: %s" % e
        return res

    rc, out = run_command(c["run"], scratch, env, timeout)
    res["planted_exit"] = rc
    res["output_tail"] = tail(out)
    if rc is None:
        res["reason"] = "planted run timed out after %ss (a hang is not a red)" % timeout
    elif rc in (126, 127):
        res["reason"] = "command could not start (exit %d), which is not a red" % rc
    elif rc == 0:
        res["reason"] = "FAIL-OPEN: planted run exited 0, the check did not notice the plant"
    elif c.get("stderr_or_stdout_matches") and not re.search(c["stderr_or_stdout_matches"], out):
        res["reason"] = ("exited %d but the output does not match /%s/, so it went red for "
                         "some other reason" % (rc, c["stderr_or_stdout_matches"]))
    else:
        res["status"] = "PASS"
        res["reason"] = "planted run exited %d" % rc
        if c.get("stderr_or_stdout_matches"):
            res["reason"] += ", output matched"
    res["seconds"] = round(time.time() - started, 1)
    return res


# ── driver ──────────────────────────────────────────────────────────────────

def run_all(manifest: str, only: list) -> tuple:
    """(exit code, results). Raises ManifestError for exit 2."""
    controls = load_manifest(manifest)
    if only:
        known = {c["id"] for c in controls}
        missing = [o for o in only if o not in known]
        if missing:
            raise ManifestError("--only names no such control: %s" % ", ".join(missing))
        controls = [c for c in controls if c["id"] in only]
    if not controls:
        return 1, []
    root = os.path.dirname(os.path.abspath(manifest))
    work = tempfile.mkdtemp(prefix="ci-controls-")
    try:
        base_copy = os.path.join(work, "head")
        export_head(root, base_copy)
        dirty = git(root, "status", "--porcelain", "--untracked-files=no").strip()
        if dirty:
            print("ci_controls: note: tracked files have uncommitted changes; the scratch copy "
                  "is HEAD, so those edits are not under test", file=sys.stderr)
        results = [run_control(c, root, base_copy, os.path.join(work, "runs")) for c in controls]
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return (0 if all(r["status"] == "PASS" for r in results) else 1), results


def render_text(results: list) -> str:
    lines = []
    for r in results:
        lines.append("%s  %s  [%s]: %s" % (r["status"], r["id"], r["check"], r["reason"]))
        if r["baseline"] == "skipped":
            lines.append("      note: baseline = false, so the real input staying green is NOT checked")
        if r["status"] == "FAIL" and r["output_tail"]:
            for l in r["output_tail"].splitlines():
                lines.append("      | " + l)
    p = sum(1 for r in results if r["status"] == "PASS")
    lines.append("%d control(s): %d passed, %d failed" % (len(results), p, len(results) - p))
    if not results:
        lines.append("FAIL  the manifest has no controls: nothing was compared")
    return "\n".join(lines)


# ── self-test ───────────────────────────────────────────────────────────────

CHECK_PY = '''import os, sys, time
mode = sys.argv[1] if len(sys.argv) > 1 else "data"
open("leak.txt", "w").write("written by the check\\n")   # must never reach the real tree
if mode == "always-ok":
    sys.exit(0)
if mode == "always-red":
    print("always red"); sys.exit(1)
if mode == "sleep":
    time.sleep(60); sys.exit(0)
if mode == "needs-file":
    if not os.path.exists("required.txt"):
        print("required.txt is gone"); sys.exit(1)
    sys.exit(0)
data = open("data.txt").read()
if "bad" in data:
    print("data.txt is bad: rule R7 violated"); sys.exit(1)
print("ok"); sys.exit(0)
'''


def _snapshot(root: str) -> dict:
    snap = {}
    for dp, dirs, names in os.walk(root):
        dirs[:] = [d for d in dirs if d != ".git"]
        for n in names:
            p = os.path.join(dp, n)
            with open(p, "rb") as fh:
                snap[os.path.relpath(p, root)] = fh.read()
    return snap


def self_test() -> int:
    me = os.path.abspath(__file__)
    py = '"%s"' % sys.executable
    failures = []

    def expect(name: str, ok: bool, detail: str = "") -> None:
        print("%s  %s%s" % ("ok  " if ok else "FAIL", name, ("  (%s)" % detail) if detail and not ok else ""))
        if not ok:
            failures.append(name)

    def mk(path: str, text: str) -> None:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)

    def sh(root: str, *args: str) -> None:
        subprocess.run(["git", "-C", root, "-c", "user.name=t", "-c", "user.email=t@example.invalid",
                        "-c", "commit.gpgsign=false"] + list(args),
                       check=True, capture_output=True)

    def run_runner(root: str, *args: str) -> tuple:
        env = dict(os.environ, TMPDIR=scratch_tmp)
        p = subprocess.run([sys.executable, me] + list(args), cwd=root, capture_output=True, text=True, env=env)
        return p.returncode, p.stdout, p.stderr

    tmp = tempfile.mkdtemp(prefix="ci-controls-selftest-")
    try:
        repo = os.path.join(tmp, "repo")
        scratch_tmp = os.path.join(tmp, "scratch")  # the runner's TMPDIR: must be empty afterwards
        os.makedirs(scratch_tmp)
        os.makedirs(repo)
        sh(repo, "init", "-q")
        mk(os.path.join(repo, "check.py"), CHECK_PY)
        mk(os.path.join(repo, "data.txt"), "all fine\n")
        mk(os.path.join(repo, "required.txt"), "here\n")
        sh(repo, "add", "-A")
        sh(repo, "commit", "-q", "-m", "fixture")

        # plants live in the working tree and are deliberately NOT committed
        bad = os.path.join(repo, "ci-controls", "plant-bad")
        mk(os.path.join(bad, "data.txt"), "bad\n")
        mk(os.path.join(repo, "ci-controls", "plant-bad-other", "data.txt"), "bad\n")
        mk(os.path.join(repo, "ci-controls", "plant-delete", "delete"), "# drop it\nrequired.txt\n")
        mk(os.path.join(repo, "ci-controls", "plant-escape", "delete"), "../outside.txt\n")
        mk(os.path.join(repo, "ci-controls", "plant-nothing", "delete"), "# only a comment\n")
        mk(os.path.join(repo, "ci-controls", "plant-ok", "data.txt"), "still fine\n")
        os.makedirs(os.path.join(repo, "ci-controls", "plant-empty"))

        def ctl(cid: str, run: str, plant: str, extra: str = "") -> str:
            return ('[[control]]\nid = "%s"\ncheck = "Fake check"\nrun = %s\nplant = "%s"\n%s\n'
                    % (cid, json.dumps(run), plant, extra))

        data_run = py + " check.py data"
        manifest = "".join([
            ctl("good-fails-on-plant", data_run, "ci-controls/plant-bad"),
            ctl("good-with-regex", data_run, "ci-controls/plant-bad",
                'stderr_or_stdout_matches = "rule R7"'),
            ctl("regex-mismatch", data_run, "ci-controls/plant-bad-other",
                'stderr_or_stdout_matches = "rule R99"'),
            ctl("fail-open", py + " check.py always-ok", "ci-controls/plant-bad"),
            ctl("plant-not-noticed", data_run, "ci-controls/plant-ok"),
            ctl("baseline-red", py + " check.py always-red", "ci-controls/plant-bad"),
            ctl("missing-plant", data_run, "ci-controls/does-not-exist"),
            ctl("empty-plant", data_run, "ci-controls/plant-empty"),
            ctl("comment-only-plant", data_run, "ci-controls/plant-nothing"),
            ctl("timeout", py + " check.py sleep", "ci-controls/plant-bad", "timeout = 1\nbaseline = false"),
            ctl("delete-plant", py + " check.py needs-file", "ci-controls/plant-delete"),
            ctl("delete-escapes-root", data_run, "ci-controls/plant-escape"),
            ctl("command-not-found", "no-such-command-ci-controls-xyz", "ci-controls/plant-bad",
                "baseline = false"),
            ctl("baseline-skipped", py + " check.py always-red", "ci-controls/plant-bad", "baseline = false"),
            ctl("env-reaches-check", py + ' -c "import os,sys; sys.exit(0 if os.environ.get(\'CI_CTL\') != \'1\' else 3)"',
                "ci-controls/plant-bad", "baseline = false\n[control.env]\nCI_CTL = \"1\""),
        ])
        mk(os.path.join(repo, "ci-controls.toml"), manifest)

        before = _snapshot(repo)
        rc, out, err = run_runner(repo, "--format", "json")
        after = _snapshot(repo)
        try:
            doc = json.loads(out)
            by = {r["id"]: r for r in doc["controls"]}
        except (ValueError, KeyError):
            expect("json output parses", False, out[:200] + err[:200])
            return 1
        expect("json output parses and lists every control", len(by) == 15)

        def verdict(cid: str, want: str, needle: str = "") -> None:
            r = by.get(cid, {})
            ok = r.get("status") == want and needle in r.get("reason", "")
            expect("%-22s -> %s%s" % (cid, want, (" (" + needle + ")") if needle else ""), ok,
                   "%s: %s" % (r.get("status"), r.get("reason")))

        verdict("good-fails-on-plant", "PASS")
        verdict("good-with-regex", "PASS", "matched")
        verdict("regex-mismatch", "FAIL", "does not match")
        verdict("fail-open", "FAIL", "FAIL-OPEN")
        verdict("plant-not-noticed", "FAIL", "FAIL-OPEN")
        verdict("baseline-red", "FAIL", "baseline is red")
        verdict("missing-plant", "FAIL", "missing")
        verdict("empty-plant", "FAIL", "plants nothing")
        verdict("comment-only-plant", "FAIL", "plants nothing")
        verdict("timeout", "FAIL", "timed out")
        verdict("delete-plant", "PASS")
        verdict("delete-escapes-root", "FAIL", "escapes")
        verdict("command-not-found", "FAIL", "could not start")
        verdict("baseline-skipped", "PASS")
        expect("a skipped baseline is reported as skipped", by.get("baseline-skipped", {}).get("baseline") == "skipped")
        r = by.get("env-reaches-check", {})
        expect("control env reaches the command (exit 3 under CI_CTL=1)",
               r.get("status") == "PASS" and r.get("planted_exit") == 3, str(r))
        expect("overall exit is 1 when any control failed", rc == 1, "rc=%d" % rc)
        expect("the working tree is byte-identical after a run", before == after)
        expect("a check's own writes stay in the scratch copy", not os.path.exists(os.path.join(repo, "leak.txt")))
        expect("the runner leaves no scratch dirs behind", os.listdir(scratch_tmp) == [], str(os.listdir(scratch_tmp)))

        # an all-green manifest exits 0, and --only narrows it
        mk(os.path.join(repo, "good.toml"), ctl("g1", data_run, "ci-controls/plant-bad") +
           ctl("g2", py + " check.py needs-file", "ci-controls/plant-delete"))
        rc, out, _ = run_runner(repo, "good.toml")
        expect("all controls passing exits 0", rc == 0 and "2 passed, 0 failed" in out, "rc=%d %s" % (rc, out))
        rc, out, _ = run_runner(repo, "good.toml", "--only", "g2")
        expect("--only runs just the named control", rc == 0 and "1 control(s)" in out, out)
        rc, _, _ = run_runner(repo, "good.toml", "--only", "nope")
        expect("--only with an unknown id exits 2", rc == 2)

        # a manifest that compares nothing, or cannot be read, is never green
        mk(os.path.join(repo, "empty.toml"), "# no controls\n")
        rc, out, _ = run_runner(repo, "empty.toml")
        expect("an empty manifest fails (exit 1 or 2)", rc in (1, 2) and rc != 0, "rc=%d" % rc)
        mk(os.path.join(repo, "garbled.toml"), "[[control]\nid = ")
        rc, _, _ = run_runner(repo, "garbled.toml")
        expect("an unparseable manifest exits 2", rc == 2, "rc=%d" % rc)
        rc, _, _ = run_runner(repo, "absent.toml")
        expect("a missing manifest exits 2", rc == 2, "rc=%d" % rc)
        mk(os.path.join(repo, "dup.toml"), ctl("x", data_run, "ci-controls/plant-bad") +
           ctl("x", data_run, "ci-controls/plant-bad"))
        rc, _, _ = run_runner(repo, "dup.toml")
        expect("a duplicate id exits 2", rc == 2, "rc=%d" % rc)
        mk(os.path.join(repo, "badexp.toml"), ctl("x", data_run, "ci-controls/plant-bad", 'expect = "pass"'))
        rc, _, _ = run_runner(repo, "badexp.toml")
        expect("expect other than \"fail\" exits 2", rc == 2, "rc=%d" % rc)

        # not a git repo / no commit: cannot run, exit 2
        bare = os.path.join(tmp, "nogit")
        os.makedirs(bare)
        mk(os.path.join(bare, "ci-controls.toml"), ctl("x", "true", "p"))
        mk(os.path.join(bare, "p", "f"), "x")
        rc, _, _ = run_runner(bare)
        expect("a directory with no git history exits 2", rc == 2, "rc=%d" % rc)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    print("self-test: %s" % ("%d check(s) FAILED" % len(failures) if failures else "all checks hold"))
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Prove CI checks can fail, with planted bad inputs.")
    ap.add_argument("manifest", nargs="?", default="ci-controls.toml")
    ap.add_argument("--format", choices=("text", "json"), default="text")
    ap.add_argument("--only", action="append", default=[], metavar="ID")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    try:
        code, results = run_all(args.manifest, args.only)
    except ManifestError as e:
        print("ci_controls: %s" % e, file=sys.stderr)
        return 2
    if args.format == "json":
        p = sum(1 for r in results if r["status"] == "PASS")
        print(json.dumps({"controls": results,
                          "summary": {"total": len(results), "passed": p, "failed": len(results) - p,
                                      "empty_manifest": not results}}, indent=2))
    else:
        print(render_text(results))
    return code


if __name__ == "__main__":
    sys.exit(main())
