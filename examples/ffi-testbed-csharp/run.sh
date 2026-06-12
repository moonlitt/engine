#!/usr/bin/env bash
# Build + run the FFI testbed against the workspace's release dylib.
#
# Usage:
#   ./run.sh              # smoke tests (CI mode)
#   ./run.sh --play       # play the melody (audible)
#   ./run.sh --interactive # D F J K → C D E F (audible)
#
# The dylib must already exist at target/release/libmoonlitt_ffi.dylib.
# If missing, run:
#   cargo build -p moonlitt-capi --release
# from the workspace root first.

set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
WORKSPACE_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
DYLIB_DIR="$WORKSPACE_ROOT/target/release"
DYLIB="$DYLIB_DIR/libmoonlitt_ffi.dylib"

if [[ ! -f "$DYLIB" ]]; then
  echo "error: $DYLIB missing — run 'cargo build -p moonlitt-capi --release' first" >&2
  exit 2
fi

cd "$SCRIPT_DIR"

# DYLD_LIBRARY_PATH is the macOS equivalent of LD_LIBRARY_PATH.
# .NET's DllImport will search here for libmoonlitt_ffi.dylib.
export DYLD_LIBRARY_PATH="$DYLIB_DIR:${DYLD_LIBRARY_PATH:-}"

dotnet run --configuration Release -- "$@"
