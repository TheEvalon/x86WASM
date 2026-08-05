#!/usr/bin/env bash
# Fetch and build SeaBIOS into firmware/seabios/.
#
# Intended hosts:
#   - Linux (native or CI) — preferred
#   - Windows Git Bash / WSL2 when a working i386-capable gcc toolchain is available
#
# This script does NOT vendor SeaBIOS sources into Rust crates. Sources stay under
# firmware/seabios/.src/ (gitignored). Only the binary + manifest land in
# firmware/seabios/ for local use (also gitignored *.bin until licensing review).
#
# Out of scope here: OVMF, SeaBIOS POST in the emulator, SeaVGABIOS option ROM.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
OUT_DIR="${ROOT}/firmware/seabios"
SRC_DIR="${SEABIOS_SRC_DIR:-${OUT_DIR}/.src}"

# Pinned release (override with SEABIOS_REPO / SEABIOS_REF / SEABIOS_COMMIT).
SEABIOS_REPO="${SEABIOS_REPO:-https://gitlab.com/qemu-project/seabios.git}"
SEABIOS_REF="${SEABIOS_REF:-rel-1.16.3}"
# Peel of annotated tag rel-1.16.3 on the QEMU mirror (verify after fetch).
SEABIOS_COMMIT="${SEABIOS_COMMIT:-a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8}"

JOBS="${JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)}"
PYTHON="${PYTHON:-python3}"

log() { printf '+ %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

check_host() {
  need_cmd git
  need_cmd make
  need_cmd "${PYTHON}"
  if ! command -v gcc >/dev/null 2>&1 && ! command -v clang >/dev/null 2>&1; then
    die "need gcc or clang (i386 / -m32 capable toolchain for SeaBIOS)"
  fi
  # SeaBIOS builds 16/32-bit BIOS objects; gcc-multilib or an i686 cross compiler is typical.
  if command -v gcc >/dev/null 2>&1; then
    if ! echo 'int main(void){return 0;}' | gcc -m32 -x c - -o /dev/null 2>/dev/null; then
      printf 'warning: gcc -m32 probe failed; SeaBIOS make may fail without gcc-multilib or a cross toolchain.\n' >&2
      printf '         Linux: sudo apt-get install build-essential gcc-multilib python3\n' >&2
      printf '         Windows: use WSL2/Ubuntu or Linux CI (see firmware/README.md).\n' >&2
    fi
  fi
}

fetch_sources() {
  mkdir -p "${OUT_DIR}"
  if [[ ! -d "${SRC_DIR}/.git" ]]; then
    log "clone ${SEABIOS_REPO} -> ${SRC_DIR}"
    git clone --filter=blob:none "${SEABIOS_REPO}" "${SRC_DIR}"
  else
    log "fetch updates in ${SRC_DIR}"
    git -C "${SRC_DIR}" fetch --tags --force origin
  fi

  log "checkout ${SEABIOS_REF} (${SEABIOS_COMMIT})"
  git -C "${SRC_DIR}" checkout --detach "${SEABIOS_COMMIT}"

  local head
  head="$(git -C "${SRC_DIR}" rev-parse HEAD)"
  [[ "${head}" == "${SEABIOS_COMMIT}" ]] || die "HEAD ${head} != pinned ${SEABIOS_COMMIT}"
}

build_seabios() {
  log "make -j${JOBS} (PYTHON=${PYTHON})"
  # Default SeaBIOS config targets QEMU-style out/bios.bin.
  make -C "${SRC_DIR}" -j"${JOBS}" PYTHON="${PYTHON}"
  [[ -f "${SRC_DIR}/out/bios.bin" ]] || die "expected ${SRC_DIR}/out/bios.bin after make"
}

install_artifacts() {
  local bios_src="${SRC_DIR}/out/bios.bin"
  local bios_dst="${OUT_DIR}/bios.bin"
  local size sha256 copied_at

  log "install ${bios_dst}"
  cp -f "${bios_src}" "${bios_dst}"

  if [[ -f "${SRC_DIR}/COPYING" ]]; then
    cp -f "${SRC_DIR}/COPYING" "${OUT_DIR}/COPYING.SeaBIOS"
  elif [[ -f "${SRC_DIR}/LICENSE" ]]; then
    cp -f "${SRC_DIR}/LICENSE" "${OUT_DIR}/COPYING.SeaBIOS"
  fi

  size="$(wc -c <"${bios_dst}" | tr -d ' ')"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256="$(sha256sum "${bios_dst}" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    sha256="$(shasum -a 256 "${bios_dst}" | awk '{print $1}')"
  else
    sha256="unavailable"
  fi
  copied_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

  cat >"${OUT_DIR}/manifest.json" <<EOF
{
  "name": "seabios",
  "component": "bios.bin",
  "repo": "${SEABIOS_REPO}",
  "ref": "${SEABIOS_REF}",
  "commit": "${SEABIOS_COMMIT}",
  "license": "LGPL-3.0-or-later (see COPYING.SeaBIOS / upstream)",
  "built_at_utc": "${copied_at}",
  "size_bytes": ${size},
  "sha256": "${sha256}",
  "notes": "Build artifact only. Not vendored into MIT/Apache crates. SeaBIOS POST not integrated."
}
EOF

  # Shared pin record (no binary) for docs / CI.
  mkdir -p "${ROOT}/firmware/manifests"
  cat >"${ROOT}/firmware/manifests/seabios.json" <<EOF
{
  "name": "seabios",
  "component": "bios.bin",
  "repo": "${SEABIOS_REPO}",
  "ref": "${SEABIOS_REF}",
  "commit": "${SEABIOS_COMMIT}",
  "license": "LGPL-3.0-or-later",
  "output_path": "firmware/seabios/bios.bin",
  "build_script": "firmware/build-scripts/build-seabios.sh",
  "notes": "Pinned revision for reproducible builds. Binary is gitignored; rebuild locally or in Linux CI."
}
EOF

  log "wrote ${OUT_DIR}/manifest.json (${size} bytes, sha256=${sha256})"
  log "wrote ${ROOT}/firmware/manifests/seabios.json"
}

main() {
  check_host
  fetch_sources
  build_seabios
  install_artifacts
  log "SeaBIOS build complete -> ${OUT_DIR}/bios.bin"
  log "Map with Machine::load_bios_rom / with_bios_rom when ready (POST not in this slice)."
}

main "$@"
