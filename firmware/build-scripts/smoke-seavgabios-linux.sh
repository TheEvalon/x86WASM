#!/usr/bin/env bash
# SeaVGABIOS Linux/WSL smoke — verify the build path without vendoring LGPL
# sources into Rust crates.
#
# Modes:
#   --preflight (default)  Check scripts, pin, notices, and header validator
#                          against a synthetic option ROM (no network/gcc).
#   --build                Also run build-seavgabios.sh (Linux/WSL only).
#
# Windows native hosts: --preflight only. Full builds remain Linux/WSL2
# (see docs/firmware-r9-seavgabios-linux-smoke.md). Exit 0 on preflight success
# even when --build is refused on non-Linux, unless SEAVGABIOS_SMOKE_REQUIRE_BUILD=1.
#
# Optional CI note: path-filter ubuntu jobs that already build SeaBIOS can add
#   ./firmware/build-scripts/smoke-seavgabios-linux.sh --build
# after gcc-multilib is installed. Do not commit vgabios.bin without review.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
MODE=preflight
REQUIRE_BUILD="${SEAVGABIOS_SMOKE_REQUIRE_BUILD:-0}"

log() { printf '+ %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --preflight) MODE=preflight; shift ;;
    --build) MODE=build; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
done

is_linux_or_wsl() {
  case "$(uname -s 2>/dev/null || echo unknown)" in
    Linux*) return 0 ;;
    *) return 1 ;;
  esac
}

preflight() {
  log "preflight SeaVGABIOS smoke (root=${ROOT})"
  [[ -f "${SCRIPT_DIR}/build-seavgabios.sh" ]] || die "missing build-seavgabios.sh"
  [[ -f "${SCRIPT_DIR}/check-option-rom.py" ]] || die "missing check-option-rom.py"
  [[ -f "${ROOT}/firmware/manifests/seavgabios.json" ]] || die "missing seavgabios.json pin"
  [[ -f "${ROOT}/firmware/seavgabios/LICENSE.notice" ]] || die "missing LICENSE.notice"
  [[ -f "${ROOT}/docs/firmware-r7-seavgabios-build.md" ]] || die "missing R7 build doc"
  [[ -f "${ROOT}/docs/firmware-r9-seavgabios-linux-smoke.md" ]] || die "missing R9 smoke doc"

  # Prefer PYTHON override (Git Bash on Windows often has a Store stub for python3).
  local py="${PYTHON:-}"
  if [[ -z "${py}" ]]; then
    if command -v python3 >/dev/null 2>&1 && python3 -c 'import sys' >/dev/null 2>&1; then
      py=python3
    elif command -v python >/dev/null 2>&1 && python -c 'import sys' >/dev/null 2>&1; then
      py=python
    else
      die "python3/python required for check-option-rom.py (set PYTHON=...)"
    fi
  fi
  command -v "${py}" >/dev/null 2>&1 || [[ -x "${py}" ]] || die "PYTHON interpreter not found: ${py}"

  # Synthetic 512-byte option ROM: 55 AA, size=1, RETF at offset 3, checksum 0.
  local tmp
  tmp="$(mktemp)"
  "${py}" - "$tmp" <<'PY'
import sys
path = sys.argv[1]
rom = bytearray(512)
rom[0], rom[1], rom[2], rom[3] = 0x55, 0xAA, 1, 0xCB
rom[-1] = (-sum(rom[:-1])) & 0xFF
open(path, "wb").write(rom)
PY
  "${py}" "${SCRIPT_DIR}/check-option-rom.py" "${tmp}"
  rm -f "${tmp}"

  # Pin fields present.
  "${py}" - "${ROOT}/firmware/manifests/seavgabios.json" <<'PY'
import sys
text = open(sys.argv[1], encoding="utf-8").read()
for needle in ("rel-1.16.3", "a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8"):
    if needle not in text:
        raise SystemExit(f"pin missing {needle}")
print("pin ok")
PY

  log "preflight ok (no LGPL sources touched; crates/ unchanged)"
}

run_build() {
  if ! is_linux_or_wsl; then
    printf 'warning: --build refused on non-Linux host (Windows native infeasible).\n' >&2
    printf '         Use WSL2/Ubuntu or Linux CI. Preflight still passed.\n' >&2
    if [[ "${REQUIRE_BUILD}" == "1" ]]; then
      die "SEAVGABIOS_SMOKE_REQUIRE_BUILD=1 but host is not Linux/WSL"
    fi
    return 0
  fi
  log "running build-seavgabios.sh"
  chmod +x "${SCRIPT_DIR}/build-seavgabios.sh"
  "${SCRIPT_DIR}/build-seavgabios.sh"
  [[ -f "${ROOT}/firmware/seavgabios/vgabios.bin" ]] \
    || die "expected firmware/seavgabios/vgabios.bin after build"
  python3 "${SCRIPT_DIR}/check-option-rom.py" \
    "${ROOT}/firmware/seavgabios/vgabios.bin"
  log "build smoke ok"
}

preflight
if [[ "${MODE}" == "build" ]]; then
  run_build
fi
