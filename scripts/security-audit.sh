#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
if (($# == 0)); then
  set -- all
fi
exec python3 scripts/security_audit.py "$@"
