#!/usr/bin/env bash
# Build + run the FFI testbed against the workspace's release dylib.
#
# Usage:
#   ./run.sh              # smoke tests (CI mode)
#   ./run.sh --play       # play the melody (audible)
#   ./run.sh --interactive # D F J K → C D E F (audible)
#
# Always rebuilds the dylib first — testing a stale binary is worse than
# the few seconds an incremental release build costs (we got burned by a
# 2.5-month-old leftover dylib silently satisfying DllImport).

set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
WORKSPACE_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
DYLIB_DIR="$WORKSPACE_ROOT/target/release"

echo "[run.sh] cargo build -p moonlitt-capi --release"
(cd "$WORKSPACE_ROOT" && cargo build -p moonlitt-capi --release)

cd "$SCRIPT_DIR"

# DYLD_LIBRARY_PATH is the macOS equivalent of LD_LIBRARY_PATH.
# .NET's DllImport will search here for libmoonlitt.dylib.
export DYLD_LIBRARY_PATH="$DYLIB_DIR:${DYLD_LIBRARY_PATH:-}"

dotnet run --configuration Release -- "$@"
