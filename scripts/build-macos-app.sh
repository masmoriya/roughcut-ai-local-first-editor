#!/usr/bin/env bash
# Build RoughCut.app locally, the same way .github/workflows/release.yml does,
# signed so macOS per-app grants survive rebuilds.
#
# macOS ties per-app grants to the app's code signature, such as access to the
# Keychain item that holds the Hosted-mode API key. An ad-hoc signature changes
# on every build, so macOS asks again after each one. When a "RoughCut Local
# Signing" code-signing identity is in your keychain (or APPLE_SIGNING_IDENTITY
# names another one), the app is signed with it, so its signature stays the
# same and grants persist. Without one, the build falls back to ad-hoc signing.
#
#   scripts/build-macos-app.sh            # → target/release/bundle/macos/RoughCut.app
#   scripts/build-macos-app.sh --bundles app,dmg
set -euo pipefail

cd "$(dirname "$0")/../app/src-tauri"

args=(--bundles app)
[[ " $* " == *" --bundles "* ]] && args=()

identity="${APPLE_SIGNING_IDENTITY:-RoughCut Local Signing}"
if security find-identity -p codesigning | grep -qF "\"$identity\""; then
  export APPLE_SIGNING_IDENTITY="$identity"
  # Keep hardened runtime off, as in ad-hoc builds; only notarized releases
  # need it.
  args+=(--config '{"bundle":{"macOS":{"hardenedRuntime":false}}}')
  echo "Signing with \"$identity\"" >&2
else
  echo "No \"$identity\" signing identity found; building ad-hoc (per-app grants reset on every build)." >&2
fi

locked_args=(-- --locked)
for arg in "$@"; do
  if [[ "$arg" == -- ]]; then
    locked_args=(--locked)
    break
  fi
done

# ${args[@]+...}: an empty array trips `set -u` in macOS's bash 3.2.
if command -v cargo-tauri >/dev/null; then
  cargo tauri build ${args[@]+"${args[@]}"} "$@" "${locked_args[@]}"
else
  echo 'Tauri CLI is missing. Install the reviewed CLI before building.' >&2
  exit 1
fi
