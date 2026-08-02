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

    let needs_modrm = matches!(def.encoding, Encoding::Modrm | Encoding::ModrmImm8)
        || matches!(
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
    } else {
        match def.encoding {
            Encoding::Imm8 | Encoding::Imm8Port => {
                if i >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(bytes[i]);
                i += 1;
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
            Encoding::None | Encoding::Modrm | Encoding::ModrmImm8 | Encoding::OpcodeReg => {}
        }
    }

    if i > 15 {
        return Err(DecodeError::TooLong);
    }

    Ok(DecodedInsn {
        prefixes,
        opcode,
        modrm,
        displacement,
        immediate,
        length: i,
        mnemonic: def.mnemonic,
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
}
