#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/ai-browser"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
exec cargo run --release -- "$@"
