//! Legacy-prefix + primary-opcode decoder for the Milestone 1 subset.
//!
//! Spec: Intel SDM Vol. 2, Chapter 2 (instruction format).

#![forbid(unsafe_code)]

use thiserror::Error;
use x86_spec::{lookup_primary, Encoding};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrefixState {
    pub op_size_override: bool,
    pub addr_size_override: bool,
    pub segment_override: Option<u8>,
    pub lock: bool,
    pub rep: bool,
    pub repne: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modrm {
    pub mod_: u8,
    pub reg: u8,
    pub rm: u8,
    pub raw: u8,
}

impl Modrm {
    pub fn decode(raw: u8) -> Self {
        Self {
            mod_: (raw >> 6) & 3,
            reg: (raw >> 3) & 7,
            rm: raw & 7,
            raw,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedInsn {
    pub prefixes: PrefixState,
    pub opcode: u8,
    pub modrm: Option<Modrm>,
    pub displacement: i32,
    pub immediate: i32,
    pub length: usize,
    pub mnemonic: &'static str,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("truncated instruction")]
    Truncated,
    #[error("unsupported opcode 0x{0:02X}")]
    UnsupportedOpcode(u8),
    #[error("instruction too long")]
    TooLong,
}

fn is_legacy_prefix(b: u8) -> bool {
    matches!(
        b,
        0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3
    )
}

/// Decode one instruction from `bytes` (max 15 bytes per SDM).
pub fn decode(bytes: &[u8]) -> Result<DecodedInsn, DecodeError> {
    if bytes.is_empty() {
        return Err(DecodeError::Truncated);
    }
    let mut i = 0usize;
    let mut prefixes = PrefixState::default();
    while i < bytes.len() && is_legacy_prefix(bytes[i]) {
        match bytes[i] {
            0x66 => prefixes.op_size_override = true,
            0x67 => prefixes.addr_size_override = true,
            0xF0 => prefixes.lock = true,
            0xF3 => prefixes.rep = true,
            0xF2 => prefixes.repne = true,
            s @ (0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65) => prefixes.segment_override = Some(s),
            _ => {}
        }
        i += 1;
        if i > 14 {
            return Err(DecodeError::TooLong);
        }
    }
    if i >= bytes.len() {
        return Err(DecodeError::Truncated);
    }
    let opcode = bytes[i];
    i += 1;
    let def = lookup_primary(opcode).ok_or(DecodeError::UnsupportedOpcode(opcode))?;

    let mut modrm = None;
    let mut displacement = 0i32;
    let mut immediate = 0i32;

    let needs_modrm = matches!(
        def.encoding,
        Encoding::Modrm | Encoding::ModrmImm8 | Encoding::ModrmImm16
    ) || matches!(
        opcode,
        0x01 | 0x03
            | 0x09
            | 0x29
            | 0x2B
            | 0x31
            | 0x33
            | 0x39
            | 0x3B
            | 0x84
            | 0x85
            | 0x88
            | 0x89
            | 0x8A
            | 0x8B
    );

    if needs_modrm {
        if i >= bytes.len() {
            return Err(DecodeError::Truncated);
        }
        let m = Modrm::decode(bytes[i]);
        i += 1;
        // 16-bit addressing displacements (default real mode, no 0x67).
        let addr16 = !prefixes.addr_size_override;
        if addr16 {
            match (m.mod_, m.rm) {
                (0, 6) => {
                    if i + 1 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    displacement = i16::from_le_bytes([bytes[i], bytes[i + 1]]) as i32;
                    i += 2;
                }
                (1, _) => {
                    if i >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    displacement = bytes[i] as i8 as i32;
                    i += 1;
                }
                (2, _) => {
                    if i + 1 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    displacement = i16::from_le_bytes([bytes[i], bytes[i + 1]]) as i32;
                    i += 2;
                }
                _ => {}
            }
        } else {
            // Minimal 32-bit path: mod=0 rm=5 disp32; mod=1 disp8; mod=2 disp32.
            match m.mod_ {
                0 if m.rm == 5 => {
                    if i + 3 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    displacement =
                        i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
                    i += 4;
                }
                1 => {
                    if i >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    displacement = bytes[i] as i8 as i32;
                    i += 1;
                }
                2 => {
                    if i + 3 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    displacement =
                        i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
                    i += 4;
                }
                _ => {}
            }
        }
        modrm = Some(m);
    }

    // Group 3 TEST (F6/F7 /0 and /1) takes an immediate; other /r forms do not.
    // Spec: Intel SDM Vol. 2 opcode map — F6 /0,/1 ib; F7 /0,/1 iw.
    let grp3_test_imm =
        matches!(opcode, 0xF6 | 0xF7) && matches!(modrm.map(|m| m.reg), Some(0) | Some(1));

    if (0xB0..=0xB7).contains(&opcode) {
        if i >= bytes.len() {
            return Err(DecodeError::Truncated);
        }
        immediate = i32::from(bytes[i]);
        i += 1;
    } else if (0xB8..=0xBF).contains(&opcode) {
        if prefixes.op_size_override {
            if i + 3 >= bytes.len() {
                return Err(DecodeError::Truncated);
            }
            immediate = i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
            i += 4;
        } else {
            if i + 1 >= bytes.len() {
                return Err(DecodeError::Truncated);
            }
            immediate = i32::from(i16::from_le_bytes([bytes[i], bytes[i + 1]]));
            i += 2;
        }
    } else if grp3_test_imm {
        if opcode == 0xF6 {
            if i >= bytes.len() {
                return Err(DecodeError::Truncated);
            }
            immediate = i32::from(bytes[i]);
            i += 1;
        } else {
            // F7 /0,/1 iw — opsize-16 path (opsize 32 out of scope).
            if i + 1 >= bytes.len() {
                return Err(DecodeError::Truncated);
            }
            immediate = i32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
            i += 2;
        }
    } else {
        match def.encoding {
            Encoding::Imm8 | Encoding::Imm8Port | Encoding::ModrmImm8 => {
                if i >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(bytes[i]);
                i += 1;
            }
            Encoding::ModrmImm16 => {
                if i + 1 >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
                i += 2;
            }
            Encoding::Rel8 => {
                if i >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(bytes[i] as i8);
                i += 1;
            }
            Encoding::Rel16 => {
                if i + 1 >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(i16::from_le_bytes([bytes[i], bytes[i + 1]]));
                i += 2;
            }
            Encoding::Imm16 => {
                if i + 1 >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(i16::from_le_bytes([bytes[i], bytes[i + 1]]));
                i += 2;
            }
            Encoding::Moffs => {
                // Absolute moffs — address-size attribute; real-mode default is 16-bit.
                // Spec: Intel SDM Vol. 2, MOV AL/AX/EAX/RAX, moffs / moffs, AL/AX/….
                // Unsupported here: address-size 32/64 (0x67 / long mode).
                if prefixes.addr_size_override {
                    return Err(DecodeError::UnsupportedOpcode(opcode));
                }
                if i + 1 >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
                i += 2;
            }
            Encoding::Imm16Imm8 => {
                // ENTER iw, ib — Spec: Intel SDM Vol. 2 "ENTER".
                if i + 2 >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
                displacement = i32::from(bytes[i + 2]);
                i += 3;
            }
            Encoding::Ptr16_16 => {
                // Far pointer: offset (imm16) then segment; segment stored in displacement.
                // Spec: Intel SDM Vol. 2, CALL/JMP far ptr16:16.
                if i + 3 >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
                displacement = i32::from(u16::from_le_bytes([bytes[i + 2], bytes[i + 3]]));
                i += 4;
            }
            Encoding::None | Encoding::Modrm | Encoding::OpcodeReg => {}
        }
    }

    if i > 15 {
        return Err(DecodeError::TooLong);
    }

    // Group 1 / 2 / 3 / 4 / 5: mnemonic from ModRM.reg (Intel SDM Vol. 2 opcode map).
    let mnemonic = if matches!(opcode, 0x80 | 0x81 | 0x83) {
        match modrm.map(|m| m.reg) {
            Some(0) => "ADD",
            Some(1) => "OR",
            Some(2) => "ADC",
            Some(3) => "SBB",
            Some(4) => "AND",
            Some(5) => "SUB",
            Some(6) => "XOR",
            Some(7) => "CMP",
            _ => def.mnemonic,
        }
    } else if matches!(opcode, 0xD0 | 0xD1 | 0xD2 | 0xD3 | 0xC0 | 0xC1) {
        match modrm.map(|m| m.reg) {
            Some(0) => "ROL",
            Some(1) => "ROR",
            Some(2) => "RCL",
            Some(3) => "RCR",
            Some(4) => "SHL",
            Some(5) => "SHR",
            Some(6) => "GRP2_RES",
            Some(7) => "SAR",
            _ => def.mnemonic,
        }
    } else if matches!(opcode, 0xF6 | 0xF7) {
        // Group 3: /0,/1 TEST; /2 NOT; /3 NEG; /4 MUL; /5 IMUL; /6 DIV; /7 IDIV (SDM Vol. 2).
        match modrm.map(|m| m.reg) {
            Some(0) | Some(1) => "TEST",
            Some(2) => "NOT",
            Some(3) => "NEG",
            Some(4) => "MUL",
            Some(5) => "IMUL",
            Some(6) => "DIV",
            Some(7) => "IDIV",
            _ => def.mnemonic,
        }
    } else if opcode == 0xFE {
        match modrm.map(|m| m.reg) {
            Some(0) => "INC",
            Some(1) => "DEC",
            _ => def.mnemonic,
        }
    } else if opcode == 0xFF {
        // Group 5: /0 INC, /1 DEC, /2 CALL near, /3 CALL far, /4 JMP near, /5 JMP far,
        // /6 PUSH (SDM Vol. 2 opcode map). /7 remains GRP5 placeholder (#UD).
        match modrm.map(|m| m.reg) {
            Some(0) => "INC",
            Some(1) => "DEC",
            Some(2) | Some(3) => "CALL",
            Some(4) | Some(5) => "JMP",
            Some(6) => "PUSH",
            _ => def.mnemonic,
        }
    } else if matches!(opcode, 0xC6 | 0xC7) {
        // Group 11: /0 MOV; other reg encodings reserved.
        match modrm.map(|m| m.reg) {
            Some(0) => "MOV",
            _ => def.mnemonic,
        }
    } else if opcode == 0x8F {
        // Group: POP r/m — only /0 is defined (Intel SDM Vol. 2 opcode map).
        match modrm.map(|m| m.reg) {
            Some(0) => "POP",
            Some(_) => "GRP_POP",
            _ => def.mnemonic,
        }
    } else {
        def.mnemonic
    };

    Ok(DecodedInsn {
        prefixes,
        opcode,
        modrm,
        displacement,
        immediate,
        length: i,
        mnemonic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_hlt() {
        let d = decode(&[0xF4]).unwrap();
        assert_eq!(d.mnemonic, "HLT");
        assert_eq!(d.length, 1);
    }

    #[test]
    fn decode_mov_bx_imm16() {
        let d = decode(&[0xBB, 0x16, 0x00]).unwrap();
        assert_eq!(d.opcode, 0xBB);
        assert_eq!(d.immediate, 0x16);
        assert_eq!(d.length, 3);
    }

    #[test]
    fn decode_mov_al_bx() {
        let d = decode(&[0x8A, 0x07]).unwrap();
        assert_eq!(d.modrm.unwrap().rm, 7);
        assert_eq!(d.length, 2);
    }

    #[test]
    fn decode_jmp_rel16() {
        let d = decode(&[0xE9, 0x0D, 0x00]).unwrap();
        assert_eq!(d.immediate, 0x0D);
        assert_eq!(d.length, 3);
    }

    #[test]
    fn truncated_modrm() {
        assert_eq!(decode(&[0x8A]), Err(DecodeError::Truncated));
    }

    #[test]
    fn unsupported_opcode() {
        assert!(matches!(
            decode(&[0x0F]),
            Err(DecodeError::UnsupportedOpcode(0x0F))
        ));
    }

    #[test]
    fn decode_int_imm8() {
        // Intel SDM Vol. 2: INT imm8 — opcode CD ib
        let d = decode(&[0xCD, 0x21]).unwrap();
        assert_eq!(d.mnemonic, "INT");
        assert_eq!(d.immediate, 0x21);
        assert_eq!(d.length, 2);
    }

    #[test]
    fn decode_iret() {
        // Intel SDM Vol. 2: IRET — opcode CF
        let d = decode(&[0xCF]).unwrap();
        assert_eq!(d.mnemonic, "IRET");
        assert_eq!(d.length, 1);
    }

    #[test]
    fn truncated_int() {
        assert_eq!(decode(&[0xCD]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_pushf() {
        // Intel SDM Vol. 2: PUSHF — opcode 9C
        let d = decode(&[0x9C]).unwrap();
        assert_eq!(d.mnemonic, "PUSHF");
        assert_eq!(d.length, 1);
    }

    #[test]
    fn decode_popf() {
        // Intel SDM Vol. 2: POPF — opcode 9D
        let d = decode(&[0x9D]).unwrap();
        assert_eq!(d.mnemonic, "POPF");
        assert_eq!(d.length, 1);
    }

    #[test]
    fn decode_call_far_ptr16_16() {
        // Intel SDM Vol. 2: CALL ptr16:16 — opcode 9A cd (offset then segment)
        let d = decode(&[0x9A, 0x34, 0x12, 0x00, 0xF0]).unwrap();
        assert_eq!(d.mnemonic, "CALL_FAR");
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.displacement, 0xF000);
        assert_eq!(d.length, 5);
    }

    #[test]
    fn decode_retf() {
        // Intel SDM Vol. 2: RETF — opcode CB
        let d = decode(&[0xCB]).unwrap();
        assert_eq!(d.mnemonic, "RETF");
        assert_eq!(d.length, 1);
    }

    #[test]
    fn truncated_call_far() {
        assert_eq!(decode(&[0x9A, 0x00, 0x10]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_jmp_far_ptr16_16() {
        // Intel SDM Vol. 2: JMP ptr16:16 — opcode EA cd
        let d = decode(&[0xEA, 0x00, 0x01, 0x00, 0xF0]).unwrap();
        assert_eq!(d.mnemonic, "JMP_FAR");
        assert_eq!(d.immediate, 0x0100);
        assert_eq!(d.displacement, 0xF000);
        assert_eq!(d.length, 5);
    }

    #[test]
    fn decode_push_pop_segment() {
        // Intel SDM Vol. 2: PUSH/POP ES/CS/SS/DS — opcodes 06/07/0E/16/17/1E/1F
        assert_eq!(decode(&[0x06]).unwrap().mnemonic, "PUSH_ES");
        assert_eq!(decode(&[0x07]).unwrap().mnemonic, "POP_ES");
        assert_eq!(decode(&[0x0E]).unwrap().mnemonic, "PUSH_CS");
        assert_eq!(decode(&[0x16]).unwrap().mnemonic, "PUSH_SS");
        assert_eq!(decode(&[0x17]).unwrap().mnemonic, "POP_SS");
        assert_eq!(decode(&[0x1E]).unwrap().mnemonic, "PUSH_DS");
        assert_eq!(decode(&[0x1F]).unwrap().mnemonic, "POP_DS");
    }

    #[test]
    fn decode_mov_sreg() {
        // Intel SDM Vol. 2: MOV r/m16, Sreg (8C /r); MOV Sreg, r/m16 (8E /r)
        // 8C D8 = MOV AX, DS (mod=11, reg=DS=3, rm=AX=0)
        let d = decode(&[0x8C, 0xD8]).unwrap();
        assert_eq!(d.mnemonic, "MOV_Sreg");
        assert_eq!(d.modrm.unwrap().reg, 3);
        assert_eq!(d.modrm.unwrap().rm, 0);
        assert_eq!(d.length, 2);
        // 8E D8 = MOV DS, AX
        let d = decode(&[0x8E, 0xD8]).unwrap();
        assert_eq!(d.mnemonic, "MOV_Sreg");
        assert_eq!(d.modrm.unwrap().reg, 3);
        assert_eq!(d.length, 2);
    }

    #[test]
    fn truncated_mov_sreg() {
        assert_eq!(decode(&[0x8C]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x8E]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_flag_ops() {
        // Intel SDM Vol. 2: CLC/STC/CLD/STD — F8/F9/FC/FD
        assert_eq!(decode(&[0xF8]).unwrap().mnemonic, "CLC");
        assert_eq!(decode(&[0xF9]).unwrap().mnemonic, "STC");
        assert_eq!(decode(&[0xFC]).unwrap().mnemonic, "CLD");
        assert_eq!(decode(&[0xFD]).unwrap().mnemonic, "STD");
    }

    #[test]
    fn decode_int3() {
        // Intel SDM Vol. 2: INT3 — opcode CC (1-byte breakpoint)
        let d = decode(&[0xCC]).unwrap();
        assert_eq!(d.mnemonic, "INT3");
        assert_eq!(d.length, 1);
    }

    #[test]
    fn decode_jcc_short_rel8() {
        // Intel SDM Vol. 2: Jcc — 70..7F cb
        assert_eq!(decode(&[0x70, 0x05]).unwrap().mnemonic, "JO");
        assert_eq!(decode(&[0x73, 0xFE]).unwrap().mnemonic, "JAE");
        assert_eq!(decode(&[0x7F, 0x00]).unwrap().mnemonic, "JG");
        let d = decode(&[0x7C, 0x10]).unwrap();
        assert_eq!(d.mnemonic, "JL");
        assert_eq!(d.immediate, 0x10);
        assert_eq!(d.length, 2);
    }

    #[test]
    fn decode_string_byte_ops() {
        // Intel SDM Vol. 2: MOVSB/STOSB/LODSB — A4/AA/AC
        assert_eq!(decode(&[0xA4]).unwrap().mnemonic, "MOVSB");
        assert_eq!(decode(&[0xAA]).unwrap().mnemonic, "STOSB");
        assert_eq!(decode(&[0xAC]).unwrap().mnemonic, "LODSB");
    }

    #[test]
    fn decode_grp2_shift_rotate_by1() {
        // Intel SDM Vol. 2 Group 2: D0/D1 /r with count=1.
        // D0 C0 = ROL AL,1; D1 E0 = SHL AX,1; D0 F8 = SAR AL,1
        let d = decode(&[0xD0, 0xC0]).unwrap();
        assert_eq!(d.mnemonic, "ROL");
        assert_eq!(d.modrm.unwrap().reg, 0);
        assert_eq!(d.length, 2);
        assert_eq!(decode(&[0xD0, 0xC8]).unwrap().mnemonic, "ROR");
        assert_eq!(decode(&[0xD0, 0xD0]).unwrap().mnemonic, "RCL");
        assert_eq!(decode(&[0xD0, 0xD8]).unwrap().mnemonic, "RCR");
        assert_eq!(decode(&[0xD1, 0xE0]).unwrap().mnemonic, "SHL");
        assert_eq!(decode(&[0xD1, 0xE8]).unwrap().mnemonic, "SHR");
        assert_eq!(decode(&[0xD0, 0xF8]).unwrap().mnemonic, "SAR");
        assert_eq!(decode(&[0xD0, 0xF0]).unwrap().mnemonic, "GRP2_RES");
    }

    #[test]
    fn truncated_grp2() {
        assert_eq!(decode(&[0xD0]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xD1]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_grp2_imm8() {
        // Intel SDM Vol. 2 Group 2: C0/C1 /r ib
        // C0 E0 03 = SHL AL, 3
        let d = decode(&[0xC0, 0xE0, 0x03]).unwrap();
        assert_eq!(d.mnemonic, "SHL");
        assert_eq!(d.immediate, 3);
        assert_eq!(d.length, 3);
        let d = decode(&[0xC1, 0xE8, 0x04]).unwrap();
        assert_eq!(d.mnemonic, "SHR");
        assert_eq!(d.immediate, 4);
        assert_eq!(d.length, 3);
        assert_eq!(decode(&[0xC0, 0xE0]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_grp2_cl() {
        // Intel SDM Vol. 2 Group 2: D2/D3 /r (count = CL)
        let d = decode(&[0xD2, 0xE0]).unwrap();
        assert_eq!(d.mnemonic, "SHL");
        assert_eq!(d.length, 2);
        assert_eq!(decode(&[0xD3, 0xE8]).unwrap().mnemonic, "SHR");
        assert_eq!(decode(&[0xD2]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_push_imm() {
        // Intel SDM Vol. 2: PUSH imm16 (68 iw), PUSH imm8 (6A ib sign-ext)
        let d = decode(&[0x68, 0x34, 0x12]).unwrap();
        assert_eq!(d.mnemonic, "PUSH");
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 3);
        let d = decode(&[0x6A, 0xFE]).unwrap();
        assert_eq!(d.mnemonic, "PUSH");
        assert_eq!(d.immediate, 0xFE); // raw byte; sign-ext at execute
        assert_eq!(d.length, 2);
        assert_eq!(decode(&[0x68, 0x00]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x6A]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_sahf_lahf() {
        // Intel SDM Vol. 2: SAHF (9E), LAHF (9F)
        assert_eq!(decode(&[0x9E]).unwrap().mnemonic, "SAHF");
        assert_eq!(decode(&[0x9F]).unwrap().mnemonic, "LAHF");
        assert_eq!(decode(&[0x9E]).unwrap().length, 1);
    }

    #[test]
    fn decode_dec_r16() {
        // Intel SDM Vol. 2: DEC r16 — 48+rw
        assert_eq!(decode(&[0x48]).unwrap().mnemonic, "DEC"); // DEC AX
        assert_eq!(decode(&[0x4B]).unwrap().mnemonic, "DEC"); // DEC BX
        assert_eq!(decode(&[0x4F]).unwrap().mnemonic, "DEC"); // DEC DI
        assert_eq!(decode(&[0x40]).unwrap().mnemonic, "INC");
    }

    #[test]
    fn decode_grp1_imm() {
        // Intel SDM Vol. 2 Group 1: 80/81/83 — /r selects ALU op
        assert_eq!(decode(&[0x80, 0xC0, 0x01]).unwrap().mnemonic, "ADD"); // ADD AL,1
        assert_eq!(decode(&[0x80, 0xE0, 0x0F]).unwrap().mnemonic, "AND"); // AND AL,0x0F
        assert_eq!(decode(&[0x80, 0xF8, 0x00]).unwrap().mnemonic, "CMP"); // CMP AL,0
        let d = decode(&[0x81, 0xC3, 0x34, 0x12]).unwrap(); // ADD BX, 0x1234
        assert_eq!(d.mnemonic, "ADD");
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 4);
        let d = decode(&[0x83, 0xE8, 0xFF]).unwrap(); // SUB AX, -1 (sign-ext)
        assert_eq!(d.mnemonic, "SUB");
        assert_eq!(d.immediate, 0xFF);
        assert_eq!(d.length, 3);
        assert_eq!(decode(&[0x81, 0xC0]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x80, 0xC0]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_and_or_al_ax_imm() {
        // Intel SDM Vol. 2: OR AL/AX,imm (0C/0D); AND AL/AX,imm (24/25)
        let d = decode(&[0x0C, 0x0F]).unwrap();
        assert_eq!(d.mnemonic, "OR");
        assert_eq!(d.immediate, 0x0F);
        assert_eq!(d.length, 2);
        let d = decode(&[0x0D, 0xF0, 0x0F]).unwrap();
        assert_eq!(d.mnemonic, "OR");
        assert_eq!(d.immediate, 0x0FF0);
        assert_eq!(d.length, 3);
        let d = decode(&[0x24, 0xF0]).unwrap();
        assert_eq!(d.mnemonic, "AND");
        assert_eq!(d.immediate, 0xF0);
        assert_eq!(d.length, 2);
        let d = decode(&[0x25, 0xFF, 0x00]).unwrap();
        assert_eq!(d.mnemonic, "AND");
        assert_eq!(d.immediate, 0x00FF);
        assert_eq!(d.length, 3);
        assert_eq!(decode(&[0x0C]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x0D, 0x00]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x24]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x25, 0x00]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_loop_jcxz() {
        // Intel SDM Vol. 2: LOOPNE/LOOPE/LOOP/JCXZ — E0–E3 rel8
        assert_eq!(decode(&[0xE0, 0xFE]).unwrap().mnemonic, "LOOPNE");
        assert_eq!(decode(&[0xE1, 0x00]).unwrap().mnemonic, "LOOPE");
        let d = decode(&[0xE2, 0xFB]).unwrap();
        assert_eq!(d.mnemonic, "LOOP");
        assert_eq!(d.immediate, -5);
        assert_eq!(d.length, 2);
        assert_eq!(decode(&[0xE3, 0x02]).unwrap().mnemonic, "JCXZ");
        assert_eq!(decode(&[0xE2]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_xchg() {
        // Intel SDM Vol. 2: XCHG r/m8,r8 (86 /r); XCHG r/m16,r16 (87 /r);
        // XCHG AX,r16 (91–97); 90 remains NOP.
        let d = decode(&[0x86, 0xC3]).unwrap(); // XCHG AL, BL
        assert_eq!(d.mnemonic, "XCHG");
        assert_eq!(d.modrm.unwrap().reg, 0);
        assert_eq!(d.modrm.unwrap().rm, 3);
        assert_eq!(d.length, 2);

        let d = decode(&[0x87, 0x06, 0x00, 0x20]).unwrap(); // XCHG AX, [0x2000]
        assert_eq!(d.mnemonic, "XCHG");
        assert_eq!(d.displacement, 0x2000);
        assert_eq!(d.length, 4);

        assert_eq!(decode(&[0x91]).unwrap().mnemonic, "XCHG");
        assert_eq!(decode(&[0x97]).unwrap().mnemonic, "XCHG");
        assert_eq!(decode(&[0x90]).unwrap().mnemonic, "NOP");
        assert_eq!(decode(&[0x86]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_cbw_cwd() {
        // Intel SDM Vol. 2: CBW (98), CWD (99) — 16-bit forms
        assert_eq!(decode(&[0x98]).unwrap().mnemonic, "CBW");
        assert_eq!(decode(&[0x99]).unwrap().mnemonic, "CWD");
    }

    #[test]
    fn decode_lea() {
        // Intel SDM Vol. 2: LEA r16, m — 8D /r
        // 8D 06 34 12 = LEA AX, [0x1234]
        let d = decode(&[0x8D, 0x06, 0x34, 0x12]).unwrap();
        assert_eq!(d.mnemonic, "LEA");
        assert_eq!(d.modrm.unwrap().reg, 0);
        assert_eq!(d.displacement, 0x1234);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0x8D]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_grp3_not_neg() {
        // Intel SDM Vol. 2 Group 3: F6/F7 /2 NOT, /3 NEG
        assert_eq!(decode(&[0xF6, 0xD0]).unwrap().mnemonic, "NOT"); // NOT AL
        assert_eq!(decode(&[0xF6, 0xD8]).unwrap().mnemonic, "NEG"); // NEG AL
        assert_eq!(decode(&[0xF7, 0xD0]).unwrap().mnemonic, "NOT"); // NOT AX
        assert_eq!(decode(&[0xF7, 0xD8]).unwrap().mnemonic, "NEG"); // NEG AX
        assert_eq!(decode(&[0xF6]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_grp3_test_mul() {
        // Intel SDM Vol. 2 Group 3: F6/F7 /0,/1 TEST imm; /4 MUL.
        let d = decode(&[0xF6, 0xC0, 0x0F]).unwrap(); // TEST AL, 0x0F
        assert_eq!(d.mnemonic, "TEST");
        assert_eq!(d.modrm.unwrap().reg, 0);
        assert_eq!(d.immediate, 0x0F);
        assert_eq!(d.length, 3);
        let d = decode(&[0xF6, 0xC8, 0x01]).unwrap(); // TEST AL, 1 (/1 alias)
        assert_eq!(d.mnemonic, "TEST");
        assert_eq!(d.modrm.unwrap().reg, 1);
        assert_eq!(d.immediate, 1);
        assert_eq!(d.length, 3);
        let d = decode(&[0xF7, 0xC0, 0x34, 0x12]).unwrap(); // TEST AX, 0x1234
        assert_eq!(d.mnemonic, "TEST");
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0xF6, 0xE0]).unwrap().mnemonic, "MUL"); // MUL AL
        assert_eq!(decode(&[0xF7, 0xE0]).unwrap().mnemonic, "MUL"); // MUL AX
        let d = decode(&[0xF6, 0x06, 0x00, 0x40, 0xFF]).unwrap(); // TEST byte [0x4000], 0xFF (/0)
        assert_eq!(d.mnemonic, "TEST");
        assert_eq!(d.modrm.unwrap().reg, 0);
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.immediate, 0xFF);
        assert_eq!(d.length, 5);
        assert_eq!(decode(&[0xF6, 0xC0]), Err(DecodeError::Truncated)); // TEST needs imm8
        assert_eq!(decode(&[0xF7, 0xC0, 0x00]), Err(DecodeError::Truncated)); // TEST needs imm16
    }

    #[test]
    fn decode_grp3_imul_div_idiv() {
        // Intel SDM Vol. 2 Group 3: F6/F7 /5 IMUL, /6 DIV, /7 IDIV (no immediate).
        assert_eq!(decode(&[0xF6, 0xE8]).unwrap().mnemonic, "IMUL"); // IMUL AL
        assert_eq!(decode(&[0xF6, 0xF0]).unwrap().mnemonic, "DIV"); // DIV AL
        assert_eq!(decode(&[0xF6, 0xF8]).unwrap().mnemonic, "IDIV"); // IDIV AL
        assert_eq!(decode(&[0xF7, 0xE8]).unwrap().mnemonic, "IMUL"); // IMUL AX
        assert_eq!(decode(&[0xF7, 0xF0]).unwrap().mnemonic, "DIV"); // DIV AX
        assert_eq!(decode(&[0xF7, 0xF8]).unwrap().mnemonic, "IDIV"); // IDIV AX
        let d = decode(&[0xF6, 0x36, 0x00, 0x40]).unwrap(); // DIV byte [0x4000]
        assert_eq!(d.mnemonic, "DIV");
        assert_eq!(d.modrm.unwrap().reg, 6);
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0xF6, 0xE8]).unwrap().length, 2);
        assert_eq!(decode(&[0xF7, 0xF0]).unwrap().length, 2);
    }

    #[test]
    fn decode_and_or_modrm() {
        // Intel SDM Vol. 2: OR/AND r/m,r and r,r/m (08–0B, 20–23).
        assert_eq!(decode(&[0x08, 0xC3]).unwrap().mnemonic, "OR"); // OR BL, AL
        assert_eq!(decode(&[0x09, 0xC3]).unwrap().mnemonic, "OR"); // OR BX, AX
        assert_eq!(decode(&[0x0A, 0xC3]).unwrap().mnemonic, "OR"); // OR AL, BL
        assert_eq!(decode(&[0x0B, 0xC3]).unwrap().mnemonic, "OR"); // OR AX, BX
        assert_eq!(decode(&[0x20, 0xC3]).unwrap().mnemonic, "AND"); // AND BL, AL
        assert_eq!(decode(&[0x21, 0xC3]).unwrap().mnemonic, "AND"); // AND BX, AX
        assert_eq!(decode(&[0x22, 0xC3]).unwrap().mnemonic, "AND"); // AND AL, BL
        assert_eq!(decode(&[0x23, 0xC3]).unwrap().mnemonic, "AND"); // AND AX, BX
        let d = decode(&[0x21, 0x06, 0x00, 0x30]).unwrap(); // AND [0x3000], AX
        assert_eq!(d.mnemonic, "AND");
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0x08]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x23]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_adc_sbb_modrm() {
        // Intel SDM Vol. 2: ADC/SBB r/m,r and r,r/m (10–13, 18–1B).
        assert_eq!(decode(&[0x10, 0xC3]).unwrap().mnemonic, "ADC"); // ADC BL, AL
        assert_eq!(decode(&[0x11, 0xC3]).unwrap().mnemonic, "ADC"); // ADC BX, AX
        assert_eq!(decode(&[0x12, 0xC3]).unwrap().mnemonic, "ADC"); // ADC AL, BL
        assert_eq!(decode(&[0x13, 0xC3]).unwrap().mnemonic, "ADC"); // ADC AX, BX
        assert_eq!(decode(&[0x18, 0xC3]).unwrap().mnemonic, "SBB"); // SBB BL, AL
        assert_eq!(decode(&[0x19, 0xC3]).unwrap().mnemonic, "SBB"); // SBB BX, AX
        assert_eq!(decode(&[0x1A, 0xC3]).unwrap().mnemonic, "SBB"); // SBB AL, BL
        assert_eq!(decode(&[0x1B, 0xC3]).unwrap().mnemonic, "SBB"); // SBB AX, BX
        let d = decode(&[0x11, 0x06, 0x00, 0x30]).unwrap(); // ADC [0x3000], AX
        assert_eq!(d.mnemonic, "ADC");
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0x10]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x1B]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_adc_sbb_al_ax_imm() {
        // Intel SDM Vol. 2: ADC AL/AX,imm (14/15); SBB AL/AX,imm (1C/1D).
        let d = decode(&[0x14, 0x01]).unwrap();
        assert_eq!(d.mnemonic, "ADC");
        assert_eq!(d.immediate, 0x01);
        assert_eq!(d.length, 2);
        let d = decode(&[0x15, 0x00, 0x10]).unwrap();
        assert_eq!(d.mnemonic, "ADC");
        assert_eq!(d.immediate, 0x1000);
        assert_eq!(d.length, 3);
        let d = decode(&[0x1C, 0x02]).unwrap();
        assert_eq!(d.mnemonic, "SBB");
        assert_eq!(d.immediate, 0x02);
        assert_eq!(d.length, 2);
        let d = decode(&[0x1D, 0x01, 0x00]).unwrap();
        assert_eq!(d.mnemonic, "SBB");
        assert_eq!(d.immediate, 0x0001);
        assert_eq!(d.length, 3);
        assert_eq!(decode(&[0x14]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x15, 0x00]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x1C]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x1D, 0x00]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_xor_modrm() {
        // Intel SDM Vol. 2: XOR r/m,r and r,r/m (30–33).
        assert_eq!(decode(&[0x30, 0xC3]).unwrap().mnemonic, "XOR"); // XOR BL, AL
        assert_eq!(decode(&[0x31, 0xC3]).unwrap().mnemonic, "XOR"); // XOR BX, AX
        assert_eq!(decode(&[0x32, 0xC3]).unwrap().mnemonic, "XOR"); // XOR AL, BL
        assert_eq!(decode(&[0x33, 0xC3]).unwrap().mnemonic, "XOR"); // XOR AX, BX
        let d = decode(&[0x30, 0x06, 0x00, 0x30]).unwrap(); // XOR [0x3000], AL
        assert_eq!(d.mnemonic, "XOR");
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0x30]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x32]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_add_sub_modrm_byte() {
        // Intel SDM Vol. 2: ADD/SUB r/m8,r8 and r8,r/m8 (00/02, 28/2A).
        assert_eq!(decode(&[0x00, 0xC3]).unwrap().mnemonic, "ADD"); // ADD BL, AL
        assert_eq!(decode(&[0x02, 0xC3]).unwrap().mnemonic, "ADD"); // ADD AL, BL
        assert_eq!(decode(&[0x28, 0xC3]).unwrap().mnemonic, "SUB"); // SUB BL, AL
        assert_eq!(decode(&[0x2A, 0xC3]).unwrap().mnemonic, "SUB"); // SUB AL, BL
        let d = decode(&[0x00, 0x06, 0x00, 0x30]).unwrap(); // ADD [0x3000], AL
        assert_eq!(d.mnemonic, "ADD");
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.length, 4);
        let d = decode(&[0x28, 0x06, 0x00, 0x30]).unwrap(); // SUB [0x3000], AL
        assert_eq!(d.mnemonic, "SUB");
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0x00]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x02]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x28]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x2A]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_cmp_modrm_byte() {
        // Intel SDM Vol. 2: CMP r/m8,r8 and r8,r/m8 (38/3A).
        assert_eq!(decode(&[0x38, 0xC3]).unwrap().mnemonic, "CMP"); // CMP BL, AL
        assert_eq!(decode(&[0x3A, 0xC3]).unwrap().mnemonic, "CMP"); // CMP AL, BL
        let d = decode(&[0x38, 0x06, 0x00, 0x30]).unwrap(); // CMP [0x3000], AL
        assert_eq!(d.mnemonic, "CMP");
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.length, 4);
        let d = decode(&[0x3A, 0x06, 0x00, 0x30]).unwrap(); // CMP AL, [0x3000]
        assert_eq!(d.mnemonic, "CMP");
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0x38]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x3A]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_sub_xor_cmp_al_ax_imm() {
        // Intel SDM Vol. 2: SUB AL/AX,imm (2C/2D); XOR AL/AX,imm (34/35); CMP AL/AX,imm (3C/3D).
        let d = decode(&[0x2C, 0x01]).unwrap();
        assert_eq!(d.mnemonic, "SUB");
        assert_eq!(d.immediate, 0x01);
        assert_eq!(d.length, 2);
        let d = decode(&[0x2D, 0x00, 0x10]).unwrap();
        assert_eq!(d.mnemonic, "SUB");
        assert_eq!(d.immediate, 0x1000);
        assert_eq!(d.length, 3);
        let d = decode(&[0x34, 0x0F]).unwrap();
        assert_eq!(d.mnemonic, "XOR");
        assert_eq!(d.immediate, 0x0F);
        assert_eq!(d.length, 2);
        let d = decode(&[0x35, 0xFF, 0x00]).unwrap();
        assert_eq!(d.mnemonic, "XOR");
        assert_eq!(d.immediate, 0x00FF);
        assert_eq!(d.length, 3);
        let d = decode(&[0x3C, 0x05]).unwrap();
        assert_eq!(d.mnemonic, "CMP");
        assert_eq!(d.immediate, 0x05);
        assert_eq!(d.length, 2);
        let d = decode(&[0x3D, 0x34, 0x12]).unwrap();
        assert_eq!(d.mnemonic, "CMP");
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 3);
        assert_eq!(decode(&[0x2C]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x2D, 0x00]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x34]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x35, 0x00]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x3C]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x3D, 0x00]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_add_ax_imm() {
        // Intel SDM Vol. 2: ADD AX, imm16 (05 iw).
        let d = decode(&[0x05, 0x34, 0x12]).unwrap();
        assert_eq!(d.mnemonic, "ADD");
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 3);
        let d = decode(&[0x05, 0x00, 0x80]).unwrap();
        assert_eq!(d.mnemonic, "ADD");
        // Imm16 is decoded via i16; high bit set → negative i32; value as u16 is 0x8000.
        assert_eq!(d.immediate as u16, 0x8000);
        assert_eq!(d.length, 3);
        assert_eq!(decode(&[0x05]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x05, 0x00]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_grp4_grp5_inc_dec() {
        // Intel SDM Vol. 2 Group 4/5: FE/FF /0 INC, /1 DEC.
        assert_eq!(decode(&[0xFE, 0xC0]).unwrap().mnemonic, "INC"); // INC AL
        assert_eq!(decode(&[0xFE, 0xC8]).unwrap().mnemonic, "DEC"); // DEC AL
        assert_eq!(decode(&[0xFF, 0xC0]).unwrap().mnemonic, "INC"); // INC AX
        assert_eq!(decode(&[0xFF, 0xC8]).unwrap().mnemonic, "DEC"); // DEC AX
        let d = decode(&[0xFE, 0x06, 0x00, 0x40]).unwrap(); // INC byte [0x4000]
        assert_eq!(d.mnemonic, "INC");
        assert_eq!(d.modrm.unwrap().reg, 0);
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.length, 4);
        let d = decode(&[0xFF, 0x0E, 0x00, 0x40]).unwrap(); // DEC word [0x4000]
        assert_eq!(d.mnemonic, "DEC");
        assert_eq!(d.modrm.unwrap().reg, 1);
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.length, 4);
        // Other FE /r forms remain group placeholders; FF /2,/4,/6 named below.
        assert_eq!(decode(&[0xFE, 0xD0]).unwrap().mnemonic, "GRP4"); // FE /2
        assert_eq!(decode(&[0xFE]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xFF]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_grp5_call_jmp_push() {
        // Intel SDM Vol. 2 Group 5: FF /2 CALL near, /4 JMP near, /6 PUSH r/m.
        assert_eq!(decode(&[0xFF, 0xD0]).unwrap().mnemonic, "CALL"); // CALL AX
        assert_eq!(decode(&[0xFF, 0xE0]).unwrap().mnemonic, "JMP"); // JMP AX
        assert_eq!(decode(&[0xFF, 0xF0]).unwrap().mnemonic, "PUSH"); // PUSH AX
        let d = decode(&[0xFF, 0x16, 0x00, 0x40]).unwrap(); // CALL word [0x4000]
        assert_eq!(d.mnemonic, "CALL");
        assert_eq!(d.modrm.unwrap().reg, 2);
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.length, 4);
        let d = decode(&[0xFF, 0x26, 0x00, 0x40]).unwrap(); // JMP word [0x4000]
        assert_eq!(d.mnemonic, "JMP");
        assert_eq!(d.modrm.unwrap().reg, 4);
        let d = decode(&[0xFF, 0x36, 0x00, 0x40]).unwrap(); // PUSH word [0x4000]
        assert_eq!(d.mnemonic, "PUSH");
        assert_eq!(d.modrm.unwrap().reg, 6);
        assert_eq!(decode(&[0xFF, 0xF8]).unwrap().mnemonic, "GRP5"); // FF /7
    }

    #[test]
    fn decode_grp5_call_jmp_far() {
        // Intel SDM Vol. 2 Group 5: FF /3 CALL far m16:16, /5 JMP far m16:16.
        assert_eq!(decode(&[0xFF, 0xD8]).unwrap().mnemonic, "CALL"); // CALL FAR AX (reg → #UD at exec)
        assert_eq!(decode(&[0xFF, 0xE8]).unwrap().mnemonic, "JMP"); // JMP FAR AX (reg → #UD at exec)
        let d = decode(&[0xFF, 0x1E, 0x00, 0x40]).unwrap(); // CALL FAR [0x4000]
        assert_eq!(d.mnemonic, "CALL");
        assert_eq!(d.modrm.unwrap().reg, 3);
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.length, 4);
        let d = decode(&[0xFF, 0x2E, 0x00, 0x40]).unwrap(); // JMP FAR [0x4000]
        assert_eq!(d.mnemonic, "JMP");
        assert_eq!(d.modrm.unwrap().reg, 5);
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0xFF, 0xF8]).unwrap().mnemonic, "GRP5"); // FF /7 still placeholder
    }

    #[test]
    fn decode_mov_rm_imm_c6_c7() {
        // Intel SDM Vol. 2: MOV r/m8,imm8 (C6 /0); MOV r/m16,imm16 (C7 /0).
        // C6 C0 5A = MOV AL, 0x5A
        let d = decode(&[0xC6, 0xC0, 0x5A]).unwrap();
        assert_eq!(d.mnemonic, "MOV");
        assert_eq!(d.modrm.unwrap().reg, 0);
        assert_eq!(d.immediate, 0x5A);
        assert_eq!(d.length, 3);
        // C6 06 00 40 99 = MOV byte [0x4000], 0x99
        let d = decode(&[0xC6, 0x06, 0x00, 0x40, 0x99]).unwrap();
        assert_eq!(d.mnemonic, "MOV");
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.immediate, 0x99);
        assert_eq!(d.length, 5);
        // C7 C3 34 12 = MOV BX, 0x1234
        let d = decode(&[0xC7, 0xC3, 0x34, 0x12]).unwrap();
        assert_eq!(d.mnemonic, "MOV");
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 4);
        // C7 06 00 30 CD AB = MOV word [0x3000], 0xABCD
        let d = decode(&[0xC7, 0x06, 0x00, 0x30, 0xCD, 0xAB]).unwrap();
        assert_eq!(d.mnemonic, "MOV");
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.immediate, 0xABCD);
        assert_eq!(d.length, 6);
        // Non-/0 keeps group mnemonic
        assert_eq!(decode(&[0xC6, 0xC8, 0x00]).unwrap().mnemonic, "GRP11");
        assert_eq!(decode(&[0xC6]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xC6, 0xC0]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xC7, 0xC0]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_mov_moffs_a0_a3() {
        // Intel SDM Vol. 2: MOV AL/AX, moffs; MOV moffs, AL/AX (A0–A3).
        let d = decode(&[0xA0, 0x34, 0x12]).unwrap();
        assert_eq!(d.mnemonic, "MOV");
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 3);
        let d = decode(&[0xA1, 0x00, 0x40]).unwrap();
        assert_eq!(d.mnemonic, "MOV");
        assert_eq!(d.immediate, 0x4000);
        assert_eq!(d.length, 3);
        let d = decode(&[0xA2, 0x00, 0x50]).unwrap();
        assert_eq!(d.mnemonic, "MOV");
        assert_eq!(d.immediate, 0x5000);
        assert_eq!(d.length, 3);
        let d = decode(&[0xA3, 0xFE, 0xFF]).unwrap();
        assert_eq!(d.mnemonic, "MOV");
        assert_eq!(d.immediate, 0xFFFE);
        assert_eq!(d.length, 3);
        // CS: override prefix
        let d = decode(&[0x2E, 0xA0, 0x00, 0x10]).unwrap();
        assert_eq!(d.prefixes.segment_override, Some(0x2E));
        assert_eq!(d.immediate, 0x1000);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0xA0]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xA1, 0x00]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_test_al_ax_imm_a8_a9() {
        // Intel SDM Vol. 2: TEST AL,imm8 (A8); TEST AX,imm16 (A9).
        let d = decode(&[0xA8, 0x0F]).unwrap();
        assert_eq!(d.mnemonic, "TEST");
        assert_eq!(d.immediate, 0x0F);
        assert_eq!(d.length, 2);
        let d = decode(&[0xA9, 0xFF, 0x00]).unwrap();
        assert_eq!(d.mnemonic, "TEST");
        assert_eq!(d.immediate, 0x00FF);
        assert_eq!(d.length, 3);
        assert_eq!(decode(&[0xA8]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xA9, 0x00]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_pusha_popa_enter_leave() {
        // Intel SDM Vol. 2: PUSHA (60), POPA (61), ENTER iw,ib (C8), LEAVE (C9).
        assert_eq!(decode(&[0x60]).unwrap().mnemonic, "PUSHA");
        assert_eq!(decode(&[0x60]).unwrap().length, 1);
        assert_eq!(decode(&[0x61]).unwrap().mnemonic, "POPA");
        let d = decode(&[0xC8, 0x08, 0x00, 0x00]).unwrap(); // ENTER 8, 0
        assert_eq!(d.mnemonic, "ENTER");
        assert_eq!(d.immediate, 8);
        assert_eq!(d.displacement, 0);
        assert_eq!(d.length, 4);
        let d = decode(&[0xC8, 0x10, 0x00, 0x02]).unwrap(); // ENTER 16, 2
        assert_eq!(d.immediate, 16);
        assert_eq!(d.displacement, 2);
        assert_eq!(decode(&[0xC9]).unwrap().mnemonic, "LEAVE");
        assert_eq!(decode(&[0xC8, 0x00]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xC8, 0x00, 0x00]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_ret_retf_imm16() {
        // Intel SDM Vol. 2: RET iw (C2), RETF iw (CA).
        let d = decode(&[0xC2, 0x04, 0x00]).unwrap();
        assert_eq!(d.mnemonic, "RET");
        assert_eq!(d.immediate, 4);
        assert_eq!(d.length, 3);
        let d = decode(&[0xCA, 0x02, 0x00]).unwrap();
        assert_eq!(d.mnemonic, "RETF");
        assert_eq!(d.immediate, 2);
        assert_eq!(d.length, 3);
        assert_eq!(decode(&[0xC2, 0x00]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xCA]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_pop_rm16() {
        // Intel SDM Vol. 2: POP r/m16 — 8F /0
        assert_eq!(decode(&[0x8F, 0xC0]).unwrap().mnemonic, "POP"); // POP AX
        let d = decode(&[0x8F, 0x06, 0x00, 0x30]).unwrap(); // POP [0x3000]
        assert_eq!(d.mnemonic, "POP");
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.length, 4);
        assert_eq!(decode(&[0x8F, 0xC8]).unwrap().mnemonic, "GRP_POP"); // /1 reserved
        assert_eq!(decode(&[0x8F]), Err(DecodeError::Truncated));
    }
}
