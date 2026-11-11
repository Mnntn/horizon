#!/bin/sh
# Builds the spike against the SDK folder alone, as the launcher builds a native mod: rustc, no Cargo,
# no game source. Prints each step's wall time.
set -e
SDK=$PWD/prebuilt; D=$SDK/deps; OUT=$PWD/out; mkdir -p $OUT
ext() { f=$(ls $D/lib$1.so 2>/dev/null || ls $D/lib$1-*.rmeta); echo "--extern $1=$f"; }
C="--edition 2024 -C prefer-dynamic -C opt-level=3 -L dependency=$D -L native=$SDK/native -L dependency=$OUT"
t() { s=$(date +%s.%N); "$@"; e=$(date +%s.%N); echo "  $(python3 -c "print(round($e - $s,2))") s: $(echo "$@" | grep -o 'crate-name [a-z_0-9]*')" >&2; }
t rustc $C --crate-type rlib --crate-name fnr_autosave $(ext mod_api) --out-dir $OUT mod/fnr_autosave.rs
t rustc $C --crate-type rlib --crate-name fnr_patch $(ext horizon_client) $(ext mod_points) --extern fnr_autosave=$OUT/libfnr_autosave.rlib --out-dir $OUT patched/fnr_patch/src/lib.rs
LINK=$(python3 -c "import json;print(' '.join('-C link-arg='+a for a in json.load(open('$SDK/horizon-prebuilt.json'))['link_args']))")
t rustc $C --crate-type bin --crate-name horizon_client_spike $LINK $(ext horizon_client) $(ext mod_api) $(ext mod_points) $(ext horizon_mod_package) --extern horizon_mod_host=$D/libhorizon_mod_host.so --extern fnr_autosave=$OUT/libfnr_autosave.rlib --extern fnr_patch=$OUT/libfnr_patch.rlib -o $OUT/horizon-client main.rs
