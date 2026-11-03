"""Install / uninstall the everyday edition (named in brand.env, e.g. DD-up)
(used by scripts/install, scripts/uninstall, scripts/rollback).

Adds, never overwrites what isn't ours:
  <app dir>/<APP_NAME>.app
  ~/.config/herdr-app/mods/{files-plus,browser,tabs-plus}
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
BUILT = ROOT / "artifacts" / f"{NAME}.app"
OFFICIAL = Path("/Applications/Herdr.app")
MODS = ["files-plus", "browser", "tabs-plus"]
LAYOUT = BRAND["LAYOUT"]


def fingerprint(p: Path) -> str:
    h = hashlib.sha256()
    if p.is_file():
        h.update(p.read_bytes())
        return h.hexdigest()
    for f in sorted(x for x in p.rglob("*") if x.is_file() and not x.is_symlink()):
        rel = f.relative_to(p).as_posix()
        # The app writes nothing into mod folders, but macOS may add these.
        if rel.endswith(".DS_Store"):
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
    return [l.split()[0] for l in out.splitlines() if exe in l or l.strip().endswith("/" + EXE)]


def official_version():
    try:
        return plistlib.loads((OFFICIAL / "Contents/Info.plist").read_bytes()).get("CFBundleShortVersionString")
    except Exception:
        return None


def die(msg, code=1):
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(code)


def plan_install(app_dir: Path):
    lock = json.loads((ROOT / "upstream.lock.json").read_text())
    want = lock["herdr_app"]["tag"].lstrip("v")
    if not BUILT.exists():
        die(f"artifacts/{NAME}.app not built: run scripts/build --daily")
    if subprocess.run(["codesign", "--verify", "--deep", str(BUILT)], capture_output=True).returncode != 0:
        die(f"artifacts/{NAME}.app fails codesign --verify")
    built_ver = plistlib.loads((BUILT / "Contents/Info.plist").read_bytes()).get("CFBundleShortVersionString")
    ov = official_version()
    if ov and ov != want:
        die(f"official Herdr is {ov}, {NAME} is built on {want}: rebuild on {ov} first (scripts/prepare-upstream after updating upstream.lock.json)")
    if not APP_HOME.is_dir():
        die(f"no Herdr home at {APP_HOME}: open Herdr once first")
    old = load_manifest() or {"entries": []}
    ours = {e["path"]: e for e in old["entries"]}
    items = [("app", app_dir / f"{NAME}.app", BUILT)]
    items += [("mod", APP_HOME / "mods" / m, ROOT / "mods" / m) for m in MODS]
    items += [("layout", APP_HOME / "layouts" / f"{LAYOUT}.json", ROOT / "layouts/herdr-preview.json.in")]
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
            actions.append(("replace", kind, dst, src))
        else:
            actions.append(("add", kind, dst, src))
    return built_ver, actions, conflicts


def copy_atomic(src: Path, dst: Path):
    dst.parent.mkdir(parents=True, exist_ok=True)
    tmp = dst.with_name(dst.name + f".hup-tmp-{os.getpid()}")
    if tmp.exists():
        shutil.rmtree(tmp) if tmp.is_dir() else tmp.unlink()
    if src.is_dir():
        # ditto keeps the framework's symlinks and the code signature intact.
        subprocess.run(["ditto", str(src), str(tmp)], check=True)
    else:
        shutil.copyfile(src, tmp)
    old = None
    if dst.exists():
        old = dst.with_name(dst.name + f".hup-old-{os.getpid()}")
        os.rename(dst, old)
    os.rename(tmp, dst)
    if old:
        shutil.rmtree(old) if old.is_dir() else old.unlink()


def install(app_dir: Path, dry: bool):
    built_ver, actions, conflicts = plan_install(app_dir)
    print(f"{NAME} {built_ver} (official Herdr {official_version() or 'not found'}), home {APP_HOME}")
    for a, kind, dst, _ in actions:
        print(f"  {a:8} {kind:6} {dst}")
    if conflicts:
        for c in conflicts:
            print(f"  REFUSE  {c}")
        die("nothing was changed")
    if running(app_dir / f"{NAME}.app"):
        die(f"{NAME} is running: quit it first (its terminals keep running in herdr-core)")
    print("  unchanged: /Applications/Herdr.app, built-in mods and layouts, workspaces, terminals, agents")
    if dry:
        print("dry run: nothing changed")
        return
    entries = []
    for _, kind, dst, src in actions:
        copy_atomic(src, dst)
        entries.append({"kind": kind, "path": str(dst), "sha256": app_fingerprint(dst) if kind == "app" else fingerprint(dst)})
    STATE.mkdir(parents=True, exist_ok=True)
    rev = plistlib.loads((BUILT / "Contents/Info.plist").read_bytes()).get("CFBundleVersion")
    MANIFEST.write_text(json.dumps({"installed_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"), "build": rev, "home": str(APP_HOME), "entries": entries}, indent=2, ensure_ascii=False) + "\n")
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
        install(app_dir, dry)
    elif cmd == "uninstall":
        uninstall(dry, "--purge-data" in rest)
    else:
        die(f"unknown command {cmd}", 2)
