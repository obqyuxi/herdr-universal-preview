"""Install / uninstall the everyday edition (named in brand.env, e.g. DD-up)
(used by scripts/install, scripts/uninstall, scripts/rollback).

Adds, never overwrites what isn't ours:
  <app dir>/<APP_NAME>.app
  ~/.config/herdr-app/mods/{files-plus,files-plus-menu,browser,tabs-plus,workspace-chrome-plus,phone-pair}
  ~/.config/herdr-app/layouts/<LAYOUT>.json
Records each with a content fingerprint in
  ~/Library/Application Support/<APP_EXE>/install-manifest.json
Uninstall removes only manifest entries whose fingerprint is unchanged.
The official /Applications/Herdr.app, its built-in mods and layouts, the
workspaces, terminals and agent sessions are never touched.
"""
import hashlib, json, os, plistlib, shutil, subprocess, sys, time
from pathlib import Path

HOME = Path.home()
ROOT = Path(__file__).resolve().parents[2]
BRAND = dict(l.split("=", 1) for l in (ROOT / "brand.env").read_text().splitlines() if l and not l.startswith("#"))
BRAND = {k: v.strip().strip('"') for k, v in BRAND.items()}
NAME, EXE = BRAND["APP_NAME"], BRAND["APP_EXE"]
APP_HOME = Path(os.environ.get("HUP_TARGET_HOME", HOME / ".config/herdr-app"))
STATE = Path(os.environ.get("HUP_STATE_DIR", HOME / "Library/Application Support" / EXE))
MANIFEST = STATE / "install-manifest.json"
# Built apps sit in a ".noindex" folder so Spotlight and Launchpad don't list them.
BUILT = ROOT / "artifacts/apps.noindex" / f"{NAME}.app"
OFFICIAL = Path("/Applications/Herdr.app")
MODS = ["files-plus", "files-plus-menu", "browser", "tabs-plus", "workspace-chrome-plus", "phone-pair"]
LAYOUT = BRAND["LAYOUT"]


def fingerprint(p: Path) -> str:
    h = hashlib.sha256()
    if p.is_file():
        h.update(p.read_bytes())
        return h.hexdigest()
    for f in sorted(x for x in p.rglob("*") if x.is_file() and not x.is_symlink()):
        rel = f.relative_to(p).as_posix()
        # Herdr rewrites each mod's type files on start (gpui-kit.d.ts, herdr.d.ts)
        # to match its own build; macOS may add .DS_Store. Neither is a user change.
        if rel.endswith(".DS_Store") or rel.endswith(".d.ts"):
            continue
        h.update(rel.encode() + b"\0" + hashlib.sha256(f.read_bytes()).digest())
    return h.hexdigest()


def app_fingerprint(app: Path) -> str:
    # The executable and Info.plist identify a build; the framework is large.
    h = hashlib.sha256()
    info = app / "Contents/Info.plist"
    h.update(info.read_bytes())
    exe = plistlib.loads(info.read_bytes())["CFBundleExecutable"]
    h.update((app / "Contents/MacOS" / exe).read_bytes())
    return h.hexdigest()


def load_manifest():
    return json.loads(MANIFEST.read_text()) if MANIFEST.exists() else None


def running(app: Path) -> list:
    out = subprocess.run(["ps", "-axo", "pid=,comm="], capture_output=True, text=True).stdout
    exe = str(app / "Contents/MacOS" / EXE)
    # Only the copy being replaced matters (a test install elsewhere may run beside it).
    return [l.split()[0] for l in out.splitlines() if l.split(None, 1)[-1].strip() == exe]


def official_version():
    try:
        return plistlib.loads((OFFICIAL / "Contents/Info.plist").read_bytes()).get("CFBundleShortVersionString")
    except Exception:
        return None


def die(msg, code=1):
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(code)


def derive_layout(base: str) -> dict:
    """The person's own layout with DD-up's changes: Files -> files-plus, tabs -> tabs-plus,
    a browser block in the stack that holds Files, and a wider panel. Everything else
    (their sidebar mods, sizes, colors) stays as it is."""
    if not base or "/" in base or base.startswith(".") or base.startswith(f"{LAYOUT}-") or base == LAYOUT:
        die(f"not a layout to build on: {base!r}")
    src = APP_HOME / "layouts" / f"{base}.json"
    if not src.is_file():
        die(f"no layout {base!r} in {APP_HOME / 'layouts'}")
    doc = json.loads(src.read_text())
    found = {"files": 0, "tabs": 0, "stack": 0}

    def walk(node, parent):
        content = node.get("content")
        if isinstance(content, dict) and content.get("mod") == "files":
            content["mod"] = "files-plus"; found["files"] += 1
        if isinstance(content, dict) and content.get("mod") == "tabs":
            content["mod"] = "tabs-plus"; found["tabs"] += 1
        # The sidebar footer gets the phone button (D27); optional, layouts without it are fine.
        if isinstance(content, dict) and content.get("mod") == "workspace-chrome":
            content["mod"] = "workspace-chrome-plus"
        kids = node.get("children", [])
        if node.get("type") == "stack" and any(isinstance(k.get("content"), dict) and k["content"].get("mod") in ("files", "files-plus") for k in kids):
            if not any(isinstance(k.get("content"), dict) and k["content"].get("mod") == "browser" for k in kids):
                kids.append({"id": "browser", "type": "block", "content": {"mod": "browser"}})
            found["stack"] += 1
            if parent is not None and isinstance(parent.get("size"), dict) and parent["size"].get("px", 0) < 560:
                parent["size"]["px"] = 560
                parent["max"] = max(parent.get("max", 0), 1400)
        for k in kids:
            walk(k, node)

    walk(doc, None)
    if not (found["files"] and found["tabs"] and found["stack"]):
        die(f"layout {base!r} has no Files panel stack and tabs bar to extend ({found}); use the {LAYOUT} layout instead")
    return doc


def layout_source(base):
    """layouts/<LAYOUT>.json: the shipped one (official main + DD-up), or the
    person's own layout `base` with DD-up's changes. One DD-up layout either way."""
    if not base:
        return ROOT / "layouts/herdr-preview.json.in"
    tmpdir = Path(os.environ.get("TMPDIR", "/tmp")) / f"hup-layouts-{os.getpid()}"
    tmpdir.mkdir(parents=True, exist_ok=True)
    src = tmpdir / f"{LAYOUT}.json"
    src.write_text(json.dumps(derive_layout(base), indent=2, ensure_ascii=False) + "\n")
    return src


def plan_install(app_dir: Path, base_arg):
    lock = json.loads((ROOT / "upstream.lock.json").read_text())
    want = lock["herdr_app"]["tag"].lstrip("v")
    if not BUILT.exists():
        die(f"artifacts/apps.noindex/{NAME}.app not built: run scripts/build --daily")
    if subprocess.run(["codesign", "--verify", "--deep", str(BUILT)], capture_output=True).returncode != 0:
        die(f"artifacts/apps.noindex/{NAME}.app fails codesign --verify")
    built_ver = plistlib.loads((BUILT / "Contents/Info.plist").read_bytes()).get("CFBundleShortVersionString")
    ov = official_version()
    if ov and ov != want:
        die(f"official Herdr is {ov}, {NAME} is built on {want}: rebuild on {ov} first (scripts/prepare-upstream after updating upstream.lock.json)")
    if not APP_HOME.is_dir():
        die(f"no Herdr home at {APP_HOME}: open Herdr once first")
    old = load_manifest() or {"entries": []}
    ours = {e["path"]: e for e in old["entries"]}
    # The layout's base: given now, else remembered ("bases" was the older list form).
    base = base_arg if base_arg is not None else old.get("base", (old.get("bases") or [None])[0])
    if base in ("", "main"):
        base = None if base == "" else base
    items = [("app", app_dir / f"{NAME}.app", BUILT)]
    items += [("mod", APP_HOME / "mods" / m, ROOT / "mods" / m) for m in MODS]
    items += [("layout", APP_HOME / "layouts" / f"{LAYOUT}.json", layout_source(base))]
    actions, conflicts = [], []
    for kind, dst, src in items:
        if dst.exists():
            prev = ours.get(str(dst))
            now = app_fingerprint(dst) if kind == "app" else fingerprint(dst)
            if not prev:
                conflicts.append(f"{dst} exists and was not installed by this project")
                continue
            if prev["sha256"] != now:
                conflicts.append(f"{dst} was changed after installing; move your changes aside first")
                continue
            if kind == "app" and now == app_fingerprint(src):
                actions.append(("same", kind, dst, src))   # same build: leave the (maybe running) app alone
                continue
            actions.append(("replace", kind, dst, src))
        else:
            actions.append(("add", kind, dst, src))
    # Earlier installs may have left entries this one no longer makes (e.g. dd-up-<base>.json).
    keep = {str(d) for _, _, d, _ in actions}
    stale = [e for e in old["entries"] if e["path"] not in keep]
    return built_ver, actions, conflicts, base, stale


def copy_atomic(src: Path, dst: Path):
    dst.parent.mkdir(parents=True, exist_ok=True)
    # Hidden, and keeping the suffix: a half-copied "X.app.tmp" is a plain folder to
    # Spotlight, which then lists the helper apps inside it as separate apps.
    tmp = dst.with_name(f".{dst.stem}.hup-tmp-{os.getpid()}{dst.suffix}")
    if tmp.exists():
        shutil.rmtree(tmp) if tmp.is_dir() else tmp.unlink()
    if src.is_dir():
        # ditto keeps the framework's symlinks and the code signature intact.
        subprocess.run(["ditto", str(src), str(tmp)], check=True)
    else:
        shutil.copyfile(src, tmp)
    old = None
    if dst.exists():
        old = dst.with_name(f".{dst.stem}.hup-old-{os.getpid()}{dst.suffix}")
        os.rename(dst, old)
    os.rename(tmp, dst)
    if old:
        shutil.rmtree(old) if old.is_dir() else old.unlink()


def install(app_dir: Path, dry: bool, base_arg=None, mods_only=False):
    built_ver, actions, conflicts, base, stale = plan_install(app_dir, base_arg)
    if mods_only:
        # Leave the installed (maybe running) app as it is; replace only mods and the layout.
        if not (app_dir / f"{NAME}.app").exists():
            die(f"--mods-only needs {NAME} installed first")
        actions = [("same", k, d, s) if k == "app" else (a, k, d, s) for a, k, d, s in actions]
    print(f"{NAME} {built_ver} (official Herdr {official_version() or 'not found'}), home {APP_HOME}")
    print(f"  layout {LAYOUT}: {'from your ' + base + '.json (left as it is)' if base else 'official main + DD-up'}")
    for a, kind, dst, _ in actions:
        print(f"  {a:8} {kind:6} {dst}")
    for e in stale:
        p = Path(e["path"])
        changed = p.exists() and (app_fingerprint(p) if e["kind"] == "app" else fingerprint(p)) != e["sha256"]
        print(f"  {'KEEP' if changed else 'remove':8} {e['kind']:6} {p}{' (changed after install; left for you)' if changed else ''}")
    if conflicts:
        for c in conflicts:
            print(f"  REFUSE  {c}")
        die("nothing was changed")
    if any(a in ("add", "replace") and k == "app" for a, k, _, _ in actions) and running(app_dir / f"{NAME}.app"):
        die(f"{NAME} is running and its app needs replacing: quit it first (its terminals keep running in herdr-core)")
    print("  unchanged: /Applications/Herdr.app, built-in mods and layouts, workspaces, terminals, agents")
    if dry:
        print("dry run: nothing changed")
        return
    entries = []
    for a, kind, dst, src in actions:
        if a != "same":
            copy_atomic(src, dst)
        entries.append({"kind": kind, "path": str(dst), "sha256": app_fingerprint(dst) if kind == "app" else fingerprint(dst)})
    for e in stale:
        p = Path(e["path"])
        if not p.exists():
            continue
        if (app_fingerprint(p) if e["kind"] == "app" else fingerprint(p)) != e["sha256"]:
            entries.append(e)
            continue
        shutil.rmtree(p) if p.is_dir() else p.unlink()
    STATE.mkdir(parents=True, exist_ok=True)
    app_plist = (app_dir / f"{NAME}.app" if mods_only else BUILT) / "Contents/Info.plist"
    rev = plistlib.loads(app_plist.read_bytes()).get("CFBundleVersion")
    MANIFEST.write_text(json.dumps({"installed_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "build": rev, "home": str(APP_HOME), "base": base, "entries": entries}, indent=2, ensure_ascii=False) + "\n")
    print(f"installed; manifest: {MANIFEST}")


def uninstall(dry: bool, purge: bool):
    m = load_manifest()
    if not m:
        print(f"{NAME} is not installed (no manifest)")
        return
    apps = [Path(e["path"]) for e in m["entries"] if e["kind"] == "app"]
    if any(running(a) for a in apps):
        die(f"{NAME} is running: quit it first")
    keep = []
    for e in m["entries"]:
        p = Path(e["path"])
        if not p.exists():
            print(f"  gone    {p}")
            continue
        now = app_fingerprint(p) if e["kind"] == "app" else fingerprint(p)
        if now != e["sha256"]:
            print(f"  KEEP    {p} (changed after install; left for you)")
            keep.append(e)
            continue
        print(f"  remove  {p}")
        if not dry:
            shutil.rmtree(p) if p.is_dir() else p.unlink()
    for d in [HOME / "Library/Application Support" / EXE / "Chromium"]:
        if d.exists():
            print(f"  {'remove' if purge else 'keep  '}  {d} (browser logins and history{'' if purge else '; --purge-data removes it'})")
            if purge and not dry:
                shutil.rmtree(d)
    if dry:
        print("dry run: nothing changed")
        return
    if keep:
        m["entries"] = keep
        MANIFEST.write_text(json.dumps(m, indent=2, ensure_ascii=False) + "\n")
    else:
        MANIFEST.unlink()
    print("uninstalled. If Herdr opens on a missing layout, pick one with ctrl-shift-l (or herdr-app do layout.switch '{\"name\":\"main\"}').")


if __name__ == "__main__":
    cmd, *rest = sys.argv[1:]
    dry = "--dry-run" in rest
    if cmd == "install":
        app_dir = Path(next((rest[i + 1] for i, a in enumerate(rest) if a == "--app-dir"), HOME / "Applications"))
        install(app_dir, dry, mods_only="--mods-only" in rest)
    elif cmd == "add-layout":
        app_dir = Path(next((rest[i + 1] for i, a in enumerate(rest) if a == "--app-dir"), HOME / "Applications"))
        install(app_dir, dry, next((a for a in rest if not a.startswith("--") and a != str(app_dir)), ""))
    elif cmd == "uninstall":
        uninstall(dry, "--purge-data" in rest)
    else:
        die(f"unknown command {cmd}", 2)
