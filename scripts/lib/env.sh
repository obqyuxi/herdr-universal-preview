# shellcheck shell=bash
# The single source of truth for the isolated test environment.
# Source it from bash scripts:  . "$(dirname "$0")/lib/env.sh"
#
# What it guarantees (see docs/ENVIRONMENT.md for the evidence):
# - the official Herdr app home (~/.config/herdr-app), core session ("default"),
#   ~/.local/bin and /Applications/Herdr.app are never targeted;
# - every hc / herdr-app call made through these helpers uses an explicit
#   absolute test binary and the test session, never PATH lookups;
# - variables inherited from an official Herdr terminal (HC_TERMINAL_ID, ...)
#   are removed so nothing can talk to the official core by accident.

set -euo pipefail

HUP_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
HUP_LOCAL=$HUP_ROOT/.local
# macOS limits unix socket paths to 104 bytes; the project path is too long for
# herdr-core's "<state>/herdr-core/<session>/<session>.holder.<hash>.sock".
# A short symlink points back into the project's .local directory.
HUP_SHORT=/tmp/hup-$(id -u)

HUP_SESSION=hup-dev
HUP_TOOLCHAIN=$HUP_LOCAL/toolchain
HUP_BIN=$HUP_LOCAL/bin                       # explicit test binaries only
HUP_HC=$HUP_BIN/hc                           # copy of herdr-core, see scripts/dev core-install
HUP_APP_HOME=$HUP_LOCAL/app-home
HUP_LOGS=$HUP_LOCAL/logs
HUP_RUN=$HUP_LOCAL/run                       # pid files of processes this project started
HUP_WORK=$HUP_ROOT/.work
HUP_UPSTREAM=$HUP_WORK/herdr-app
HUP_APPS=$HUP_ROOT/artifacts/apps.noindex          # ".noindex": Spotlight/Launchpad skip built apps
HUP_DEV_APP="$HUP_APPS/Herdr Preview Dev.app"
HUP_DEV_EXE="$HUP_DEV_APP/Contents/MacOS/HerdrPreviewDev"

# Official locations: read-only observation targets, never written.
OFFICIAL_APP=/Applications/Herdr.app
OFFICIAL_HOME=$HOME/.config/herdr-app
OFFICIAL_HC=$HOME/.local/bin/hc

hup_die() { echo "error: $*" >&2; exit 1; }

hup_ensure_short_link() {
  mkdir -p "$HUP_LOCAL"
  if [[ -L $HUP_SHORT ]]; then
    [[ $(readlink "$HUP_SHORT") == "$HUP_LOCAL" ]] || hup_die "$HUP_SHORT points elsewhere ($(readlink "$HUP_SHORT")); not touching it"
  elif [[ -e $HUP_SHORT ]]; then
    hup_die "$HUP_SHORT exists and is not our symlink; not touching it"
  else
    ln -s "$HUP_LOCAL" "$HUP_SHORT"
  fi
}

# Export the isolated environment into the current shell.
hup_isolate() {
  # Never inherit a connection to the official core or terminal.
  unset HC_SOCKET HC_TERMINAL_ID HC_TERMINAL_SOCKET HC_SCOPE HC_SESSION \
        HERDR_APP_HOME HERDR_CORE_DIRS HERDR_RELEASES_URL HC_INSTALL_DIR
  export HC_SESSION=$HUP_SESSION
  export XDG_STATE_HOME=$HUP_SHORT/state
  export XDG_CONFIG_HOME=$HUP_SHORT/config
  export HERDR_APP_HOME=$HUP_APP_HOME
  export HERDR_CORE_DIRS=$HUP_BIN            # app looks for its core only here
  export HC_INSTALL_DIR=$HUP_BIN             # a core install, if ever triggered, lands here
  export HERDR_APP_NO_SOUND=1
  export HUP_CHROMIUM_PROFILE=$HUP_LOCAL/chromium-profile   # never the person's Chrome profile
  export HERDR_TELEMETRY=0                   # source builds never send; also keep it off locally
  mkdir -p "$HUP_LOCAL/state" "$HUP_LOCAL/config" "$HUP_APP_HOME" "$HUP_LOGS" "$HUP_RUN"
}

# Toolchain on PATH for this process only (project-local rustup + zig).
hup_toolchain() {
  export RUSTUP_HOME=$HUP_TOOLCHAIN/rustup
  export CARGO_HOME=$HUP_TOOLCHAIN/cargo
  export ZIG=$HUP_TOOLCHAIN/zig/zig
  export CEF_PATH=$HUP_LOCAL/cef                # CEF 156 binaries (export-cef-dir)
  export PATH=$CARGO_HOME/bin:$HUP_TOOLCHAIN/zig:$HUP_LOCAL/py/bin:$PATH
}

# Run the test hc (never the PATH one) with the isolated env.
hup_hc() {
  [[ -x $HUP_HC ]] || hup_die "test hc missing: run scripts/dev core-install"
  "$HUP_HC" "$@"
}

# Read-only snapshot of the official environment, printed as JSON lines.
hup_official_snapshot() {
  (
    unset HC_SOCKET HC_SESSION XDG_STATE_HOME XDG_CONFIG_HOME HERDR_APP_HOME HERDR_CORE_DIRS
    printf '{"official_core":%s}\n' "$("$OFFICIAL_HC" server status --json 2>/dev/null | /usr/bin/python3 -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps({"id":d["core"]["id"],"pid":d["pid"],"live":d["live"],"version":d["version"]}))' 2>/dev/null || echo null)"
  )
  printf '{"launchd":%s}\n' "$(launchctl list 2>/dev/null | awk '/herdr/{print $3}' | sort | /usr/bin/python3 -c 'import json,sys; print(json.dumps([l.strip() for l in sys.stdin if l.strip()]))')"
  local f
  for f in "$HOME/.local/bin/herdr-app" "$HOME/.local/bin/hc" "$HOME/.local/bin/herdr-core" \
           "$OFFICIAL_HOME/keys.json" "$OFFICIAL_HOME/settings.json" "$OFFICIAL_HOME/layouts/main.json" \
           "$OFFICIAL_APP/Contents/Info.plist" "$OFFICIAL_APP/Contents/MacOS/Herdr"; do
    if [[ -L $f ]]; then
      printf '{"path":"%s","link":"%s"}\n' "$f" "$(readlink "$f")"
    elif [[ -f $f ]]; then
      printf '{"path":"%s","sha256":"%s"}\n' "$f" "$(shasum -a 256 "$f" | cut -c1-16)"
    else
      printf '{"path":"%s","missing":true}\n' "$f"
    fi
  done
}
