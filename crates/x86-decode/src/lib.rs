//! Legacy-prefix + primary-opcode decoder for the Milestone 1 subset.
//!
//! Spec: Intel SDM Vol. 2, Chapter 2 (instruction format).

#![forbid(unsafe_code)]

use thiserror::Error;
use x86_spec::{lookup_0f, lookup_primary, Encoding};

/// Default operand-size and address-size attributes supplied by the executing
/// code segment.
///
/// Legacy real-address mode and a `D=0` protected-mode code segment both use
/// 16-bit defaults; a `D=1` code segment uses 32-bit defaults. The `0x66` and
/// `0x67` override prefixes always select the *other* size, so they invert
/// under `D=1`.
///
/// Spec: Intel SDM Vol. 1 §3.6 (Table 3-4); Vol. 2 Chapter 2 (66H/67H);
/// Vol. 3 §3.4.5 (D/B flag).
///
/// Unsupported here: 64-bit mode defaults (REX.W / `L=1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodeMode {
    pub default_operand_size_32: bool,
    pub default_address_size_32: bool,
}

impl DecodeMode {
    /// Real-address mode / `D=0` protected mode.
    pub const LEGACY16: Self = Self {
        default_operand_size_32: false,
        default_address_size_32: false,
    };
    /// `D=1` protected mode.
    pub const DEFAULT32: Self = Self {
        default_operand_size_32: true,
        default_address_size_32: true,
    };

    /// Select defaults from the cached code-segment `D` bit.
    pub const fn from_cs_default_big(default_big: bool) -> Self {
        if default_big {
            Self::DEFAULT32
        } else {
            Self::LEGACY16
        }
    }
}

impl Default for DecodeMode {
    fn default() -> Self {
        Self::LEGACY16
    }
}

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
    /// Primary opcode, or secondary opcode when `two_byte` (0F escape).
    pub opcode: u8,
    /// True when the instruction used the two-byte opcode map (0F xx).
    /// Spec: Intel SDM Vol. 2 Chapter 2 (two-byte opcode escape).
    pub two_byte: bool,
    pub modrm: Option<Modrm>,
    /// SIB byte when address-size 32 and ModRM.rm = 4 (memory form).
    /// Spec: Intel SDM Vol. 2 Chapter 2 (SIB byte).
    pub sib: Option<u8>,
    pub displacement: i32,
    pub immediate: i32,
    pub length: usize,
    pub mnemonic: &'static str,
    /// Effective operand-size attribute: `true` = 32, `false` = 16.
    ///
    /// Computed from the [`DecodeMode`] default and the `0x66` override.
    /// Spec: Intel SDM Vol. 1 §3.6 (Table 3-4); Vol. 2 Chapter 2.
    pub operand_size_32: bool,
    /// Effective address-size attribute: `true` = 32, `false` = 16.
    ///
    /// Computed from the [`DecodeMode`] default and the `0x67` override.
    /// Spec: Intel SDM Vol. 1 §3.6 (Table 3-4); Vol. 2 Chapter 2.
    pub address_size_32: bool,
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

/// Decode one instruction from `bytes` (max 15 bytes per SDM) using legacy
/// 16-bit defaults.
///
/// Equivalent to [`decode_with_mode`] with [`DecodeMode::LEGACY16`].
pub fn decode(bytes: &[u8]) -> Result<DecodedInsn, DecodeError> {
    decode_with_mode(bytes, DecodeMode::LEGACY16)
}

/// Decode one instruction from `bytes` (max 15 bytes per SDM) with explicit
/// code-segment default operand/address sizes.
///
/// Spec: Intel SDM Vol. 2 Chapter 2 (instruction format, 66H/67H);
/// Vol. 1 §3.6 (Table 3-4).
pub fn decode_with_mode(bytes: &[u8], mode: DecodeMode) -> Result<DecodedInsn, DecodeError> {
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
            // F2/F3 are mutually exclusive; last one wins (SDM Vol. 2, Chapter 2).
            0xF3 => {
                prefixes.rep = true;
                prefixes.repne = false;
            }
            0xF2 => {
                prefixes.repne = true;
                prefixes.rep = false;
            }
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
    let primary = bytes[i];
    i += 1;
    // Two-byte opcode escape 0F — Spec: Intel SDM Vol. 2 Chapter 2.
    let (opcode, two_byte, def) = if primary == 0x0F {
        if i >= bytes.len() {
            return Err(DecodeError::Truncated);
        }
        let secondary = bytes[i];
        i += 1;
        let def = lookup_0f(secondary).ok_or(DecodeError::UnsupportedOpcode(secondary))?;
        (secondary, true, def)
    } else {
        let def = lookup_primary(primary).ok_or(DecodeError::UnsupportedOpcode(primary))?;
        (primary, false, def)
    };

    // Effective attributes: the override prefixes always select the size the
    // code segment does *not* default to (SDM Vol. 1 §3.6, Table 3-4).
    let operand_size_32 = mode.default_operand_size_32 != prefixes.op_size_override;
    let address_size_32 = mode.default_address_size_32 != prefixes.addr_size_override;

    let mut modrm = None;
    let mut sib = None;
    let mut displacement = 0i32;
    let mut immediate = 0i32;

    let needs_modrm = matches!(
        def.encoding,
        Encoding::Modrm | Encoding::ModrmImm8 | Encoding::ModrmImm16
    ) || (!two_byte
        && matches!(
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
        ));

    if needs_modrm {
        if i >= bytes.len() {
            return Err(DecodeError::Truncated);
        }
        let m = Modrm::decode(bytes[i]);
        i += 1;
        // MOV to/from control registers (0F 20/22): the mod field is
        // architecturally ignored — the operand is always register-direct
        // and no SIB byte or displacement follows the ModR/M byte, even when
        // the raw mod bits are not 11. Spec: Intel SDM Vol. 2 "MOV—Move
        // to/from Control Registers" ("The 2 bits in the mod field ... are
        // ignored").
        let mov_crn = two_byte && matches!(opcode, 0x20 | 0x22);
        // Address-size attribute from the code-segment default plus 0x67.
        // Spec: Intel SDM Vol. 1 §3.6; Vol. 2 Chapter 2 (ModR/M, SIB, displacement).
        let addr16 = !address_size_32;
        if mov_crn {
            // No SIB/displacement bytes for MOV CRn — see comment above.
        } else if addr16 {
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
        } else if m.mod_ != 3 {
            // 32-bit addressing: optional SIB when rm=4; disp per mod/base.
            if m.rm == 4 {
                if i >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                let sib_byte = bytes[i];
                i += 1;
                sib = Some(sib_byte);
                let base = sib_byte & 7;
                match m.mod_ {
                    0 if base == 5 => {
                        if i + 3 >= bytes.len() {
                            return Err(DecodeError::Truncated);
                        }
                        displacement = i32::from_le_bytes([
                            bytes[i],
                            bytes[i + 1],
                            bytes[i + 2],
                            bytes[i + 3],
                        ]);
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
                        displacement = i32::from_le_bytes([
                            bytes[i],
                            bytes[i + 1],
                            bytes[i + 2],
                            bytes[i + 3],
                        ]);
                        i += 4;
                    }
                    _ => {}
                }
            } else {
                match m.mod_ {
                    0 if m.rm == 5 => {
                        if i + 3 >= bytes.len() {
                            return Err(DecodeError::Truncated);
                        }
                        displacement = i32::from_le_bytes([
                            bytes[i],
                            bytes[i + 1],
                            bytes[i + 2],
                            bytes[i + 3],
                        ]);
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
                        displacement = i32::from_le_bytes([
                            bytes[i],
                            bytes[i + 1],
                            bytes[i + 2],
                            bytes[i + 3],
                        ]);
                        i += 4;
                    }
                    _ => {}
                }
            }
        }
        modrm = Some(m);
    }

    // Group 3 TEST (F6/F7 /0 and /1) takes an immediate; other /r forms do not.
    // Spec: Intel SDM Vol. 2 opcode map — F6 /0,/1 ib; F7 /0,/1 iw|id (OsZ).
    //
    // The immediate rules below (and the `B0`–`BF` MOV-imm ranges) belong to the
    // *primary* map only. The two-byte map reuses those opcode bytes for
    // instructions with no immediate at all (e.g. `0F B6` is `MOVZX Gv,Eb`), so
    // a two-byte opcode must never consume immediate bytes here.
    let grp3_test_imm = !two_byte
        && matches!(opcode, 0xF6 | 0xF7)
        && matches!(modrm.map(|m| m.reg), Some(0) | Some(1));

    if !two_byte && (0xB0..=0xB7).contains(&opcode) {
        if i >= bytes.len() {
            return Err(DecodeError::Truncated);
        }
        immediate = i32::from(bytes[i]);
        i += 1;
    } else if !two_byte && (0xB8..=0xBF).contains(&opcode) {
        if operand_size_32 {
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
        } else if operand_size_32 {
            // F7 /0,/1 id — OsZ32. Spec: Intel SDM Vol. 2 Ch. 2 (66H); opcode map.
            if i + 3 >= bytes.len() {
                return Err(DecodeError::Truncated);
            }
            immediate = i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
            i += 4;
        } else {
            // F7 /0,/1 iw — default 16-bit operand size.
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
                // OsZ immediate: Imm16 default, Imm32 with 0x66 in 16-bit default modes.
                // Spec: Intel SDM Vol. 2 Ch. 2 (operand-size override); opcode map 81/C7.
                if operand_size_32 {
                    if i + 3 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate =
                        i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
                    i += 4;
                } else {
                    if i + 1 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate = i32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
                    i += 2;
                }
            }
            Encoding::Rel8 => {
                if i >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                immediate = i32::from(bytes[i] as i8);
                i += 1;
            }
            Encoding::Rel16 => {
                // Near CALL/JMP: rel16 default; rel32 with operand-size override.
                // Spec: Intel SDM Vol. 2 Ch. 2; "CALL"/"JMP" near relative.
                if operand_size_32 {
                    if i + 3 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate =
                        i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
                    i += 4;
                } else {
                    if i + 1 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate = i32::from(i16::from_le_bytes([bytes[i], bytes[i + 1]]));
                    i += 2;
                }
            }
            Encoding::Imm16 => {
                // Most Imm16 encodings follow OsZ (imm16↔imm32). Exceptions: RET/RETF iw
                // (C2/CA) always take a 16-bit immediate stack-release count.
                // Spec: Intel SDM Vol. 2 Ch. 2; "RET" (near/far imm16).
                let always_imm16 = matches!(opcode, 0xC2 | 0xCA);
                if operand_size_32 && !always_imm16 {
                    if i + 3 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate =
                        i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
                    i += 4;
                } else {
                    if i + 1 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate = i32::from(i16::from_le_bytes([bytes[i], bytes[i + 1]]));
                    i += 2;
                }
            }
            Encoding::Moffs => {
                // Absolute moffs — address-size attribute; real-mode default is 16-bit.
                // Spec: Intel SDM Vol. 2, MOV AL/AX/EAX/RAX, moffs / moffs, AL/AX/….
                // 0x67 → moffs32 (unreal-mode high offsets). Unsupported: moffs64 / long mode.
                if address_size_32 {
                    if i + 3 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate =
                        i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
                    i += 4;
                } else {
                    if i + 1 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate = i32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
                    i += 2;
                }
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
                // Far pointer: offset then segment (segment in `displacement`).
                // Spec: Intel SDM Vol. 2 CALL/JMP far ptr16:16 / ptr16:32; Ch. 2 (66H).
                // Operand-size 16 → offset16+selector16; 0x66 → offset32+selector16.
                if operand_size_32 {
                    if i + 5 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate =
                        i32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
                    displacement = i32::from(u16::from_le_bytes([bytes[i + 4], bytes[i + 5]]));
                    i += 6;
                } else {
                    if i + 3 >= bytes.len() {
                        return Err(DecodeError::Truncated);
                    }
                    immediate = i32::from(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
                    displacement = i32::from(u16::from_le_bytes([bytes[i + 2], bytes[i + 3]]));
                    i += 4;
                }
            }
            Encoding::None | Encoding::Modrm | Encoding::OpcodeReg => {}
        }
    }

    if i > 15 {
        return Err(DecodeError::TooLong);
    }

    // Group 1 / 2 / 3 / 4 / 5: mnemonic from ModRM.reg (Intel SDM Vol. 2 opcode map).
    // These groups live in the *primary* map only; the two-byte map reuses the
    // same opcode bytes for unrelated instructions (e.g. `0F 80` is `JO rel16`).
    let mnemonic = if two_byte {
        // Group 8 (`0F BA`): ModR/M.reg selects the bit operation; /0–/3 are
        // reserved and keep the group placeholder (#UD at execute).
        if opcode == 0xBA {
            match modrm.map(|m| m.reg) {
                Some(4) => "BT",
                Some(5) => "BTS",
                Some(6) => "BTR",
                Some(7) => "BTC",
                _ => def.mnemonic,
            }
        } else {
            def.mnemonic
        }
    } else if matches!(opcode, 0x80 | 0x81 | 0x83) {
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
        two_byte,
        modrm,
        sib,
        displacement,
        immediate,
        length: i,
        mnemonic,
        operand_size_32,
        address_size_32,
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
        // Lone 0F waits for secondary; unknown secondary is unsupported.
        assert_eq!(decode(&[0x0F]), Err(DecodeError::Truncated));
        // 0F 01 is GRP7 (needs ModRM); truncated without ModRM.
        assert_eq!(decode(&[0x0F, 0x01]), Err(DecodeError::Truncated));
        assert!(matches!(
            decode(&[0x0F, 0x02]),
            Err(DecodeError::UnsupportedOpcode(0x02))
        ));
    }

    #[test]
    fn decode_lgdt_sgdt_m16() {
        // Spec: Intel SDM Vol. 2 STR/LTR — 0F 00 /1 and /3.
        let str_ax = decode(&[0x0F, 0x00, 0xC8]).unwrap(); // STR AX
        assert_eq!(str_ax.mnemonic, "GRP6");
        assert!(str_ax.two_byte);
        assert_eq!(str_ax.opcode, 0x00);
        assert_eq!(str_ax.modrm.unwrap().reg, 1);
        assert_eq!(str_ax.length, 3);

        let ltr_ax = decode(&[0x0F, 0x00, 0xD8]).unwrap(); // LTR AX
        assert_eq!(ltr_ax.modrm.unwrap().reg, 3);
        assert_eq!(ltr_ax.length, 3);

        // Spec: Intel SDM Vol. 2 LGDT/SGDT — 0F 01 /2 and /0, memory form.
        let sgdt = decode(&[0x0F, 0x01, 0x06, 0x00, 0x20]).unwrap(); // SGDT [0x2000]
        assert_eq!(sgdt.mnemonic, "GRP7");
        assert!(sgdt.two_byte);
        assert_eq!(sgdt.opcode, 0x01);
        assert_eq!(sgdt.modrm.unwrap().reg, 0);
        assert_eq!(sgdt.length, 5);

        let lgdt = decode(&[0x0F, 0x01, 0x16, 0x00, 0x20]).unwrap(); // LGDT [0x2000]
        assert_eq!(lgdt.modrm.unwrap().reg, 2);
        assert_eq!(lgdt.length, 5);

        let lgdt32 = decode(&[0x66, 0x0F, 0x01, 0x16, 0x00, 0x20]).unwrap();
        assert!(lgdt32.prefixes.op_size_override);
        assert_eq!(lgdt32.length, 6);
    }

    #[test]
    fn decode_lidt_sidt_m16() {
        // Spec: Intel SDM Vol. 2 LIDT/SIDT — 0F 01 /3 and /1, memory form.
        let sidt = decode(&[0x0F, 0x01, 0x0E, 0x00, 0x20]).unwrap(); // SIDT [0x2000]
        assert_eq!(sidt.mnemonic, "GRP7");
        assert!(sidt.two_byte);
        assert_eq!(sidt.opcode, 0x01);
        assert_eq!(sidt.modrm.unwrap().reg, 1);
        assert_eq!(sidt.length, 5);

        let lidt = decode(&[0x0F, 0x01, 0x1E, 0x00, 0x20]).unwrap(); // LIDT [0x2000]
        assert_eq!(lidt.modrm.unwrap().reg, 3);
        assert_eq!(lidt.length, 5);

        let lidt32 = decode(&[0x66, 0x0F, 0x01, 0x1E, 0x00, 0x20]).unwrap();
        assert!(lidt32.prefixes.op_size_override);
        assert_eq!(lidt32.length, 6);
    }

    #[test]
    fn decode_smsw_lmsw() {
        // Spec: Intel SDM Vol. 2 SMSW/LMSW — 0F 01 /4 and /6.
        let smsw_ax = decode(&[0x0F, 0x01, 0xE0]).unwrap(); // SMSW AX
        assert_eq!(smsw_ax.modrm.unwrap().reg, 4);
        assert_eq!(smsw_ax.modrm.unwrap().rm, 0);
        assert_eq!(smsw_ax.length, 3);

        let smsw_m = decode(&[0x0F, 0x01, 0x26, 0x00, 0x20]).unwrap(); // SMSW [0x2000]
        assert_eq!(smsw_m.modrm.unwrap().reg, 4);
        assert_eq!(smsw_m.length, 5);

        let lmsw_ax = decode(&[0x0F, 0x01, 0xF0]).unwrap(); // LMSW AX
        assert_eq!(lmsw_ax.modrm.unwrap().reg, 6);
        assert_eq!(lmsw_ax.length, 3);
    }

    #[test]
    fn decode_invlpg() {
        // Spec: Intel SDM Vol. 2 INVLPG — 0F 01 /7, memory form (register form #UD at execute).
        let mem = decode(&[0x0F, 0x01, 0x3E, 0x00, 0x20]).unwrap(); // INVLPG [0x2000]
        assert_eq!(mem.mnemonic, "GRP7");
        assert!(mem.two_byte);
        assert_eq!(mem.opcode, 0x01);
        assert_eq!(mem.modrm.unwrap().reg, 7);
        assert_ne!(mem.modrm.unwrap().mod_, 3);
        assert_eq!(mem.length, 5);

        let reg = decode(&[0x0F, 0x01, 0xF8]).unwrap(); // INVLPG EAX (mod=11) — decodes; #UD later
        assert_eq!(reg.modrm.unwrap().reg, 7);
        assert_eq!(reg.modrm.unwrap().mod_, 3);
        assert_eq!(reg.length, 3);
    }

    #[test]
    fn decode_clts() {
        // Spec: Intel SDM Vol. 2 "CLTS—Clear Task-Switched Flag in CR0" — 0F 06.
        let clts = decode(&[0x0F, 0x06]).unwrap();
        assert!(clts.two_byte);
        assert_eq!(clts.opcode, 0x06);
        assert_eq!(clts.mnemonic, "CLTS");
        assert!(clts.modrm.is_none());
        assert_eq!(clts.length, 2);
    }

    #[test]
    fn decode_mov_cr0() {
        // Spec: Intel SDM Vol. 2 "MOV—Move to/from Control Registers" — 0F 20/22 /r.
        let mov_eax_cr0 = decode(&[0x0F, 0x20, 0xC0]).unwrap(); // MOV EAX, CR0
        assert!(mov_eax_cr0.two_byte);
        assert_eq!(mov_eax_cr0.opcode, 0x20);
        assert_eq!(mov_eax_cr0.modrm.unwrap().reg, 0); // CR0
        assert_eq!(mov_eax_cr0.modrm.unwrap().rm, 0); // EAX
        assert_eq!(mov_eax_cr0.length, 3);

        let mov_cr0_ebx = decode(&[0x0F, 0x22, 0xC3]).unwrap(); // MOV CR0, EBX
        assert!(mov_cr0_ebx.two_byte);
        assert_eq!(mov_cr0_ebx.opcode, 0x22);
        assert_eq!(mov_cr0_ebx.modrm.unwrap().reg, 0); // CR0
        assert_eq!(mov_cr0_ebx.modrm.unwrap().rm, 3); // EBX
        assert_eq!(mov_cr0_ebx.length, 3);

        // CR1 selector (ModRM.reg == 1) still decodes; #UD is an interpreter concern.
        let mov_eax_cr1 = decode(&[0x0F, 0x20, 0xC8]).unwrap(); // MOV EAX, CR1 (reg=1)
        assert_eq!(mov_eax_cr1.modrm.unwrap().reg, 1);

        // Spec: mod field is ignored for MOV to/from control registers — the CPU
        // always treats the operand as register-direct and does not consume a
        // SIB byte or displacement even when mod != 11. A non-3 mod byte here
        // (0x40 = mod=01, reg=0, rm=0) must decode as if mod were 3, leaving the
        // trailing byte (0xFD = STD) as the *next* instruction.
        let ignored_mod = decode(&[0x0F, 0x20, 0x40, 0xFD]).unwrap(); // MOV EAX, CR0; STD follows
        assert_eq!(ignored_mod.modrm.unwrap().rm, 0); // EAX, not [EAX+disp8]
        assert_eq!(ignored_mod.length, 3, "mod/disp8 byte must not be consumed");
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
    fn decode_call_jmp_far_ptr16_32_opsize() {
        // Intel SDM Vol. 2: CALL/JMP ptr16:32 with 66H — offset32 then selector16.
        let d = decode(&[0x66, 0x9A, 0x78, 0x56, 0x34, 0x12, 0x00, 0xF0]).unwrap();
        assert!(d.prefixes.op_size_override);
        assert_eq!(d.mnemonic, "CALL_FAR");
        assert_eq!(d.immediate, 0x1234_5678u32 as i32);
        assert_eq!(d.displacement, 0xF000);
        assert_eq!(d.length, 8);
        let d = decode(&[0x66, 0xEA, 0x00, 0x02, 0x00, 0x00, 0x00, 0x10]).unwrap();
        assert!(d.prefixes.op_size_override);
        assert_eq!(d.mnemonic, "JMP_FAR");
        assert_eq!(d.immediate, 0x0000_0200);
        assert_eq!(d.displacement, 0x1000);
        assert_eq!(d.length, 8);
        assert_eq!(
            decode(&[0x66, 0x9A, 0x00, 0x00, 0x00, 0x00]),
            Err(DecodeError::Truncated)
        );
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
    fn decode_into_bound() {
        // Intel SDM Vol. 2: INTO — CE; BOUND r16, m16&16 — 62 /r
        let into = decode(&[0xCE]).unwrap();
        assert_eq!(into.mnemonic, "INTO");
        assert_eq!(into.length, 1);
        assert!(into.modrm.is_none());

        // 62 06 00 20 = BOUND AX, [0x2000]
        let bound = decode(&[0x62, 0x06, 0x00, 0x20]).unwrap();
        assert_eq!(bound.mnemonic, "BOUND");
        assert_eq!(bound.modrm.unwrap().reg, 0);
        assert_eq!(bound.displacement, 0x2000);
        assert_eq!(bound.length, 4);

        assert_eq!(decode(&[0x62]), Err(DecodeError::Truncated));
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
        // Intel SDM Vol. 2: MOVSB/STOSB/LODSB/CMPSB/SCASB — A4/AA/AC/A6/AE
        assert_eq!(decode(&[0xA4]).unwrap().mnemonic, "MOVSB");
        assert_eq!(decode(&[0xAA]).unwrap().mnemonic, "STOSB");
        assert_eq!(decode(&[0xAC]).unwrap().mnemonic, "LODSB");
        assert_eq!(decode(&[0xA6]).unwrap().mnemonic, "CMPSB");
        assert_eq!(decode(&[0xAE]).unwrap().mnemonic, "SCASB");
    }

    #[test]
    fn decode_string_word_ops() {
        // Intel SDM Vol. 2: MOVSW/STOSW/LODSW/CMPSW/SCASW — A5/AB/AD/A7/AF
        // (under 0x66 these are dword forms MOVSD/…; mnemonic stays *W in this decoder)
        assert_eq!(decode(&[0xA5]).unwrap().mnemonic, "MOVSW");
        assert_eq!(decode(&[0xAB]).unwrap().mnemonic, "STOSW");
        assert_eq!(decode(&[0xAD]).unwrap().mnemonic, "LODSW");
        assert_eq!(decode(&[0xA7]).unwrap().mnemonic, "CMPSW");
        assert_eq!(decode(&[0xAF]).unwrap().mnemonic, "SCASW");
        let opsz = decode(&[0x66, 0xA5]).unwrap();
        assert!(opsz.prefixes.op_size_override);
        assert_eq!(opsz.mnemonic, "MOVSW");
        assert_eq!(opsz.length, 2);
    }

    #[test]
    fn decode_ins_outs() {
        // Intel SDM Vol. 2: INS/INSB/INSW/INSD — 6C/6D; OUTS/OUTSB/OUTSW/OUTSD — 6E/6F.
        // Under 0x66, 6D/6F are dword forms; mnemonic stays *W in this decoder.
        assert_eq!(decode(&[0x6C]).unwrap().mnemonic, "INSB");
        assert_eq!(decode(&[0x6D]).unwrap().mnemonic, "INSW");
        assert_eq!(decode(&[0x6E]).unwrap().mnemonic, "OUTSB");
        assert_eq!(decode(&[0x6F]).unwrap().mnemonic, "OUTSW");
        let rep_ins = decode(&[0xF3, 0x6C]).unwrap();
        assert!(rep_ins.prefixes.rep);
        assert_eq!(rep_ins.mnemonic, "INSB");
        assert_eq!(rep_ins.length, 2);
        let opsz = decode(&[0x66, 0x6D]).unwrap();
        assert!(opsz.prefixes.op_size_override);
        assert_eq!(opsz.mnemonic, "INSW");
        assert_eq!(opsz.length, 2);
        let rep_outsd = decode(&[0xF3, 0x66, 0x6F]).unwrap();
        assert!(rep_outsd.prefixes.rep);
        assert!(rep_outsd.prefixes.op_size_override);
        assert_eq!(rep_outsd.mnemonic, "OUTSW");
    }

    #[test]
    fn decode_rep_prefixes_on_string_ops() {
        // Intel SDM Vol. 2: REP/REPE = F3, REPNE = F2 (legacy prefixes).
        let rep_stos = decode(&[0xF3, 0xAA]).unwrap();
        assert!(rep_stos.prefixes.rep);
        assert!(!rep_stos.prefixes.repne);
        assert_eq!(rep_stos.mnemonic, "STOSB");
        assert_eq!(rep_stos.length, 2);

        let repe_scas = decode(&[0xF3, 0xAE]).unwrap();
        assert!(repe_scas.prefixes.rep);
        assert!(!repe_scas.prefixes.repne);

        let repne_cmps = decode(&[0xF2, 0xA6]).unwrap();
        assert!(repne_cmps.prefixes.repne);
        assert!(!repne_cmps.prefixes.rep);

        // Word forms accept the same REP prefixes.
        let rep_movsw = decode(&[0xF3, 0xA5]).unwrap();
        assert!(rep_movsw.prefixes.rep);
        assert_eq!(rep_movsw.mnemonic, "MOVSW");
        let repe_scasw = decode(&[0xF3, 0xAF]).unwrap();
        assert!(repe_scasw.prefixes.rep);
        assert_eq!(repe_scasw.mnemonic, "SCASW");
        let repne_cmpsw = decode(&[0xF2, 0xA7]).unwrap();
        assert!(repne_cmpsw.prefixes.repne);
        assert_eq!(repne_cmpsw.mnemonic, "CMPSW");

        // Last F2/F3 wins (mutually exclusive).
        let last_f2 = decode(&[0xF3, 0xF2, 0xA4]).unwrap();
        assert!(last_f2.prefixes.repne);
        assert!(!last_f2.prefixes.rep);
        let last_f3 = decode(&[0xF2, 0xF3, 0xA4]).unwrap();
        assert!(last_f3.prefixes.rep);
        assert!(!last_f3.prefixes.repne);
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
    fn decode_imul_imm_69_6b() {
        // Intel SDM Vol. 2 "IMUL": 69 /r iw — IMUL r16, r/m16, imm16; 6B /r ib — imm8.
        // ModRM.reg = dest, ModRM.rm = src. 0xD8 = mod3 reg=BX rm=AX.
        let d = decode(&[0x69, 0xD8, 0x34, 0x12]).unwrap(); // IMUL BX, AX, 0x1234
        assert_eq!(d.mnemonic, "IMUL");
        assert_eq!(d.opcode, 0x69);
        assert_eq!(d.modrm.unwrap().reg, 3); // BX
        assert_eq!(d.modrm.unwrap().rm, 0); // AX
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 4);

        let d = decode(&[0x6B, 0xD8, 0xFF]).unwrap(); // IMUL BX, AX, -1 (imm8)
        assert_eq!(d.mnemonic, "IMUL");
        assert_eq!(d.opcode, 0x6B);
        assert_eq!(d.immediate, 0xFF);
        assert_eq!(d.length, 3);

        // Memory form: 69 1E 00 40 02 00 = IMUL BX, [0x4000], 2
        let d = decode(&[0x69, 0x1E, 0x00, 0x40, 0x02, 0x00]).unwrap();
        assert_eq!(d.mnemonic, "IMUL");
        assert_eq!(d.modrm.unwrap().mod_, 0);
        assert_eq!(d.modrm.unwrap().rm, 6);
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.immediate, 2);
        assert_eq!(d.length, 6);

        assert_eq!(decode(&[0x69, 0xD8]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x6B, 0xD8]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x69]), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_imul_0f_af() {
        // Intel SDM Vol. 2 "IMUL": 0F AF /r — IMUL r16, r/m16 (two-operand).
        // Distinct from primary AF (SCASW).
        let d = decode(&[0x0F, 0xAF, 0xD8]).unwrap(); // IMUL BX, AX
        assert_eq!(d.mnemonic, "IMUL");
        assert_eq!(d.opcode, 0xAF);
        assert!(d.two_byte);
        assert_eq!(d.modrm.unwrap().reg, 3); // BX
        assert_eq!(d.modrm.unwrap().rm, 0); // AX
        assert_eq!(d.length, 3);

        // Memory: 0F AF 1E 00 40 = IMUL BX, [0x4000]
        let d = decode(&[0x0F, 0xAF, 0x1E, 0x00, 0x40]).unwrap();
        assert_eq!(d.mnemonic, "IMUL");
        assert!(d.two_byte);
        assert_eq!(d.length, 5);

        // Opsize 32: 66 0F AF C3 = IMUL EAX, EBX
        let d = decode(&[0x66, 0x0F, 0xAF, 0xC3]).unwrap();
        assert_eq!(d.mnemonic, "IMUL");
        assert!(d.two_byte);
        assert!(d.prefixes.op_size_override);
        assert_eq!(d.modrm.unwrap().reg, 0); // EAX
        assert_eq!(d.modrm.unwrap().rm, 3); // EBX
        assert_eq!(d.length, 4);

        // Truncated escape / unknown secondary.
        assert_eq!(decode(&[0x0F]), Err(DecodeError::Truncated));
        // GRP6 (0F 00) needs a ModRM byte.
        assert_eq!(decode(&[0x0F, 0x00]), Err(DecodeError::Truncated));
        assert!(matches!(
            decode(&[0x0F, 0x04]),
            Err(DecodeError::UnsupportedOpcode(0x04))
        ));
        // Primary AF remains SCASW (not two-byte).
        let d = decode(&[0xAF]).unwrap();
        assert_eq!(d.mnemonic, "SCASW");
        assert!(!d.two_byte);
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
    fn decode_bcd_adjust() {
        // Intel SDM Vol. 2: DAA (27), DAS (2F), AAA (37), AAS (3F), AAM (D4 ib), AAD (D5 ib).
        assert_eq!(decode(&[0x27]).unwrap().mnemonic, "DAA");
        assert_eq!(decode(&[0x27]).unwrap().length, 1);
        assert_eq!(decode(&[0x2F]).unwrap().mnemonic, "DAS");
        assert_eq!(decode(&[0x37]).unwrap().mnemonic, "AAA");
        assert_eq!(decode(&[0x3F]).unwrap().mnemonic, "AAS");

        let aam = decode(&[0xD4, 0x0A]).unwrap();
        assert_eq!(aam.mnemonic, "AAM");
        assert_eq!(aam.immediate, 0x0A);
        assert_eq!(aam.length, 2);

        let aad = decode(&[0xD5, 0x0A]).unwrap();
        assert_eq!(aad.mnemonic, "AAD");
        assert_eq!(aad.immediate, 0x0A);
        assert_eq!(aad.length, 2);

        // Non-default bases are valid encodings (imm8 is not fixed to 0Ah).
        assert_eq!(decode(&[0xD4, 0x10]).unwrap().immediate, 0x10);
        assert_eq!(decode(&[0xD5, 0x10]).unwrap().immediate, 0x10);

        assert_eq!(decode(&[0xD4]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xD5]), Err(DecodeError::Truncated));
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
        // 0x67 → moffs32 (SDM Vol. 2 MOV address-size attribute).
        let d = decode(&[0x67, 0xA0, 0x00, 0x00, 0x01, 0x00]).unwrap();
        assert!(d.prefixes.addr_size_override);
        assert_eq!(d.immediate, 0x0001_0000);
        assert_eq!(d.length, 6);
        assert_eq!(
            decode(&[0x67, 0xA0, 0x00, 0x00]),
            Err(DecodeError::Truncated)
        );
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
        // 66 A9 id = TEST EAX,imm32 (SDM Vol. 2 TEST; Ch. 2).
        let d = decode(&[0x66, 0xA9, 0xEF, 0xBE, 0xAD, 0xDE]).unwrap();
        assert!(d.prefixes.op_size_override);
        assert_eq!(d.immediate, 0xDEAD_BEEFu32 as i32);
        assert_eq!(d.length, 6);
        assert_eq!(decode(&[0xA8]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xA9, 0x00]), Err(DecodeError::Truncated));
        assert_eq!(
            decode(&[0x66, 0xA9, 0x00, 0x00]),
            Err(DecodeError::Truncated)
        );
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

    #[test]
    fn decode_les_lds_xlat() {
        // Intel SDM Vol. 2: LES r16, m16:16 (C4 /r); LDS r16, m16:16 (C5 /r); XLAT/XLATB (D7).
        // C4 06 00 20 = LES AX, [0x2000]
        let les = decode(&[0xC4, 0x06, 0x00, 0x20]).unwrap();
        assert_eq!(les.mnemonic, "LES");
        assert_eq!(les.modrm.unwrap().reg, 0);
        assert_eq!(les.displacement, 0x2000);
        assert_eq!(les.length, 4);

        // C5 1E 34 12 = LDS BX, [0x1234]
        let lds = decode(&[0xC5, 0x1E, 0x34, 0x12]).unwrap();
        assert_eq!(lds.mnemonic, "LDS");
        assert_eq!(lds.modrm.unwrap().reg, 3);
        assert_eq!(lds.displacement, 0x1234);
        assert_eq!(lds.length, 4);

        let xlat = decode(&[0xD7]).unwrap();
        assert_eq!(xlat.mnemonic, "XLAT");
        assert_eq!(xlat.length, 1);
        assert!(xlat.modrm.is_none());

        assert_eq!(decode(&[0xC4]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0xC5]), Err(DecodeError::Truncated));
        assert_eq!(decode(&[0x26, 0xD7]).unwrap().mnemonic, "XLAT");
        assert_eq!(
            decode(&[0x26, 0xD7]).unwrap().prefixes.segment_override,
            Some(0x26)
        );
    }

    /// Operand-size override 0x66 selects 32-bit immediates / rel32 in real mode.
    /// Spec: Intel SDM Vol. 2 Chapter 2 (66H operand-size override); Vol. 1 §3.6.
    #[test]
    fn decode_opsize32_imm_and_rel() {
        // MOV EAX, imm32 — 66 B8 id
        let d = decode(&[0x66, 0xB8, 0x78, 0x56, 0x34, 0x12]).unwrap();
        assert!(d.prefixes.op_size_override);
        assert_eq!(d.opcode, 0xB8);
        assert_eq!(d.immediate, 0x1234_5678);
        assert_eq!(d.length, 6);

        // Without 0x66: MOV AX, imm16
        let d = decode(&[0xB8, 0x78, 0x56]).unwrap();
        assert!(!d.prefixes.op_size_override);
        assert_eq!(d.immediate, 0x5678);
        assert_eq!(d.length, 3);

        // ADD EAX, imm32 — 66 05 id
        let d = decode(&[0x66, 0x05, 0x01, 0x00, 0x00, 0x80]).unwrap();
        assert_eq!(d.immediate as u32, 0x8000_0001);
        assert_eq!(d.length, 6);

        // PUSH imm32 — 66 68 id
        let d = decode(&[0x66, 0x68, 0xEF, 0xBE, 0xAD, 0xDE]).unwrap();
        assert_eq!(d.immediate as u32, 0xDEAD_BEEF);
        assert_eq!(d.length, 6);

        // MOV r/m32, imm32 — 66 C7 /0 id
        let d = decode(&[0x66, 0xC7, 0xC0, 0x44, 0x33, 0x22, 0x11]).unwrap();
        assert_eq!(d.immediate as u32, 0x1122_3344);
        assert_eq!(d.length, 7);

        // Group1 r/m32, imm32 — 66 81 /0 id
        let d = decode(&[0x66, 0x81, 0xC3, 0x00, 0x00, 0x00, 0x01]).unwrap();
        assert_eq!(d.immediate as u32, 0x0100_0000);
        assert_eq!(d.length, 7);

        // Near JMP rel32 — 66 E9 cd
        let d = decode(&[0x66, 0xE9, 0x10, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(d.immediate, 0x10);
        assert_eq!(d.length, 6);

        // Near CALL rel32 — 66 E8 cd
        let d = decode(&[0x66, 0xE8, 0x04, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(d.immediate, 0x04);
        assert_eq!(d.length, 6);

        // RET iw keeps imm16 even with 0x66 (stack-release count).
        let d = decode(&[0x66, 0xC2, 0x04, 0x00]).unwrap();
        assert!(d.prefixes.op_size_override);
        assert_eq!(d.immediate, 4);
        assert_eq!(d.length, 4);

        // Group 3 TEST r/m32, imm32 — 66 F7 /0 id
        // Spec: Intel SDM Vol. 2 opcode map F7 /0 id; Ch. 2 (66H).
        let d = decode(&[0x66, 0xF7, 0xC0, 0xEF, 0xBE, 0xAD, 0xDE]).unwrap();
        assert!(d.prefixes.op_size_override);
        assert_eq!(d.immediate as u32, 0xDEAD_BEEF);
        assert_eq!(d.length, 7);
        assert_eq!(
            decode(&[0x66, 0xF7, 0xC0, 0x00, 0x00]),
            Err(DecodeError::Truncated)
        );

        // IMUL r32, r/m32, imm32 — 66 69 /r id (ModrmImm16 follows OsZ).
        // Spec: Intel SDM Vol. 2 "IMUL"; Ch. 2 (66H).
        let d = decode(&[0x66, 0x69, 0xD8, 0x78, 0x56, 0x34, 0x12]).unwrap();
        assert!(d.prefixes.op_size_override);
        assert_eq!(d.immediate as u32, 0x1234_5678);
        assert_eq!(d.length, 7);
        assert_eq!(
            decode(&[0x66, 0x69, 0xD8, 0x00, 0x00]),
            Err(DecodeError::Truncated)
        );
    }

    /// Address-size override 0x67: 32-bit ModRM displacement / SIB forms.
    /// Spec: Intel SDM Vol. 1 §3.6; Vol. 2 Chapter 2 (address-size attribute, SIB).
    #[test]
    fn decode_asize32_modrm_disp_and_sib() {
        // 67 8B 03 = MOV r16, [EBX] (mod=0 rm=3, no disp)
        let d = decode(&[0x67, 0x8B, 0x03]).unwrap();
        assert!(d.prefixes.addr_size_override);
        assert_eq!(d.modrm.unwrap().rm, 3);
        assert_eq!(d.displacement, 0);
        assert!(d.sib.is_none());
        assert_eq!(d.length, 3);

        // 67 8B 05 78 56 34 12 = MOV r16, [0x12345678] (mod=0 rm=5 disp32)
        let d = decode(&[0x67, 0x8B, 0x05, 0x78, 0x56, 0x34, 0x12]).unwrap();
        assert_eq!(d.displacement, 0x1234_5678);
        assert_eq!(d.length, 7);

        // 67 8B 43 04 = MOV r16, [EBX+0x04] (mod=1 disp8)
        let d = decode(&[0x67, 0x8B, 0x43, 0x04]).unwrap();
        assert_eq!(d.displacement, 4);
        assert_eq!(d.length, 4);

        // 67 8B 83 00 10 00 00 = MOV r16, [EBX+0x1000] (mod=2 disp32)
        let d = decode(&[0x67, 0x8B, 0x83, 0x00, 0x10, 0x00, 0x00]).unwrap();
        assert_eq!(d.displacement, 0x1000);
        assert_eq!(d.length, 7);

        // 67 8B 44 24 08 = MOV r16, [ESP+8] (mod=1 rm=4 SIB=0x24 disp8)
        // Spec: SDM Vol. 2 Chapter 2 — SIB with base=ESP, index=none (index=4).
        let d = decode(&[0x67, 0x8B, 0x44, 0x24, 0x08]).unwrap();
        assert_eq!(d.modrm.unwrap().rm, 4);
        assert_eq!(d.sib, Some(0x24));
        assert_eq!(d.displacement, 8);
        assert_eq!(d.length, 5);

        // 67 8B 04 85 00 20 00 00 = MOV r16, [EAX*4 + 0x2000]
        // mod=0 rm=4; SIB scale=2 index=0 base=5 → disp32, no base reg.
        let d = decode(&[0x67, 0x8B, 0x04, 0x85, 0x00, 0x20, 0x00, 0x00]).unwrap();
        assert_eq!(d.sib, Some(0x85));
        assert_eq!(d.displacement, 0x2000);
        assert_eq!(d.length, 8);
    }

    /// Intel SDM Vol. 1 §3.6 Table 3-4; Vol. 2 Ch. 2 (66H/67H): the code
    /// segment D flag picks the default operand/address size and the override
    /// prefixes select the *other* size, so `0x66`/`0x67` invert under D=1.
    #[test]
    fn decode_mode_defaults_are_inverted_by_override_prefixes() {
        assert_eq!(DecodeMode::default(), DecodeMode::LEGACY16);
        assert_eq!(DecodeMode::from_cs_default_big(false), DecodeMode::LEGACY16);
        assert_eq!(DecodeMode::from_cs_default_big(true), DecodeMode::DEFAULT32);

        // B8 id — MOV EAX, imm32 without any prefix under D=1.
        let d = decode_with_mode(&[0xB8, 0x78, 0x56, 0x34, 0x12], DecodeMode::DEFAULT32).unwrap();
        assert!(d.operand_size_32);
        assert_eq!(d.immediate, 0x1234_5678);
        assert_eq!(d.length, 5);

        // 66 B8 iw — the override selects 16-bit operand size under D=1.
        let d = decode_with_mode(&[0x66, 0xB8, 0x34, 0x12], DecodeMode::DEFAULT32).unwrap();
        assert!(!d.operand_size_32);
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 4);

        // The same bytes keep their 16-bit default meaning under D=0.
        let d = decode_with_mode(&[0xB8, 0x34, 0x12], DecodeMode::LEGACY16).unwrap();
        assert!(!d.operand_size_32);
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 3);
        assert_eq!(decode(&[0xB8, 0x34, 0x12]).unwrap(), d);
    }

    /// Intel SDM Vol. 2 Ch. 2 (ModR/M, SIB, displacement): D=1 makes 32-bit
    /// addressing the default and `0x67` selects the 16-bit ModR/M forms.
    #[test]
    fn decode_mode_default32_selects_32_bit_addressing_forms() {
        // 8B 05 id — MOV EAX, [disp32] (mod=0 rm=5) with no prefix under D=1.
        let d =
            decode_with_mode(&[0x8B, 0x05, 0x78, 0x56, 0x34, 0x12], DecodeMode::DEFAULT32).unwrap();
        assert!(d.address_size_32);
        assert_eq!(d.displacement, 0x1234_5678);
        assert_eq!(d.length, 6);

        // 8B 44 24 08 — MOV EAX, [ESP+8]: SIB is decoded without 0x67 under D=1.
        let d = decode_with_mode(&[0x8B, 0x44, 0x24, 0x08], DecodeMode::DEFAULT32).unwrap();
        assert_eq!(d.sib, Some(0x24));
        assert_eq!(d.displacement, 8);
        assert_eq!(d.length, 4);

        // 67 8B 06 iw — the override restores the 16-bit [disp16] form.
        let d = decode_with_mode(&[0x67, 0x8B, 0x06, 0x00, 0x30], DecodeMode::DEFAULT32).unwrap();
        assert!(!d.address_size_32);
        assert!(d.sib.is_none());
        assert_eq!(d.displacement, 0x3000);
        assert_eq!(d.length, 5);
    }

    /// Intel SDM Vol. 2 "JMP"/"CALL" (near relative) and Ch. 2: the
    /// displacement width follows the effective operand-size attribute.
    #[test]
    fn decode_mode_default32_widens_relative_and_pointer_operands() {
        let d = decode_with_mode(&[0xE9, 0x00, 0x10, 0x00, 0x00], DecodeMode::DEFAULT32).unwrap();
        assert_eq!(d.mnemonic, "JMP");
        assert_eq!(d.immediate, 0x1000);
        assert_eq!(d.length, 5);

        let d = decode_with_mode(&[0x66, 0xE8, 0x05, 0x00], DecodeMode::DEFAULT32).unwrap();
        assert_eq!(d.mnemonic, "CALL");
        assert_eq!(d.immediate, 5);
        assert_eq!(d.length, 4);

        // EA cd — JMP far ptr16:32 is the D=1 default form.
        let d = decode_with_mode(
            &[0xEA, 0x00, 0x20, 0x00, 0x00, 0x18, 0x00],
            DecodeMode::DEFAULT32,
        )
        .unwrap();
        assert_eq!(d.immediate, 0x2000);
        assert_eq!(d.displacement, 0x0018);
        assert_eq!(d.length, 7);

        // Truncated 32-bit forms still fail cleanly.
        assert_eq!(
            decode_with_mode(&[0xE9, 0x00, 0x10, 0x00], DecodeMode::DEFAULT32),
            Err(DecodeError::Truncated)
        );
    }

    /// Intel SDM Vol. 2 "Jcc" (near form): `0F 80`+cc takes a rel16 under a
    /// 16-bit operand size and a rel32 under a 32-bit one, in both code-segment
    /// defaults. The primary-map Group 1 opcodes `80`/`81`/`83` must not leak
    /// their ModR/M.reg mnemonics into the two-byte map.
    #[test]
    fn decode_jcc_near_rel16_rel32() {
        const MNEMONICS: [&str; 16] = [
            "JO", "JNO", "JB", "JAE", "JE", "JNE", "JBE", "JA", "JS", "JNS", "JP", "JNP", "JL",
            "JGE", "JLE", "JG",
        ];
        for cc in 0u8..16 {
            let d = decode(&[0x0F, 0x80 | cc, 0x34, 0x12]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.opcode, 0x80 | cc);
            assert_eq!(d.mnemonic, MNEMONICS[cc as usize]);
            assert!(d.modrm.is_none());
            assert_eq!(d.immediate, 0x1234);
            assert_eq!(d.length, 4);
            assert!(!d.operand_size_32);
        }

        // Negative rel16 sign-extends.
        let d = decode(&[0x0F, 0x85, 0x8E, 0xF9]).unwrap(); // SeaBIOS reset-vector JNZ
        assert_eq!(d.mnemonic, "JNE");
        assert_eq!(d.immediate, -1650);
        assert_eq!(d.length, 4);

        // 66 0F 8x cd — rel32 under a 16-bit code segment.
        let d = decode(&[0x66, 0x0F, 0x84, 0x78, 0x56, 0x34, 0x12]).unwrap();
        assert_eq!(d.mnemonic, "JE");
        assert!(d.operand_size_32);
        assert_eq!(d.immediate, 0x1234_5678);
        assert_eq!(d.length, 7);

        // D=1 defaults to rel32; 0x66 selects rel16.
        let d =
            decode_with_mode(&[0x0F, 0x8F, 0x00, 0x10, 0x00, 0x00], DecodeMode::DEFAULT32).unwrap();
        assert_eq!(d.mnemonic, "JG");
        assert!(d.operand_size_32);
        assert_eq!(d.immediate, 0x1000);
        assert_eq!(d.length, 6);
        let d = decode_with_mode(&[0x66, 0x0F, 0x8F, 0x00, 0x10], DecodeMode::DEFAULT32).unwrap();
        assert!(!d.operand_size_32);
        assert_eq!(d.immediate, 0x1000);
        assert_eq!(d.length, 5);

        // Truncated displacements fail cleanly in both widths.
        assert_eq!(decode(&[0x0F, 0x85, 0x00]), Err(DecodeError::Truncated));
        assert_eq!(
            decode(&[0x66, 0x0F, 0x85, 0x00, 0x00, 0x00]),
            Err(DecodeError::Truncated)
        );
    }

    /// Intel SDM Vol. 2 "SETcc": `0F 90`+cc /r always has a byte destination,
    /// register or memory, and the operand-size prefix does not change it.
    #[test]
    fn decode_setcc_rm8() {
        const MNEMONICS: [&str; 16] = [
            "SETO", "SETNO", "SETB", "SETAE", "SETE", "SETNE", "SETBE", "SETA", "SETS", "SETNS",
            "SETP", "SETNP", "SETL", "SETGE", "SETLE", "SETG",
        ];
        for cc in 0u8..16 {
            // mod=11, rm=0 → AL.
            let d = decode(&[0x0F, 0x90 | cc, 0xC0]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.opcode, 0x90 | cc);
            assert_eq!(d.mnemonic, MNEMONICS[cc as usize]);
            assert_eq!(d.modrm.unwrap().mod_, 3);
            assert_eq!(d.modrm.unwrap().rm, 0);
            assert_eq!(d.length, 3);
        }

        // Legacy high-byte register form (rm=7 → BH).
        let d = decode(&[0x0F, 0x94, 0xC7]).unwrap();
        assert_eq!(d.modrm.unwrap().rm, 7);
        assert_eq!(d.length, 3);

        // ModR/M.reg is not used by SETcc; a nonzero reg still decodes.
        let d = decode(&[0x0F, 0x94, 0xF8]).unwrap();
        assert_eq!(d.mnemonic, "SETE");
        assert_eq!(d.modrm.unwrap().reg, 7);

        // 0F 95 06 00 40 = SETNE byte [0x4000]
        let d = decode(&[0x0F, 0x95, 0x06, 0x00, 0x40]).unwrap();
        assert_eq!(d.mnemonic, "SETNE");
        assert_eq!(d.modrm.unwrap().mod_, 0);
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.length, 5);

        // 0x66 does not add an operand; the destination stays a byte.
        let d = decode(&[0x66, 0x0F, 0x9F, 0xC3]).unwrap();
        assert_eq!(d.mnemonic, "SETG");
        assert_eq!(d.length, 4);

        // D=1 selects 32-bit addressing for the memory form (SIB decoded).
        let d = decode_with_mode(&[0x0F, 0x94, 0x44, 0x24, 0x08], DecodeMode::DEFAULT32).unwrap();
        assert_eq!(d.sib, Some(0x24));
        assert_eq!(d.displacement, 8);
        assert_eq!(d.length, 5);

        assert_eq!(decode(&[0x0F, 0x94]), Err(DecodeError::Truncated));
        assert_eq!(
            decode(&[0x0F, 0x95, 0x06, 0x00]),
            Err(DecodeError::Truncated)
        );
    }

    /// Intel SDM Vol. 2 "PUSH"/"POP" (opcode map 2): `0F A0`/`A1`/`A8`/`A9`
    /// take no ModR/M and no immediate, in either code-segment default.
    #[test]
    fn decode_push_pop_fs_gs() {
        for (bytes, mnemonic) in [
            ([0x0Fu8, 0xA0], "PUSH_FS"),
            ([0x0F, 0xA1], "POP_FS"),
            ([0x0F, 0xA8], "PUSH_GS"),
            ([0x0F, 0xA9], "POP_GS"),
        ] {
            let d = decode(&bytes).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.mnemonic, mnemonic);
            assert!(d.modrm.is_none());
            assert_eq!(d.immediate, 0);
            assert_eq!(d.length, 2);

            let d = decode_with_mode(&bytes, DecodeMode::DEFAULT32).unwrap();
            assert!(d.operand_size_32);
            assert_eq!(d.length, 2);
        }

        // 66 0F A0 — the override selects the other operand size, still 2+1 bytes.
        let d = decode(&[0x66, 0x0F, 0xA0]).unwrap();
        assert!(d.operand_size_32);
        assert_eq!(d.length, 3);
    }

    /// Intel SDM Vol. 2 "LDS/LES/LFS/LGS/LSS": `0F B2`/`B4`/`B5` are ModR/M
    /// forms with no immediate. The primary map's `B0`–`B7` `MOV r8, imm8`
    /// range must not make them swallow an immediate byte.
    #[test]
    fn decode_lss_lfs_lgs() {
        for (op, mnemonic) in [(0xB2u8, "LSS"), (0xB4, "LFS"), (0xB5, "LGS")] {
            // 0F op 06 00 20 = Lxx AX, [0x2000]
            let d = decode(&[0x0F, op, 0x06, 0x00, 0x20]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.mnemonic, mnemonic);
            assert_eq!(d.modrm.unwrap().reg, 0);
            assert_eq!(d.displacement, 0x2000);
            assert_eq!(d.immediate, 0);
            assert_eq!(d.length, 5, "{mnemonic} must not consume an immediate");

            // Register form decodes; #UD is an interpreter concern.
            let d = decode(&[0x0F, op, 0xC0]).unwrap();
            assert_eq!(d.modrm.unwrap().mod_, 3);
            assert_eq!(d.length, 3);

            assert_eq!(decode(&[0x0F, op]), Err(DecodeError::Truncated));
        }

        // D=1: 32-bit addressing with SIB, still no immediate.
        let d = decode_with_mode(&[0x0F, 0xB2, 0x64, 0x24, 0x04], DecodeMode::DEFAULT32).unwrap();
        assert_eq!(d.sib, Some(0x24));
        assert_eq!(d.displacement, 4);
        assert_eq!(d.length, 5);
    }

    /// Intel SDM Vol. 2 "MOVZX"/"MOVSX": `0F B6`/`B7`/`BE`/`BF` are ModR/M
    /// forms with no immediate, even though the primary map uses `B0`–`BF` for
    /// `MOV r8/r16/r32, imm`.
    #[test]
    fn decode_movzx_movsx() {
        for (op, mnemonic) in [
            (0xB6u8, "MOVZX"),
            (0xB7, "MOVZX"),
            (0xBE, "MOVSX"),
            (0xBF, "MOVSX"),
        ] {
            // mod=11: 0F op D8 = Mxx BX, AL/AX
            let d = decode(&[0x0F, op, 0xD8]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.mnemonic, mnemonic);
            assert_eq!(d.modrm.unwrap().reg, 3);
            assert_eq!(d.modrm.unwrap().rm, 0);
            assert_eq!(d.immediate, 0);
            assert_eq!(d.length, 3, "{mnemonic} must not consume an immediate");

            // Memory form: 0F op 1E 00 40
            let d = decode(&[0x0F, op, 0x1E, 0x00, 0x40]).unwrap();
            assert_eq!(d.displacement, 0x4000);
            assert_eq!(d.length, 5);

            // 66 selects the other destination width without changing length.
            let d = decode(&[0x66, 0x0F, op, 0xD8]).unwrap();
            assert!(d.operand_size_32);
            assert_eq!(d.length, 4);

            assert_eq!(decode(&[0x0F, op]), Err(DecodeError::Truncated));
        }

        // Primary B6/BE keep their `MOV r8, imm8` / `MOV r32, imm32` meanings.
        let d = decode(&[0xB6, 0x12]).unwrap();
        assert!(!d.two_byte);
        assert_eq!(d.immediate, 0x12);
        assert_eq!(d.length, 2);
        let d = decode(&[0xBE, 0x34, 0x12]).unwrap();
        assert!(!d.two_byte);
        assert_eq!(d.immediate, 0x1234);
        assert_eq!(d.length, 3);
    }

    /// Intel SDM Vol. 2 "BT"/"BTS"/"BTR"/"BTC": the register bit-offset forms
    /// are plain ModR/M encodings with no immediate.
    #[test]
    fn decode_bt_family_register_offset_forms() {
        for (op, mnemonic) in [(0xA3u8, "BT"), (0xAB, "BTS"), (0xB3, "BTR"), (0xBB, "BTC")] {
            // 0F op C8 = xx AX, CX (mod 11, reg = CX, rm = AX)
            let d = decode(&[0x0F, op, 0xC8]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.mnemonic, mnemonic);
            assert_eq!(d.modrm.unwrap().reg, 1);
            assert_eq!(d.modrm.unwrap().rm, 0);
            assert_eq!(d.immediate, 0);
            assert_eq!(d.length, 3);

            // 0F op 0E 00 40 = xx [0x4000], CX
            let d = decode(&[0x0F, op, 0x0E, 0x00, 0x40]).unwrap();
            assert_eq!(d.displacement, 0x4000);
            assert_eq!(d.length, 5);

            assert_eq!(decode(&[0x0F, op]), Err(DecodeError::Truncated));
        }
    }

    /// Intel SDM Vol. 2 opcode map 2, Group 8 (`0F BA`): ModR/M.reg selects the
    /// bit operation and an imm8 follows. `/0`–`/3` stay a group placeholder.
    #[test]
    fn decode_grp8_bit_immediate_forms() {
        for (reg, mnemonic) in [(4u8, "BT"), (5, "BTS"), (6, "BTR"), (7, "BTC")] {
            let modrm = 0xC0 | (reg << 3);
            let d = decode(&[0x0F, 0xBA, modrm, 0x05]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.opcode, 0xBA);
            assert_eq!(d.mnemonic, mnemonic);
            assert_eq!(d.modrm.unwrap().reg, reg);
            assert_eq!(d.immediate, 5);
            assert_eq!(d.length, 4);
        }

        // /0–/3 are reserved and keep the group mnemonic.
        assert_eq!(decode(&[0x0F, 0xBA, 0xC0, 0x00]).unwrap().mnemonic, "GRP8");
        assert_eq!(decode(&[0x0F, 0xBA, 0xD8, 0x00]).unwrap().mnemonic, "GRP8");

        // Memory form: 0F BA 26 00 40 09 = BT word [0x4000], 9
        let d = decode(&[0x0F, 0xBA, 0x26, 0x00, 0x40, 0x09]).unwrap();
        assert_eq!(d.mnemonic, "BT");
        assert_eq!(d.displacement, 0x4000);
        assert_eq!(d.immediate, 9);
        assert_eq!(d.length, 6);

        // The imm8 stays one byte under a 32-bit operand size.
        let d = decode_with_mode(&[0x0F, 0xBA, 0xE0, 0x15], DecodeMode::DEFAULT32).unwrap();
        assert!(d.operand_size_32);
        assert_eq!(d.immediate, 0x15);
        assert_eq!(d.length, 4);

        assert_eq!(decode(&[0x0F, 0xBA, 0xE0]), Err(DecodeError::Truncated));
    }

    /// Intel SDM Vol. 2 "BSF"/"BSR"/"BSWAP"/"XADD"/"CMPXCHG": ModR/M forms with
    /// no immediate, plus the register-in-opcode `BSWAP` range.
    #[test]
    fn decode_bit_scan_bswap_xadd_cmpxchg() {
        for (op, mnemonic) in [
            (0xBCu8, "BSF"),
            (0xBD, "BSR"),
            (0xB0, "CMPXCHG"),
            (0xB1, "CMPXCHG"),
            (0xC0, "XADD"),
            (0xC1, "XADD"),
        ] {
            let d = decode(&[0x0F, op, 0xC8]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.mnemonic, mnemonic);
            assert_eq!(d.immediate, 0);
            assert_eq!(d.length, 3, "0F {op:02X} must not consume an immediate");

            let d = decode(&[0x0F, op, 0x0E, 0x00, 0x40]).unwrap();
            assert_eq!(d.displacement, 0x4000);
            assert_eq!(d.length, 5);

            assert_eq!(decode(&[0x0F, op]), Err(DecodeError::Truncated));
        }

        // BSWAP encodes the register in the opcode and takes no ModR/M.
        for reg in 0u8..8 {
            let d = decode(&[0x0F, 0xC8 + reg]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.mnemonic, "BSWAP");
            assert_eq!(d.opcode, 0xC8 + reg);
            assert!(d.modrm.is_none());
            assert_eq!(d.length, 2);
        }
    }

    /// Intel SDM Vol. 2 "INVD"/"WBINVD"/"UD2"/"WRMSR"/"RDMSR"/"CPUID": all are
    /// two-byte opcodes with no ModR/M byte and no immediate.
    #[test]
    fn decode_two_byte_system_and_identification() {
        for (op, mnemonic) in [
            (0x08u8, "INVD"),
            (0x09, "WBINVD"),
            (0x0B, "UD2"),
            (0x30, "WRMSR"),
            (0x32, "RDMSR"),
            (0xA2, "CPUID"),
        ] {
            let d = decode(&[0x0F, op]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.opcode, op);
            assert_eq!(d.mnemonic, mnemonic);
            assert!(d.modrm.is_none());
            assert_eq!(d.immediate, 0);
            assert_eq!(d.length, 2);

            let d = decode_with_mode(&[0x0F, op], DecodeMode::DEFAULT32).unwrap();
            assert_eq!(d.mnemonic, mnemonic);
            assert_eq!(d.length, 2);
        }
    }

    /// Intel SDM Vol. 2 "SHLD"/"SHRD" (opcode map 2): the `imm8` forms
    /// (`0F A4`/`0F AC`) consume exactly one immediate byte at every operand
    /// size; the `CL` forms (`0F A5`/`0F AD`) consume none.
    #[test]
    fn decode_double_precision_shifts() {
        for (op, mnemonic) in [(0xA4u8, "SHLD"), (0xAC, "SHRD")] {
            // Register form: 0F A4 D0 10 = SHLD (E)AX, (E)DX, 16.
            let d = decode(&[0x0F, op, 0xD0, 0x10]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.mnemonic, mnemonic);
            assert_eq!(d.modrm.unwrap().reg, 2);
            assert_eq!(d.modrm.unwrap().rm, 0);
            assert_eq!(d.immediate, 0x10);
            assert_eq!(d.length, 4);

            // Memory destination with a 16-bit displacement.
            let d = decode(&[0x0F, op, 0x1E, 0x00, 0x40, 0x04]).unwrap();
            assert_eq!(d.displacement, 0x4000);
            assert_eq!(d.immediate, 4);
            assert_eq!(d.length, 6);

            // The immediate stays one byte under a 32-bit operand size.
            let d = decode_with_mode(&[0x0F, op, 0xD0, 0x10], DecodeMode::DEFAULT32).unwrap();
            assert!(d.operand_size_32);
            assert_eq!(d.immediate, 0x10);
            assert_eq!(d.length, 4);

            assert_eq!(decode(&[0x0F, op, 0xD0]), Err(DecodeError::Truncated));
        }

        for (op, mnemonic) in [(0xA5u8, "SHLD"), (0xAD, "SHRD")] {
            let d = decode(&[0x0F, op, 0xD0]).unwrap();
            assert_eq!(d.mnemonic, mnemonic);
            assert_eq!(d.immediate, 0, "{mnemonic} CL form takes no immediate");
            assert_eq!(d.length, 3);

            let d = decode(&[0x0F, op, 0x1E, 0x00, 0x40]).unwrap();
            assert_eq!(d.displacement, 0x4000);
            assert_eq!(d.length, 5);

            assert_eq!(decode(&[0x0F, op]), Err(DecodeError::Truncated));
        }
    }

    /// Intel SDM Vol. 2 "CMOVcc" (opcode map 2): a ModR/M form with no
    /// immediate, at both operand sizes, in 16- and 32-bit addressing.
    #[test]
    fn decode_cmovcc_range() {
        for cc in 0u8..16 {
            let op = 0x40 | cc;

            // Register form: 0F 4x C1 = CMOVcc r, r.
            let d = decode(&[0x0F, op, 0xC1]).unwrap();
            assert!(d.two_byte);
            assert_eq!(d.opcode, op);
            assert_eq!(d.modrm.unwrap().reg, 0);
            assert_eq!(d.modrm.unwrap().rm, 1);
            assert_eq!(d.immediate, 0);
            assert_eq!(d.length, 3, "0F {op:02X} must not consume an immediate");
            assert!(!d.operand_size_32);

            // Memory form with a 16-bit displacement.
            let d = decode(&[0x0F, op, 0x1E, 0x00, 0x40]).unwrap();
            assert_eq!(d.displacement, 0x4000);
            assert_eq!(d.length, 5);

            // `0x66` selects 32 under a 16-bit default; a `D=1` segment defaults
            // to 32 and `0x66` selects 16.
            let d = decode(&[0x66, 0x0F, op, 0xC1]).unwrap();
            assert!(d.operand_size_32);
            let d = decode_with_mode(&[0x0F, op, 0xC1], DecodeMode::DEFAULT32).unwrap();
            assert!(d.operand_size_32);
            let d = decode_with_mode(&[0x66, 0x0F, op, 0xC1], DecodeMode::DEFAULT32).unwrap();
            assert!(!d.operand_size_32);

            assert_eq!(decode(&[0x0F, op]), Err(DecodeError::Truncated));
        }
    }

    /// Intel SDM Vol. 2 "IN"/"OUT"; Appendix A opcode map 1: the accumulator
    /// port forms `E5`/`E7`/`ED`/`EF` decode at both operand sizes, and the
    /// `imm8` port number stays one byte in every case.
    #[test]
    fn decode_accumulator_port_io_forms() {
        for (op, mnemonic) in [(0xEDu8, "IN_DX"), (0xEF, "OUT_DX")] {
            let d = decode(&[op]).unwrap();
            assert!(!d.two_byte);
            assert_eq!(d.opcode, op);
            assert_eq!(d.mnemonic, mnemonic);
            assert!(d.modrm.is_none());
            assert_eq!(d.immediate, 0);
            assert_eq!(d.length, 1);
            assert!(
                !d.operand_size_32,
                "{op:02X} defaults to 16-bit in LEGACY16"
            );

            // `0x66` selects 32 under a 16-bit default and 16 under `D=1`.
            let d = decode(&[0x66, op]).unwrap();
            assert!(d.operand_size_32);
            assert_eq!(d.length, 2);
            let d = decode_with_mode(&[op], DecodeMode::DEFAULT32).unwrap();
            assert!(d.operand_size_32);
            let d = decode_with_mode(&[0x66, op], DecodeMode::DEFAULT32).unwrap();
            assert!(!d.operand_size_32);
        }

        for (op, mnemonic) in [(0xE5u8, "IN_imm8"), (0xE7, "OUT_imm8")] {
            let d = decode(&[op, 0x70]).unwrap();
            assert_eq!(d.mnemonic, mnemonic);
            assert_eq!(d.immediate, 0x70);
            assert_eq!(d.length, 2);
            assert!(!d.operand_size_32);

            // A 32-bit operand size does not widen the port immediate.
            let d = decode_with_mode(&[op, 0xCF], DecodeMode::DEFAULT32).unwrap();
            assert!(d.operand_size_32);
            assert_eq!(d.immediate, 0xCF);
            assert_eq!(d.length, 2);

            assert_eq!(decode(&[op]), Err(DecodeError::Truncated));
        }
    }

    /// Intel SDM Vol. 2 "RET" (near/far imm16): the stack-release immediate is
    /// always 16 bits, independent of the operand-size attribute.
    #[test]
    fn decode_mode_default32_keeps_ret_release_immediate_16_bit() {
        let d = decode_with_mode(&[0xC2, 0x04, 0x00], DecodeMode::DEFAULT32).unwrap();
        assert_eq!(d.immediate, 4);
        assert_eq!(d.length, 3);
        let d = decode_with_mode(&[0xCA, 0x02, 0x00], DecodeMode::DEFAULT32).unwrap();
        assert_eq!(d.immediate, 2);
        assert_eq!(d.length, 3);
    }
}
