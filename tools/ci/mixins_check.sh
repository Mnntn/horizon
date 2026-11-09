#!/usr/bin/env bash
# Build API and area-file slots plus a unified internals source listener at this exact head.
# Force the latter off the compiled slot list in the disposable source copy, as the
# native source regression does; assert the logged route after the real build.
# Compile-only: no game runs and no vanilla data is read.
#   tools/ci/mixins_check.sh LAUNCHER SCRATCH
set -euo pipefail
launcher=$(realpath "$1")
scratch=$2
root=$(git rev-parse --show-toplevel)
revision=$(git -C "$root" rev-parse HEAD)

rm -rf "$scratch"
mkdir -p "$scratch"/{home,launcher,data,config,trash,clipboard,desktop}
scratch=$(realpath "$scratch")
data=$scratch/launcher

# The source the launcher would download for `revision`, from this checkout instead: a pull
# request's merge commit is in no archive GitHub serves.
source=$data/native/source/$revision
mkdir -p "$source"
git -C "$root" archive HEAD | tar -x -C "$source"
# The launcher's own download leaves links out (horizon-mixins refuses them).
find "$source" -type l -delete
printf '%s' "$revision" > "$source/.horizon-revision"

# What a player's launcher holds before this build: an installed, prepared base version `ci`,
# and a native build of `revision` on it, so the launcher takes the commit from the build instead
# of asking GitHub about it. Native builds read their base's prepared resources and carry none of
# their own. The server export reads none of its game files, so they are an empty manifest.
base=$data/versions/ci
mkdir -p "$base/.local/horizon-client/26.3"
touch "$base/.installed"
echo '{}' > "$base/.local/horizon-client/26.3/horizon-resources.json"
version=$data/versions/native-ci
mkdir -p "$version"
touch "$version/.installed"
printf '{"base": "ci", "horizon": "%s", "target": "server"}\n' "$revision" > "$version/native-mods.json"

mod=$scratch/mixins_check
cp -r "$root/tools/ci/mixins-check" "$mod"
sed -i "s/^horizon = \"HEAD\"$/horizon = \"$revision\"/" "$mod/horizon-mod.toml"
sed -i "s/rev = \"HEAD\"/rev = \"$revision\"/" "$mod/Cargo.toml"

# inline(always) excludes this function from compiled slots, but native source lowering
# can still wrap it. Never change the real checkout or rely on an accidentally missing slot.
python3 - "$source/crates/horizon-client/src/game.rs" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
text = p.read_text()
needle = '    fn tick_mobs(&mut self) -> Result<()> {'
assert text.count(needle) == 1
p.write_text(text.replace(needle, '    #[inline(always)]\n' + needle))
PY
cp -a "$source" "$scratch/catalog-source"
CARGO_TARGET_DIR=$scratch/sdk-target cargo run --quiet --manifest-path "$source/Cargo.toml" \
  -p horizon-mod-api-gen --bin horizon-stage -- "$scratch/catalog-source" \
  "$scratch/points.json" "$scratch/internals.json" --sync-internals
python3 - "$scratch" "$mod/horizon-mod.toml" <<'PY'
import json
from pathlib import Path
import sys
scratch, manifest = map(Path, sys.argv[1:])
rows = json.loads((scratch/'internals.json').read_text())
points = json.loads((scratch/'points.json').read_text())['points']
target = "horizon_client::game::Game<'_>::tick_mobs"
area = "horizon_client::game::using_item::Game<'_>::start_eating"
assert not any(row['target'] == target for row in points)
# at_return describes API listeners only. This internals injection instead
# requires the typed raw return listener (start_eating takes engine ItemInfo).
assert any(row['target'] == area and row['raw'] and row.get('return_listener') for row in points)
text = manifest.read_text()
for token, path, kind in [('GAME_HASH', 'horizon_client::game::Game', 'struct'),
                          ('TICK_HASH', "horizon_client::game::Game<'_>::tick", 'method'),
                          ('COUNT_HASH', target, 'method'),
                          ('AREA_HASH', area, 'method')]:
    matches = [row for row in rows if row['path'] == path and row['kind'] == kind]
    assert len(matches) == 1
    assert text.count(token) == 1
    text = text.replace(token, matches[0]['hash'])
manifest.write_text(text)
PY
unset CARGO_TARGET_DIR
HORIZON_HOME=$scratch/home HORIZON_LAUNCHER_HOME=$data XDG_DATA_HOME=$scratch/data \
  XDG_CONFIG_HOME=$scratch/config HORIZON_LAUNCHER_NO_SELF_UPDATE=1 HORIZON_NO_AUDIO=1 \
  HORIZON_LAUNCHER_TRASH=$scratch/trash HORIZON_LAUNCHER_CLIPBOARD=$scratch/clipboard \
  HORIZON_LAUNCHER_DESKTOP=$scratch/desktop \
  "$launcher" native-build "$mod" --server-output "$scratch/engine" --version native-ci \
  --profile developer --trust-native-code
test -x "$scratch/engine/horizon-engine"
grep -F 'mixins_check:count:' "$data/native/build.log" | grep -F -- '-> source (private slot,'
grep -F 'mixins_check:area:' "$data/native/build.log" | grep -F -- '-> slot (return, internals)'
grep -F 'mod_0::area).ordered(0, "area")' "$data/native/stage/crates/native-game/src/main.rs"
