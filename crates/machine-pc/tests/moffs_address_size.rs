//! Reproducer for the CPU-side defect behind the `0xF0000000` write sweep.
//!
//! **These tests are `#[ignore]`d because they fail today, and the fix is in
//! `crates/x86-interpreter`, which this round's memory-semantics slice does not
//! own.** Un-ignore them with the fix; see `docs/machine-r4-fseg-sweep.md` for
//! the measurement that led here.
//!
//! The defect: `x86_interpreter::moffs_offset` picks the width of the `MOV`
//! absolute-offset forms (`A0`–`A3`) from the *presence of the `0x67` prefix*
//! rather than from the resolved effective address-size attribute. In a
//! `CS.D = 1` code segment the default address size is 32, so an unprefixed
//! `A1` carries a 32-bit `moffs32` — but the interpreter truncates the offset
//! it already decoded to 16 bits and reads the wrong address. The decoder is
//! correct: it consumes four immediate bytes, so only the address is wrong.
//!
//! Spec: Intel SDM Vol. 1 §3.6 Table 3-4 (effective address size = the
//! code-segment default `D`, inverted by `67H`); Vol. 2 "MOV" (the `moffs8` /
//! `moffs16` / `moffs32` operand is sized by the address-size attribute);
//! Vol. 3 §3.4.5 (the `D` flag).

use machine_pc::Machine;

/// Offset of the GDT inside the test image (linear, via the `0xF0000` alias).
const GDT_LINEAR: u32 = 0x000F_0040;
/// Where the guest's 32-bit `MOV EAX, moffs32` reads from.
const PROBE_ADDR: u32 = 0x0002_0000;
const PROBE_VALUE: u32 = 0xDEAD_BEEF;
/// What a 16-bit-truncated `moffs` would read instead (`PROBE_ADDR as u16`).
const DECOY_VALUE: u32 = 0x1122_3344;

/// Build a 64 KiB BIOS image that enters a flat `CS.D=1` ring-0 segment and
/// then runs `tail32`.
///
/// Spec: Intel SDM Vol. 3 §9.1.4 (reset vector), §2.4.1 (`LGDT`), §9.9.1
/// (switching to protected mode: set `CR0.PE`, then a far jump loads `CS`).
fn protected_mode_image(tail32: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xF4u8; 64 * 1024];

    #[rustfmt::skip]
    let entry16: &[u8] = &[
        0xFA,                                     // CLI
        0x2E, 0x0F, 0x01, 0x16, 0x58, 0x00,       // LGDT CS:[0x0058]
        0x0F, 0x20, 0xC0,                         // MOV EAX, CR0
        0x0C, 0x01,                               // OR AL, 1
        0x0F, 0x22, 0xC0,                         // MOV CR0, EAX
        0x66, 0xEA,                               // JMP FAR ptr16:32 ...
        0x20, 0x00, 0x0F, 0x00,                   // ... offset 0x000F0020
        0x08, 0x00,                               // ... selector 0x0008
    ];
    rom[..entry16.len()].copy_from_slice(entry16);
    rom[0x0020..0x0020 + tail32.len()].copy_from_slice(tail32);

    // GDT: null, flat 32-bit ring-0 code (`D=1`, `G=1`), flat 32-bit data.
    #[rustfmt::skip]
    let gdt: &[u8] = &[
        0, 0, 0, 0, 0, 0, 0, 0,
        0xFF, 0xFF, 0x00, 0x00, 0x00, 0x9A, 0xCF, 0x00,
        0xFF, 0xFF, 0x00, 0x00, 0x00, 0x92, 0xCF, 0x00,
    ];
    rom[0x0040..0x0040 + gdt.len()].copy_from_slice(gdt);

    // GDTR image: limit then 32-bit base.
    rom[0x0058..0x005A].copy_from_slice(&((gdt.len() - 1) as u16).to_le_bytes());
    rom[0x005A..0x005E].copy_from_slice(&GDT_LINEAR.to_le_bytes());

    // Reset vector: far jump to F000:0000.
    rom[0xFFF0..0xFFF5].copy_from_slice(&[0xEA, 0x00, 0x00, 0x00, 0xF0]);
    rom
}

fn run_protected_tail(tail32: &[u8]) -> Machine {
    let mut m =
        Machine::with_bios_rom(1024 * 1024, &protected_mode_image(tail32)).expect("map BIOS");
    for (index, byte) in PROBE_VALUE.to_le_bytes().iter().enumerate() {
        m.mem
            .write_u8(u64::from(PROBE_ADDR) + index as u64, *byte)
            .unwrap();
    }
    for (index, byte) in DECOY_VALUE.to_le_bytes().iter().enumerate() {
        m.mem
            .write_u8(u64::from(PROBE_ADDR as u16) + index as u64, *byte)
            .unwrap();
    }
    let report = m.probe_post(10_000);
    assert!(m.cpu.halted, "guest did not reach HLT: {report}");
    m
}

/// `A1` with no `67H` in a `CS.D=1` segment is `MOV EAX, moffs32`.
///
/// This is the exact instruction SeaBIOS uses to read its f-segment globals
/// (`A1 40 0B 0F 00` = `MOV EAX, ds:0x000F0B40`), and reading the wrong address
/// is what makes its memory-zone list look empty. See
/// `docs/machine-r4-fseg-sweep.md`.
#[test]
#[ignore = "fails until x86-interpreter's moffs_offset uses the resolved address size"]
fn moffs32_is_the_default_in_a_32_bit_code_segment() {
    #[rustfmt::skip]
    let tail32: &[u8] = &[
        0xB8, 0x10, 0x00, 0x00, 0x00,             // MOV EAX, 0x10
        0x8E, 0xD8,                               // MOV DS, AX
        0xA1, 0x00, 0x00, 0x02, 0x00,             // MOV EAX, ds:0x00020000
        0xF4,                                     // HLT
    ];
    let m = run_protected_tail(tail32);
    assert_eq!(
        m.cpu.gpr[0] as u32, PROBE_VALUE,
        "MOV EAX, moffs32 read the wrong address; {:#010X} is what a 16-bit \
         truncation of the offset would return",
        DECOY_VALUE
    );
}

/// The mirror case, so a fix cannot simply invert the condition: `67H` in a
/// `CS.D=1` segment selects a **16-bit** `moffs16`. This one passes today
/// (the decoder already sizes the immediate correctly, and both the right and
/// the wrong rule produce the same offset here), and it is not ignored so that
/// it guards the fix rather than waiting on it.
///
/// Spec: Intel SDM Vol. 1 §3.6 Table 3-4 — `67H` toggles the default, it does
/// not select 32 unconditionally.
#[test]
fn address_size_override_selects_moffs16_in_a_32_bit_code_segment() {
    #[rustfmt::skip]
    let tail32: &[u8] = &[
        0xB8, 0x10, 0x00, 0x00, 0x00,             // MOV EAX, 0x10
        0x8E, 0xD8,                               // MOV DS, AX
        0x67, 0xA1, 0x00, 0x00,                   // MOV EAX, ds:0x0000 (moffs16)
        0xF4,                                     // HLT
    ];
    let m = run_protected_tail(tail32);
    assert_eq!(
        m.cpu.gpr[0] as u32, DECOY_VALUE,
        "67H in a D=1 segment must select a 16-bit absolute offset"
    );
}
