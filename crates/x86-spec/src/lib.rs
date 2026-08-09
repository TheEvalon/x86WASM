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
    /// `ENTER iw, ib` — frame size then nesting level (Intel SDM Vol. 2 ENTER).
    Imm16Imm8,
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
    /// Absolute memory offset (moffs) following address-size attribute.
    /// Real-mode default: 16-bit offset in the immediate field.
    Moffs,
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

/// Executable primary-opcode subset (M1 HELLO path + early M2 real-mode foundation).
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
        opcode: 0x00,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "ADD",
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
        opcode: 0x02,
        encoding: Encoding::Modrm,
        width: Width::W8,
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
        mnemonic: "ADC",
        opcode: 0x10,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "ADC",
    },
    InstrDef {
        mnemonic: "ADC",
        opcode: 0x11,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "ADC",
    },
    InstrDef {
        mnemonic: "ADC",
        opcode: 0x12,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "ADC",
    },
    InstrDef {
        mnemonic: "ADC",
        opcode: 0x13,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "ADC",
    },
    InstrDef {
        mnemonic: "ADC",
        opcode: 0x14,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "ADC",
    },
    InstrDef {
        mnemonic: "ADC",
        opcode: 0x15,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "ADC",
    },
    InstrDef {
        mnemonic: "SBB",
        opcode: 0x18,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "SBB",
    },
    InstrDef {
        mnemonic: "SBB",
        opcode: 0x19,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "SBB",
    },
    InstrDef {
        mnemonic: "SBB",
        opcode: 0x1A,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "SBB",
    },
    InstrDef {
        mnemonic: "SBB",
        opcode: 0x1B,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "SBB",
    },
    InstrDef {
        mnemonic: "SBB",
        opcode: 0x1C,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "SBB",
    },
    InstrDef {
        mnemonic: "SBB",
        opcode: 0x1D,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "SBB",
    },
    InstrDef {
        mnemonic: "OR",
        opcode: 0x08,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "OR",
    },
    InstrDef {
        mnemonic: "OR",
        opcode: 0x09,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "OR",
    },
    InstrDef {
        mnemonic: "OR",
        opcode: 0x0A,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "OR",
    },
    InstrDef {
        mnemonic: "OR",
        opcode: 0x0B,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "OR",
    },
    InstrDef {
        mnemonic: "OR",
        opcode: 0x0C,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "OR",
    },
    InstrDef {
        mnemonic: "OR",
        opcode: 0x0D,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "OR",
    },
    InstrDef {
        mnemonic: "AND",
        opcode: 0x20,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "AND",
    },
    InstrDef {
        mnemonic: "AND",
        opcode: 0x21,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "AND",
    },
    InstrDef {
        mnemonic: "AND",
        opcode: 0x22,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "AND",
    },
    InstrDef {
        mnemonic: "AND",
        opcode: 0x23,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "AND",
    },
    InstrDef {
        mnemonic: "AND",
        opcode: 0x24,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "AND",
    },
    InstrDef {
        mnemonic: "AND",
        opcode: 0x25,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "AND",
    },
    InstrDef {
        mnemonic: "SUB",
        opcode: 0x28,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "SUB",
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
        opcode: 0x2A,
        encoding: Encoding::Modrm,
        width: Width::W8,
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
        mnemonic: "SUB",
        opcode: 0x2C,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "SUB",
    },
    InstrDef {
        mnemonic: "SUB",
        opcode: 0x2D,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "SUB",
    },
    InstrDef {
        mnemonic: "DAA",
        opcode: 0x27,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "DAA",
    },
    InstrDef {
        mnemonic: "DAS",
        opcode: 0x2F,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "DAS",
    },
    InstrDef {
        mnemonic: "XOR",
        opcode: 0x30,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "XOR",
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
        opcode: 0x32,
        encoding: Encoding::Modrm,
        width: Width::W8,
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
        mnemonic: "XOR",
        opcode: 0x34,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "XOR",
    },
    InstrDef {
        mnemonic: "XOR",
        opcode: 0x35,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "XOR",
    },
    InstrDef {
        mnemonic: "CMP",
        opcode: 0x38,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "CMP",
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
        opcode: 0x3A,
        encoding: Encoding::Modrm,
        width: Width::W8,
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
        mnemonic: "CMP",
        opcode: 0x3C,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "CMP",
    },
    InstrDef {
        mnemonic: "CMP",
        opcode: 0x3D,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "CMP",
    },
    InstrDef {
        mnemonic: "AAA",
        opcode: 0x37,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "AAA",
    },
    InstrDef {
        mnemonic: "AAS",
        opcode: 0x3F,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "AAS",
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
        mnemonic: "PUSH",
        opcode: 0x68,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "PUSH",
    },
    InstrDef {
        mnemonic: "IMUL",
        opcode: 0x69,
        encoding: Encoding::ModrmImm16,
        width: Width::OsZ,
        sdm: "IMUL",
    },
    InstrDef {
        mnemonic: "PUSH",
        opcode: 0x6A,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "PUSH",
    },
    InstrDef {
        mnemonic: "IMUL",
        opcode: 0x6B,
        encoding: Encoding::ModrmImm8,
        width: Width::OsZ,
        sdm: "IMUL",
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
        mnemonic: "ADD",
        opcode: 0x05,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "ADD",
    },
    InstrDef {
        mnemonic: "MOV",
        opcode: 0xA0,
        encoding: Encoding::Moffs,
        width: Width::W8,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "MOV",
        opcode: 0xA1,
        encoding: Encoding::Moffs,
        width: Width::OsZ,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "MOV",
        opcode: 0xA2,
        encoding: Encoding::Moffs,
        width: Width::W8,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "MOV",
        opcode: 0xA3,
        encoding: Encoding::Moffs,
        width: Width::OsZ,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "TEST",
        opcode: 0xA8,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "TEST",
    },
    InstrDef {
        mnemonic: "TEST",
        opcode: 0xA9,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "TEST",
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
        mnemonic: "CMPSB",
        opcode: 0xA6,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "CMPS",
    },
    InstrDef {
        mnemonic: "SCASB",
        opcode: 0xAE,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "SCAS",
    },
    InstrDef {
        mnemonic: "MOVSW",
        opcode: 0xA5,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "MOVS",
    },
    InstrDef {
        mnemonic: "STOSW",
        opcode: 0xAB,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "STOS",
    },
    InstrDef {
        mnemonic: "LODSW",
        opcode: 0xAD,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "LODS",
    },
    InstrDef {
        mnemonic: "CMPSW",
        opcode: 0xA7,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "CMPS",
    },
    InstrDef {
        mnemonic: "SCASW",
        opcode: 0xAF,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "SCAS",
    },
    InstrDef {
        mnemonic: "INSB",
        opcode: 0x6C,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "INS",
    },
    InstrDef {
        mnemonic: "INSW",
        opcode: 0x6D,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "INS",
    },
    InstrDef {
        mnemonic: "OUTSB",
        opcode: 0x6E,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "OUTS",
    },
    InstrDef {
        mnemonic: "OUTSW",
        opcode: 0x6F,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "OUTS",
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
        mnemonic: "SAHF",
        opcode: 0x9E,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "SAHF",
    },
    InstrDef {
        mnemonic: "LAHF",
        opcode: 0x9F,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "LAHF",
    },
    InstrDef {
        mnemonic: "PUSHA",
        opcode: 0x60,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "PUSHA/PUSHAD",
    },
    InstrDef {
        mnemonic: "POPA",
        opcode: 0x61,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "POPA/POPAD",
    },
    InstrDef {
        mnemonic: "BOUND",
        opcode: 0x62,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "BOUND",
    },
    InstrDef {
        mnemonic: "POP",
        opcode: 0x8F,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "POP",
    },
    InstrDef {
        mnemonic: "RET",
        opcode: 0xC2,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "RET",
    },
    InstrDef {
        mnemonic: "ENTER",
        opcode: 0xC8,
        encoding: Encoding::Imm16Imm8,
        width: Width::OsZ,
        sdm: "ENTER",
    },
    InstrDef {
        mnemonic: "LEAVE",
        opcode: 0xC9,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "LEAVE",
    },
    InstrDef {
        mnemonic: "RETF",
        opcode: 0xCA,
        encoding: Encoding::Imm16,
        width: Width::OsZ,
        sdm: "RET",
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
        mnemonic: "LES",
        opcode: 0xC4,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "LES",
    },
    InstrDef {
        mnemonic: "LDS",
        opcode: 0xC5,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "LDS",
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
        mnemonic: "INTO",
        opcode: 0xCE,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "INTO",
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
        mnemonic: "XLAT",
        opcode: 0xD7,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "XLAT/XLATB",
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
    InstrDef {
        mnemonic: "AAM",
        opcode: 0xD4,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "AAM",
    },
    InstrDef {
        mnemonic: "AAD",
        opcode: 0xD5,
        encoding: Encoding::Imm8,
        width: Width::W8,
        sdm: "AAD",
    },
    // Group 3: TEST/NOT/NEG/MUL/IMUL/DIV/IDIV (/0–/7). TEST imm via decoder special-case.
    InstrDef {
        mnemonic: "GRP3",
        opcode: 0xF6,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "TEST/NOT/NEG/MUL/IMUL/DIV/IDIV",
    },
    InstrDef {
        mnemonic: "GRP3",
        opcode: 0xF7,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "TEST/NOT/NEG/MUL/IMUL/DIV/IDIV",
    },
    InstrDef {
        mnemonic: "GRP11",
        opcode: 0xC7,
        encoding: Encoding::ModrmImm16,
        width: Width::OsZ,
        sdm: "MOV",
    },
    InstrDef {
        mnemonic: "GRP11",
        opcode: 0xC6,
        encoding: Encoding::ModrmImm8,
        width: Width::W8,
        sdm: "MOV",
    },
    // Group 4/5: ModRM.reg selects op (SDM Vol. 2 opcode map).
    InstrDef {
        mnemonic: "GRP4",
        opcode: 0xFE,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "INC/DEC",
    },
    InstrDef {
        mnemonic: "GRP5",
        opcode: 0xFF,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "INC/DEC/CALL/JMP/PUSH",
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
        sdm: "IN AL, imm8",
    },
    // `IN eAX, imm8` / `OUT imm8, eAX` — the port number stays an imm8 at
    // every operand size; only the accumulator width follows the operand-size
    // attribute. Spec: Intel SDM Vol. 2 "IN"/"OUT"; Appendix A opcode map 1.
    InstrDef {
        mnemonic: "IN_imm8",
        opcode: 0xE5,
        encoding: Encoding::Imm8Port,
        width: Width::OsZ,
        sdm: "IN eAX, imm8",
    },
    InstrDef {
        mnemonic: "OUT_imm8",
        opcode: 0xE6,
        encoding: Encoding::Imm8Port,
        width: Width::W8,
        sdm: "OUT imm8, AL",
    },
    InstrDef {
        mnemonic: "OUT_imm8",
        opcode: 0xE7,
        encoding: Encoding::Imm8Port,
        width: Width::OsZ,
        sdm: "OUT imm8, eAX",
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
        sdm: "IN AL, DX",
    },
    // `IN eAX, DX` / `OUT DX, eAX` — no immediate; the accumulator width
    // follows the operand-size attribute.
    // Spec: Intel SDM Vol. 2 "IN"/"OUT"; Appendix A opcode map 1.
    InstrDef {
        mnemonic: "IN_DX",
        opcode: 0xED,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "IN eAX, DX",
    },
    InstrDef {
        mnemonic: "OUT_DX",
        opcode: 0xEE,
        encoding: Encoding::None,
        width: Width::W8,
        sdm: "OUT DX, AL",
    },
    InstrDef {
        mnemonic: "OUT_DX",
        opcode: 0xEF,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "OUT DX, eAX",
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

/// Two-byte opcode map entries (primary escape `0F`).
/// Spec: Intel SDM Vol. 2 Chapter 2; opcode map 2.
pub const M1_0F_SUBSET: &[InstrDef] = &[
    // Group 6: ModRM.reg selects op (SDM Vol. 2 opcode map 2 — 0F 00).
    // Implemented: /1 STR, /3 LTR (32-bit available TSS). Unsupported here:
    // /0 SLDT, /2 LLDT, /4 VERR, /5 VERW, and 16-bit TSS forms.
    InstrDef {
        mnemonic: "GRP6",
        opcode: 0x00,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "STR/LTR",
    },
    // Group 7: ModRM.reg selects op (SDM Vol. 2 opcode map 2 — 0F 01).
    // Implemented: /0 SGDT, /1 SIDT, /2 LGDT, /3 LIDT, /4 SMSW, /6 LMSW,
    // /7 INVLPG (real-mode NOP; mod=11 #UD). /5 extensions unsupported.
    InstrDef {
        mnemonic: "GRP7",
        opcode: 0x01,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "SGDT/SIDT/LGDT/LIDT/SMSW/LMSW/INVLPG",
    },
    // CLTS — Spec: Intel SDM Vol. 2 "CLTS—Clear Task-Switched Flag in CR0"
    // (opcode map 2 — 0F 06). No ModR/M. Clears CR0.TS (bit 3) only.
    InstrDef {
        mnemonic: "CLTS",
        opcode: 0x06,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "CLTS",
    },
    InstrDef {
        mnemonic: "IMUL",
        opcode: 0xAF,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "IMUL",
    },
    // MOV r32, CR0 / MOV CR0, r32 — Spec: Intel SDM Vol. 2 "MOV—Move to/from
    // Control Registers". ModRM.reg selects the control register (0=CR0;
    // 1=CR1 is #UD; 2/3/4 = CR2/CR3/CR4, out of scope for this slice).
    // The mod field is architecturally ignored (always register-direct); the
    // decoder special-cases these two opcodes to avoid consuming SIB/disp.
    InstrDef {
        mnemonic: "MOV",
        opcode: 0x20,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "MOV r32,CRn",
    },
    InstrDef {
        mnemonic: "MOV",
        opcode: 0x22,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "MOV CRn,r32",
    },
    // Near `Jcc rel16/rel32` — Spec: Intel SDM Vol. 2 "Jcc—Jump if Condition
    // Is Met" (opcode map 2 — `0F 80`+cc). The displacement follows the
    // operand-size attribute, so `Encoding::Rel16` widens to rel32 under a
    // 32-bit operand size exactly as the near `E8`/`E9` forms do.
    jcc_near(0x80, "JO"),
    jcc_near(0x81, "JNO"),
    jcc_near(0x82, "JB"),
    jcc_near(0x83, "JAE"),
    jcc_near(0x84, "JE"),
    jcc_near(0x85, "JNE"),
    jcc_near(0x86, "JBE"),
    jcc_near(0x87, "JA"),
    jcc_near(0x88, "JS"),
    jcc_near(0x89, "JNS"),
    jcc_near(0x8A, "JP"),
    jcc_near(0x8B, "JNP"),
    jcc_near(0x8C, "JL"),
    jcc_near(0x8D, "JGE"),
    jcc_near(0x8E, "JLE"),
    jcc_near(0x8F, "JG"),
    // `SETcc r/m8` — Spec: Intel SDM Vol. 2 "SETcc—Set Byte on Condition"
    // (opcode map 2 — `0F 90`+cc /r). Always an 8-bit destination; the
    // operand-size attribute has no effect. ModR/M.reg is not used.
    setcc(0x90, "SETO"),
    setcc(0x91, "SETNO"),
    setcc(0x92, "SETB"),
    setcc(0x93, "SETAE"),
    setcc(0x94, "SETE"),
    setcc(0x95, "SETNE"),
    setcc(0x96, "SETBE"),
    setcc(0x97, "SETA"),
    setcc(0x98, "SETS"),
    setcc(0x99, "SETNS"),
    setcc(0x9A, "SETP"),
    setcc(0x9B, "SETNP"),
    setcc(0x9C, "SETL"),
    setcc(0x9D, "SETGE"),
    setcc(0x9E, "SETLE"),
    setcc(0x9F, "SETG"),
    // `CMOVcc r, r/m` — Spec: Intel SDM Vol. 2 "CMOVcc—Conditional Move"
    // (opcode map 2 — `0F 40`+cc /r). Same low-nibble condition encoding as
    // `Jcc` and `SETcc`.
    cmovcc(0x40, "CMOVO"),
    cmovcc(0x41, "CMOVNO"),
    cmovcc(0x42, "CMOVB"),
    cmovcc(0x43, "CMOVAE"),
    cmovcc(0x44, "CMOVE"),
    cmovcc(0x45, "CMOVNE"),
    cmovcc(0x46, "CMOVBE"),
    cmovcc(0x47, "CMOVA"),
    cmovcc(0x48, "CMOVS"),
    cmovcc(0x49, "CMOVNS"),
    cmovcc(0x4A, "CMOVP"),
    cmovcc(0x4B, "CMOVNP"),
    cmovcc(0x4C, "CMOVL"),
    cmovcc(0x4D, "CMOVGE"),
    cmovcc(0x4E, "CMOVLE"),
    cmovcc(0x4F, "CMOVG"),
    // `PUSH`/`POP FS`/`GS` — Spec: Intel SDM Vol. 2 "PUSH"/"POP" (opcode map 2
    // — `0F A0`/`0F A1`/`0F A8`/`0F A9`). No ModR/M; the stack slot width
    // follows the operand-size attribute.
    InstrDef {
        mnemonic: "PUSH_FS",
        opcode: 0xA0,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "PUSH FS",
    },
    InstrDef {
        mnemonic: "POP_FS",
        opcode: 0xA1,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "POP FS",
    },
    InstrDef {
        mnemonic: "PUSH_GS",
        opcode: 0xA8,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "PUSH GS",
    },
    InstrDef {
        mnemonic: "POP_GS",
        opcode: 0xA9,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "POP GS",
    },
    // `LSS`/`LFS`/`LGS r16/r32, m16:16/m16:32` — Spec: Intel SDM Vol. 2
    // "LDS/LES/LFS/LGS/LSS—Load Far Pointer" (opcode map 2 — `0F B2`/`B4`/`B5`).
    // Memory operand only; the register form is `#UD` at execute.
    InstrDef {
        mnemonic: "LSS",
        opcode: 0xB2,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "LSS",
    },
    InstrDef {
        mnemonic: "LFS",
        opcode: 0xB4,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "LFS",
    },
    InstrDef {
        mnemonic: "LGS",
        opcode: 0xB5,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "LGS",
    },
    // `MOVZX`/`MOVSX Gv, Eb|Ew` — Spec: Intel SDM Vol. 2 "MOVZX"/"MOVSX"
    // (opcode map 2 — `0F B6`/`B7`/`BE`/`BF`). `width` records the *source*
    // width, which the opcode fixes; the destination follows the operand-size
    // attribute.
    InstrDef {
        mnemonic: "MOVZX",
        opcode: 0xB6,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "MOVZX Gv,Eb",
    },
    InstrDef {
        mnemonic: "MOVZX",
        opcode: 0xB7,
        encoding: Encoding::Modrm,
        width: Width::W16,
        sdm: "MOVZX Gv,Ew",
    },
    InstrDef {
        mnemonic: "MOVSX",
        opcode: 0xBE,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "MOVSX Gv,Eb",
    },
    InstrDef {
        mnemonic: "MOVSX",
        opcode: 0xBF,
        encoding: Encoding::Modrm,
        width: Width::W16,
        sdm: "MOVSX Gv,Ew",
    },
    // Bit test/modify, register bit-offset forms — Spec: Intel SDM Vol. 2
    // "BT"/"BTS"/"BTR"/"BTC" (opcode map 2 — `0F A3`/`AB`/`B3`/`BB`).
    InstrDef {
        mnemonic: "BT",
        opcode: 0xA3,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "BT r/m,r",
    },
    InstrDef {
        mnemonic: "BTS",
        opcode: 0xAB,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "BTS r/m,r",
    },
    InstrDef {
        mnemonic: "BTR",
        opcode: 0xB3,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "BTR r/m,r",
    },
    InstrDef {
        mnemonic: "BTC",
        opcode: 0xBB,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "BTC r/m,r",
    },
    // Group 8: ModR/M.reg selects the bit operation and an imm8 supplies the
    // bit offset. /4 BT, /5 BTS, /6 BTR, /7 BTC; /0–/3 are reserved (#UD).
    // Spec: Intel SDM Vol. 2 opcode map 2 (`0F BA`), Group 8 table.
    InstrDef {
        mnemonic: "GRP8",
        opcode: 0xBA,
        encoding: Encoding::ModrmImm8,
        width: Width::OsZ,
        sdm: "BT/BTS/BTR/BTC r/m,imm8",
    },
    // Double-precision shifts — Spec: Intel SDM Vol. 2 "SHLD—Double Precision
    // Shift Left" (`0F A4` ib, `0F A5` CL) and "SHRD—Double Precision Shift
    // Right" (`0F AC` ib, `0F AD` CL). The destination is `r/m`, the bit source
    // is `ModR/M.reg`, and both follow the operand-size attribute.
    InstrDef {
        mnemonic: "SHLD",
        opcode: 0xA4,
        encoding: Encoding::ModrmImm8,
        width: Width::OsZ,
        sdm: "SHLD r/m,r,imm8",
    },
    InstrDef {
        mnemonic: "SHLD",
        opcode: 0xA5,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "SHLD r/m,r,CL",
    },
    InstrDef {
        mnemonic: "SHRD",
        opcode: 0xAC,
        encoding: Encoding::ModrmImm8,
        width: Width::OsZ,
        sdm: "SHRD r/m,r,imm8",
    },
    InstrDef {
        mnemonic: "SHRD",
        opcode: 0xAD,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "SHRD r/m,r,CL",
    },
    // Bit scans — Spec: Intel SDM Vol. 2 "BSF"/"BSR" (`0F BC`/`BD`).
    InstrDef {
        mnemonic: "BSF",
        opcode: 0xBC,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "BSF r,r/m",
    },
    InstrDef {
        mnemonic: "BSR",
        opcode: 0xBD,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "BSR r,r/m",
    },
    // Exchange-and-modify — Spec: Intel SDM Vol. 2 "CMPXCHG" (`0F B0`/`B1`)
    // and "XADD" (`0F C0`/`C1`).
    InstrDef {
        mnemonic: "CMPXCHG",
        opcode: 0xB0,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "CMPXCHG r/m8,r8",
    },
    InstrDef {
        mnemonic: "CMPXCHG",
        opcode: 0xB1,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "CMPXCHG r/m,r",
    },
    InstrDef {
        mnemonic: "XADD",
        opcode: 0xC0,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "XADD r/m8,r8",
    },
    InstrDef {
        mnemonic: "XADD",
        opcode: 0xC1,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "XADD r/m,r",
    },
    // Cache management and the reserved undefined opcode — Spec: Intel SDM
    // Vol. 2 "INVD" (`0F 08`), "WBINVD" (`0F 09`), "UD2" (`0F 0B`).
    InstrDef {
        mnemonic: "INVD",
        opcode: 0x08,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "INVD",
    },
    InstrDef {
        mnemonic: "WBINVD",
        opcode: 0x09,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "WBINVD",
    },
    InstrDef {
        mnemonic: "UD2",
        opcode: 0x0B,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "UD2",
    },
    // Model-specific registers and identification — Spec: Intel SDM Vol. 2
    // "WRMSR" (`0F 30`), "RDMSR" (`0F 32`), "CPUID" (`0F A2`).
    InstrDef {
        mnemonic: "WRMSR",
        opcode: 0x30,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "WRMSR",
    },
    InstrDef {
        mnemonic: "RDMSR",
        opcode: 0x32,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "RDMSR",
    },
    InstrDef {
        mnemonic: "CPUID",
        opcode: 0xA2,
        encoding: Encoding::None,
        width: Width::OsZ,
        sdm: "CPUID",
    },
    // `BSWAP r32` — Spec: Intel SDM Vol. 2 "BSWAP" (`0F C8`+rd). The register
    // is encoded in the low three opcode bits; there is no ModR/M byte.
    bswap(0xC8),
    bswap(0xC9),
    bswap(0xCA),
    bswap(0xCB),
    bswap(0xCC),
    bswap(0xCD),
    bswap(0xCE),
    bswap(0xCF),
];

/// Two-byte `BSWAP r32` entry (`0F C8`+rd).
const fn bswap(opcode: u8) -> InstrDef {
    InstrDef {
        mnemonic: "BSWAP",
        opcode,
        encoding: Encoding::OpcodeReg,
        width: Width::OsZ,
        sdm: "BSWAP r32",
    }
}

/// Two-byte near `Jcc rel16/rel32` entry (`0F 80`+cc cw/cd).
const fn jcc_near(opcode: u8, mnemonic: &'static str) -> InstrDef {
    InstrDef {
        mnemonic,
        opcode,
        encoding: Encoding::Rel16,
        width: Width::OsZ,
        sdm: "Jcc rel16/rel32",
    }
}

/// Two-byte `SETcc r/m8` entry (`0F 90`+cc /r).
const fn setcc(opcode: u8, mnemonic: &'static str) -> InstrDef {
    InstrDef {
        mnemonic,
        opcode,
        encoding: Encoding::Modrm,
        width: Width::W8,
        sdm: "SETcc r/m8",
    }
}

/// Two-byte `CMOVcc r, r/m` entry (`0F 40`+cc /r).
///
/// The destination is `ModR/M.reg` and the width follows the operand-size
/// attribute; there is no byte form. Spec: Intel SDM Vol. 2 "CMOVcc".
const fn cmovcc(opcode: u8, mnemonic: &'static str) -> InstrDef {
    InstrDef {
        mnemonic,
        opcode,
        encoding: Encoding::Modrm,
        width: Width::OsZ,
        sdm: "CMOVcc r16/r32, r/m16/r/m32",
    }
}

pub fn lookup_0f(opcode: u8) -> Option<&'static InstrDef> {
    M1_0F_SUBSET.iter().find(|d| d.opcode == opcode)
}

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
            0x9C, 0x9D, 0x9E, 0x9F, 0x9A, 0xCB, 0xEA, 0x06, 0x07, 0x0E, 0x16, 0x17, 0x1E, 0x1F,
            0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x8C, 0x8E, 0xF8, 0xF9, 0xFC, 0xFD,
            0xCC, 0x70, 0x71, 0x73, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B, 0x7C, 0x7D, 0x7E, 0x7F,
            0xA4, 0xA5, 0xA6, 0xA7, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF, 0x8D, 0x86, 0x87, 0x91,
            0x97, 0x98, 0x99, 0x80, 0x81, 0x83, 0xC0, 0xC1, 0xD0, 0xD1, 0xD2, 0xD3, 0xE0, 0xE1,
            0xE2, 0xE3, 0xF6, 0xF7, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x10, 0x11, 0x12, 0x13,
            0x14, 0x15, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25,
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2F,
            0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3F,
            0xA0, 0xA1, 0xA2, 0xA3, 0xA8, 0xA9, 0xC6, 0xC7, 0x60, 0x61, 0x62, 0x8F, 0xC2, 0xC8,
            0xC9, 0xCA, 0xC4, 0xC5, 0xCE, 0xD4, 0xD5, 0xD7,
        ] {
            assert!(lookup_primary(op).is_some(), "missing {op:#x}");
        }
        assert!(
            lookup_0f(0x00).is_some(),
            "missing 0F 00 GRP6 STR/LTR"
        );
        assert!(
            lookup_0f(0x01).is_some(),
            "missing 0F 01 GRP7 SGDT/SIDT/LGDT/LIDT"
        );
        assert!(lookup_0f(0xAF).is_some(), "missing 0F AF IMUL");
        assert!(lookup_0f(0x20).is_some(), "missing 0F 20 MOV r32,CRn");
        assert!(lookup_0f(0x22).is_some(), "missing 0F 22 MOV CRn,r32");
    }

    /// Intel SDM Vol. 2 "IN—Input from Port" / "OUT—Output to Port"; Appendix A
    /// opcode map 1: both the fixed-`AL` byte forms and the accumulator forms
    /// that follow the operand-size attribute are present. The port number of
    /// `E4`–`E7` is an imm8 at every operand size.
    #[test]
    fn subset_includes_accumulator_port_io_at_both_operand_sizes() {
        for (opcode, encoding) in [
            (0xE4u8, Encoding::Imm8Port),
            (0xE5, Encoding::Imm8Port),
            (0xE6, Encoding::Imm8Port),
            (0xE7, Encoding::Imm8Port),
            (0xEC, Encoding::None),
            (0xED, Encoding::None),
            (0xEE, Encoding::None),
            (0xEF, Encoding::None),
        ] {
            let def = lookup_primary(opcode)
                .unwrap_or_else(|| panic!("missing primary opcode {opcode:#04X}"));
            assert_eq!(def.encoding, encoding, "{opcode:#04X} encoding");
            // Even opcodes are the fixed byte forms; odd ones follow OsZ.
            let expected_width = if opcode.is_multiple_of(2) {
                Width::W8
            } else {
                Width::OsZ
            };
            assert_eq!(def.width, expected_width, "{opcode:#04X} width");
        }
    }

    /// Intel SDM Vol. 2 "Jcc" / "SETcc" (opcode map 2): the whole `0F 80`–`0F 8F`
    /// and `0F 90`–`0F 9F` condition ranges are present with the documented
    /// mnemonic order, rel16/rel32 vs byte-destination encodings.
    #[test]
    fn two_byte_condition_ranges_are_complete() {
        const JCC: [&str; 16] = [
            "JO", "JNO", "JB", "JAE", "JE", "JNE", "JBE", "JA", "JS", "JNS", "JP", "JNP", "JL",
            "JGE", "JLE", "JG",
        ];
        const SETCC: [&str; 16] = [
            "SETO", "SETNO", "SETB", "SETAE", "SETE", "SETNE", "SETBE", "SETA", "SETS", "SETNS",
            "SETP", "SETNP", "SETL", "SETGE", "SETLE", "SETG",
        ];
        for cc in 0u8..16 {
            let jcc =
                lookup_0f(0x80 | cc).unwrap_or_else(|| panic!("missing 0F {:02X}", 0x80 | cc));
            assert_eq!(jcc.mnemonic, JCC[cc as usize]);
            assert_eq!(jcc.encoding, Encoding::Rel16);
            assert_eq!(jcc.width, Width::OsZ);

            let set =
                lookup_0f(0x90 | cc).unwrap_or_else(|| panic!("missing 0F {:02X}", 0x90 | cc));
            assert_eq!(set.mnemonic, SETCC[cc as usize]);
            assert_eq!(set.encoding, Encoding::Modrm);
            assert_eq!(set.width, Width::W8);
        }
    }

    /// Intel SDM Vol. 2 "SHLD"/"SHRD" (opcode map 2): the `imm8` count forms
    /// carry a one-byte immediate, the `CL` count forms carry none, and both
    /// follow the operand-size attribute.
    #[test]
    fn two_byte_double_precision_shifts_are_present() {
        for (opcode, mnemonic, encoding) in [
            (0xA4u8, "SHLD", Encoding::ModrmImm8),
            (0xA5, "SHLD", Encoding::Modrm),
            (0xAC, "SHRD", Encoding::ModrmImm8),
            (0xAD, "SHRD", Encoding::Modrm),
        ] {
            let def =
                lookup_0f(opcode).unwrap_or_else(|| panic!("missing 0F {opcode:02X} {mnemonic}"));
            assert_eq!(def.mnemonic, mnemonic);
            assert_eq!(def.encoding, encoding);
            assert_eq!(def.width, Width::OsZ);
        }
    }

    /// Intel SDM Vol. 2 "CMOVcc—Conditional Move" (opcode map 2): the whole
    /// `0F 40`–`0F 4F` range is present, keyed on the same low-nibble condition
    /// encoding as `Jcc`/`SETcc`, with no byte form (`Width::OsZ` only).
    #[test]
    fn two_byte_cmovcc_range_is_complete() {
        const CMOVCC: [&str; 16] = [
            "CMOVO", "CMOVNO", "CMOVB", "CMOVAE", "CMOVE", "CMOVNE", "CMOVBE", "CMOVA", "CMOVS",
            "CMOVNS", "CMOVP", "CMOVNP", "CMOVL", "CMOVGE", "CMOVLE", "CMOVG",
        ];
        for cc in 0u8..16 {
            let def =
                lookup_0f(0x40 | cc).unwrap_or_else(|| panic!("missing 0F {:02X}", 0x40 | cc));
            assert_eq!(def.mnemonic, CMOVCC[cc as usize]);
            assert_eq!(def.encoding, Encoding::Modrm);
            assert_eq!(def.width, Width::OsZ);
        }
    }
}
