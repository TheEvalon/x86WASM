//! Instruction metadata schema and primary-opcode subset table.
//!
//! Spec authority: Intel SDM Vol. 2. Do not invent encodings.

#![forbid(unsafe_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    None,
    Modrm,
    Imm8,
    Imm16,
    Rel8,
    Rel16,
    /// Far pointer `ptr16:16` — offset then segment (e.g. `CALL`/`JMP` far).
    Ptr16_16,
    /// Opcode encodes register in low 3 bits (e.g. `B8+rw`).
    OpcodeReg,
    ModrmImm8,
    /// ModRM plus imm16 (e.g. Group 1 `81 /r iw`).
    ModrmImm16,
    /// `OUT imm8, AL` / `IN AL, imm8`
    Imm8Port,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    W8,
    W16,
    /// Follows current operand-size attribute (16 default in real mode).
    OsZ,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstrDef {
    pub mnemonic: &'static str,
    pub opcode: u8,
    pub encoding: Encoding,
    pub width: Width,
    /// Intel SDM Vol. 2 citation hint (instruction mnemonic form).
    pub sdm: &'static str,
}

/// Executable primary-opcode subset (M1 HELLO path + early M2 real-mode INT).
pub const M1_SUBSET: &[InstrDef] = &[
    InstrDef {
        mnemonic: "PUSH_ES",
        opcode: 0x06,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "PUSH",
    },
    InstrDef {
        mnemonic: "POP_ES",
        opcode: 0x07,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "POP",
    },
    InstrDef {
        mnemonic: "PUSH_CS",
        opcode: 0x0E,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "PUSH",
    },
    InstrDef {
        mnemonic: "PUSH_SS",
        opcode: 0x16,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "PUSH",
    },
    InstrDef {
        mnemonic: "POP_SS",
        opcode: 0x17,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "POP",
    },
    InstrDef {
        mnemonic: "PUSH_DS",
        opcode: 0x1E,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "PUSH",
    },
    InstrDef {
        mnemonic: "POP_DS",
        opcode: 0x1F,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "POP",
    },
    InstrDef {
        mnemonic: "ADD",
        opcode: 0x01,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "ADD",
    },
    InstrDef {
        mnemonic: "ADD",
        opcode: 0x03,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "ADD",
    },
    InstrDef {
        mnemonic: "OR",
        opcode: 0x09,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "OR",
    },
    InstrDef {
        mnemonic: "SUB",
        opcode: 0x29,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "SUB",
    },
    InstrDef {
        mnemonic: "SUB",
        opcode: 0x2B,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "SUB",
    },
    InstrDef {
        mnemonic: "XOR",
        opcode: 0x31,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "XOR",
    },
    InstrDef {
        mnemonic: "XOR",
        opcode: 0x33,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "XOR",
    },
    InstrDef {
        mnemonic: "CMP",
        opcode: 0x39,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "CMP",
    },
    InstrDef {
        mnemonic: "CMP",
        opcode: 0x3B,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "CMP",
    },
    InstrDef {
        mnemonic: "INC",
        opcode: 0x40,
        encoding: Encoding::OpcodeReg,
        width: Width::OsZ,
        sdm: "INC",
    },
    InstrDef {
        mnemonic: "DEC",
        opcode: 0x48,
        encoding: Encoding::OpcodeReg,
        width: Width::OsZ,
        sdm: "DEC",
    },
    InstrDef {
        mnemonic: "PUSH",
        opcode: 0x50,
        encoding: Encoding::OpcodeReg,
        width: Width::OsZ,
        sdm: "PUSH",
    },
    InstrDef {
        mnemonic: "POP",
        opcode: 0x58,
        encoding: Encoding::OpcodeReg,
        width: Width::OsZ,
        sdm: "POP",
    },
    InstrDef {
        mnemonic: "JO",
        opcode: 0x70,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JNO",
        opcode: 0x71,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JB",
        opcode: 0x72,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JAE",
        opcode: 0x73,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JZ",
        opcode: 0x74,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JNZ",
        opcode: 0x75,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JBE",
        opcode: 0x76,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JA",
        opcode: 0x77,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JS",
        opcode: 0x78,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JNS",
        opcode: 0x79,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JP",
        opcode: 0x7A,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JNP",
        opcode: 0x7B,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JL",
        opcode: 0x7C,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JGE",
        opcode: 0x7D,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JLE",
        opcode: 0x7E,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "JG",
        opcode: 0x7F,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "Jcc",
    },
    InstrDef {
        mnemonic: "ADD_imm8_AL",
        opcode: 0x04,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "ADD",
    },
    InstrDef {
        mnemonic: "MOVSB",
        opcode: 0xA4,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "MOVS",
    },
    InstrDef {
        mnemonic: "STOSB",
        opcode: 0xAA,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "STOS",
    },
    InstrDef {
        mnemonic: "LODSB",
        opcode: 0xAC,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "LODS",
    },
    InstrDef {
        mnemonic: "TEST",
        opcode: 0x84,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "TEST",
    },
    InstrDef {
        mnemonic: "TEST",
        opcode: 0x85,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "TEST",
    },
    InstrDef {
        mnemonic: "XCHG",
        opcode: 0x86,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "XCHG",
    },
    InstrDef {
        mnemonic: "XCHG",
        opcode: 0x87,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "XCHG",
    },
    InstrDef {
        mnemonic: "MOV",
        opcode: 0x88,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "MOV",
        opcode: 0x89,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "MOV",
        opcode: 0x8A,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "MOV",
        opcode: 0x8B,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "MOV_Sreg",
        opcode: 0x8C,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "LEA",
        opcode: 0x8D,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "LEA",
    },
    InstrDef {
        mnemonic: "MOV_Sreg",
        opcode: 0x8E,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "NOP",
        opcode: 0x90,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "NOP",
    },
    // XCHG AX, r16 — opcode 91+rw (CX…DI). 90 remains NOP (XCHG AX,AX).
    InstrDef {
        mnemonic: "XCHG",
        opcode: 0x91,
        encoding: Encoding::OpcodeReg,
        width: Width::OsZ,
        sdm: "XCHG",
    },
    InstrDef {
        mnemonic: "CBW",
        opcode: 0x98,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "CBW/CWDE/CDQE",
    },
    InstrDef {
        mnemonic: "CWD",
        opcode: 0x99,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "CWD/CDQ/CQO",
    },
    InstrDef {
        mnemonic: "CALL_FAR",
        opcode: 0x9A,
        encoding: Encoding::Ptr16_16,
        width: Width::OsZ,
        sdm: "CALL",
    },
    InstrDef {
        mnemonic: "PUSHF",
        opcode: 0x9C,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "PUSHF",
    },
    InstrDef {
        mnemonic: "POPF",
        opcode: 0x9D,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "POPF",
    },
    InstrDef {
        mnemonic: "RETF",
        opcode: 0xCB,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "RET",
    },
    InstrDef {
        mnemonic: "MOV_imm8",
        opcode: 0xB0,
        encoding: Encoding::OpcodeReg,
        width: Width::W8,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "MOV_imm16",
        opcode: 0xB8,
        encoding: Encoding::OpcodeReg,
        width: Width::OsZ,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "RET",
        opcode: 0xC3,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "RET",
    },
    InstrDef {
        mnemonic: "INT3",
        opcode: 0xCC,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "INT3",
    },
    InstrDef {
        mnemonic: "INT",
        opcode: 0xCD,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "INT n",
    },
    InstrDef {
        mnemonic: "IRET",
        opcode: 0xCF,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "IRET",
    },
    // Group 1: ALU imm — ModRM.reg selects op (SDM Vol. 2 opcode map).
    InstrDef {
        mnemonic: "GRP1",
        opcode: 0x80,
        encoding: Encoding::ModrmImm8,
        width: Width::W8,
        sdm: "ADD/OR/ADC/SBB/AND/SUB/XOR/CMP",
    },
    InstrDef {
        mnemonic: "GRP1",
        opcode: 0x81,
        encoding: Encoding::ModrmImm16,
        width: Width::OsZ,
        sdm: "ADD/OR/ADC/SBB/AND/SUB/XOR/CMP",
    },
    InstrDef {
        mnemonic: "GRP1",
        opcode: 0x83,
        encoding: Encoding::ModrmImm8,
        width: Width::OsZ,
        sdm: "ADD/OR/ADC/SBB/AND/SUB/XOR/CMP",
    },
    // Group 2: shift/rotate — ModRM.reg selects op (SDM Vol. 2 opcode map).
    InstrDef {
        mnemonic: "GRP2",
        opcode: 0xC0,
        encoding: Encoding::ModrmImm8,
        width: Width::W8,
        sdm: "SAL/SAR/SHL/SHR/ROL/ROR/RCL/RCR",
    },
    InstrDef {
        mnemonic: "GRP2",
        opcode: 0xC1,
        encoding: Encoding::ModrmImm8,
        width: Width::OsZ,
        sdm: "SAL/SAR/SHL/SHR/ROL/ROR/RCL/RCR",
    },
    InstrDef {
        mnemonic: "GRP2",
        opcode: 0xD0,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "SAL/SAR/SHL/SHR/ROL/ROR/RCL/RCR",
    },
    InstrDef {
        mnemonic: "GRP2",
        opcode: 0xD1,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "SAL/SAR/SHL/SHR/ROL/ROR/RCL/RCR",
    },
    InstrDef {
        mnemonic: "GRP2",
        opcode: 0xD2,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "SAL/SAR/SHL/SHR/ROL/ROR/RCL/RCR",
    },
    InstrDef {
        mnemonic: "GRP2",
        opcode: 0xD3,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "SAL/SAR/SHL/SHR/ROL/ROR/RCL/RCR",
    },
    // Group 3: NOT/NEG (other /r forms out of scope this slice).
    InstrDef {
        mnemonic: "GRP3",
        opcode: 0xF6,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "NOT/NEG",
    },
    InstrDef {
        mnemonic: "GRP3",
        opcode: 0xF7,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "NOT/NEG",
    },
    InstrDef {
        mnemonic: "LOOPNE",
        opcode: 0xE0,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "LOOP",
    },
    InstrDef {
        mnemonic: "LOOPE",
        opcode: 0xE1,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "LOOP",
    },
    InstrDef {
        mnemonic: "LOOP",
        opcode: 0xE2,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "LOOP",
    },
    InstrDef {
        mnemonic: "JCXZ",
        opcode: 0xE3,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "JCXZ/JECXZ/JRCXZ",
    },
    InstrDef {
        mnemonic: "IN_imm8",
        opcode: 0xE4,
        encoding: Encoding::Imm8Port,
        width: Width::W8,
        sdm: "IN",
    },
    InstrDef {
        mnemonic: "OUT_imm8",
        opcode: 0xE6,
        encoding: Encoding::Imm8Port,
        width: Width::W8,
        sdm: "OUT",
    },
    InstrDef {
        mnemonic: "CALL",
        opcode: 0xE8,
        encoding: Encoding::Rel16,
        width: Width::OsZ,
        sdm: "CALL",
    },
    InstrDef {
        mnemonic: "JMP",
        opcode: 0xE9,
        encoding: Encoding::Rel16,
        width: Width::OsZ,
        sdm: "JMP",
    },
    InstrDef {
        mnemonic: "JMP_FAR",
        opcode: 0xEA,
        encoding: Encoding::Ptr16_16,
        width: Width::OsZ,
        sdm: "JMP",
    },
    InstrDef {
        mnemonic: "JMP",
        opcode: 0xEB,
        encoding: Encoding::Rel8,
        width: Width::W8,
        sdm: "JMP",
    },
    InstrDef {
        mnemonic: "IN_DX",
        opcode: 0xEC,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "IN",
    },
    InstrDef {
        mnemonic: "OUT_DX",
        opcode: 0xEE,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "OUT",
    },
    InstrDef {
        mnemonic: "HLT",
        opcode: 0xF4,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "HLT",
    },
    InstrDef {
        mnemonic: "CMC",
        opcode: 0xF5,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "CMC",
    },
    InstrDef {
        mnemonic: "CLC",
        opcode: 0xF8,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "CLC",
    },
    InstrDef {
        mnemonic: "STC",
        opcode: 0xF9,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "STC",
    },
    InstrDef {
        mnemonic: "CLI",
        opcode: 0xFA,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "CLI",
    },
    InstrDef {
        mnemonic: "STI",
        opcode: 0xFB,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "STI",
    },
    InstrDef {
        mnemonic: "CLD",
        opcode: 0xFC,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "CLD",
    },
    InstrDef {
        mnemonic: "STD",
        opcode: 0xFD,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "STD",
    },
];

pub fn lookup_primary(opcode: u8) -> Option<&'static InstrDef> {
    // Opcode-reg groups (indices shift when entries are prepended — match by opcode).
    if (0x40..=0x47).contains(&opcode) {
        return M1_SUBSET.iter().find(|d| d.opcode == 0x40);
    }
    if (0x48..=0x4F).contains(&opcode) {
        return M1_SUBSET.iter().find(|d| d.opcode == 0x48);
    }
    if (0x50..=0x57).contains(&opcode) {
        return M1_SUBSET.iter().find(|d| d.opcode == 0x50);
    }
    if (0x58..=0x5F).contains(&opcode) {
        return M1_SUBSET.iter().find(|d| d.opcode == 0x58);
    }
    if (0x91..=0x97).contains(&opcode) {
        return M1_SUBSET.iter().find(|d| d.opcode == 0x91);
    }
    if (0xB0..=0xB7).contains(&opcode) {
        return M1_SUBSET.iter().find(|d| d.opcode == 0xB0);
    }
    if (0xB8..=0xBF).contains(&opcode) {
        return M1_SUBSET.iter().find(|d| d.opcode == 0xB8);
    }
    M1_SUBSET.iter().find(|d| d.opcode == opcode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subset_includes_hello_opcodes() {
        for op in [
            0x8Au8, 0x84, 0x74, 0xBA, 0xEE, 0x43, 0x48, 0x4F, 0xEB, 0xF4, 0xFA, 0xE9, 0xCD, 0xCF,
            0x9C, 0x9D, 0x9A, 0xCB, 0xEA, 0x06, 0x07, 0x0E, 0x16, 0x17, 0x1E, 0x1F, 0x8C, 0x8E,
            0xF8, 0xF9, 0xFC, 0xFD, 0xCC, 0x70, 0x71, 0x73, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B,
            0x7C, 0x7D, 0x7E, 0x7F, 0xA4, 0xAA, 0xAC, 0x8D, 0x86, 0x87, 0x91, 0x97, 0x98, 0x99,
            0x80, 0x81, 0x83, 0xC0, 0xC1, 0xD0, 0xD1, 0xD2, 0xD3, 0xE0, 0xE1, 0xE2, 0xE3, 0xF6,
            0xF7,
        ] {
            assert!(lookup_primary(op).is_some(), "missing {op:#x}");
        }
    }
}
