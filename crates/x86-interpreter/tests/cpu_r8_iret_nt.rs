//! Round-8 slice 2: `IRET`/`IRETD` with `NT=1` nested-task return.
//!
//! Spec: Intel SDM Vol. 2 "IRET/IRETD" (Operation — nested task); Vol. 3
//! §§7.2–7.3 Table 7-1 (IRET clears outgoing busy and NT, restores the linked
//! busy TSS).

use x86_core::CpuState;
use x86_interpreter::{step, Bus, ExecError};

struct RamBus {
    mem: Vec<u8>,
}

impl RamBus {
    fn new(size: usize) -> Self {
        Self {
            mem: vec![0u8; size],
        }
    }

    fn write_bytes(&mut self, addr: usize, bytes: &[u8]) {
        self.mem[addr..addr + bytes.len()].copy_from_slice(bytes);
    }

    fn poke_u32(&mut self, addr: usize, value: u32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn poke_u16(&mut self, addr: usize, value: u16) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn peek_u8(&self, addr: usize) -> u8 {
        self.mem[addr]
    }
}

impl Bus for RamBus {
    fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
        let index = addr as usize;
        if index >= self.mem.len() {
            return Err(ExecError::MemoryFault(addr));
        }
        Ok(self.mem[index])
    }

    fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
        let index = addr as usize;
        if index >= self.mem.len() {
            return Err(ExecError::MemoryFault(addr));
        }
        self.mem[index] = val;
        Ok(())
    }

    fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
        Ok(0xFF)
    }

    fn port_out_u8(&mut self, _port: u16, _val: u8) -> Result<(), ExecError> {
        Ok(())
    }
}

const GDT: usize = 0x2000;
const IDT: usize = 0x2800;
const OLD_TSS: usize = 0x3000;
const NEW_TSS: usize = 0x3100;
const CODE: usize = 0x1000;
const TASK_CODE: usize = 0x1800;
const AFTER_CALL: usize = 0x1007; // immediately after the 7-byte far CALL

const SEL_KCODE: u16 = 0x0008;
const SEL_KDATA: u16 = 0x0010;
const SEL_OLD_TSS: u16 = 0x0018;
const SEL_NEW_TSS: u16 = 0x0020;

fn encode_seg_desc(base: u32, limit20: u32, access: u8, gran_flags: u8) -> [u8; 8] {
    let lim = limit20 & 0xF_FFFF;
    [
        lim as u8,
        (lim >> 8) as u8,
        base as u8,
        (base >> 8) as u8,
        (base >> 16) as u8,
        access,
        ((lim >> 16) as u8 & 0x0F) | (gran_flags & 0xF0),
        (base >> 24) as u8,
    ]
}

fn install_gdt(bus: &mut RamBus) {
    bus.write_bytes(GDT, &[0u8; 8]);
    bus.write_bytes(GDT + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(GDT + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(GDT + 24, &encode_seg_desc(OLD_TSS as u32, 0x67, 0x8B, 0));
    bus.write_bytes(GDT + 32, &encode_seg_desc(NEW_TSS as u32, 0x67, 0x89, 0));
}

fn fill_new_tss(bus: &mut RamBus) {
    bus.poke_u32(NEW_TSS + 28, 0);
    bus.poke_u32(NEW_TSS + 32, TASK_CODE as u32);
    bus.poke_u32(NEW_TSS + 36, 0x202);
    bus.poke_u32(NEW_TSS + 40, 0x1111_1111);
    bus.poke_u32(NEW_TSS + 44, 0);
    bus.poke_u32(NEW_TSS + 48, 0);
    bus.poke_u32(NEW_TSS + 52, 0);
    bus.poke_u32(NEW_TSS + 56, 0x0000_A000);
    bus.poke_u32(NEW_TSS + 60, 0);
    bus.poke_u32(NEW_TSS + 64, 0);
    bus.poke_u32(NEW_TSS + 68, 0);
    bus.poke_u16(NEW_TSS + 72, SEL_KDATA);
    bus.poke_u16(NEW_TSS + 76, SEL_KCODE);
    bus.poke_u16(NEW_TSS + 80, SEL_KDATA);
    bus.poke_u16(NEW_TSS + 84, SEL_KDATA);
    bus.poke_u16(NEW_TSS + 88, 0);
    bus.poke_u16(NEW_TSS + 92, 0);
    bus.poke_u16(NEW_TSS + 96, 0);
}

fn fixture(code: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    install_gdt(&mut bus);
    fill_new_tss(&mut bus);
    bus.write_bytes(CODE, code);
    // Nested task body: IRETD (CF under CS.D=1).
    bus.mem[TASK_CODE] = 0xCF;
    // Resume point after the CALL in the parent task.
    bus.mem[AFTER_CALL] = 0xF4;

    let gp_gate = {
        let off = (0x1900u32).to_le_bytes();
        let sel = SEL_KCODE.to_le_bytes();
        [off[0], off[1], sel[0], sel[1], 0, 0x8E, off[2], off[3]]
    };
    bus.write_bytes(IDT + 13 * 8, &gp_gate);
    bus.mem[0x1900] = 0xF4;

    let mut cpu = CpuState::reset();
    cpu.cr0 |= 1;
    cpu.cs = x86_core::SegmentReg {
        selector: SEL_KCODE,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC09A,
    };
    cpu.ss = x86_core::SegmentReg {
        selector: SEL_KDATA,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC093,
    };
    cpu.ds = cpu.ss.clone();
    cpu.es = cpu.ss.clone();
    cpu.tr = x86_core::SegmentReg {
        selector: SEL_OLD_TSS,
        base: OLD_TSS as u64,
        limit: 0x67,
        flags: 0x008B,
    };
    cpu.gdtr.base = GDT as u64;
    cpu.gdtr.limit = 39;
    cpu.idtr.base = IDT as u64;
    cpu.idtr.limit = 0x7FF;
    cpu.rip = CODE as u64;
    cpu.set_gpr_u32(CpuState::RSP, 0x0000_8000);
    cpu.set_gpr_u32(CpuState::RAX, 0xAAAA_AAAA);
    cpu.rflags = 0x202;
    (cpu, bus)
}

/// `CALL` → nested TSS then `IRETD` with `NT=1` restores the parent task.
#[test]
fn iretd_with_nt_returns_to_parent_tss() {
    // 9A ptr16:32 CALL new TSS; next byte is the resume point.
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_NEW_TSS.to_le_bytes());
    bytes.push(0xF4);
    let (mut cpu, mut bus) = fixture(&bytes);

    step(&mut cpu, &mut bus).unwrap(); // CALL nests
    assert_eq!(cpu.tr.selector, SEL_NEW_TSS);
    assert_ne!(cpu.rflags & (1 << 14), 0);
    assert_eq!(cpu.rip, TASK_CODE as u64);

    step(&mut cpu, &mut bus).unwrap(); // IRETD with NT=1

    assert_eq!(cpu.tr.selector, SEL_OLD_TSS);
    assert_eq!(cpu.rip, AFTER_CALL as u64);
    assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xAAAA_AAAA);
    assert_eq!(cpu.rflags & (1 << 14), 0, "NT cleared on IRET return");
    assert_eq!(
        bus.peek_u8(GDT + 32 + 5) & 0x0F,
        0x9,
        "nested TSS becomes available"
    );
    assert_eq!(
        bus.peek_u8(GDT + 24 + 5) & 0x0F,
        0xB,
        "parent TSS stays busy"
    );
}

/// `IRET` with `NT=1` and a non-busy back-link raises `#GP(selector)`.
#[test]
fn iretd_nt_with_available_backlink_raises_gp() {
    let (mut cpu, mut bus) = fixture(&[0xCF]);
    // Pretend we are already in the nested task with NT set and a bad link.
    cpu.tr = x86_core::SegmentReg {
        selector: SEL_NEW_TSS,
        base: NEW_TSS as u64,
        limit: 0x67,
        flags: 0x008B,
    };
    bus.mem[GDT + 32 + 5] = 0x8B; // new busy
    bus.mem[GDT + 24 + 5] = 0x89; // old available — illegal for IRET return
    bus.poke_u16(NEW_TSS, SEL_OLD_TSS);
    cpu.rflags |= 1 << 14;
    cpu.rip = TASK_CODE as u64;
    bus.mem[TASK_CODE] = 0xCF;

    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rip, 0x1900, "vectored through #GP");
}

/// `IRET` with `NT=1` does not consume a stack frame.
#[test]
fn iretd_nt_does_not_pop_stack_frame() {
    let mut bytes = vec![0x9A];
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&SEL_NEW_TSS.to_le_bytes());
    bytes.push(0xF4);
    let (mut cpu, mut bus) = fixture(&bytes);

    step(&mut cpu, &mut bus).unwrap();
    let esp_in_nested = cpu.gpr_u32(CpuState::RSP);
    // Poison what would look like an IRET frame; NT path must ignore it.
    bus.poke_u32(esp_in_nested as usize, 0xBAD0_0000);
    bus.poke_u32(esp_in_nested as usize + 4, 0xBAD0_0008);
    bus.poke_u32(esp_in_nested as usize + 8, 0xBAD0_0202);

    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rip, AFTER_CALL as u64);
    assert_eq!(cpu.tr.selector, SEL_OLD_TSS);
}
