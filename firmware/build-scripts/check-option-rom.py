#!/usr/bin/env python3
"""Validate a PC option ROM image header.

An option ROM starts with the bytes 55 AA, stores its size in 512-byte blocks
at offset 2, and the declared extent must checksum to zero modulo 256 (IBM PC
option ROM convention, also stated in the PCI Firmware Specification for
legacy-compatible ROMs). Used by build-seavgabios.sh before installing a build
output, and usable by hand on any ROM image.

Usage: check-option-rom.py IMAGE
Exit status: 0 when the image is a well-formed option ROM, 1 otherwise.
"""

import sys

BLOCK_BYTES = 512


def check(path: str) -> int:
    try:
        with open(path, "rb") as handle:
            data = handle.read()
    except OSError as err:
        print(f"{path}: {err}", file=sys.stderr)
        return 1

    if len(data) < 3 or data[0] != 0x55 or data[1] != 0xAA:
        print(f"{path}: missing 55 AA option ROM signature", file=sys.stderr)
        return 1

    blocks = data[2]
    declared = blocks * BLOCK_BYTES
    if declared == 0:
        print(f"{path}: option ROM size byte is 0", file=sys.stderr)
        return 1
    if declared > len(data):
        print(
            f"{path}: size byte claims {declared} bytes, image is {len(data)}",
            file=sys.stderr,
        )
        return 1

    checksum = sum(data[:declared]) & 0xFF
    if checksum != 0:
        print(
            f"{path}: option ROM checksum is {checksum:#04x}, expected 0x00",
            file=sys.stderr,
        )
        return 1

    print(
        f"{path}: option ROM ok - file={len(data)} bytes, "
        f"declared_map={declared} bytes ({blocks}×{BLOCK_BYTES}), "
        f"checksum=0"
        + (
            f", trailing_unmapped={len(data) - declared}"
            if len(data) > declared
            else ""
        )
    )
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    return check(argv[1])


if __name__ == "__main__":
    sys.exit(main(sys.argv))
