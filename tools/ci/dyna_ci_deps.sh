#!/usr/bin/env bash
# Install and cache the Ubuntu runner's build dependencies without root.
set -euo pipefail
# The baked runner image may already supply the exact dependency set. Only the
# fallback prefix needs pkg-config/linker overrides; never apply them to the image.
packages=(pkg-config libasound2-dev libudev-dev libxkbcommon-x11-dev libwayland-dev
          libx11-xcb-dev libfontconfig-dev libvulkan-dev libxcursor1 libxi6 libxrandr2)
baked=true
for package in "${packages[@]}"; do
  if [ "$(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null || true)" != installed ]; then
    baked=false
    break
  fi
done
if "$baked"; then
  echo "Runner dependencies already installed; using system headers and libraries"
  echo "$CARGO_HOME/bin" >> "$GITHUB_PATH"
  echo "RUSTUP_HOME=$DYNA_CI_BUILD/rustup" >> "$GITHUB_ENV"
  exit 0
fi
# The Ubuntu mirror proxy sometimes serves an index or package mid-sync ("File has unexpected size"): a later try
# succeeds. Bounded, with a short growing pause; apt's own error is already on stderr, and after the last try the
# command's exit status is returned, so `set -e` fails the step with that real error.
tries="${DYNA_CI_APT_TRIES:-4}"
pause="${DYNA_CI_APT_PAUSE:-5}"
retry() {
  local attempt=1 status=0
  while :; do
    "$@" && return 0
    status=$?
    [ "$attempt" -lt "$tries" ] || { echo "$1 failed after $attempt tries (exit $status)" >&2; return "$status"; }
    echo "$1 failed (exit $status), try $attempt of $tries; retrying in $((pause * attempt))s" >&2
    sleep "$((pause * attempt))"
    attempt=$((attempt + 1))
  done
}
prefix="$DYNA_CI_BUILD/apt-prefix"
aptroot="$HOME/dyna-ci-apt"
hash=$(sha256sum "$0" | cut -d ' ' -f1)
if [ ! -f "$prefix/.dyna-hash" ] || [ "$(< "$prefix/.dyna-hash")" != "$hash" ]; then
  stage="$DYNA_CI_BUILD/apt-prefix.new"
  rm -rf "$stage" "$aptroot"
  mkdir -p "$aptroot/lists/partial" "$aptroot/archives/partial" "$stage"
  : > "$aptroot/status"
  printf '%s\n' \
    'deb https://archive.ubuntu.com/ubuntu noble main universe' \
    'deb https://archive.ubuntu.com/ubuntu noble-updates main universe' \
    'deb https://security.ubuntu.com/ubuntu noble-security main universe' > "$aptroot/sources.list"
  opts=(-o "Dir::Etc::sourcelist=$aptroot/sources.list" -o Dir::Etc::sourceparts=-
        -o "Dir::State::lists=$aptroot/lists" -o "Dir::State::status=$aptroot/status"
        -o "Dir::Cache::archives=$aptroot/archives" -o APT::Sandbox::User=root)
  retry apt-get "${opts[@]}" update
  retry apt-get "${opts[@]}" -y --download-only --no-install-recommends install \
    "${packages[@]}"
  for deb in "$aptroot"/archives/*.deb; do
    # Build-essential already supplies matching libc, loader and system headers.
    case "${deb##*/}" in libc6_*|libc6-dev_*|linux-libc-dev_*) continue ;; esac
    dpkg-deb -x "$deb" "$stage"
  done
  echo "$hash" > "$stage/.dyna-hash"
  rm -rf "$prefix"
  mv "$stage" "$prefix"
  rm -rf "$aptroot"
fi
arch=x86_64-linux-gnu
{
  echo "$prefix/usr/bin"
  echo "$CARGO_HOME/bin"
} >> "$GITHUB_PATH"
{
  echo "RUSTUP_HOME=$DYNA_CI_BUILD/rustup"
  echo "PKG_CONFIG_SYSROOT_DIR=$prefix"
  echo "PKG_CONFIG_PATH=$prefix/usr/lib/$arch/pkgconfig:$prefix/usr/share/pkgconfig"
  echo "LIBRARY_PATH=$prefix/usr/lib/$arch:$prefix/lib/$arch"
  echo "C_INCLUDE_PATH=$prefix/usr/include:$prefix/usr/include/$arch"
  echo "LD_LIBRARY_PATH=$prefix/usr/lib/$arch:$prefix/lib/$arch"
} >> "$GITHUB_ENV"
