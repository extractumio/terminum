# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Which tests exist and which acceptance cases they cite, for scripts/check_changed.py (spec AC-104). A Python
runner's checks split out into `scripts/*_checks.py` modules count as the runner's own: their cases select it,
and a change to one runs it."""
import ast
from pathlib import Path
import re


def page_checks(root):
    """{web/test file: the check of e2e_all.mjs that runs it}, a helper its check imports included."""
    path = root / "web" / "test" / "e2e_all.mjs"
    text = path.read_text() if path.is_file() else ""
    found = {f"web/test/{file}": name for name, file in re.findall(r'import \{ (web\w+) \} from "\./([\w.]+)"', text)}
    for file, name in list(found.items()):
        if (root / file).is_file():
            for helper in re.findall(r'from "\./(e2e_[\w.]+)"', (root / file).read_text(errors="replace")):
                found.setdefault(f"web/test/{helper}", name)
    return found


def is_rust_test(path):
    return "_tests" in Path(path).name or "/tests/" in path


def runnable_e2e(path):
    """Python e2e entry points have a top-level main guard; inspect source without importing it."""
    if not path.is_file():
        return False
    guard = ast.dump(ast.parse('__name__ == "__main__"', mode="eval").body)
    tree = ast.parse(path.read_text(), filename=str(path))
    return any(isinstance(node, ast.If) and ast.dump(node.test) == guard for node in tree.body)


def test_files(root):
    """Files that hold tests: Rust test modules and Python e2e entry points, without their helpers."""
    found = []
    for pattern in ("web/test/*.mjs", "ui/test/*.mjs", "scripts/e2e_*.py", "scripts/tests/*.py", "terminumd/src/**/*.rs"):
        found += [p for p in root.glob(pattern) if p.is_file() and (p.suffix != ".rs" or is_rust_test(str(p)))
                  and (pattern != "scripts/e2e_*.py" or runnable_e2e(p))]
    return found


def citing(cases, root):
    """Test files that cite one of the cases, relative to `root`."""
    if not cases:
        return []
    wanted = re.compile(r"\bAC-(?:" + "|".join(sorted(cases)) + r")\b")
    return sorted(str(p.relative_to(root)) for p in test_files(root) if wanted.search(source(p, root)))


def new_cases(diff):
    """Cases this change adds to a spec's scenarios: `+### AC-nn — …`."""
    return set(re.findall(r"^\+### AC-(\d{2,3}) — ", diff, re.M))


def uncited(cases, root):
    """Those of `cases` no test cites: new behavior comes with tests from its acceptance cases."""
    tests = "\n".join(source(p, root) for p in test_files(root))
    return sorted(c for c in cases if not re.search(rf"\bAC-{c}\b", tests))


def imported_checks(path):
    """The `*_checks` module names a Python file imports, wherever it imports them."""
    names = []
    for node in ast.walk(ast.parse(path.read_text(), filename=str(path))):
        if isinstance(node, ast.Import):
            names += [a.name for a in node.names]
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            names.append(node.module)
    return {n for n in names if n.endswith("_checks")}


def check_modules(path, root):
    """The `*_checks` module names a Python runner imports, directly or through one another's `scripts/` file."""
    found, todo = set(), [path]
    while todo:
        for name in imported_checks(todo.pop()) - found:
            found.add(name)
            if (root / "scripts" / f"{name}.py").is_file():
                todo.append(root / "scripts" / f"{name}.py")
    return found


def source(path, root):
    """A test file's text; a Python runner's with its check modules'."""
    runner = path.parent == root / "scripts"
    modules = [root / "scripts" / f"{n}.py" for n in sorted(check_modules(path, root))] if runner else []
    return "\n".join(p.read_text(errors="replace") for p in [path, *modules] if p.is_file())


def runners_of(files, root):
    """The Python runners that import one of these changed (or deleted) check modules, relative to `root`."""
    changed = {Path(f).stem for f in files if re.fullmatch(r"scripts/\w+_checks\.py", f)}
    if not changed:
        return []
    return sorted(str(p.relative_to(root)) for p in root.glob("scripts/e2e_*.py")
                  if runnable_e2e(p) and changed & check_modules(p, root))
