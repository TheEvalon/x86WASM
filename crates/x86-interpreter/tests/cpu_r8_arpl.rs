//! Round-8 slice 3 substitute: `ARPL` (`63`) — VM86 deferred.
//!
//! Spec: Intel SDM Vol. 2 "ARPL"; Vol. 3 §5.4.3 (RPL). Real-address mode → `#UD`.

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

fn pm_fixture(code: &[u8]) -> (CpuState, RamBus) {
    let mut bus = RamBus::new(0x10000);
    let gdt = 0x2000usize;
    bus.write_bytes(gdt, &[0u8; 8]);
    bus.write_bytes(gdt + 8, &encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
    bus.write_bytes(gdt + 16, &encode_seg_desc(0, 0xF_FFFF, 0x93, 0xC0));
    bus.write_bytes(0x1000, code);

    let mut cpu = CpuState::reset();
    cpu.cr0 |= 1;
    cpu.cs = x86_core::SegmentReg {
        selector: 0x0008,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC09A,
    };
    cpu.ss = x86_core::SegmentReg {
        selector: 0x0010,
        base: 0,
        limit: 0xFFFF_FFFF,
        flags: 0xC093,
    };
    cpu.ds = cpu.ss.clone();
    cpu.gdtr.base = gdt as u64;
    cpu.gdtr.limit = 23;
    cpu.rip = 0x1000;
    cpu.set_gpr_u32(CpuState::RSP, 0x8000);
    cpu.rflags = 0x2;
    (cpu, bus)
}

/// When DEST.RPL < SRC.RPL, ARPL raises DEST.RPL and sets ZF.
#[test]
fn arpl_raises_dest_rpl_and_sets_zf() {
    // 63 C8   ARPL AX, CX   (mod=11 reg=CX rm=AX)
    let (mut cpu, mut bus) = pm_fixture(&[0x63, 0xC8, 0xF4]);
    cpu.set_gpr_u16(CpuState::RAX, 0x0010); // RPL=0
    cpu.set_gpr_u16(CpuState::RCX, 0x0003); // RPL=3

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x0013);
    assert_ne!(cpu.rflags & (1 << 6), 0, "ZF set");
}

/// When DEST.RPL ≥ SRC.RPL, ARPL leaves DEST unchanged and clears ZF.
#[test]
fn arpl_leaves_dest_when_rpl_sufficient() {
    let (mut cpu, mut bus) = pm_fixture(&[0x63, 0xC8, 0xF4]);
    cpu.set_gpr_u16(CpuState::RAX, 0x0012); // RPL=2
    cpu.set_gpr_u16(CpuState::RCX, 0x0001); // RPL=1

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x0012);
    assert_eq!(cpu.rflags & (1 << 6), 0, "ZF clear");
}

/// Memory destination form adjusts the in-memory selector.
#[test]
fn arpl_memory_destination() {
    // 67 63 00   ARPL [EAX], AX  under asize32 — wait: under CS.D=1 default
    // asize32: 63 00 = ARPL [EAX], AX with mod=00 rm=000.
    // Actually encoding: modrm reg field is source. ARPL r/m, r → reg=source.
    // 63 08 = mod=00 reg=001 (CX) rm=000 (EAX) → ARPL [EAX], CX
    let (mut cpu, mut bus) = pm_fixture(&[0x63, 0x08, 0xF4]);
    cpu.set_gpr_u32(CpuState::RAX, 0x4000);
    bus.write_bytes(0x4000, &0x00A0u16.to_le_bytes()); // RPL=0
    cpu.set_gpr_u16(CpuState::RCX, 0x0002);

    step(&mut cpu, &mut bus).unwrap();

    assert_eq!(bus.mem[0x4000], 0xA2);
    assert_eq!(bus.mem[0x4001], 0x00);
    assert_ne!(cpu.rflags & (1 << 6), 0);
}

/// Real-address mode still raises `#UD` for ARPL.
#[test]
fn arpl_real_mode_is_ud() {
    let mut bus = RamBus::new(0x10000);
    bus.write_bytes(0x1000, &[0x63, 0xC8, 0xF4]);
    // IVT vector 6 → 0000:0900
    bus.write_bytes(6 * 4, &[0x00, 0x09, 0x00, 0x00]);
    bus.mem[0x0900] = 0xF4;
    let mut cpu = CpuState::reset();
    cpu.cs = x86_core::SegmentReg::real_mode_code(0);
    cpu.ss = x86_core::SegmentReg::real_mode(0);
    cpu.ds = cpu.ss.clone();
    cpu.rip = 0x1000;
    cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.rip, 0x0900);
}
