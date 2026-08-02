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
    /// Opcode encodes register in low 3 bits (e.g. `B8+rw`).
    OpcodeReg,
    ModrmImm8,
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
        mnemonic: "JB",
        opcode: 0x72,
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
        mnemonic: "ADD_imm8_AL",
        opcode: 0x04,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "ADD",
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
        mnemonic: "NOP",
        opcode: 0x90,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "NOP",
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
];

pub fn lookup_primary(opcode: u8) -> Option<&'static InstrDef> {
    // Opcode-reg groups
    if (0x40..=0x47).contains(&opcode) {
        return Some(&M1_SUBSET[9]); // INC
    }
    if (0x50..=0x57).contains(&opcode) {
        return M1_SUBSET.iter().find(|d| d.opcode == 0x50);
    }
    if (0x58..=0x5F).contains(&opcode) {
        return M1_SUBSET.iter().find(|d| d.opcode == 0x58);
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
            0x8Au8, 0x84, 0x74, 0xBA, 0xEE, 0x43, 0xEB, 0xF4, 0xFA, 0xE9, 0xCD, 0xCF, 0x9C, 0x9D,
        ] {
            assert!(lookup_primary(op).is_some(), "missing {op:#x}");
        }
    }
}
