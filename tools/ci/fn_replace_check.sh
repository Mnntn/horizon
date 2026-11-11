#!/usr/bin/env bash
# Function replacement end to end (fn-replace): two native mods' `[[native.mixin]]` rows built by the
# launcher's normal native build against a PREBUILT game, with no game rebuild, then run on a
# dedicated server.
#   [FNR_MODS="alpha beta"] tools/ci/fn_replace_check.sh LAUNCHER SDK SCRATCH [RESOURCES]
# SDK is a folder from `tools/prebuilt_game.py build --server` at this checkout's HEAD. With
# RESOURCES (a prepared `horizon-client/26.3` folder), the server starts, takes `/tick` commands
# and stops; every mixin helper prints a line starting `fnr:`.
set -euo pipefail
launcher=$(realpath "$1")
sdk=$(realpath "$2")
scratch=$3
resources=${4:-}
if [ -n "$resources" ]; then resources=$(realpath "$resources"); fi
side=${FNR_SIDE:-server}
case "$side" in server|client) ;; *) echo "FNR_SIDE must be server or client" >&2; exit 2;; esac
root=$(git rev-parse --show-toplevel)
# The game revision is the SDK's: its source is what a patched function is copied out of.
revision=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['commit'])" "$2/horizon-prebuilt.json")
mixins=$(sed -n 's/^pub const MIXINS_REV: &str = "\(.*\)";$/\1/p' "$root/crates/horizon-mod-package/src/index/native.rs")
archive=$(sed -n 's/^pub const PREBUILT_ARCHIVE: &str = "\(.*\)";$/\1/p' "$root/crates/horizon-mod-package/src/index/native.rs")

# FNR_KEEP=1 keeps the launcher's folder (toolchain, horizon-mixins, source): a warm build.
if [ -n "${FNR_KEEP:-}" ]; then rm -rf "$scratch/mods" "$scratch/engine" "$scratch/run" "$scratch/launcher/native/server-builds" "$scratch/launcher/versions"/native-*; else rm -rf "$scratch"; fi
mkdir -p "$scratch"/{home,launcher,data,config,trash,clipboard,desktop,mods}
scratch=$(realpath "$scratch")
data=$scratch/launcher

# The pinned source a patched function is copied out of, from this checkout.
source=$data/native/source/$revision
rm -rf "$source" "$data/native/prebuilt"
mkdir -p "$source"
git -C "$root" archive "$revision" | tar -x -C "$source"
find "$source" -type l -delete
printf '%s' "$revision" > "$source/.horizon-revision"

# The prebuilt game, where the launcher keeps a downloaded one.
host=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p "$data/native/prebuilt"
sdk_suffix=
if [ "$side" = server ]; then sdk_suffix=-server; fi
cp -a "$sdk" "$data/native/prebuilt/$revision-$archive$host$sdk_suffix"

base=$data/versions/ci
mkdir -p "$base/.local/horizon-client/26.3"
touch "$base/.installed"
echo '{}' > "$base/.local/horizon-client/26.3/horizon-resources.json"
version=$data/versions/native-ci
mkdir -p "$version"
touch "$version/.installed"
printf '{"base": "ci", "horizon": "%s", "target": "%s"}\n' "$revision" "$side" > "$version/native-mods.json"

mods=$scratch/mods/fnr
mkdir -p "$mods"
for mod in ${FNR_MODS:-alpha beta}; do
  cp -r "$root/tools/ci/fn-replace-check/$mod" "$mods/fnr_$mod"
  sed -i "s/^horizon = \"HEAD\"$/horizon = \"$revision\"/; s/^mixins = \"MIXINS\"$/mixins = \"$mixins\"/" "$mods/fnr_$mod/horizon-mod.toml"
  sed -i "s/rev = \"HEAD\"/rev = \"$revision\"/" "$mods/fnr_$mod/Cargo.toml"
done
# The game library: libhorizon_mod_host.so / .dylib, or horizon_mod_host.dll on Windows.
game_library() { ls "$sdk"/deps/*horizon_mod_host.so "$sdk"/deps/*horizon_mod_host.dylib "$sdk"/deps/*horizon_mod_host.dll 2>/dev/null | head -1; }
game_hash=$(sha256sum "$(game_library)" | cut -d' ' -f1)

env_vars=(HORIZON_HOME="$scratch/home" HORIZON_LAUNCHER_HOME="$data" XDG_DATA_HOME="$scratch/data"
  XDG_CONFIG_HOME="$scratch/config" HORIZON_LAUNCHER_NO_SELF_UPDATE=1 HORIZON_NO_AUDIO=1
  HORIZON_LAUNCHER_TRASH="$scratch/trash" HORIZON_LAUNCHER_CLIPBOARD="$scratch/clipboard"
  HORIZON_LAUNCHER_DESKTOP="$scratch/desktop")
started=$(date +%s.%N)
output_args=()
if [ "$side" = server ]; then output_args=(--server-output "$scratch/engine"); fi
env "${env_vars[@]}" "$launcher" native-build "$mods" "${output_args[@]}" --version native-ci --profile optimized \
  --trust-native-code
if [ "$side" = client ]; then
  binary=$(find "$data/versions" -type f -name horizon-client | head -1)
  test -n "$binary"
  cp -a "$(dirname "$binary")" "$scratch/engine"
  cp "$scratch/engine/horizon-client" "$scratch/engine/horizon-engine"
fi
echo "native build: $(python3 -c "print(round($(date +%s.%N) - $started, 1))") s"
test -x "$scratch/engine/horizon-engine" || test -x "$scratch/engine/horizon-engine.exe"
log=$data/native/build.log
grep -F 'Function replacement:' "$log"
# The prebuilt route, and no game source compiled.
grep -F '"route": "prebuilt"' "$data"/versions/*/native-mods.json > /dev/null || grep -rF 'prebuilt' "$log" > /dev/null
test ! -d "$data/native/stage/crates/horizon-client" || { echo "the game was staged for a source build"; exit 1; }
test "$(sha256sum "$(game_library)" | cut -d' ' -f1)" = "$game_hash"

if [ -n "$resources" ]; then
  run=$scratch/run
  mkdir -p "$run"
  echo eula=true > "$run/eula.txt"
  (sleep 30; echo "tick rate 40"; sleep 3; echo "tick rate 13"; sleep 3; echo "tick query"; sleep 3; echo "stop"; sleep 20) |
    (cd "$run" && env "${env_vars[@]}" timeout 180 "$scratch/engine/horizon-engine" --resources "$(realpath "$resources")" \
      --dedicated --nogui) > "$scratch/server.log" 2>&1 || true
  grep -F 'fnr:' "$scratch/server.log"
  if [[ " ${FNR_MODS:-alpha beta} " == *" alpha "* ]]; then
    grep -F 'OVERWRITE by fnr_alpha' "$scratch/server.log"
  fi
  if [[ " ${FNR_MODS:-alpha beta} " == *" fdshape "* ]]; then
    grep -F 'fnr: native_server_started dedicated=true' "$scratch/server.log"
  fi
fi
