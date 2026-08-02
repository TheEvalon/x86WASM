//! Custom reset ROM that prints `HELLO FROM EMULATOR` via COM1 and port 0x402.
//!
//! Assembled by hand for the Milestone 1 opcode subset (Intel SDM Vol. 2 encodings).
//! Layout: 64 KiB ROM at `0xFFFF_0000`; near JMP at `0xFFF0` to offset `0`.
//!
//! String bytes live in the ROM, so loads use a CS segment override (`0x2E`).

pub const EXPECTED_HELLO: &str = "HELLO FROM EMULATOR";

/// Build a 64 KiB ROM image with code at offset 0 and reset vector at 0xFFF0.
pub fn build_hello_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 64 * 1024];

    // 0000 FA             cli
    // 0001 BB 17 00       mov bx, msg
    // 0004 2E 8A 07       L: mov al, cs:[bx]
    // 0007 84 C0          test al, al
    // 0009 74 0B          jz done
    // 000B BA F8 03       mov dx, 0x3F8
    // 000E EE             out dx, al
    // 000F BA 02 04       mov dx, 0x402
    // 0012 EE             out dx, al
    // 0013 43             inc bx
    // 0014 EB EE          jmp L
    // 0016 F4             done: hlt
    // 0017 msg            db "HELLO FROM EMULATOR", 0
    let code: &[u8] = &[
        0xFA, // cli
        0xBB, 0x17, 0x00, // mov bx, 0x0017
        0x2E, 0x8A, 0x07, // mov al, cs:[bx]
        0x84, 0xC0, // test al, al
        0x74, 0x0B, // jz done (0x16)
        0xBA, 0xF8, 0x03, // mov dx, 0x3F8
        0xEE, // out dx, al
        0xBA, 0x02, 0x04, // mov dx, 0x402
        0xEE, // out dx, al
        0x43, // inc bx
        0xEB, 0xEE, // jmp L (0x04)
        0xF4, // hlt
    ];
    assert_eq!(code.len(), 0x17);
    rom[..code.len()].copy_from_slice(code);
    let msg = EXPECTED_HELLO.as_bytes();
    rom[0x17..0x17 + msg.len()].copy_from_slice(msg);
    rom[0x17 + msg.len()] = 0;

    // Reset vector at 0xFFF0: JMP rel16 to 0
    // next IP = 0xFFF3; rel16 = 0x0000 - 0xFFF3 = 0x000D (mod 2^16)
    rom[0xFFF0] = 0xE9;
    rom[0xFFF1] = 0x0D;
    rom[0xFFF2] = 0x00;

    rom
}

#[cfg(test)]
mod tests {
    use super::*;
    use x86_decode::decode;

    #[test]
    fn reset_vector_decodes_as_jmp() {
        let rom = build_hello_rom();
        let d = decode(&rom[0xFFF0..0xFFF0 + 3]).unwrap();
        assert_eq!(d.opcode, 0xE9);
        assert_eq!(d.immediate, 0x0D);
    }

    #[test]
    fn msg_bytes_present() {
        let rom = build_hello_rom();
        assert_eq!(
            &rom[0x17..0x17 + EXPECTED_HELLO.len()],
            EXPECTED_HELLO.as_bytes()
        );
    }

    #[test]
    fn cs_override_mov_decodes() {
        let rom = build_hello_rom();
        let d = decode(&rom[0x04..0x04 + 3]).unwrap();
        assert_eq!(d.opcode, 0x8A);
        assert_eq!(d.prefixes.segment_override, Some(0x2E));
        assert_eq!(d.length, 3);
    }
}
