//! Reference interpreter for the lab opcode subset (M1 + early M2).
//!
//! Semantics follow Intel SDM Vol. 2 / Vol. 3 for the implemented forms only.

#![forbid(unsafe_code)]

use thiserror::Error;
use x86_core::CpuState;
use x86_decode::{decode_with_mode, DecodeError, DecodedInsn};
use x86_mmu::paging::{
    Access, AccessKind, AccessMode, Mmu, PageTableMemory, PagingContext, TranslateError,
    UnsupportedPaging,
};
use x86_mmu::{checked_linear_addr, linear_addr};

/// Memory + port callbacks supplied by `machine-pc`.
///
/// Addresses are **linear**, not physical. When `CR0.PG = 1` the interpreter
/// wraps the machine's bus in [`PagedBus`], which translates each access
/// through the 32-bit paging engine before forwarding a physical address here;
/// with `CR0.PG = 0` a linear address *is* the physical address and the wrapper
/// forwards it unchanged (SDM Vol. 3 §4.1.1).
pub trait Bus {
    fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError>;
    fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError>;
    fn read_u16(&mut self, addr: u64) -> Result<u16, ExecError> {
        let lo = self.read_u8(addr)?;
        let hi = self.read_u8(addr.wrapping_add(1))?;
        Ok(u16::from_le_bytes([lo, hi]))
    }
    fn write_u16(&mut self, addr: u64, val: u16) -> Result<(), ExecError> {
        let bytes = val.to_le_bytes();
        self.write_u8(addr, bytes[0])?;
        self.write_u8(addr.wrapping_add(1), bytes[1])
    }
    fn read_u32(&mut self, addr: u64) -> Result<u32, ExecError> {
        let lo = self.read_u16(addr)?;
        let hi = self.read_u16(addr.wrapping_add(2))?;
        Ok(u32::from(lo) | (u32::from(hi) << 16))
    }
    fn write_u32(&mut self, addr: u64, val: u32) -> Result<(), ExecError> {
        self.write_u16(addr, val as u16)?;
        self.write_u16(addr.wrapping_add(2), (val >> 16) as u16)
    }
    fn port_in_u8(&mut self, port: u16) -> Result<u8, ExecError>;
    fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError>;
    /// Default: two consecutive byte ports (port, port+1). Machine buses may override.
    fn port_in_u16(&mut self, port: u16) -> Result<u16, ExecError> {
        let lo = self.port_in_u8(port)?;
        let hi = self.port_in_u8(port.wrapping_add(1))?;
        Ok(u16::from_le_bytes([lo, hi]))
    }
    fn port_out_u16(&mut self, port: u16, val: u16) -> Result<(), ExecError> {
        let bytes = val.to_le_bytes();
        self.port_out_u8(port, bytes[0])?;
        self.port_out_u8(port.wrapping_add(1), bytes[1])
    }
    /// Default: two consecutive word ports (port, port+2). Machine buses may override.
    fn port_in_u32(&mut self, port: u16) -> Result<u32, ExecError> {
        let lo = self.port_in_u16(port)?;
        let hi = self.port_in_u16(port.wrapping_add(2))?;
        Ok(u32::from(lo) | (u32::from(hi) << 16))
    }
    fn port_out_u32(&mut self, port: u16, val: u32) -> Result<(), ExecError> {
        self.port_out_u16(port, val as u16)?;
        self.port_out_u16(port.wrapping_add(2), (val >> 16) as u16)
    }

    /// Drain a device-model IRQ latch into the CPU (PIC stub).
    ///
    /// Default: none. Test buses may return a vector after N memory ops so
    /// REP can observe an interrupt between iterations. Full 8259 is later.
    fn poll_external_irq(&mut self) -> Option<u8> {
        None
    }

    /// The guest executed `MOV to CR0`, `MOV to CR3`, `MOV to CR4`, `LMSW` or
    /// `CLTS`; `reg` is the control-register number that was written and the
    /// three values are the state after the write.
    ///
    /// [`PagedBus`] uses this to refresh its [`PagingContext`] and to apply the
    /// SDM §4.10.4.1 TLB invalidation for the specific register written — which
    /// is not the same as noticing a changed value, because `MOV to CR3`
    /// invalidates even when it stores the value `CR3` already held.
    ///
    /// Default: nothing, for a bus that models no translation.
    fn on_mov_to_control_register(&mut self, reg: u8, cr0: u64, cr3: u64, cr4: u64) {
        let _ = (reg, cr0, cr3, cr4);
    }

    /// The guest executed `INVLPG` for this linear address (SDM §4.10.4.1).
    ///
    /// Default: nothing. A bus that caches no translation has nothing to drop.
    fn invalidate_page(&mut self, linear: u64) {
        let _ = linear;
    }

    /// Read one instruction byte.
    ///
    /// Separate from [`Bus::read_u8`] only so the paging path can pass
    /// `AccessKind::InstructionFetch` (SDM §4.6.1, §4.7). Default: an ordinary
    /// read, which is what a bus with no translation does.
    fn fetch_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
        self.read_u8(addr)
    }

    /// Check that a `size`-byte store at `addr` could happen, without storing.
    ///
    /// For instructions whose store is preceded by an effect that cannot be
    /// replayed — `INS` reads its port before it writes memory — the store has
    /// to be known possible first, because an instruction-boundary rollback
    /// cannot un-read a port. Default: nothing can fail, so nothing to check.
    fn probe_write(&mut self, addr: u64, size: u64) -> Result<(), ExecError> {
        let _ = (addr, size);
        Ok(())
    }

    /// A `REP` iteration completed, so a `#PF` in a later iteration must
    /// restart from here rather than from the start of the instruction.
    ///
    /// Spec: Intel SDM Vol. 2 "REP/REPE/REPZ/REPNE/REPNZ" — after a suspending
    /// exception "the source and destination registers point to the next
    /// string elements to be operated on, the EIP register points to the
    /// string instruction, and the ECX register has the value it held
    /// following the last successful iteration".
    ///
    /// Default: nothing, for a bus on which no access can fault mid-instruction.
    fn commit_string_iteration(&mut self, cpu: &CpuState) {
        let _ = cpu;
    }

    /// Read one byte of a GDT, LDT, IDT or TSS entry.
    ///
    /// §4.6.1 makes accesses the processor performs to those tables
    /// supervisor-mode accesses *regardless of CPL*, so they cannot derive
    /// their access mode from the current privilege level. Default: an
    /// ordinary read.
    fn read_system_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
        self.read_u8(addr)
    }

    /// Write one byte of a GDT, LDT, IDT or TSS entry (supervisor access).
    ///
    /// Used when the processor itself updates a system descriptor — for
    /// example marking a TSS busy on `LTR` (SDM Vol. 3 §§4.6.1, 7.2.2).
    /// Default: an ordinary write.
    fn write_system_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
        self.write_u8(addr, val)
    }
}

/// The interpreter's memory path with 32-bit paging in it.
///
/// Every access the interpreter makes goes through one of these, whether or not
/// paging is enabled, so that the translated and untranslated paths cannot
/// drift apart. With `CR0.PG = 0` it forwards linear addresses to the machine
/// bus unchanged, which is exactly what the interpreter did before paging
/// existed; with `CR0.PG = 1` it translates through [`Mmu`] first.
///
/// Spec: Intel SDM Vol. 3 §4.1.1 (a linear address is a physical address when
/// `CR0.PG = 0`), §4.3 (32-bit paging), §4.6.1 (access rights).
pub struct PagedBus<'a> {
    inner: &'a mut dyn Bus,
    mmu: &'a mut Mmu,
    ctx: PagingContext,
    /// CPL sampled at the instruction boundary (SDM §4.6.1). The delivery paths
    /// this interpreter implements are same-CPL, so it cannot change mid-
    /// instruction; a future privilege-changing gate must resample it.
    cpl: u8,
    /// Architectural state a `#PF` in this instruction restarts from.
    ///
    /// `#PF` is a fault, so the instruction re-executes and must therefore
    /// have committed nothing (SDM Vol. 3 §6.5). Rather than auditing every
    /// opcode for the order in which it writes registers, flags and memory,
    /// the interpreter checkpoints the architectural state at the instruction
    /// boundary and rolls back to it when a translation fails. Two properties
    /// make that exact rather than approximate:
    ///
    /// * `RFLAGS` in the checkpoint is always the instruction-boundary value,
    ///   even after [`Bus::commit_string_iteration`] advances the rest. That
    ///   is the SDM's own rule for a faulting `REPE`/`REPNE` `CMPS`/`SCAS`:
    ///   "the EFLAGS value is restored to the state prior to the execution of
    ///   the instruction".
    /// * It is armed only while `CR0.PG = 1`. Nothing can page-fault with
    ///   paging off, so the pre-paging execution path is untouched and pays
    ///   nothing for this.
    ///
    /// What a checkpoint cannot undo is a memory write. The instructions that
    /// write more than one location before they can fault write only to the
    /// stack below the restored pointer, where the retry rewrites the same
    /// bytes. A *single* operand that straddles a page boundary is the case
    /// that genuinely needs both halves translated before either is written.
    restart_point: Option<CpuState>,
}

impl<'a> PagedBus<'a> {
    /// Wrap `inner` with the paging state `cpu` currently selects.
    ///
    /// Construction polls [`Mmu::sync_control_registers`], which applies any
    /// invalidation implied by a control-register change that did not come
    /// through `MOV to CRn` — a reset, or a path a later slice adds. The
    /// explicit `MOV to CRn` hooks stay the precise interface (SDM §4.10.4.1).
    pub fn new(inner: &'a mut dyn Bus, mmu: &'a mut Mmu, cpu: &CpuState) -> Self {
        let ctx = PagingContext::new(cpu.cr0, cpu.cr3, cpu.cr4);
        mmu.sync_control_registers(&ctx);
        Self {
            inner,
            mmu,
            ctx,
            // Virtual-8086 mode forces CPL 3 regardless of CS[1:0]
            // (SDM Vol. 3 §5.5, §20.1.1).
            cpl: architectural_cpl(cpu),
            restart_point: None,
        }
    }

    /// Checkpoint the architectural state this instruction would restart from.
    /// A no-op with paging off, where nothing can page-fault.
    fn arm_restart_point(&mut self, cpu: &CpuState) {
        self.restart_point = self.ctx.paging_enabled().then(|| cpu.clone());
    }

    /// The checkpoint to roll back to, consumed by the `#PF` path.
    fn take_restart_point(&mut self) -> Option<CpuState> {
        self.restart_point.take()
    }

    /// The control-register state this path is translating with.
    pub fn paging_context(&self) -> &PagingContext {
        &self.ctx
    }

    /// CPL of the accesses this path makes (SDM §4.6.1).
    pub fn cpl(&self) -> u8 {
        self.cpl
    }

    /// Is a linear address a physical address right now? (`CR0.PG = 0`.)
    fn identity_mapped(&self) -> bool {
        !self.ctx.paging_enabled()
    }

    /// Translate one linear address for one access.
    ///
    /// Returns the physical address, or the error the interpreter must raise:
    /// [`ExecError::PageFault`] for an architectural `#PF`,
    /// [`ExecError::PageTableFault`] when the walk itself could not reach
    /// physical memory, or [`ExecError::UnsupportedPaging`] for a mode this
    /// engine does not model.
    ///
    /// Spec: SDM Vol. 3 §4.3 (the walk), §4.6.1 (access rights and the
    /// supervisor/user split), §4.7 (`CR2` and the error code).
    fn translate(&mut self, linear: u64, access: Access) -> Result<u64, ExecError> {
        if self.identity_mapped() {
            return Ok(linear);
        }
        let ctx = self.ctx;
        let mut mem = PageTableWalkBus {
            inner: &mut *self.inner,
            error: None,
        };
        let result = self.mmu.translate(&ctx, &mut mem, linear as u32, access);
        if let Some(phys) = mem.error {
            return Err(ExecError::PageTableFault(phys));
        }
        match result {
            Ok(translation) => Ok(translation.phys_addr),
            Err(TranslateError::Fault(fault)) => Err(ExecError::PageFault {
                linear: fault.cr2(),
                error_code: fault.error_code(),
            }),
            Err(TranslateError::Unsupported(kind)) => Err(ExecError::UnsupportedPaging(kind)),
        }
    }

    /// Check that `access` at `linear` would succeed, with no side effect at
    /// all — no accessed or dirty flag, no cached translation.
    ///
    /// Spec: SDM Vol. 3 §4.8 (the flags a real access would write), §4.10.2.3
    /// (why a probe may not cache).
    fn probe(&mut self, linear: u64, access: Access) -> Result<(), ExecError> {
        if self.identity_mapped() {
            return Ok(());
        }
        let ctx = self.ctx;
        let mut mem = PageTableWalkBus {
            inner: &mut *self.inner,
            error: None,
        };
        let result = self.mmu.probe(&ctx, &mut mem, linear as u32, access);
        if let Some(phys) = mem.error {
            return Err(ExecError::PageTableFault(phys));
        }
        match result {
            Ok(()) => Ok(()),
            Err(TranslateError::Fault(fault)) => Err(ExecError::PageFault {
                linear: fault.cr2(),
                error_code: fault.error_code(),
            }),
            Err(TranslateError::Unsupported(kind)) => Err(ExecError::UnsupportedPaging(kind)),
        }
    }

    /// Translate one architectural access of `size` bytes starting at
    /// `linear`, which may straddle a 4-KiB page boundary.
    ///
    /// The engine translates one address, so splitting the access, translating
    /// both halves, and discovering a second-half fault **before** the first
    /// half is written is caller work. That ordering is the whole point: the
    /// two halves have unrelated translations, and a `#PF` on the second one
    /// must leave a partially written operand behind no more than it leaves a
    /// partially updated register.
    ///
    /// A split therefore probes both halves before translating either, so a
    /// faulting access also writes no accessed or dirty flag. An access inside
    /// a single page skips the probe: the engine already performs every fault
    /// check before it touches a paging structure.
    ///
    /// Splitting at 4 KiB is correct for a 4-MiB page too — the two halves
    /// simply translate to adjacent physical addresses.
    ///
    /// Model choice: when both halves fault, the lower address is reported.
    /// §4.7 does not pin the order down, and ascending is the order the access
    /// itself would take.
    fn translate_span(
        &mut self,
        linear: u64,
        size: usize,
        access: Access,
    ) -> Result<Span, ExecError> {
        if self.identity_mapped() {
            return Ok(Span::whole(linear, size));
        }
        let page_offset = (linear & PAGE_OFFSET_MASK) as usize;
        let first_len = PAGE_SIZE - page_offset;
        if first_len >= size {
            return Ok(Span::whole(self.translate(linear, access)?, size));
        }

        let second_linear = linear.wrapping_add(first_len as u64) & LINEAR_ADDRESS_MASK;
        self.probe(linear, access)?;
        self.probe(second_linear, access)?;
        Ok(Span {
            first: self.translate(linear, access)?,
            first_len,
            second: self.translate(second_linear, access)?,
        })
    }

    /// Physical address of byte `index` of a span.
    fn byte_of(span: &Span, index: usize) -> u64 {
        if index < span.first_len {
            span.first + index as u64
        } else {
            span.second + (index - span.first_len) as u64
        }
    }

    /// The access a data reference of `kind` makes at the current CPL.
    fn data_access(&self, kind: AccessKind) -> Access {
        Access::from_cpl(kind, self.cpl)
    }

    /// An access the processor makes on software's behalf to the GDT, LDT, IDT
    /// or TSS: supervisor mode whatever the CPL is (SDM §4.6.1).
    fn system_access(kind: AccessKind) -> Access {
        Access::new(kind, AccessMode::Supervisor)
    }
}

/// 4-KiB page geometry of the linear address space (SDM Vol. 3 §4.3).
const PAGE_SIZE: usize = 0x1000;
const PAGE_OFFSET_MASK: u64 = 0xFFF;
/// Outside 64-bit mode the linear address space is 4 GiB (SDM Vol. 3 §3.3.1),
/// so an access that runs off the top wraps rather than carrying into bit 32.
const LINEAR_ADDRESS_MASK: u64 = 0xFFFF_FFFF;

/// One architectural access resolved into at most two physical runs.
struct Span {
    first: u64,
    /// Bytes taken from `first`; the rest come from `second`.
    first_len: usize,
    second: u64,
}

impl Span {
    fn whole(phys: u64, size: usize) -> Self {
        Self {
            first: phys,
            first_len: size,
            second: phys,
        }
    }

    /// Does the whole access live in one page?
    fn contiguous(&self, size: usize) -> bool {
        self.first_len >= size
    }
}

/// Guest physical memory as the page walker sees it: the machine bus, so a
/// paging-structure access goes through exactly the same address decode,
/// shadowing and A20 masking as any other physical access.
///
/// A bus failure cannot be reported through [`PageTableMemory`], so the first
/// failing address is latched and [`PagedBus::translate`] turns it into
/// [`ExecError::PageTableFault`] rather than letting a zero entry masquerade as
/// a not-present page.
struct PageTableWalkBus<'b> {
    inner: &'b mut dyn Bus,
    error: Option<u64>,
}

impl PageTableMemory for PageTableWalkBus<'_> {
    fn read_entry_u32(&mut self, phys_addr: u64) -> u32 {
        match self.inner.read_u32(phys_addr) {
            Ok(value) => value,
            Err(_) => {
                self.error.get_or_insert(phys_addr);
                0
            }
        }
    }

    fn write_entry_u32(&mut self, phys_addr: u64, value: u32) {
        if self.inner.write_u32(phys_addr, value).is_err() {
            self.error.get_or_insert(phys_addr);
        }
    }
}

impl Bus for PagedBus<'_> {
    fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
        let access = self.data_access(AccessKind::Read);
        let phys = self.translate(addr, access)?;
        self.inner.read_u8(phys)
    }

    fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
        let access = self.data_access(AccessKind::Write);
        let phys = self.translate(addr, access)?;
        self.inner.write_u8(phys, val)
    }

    // A multi-byte access can straddle a page boundary, where the two halves
    // have unrelated translations. `translate_span` resolves the whole access
    // before any byte of it moves; an access that stays inside one page keeps
    // its original width on the machine bus, which matters for MMIO.
    fn read_u16(&mut self, addr: u64) -> Result<u16, ExecError> {
        if self.identity_mapped() {
            return self.inner.read_u16(addr);
        }
        let access = self.data_access(AccessKind::Read);
        let span = self.translate_span(addr, 2, access)?;
        if span.contiguous(2) {
            return self.inner.read_u16(span.first);
        }
        let lo = self.inner.read_u8(Self::byte_of(&span, 0))?;
        let hi = self.inner.read_u8(Self::byte_of(&span, 1))?;
        Ok(u16::from_le_bytes([lo, hi]))
    }

    fn write_u16(&mut self, addr: u64, val: u16) -> Result<(), ExecError> {
        if self.identity_mapped() {
            return self.inner.write_u16(addr, val);
        }
        let access = self.data_access(AccessKind::Write);
        let span = self.translate_span(addr, 2, access)?;
        if span.contiguous(2) {
            return self.inner.write_u16(span.first, val);
        }
        let bytes = val.to_le_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            self.inner.write_u8(Self::byte_of(&span, index), *byte)?;
        }
        Ok(())
    }

    fn read_u32(&mut self, addr: u64) -> Result<u32, ExecError> {
        if self.identity_mapped() {
            return self.inner.read_u32(addr);
        }
        let access = self.data_access(AccessKind::Read);
        let span = self.translate_span(addr, 4, access)?;
        if span.contiguous(4) {
            return self.inner.read_u32(span.first);
        }
        let mut bytes = [0u8; 4];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = self.inner.read_u8(Self::byte_of(&span, index))?;
        }
        Ok(u32::from_le_bytes(bytes))
    }

    fn write_u32(&mut self, addr: u64, val: u32) -> Result<(), ExecError> {
        if self.identity_mapped() {
            return self.inner.write_u32(addr, val);
        }
        let access = self.data_access(AccessKind::Write);
        let span = self.translate_span(addr, 4, access)?;
        if span.contiguous(4) {
            return self.inner.write_u32(span.first, val);
        }
        let bytes = val.to_le_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            self.inner.write_u8(Self::byte_of(&span, index), *byte)?;
        }
        Ok(())
    }

    fn probe_write(&mut self, addr: u64, size: u64) -> Result<(), ExecError> {
        if self.identity_mapped() || size == 0 {
            return Ok(());
        }
        let access = self.data_access(AccessKind::Write);
        let last = addr.wrapping_add(size - 1);
        self.probe(addr, access)?;
        if (addr & !PAGE_OFFSET_MASK) != (last & !PAGE_OFFSET_MASK) {
            self.probe(last & LINEAR_ADDRESS_MASK, access)?;
        }
        Ok(())
    }

    fn fetch_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
        let access = self.data_access(AccessKind::InstructionFetch);
        let phys = self.translate(addr, access)?;
        self.inner.read_u8(phys)
    }

    fn read_system_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
        let access = Self::system_access(AccessKind::Read);
        let phys = self.translate(addr, access)?;
        self.inner.read_u8(phys)
    }

    fn write_system_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
        let access = Self::system_access(AccessKind::Write);
        let phys = self.translate(addr, access)?;
        self.inner.write_u8(phys, val)
    }

    fn commit_string_iteration(&mut self, cpu: &CpuState) {
        if let Some(point) = &mut self.restart_point {
            // Keep the instruction-boundary RFLAGS: a faulting REPE/REPNE
            // CMPS/SCAS restores flags to their pre-instruction value even
            // though its index and count progress survives.
            let boundary_flags = point.rflags;
            *point = cpu.clone();
            point.rflags = boundary_flags;
        }
    }

    fn port_in_u8(&mut self, port: u16) -> Result<u8, ExecError> {
        self.inner.port_in_u8(port)
    }

    fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError> {
        self.inner.port_out_u8(port, val)
    }

    fn port_in_u16(&mut self, port: u16) -> Result<u16, ExecError> {
        self.inner.port_in_u16(port)
    }

    fn port_out_u16(&mut self, port: u16, val: u16) -> Result<(), ExecError> {
        self.inner.port_out_u16(port, val)
    }

    fn port_in_u32(&mut self, port: u16) -> Result<u32, ExecError> {
        self.inner.port_in_u32(port)
    }

    fn port_out_u32(&mut self, port: u16, val: u32) -> Result<(), ExecError> {
        self.inner.port_out_u32(port, val)
    }

    fn poll_external_irq(&mut self) -> Option<u8> {
        self.inner.poll_external_irq()
    }

    fn on_mov_to_control_register(&mut self, reg: u8, cr0: u64, cr3: u64, cr4: u64) {
        let previous = self.ctx;
        self.ctx = PagingContext::with_profile(cr0, cr3, cr4, previous.profile);
        match reg {
            0 => self.mmu.on_mov_to_cr0(previous.cr0, cr0),
            3 => self.mmu.on_mov_to_cr3(cr3),
            4 => self.mmu.on_mov_to_cr4(previous.cr4, cr4),
            _ => {}
        }
        // Keep the polled shadow level with the explicit hook so the next
        // `sync_control_registers` does not repeat the same invalidation.
        self.mmu.sync_control_registers(&self.ctx);
    }

    fn invalidate_page(&mut self, linear: u64) {
        self.mmu.invlpg(linear as u32);
    }
}

/// Deterministic reasons the bounded protected-mode exception-delivery path can
/// reject a transfer instead of synthesizing nested #DF/triple-fault behavior.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum ProtectedModeDeliveryError {
    #[error("IDT limit excludes the vector gate")]
    IdtLimit,
    #[error("IDT gate read failed at {0:#x}")]
    IdtRead(u64),
    #[error("descriptor is not a 16-bit or 32-bit interrupt/trap gate (access {0:#04x})")]
    GateType(u8),
    #[error("gate is not present")]
    GateNotPresent,
    #[error("null target selector")]
    NullTargetSelector,
    #[error("LDT target selector is unsupported")]
    LdtTargetSelector,
    #[error("GDT limit excludes the target descriptor")]
    GdtLimit,
    #[error("GDT target descriptor read failed at {0:#x}")]
    GdtRead(u64),
    #[error("target code segment is not present")]
    TargetNotPresent,
    #[error("target descriptor is not a usable executable code segment")]
    TargetCode,
    #[error("16-bit gate target descriptor is not a 16-bit code segment")]
    TargetNot16Bit,
    #[error("target descriptor is a 64-bit (L=1) code segment")]
    TargetLongMode,
    #[error("target offset exceeds the code-segment limit")]
    TargetOffsetLimit,
    #[error("current code segment is not a supported 16-bit privilege context")]
    CurrentPrivilege,
    #[error("current stack is not 16-bit")]
    StackWidth,
    #[error("TR does not reference a usable 32-bit TSS")]
    TssInvalid,
    #[error("TSS limit excludes the inner-level stack pointers")]
    TssLimit,
    #[error("TSS read failed at {0:#x}")]
    TssRead(u64),
    #[error("inner-level stack selector is invalid")]
    InnerStackSelector,
    #[error("stack limit excludes the protected-mode frame")]
    StackLimit,
    #[error("stack read failed at {0:#x}")]
    StackRead(u64),
    #[error("stack write failed at {0:#x}")]
    StackWrite(u64),
    #[error("stack rollback failed at {0:#x}")]
    StackRollback(u64),
}

/// Host-visible execution errors.
///
/// Architectural faults delivered through the real-mode IVT (`CR0.PE=0`) or
/// the bounded 16-bit protected-mode IDT path (`CR0.PE=1`) return `Ok(())`
/// from [`step`] after vectoring:
/// - `#DE` 0, `#BR` 5, `#UD` 6, `#SS` 12, `#GP` 13
///
/// Remaining host errors:
/// - `Decode`: truncated fetch, or sparse-table misses that are **not**
///   architectural `#UD` (valid-but-unimplemented primary opcodes — see
///   [`real_mode_primary_opcode_is_ud`])
/// - `MemoryFault`: bus errors outside architectural classification or during
///   the legacy real-mode IVT path
/// - `Unsupported`: valid-but-unimplemented forms reached after decode
/// - `ArchFault`: internal vector + optional error code consumed by [`step`]
/// - `ProtectedModeExceptionDelivery`: bounded delivery-time failure; nested
///   `#DF`/triple-fault behavior is not modeled
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExecError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("memory fault at {0:#x}")]
    MemoryFault(u64),
    #[error("unsupported encoding for opcode 0x{0:02X}")]
    Unsupported(u8),
    /// Pending architectural fault delivery; consumed by [`step`].
    #[error("architectural fault vector {vector}, error code {error_code:?}")]
    ArchFault { vector: u8, error_code: Option<u16> },
    #[error(
        "protected-mode exception delivery for vector {vector} failed \
         (nested #DF/triple fault unsupported): {reason}"
    )]
    ProtectedModeExceptionDelivery {
        vector: u8,
        reason: ProtectedModeDeliveryError,
    },
    /// A fault during `#DF` delivery (or `#DF` that cannot be entered).
    ///
    /// Spec: Intel SDM Vol. 3 §6.15 (Interrupt 8—Double Fault Exception);
    /// the processor shuts down after a triple fault.
    #[error("triple fault while delivering double fault ({reason})")]
    TripleFault { reason: ProtectedModeDeliveryError },
    /// Pending `#PF` (vector 14). Carried separately from
    /// [`ExecError::ArchFault`] because it needs a doubleword error code and
    /// because `CR2` must be loaded with the faulting linear address before
    /// delivery. Consumed by [`step`].
    /// Spec: Intel SDM Vol. 3 §4.7; Vol. 3 "Interrupt 14—Page-Fault Exception".
    #[error("page fault at linear {linear:#x}, error code {error_code:#x}")]
    PageFault { linear: u64, error_code: u32 },
    /// A page-table walk could not reach physical memory. This is a machine
    /// failure, not an architectural `#PF`: an entry read that the bus rejects
    /// must not be mistaken for a not-present entry, so it is reported rather
    /// than turned into a guest-visible exception.
    #[error("page-table walk could not reach physical memory at {0:#x}")]
    PageTableFault(u64),
    /// A paging mode the 32-bit engine does not model. Never deliverable as a
    /// guest exception; `MOV to CR4` already refuses `CR4.PAE`, so nothing in
    /// this build can reach it.
    #[error("unsupported paging mode: {0:?}")]
    UnsupportedPaging(UnsupportedPaging),
}

fn arch_fault(vector: u8) -> ExecError {
    ExecError::ArchFault {
        vector,
        error_code: None,
    }
}

fn arch_fault_with_error_code(vector: u8, error_code: u16) -> ExecError {
    ExecError::ArchFault {
        vector,
        error_code: Some(error_code),
    }
}

fn protected_mode_delivery_error(vector: u8, reason: ProtectedModeDeliveryError) -> ExecError {
    ExecError::ProtectedModeExceptionDelivery { vector, reason }
}

/// Build a selector-based exception error code.
///
/// The faulting MOV supplies neither EXT nor IDT, so selector RPL bits 1:0 are
/// cleared while TI (bit 2) and the selector index are preserved.
/// Spec: Intel SDM Vol. 3 §6.13.
fn selector_fault(vector: u8, selector: u16) -> ExecError {
    arch_fault_with_error_code(vector, selector & 0xFFFC)
}

/// SF/ZF/PF from an 8-bit BCD-adjust result (DAA/DAS/AAM/AAD).
/// Spec: Intel SDM Vol. 2 DAA/DAS/AAM/AAD — Flags Affected.
fn set_bcd_szp_flags_u8(cpu: &mut CpuState, result: u8) {
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_zf(result == 0);
    cpu.set_pf(parity_even(result));
}

/// DAA — Decimal Adjust AL after Addition.
/// Spec: Intel SDM Vol. 2 "DAA". OF undefined (left unchanged).
fn exec_daa(cpu: &mut CpuState) {
    let old_al = cpu.al();
    let old_cf = cpu.rflags & 1 != 0;
    let af = cpu.rflags & (1 << 4) != 0;
    let mut al = old_al;
    cpu.set_cf(false);
    if (al & 0x0F) > 9 || af {
        let (r, carry) = al.overflowing_add(6);
        al = r;
        cpu.set_cf(old_cf || carry);
        cpu.set_af(true);
    } else {
        cpu.set_af(false);
    }
    if old_al > 0x99 || old_cf {
        al = al.wrapping_add(0x60);
        cpu.set_cf(true);
    } else {
        cpu.set_cf(false);
    }
    cpu.set_al(al);
    set_bcd_szp_flags_u8(cpu, al);
}

/// DAS — Decimal Adjust AL after Subtraction.
/// Spec: Intel SDM Vol. 2 "DAS". OF undefined (left unchanged).
fn exec_das(cpu: &mut CpuState) {
    let old_al = cpu.al();
    let old_cf = cpu.rflags & 1 != 0;
    let af = cpu.rflags & (1 << 4) != 0;
    let mut al = old_al;
    cpu.set_cf(false);
    if (al & 0x0F) > 9 || af {
        let (r, borrow) = al.overflowing_sub(6);
        al = r;
        cpu.set_cf(old_cf || borrow);
        cpu.set_af(true);
    } else {
        cpu.set_af(false);
    }
    if old_al > 0x99 || old_cf {
        al = al.wrapping_sub(0x60);
        cpu.set_cf(true);
    } else {
        cpu.set_cf(false);
    }
    cpu.set_al(al);
    set_bcd_szp_flags_u8(cpu, al);
}

/// AAA — ASCII Adjust After Addition.
/// Spec: Intel SDM Vol. 2 "AAA". OF/SF/ZF/PF undefined (left unchanged).
fn exec_aaa(cpu: &mut CpuState) {
    let al = cpu.al();
    let af = cpu.rflags & (1 << 4) != 0;
    if (al & 0x0F) > 9 || af {
        let ax = cpu.ax().wrapping_add(0x106);
        cpu.set_ax(ax);
        cpu.set_af(true);
        cpu.set_cf(true);
    } else {
        cpu.set_af(false);
        cpu.set_cf(false);
    }
    cpu.set_al(cpu.al() & 0x0F);
}

/// AAS — ASCII Adjust AL After Subtraction.
/// Spec: Intel SDM Vol. 2 "AAS". OF/SF/ZF/PF undefined (left unchanged).
fn exec_aas(cpu: &mut CpuState) {
    let al = cpu.al();
    let af = cpu.rflags & (1 << 4) != 0;
    if (al & 0x0F) > 9 || af {
        let ax = cpu.ax().wrapping_sub(0x106);
        cpu.set_ax(ax);
        cpu.set_af(true);
        cpu.set_cf(true);
    } else {
        cpu.set_af(false);
        cpu.set_cf(false);
    }
    cpu.set_al(cpu.al() & 0x0F);
}

fn parity_even(v: u8) -> bool {
    v.count_ones().is_multiple_of(2)
}

/// Two/three-operand IMUL (and Group 3 word IMUL fit check): CF=OF=1 iff signed
/// product does not fit in i16. SF/ZF/AF/PF undefined (left unchanged).
/// Spec: Intel SDM Vol. 2 "IMUL".
fn set_imul_flags_i16(cpu: &mut CpuState, prod: i32) {
    let fits = prod == i32::from(prod as i16);
    cpu.set_cf(!fits);
    cpu.set_of(!fits);
}

/// Two-operand IMUL opsize-32: CF=OF=1 iff signed product does not fit in i32.
/// SF/ZF/AF/PF undefined (left unchanged). Spec: Intel SDM Vol. 2 "IMUL".
fn set_imul_flags_i32(cpu: &mut CpuState, prod: i64) {
    let fits = prod == i64::from(prod as i32);
    cpu.set_cf(!fits);
    cpu.set_of(!fits);
}

fn set_logic_flags_u8(cpu: &mut CpuState, result: u8) {
    cpu.set_cf(false);
    cpu.set_of(false);
    cpu.set_af(false);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
}

fn set_logic_flags_u16(cpu: &mut CpuState, result: u16) {
    cpu.set_cf(false);
    cpu.set_of(false);
    cpu.set_af(false);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
}

fn set_logic_flags_u32(cpu: &mut CpuState, result: u32) {
    cpu.set_cf(false);
    cpu.set_of(false);
    cpu.set_af(false);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
}

fn set_add_flags_u8(cpu: &mut CpuState, a: u8, b: u8, result: u8) {
    cpu.set_cf((u16::from(a) + u16::from(b)) > 0xFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = (!(a ^ b) & (a ^ result) & 0x80) != 0;
    cpu.set_of(of);
}

fn set_add_flags_u16(cpu: &mut CpuState, a: u16, b: u16, result: u16) {
    cpu.set_cf((a as u32) + (b as u32) > 0xFFFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = (!(a ^ b) & (a ^ result) & 0x8000) != 0;
    cpu.set_of(of);
}

fn set_add_flags_u32(cpu: &mut CpuState, a: u32, b: u32, result: u32) {
    cpu.set_cf((a as u64) + (b as u64) > 0xFFFF_FFFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = (!(a ^ b) & (a ^ result) & 0x8000_0000) != 0;
    cpu.set_of(of);
}

fn set_sub_flags_u16(cpu: &mut CpuState, a: u16, b: u16, result: u16) {
    cpu.set_cf(a < b);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = ((a ^ b) & (a ^ result) & 0x8000) != 0;
    cpu.set_of(of);
}

fn set_sub_flags_u32(cpu: &mut CpuState, a: u32, b: u32, result: u32) {
    cpu.set_cf(a < b);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = ((a ^ b) & (a ^ result) & 0x8000_0000) != 0;
    cpu.set_of(of);
}

fn set_sub_flags_u8(cpu: &mut CpuState, a: u8, b: u8, result: u8) {
    cpu.set_cf(a < b);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
    cpu.set_af(((a ^ b ^ result) & 0x10) != 0);
    let of = ((a ^ b) & (a ^ result) & 0x80) != 0;
    cpu.set_of(of);
}

fn set_adc_flags_u8(cpu: &mut CpuState, a: u8, b: u8, cf_in: bool, result: u8) {
    let cf = u8::from(cf_in);
    let sum = u16::from(a) + u16::from(b) + u16::from(cf);
    cpu.set_cf(sum > 0xFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
    cpu.set_af((u16::from(a & 0xF) + u16::from(b & 0xF) + u16::from(cf)) > 0xF);
    let of = (!(a ^ b) & (a ^ result) & 0x80) != 0;
    cpu.set_of(of);
}

fn set_adc_flags_u16(cpu: &mut CpuState, a: u16, b: u16, cf_in: bool, result: u16) {
    let cf = u16::from(cf_in);
    let sum = u32::from(a) + u32::from(b) + u32::from(cf);
    cpu.set_cf(sum > 0xFFFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a & 0xF) + (b & 0xF) + cf) > 0xF);
    let of = (!(a ^ b) & (a ^ result) & 0x8000) != 0;
    cpu.set_of(of);
}

fn set_adc_flags_u32(cpu: &mut CpuState, a: u32, b: u32, cf_in: bool, result: u32) {
    let cf = u32::from(cf_in);
    let sum = u64::from(a) + u64::from(b) + u64::from(cf);
    cpu.set_cf(sum > 0xFFFF_FFFF);
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af(((a & 0xF) + (b & 0xF) + cf) > 0xF);
    let of = (!(a ^ b) & (a ^ result) & 0x8000_0000) != 0;
    cpu.set_of(of);
}

fn set_sbb_flags_u8(cpu: &mut CpuState, a: u8, b: u8, cf_in: bool, result: u8) {
    let cf = u8::from(cf_in);
    cpu.set_cf(u16::from(a) < u16::from(b) + u16::from(cf));
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
    cpu.set_af((a & 0xF) < ((b & 0xF) + cf));
    let of = ((a ^ b) & (a ^ result) & 0x80) != 0;
    cpu.set_of(of);
}

fn set_sbb_flags_u16(cpu: &mut CpuState, a: u16, b: u16, cf_in: bool, result: u16) {
    let cf = u16::from(cf_in);
    cpu.set_cf(u32::from(a) < u32::from(b) + u32::from(cf));
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af((a & 0xF) < ((b & 0xF) + cf));
    let of = ((a ^ b) & (a ^ result) & 0x8000) != 0;
    cpu.set_of(of);
}

fn set_sbb_flags_u32(cpu: &mut CpuState, a: u32, b: u32, cf_in: bool, result: u32) {
    let cf = u32::from(cf_in);
    cpu.set_cf(u64::from(a) < u64::from(b) + u64::from(cf));
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
    cpu.set_af((a & 0xF) < ((b & 0xF) + cf));
    let of = ((a ^ b) & (a ^ result) & 0x8000_0000) != 0;
    cpu.set_of(of);
}

/// Group 1 ALU on 8-bit operands. Spec: Intel SDM Vol. 2 opcode map (80 /r).
/// Returns `Some(result)` to write back, or `None` for CMP.
fn grp1_u8(cpu: &mut CpuState, op: u8, a: u8, b: u8) -> Result<Option<u8>, ExecError> {
    let cf_in = cpu.rflags & 1 != 0;
    match op {
        0 => {
            let r = a.wrapping_add(b);
            set_add_flags_u8(cpu, a, b, r);
            Ok(Some(r))
        }
        1 => {
            let r = a | b;
            set_logic_flags_u8(cpu, r);
            Ok(Some(r))
        }
        2 => {
            let r = a.wrapping_add(b).wrapping_add(u8::from(cf_in));
            set_adc_flags_u8(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        3 => {
            let r = a.wrapping_sub(b).wrapping_sub(u8::from(cf_in));
            set_sbb_flags_u8(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        4 => {
            let r = a & b;
            set_logic_flags_u8(cpu, r);
            Ok(Some(r))
        }
        5 => {
            let r = a.wrapping_sub(b);
            set_sub_flags_u8(cpu, a, b, r);
            Ok(Some(r))
        }
        6 => {
            let r = a ^ b;
            set_logic_flags_u8(cpu, r);
            Ok(Some(r))
        }
        7 => {
            set_sub_flags_u8(cpu, a, b, a.wrapping_sub(b));
            Ok(None)
        }
        _ => Err(ExecError::Unsupported(0x80)),
    }
}

/// Group 1 ALU on 16-bit operands. Spec: Intel SDM Vol. 2 opcode map (81/83 /r).
fn grp1_u16(cpu: &mut CpuState, op: u8, a: u16, b: u16) -> Result<Option<u16>, ExecError> {
    let cf_in = cpu.rflags & 1 != 0;
    match op {
        0 => {
            let r = a.wrapping_add(b);
            set_add_flags_u16(cpu, a, b, r);
            Ok(Some(r))
        }
        1 => {
            let r = a | b;
            set_logic_flags_u16(cpu, r);
            Ok(Some(r))
        }
        2 => {
            let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
            set_adc_flags_u16(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        3 => {
            let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
            set_sbb_flags_u16(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        4 => {
            let r = a & b;
            set_logic_flags_u16(cpu, r);
            Ok(Some(r))
        }
        5 => {
            let r = a.wrapping_sub(b);
            set_sub_flags_u16(cpu, a, b, r);
            Ok(Some(r))
        }
        6 => {
            let r = a ^ b;
            set_logic_flags_u16(cpu, r);
            Ok(Some(r))
        }
        7 => {
            set_sub_flags_u16(cpu, a, b, a.wrapping_sub(b));
            Ok(None)
        }
        _ => Err(ExecError::Unsupported(0x81)),
    }
}

/// Group 1 ALU on 32-bit operands (opsize override in 16-bit default modes).
/// Spec: Intel SDM Vol. 2 opcode map (81/83 /r); Vol. 2 Ch. 2 (66H).
fn grp1_u32(cpu: &mut CpuState, op: u8, a: u32, b: u32) -> Result<Option<u32>, ExecError> {
    let cf_in = cpu.rflags & 1 != 0;
    match op {
        0 => {
            let r = a.wrapping_add(b);
            set_add_flags_u32(cpu, a, b, r);
            Ok(Some(r))
        }
        1 => {
            let r = a | b;
            set_logic_flags_u32(cpu, r);
            Ok(Some(r))
        }
        2 => {
            let r = a.wrapping_add(b).wrapping_add(u32::from(cf_in));
            set_adc_flags_u32(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        3 => {
            let r = a.wrapping_sub(b).wrapping_sub(u32::from(cf_in));
            set_sbb_flags_u32(cpu, a, b, cf_in, r);
            Ok(Some(r))
        }
        4 => {
            let r = a & b;
            set_logic_flags_u32(cpu, r);
            Ok(Some(r))
        }
        5 => {
            let r = a.wrapping_sub(b);
            set_sub_flags_u32(cpu, a, b, r);
            Ok(Some(r))
        }
        6 => {
            let r = a ^ b;
            set_logic_flags_u32(cpu, r);
            Ok(Some(r))
        }
        7 => {
            set_sub_flags_u32(cpu, a, b, a.wrapping_sub(b));
            Ok(None)
        }
        _ => Err(ExecError::Unsupported(0x81)),
    }
}

/// Effective operand-size attribute resolved at decode time.
///
/// Real-address mode and `CS.D=0` default to 16 with `0x66` selecting 32;
/// `CS.D=1` defaults to 32 with `0x66` selecting 16.
/// Spec: Intel SDM Vol. 2 Chapter 2; Vol. 1 §3.6 (Table 3-4); Vol. 3 §3.4.5.
fn opsz32(insn: &DecodedInsn) -> bool {
    insn.operand_size_32
}

/// Effective address-size attribute resolved at decode time (`0x67` inverts
/// the code-segment default).
/// Spec: Intel SDM Vol. 1 §3.6 (Table 3-4); Vol. 2 Chapter 2; Vol. 3 §3.4.5.
fn asize32(insn: &DecodedInsn) -> bool {
    insn.address_size_32
}

/// Current architectural instruction pointer within the CS.D execution window.
///
/// `CS.D=0` keeps the legacy 16-bit `IP` window (`EIP[31:16]` preserved but
/// unused); `CS.D=1` executes with the full 32-bit `EIP`.
/// Spec: Intel SDM Vol. 1 §3.5; Vol. 3 §3.4.5 (D flag).
fn current_ip(cpu: &CpuState) -> u32 {
    if cpu.cs.default_big() {
        cpu.rip as u32
    } else {
        u32::from(cpu.ip16())
    }
}

/// Commit an instruction pointer within the CS.D execution window.
///
/// `CS.D=0` writes only `IP` (preserving `EIP[31:16]`, matching the legacy
/// real-address/16-bit protected path); `CS.D=1` writes the full `EIP`.
/// Spec: Intel SDM Vol. 1 §3.5; Vol. 3 §3.4.5.
fn set_current_ip(cpu: &mut CpuState, value: u32) {
    if cpu.cs.default_big() {
        cpu.rip = u64::from(value);
    } else {
        cpu.set_ip16(value as u16);
    }
}

/// Sequential next-instruction pointer after a `length`-byte instruction.
///
/// Wraps within the CS.D execution window (16-bit under `D=0`).
/// Spec: Intel SDM Vol. 1 §3.5; Vol. 3 §3.4.5.
fn next_ip_after(cpu: &CpuState, length: usize) -> u32 {
    let ip = current_ip(cpu);
    if cpu.cs.default_big() {
        ip.wrapping_add(length as u32)
    } else {
        u32::from((ip as u16).wrapping_add(length as u16))
    }
}

/// Near-branch target from a base EIP and a signed displacement.
///
/// "If the operand-size attribute is 16, the upper two bytes of the EIP
/// register are cleared" — Intel SDM Vol. 2 "JMP"/"CALL"/"Jcc"/"LOOP"
/// (Operation). The CS-limit check happens on the next instruction fetch.
fn near_branch_target(base_ip: u32, displacement: i32, operand_size_32: bool) -> u32 {
    let target = base_ip.wrapping_add(displacement as u32);
    if operand_size_32 {
        target
    } else {
        target & 0xFFFF
    }
}

/// Near absolute (indirect or popped) branch target for the operand size.
///
/// Same 16-bit truncation rule as [`near_branch_target`].
fn near_absolute_target(value: u32, operand_size_32: bool) -> u32 {
    if operand_size_32 {
        value
    } else {
        value & 0xFFFF
    }
}

/// Effective address from ModRM using the instruction address-size attribute.
/// Returns `(linear, is_register, uses_ss)` — `uses_ss` selects `#SS` vs `#GP`
/// when a bus `MemoryFault` is classified or a segment-limit fault is raised
/// (SDM Vol. 3 §5.3, §6.15).
/// Real-mode segmentation remains `selector << 4` (base + offset).
fn ea(
    cpu: &CpuState,
    insn: &DecodedInsn,
    access_size: u64,
) -> Result<(u64, bool, bool), ExecError> {
    if asize32(insn) {
        ea_32(cpu, insn, access_size)
    } else {
        ea_16(cpu, insn, access_size)
    }
}

/// Segment, `uses_ss`, and the unchecked effective offset of a 16-bit ModR/M
/// memory operand (`mod != 11`).
fn ea_parts_16<'a>(
    cpu: &'a CpuState,
    insn: &DecodedInsn,
) -> Result<(&'a x86_core::SegmentReg, bool, u64), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    let off = u64::from(calc_ea16(cpu, m.mod_, m.rm, insn.displacement)?);
    let (seg, uses_ss) = match insn.prefixes.segment_override {
        Some(0x2E) => (&cpu.cs, false),
        Some(0x36) => (&cpu.ss, true),
        Some(0x26) => (&cpu.es, false),
        Some(0x64) => (&cpu.fs, false),
        Some(0x65) => (&cpu.gs, false),
        Some(0x3E) | None => {
            // Default DS, except BP-based uses SS.
            if m.rm == 2 || m.rm == 3 || (m.rm == 6 && m.mod_ != 0) {
                (&cpu.ss, true)
            } else {
                (&cpu.ds, false)
            }
        }
        _ => (&cpu.ds, false),
    };
    Ok((seg, uses_ss, off))
}

/// 16-bit effective address from ModRM (real-mode / 16-bit address size).
fn ea_16(
    cpu: &CpuState,
    insn: &DecodedInsn,
    access_size: u64,
) -> Result<(u64, bool, bool), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Ok((0, true, false));
    }
    let (seg, uses_ss, off) = ea_parts_16(cpu, insn)?;
    let addr = seg_linear_checked(seg, off, access_size, uses_ss)?;
    Ok((addr, false, uses_ss))
}

/// Linear address for a data/stack access with cached segment-limit enforcement.
/// Spec: Intel SDM Vol. 3 §5.3; Vol. 2 MOV real-address `#GP`/`#SS`.
fn seg_linear_checked(
    seg: &x86_core::SegmentReg,
    offset: u64,
    size: u64,
    uses_ss: bool,
) -> Result<u64, ExecError> {
    checked_linear_addr(seg, offset, size)
        .map_err(|_| arch_fault_with_error_code(if uses_ss { 12 } else { 13 }, 0))
}

/// Absolute moffs offset from the effective address-size attribute.
///
/// `moffs16` vs `moffs32` follows the *attribute*, not the presence of a
/// `0x67` prefix: under `CS.D=1` the offset is 32 bits with no prefix and 16
/// bits with one, which is the inverse of the `D=0` case the decoder already
/// resolves into `insn.address_size_32`.
/// Spec: Intel SDM Vol. 2 MOV (moffs8/moffs16/moffs32); Vol. 1 §3.6 Table 3-4.
fn moffs_offset(insn: &DecodedInsn) -> u64 {
    if asize32(insn) {
        u64::from(insn.immediate as u32)
    } else {
        u64::from(insn.immediate as u16)
    }
}

/// 32-bit effective address from ModRM/SIB (real-mode with 0x67).
/// Spec: Intel SDM Vol. 1 §3.6; Vol. 2 Chapter 2 (32-bit addressing forms).
fn ea_32(
    cpu: &CpuState,
    insn: &DecodedInsn,
    access_size: u64,
) -> Result<(u64, bool, bool), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Ok((0, true, false));
    }
    let (seg, uses_ss, off) = ea_parts_32(cpu, insn)?;
    let addr = seg_linear_checked(seg, off, access_size, uses_ss)?;
    Ok((addr, false, uses_ss))
}

/// Segment, `uses_ss`, and the unchecked effective offset of a 32-bit
/// ModR/M + SIB memory operand (`mod != 11`).
fn ea_parts_32<'a>(
    cpu: &'a CpuState,
    insn: &DecodedInsn,
) -> Result<(&'a x86_core::SegmentReg, bool, u64), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    let off = u64::from(calc_ea32(cpu, insn)?);
    let (seg, uses_ss) = match insn.prefixes.segment_override {
        Some(0x2E) => (&cpu.cs, false),
        Some(0x36) => (&cpu.ss, true),
        Some(0x26) => (&cpu.es, false),
        Some(0x64) => (&cpu.fs, false),
        Some(0x65) => (&cpu.gs, false),
        Some(0x3E) | None => {
            // Default DS; SS when base is EBP/ESP (incl. SIB base).
            let uses_ss = if m.rm == 4 {
                let sib = insn.sib.ok_or(ExecError::Unsupported(insn.opcode))?;
                let base = sib & 7;
                base == 4 || (base == 5 && m.mod_ != 0)
            } else {
                m.rm == 5 && m.mod_ != 0
            };
            if uses_ss {
                (&cpu.ss, true)
            } else {
                (&cpu.ds, false)
            }
        }
        _ => (&cpu.ds, false),
    };
    Ok((seg, uses_ss, off))
}

/// Segment, `uses_ss`, and the unchecked effective offset of a ModR/M memory
/// operand, using the instruction address-size attribute.
///
/// Callers that need a plain operand should use [`ea`]; this variant exists for
/// the `BT`/`BTS`/`BTR`/`BTC` bit-string forms, which displace the effective
/// address by a bit offset before the segment-limit check.
fn ea_parts<'a>(
    cpu: &'a CpuState,
    insn: &DecodedInsn,
) -> Result<(&'a x86_core::SegmentReg, bool, u64), ExecError> {
    if asize32(insn) {
        ea_parts_32(cpu, insn)
    } else {
        ea_parts_16(cpu, insn)
    }
}

/// Map a bus `MemoryFault` to `#SS` (vector 12) or `#GP` (vector 13).
/// Spec: Intel SDM Vol. 3 §6.15 (#SS / #GP).
fn classify_mem_fault(err: ExecError, uses_ss: bool) -> ExecError {
    match err {
        ExecError::MemoryFault(_) => arch_fault_with_error_code(if uses_ss { 12 } else { 13 }, 0),
        e => e,
    }
}

fn calc_ea16(cpu: &CpuState, mod_: u8, rm: u8, displacement: i32) -> Result<u16, ExecError> {
    let disp = displacement as i16 as u16;
    let base = match rm {
        0 => cpu
            .gpr_u16(CpuState::RBX)
            .wrapping_add(cpu.gpr_u16(CpuState::RSI)),
        1 => cpu
            .gpr_u16(CpuState::RBX)
            .wrapping_add(cpu.gpr_u16(CpuState::RDI)),
        2 => cpu
            .gpr_u16(CpuState::RBP)
            .wrapping_add(cpu.gpr_u16(CpuState::RSI)),
        3 => cpu
            .gpr_u16(CpuState::RBP)
            .wrapping_add(cpu.gpr_u16(CpuState::RDI)),
        4 => cpu.gpr_u16(CpuState::RSI),
        5 => cpu.gpr_u16(CpuState::RDI),
        6 if mod_ == 0 => return Ok(disp),
        6 => cpu.gpr_u16(CpuState::RBP),
        7 => cpu.gpr_u16(CpuState::RBX),
        _ => return Err(ExecError::Unsupported(0)),
    };
    Ok(match mod_ {
        0 => base,
        1 | 2 => base.wrapping_add(disp),
        _ => return Err(ExecError::Unsupported(0)),
    })
}

/// 32-bit ModRM/SIB effective address (offset only).
/// Spec: Intel SDM Vol. 2 Chapter 2 — 32-bit addressing forms + SIB.
fn calc_ea32(cpu: &CpuState, insn: &DecodedInsn) -> Result<u32, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    let disp = insn.displacement as u32;
    if m.rm == 4 {
        let sib = insn.sib.ok_or(ExecError::Unsupported(insn.opcode))?;
        let scale = 1u32 << (sib >> 6);
        let index = (sib >> 3) & 7;
        let base_reg = sib & 7;
        let index_val = if index == 4 {
            0
        } else {
            cpu.gpr_u32(index as usize).wrapping_mul(scale)
        };
        let base_val = if base_reg == 5 && m.mod_ == 0 {
            0
        } else {
            cpu.gpr_u32(base_reg as usize)
        };
        return Ok(base_val.wrapping_add(index_val).wrapping_add(disp));
    }
    let base = match (m.mod_, m.rm) {
        (0, 5) => return Ok(disp),
        (_, 0) => cpu.gpr_u32(CpuState::RAX),
        (_, 1) => cpu.gpr_u32(CpuState::RCX),
        (_, 2) => cpu.gpr_u32(CpuState::RDX),
        (_, 3) => cpu.gpr_u32(CpuState::RBX),
        (_, 5) => cpu.gpr_u32(CpuState::RBP),
        (_, 6) => cpu.gpr_u32(CpuState::RSI),
        (_, 7) => cpu.gpr_u32(CpuState::RDI),
        _ => return Err(ExecError::Unsupported(insn.opcode)),
    };
    Ok(match m.mod_ {
        0 => base,
        1 | 2 => base.wrapping_add(disp),
        _ => return Err(ExecError::Unsupported(insn.opcode)),
    })
}

/// ModR/M.reg / opcode B0-B7 legacy byte register (AL..BH).
#[inline]
fn read_reg_u8(cpu: &CpuState, reg: u8) -> u8 {
    cpu.gpr_u8(reg as usize)
}

/// Write ModR/M.reg / opcode B0-B7 legacy byte register (AL..BH).
#[inline]
fn write_reg_u8(cpu: &mut CpuState, reg: u8, val: u8) {
    cpu.set_gpr_u8(reg as usize, val);
}

fn read_rm_u8(cpu: &CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<u8, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        // Legacy byte r/m: 0-3 AL/CL/DL/BL, 4-7 AH/CH/DH/BH (SDM Vol. 2 App. B).
        Ok(read_reg_u8(cpu, m.rm))
    } else {
        let (addr, _, uses_ss) = ea(cpu, insn, 1)?;
        bus.read_u8(addr)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

fn write_rm_u8(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    val: u8,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        write_reg_u8(cpu, m.rm, val);
        Ok(())
    } else {
        let (addr, _, uses_ss) = ea(cpu, insn, 1)?;
        bus.write_u8(addr, val)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

fn read_rm_u16(cpu: &CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<u16, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        Ok(cpu.gpr_u16(m.rm as usize))
    } else {
        let (addr, _, uses_ss) = ea(cpu, insn, 2)?;
        bus.read_u16(addr)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

fn write_rm_u16(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    val: u16,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        cpu.set_gpr_u16(m.rm as usize, val);
        Ok(())
    } else {
        let (addr, _, uses_ss) = ea(cpu, insn, 2)?;
        bus.write_u16(addr, val)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

fn read_rm_u32(cpu: &CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<u32, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        Ok(cpu.gpr_u32(m.rm as usize))
    } else {
        let (addr, _, uses_ss) = ea(cpu, insn, 4)?;
        bus.read_u32(addr)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

fn write_rm_u32(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    val: u32,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        cpu.set_gpr_u32(m.rm as usize, val);
        Ok(())
    } else {
        let (addr, _, uses_ss) = ea(cpu, insn, 4)?;
        bus.write_u32(addr, val)
            .map_err(|e| classify_mem_fault(e, uses_ss))
    }
}

/// Read an 8-byte memory operand; register form is `#UD`.
///
/// Spec: Intel SDM Vol. 2 "CMPXCHG8B" — destination is memory only.
fn read_mem_u64(cpu: &CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<u64, ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Err(arch_fault(6));
    }
    let (addr, _, uses_ss) = ea(cpu, insn, 8)?;
    let lo = bus
        .read_u32(addr)
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    let hi = bus
        .read_u32(addr.wrapping_add(4))
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    Ok(u64::from(lo) | (u64::from(hi) << 32))
}

/// Write an 8-byte memory operand; register form is `#UD`.
fn write_mem_u64(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    val: u64,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Err(arch_fault(6));
    }
    let (addr, _, uses_ss) = ea(cpu, insn, 8)?;
    bus.write_u32(addr, val as u32)
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    bus.write_u32(addr.wrapping_add(4), (val >> 32) as u32)
        .map_err(|e| classify_mem_fault(e, uses_ss))
}

/// Stack-address size selected by the cached `SS.B` bit.
///
/// The `0x67` address-size prefix applies to memory operands, **not** to the
/// stack pointer. Spec: Intel SDM Vol. 1 §6.2.2; Vol. 3 §3.4.5.1 (B flag).
fn stack_addr_size_32(cpu: &CpuState) -> bool {
    cpu.ss.default_big()
}

/// Current stack pointer within the `SS.B` window (`ESP` or zero-extended `SP`).
fn stack_pointer(cpu: &CpuState) -> u32 {
    if stack_addr_size_32(cpu) {
        cpu.gpr_u32(CpuState::RSP)
    } else {
        u32::from(cpu.gpr_u16(CpuState::RSP))
    }
}

/// Commit a stack pointer within the `SS.B` window.
///
/// `B=0` writes only `SP`, preserving `ESP[31:16]` exactly as the legacy
/// 16-bit path did.
fn set_stack_pointer(cpu: &mut CpuState, value: u32) {
    if stack_addr_size_32(cpu) {
        cpu.set_gpr_u32(CpuState::RSP, value);
    } else {
        cpu.set_gpr_u16(CpuState::RSP, value as u16);
    }
}

/// Step a stack pointer with an explicit `SS.B` width.
fn stack_step_width(addr_size_32: bool, base: u32, delta: i32) -> u32 {
    let stepped = base.wrapping_add(delta as u32);
    if addr_size_32 {
        stepped
    } else {
        u32::from(stepped as u16)
    }
}

/// Step a stack pointer, wrapping modulo 2^32 (`B=1`) or 2^16 (`B=0`).
fn stack_step(cpu: &CpuState, base: u32, delta: i32) -> u32 {
    stack_step_width(stack_addr_size_32(cpu), base, delta)
}

/// Stack push without `#SS` classification (used by IVT delivery itself).
fn push16_unchecked(cpu: &mut CpuState, bus: &mut dyn Bus, val: u16) -> Result<(), ExecError> {
    let old_sp = stack_pointer(cpu);
    let sp = stack_step(cpu, old_sp, -2);
    set_stack_pointer(cpu, sp);
    let addr = linear_addr(&cpu.ss, u64::from(sp));
    match bus.write_u16(addr, val) {
        Ok(()) => Ok(()),
        Err(e) => {
            set_stack_pointer(cpu, old_sp);
            Err(e)
        }
    }
}

fn push16(cpu: &mut CpuState, bus: &mut dyn Bus, val: u16) -> Result<(), ExecError> {
    let old_sp = stack_pointer(cpu);
    let sp = stack_step(cpu, old_sp, -2);
    // Limit check before mutating SP/ESP (SDM Vol. 3 §5.3 / §6.15 #SS).
    seg_linear_checked(&cpu.ss, u64::from(sp), 2, true)?;
    set_stack_pointer(cpu, sp);
    let addr = linear_addr(&cpu.ss, u64::from(sp));
    match bus.write_u16(addr, val) {
        Ok(()) => Ok(()),
        Err(e) => {
            set_stack_pointer(cpu, old_sp);
            Err(classify_mem_fault(e, true))
        }
    }
}

/// Read a 16-bit stack value and calculate the next bounded SP/ESP without
/// committing either. Segment-register POP uses this to validate the target
/// descriptor before any architectural update.
fn peek_pop16(cpu: &CpuState, bus: &mut dyn Bus) -> Result<(u16, u32), ExecError> {
    let sp = stack_pointer(cpu);
    let addr = seg_linear_checked(&cpu.ss, u64::from(sp), 2, true)?;
    let v = bus
        .read_u16(addr)
        .map_err(|e| classify_mem_fault(e, true))?;
    Ok((v, stack_step(cpu, sp, 2)))
}

fn pop16(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<u16, ExecError> {
    let (v, next_sp) = peek_pop16(cpu, bus)?;
    set_stack_pointer(cpu, next_sp);
    Ok(v)
}

/// PUSH with 32-bit operand size; the pointer width follows `SS.B`.
/// Spec: Intel SDM Vol. 2 "PUSH"; Vol. 1 §§3.6, 6.2.2.
fn push32(cpu: &mut CpuState, bus: &mut dyn Bus, val: u32) -> Result<(), ExecError> {
    let old_sp = stack_pointer(cpu);
    let sp = stack_step(cpu, old_sp, -4);
    seg_linear_checked(&cpu.ss, u64::from(sp), 4, true)?;
    set_stack_pointer(cpu, sp);
    let addr = linear_addr(&cpu.ss, u64::from(sp));
    match bus.write_u32(addr, val) {
        Ok(()) => Ok(()),
        Err(e) => {
            set_stack_pointer(cpu, old_sp);
            Err(classify_mem_fault(e, true))
        }
    }
}

/// Read the 32-bit stack top without committing the pointer.
fn peek_pop32(cpu: &CpuState, bus: &mut dyn Bus) -> Result<(u32, u32), ExecError> {
    let sp = stack_pointer(cpu);
    let addr = seg_linear_checked(&cpu.ss, u64::from(sp), 4, true)?;
    let v = bus
        .read_u32(addr)
        .map_err(|e| classify_mem_fault(e, true))?;
    Ok((v, stack_step(cpu, sp, 4)))
}

fn pop32(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<u32, ExecError> {
    let (v, next_sp) = peek_pop32(cpu, bus)?;
    set_stack_pointer(cpu, next_sp);
    Ok(v)
}

/// Release `count` bytes from the stack after a `RET`/`RETF` immediate.
/// Spec: Intel SDM Vol. 2 "RET" (near/far imm16).
fn stack_release(cpu: &mut CpuState, count: u16) {
    let sp = stack_step(cpu, stack_pointer(cpu), i32::from(count));
    set_stack_pointer(cpu, sp);
}

/// ModRM.reg → segment register index for MOV Sreg forms (SDM Vol. 2, MOV).
/// Returns None for reserved encodings (6, 7) which cause #UD.
fn sreg_from_modrm_reg(reg: u8) -> Option<u8> {
    match reg {
        0..=5 => Some(reg),
        _ => None,
    }
}

fn read_sreg_selector(cpu: &CpuState, sreg: u8) -> u16 {
    match sreg {
        0 => cpu.es.selector,
        1 => cpu.cs.selector,
        2 => cpu.ss.selector,
        3 => cpu.ds.selector,
        4 => cpu.fs.selector,
        5 => cpu.gs.selector,
        _ => unreachable!("sreg filtered by sreg_from_modrm_reg"),
    }
}

fn cr0_pe(cpu: &CpuState) -> bool {
    cpu.cr0 & 1 != 0
}

/// `EFLAGS.VM` — virtual-8086 mode (SDM Vol. 1 §3.4.3; Vol. 3 §20.1).
const EFLAGS_VM: u64 = 1 << 17;
/// `EFLAGS.NT` (nested task).
const EFLAGS_NT: u64 = 1 << 14;

fn eflags_vm(rflags: u64) -> bool {
    rflags & EFLAGS_VM != 0
}

fn eflags_iopl(rflags: u64) -> u8 {
    ((rflags >> 12) & 3) as u8
}

/// `#GP(0)` when VM86 software `INT n` lacks IOPL=3 (no VME).
///
/// Applies only to opcode `CD` (`INT imm8`). `INT3` and `INTO` are not
/// IOPL-sensitive and must not call this. Spec: Intel SDM Vol. 3 §20.2.2
/// Table 20-2; Vol. 2 INT n Virtual-8086 Mode Exceptions.
fn require_vm86_iopl_for_soft_int(cpu: &CpuState) -> Result<(), ExecError> {
    if eflags_vm(cpu.rflags) && eflags_iopl(cpu.rflags) < 3 {
        Err(arch_fault_with_error_code(13, 0))
    } else {
        Ok(())
    }
}

/// `#GP(0)` when `CLI`/`STI` lack sufficient IOPL privilege (no VME/PVI).
///
/// Real-address mode always succeeds. Protected mode (VM=0): require
/// `IOPL ≥ CPL`. Virtual-8086 mode: require `IOPL = 3` (CPL is forced to 3).
/// Spec: Intel SDM Vol. 2 "CLI"/"STI" Table 3-7; Vol. 3 §20.2.1.
fn require_iopl_for_cli_sti(cpu: &CpuState) -> Result<(), ExecError> {
    if !cr0_pe(cpu) {
        return Ok(());
    }
    let cpl = architectural_cpl(cpu);
    if eflags_iopl(cpu.rflags) >= cpl {
        Ok(())
    } else {
        Err(arch_fault_with_error_code(13, 0))
    }
}

/// Pop FLAGS/EFLAGS with IOPL/IF privilege masking.
///
/// VM86 without VME: `IOPL < 3` → `#GP(0)`. Otherwise load permitted bits;
/// `VM`/`RF` are never taken from the image; RF is cleared; bit 1 stays set.
/// `VIP`/`VIF` are **never** loaded from the image (VME is unsupported —
/// CPUID.VME clear, `CR4.VME` reserved); they remain sticky.
/// Spec: Intel SDM Vol. 2 "POPF/POPFD"; Vol. 3 §20.2.2 / Table 20-2 (VME=0).
fn popf_execute(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    operand_size_32: bool,
) -> Result<(), ExecError> {
    if cr0_pe(cpu) && eflags_vm(cpu.rflags) && eflags_iopl(cpu.rflags) < 3 {
        return Err(arch_fault_with_error_code(13, 0));
    }

    let image = if operand_size_32 {
        u64::from(pop32(cpu, bus)?)
    } else {
        u64::from(pop16(cpu, bus)?)
    };

    let cpl = architectural_cpl(cpu);
    let iopl = eflags_iopl(cpu.rflags);
    let vm = eflags_vm(cpu.rflags);

    // Bits always loadable from the image width (status/control except IF/IOPL).
    // Low-word changeable: CF PF AF ZF SF TF DF OF NT (+ IF/IOPL conditionally).
    const STATUS16: u64 = 0x08D5; // CF PF AF ZF SF TF DF OF (no IF/IOPL/NT yet)
    const NT_BIT: u64 = 1 << 14;
    const IF_BIT: u64 = 1 << 9;
    const IOPL_BITS: u64 = 3 << 12;
    // High dword changeable at CPL 0 (32-bit opsize): AC ID (VIF/VIP never from POPF).
    const HIGH_CPL0: u64 = (1 << 18) | (1 << 21); // AC | ID

    let mut change = STATUS16 | NT_BIT;
    if operand_size_32 {
        // RF is architecturally cleared after POPF; VM never loads from image.
        if cpl == 0 && !vm {
            change |= HIGH_CPL0;
        } else if !vm {
            // CPL > 0 protected: AC and ID still changeable on 32-bit POPF.
            change |= HIGH_CPL0;
        }
        // VM86 IOPL=3: all non-reserved except IOPL/VIP/VIF/VM/RF — IF yes.
        if vm {
            change |= IF_BIT;
        }
    }

    if !vm {
        if cpl == 0 {
            change |= IOPL_BITS | IF_BIT;
        } else {
            // IOPL never changes at CPL > 0.
            if cpl <= iopl {
                change |= IF_BIT;
            }
        }
    } else {
        // VM86 with IOPL=3: IF may change; IOPL may not.
        change |= IF_BIT;
    }

    let mask = if operand_size_32 {
        change
    } else {
        change & 0xFFFF
    };

    // Preserve bits outside the writable set, including VM/RF/VIP/VIF and
    // upper RFLAGS. Clear RF after POPF (SDM Vol. 2 POPF note).
    let preserve = !mask;
    let mut new_flags = (cpu.rflags & preserve) | (image & mask) | 2;
    new_flags &= !(1 << 16); // RF := 0
                             // Keep VM sticky (never from image).
    if vm {
        new_flags |= EFLAGS_VM;
    } else {
        new_flags &= !EFLAGS_VM;
    }
    if operand_size_32 {
        cpu.rflags = (cpu.rflags & !0xFFFF_FFFF) | (new_flags & 0xFFFF_FFFF);
    } else {
        cpu.rflags = (cpu.rflags & !0xFFFF) | (new_flags & 0xFFFF);
    }
    Ok(())
}

/// Architectural CPL: 0 in real-address mode, 3 while `EFLAGS.VM=1`, else CS.RPL.
///
/// Spec: Intel SDM Vol. 3 §5.5; §20.1.1 (VM86 forces CPL 3; CS[1:0] is not RPL).
fn architectural_cpl(cpu: &CpuState) -> u8 {
    if !cr0_pe(cpu) {
        0
    } else if eflags_vm(cpu.rflags) {
        3
    } else {
        (cpu.cs.selector & 3) as u8
    }
}

/// Null selector: index=0 and TI=0 (values 0000–0003). Spec: SDM Vol. 3 §3.4.2.
fn is_null_selector(selector: u16) -> bool {
    selector & 0xFFFC == 0
}

/// Write an 8-byte GDT/LDT-style segment descriptor into `out` (test / host helper).
/// `limit20` is the raw 20-bit limit field; `gran_flags` supplies G/D/B/L/AVL in bits 7:4.
#[cfg(test)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ParsedSegmentDescriptor {
    base: u64,
    limit: u32,
    flags: u16,
}

/// Parse the common base, effective limit, and cached attribute fields.
///
/// `flags` keeps the access byte in bits 7:0 and AVL/L/D-B/G in bits 15:12,
/// matching their relative positions in the descriptor. Spec: Intel SDM
/// Vol. 3 §§3.4.3–3.4.5.
fn parse_segment_descriptor(desc: [u8; 8]) -> ParsedSegmentDescriptor {
    let base = u64::from(desc[2])
        | (u64::from(desc[3]) << 8)
        | (u64::from(desc[4]) << 16)
        | (u64::from(desc[7]) << 24);
    let limit20 =
        u32::from(desc[0]) | (u32::from(desc[1]) << 8) | (u32::from(desc[6] & 0x0F) << 16);
    let limit = if desc[6] & 0x80 != 0 {
        (limit20 << 12) | 0xFFF
    } else {
        limit20
    };
    let flags = u16::from(desc[5]) | (u16::from(desc[6] & 0xF0) << 8);
    ParsedSegmentDescriptor { base, limit, flags }
}

/// Validate a same-CPL protected far code transfer without committing state.
///
/// Accepts a present nonconforming `L=0` GDT code segment at CPL 0 with
/// `DPL == CPL` and `RPL ≤ CPL`. Returns the CPL-adjusted selector and parsed
/// descriptor cache fields. Spec: Intel SDM Vol. 2 JMP/CALL (Protected Mode
/// Exceptions); Vol. 3 §§3.4.5, 5.8.1, 6.13.
fn prepare_protected_far_cs(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    target_offset: u32,
    selector: u16,
) -> Result<(u16, ParsedSegmentDescriptor), ExecError> {
    if is_null_selector(selector) {
        return Err(selector_fault(13, selector));
    }
    if selector & 0x4 != 0 {
        // LDT resolution is outside this bounded GDT-only slice.
        return Err(selector_fault(13, selector));
    }

    let descriptor_offset = u64::from(selector >> 3) * 8;
    if descriptor_offset + 7 > u64::from(cpu.gdtr.limit) {
        return Err(selector_fault(13, selector));
    }
    let descriptor_addr = cpu.gdtr.base.wrapping_add(descriptor_offset);
    let mut descriptor = [0u8; 8];
    for (index, byte) in descriptor.iter_mut().enumerate() {
        *byte = bus
            .read_u8(descriptor_addr.wrapping_add(index as u64))
            .map_err(|error| classify_mem_fault(error, false))?;
    }

    let access = descriptor[5];
    let code = access & 0x18 == 0x18;
    let conforming = access & 0x04 != 0;
    if !code || conforming {
        return Err(selector_fault(13, selector));
    }

    let cpl = (cpu.cs.selector & 3) as u8;
    let rpl = (selector & 3) as u8;
    let dpl = (access >> 5) & 3;
    if cpl != 0 || dpl != cpl || rpl > cpl {
        return Err(selector_fault(13, selector));
    }
    if access & 0x80 == 0 {
        return Err(selector_fault(11, selector));
    }

    let parsed = parse_segment_descriptor(descriptor);
    if parsed.flags & x86_core::SegmentReg::FLAG_LONG != 0 {
        // L=1 (64-bit) code segments remain a later milestone.
        return Err(selector_fault(13, selector));
    }
    if target_offset > parsed.limit {
        // Target offset beyond CS.limit → #GP(0).
        return Err(arch_fault_with_error_code(13, 0));
    }

    Ok(((selector & !3) | u16::from(cpl), parsed))
}

/// Protected-mode far `JMP` into a same-CPL GDT code segment, or a JMP-form
/// hardware task switch to a 32-bit TSS / task gate.
///
/// Code path: nonconforming ring-0 `L=0` GDT code (`D=0` or `D=1`).
/// Task path: available 32-bit TSS or task gate (see [`task_switch_jmp`]).
/// Spec: Intel SDM Vol. 2 JMP; Vol. 3 §§3.4.5, 5.8.1, 6.13, 7.2–7.3.
fn protected_far_jump(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    target_offset: u32,
    selector: u16,
    next_ip: u32,
) -> Result<(), ExecError> {
    if protected_far_is_task_target(cpu, bus, selector)? {
        return task_switch_jmp(cpu, bus, selector, next_ip);
    }
    let (sel, parsed) = prepare_protected_far_cs(cpu, bus, target_offset, selector)?;
    cpu.cs
        .load_descriptor_cache(sel, parsed.base, parsed.limit, parsed.flags);
    cpu.rip = u64::from(target_offset);
    Ok(())
}

/// Protected-mode far `CALL` into a same-CPL GDT code segment, through a
/// 32-bit GDT call gate, or a CALL-form hardware task switch to a 32-bit TSS /
/// task gate.
///
/// Call-gate path (type `0xC`, param count 0): same-CPL or privilege-changing
/// transfer using TSS `SSn:ESPn` when the target code DPL is more privileged.
/// Task path: available 32-bit TSS or task gate (see [`task_switch_call`]).
/// Spec: Intel SDM Vol. 2 CALL; Vol. 3 §§5.8.1–5.8.2, 6.13, 7.2–7.3.
///
/// Unsupported here: 16-bit call gates (`type=4`), non-zero param count,
/// LDT-resident gates (except LDT call gates already handled separately).
fn protected_far_call(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    target_offset: u32,
    selector: u16,
    next_ip: u32,
    operand_size_32: bool,
) -> Result<(), ExecError> {
    if protected_far_is_task_target(cpu, bus, selector)? {
        return task_switch_call(cpu, bus, selector, next_ip);
    }
    if protected_far_is_call_gate(cpu, bus, selector)? {
        return call_gate_far_call(cpu, bus, selector, next_ip);
    }
    let (sel, parsed) = prepare_protected_far_cs(cpu, bus, target_offset, selector)?;
    let return_cs = cpu.cs.selector;
    if operand_size_32 {
        push16(cpu, bus, return_cs)?;
        push32(cpu, bus, next_ip)?;
    } else {
        push16(cpu, bus, return_cs)?;
        push16(cpu, bus, next_ip as u16)?;
    }
    cpu.cs
        .load_descriptor_cache(sel, parsed.base, parsed.limit, parsed.flags);
    cpu.rip = u64::from(target_offset);
    Ok(())
}

/// System-descriptor type: 32-bit call gate (SDM Vol. 3 Table 3-2).
const DESC_TYPE_CALL_GATE32: u8 = 0xC;
/// System-descriptor type: 16-bit call gate (unsupported in this slice).
const DESC_TYPE_CALL_GATE16: u8 = 0x4;

fn protected_far_is_call_gate(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<bool, ExecError> {
    if is_null_selector(selector) {
        return Ok(false);
    }
    let access = if selector & 0x4 != 0 {
        if is_null_selector(cpu.ldtr.selector) {
            return Ok(false);
        }
        let offset = u64::from(selector >> 3) * 8;
        if offset + 7 > u64::from(cpu.ldtr.limit) {
            return Ok(false);
        }
        bus.read_system_u8(cpu.ldtr.base.wrapping_add(offset).wrapping_add(5))
            .map_err(|error| classify_mem_fault(error, false))?
    } else {
        let offset = u64::from(selector >> 3) * 8;
        if offset + 7 > u64::from(cpu.gdtr.limit) {
            return Ok(false);
        }
        bus.read_system_u8(cpu.gdtr.base.wrapping_add(offset).wrapping_add(5))
            .map_err(|error| classify_mem_fault(error, false))?
    };
    if access & 0x10 != 0 {
        return Ok(false);
    }
    Ok(matches!(
        access & 0x0F,
        DESC_TYPE_CALL_GATE16 | DESC_TYPE_CALL_GATE32
    ))
}

fn read_tss32_inner_stack_arch(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    new_cpl: u8,
) -> Result<(u16, u32), ExecError> {
    let type_field = (cpu.tr.flags & 0x0F) as u8;
    if type_field != DESC_TYPE_TSS32_BUSY && type_field != DESC_TYPE_TSS32_AVAILABLE {
        return Err(selector_fault(13, cpu.tr.selector));
    }
    if cpu.tr.limit < TSS32_MIN_LIMIT {
        return Err(selector_fault(13, cpu.tr.selector));
    }
    let (esp_off, ss_off) = match new_cpl {
        0 => (4u32, 8u32),
        1 => (12, 16),
        2 => (20, 24),
        _ => return Err(arch_fault_with_error_code(13, 0)),
    };
    if ss_off + 1 > cpu.tr.limit {
        return Err(selector_fault(13, cpu.tr.selector));
    }
    let base = cpu.tr.base;
    let esp = tss32_read_u32(bus, base, esp_off)?;
    let ss = tss32_read_u16(bus, base, ss_off)?;
    Ok((ss, esp))
}

/// Far `CALL` through a 32-bit GDT or LDT call gate (param count must be 0).
///
/// Spec: Intel SDM Vol. 2 "CALL"; Vol. 3 §5.8.2 Figures 5-8 / 5-9.
fn call_gate_far_call(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    gate_selector: u16,
    next_ip: u32,
) -> Result<(), ExecError> {
    let cpl = (cpu.cs.selector & 3) as u8;
    let rpl = (gate_selector & 3) as u8;
    let gate = read_dt_raw_descriptor(cpu, bus, gate_selector)?;
    let gate_access = gate[5];
    let gate_type = gate_access & 0x0F;
    if gate_type == DESC_TYPE_CALL_GATE16 {
        return Err(ExecError::Unsupported(0x9A));
    }
    if gate_type != DESC_TYPE_CALL_GATE32 {
        return Err(selector_fault(13, gate_selector));
    }
    let gate_dpl = (gate_access >> 5) & 3;
    if cpl > gate_dpl || rpl > gate_dpl {
        return Err(selector_fault(13, gate_selector));
    }
    if gate_access & 0x80 == 0 {
        return Err(selector_fault(11, gate_selector));
    }
    let param_count = gate[4] & 0x1F;
    if param_count != 0 {
        // Parameter copying is deferred; refuse rather than silently drop args.
        return Err(ExecError::Unsupported(0x9A));
    }

    let code_selector = u16::from_le_bytes([gate[2], gate[3]]);
    if is_null_selector(code_selector) || code_selector & 0x4 != 0 {
        // Target code through LDT remains out of scope for this slice.
        return Err(selector_fault(13, code_selector));
    }
    let code_desc = read_gdt_segment_descriptor(cpu, bus, code_selector)?;
    let code_access = code_desc[5];
    let s_bit = code_access & 0x10 != 0;
    let executable = code_access & 0x08 != 0;
    let conforming = executable && code_access & 0x04 != 0;
    let code_dpl = (code_access >> 5) & 3;
    if !s_bit || !executable {
        return Err(selector_fault(13, code_selector));
    }
    // Destination must be more privileged or same: DPL ≤ CPL.
    // Nonconforming with DPL < CPL switches privilege; conforming never
    // changes CPL. Spec: Vol. 3 §5.8.2.
    if code_dpl > cpl {
        return Err(selector_fault(13, code_selector));
    }
    if code_access & 0x80 == 0 {
        return Err(selector_fault(11, code_selector));
    }
    let parsed = parse_segment_descriptor(code_desc);
    if parsed.flags & x86_core::SegmentReg::FLAG_LONG != 0 {
        return Err(selector_fault(13, code_selector));
    }
    let gate_offset = u32::from(u16::from_le_bytes([gate[0], gate[1]]))
        | (u32::from(u16::from_le_bytes([gate[6], gate[7]])) << 16);
    if gate_offset > parsed.limit {
        return Err(arch_fault_with_error_code(13, 0));
    }

    let privilege_change = !conforming && code_dpl < cpl;
    let new_cpl = if privilege_change { code_dpl } else { cpl };
    let new_cs_sel = (code_selector & !3) | u16::from(new_cpl);

    let return_cs = cpu.cs.selector;
    let old_ss = cpu.ss.selector;
    let old_esp = stack_pointer(cpu);

    if privilege_change {
        let (ss_sel, mut sp) = read_tss32_inner_stack_arch(cpu, bus, new_cpl)?;
        let ss_loaded = prepare_ss_from_gdt_for_cpl(cpu, bus, ss_sel, new_cpl)?;
        // Privilege-changing CALL frame (Vol. 3 Figure 5-9), low→high:
        // EIP, CS, ESP, SS. Writes are supervisor accesses (§4.6.1).
        let frame = [next_ip, u32::from(return_cs), old_esp, u32::from(old_ss)];
        let stack_b32 = ss_loaded.default_big();
        for &value in frame.iter().rev() {
            sp = if stack_b32 {
                sp.wrapping_sub(4)
            } else {
                u32::from((sp as u16).wrapping_sub(4))
            };
            let addr = seg_linear_checked(&ss_loaded, u64::from(sp), 4, true)?;
            for (index, byte) in value.to_le_bytes().iter().enumerate() {
                bus.write_system_u8(addr.wrapping_add(index as u64), *byte)
                    .map_err(|error| classify_mem_fault(error, true))?;
            }
        }
        cpu.ss = ss_loaded;
        set_stack_pointer(cpu, sp);
    } else {
        // Same privilege: 32-bit gate pushes dword CS and EIP.
        push32(cpu, bus, u32::from(return_cs))?;
        push32(cpu, bus, next_ip)?;
    }
    cpu.cs
        .load_descriptor_cache(new_cs_sel, parsed.base, parsed.limit, parsed.flags);
    cpu.rip = u64::from(gate_offset);
    Ok(())
}

/// Protected-mode `IRET` / `IRETD`: nested-task return, same-CPL, outer return,
/// or return to virtual-8086 mode.
///
/// When `NT=1`, perform an IRET-form hardware task switch to the previous-task
/// link (Vol. 3 §7.3) instead of popping a stack frame. `next_ip` is the EIP
/// saved into the outgoing TSS for that path.
///
/// Otherwise `operand_size_32` selects the 32-bit `EIP`/`CS`/`EFLAGS` frame
/// (`IRETD`) or the 16-bit `IP`/`CS`/`FLAGS` frame (`IRET`). Stack-pointer
/// width follows the current `SS.B`. When the return CS.RPL is greater than CPL,
/// the frame also carries outer `ESP`/`SS`, which are validated and loaded so
/// CPL drops to the return RPL (Vol. 2 IRET; Vol. 3 §6.12.1).
///
/// When the 32-bit EFLAGS image has `VM=1` and the instruction runs at CPL 0,
/// the processor returns to virtual-8086 mode from the 9-dword PL0 frame
/// (EIP/CS/EFLAGS/ESP/SS/ES/DS/FS/GS) and forces CPL 3 (Vol. 2 IRET
/// RETURN-TO-VIRTUAL-8086-MODE; Vol. 3 §20.2 Figure 20-4).
///
/// Non-nested returns still require the instruction itself to execute at CPL 0.
/// `IRET` while already in VM86 is handled by [`vm86_iret`].
///
/// Spec: Intel SDM Vol. 2 IRET/IRETD/IRETQ; Vol. 1 §3.4.3; Vol. 3
/// §§3.4.2–3.4.5, 5.5, 6.12.1, 6.13, 7.3, 20.2–20.3.
fn protected_iret(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    operand_size_32: bool,
    next_ip: u32,
) -> Result<(), ExecError> {
    debug_assert!(!eflags_vm(cpu.rflags), "VM86 IRET must use vm86_iret");
    if cpu.rflags & EFLAGS_NT != 0 {
        return task_switch(cpu, bus, TaskSwitchCause::Iret, None, next_ip);
    }
    if architectural_cpl(cpu) != 0 {
        return Err(ExecError::Unsupported(0xCF));
    }
    let cpl = 0u8;

    let entry_size = if operand_size_32 { 4u32 } else { 2 };
    let old_sp = stack_pointer(cpu);
    let mut slots = [0u32; 5];
    for (index, slot) in slots.iter_mut().enumerate().take(3) {
        let stack_offset = stack_step(cpu, old_sp, (index as i32) * entry_size as i32);
        let addr = seg_linear_checked(
            &cpu.ss,
            u64::from(stack_offset),
            u64::from(entry_size),
            true,
        )?;
        *slot = if operand_size_32 {
            bus.read_u32(addr)
                .map_err(|error| classify_mem_fault(error, true))?
        } else {
            u32::from(
                bus.read_u16(addr)
                    .map_err(|error| classify_mem_fault(error, true))?,
            )
        };
    }
    let target_ip = slots[0];
    let selector = slots[1] as u16;
    let flags = slots[2];
    // CPL 0 + 32-bit operand size + VM in the image → VM86 enter.
    if operand_size_32 && eflags_vm(u64::from(flags)) {
        return return_to_virtual_8086_mode(cpu, bus, old_sp, target_ip, selector, flags);
    }

    if is_null_selector(selector) {
        return Err(selector_fault(13, selector));
    }
    if selector & 0x4 != 0 {
        return Err(selector_fault(13, selector));
    }

    let descriptor = read_gdt_segment_descriptor(cpu, bus, selector)?;
    let access = descriptor[5];
    let system = access & 0x10 == 0;
    let executable = access & 0x08 != 0;
    let conforming = access & 0x04 != 0;
    if system || !executable || conforming {
        return Err(selector_fault(13, selector));
    }

    let rpl = (selector & 3) as u8;
    let dpl = (access >> 5) & 3;
    if rpl < cpl {
        return Err(selector_fault(13, selector));
    }
    // Nonconforming code: DPL must equal the return RPL (new CPL).
    if dpl != rpl {
        return Err(selector_fault(13, selector));
    }
    if access & 0x80 == 0 {
        return Err(selector_fault(11, selector));
    }

    let parsed = parse_segment_descriptor(descriptor);
    if parsed.flags & x86_core::SegmentReg::FLAG_LONG != 0 {
        return Err(selector_fault(13, selector));
    }
    if target_ip > parsed.limit {
        return Err(arch_fault_with_error_code(13, 0));
    }

    let outer = rpl > cpl;
    let prepared_ss = if outer {
        for (index, slot) in slots.iter_mut().enumerate().skip(3).take(2) {
            let stack_offset = stack_step(cpu, old_sp, (index as i32) * entry_size as i32);
            let addr = seg_linear_checked(
                &cpu.ss,
                u64::from(stack_offset),
                u64::from(entry_size),
                true,
            )?;
            *slot = if operand_size_32 {
                bus.read_u32(addr)
                    .map_err(|error| classify_mem_fault(error, true))?
            } else {
                u32::from(
                    bus.read_u16(addr)
                        .map_err(|error| classify_mem_fault(error, true))?,
                )
            };
        }
        let outer_esp = slots[3];
        let outer_ss = slots[4] as u16;
        let prepared = prepare_ss_from_gdt_for_cpl(cpu, bus, outer_ss, rpl)?;
        Some((outer_esp, prepared))
    } else {
        None
    };

    // Defined flag bits at CPL 0: CF, PF, AF, ZF, SF, TF, IF, DF, OF, IOPL, NT
    // in the low word, plus RF, AC, VIF, VIP, and ID in the high word. VM is
    // excluded — a `VM=1` image took the VM86 path above. Reserved bits 3, 5,
    // and 15 stay clear and bit 1 stays set (SDM Vol. 1 §3.4.3, Figure 3-8).
    const DEFINED_FLAGS16: u64 = 0x7FD5;
    const DEFINED_FLAGS32: u64 = 0x003D_7FD5;
    let temp_sp = stack_step(cpu, old_sp, (if outer { 5 } else { 3 }) * entry_size as i32);

    cpu.cs
        .load_descriptor_cache(selector, parsed.base, parsed.limit, parsed.flags);
    cpu.rip = u64::from(target_ip);
    if operand_size_32 {
        cpu.rflags = (cpu.rflags & !0xFFFF_FFFF) | (u64::from(flags) & DEFINED_FLAGS32) | 2;
    } else {
        cpu.rflags = (cpu.rflags & !0xFFFF) | (u64::from(flags) & DEFINED_FLAGS16) | 2;
    }

    if let Some((outer_esp, ss)) = prepared_ss {
        cpu.ss = ss;
        if cpu.ss.default_big() {
            cpu.set_gpr_u32(CpuState::RSP, outer_esp);
        } else {
            cpu.set_gpr_u16(CpuState::RSP, outer_esp as u16);
        }
    } else {
        set_stack_pointer(cpu, temp_sp);
    }
    Ok(())
}

/// `IRETD` return to virtual-8086 mode from CPL 0 (9-dword PL0 frame).
///
/// Frame order at increasing addresses: EIP, CS, EFLAGS, ESP, SS, ES, DS, FS,
/// GS. Segment registers are loaded with real-address bases (`selector << 4`).
/// Architectural CPL becomes 3 via `EFLAGS.VM=1`.
///
/// Unsupported here: VME/PVI (`CR4` bits stay reserved/clear; CPUID does not
/// advertise them); 16-bit `IRET` enter (VM lives in EFLAGS[31:16]).
/// VM86→CPL0 delivery that **builds** this frame is in `deliver_protected_mode_gate`.
///
/// Spec: Intel SDM Vol. 2 "IRET/IRETD" RETURN-TO-VIRTUAL-8086-MODE; Vol. 3
/// §§20.2–20.3 Figure 20-4; Vol. 3 §3.4.2 (real-mode base).
fn return_to_virtual_8086_mode(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    old_sp: u32,
    eip: u32,
    cs_sel: u16,
    eflags: u32,
) -> Result<(), ExecError> {
    // Slots 0..2 already validated by the caller; read ESP/SS/ES/DS/FS/GS.
    let mut extra = [0u32; 6];
    for (index, slot) in extra.iter_mut().enumerate() {
        let stack_offset = stack_step(cpu, old_sp, ((index + 3) as i32) * 4);
        let addr = seg_linear_checked(&cpu.ss, u64::from(stack_offset), 4, true)?;
        *slot = bus
            .read_u32(addr)
            .map_err(|error| classify_mem_fault(error, true))?;
    }
    let new_esp = extra[0];
    let new_ss = extra[1] as u16;
    let new_es = extra[2] as u16;
    let new_ds = extra[3] as u16;
    let new_fs = extra[4] as u16;
    let new_gs = extra[5] as u16;

    // Real-mode-style CS limit is 64 KiB (Vol. 3 §3.4.2 / §20.1.3).
    if eip > 0xFFFF {
        return Err(arch_fault_with_error_code(13, 0));
    }

    // Defined EFLAGS bits including VM (bit 17). Same mask as CPL-0 IRETD
    // plus VM, which the non-VM86 path deliberately excludes.
    const DEFINED_FLAGS32_WITH_VM: u64 = 0x003F_7FD5;

    cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
    cpu.ss = x86_core::SegmentReg::real_mode(new_ss);
    cpu.es = x86_core::SegmentReg::real_mode(new_es);
    cpu.ds = x86_core::SegmentReg::real_mode(new_ds);
    cpu.fs = x86_core::SegmentReg::real_mode(new_fs);
    cpu.gs = x86_core::SegmentReg::real_mode(new_gs);
    cpu.rip = u64::from(eip);
    cpu.rflags = (cpu.rflags & !0xFFFF_FFFF) | (u64::from(eflags) & DEFINED_FLAGS32_WITH_VM) | 2;
    // ESP image is a full dword; subsequent VM86 stack ops use SP when B=0.
    cpu.set_gpr_u32(CpuState::RSP, new_esp);
    Ok(())
}

/// `IRET`/`IRETD` while already in virtual-8086 mode.
///
/// Without VME: `IOPL < 3` → `#GP(0)`; `IOPL = 3` → real-mode-like pop that
/// leaves `VM`/`IOPL`/`VIP`/`VIF` unchanged (Vol. 2 IRET
/// RETURN-FROM-VIRTUAL-8086-MODE). Leaving VM86 entirely requires a privilege
/// transition to CPL 0 (interrupt/task) then `IRETD` with `VM=0` in the image.
/// Unsupported: VME redirect of IOPL-sensitive IRET; VIP∧VIF `#GP` is a VME
/// feature and is **not** implemented here (CPUID.VME clear).
///
/// Spec: Intel SDM Vol. 2 "IRET/IRETD"; Vol. 3 §20.2.3 / ch.20 / Table 20-2.
fn vm86_iret(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    operand_size_32: bool,
) -> Result<(), ExecError> {
    if eflags_iopl(cpu.rflags) < 3 {
        return Err(arch_fault_with_error_code(13, 0));
    }

    // Preserve VM, IOPL, VIP, VIF across the flag load (Vol. 2 RETURN-FROM-VM86).
    const STICKY_HIGH: u64 = EFLAGS_VM | (3 << 12) | (1 << 19) | (1 << 20);

    if operand_size_32 {
        let eip = pop32(cpu, bus)?;
        let cs_raw = pop32(cpu, bus)?;
        let flags = pop32(cpu, bus)?;
        let cs_sel = cs_raw as u16;
        if eip > 0xFFFF {
            return Err(arch_fault_with_error_code(13, 0));
        }
        let sticky = cpu.rflags & STICKY_HIGH;
        // Load defined bits except the sticky VM/IOPL/VIP/VIF set.
        // RF (bit 16) is architecturally cleared after IRET (Vol. 2).
        const DEFINED32_NO_RF: u64 = 0x003C_7FD5;
        cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
        cpu.rip = u64::from(eip);
        cpu.rflags = (cpu.rflags & !0xFFFF_FFFF)
            | (u64::from(flags) & DEFINED32_NO_RF & !STICKY_HIGH)
            | sticky
            | 2;
    } else {
        let ip = pop16(cpu, bus)?;
        let cs_sel = pop16(cpu, bus)?;
        let flags = pop16(cpu, bus)?;
        let sticky_iopl = cpu.rflags & (3 << 12);
        cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
        cpu.set_ip16(ip);
        // Low FLAGS without IOPL; IOPL sticky. VM lives in high word.
        const DEFINED16_NO_IOPL: u64 = 0x4FD5; // excludes IOPL bits 13:12
        cpu.rflags =
            (cpu.rflags & !0xFFFF) | (u64::from(flags) & DEFINED16_NO_IOPL) | sticky_iopl | 2;
    }
    Ok(())
}

/// Read one complete GDT descriptor after common selector/table validation.
///
/// LDT lookup remains outside this bounded slice. All eight bytes are read
/// before a caller can commit visible or hidden segment state.
/// Spec: Intel SDM Vol. 3 §§3.4.2, 3.5.1, 6.13.
fn read_gdt_segment_descriptor(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<[u8; 8], ExecError> {
    if selector & 0x4 != 0 {
        return Err(selector_fault(13, selector));
    }
    let offset = u64::from(selector >> 3) * 8;
    if offset + 7 > u64::from(cpu.gdtr.limit) {
        return Err(selector_fault(13, selector));
    }
    let addr = cpu.gdtr.base.wrapping_add(offset);
    let mut descriptor = [0u8; 8];
    for (index, byte) in descriptor.iter_mut().enumerate() {
        // A descriptor-table read is a supervisor-mode access whatever the CPL
        // is (SDM Vol. 3 §4.6.1), so it must not go through the CPL-derived
        // data path.
        *byte = bus
            .read_system_u8(addr.wrapping_add(index as u64))
            .map_err(|error| classify_mem_fault(error, false))?;
    }
    Ok(descriptor)
}

/// Soft GDT descriptor fetch for `LAR`/`LSL`: null, LDT (TI=1), or out-of-limit
/// returns `None` (caller clears ZF). Memory faults still propagate.
/// Spec: Intel SDM Vol. 2 "LAR"/"LSL"; Vol. 3 §§3.5.1, 5.5.
fn try_read_gdt_descriptor_for_lar_lsl(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<Option<[u8; 8]>, ExecError> {
    if is_null_selector(selector) || selector & 0x4 != 0 {
        return Ok(None);
    }
    let offset = u64::from(selector >> 3) * 8;
    if offset + 7 > u64::from(cpu.gdtr.limit) {
        return Ok(None);
    }
    let addr = cpu.gdtr.base.wrapping_add(offset);
    let mut descriptor = [0u8; 8];
    for (index, byte) in descriptor.iter_mut().enumerate() {
        *byte = bus
            .read_system_u8(addr.wrapping_add(index as u64))
            .map_err(|error| classify_mem_fault(error, false))?;
    }
    Ok(Some(descriptor))
}

/// Whether a descriptor type is valid for `LAR` (SDM Vol. 2 Table for LAR).
fn lar_type_valid(access: u8) -> bool {
    if access & 0x10 != 0 {
        return true; // all code/data
    }
    matches!(access & 0x0F, 0x1 | 0x2 | 0x3 | 0x4 | 0x5 | 0x9 | 0xB | 0xC)
}

/// Whether a descriptor type is valid for `LSL` (SDM Vol. 2 Table for LSL).
fn lsl_type_valid(access: u8) -> bool {
    if access & 0x10 != 0 {
        return true; // all code/data
    }
    matches!(access & 0x0F, 0x1 | 0x2 | 0x3 | 0x9 | 0xB)
}

/// Soft privilege / type check shared by `LAR` and `LSL`.
///
/// Not-present descriptors clear ZF. Conforming code skips the DPL check.
/// Spec: Intel SDM Vol. 2 "LAR"/"LSL".
fn lar_lsl_descriptor_usable(access: u8, selector: u16, cpl: u8, for_lsl: bool) -> bool {
    if access & 0x80 == 0 {
        return false;
    }
    if for_lsl {
        if !lsl_type_valid(access) {
            return false;
        }
    } else if !lar_type_valid(access) {
        return false;
    }
    let s_bit = access & 0x10 != 0;
    let executable = access & 0x08 != 0;
    let conforming = s_bit && executable && access & 0x04 != 0;
    if conforming {
        return true;
    }
    let rpl = (selector & 3) as u8;
    let dpl = (access >> 5) & 3;
    cpl <= dpl && rpl <= dpl
}

/// Access-rights value loaded by `LAR` (bits 7:0 and 31:24 clear; 19:16 zeroed).
fn lar_access_rights_value(desc: [u8; 8]) -> u32 {
    (u32::from(desc[5]) << 8) | (u32::from(desc[6] & 0xF0) << 16)
}

/// Execute `LAR` or `LSL`. Real-address mode → `#UD`.
///
/// Spec: Intel SDM Vol. 2 "LAR"/"LSL". Unsupported here: LDT resolution (TI=1
/// clears ZF), long mode, and the `#UD` for a `LOCK` prefix.
fn exec_lar_lsl(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    for_lsl: bool,
) -> Result<(), ExecError> {
    if !cr0_pe(cpu) {
        return Err(arch_fault(6));
    }
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    // Source selector is always 16-bit (r16/m16), even under a 32-bit operand size.
    let selector = read_rm_u16(cpu, bus, insn)?;
    let cpl = (cpu.cs.selector & 3) as u8;
    let ok = match try_read_gdt_descriptor_for_lar_lsl(cpu, bus, selector)? {
        Some(desc) if lar_lsl_descriptor_usable(desc[5], selector, cpl, for_lsl) => {
            if for_lsl {
                let limit = parse_segment_descriptor(desc).limit;
                if opsz32(insn) {
                    cpu.set_gpr_u32(m.reg as usize, limit);
                } else {
                    cpu.set_gpr_u16(m.reg as usize, limit as u16);
                }
            } else {
                let ar = lar_access_rights_value(desc);
                if opsz32(insn) {
                    cpu.set_gpr_u32(m.reg as usize, ar);
                } else {
                    cpu.set_gpr_u16(m.reg as usize, ar as u16);
                }
            }
            true
        }
        _ => false,
    };
    cpu.set_zf(ok);
    Ok(())
}

/// Soft check for `VERR` / `VERW` (SDM Vol. 2).
///
/// `VERR`: present readable data or readable code, with the ordinary segment
/// privilege check (conforming code skips DPL). `VERW`: present writable data
/// only (`max(CPL,RPL) ≤ DPL`). System segments and execute-only code clear ZF.
fn verr_verw_usable(access: u8, selector: u16, cpl: u8, for_write: bool) -> bool {
    if access & 0x80 == 0 {
        return false;
    }
    let s_bit = access & 0x10 != 0;
    if !s_bit {
        return false;
    }
    let executable = access & 0x08 != 0;
    let conforming = executable && access & 0x04 != 0;
    let readable = !executable || access & 0x02 != 0;
    let writable = !executable && access & 0x02 != 0;
    if for_write {
        if !writable {
            return false;
        }
    } else if !readable {
        return false;
    }
    if conforming && !for_write {
        return true;
    }
    let rpl = (selector & 3) as u8;
    let dpl = (access >> 5) & 3;
    cpl <= dpl && rpl <= dpl
}

/// Execute `VERR` or `VERW`. Real-address mode → `#UD`.
///
/// Sets `ZF` only; does not load the segment. Spec: Intel SDM Vol. 2
/// "VERR"/"VERW". Unsupported here: LDT resolution (TI=1 clears ZF).
fn exec_verr_verw(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    for_write: bool,
) -> Result<(), ExecError> {
    if !cr0_pe(cpu) {
        return Err(arch_fault(6));
    }
    let _m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    let selector = read_rm_u16(cpu, bus, insn)?;
    let cpl = (cpu.cs.selector & 3) as u8;
    let ok = matches!(
        try_read_gdt_descriptor_for_lar_lsl(cpu, bus, selector)?,
        Some(desc) if verr_verw_usable(desc[5], selector, cpl, for_write)
    );
    cpu.set_zf(ok);
    Ok(())
}

/// Validate a DS/ES/FS/GS descriptor and return cached base/limit/AR.
///
/// Data and readable code are accepted. Data and nonconforming code require
/// both CPL and selector RPL no more privileged than DPL; conforming readable
/// code does not use that check. Type/privilege faults precede the P check.
/// Spec: Intel SDM Vol. 2 MOV/POP Sreg protected-mode checks; Vol. 3
/// §§3.4.5, 5.4.1, 5.5, 5.6.
fn parse_data_segment_descriptor(
    desc: [u8; 8],
    selector: u16,
    cpl: u8,
) -> Result<(u64, u32, u16), ExecError> {
    let access = desc[5];
    let present = access & 0x80 != 0;
    let s_bit = access & 0x10 != 0;
    let executable = access & 0x08 != 0;
    let conforming = executable && access & 0x04 != 0;
    let readable = !executable || access & 0x02 != 0;
    if !s_bit || !readable {
        // System or execute-only code descriptor.
        return Err(selector_fault(13, selector));
    }
    let rpl = (selector & 3) as u8;
    let dpl = (access >> 5) & 3;
    if !conforming && (cpl > dpl || rpl > dpl) {
        return Err(selector_fault(13, selector));
    }
    if !present {
        return Err(selector_fault(11, selector));
    }
    let parsed = parse_segment_descriptor(desc);
    Ok((parsed.base, parsed.limit, parsed.flags))
}

/// Prepare a protected-mode DS/ES/FS/GS cache without mutating CPU state.
fn prepare_data_sreg_from_gdt(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<x86_core::SegmentReg, ExecError> {
    if is_null_selector(selector) {
        return Ok(x86_core::SegmentReg {
            selector,
            base: 0,
            limit: 0,
            flags: 0,
        });
    }
    let descriptor = read_gdt_segment_descriptor(cpu, bus, selector)?;
    let cpl = (cpu.cs.selector & 3) as u8;
    let (base, limit, flags) = parse_data_segment_descriptor(descriptor, selector, cpl)?;
    Ok(x86_core::SegmentReg {
        selector,
        base,
        limit,
        flags,
    })
}

/// Protected-mode load of DS/ES/FS/GS from the GDT.
///
/// Spec: Intel SDM Vol. 2 MOV (Sreg, r/m16) protected-mode checks; Vol. 3 §3.5.1
/// (segment loading); §5.4.1 (null selector into DS/ES/FS/GS allowed).
/// Unsupported here: LDT resolution.
fn load_data_sreg_from_gdt(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    sreg: u8,
    selector: u16,
) -> Result<(), ExecError> {
    debug_assert!(
        matches!(sreg, 0 | 3 | 4 | 5),
        "only DS/ES/FS/GS in this helper"
    );
    let loaded = prepare_data_sreg_from_gdt(cpu, bus, selector)?;
    match sreg {
        0 => cpu.es = loaded,
        3 => cpu.ds = loaded,
        4 => cpu.fs = loaded,
        5 => cpu.gs = loaded,
        _ => unreachable!(),
    }
    Ok(())
}

/// Parse an 8-byte stack-segment descriptor (writable data / expand-down).
///
/// Requires P=1, S=1, non-executable, writable (W=1). Applies G-bit to limit.
/// Spec: Intel SDM Vol. 2 MOV/POP SS protected-mode checks; Vol. 3 §§3.4.5,
/// 5.4.1, 5.5, 5.7. Not present → `#SS(selector)` (vector 12), not `#NP`.
fn parse_stack_segment_descriptor(
    desc: [u8; 8],
    selector: u16,
    cpl: u8,
) -> Result<(u64, u32, u16), ExecError> {
    let access = desc[5];
    let present = access & 0x80 != 0;
    let s_bit = access & 0x10 != 0;
    let executable = access & 0x08 != 0;
    let writable = access & 0x02 != 0;
    let rpl = (selector & 3) as u8;
    let dpl = (access >> 5) & 3;
    if !s_bit || executable || !writable || rpl != cpl || dpl != cpl {
        return Err(selector_fault(13, selector));
    }
    if !present {
        return Err(selector_fault(12, selector));
    }
    let parsed = parse_segment_descriptor(desc);
    Ok((parsed.base, parsed.limit, parsed.flags))
}

/// Prepare a stack-segment cache for a specific CPL (privilege-change delivery).
///
/// Spec: Intel SDM Vol. 3 §6.12.1 (stack switch), §5.4.1 / §5.7 (SS checks).
fn prepare_ss_from_gdt_for_cpl(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
    cpl: u8,
) -> Result<x86_core::SegmentReg, ExecError> {
    if is_null_selector(selector) {
        return Err(selector_fault(13, selector));
    }
    let descriptor = read_gdt_segment_descriptor(cpu, bus, selector)?;
    let (base, limit, flags) = parse_stack_segment_descriptor(descriptor, selector, cpl)?;
    Ok(x86_core::SegmentReg {
        selector,
        base,
        limit,
        flags,
    })
}

/// Read `SSn:ESPn` for an inner privilege level from the current 32-bit TSS.
///
/// Spec: Intel SDM Vol. 3 §6.12.1 Figure 6-5; §7.2.1 (TSS offsets).
fn read_tss32_inner_stack(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    new_cpl: u8,
    vector: u8,
) -> Result<(u16, u32), ExecError> {
    let type_field = (cpu.tr.flags & 0x0F) as u8;
    if type_field != DESC_TYPE_TSS32_BUSY && type_field != DESC_TYPE_TSS32_AVAILABLE {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::TssInvalid,
        ));
    }
    if cpu.tr.limit < TSS32_MIN_LIMIT {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::TssLimit,
        ));
    }
    let (esp_off, ss_off) = match new_cpl {
        0 => (4u32, 8u32),
        1 => (12, 16),
        2 => (20, 24),
        _ => {
            return Err(protected_mode_delivery_error(
                vector,
                ProtectedModeDeliveryError::TssInvalid,
            ));
        }
    };
    if ss_off + 1 > cpu.tr.limit {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::TssLimit,
        ));
    }
    let base = cpu.tr.base;
    let mut esp_bytes = [0u8; 4];
    for (index, byte) in esp_bytes.iter_mut().enumerate() {
        let addr = base.wrapping_add(u64::from(esp_off) + index as u64);
        *byte = bus.read_system_u8(addr).map_err(|_| {
            protected_mode_delivery_error(vector, ProtectedModeDeliveryError::TssRead(addr))
        })?;
    }
    let mut ss_bytes = [0u8; 2];
    for (index, byte) in ss_bytes.iter_mut().enumerate() {
        let addr = base.wrapping_add(u64::from(ss_off) + index as u64);
        *byte = bus.read_system_u8(addr).map_err(|_| {
            protected_mode_delivery_error(vector, ProtectedModeDeliveryError::TssRead(addr))
        })?;
    }
    Ok((u16::from_le_bytes(ss_bytes), u32::from_le_bytes(esp_bytes)))
}

/// Prepare a protected-mode SS cache without mutating CPU state.
fn prepare_ss_from_gdt(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<x86_core::SegmentReg, ExecError> {
    let cpl = (cpu.cs.selector & 3) as u8;
    prepare_ss_from_gdt_for_cpl(cpu, bus, selector, cpl)
}

/// Protected-mode load of SS from the GDT.
///
/// Spec: Intel SDM Vol. 2 MOV (SS, r/m16); Vol. 3 §3.5.1 / §5.4.1.
/// Null selector → `#GP(0)`; P=0 → `#SS(selector)`; non-writable/code/system →
/// `#GP(selector)`; index outside GDTR.limit → `#GP(selector)`.
/// Unsupported here: LDT resolution.
fn load_ss_from_gdt(cpu: &mut CpuState, bus: &mut dyn Bus, selector: u16) -> Result<(), ExecError> {
    let loaded = prepare_ss_from_gdt(cpu, bus, selector)?;
    cpu.ss = loaded;
    Ok(())
}

fn write_sreg(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    sreg: u8,
    selector: u16,
) -> Result<(), ExecError> {
    // Caller must reject MOV CS and reserved Sreg encodings (#UD) before calling.
    // PE=0 or VM86: sticky unreal / real-address base (SDM Vol. 3 §3.4.2–§3.4.3,
    // §20.1.1). PE=1 and VM=0: DS/ES/FS/GS/SS load from GDT.
    let use_gdt = cr0_pe(cpu) && !eflags_vm(cpu.rflags);
    match sreg {
        0 | 3 | 4 | 5 if use_gdt => load_data_sreg_from_gdt(cpu, bus, sreg, selector),
        2 if use_gdt => load_ss_from_gdt(cpu, bus, selector),
        0 => {
            cpu.es.load_real_mode_selector(selector);
            Ok(())
        }
        2 => {
            cpu.ss.load_real_mode_selector(selector);
            Ok(())
        }
        3 => {
            cpu.ds.load_real_mode_selector(selector);
            Ok(())
        }
        4 => {
            cpu.fs.load_real_mode_selector(selector);
            Ok(())
        }
        5 => {
            cpu.gs.load_real_mode_selector(selector);
            Ok(())
        }
        _ => Err(ExecError::Unsupported(0x8E)),
    }
}

/// PUSH a segment selector using the effective operand-size attribute.
///
/// A 32-bit operand size decrements the stack pointer by four and writes the
/// zero-extended selector; a 16-bit one writes the bare selector word.
/// Spec: Intel SDM Vol. 2 "PUSH" (Operation, segment-register source).
fn push_sreg(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    selector: u16,
    operand_size_32: bool,
) -> Result<(), ExecError> {
    if operand_size_32 {
        push32(cpu, bus, u32::from(selector))
    } else {
        push16(cpu, bus, selector)
    }
}

/// POP a 16-bit selector into ES/SS/DS/FS/GS with a single atomic commit.
///
/// The stack slot and, in protected mode, all descriptor bytes/checks complete
/// before either SP or the destination cache changes. The old SS cache is used
/// for the stack read even for POP SS. A 32-bit operand size consumes a
/// doubleword slot and takes the selector from its low word.
/// Spec: Intel SDM Vol. 2 POP; Vol. 3 §§3.5.1, 5.4.1.
fn pop_sreg(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    sreg: u8,
    operand_size_32: bool,
) -> Result<(), ExecError> {
    debug_assert!(matches!(sreg, 0 | 2 | 3 | 4 | 5));
    let (selector, next_sp) = if operand_size_32 {
        let (value, next_sp) = peek_pop32(cpu, bus)?;
        (value as u16, next_sp)
    } else {
        peek_pop16(cpu, bus)?
    };
    // POP SS pops through the *old* stack segment, so the pointer width for the
    // committed SP/ESP is the old `SS.B` (SDM Vol. 2 POP, Operation).
    let old_stack_addr_size_32 = stack_addr_size_32(cpu);
    let protected_cache = prepare_sreg_load(cpu, bus, sreg, selector)?;

    commit_sreg_load(cpu, sreg, selector, protected_cache);
    if old_stack_addr_size_32 {
        cpu.set_gpr_u32(CpuState::RSP, next_sp);
    } else {
        cpu.set_gpr_u16(CpuState::RSP, next_sp as u16);
    }
    Ok(())
}

/// Validate a selector for ES/SS/DS/FS/GS without mutating CPU state.
///
/// Returns `None` in real-address mode **and** virtual-8086 mode, where the
/// load is the sticky-unreal / real-address `selector << 4` base update with
/// no descriptor to read (Vol. 3 §20.1.1). Protected mode (`PE=1`, `VM=0`)
/// reads the GDT.
/// Spec: Intel SDM Vol. 3 §§3.4.2, 3.5.1, 5.4.1, 20.1.1.
fn prepare_sreg_load(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    sreg: u8,
    selector: u16,
) -> Result<Option<x86_core::SegmentReg>, ExecError> {
    debug_assert!(matches!(sreg, 0 | 2 | 3 | 4 | 5));
    if !cr0_pe(cpu) || eflags_vm(cpu.rflags) {
        return Ok(None);
    }
    Ok(Some(if sreg == 2 {
        prepare_ss_from_gdt(cpu, bus, selector)?
    } else {
        prepare_data_sreg_from_gdt(cpu, bus, selector)?
    }))
}

/// Commit a previously validated ES/SS/DS/FS/GS load.
fn commit_sreg_load(
    cpu: &mut CpuState,
    sreg: u8,
    selector: u16,
    prepared: Option<x86_core::SegmentReg>,
) {
    let target = match sreg {
        0 => &mut cpu.es,
        2 => &mut cpu.ss,
        3 => &mut cpu.ds,
        4 => &mut cpu.fs,
        5 => &mut cpu.gs,
        _ => unreachable!("segment register index must be ES/SS/DS/FS/GS"),
    };
    match prepared {
        Some(loaded) => *target = loaded,
        None => target.load_real_mode_selector(selector),
    }
}

/// `LSS`/`LFS`/`LGS` — load a far pointer into a GPR and a segment register.
///
/// The complete pointer is read and, in protected mode, the descriptor is
/// validated before anything commits, so a fault leaves the CPU untouched.
/// `SS` uses the stack-segment descriptor rules (null selector → `#GP(0)`,
/// writable data required); `FS`/`GS` use the DS/ES data rules and accept a
/// null selector. The register form (`mod=11`) is `#UD`.
/// Spec: Intel SDM Vol. 2 "LDS/LES/LFS/LGS/LSS—Load Far Pointer"; Vol. 3
/// §§3.4.2–3.4.5, 5.4.1, 6.15.
fn load_far_pointer(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    sreg: u8,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Err(arch_fault(6));
    }
    if opsz32(insn) {
        let (offset, selector) = read_far_ptr32(cpu, bus, insn)?;
        let prepared = prepare_sreg_load(cpu, bus, sreg, selector)?;
        cpu.set_gpr_u32(m.reg as usize, offset);
        commit_sreg_load(cpu, sreg, selector, prepared);
    } else {
        let (offset, selector) = read_far_ptr16(cpu, bus, insn)?;
        let prepared = prepare_sreg_load(cpu, bus, sreg, selector)?;
        cpu.set_gpr_u16(m.reg as usize, offset);
        commit_sreg_load(cpu, sreg, selector, prepared);
    }
    Ok(())
}

/// SI/DI step for string ops: +size if DF=0, −size if DF=1 (SDM Vol. 1 §3.4.3).
fn string_index_delta(cpu: &CpuState, size: u16) -> u16 {
    if cpu.direction_flag() {
        size.wrapping_neg()
    } else {
        size
    }
}

/// ESI/EDI step for address-size 32 string ops (SDM Vol. 1 §3.4.3 / §3.6).
fn string_index_delta32(cpu: &CpuState, size: u32) -> u32 {
    if cpu.direction_flag() {
        size.wrapping_neg()
    } else {
        size
    }
}

fn data_seg_for_string_src<'a>(cpu: &'a CpuState, insn: &DecodedInsn) -> &'a x86_core::SegmentReg {
    match insn.prefixes.segment_override {
        Some(0x26) => &cpu.es,
        Some(0x2E) => &cpu.cs,
        Some(0x36) => &cpu.ss,
        Some(0x64) => &cpu.fs,
        Some(0x65) => &cpu.gs,
        Some(0x3E) | None => &cpu.ds,
        _ => &cpu.ds,
    }
}

/// SS override on string/moffs source → `#SS`; otherwise `#GP`.
/// Spec: Intel SDM Vol. 3 §6.15 (#SS / #GP).
fn string_src_uses_ss(insn: &DecodedInsn) -> bool {
    matches!(insn.prefixes.segment_override, Some(0x36))
}

fn map_string_src_fault(err: ExecError, insn: &DecodedInsn) -> ExecError {
    classify_mem_fault(err, string_src_uses_ss(insn))
}

/// ES: string destination / SCAS — not SS → `#GP`.
fn map_es_mem_fault(err: ExecError) -> ExecError {
    classify_mem_fault(err, false)
}

/// String source linear address with cached segment-limit check.
/// Spec: Intel SDM Vol. 3 §5.3; Vol. 2 MOVS/LODS/CMPS/OUTS.
fn string_src_linear(
    cpu: &CpuState,
    insn: &DecodedInsn,
    offset: u64,
    size: u64,
) -> Result<u64, ExecError> {
    seg_linear_checked(
        data_seg_for_string_src(cpu, insn),
        offset,
        size,
        string_src_uses_ss(insn),
    )
}

/// ES:(E)DI string destination / SCAS linear address with limit check → `#GP`.
/// Spec: Intel SDM Vol. 3 §5.3; Vol. 2 MOVS/STOS/SCAS/INS.
fn string_es_linear(cpu: &CpuState, offset: u64, size: u64) -> Result<u64, ExecError> {
    seg_linear_checked(&cpu.es, offset, size, false)
}

fn zf_set(cpu: &CpuState) -> bool {
    cpu.rflags & (1 << 6) != 0
}

/// One MOVSB iteration (no IP update). Spec: SDM Vol. 2 MOVS/MOVSB.
fn movsb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u8(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u8(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One STOSB iteration (no IP update). Spec: SDM Vol. 2 STOS/STOSB.
fn stosb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        bus.write_u8(dst, cpu.al()).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        bus.write_u8(dst, cpu.al()).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One LODSB iteration (no IP update). Spec: SDM Vol. 2 LODS/LODSB.
fn lodsb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_al(v);
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_al(v);
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One SCASB iteration (no IP update). Spec: SDM Vol. 2 SCAS/SCASB.
fn scasb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 1)?;
        let mem = bus.read_u8(addr).map_err(map_es_mem_fault)?;
        let al = cpu.al();
        let result = al.wrapping_sub(mem);
        set_sub_flags_u8(cpu, al, mem, result);
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 1)?;
        let mem = bus.read_u8(addr).map_err(map_es_mem_fault)?;
        let al = cpu.al();
        let result = al.wrapping_sub(mem);
        set_sub_flags_u8(cpu, al, mem, result);
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One CMPSB iteration (no IP update). Spec: SDM Vol. 2 CMPS/CMPSB.
fn cmpsb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let a = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u8(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u8(cpu, a, b, result);
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        let a = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u8(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u8(cpu, a, b, result);
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One MOVSW iteration (no IP update). Spec: SDM Vol. 2 MOVS/MOVSW.
fn movsw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u16(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u16(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One MOVSD iteration (no IP update). Spec: SDM Vol. 2 MOVS/MOVSD (opsize 32).
fn movsd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u32(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.write_u32(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One STOSW iteration (no IP update). Spec: SDM Vol. 2 STOS/STOSW.
fn stosw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        bus.write_u16(dst, cpu.ax()).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        bus.write_u16(dst, cpu.ax()).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One STOSD iteration (no IP update). Spec: SDM Vol. 2 STOS/STOSD (opsize 32).
fn stosd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        bus.write_u32(dst, cpu.eax()).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        bus.write_u32(dst, cpu.eax()).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One LODSW iteration (no IP update). Spec: SDM Vol. 2 LODS/LODSW.
fn lodsw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_ax(v);
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_ax(v);
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One LODSD iteration (no IP update). Spec: SDM Vol. 2 LODS/LODSD (opsize 32).
fn lodsd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_eax(v);
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        cpu.set_eax(v);
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One SCASW iteration (no IP update). Spec: SDM Vol. 2 SCAS/SCASW.
fn scasw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 2)?;
        let mem = bus.read_u16(addr).map_err(map_es_mem_fault)?;
        let ax = cpu.ax();
        let result = ax.wrapping_sub(mem);
        set_sub_flags_u16(cpu, ax, mem, result);
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 2)?;
        let mem = bus.read_u16(addr).map_err(map_es_mem_fault)?;
        let ax = cpu.ax();
        let result = ax.wrapping_sub(mem);
        set_sub_flags_u16(cpu, ax, mem, result);
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One SCASD iteration (no IP update). Spec: SDM Vol. 2 SCAS/SCASD (opsize 32).
fn scasd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 4)?;
        let mem = bus.read_u32(addr).map_err(map_es_mem_fault)?;
        let eax = cpu.eax();
        let result = eax.wrapping_sub(mem);
        set_sub_flags_u32(cpu, eax, mem, result);
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let addr = string_es_linear(cpu, u64::from(di), 4)?;
        let mem = bus.read_u32(addr).map_err(map_es_mem_fault)?;
        let eax = cpu.eax();
        let result = eax.wrapping_sub(mem);
        set_sub_flags_u32(cpu, eax, mem, result);
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One CMPSW iteration (no IP update). Spec: SDM Vol. 2 CMPS/CMPSW.
fn cmpsw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let a = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u16(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u16(cpu, a, b, result);
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        let a = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u16(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u16(cpu, a, b, result);
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One CMPSD iteration (no IP update). Spec: SDM Vol. 2 CMPS/CMPSD (opsize 32).
fn cmpsd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let di = cpu.gpr_u32(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let a = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u32(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u32(cpu, a, b, result);
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let di = cpu.gpr_u16(CpuState::RDI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        let a = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        let b = bus.read_u32(dst).map_err(map_es_mem_fault)?;
        let result = a.wrapping_sub(b);
        set_sub_flags_u32(cpu, a, b, result);
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One INSB iteration (no IP update). Spec: SDM Vol. 2 INS/INSB/INSW/INSD.
/// Port = DX; destination = ES:(E)DI (no segment override for dest).
fn insb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        bus.probe_write(dst, 1).map_err(map_es_mem_fault)?;
        let v = bus.port_in_u8(port)?;
        bus.write_u8(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 1)?;
        bus.probe_write(dst, 1).map_err(map_es_mem_fault)?;
        let v = bus.port_in_u8(port)?;
        bus.write_u8(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One INSW iteration (no IP update). Spec: SDM Vol. 2 INS/INSW.
fn insw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        bus.probe_write(dst, 2).map_err(map_es_mem_fault)?;
        let v = bus.port_in_u16(port)?;
        bus.write_u16(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 2)?;
        bus.probe_write(dst, 2).map_err(map_es_mem_fault)?;
        let v = bus.port_in_u16(port)?;
        bus.write_u16(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One INSD iteration (no IP update). Spec: SDM Vol. 2 INS/INSD (opsize 32).
fn insd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let di = cpu.gpr_u32(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        bus.probe_write(dst, 4).map_err(map_es_mem_fault)?;
        let v = bus.port_in_u32(port)?;
        bus.write_u32(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RDI, di.wrapping_add(d));
    } else {
        let di = cpu.gpr_u16(CpuState::RDI);
        let dst = string_es_linear(cpu, u64::from(di), 4)?;
        bus.probe_write(dst, 4).map_err(map_es_mem_fault)?;
        let v = bus.port_in_u32(port)?;
        bus.write_u32(dst, v).map_err(map_es_mem_fault)?;
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RDI, di.wrapping_add(d));
    }
    Ok(())
}

/// One OUTSB iteration (no IP update). Spec: SDM Vol. 2 OUTS/OUTSB/OUTSW/OUTSD.
/// Port = DX; source = DS:(E)SI (segment override allowed).
fn outsb_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u8(port, v)?;
        let d = string_index_delta32(cpu, 1);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 1)?;
        let v = bus
            .read_u8(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u8(port, v)?;
        let d = string_index_delta(cpu, 1);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One OUTSW iteration (no IP update). Spec: SDM Vol. 2 OUTS/OUTSW.
fn outsw_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u16(port, v)?;
        let d = string_index_delta32(cpu, 2);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 2)?;
        let v = bus
            .read_u16(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u16(port, v)?;
        let d = string_index_delta(cpu, 2);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// One OUTSD iteration (no IP update). Spec: SDM Vol. 2 OUTS/OUTSD (opsize 32).
fn outsd_once(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    let port = cpu.gpr_u16(CpuState::RDX);
    if asize32(insn) {
        let si = cpu.gpr_u32(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u32(port, v)?;
        let d = string_index_delta32(cpu, 4);
        cpu.set_gpr_u32(CpuState::RSI, si.wrapping_add(d));
    } else {
        let si = cpu.gpr_u16(CpuState::RSI);
        let src = string_src_linear(cpu, insn, u64::from(si), 4)?;
        let v = bus
            .read_u32(src)
            .map_err(|e| map_string_src_fault(e, insn))?;
        bus.port_out_u32(port, v)?;
        let d = string_index_delta(cpu, 4);
        cpu.set_gpr_u16(CpuState::RSI, si.wrapping_add(d));
    }
    Ok(())
}

/// `IN` into the accumulator (`E4`/`E5`/`EC`/`ED`), no IP update.
///
/// `byte_form` is the fixed-`AL` encoding (`E4`/`EC`); otherwise the
/// operand-size attribute selects `AX` or `EAX`. A 16-bit destination writes
/// only `AX`, leaving `EAX[31:16]` untouched (SDM Vol. 1 §3.4.1.1). The access
/// goes through the width-specific `Bus` port accessors, the same ones
/// `INSB`/`INSW`/`INSD` use, so a word or doubleword port sees one access of
/// that width. No flags are affected.
/// Spec: Intel SDM Vol. 2 "IN—Input from Port"; Vol. 1 §3.6 (Table 3-4).
fn port_in_accumulator(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    port: u16,
    byte_form: bool,
) -> Result<(), ExecError> {
    if byte_form {
        let v = bus.port_in_u8(port)?;
        cpu.set_al(v);
    } else if opsz32(insn) {
        let v = bus.port_in_u32(port)?;
        cpu.set_eax(v);
    } else {
        let v = bus.port_in_u16(port)?;
        cpu.set_ax(v);
    }
    Ok(())
}

/// `OUT` from the accumulator (`E6`/`E7`/`EE`/`EF`), no IP update.
///
/// Mirrors [`port_in_accumulator`]: `byte_form` writes `AL`, otherwise the
/// operand-size attribute selects `AX` or `EAX`, and the transfer is one
/// access of that width. No flags are affected.
/// Spec: Intel SDM Vol. 2 "OUT—Output to Port"; Vol. 1 §3.6 (Table 3-4).
fn port_out_accumulator(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    port: u16,
    byte_form: bool,
) -> Result<(), ExecError> {
    if byte_form {
        bus.port_out_u8(port, cpu.al())
    } else if opsz32(insn) {
        bus.port_out_u32(port, cpu.eax())
    } else {
        bus.port_out_u16(port, cpu.ax())
    }
}

/// Architectural `#NMI` vector (Intel SDM Vol. 3 §6.3.3 / §6.15).
const VECTOR_NMI: u8 = 2;

/// Service a latched platform `#NMI` if pending.
///
/// Not gated by `RFLAGS.IF`. Clears `halted` so NMI can wake `HLT`.
/// Spec: Intel SDM Vol. 3 §§6.3.3, 6.7 (NMI), 6.12.1 (protected delivery).
/// Stub: no SMRAM/SMI, no NMI blocking window after delivery.
fn service_pending_nmi(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<bool, ExecError> {
    if !cpu.pending_nmi {
        return Ok(false);
    }
    let return_ip = current_ip(cpu);
    deliver_hardware_interrupt(cpu, bus, VECTOR_NMI, return_ip)?;
    cpu.pending_nmi = false;
    cpu.halted = false;
    Ok(true)
}

/// Service a latched maskable external IRQ if `IF=1`.
///
/// Pulls [`Bus::poll_external_irq`] into [`CpuState::pending_irq`], then
/// delivers through the current mode's IVT or IDT when enabled. Return IP is
/// the current instruction start (REP string ops leave IP unadvanced until
/// completion).
///
/// Spec: Intel SDM Vol. 2 "REP/REPE/REPNE" (service pending interrupts between
/// iterations); Vol. 3 §§6.8.1, 6.8.3 (maskable interrupts when IF=1 and the
/// MOV/POP SS inhibition window is inactive).
/// Stub: not a full 8259 — no priority / IRR / EOI.
fn service_pending_external_interrupt(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
) -> Result<bool, ExecError> {
    if let Some(vector) = bus.poll_external_irq() {
        cpu.request_interrupt(vector);
    }
    if cpu.maskable_interrupts_inhibited() || !cpu.interrupt_flag() {
        return Ok(false);
    }
    let Some(vector) = cpu.pending_irq else {
        return Ok(false);
    };
    let return_ip = current_ip(cpu);
    deliver_hardware_interrupt(cpu, bus, vector, return_ip)?;
    cpu.pending_irq = None;
    cpu.halted = false;
    Ok(true)
}

/// REP / REPE / REPNE wrapper — count = CX (asize16) or ECX (asize32).
///
/// Spec: Intel SDM Vol. 2 "REP/REPE/REPNE/REPZ/REPNZ"; Vol. 1 §3.6.
/// - `zf_terminate`: `None` = unconditional REP (MOVS/STOS/LODS);
///   `Some(true)` = REPE (stop when ZF=0 after an iteration);
///   `Some(false)` = REPNE (stop when ZF=1 after an iteration).
/// - Returns `Ok(true)` if a maskable external interrupt suspended the repeat
///   (IP already at the handler; CX/SI/DI preserved for resume).
///
/// Unsupported here: asize 64 (RCX). Per-instruction IRQ poll is in [`step`].
fn exec_string_with_rep<F>(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    zf_terminate: Option<bool>,
    mut once: F,
) -> Result<bool, ExecError>
where
    F: FnMut(&mut CpuState, &mut dyn Bus, &DecodedInsn) -> Result<(), ExecError>,
{
    let use_rep = insn.prefixes.rep || insn.prefixes.repne;
    if !use_rep {
        once(cpu, bus, insn)?;
        return Ok(false);
    }

    let use_ecx = asize32(insn);
    loop {
        if use_ecx {
            let ecx = cpu.gpr_u32(CpuState::RCX);
            if ecx == 0 {
                break;
            }
        } else {
            let cx = cpu.gpr_u16(CpuState::RCX);
            if cx == 0 {
                break;
            }
        }
        // SDM: service pending interrupts before each string iteration.
        // `#NMI` outranks maskable IRQs (Vol. 3 §6.7).
        if service_pending_nmi(cpu, bus)? {
            return Ok(true);
        }
        if service_pending_external_interrupt(cpu, bus)? {
            return Ok(true);
        }
        if use_ecx {
            let ecx = cpu.gpr_u32(CpuState::RCX);
            once(cpu, bus, insn)?;
            cpu.set_gpr_u32(CpuState::RCX, ecx.wrapping_sub(1));
        } else {
            let cx = cpu.gpr_u16(CpuState::RCX);
            once(cpu, bus, insn)?;
            cpu.set_gpr_u16(CpuState::RCX, cx.wrapping_sub(1));
        }
        // The count is decremented only once the iteration's accesses have all
        // succeeded, and each `once` advances SI/DI last, so a fault inside an
        // iteration leaves the count and the indices describing the iteration
        // to retry. Publishing that as the restart point is what stops the
        // instruction-boundary rollback from discarding finished iterations.
        bus.commit_string_iteration(cpu);
        if let Some(continue_while_zf) = zf_terminate {
            // REPE (`true`): stop when ZF=0. REPNE (`false`): stop when ZF=1.
            let zf = zf_set(cpu);
            if continue_while_zf != zf {
                break;
            }
        }
    }
    Ok(false)
}

/// Run a (possibly repeated) string op; advance IP only if not IRQ-suspended.
fn exec_string_op<F>(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    next_ip: u32,
    zf_terminate: Option<bool>,
    once: F,
) -> Result<(), ExecError>
where
    F: FnMut(&mut CpuState, &mut dyn Bus, &DecodedInsn) -> Result<(), ExecError>,
{
    if exec_string_with_rep(cpu, bus, insn, zf_terminate, once)? {
        return Ok(());
    }
    set_current_ip(cpu, next_ip);
    Ok(())
}

/// Read far pointer `m16:16` (offset then selector) for LES/LDS.
/// Spec: Intel SDM Vol. 2 LES/LDS — memory operand only (mod=11 is #UD).
fn read_far_ptr16(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
) -> Result<(u16, u16), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        // Caller should deliver #UD; keep helper defensive.
        return Err(ExecError::Unsupported(insn.opcode));
    }
    let (addr, _, uses_ss) = ea(cpu, insn, 4)?;
    let offset = bus
        .read_u16(addr)
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    let selector = bus
        .read_u16(addr.wrapping_add(2))
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    Ok((offset, selector))
}

/// Read far pointer `m16:32` (offset32 then selector16) for LES/LDS opsize-32.
/// Spec: Intel SDM Vol. 2 LES/LDS; Ch. 2 (66H).
fn read_far_ptr32(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
) -> Result<(u32, u16), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    if m.mod_ == 3 {
        return Err(ExecError::Unsupported(insn.opcode));
    }
    let (addr, _, uses_ss) = ea(cpu, insn, 6)?;
    let offset = bus
        .read_u32(addr)
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    let selector = bus
        .read_u16(addr.wrapping_add(4))
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    Ok((offset, selector))
}

/// SF/ZF/PF for shift results (SHL/SHR/SAR). AF undefined — left unchanged.
/// Spec: Intel SDM Vol. 2 SAL/SAR/SHL/SHR — Flags Affected.
fn set_shift_result_flags_u8(cpu: &mut CpuState, result: u8) {
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x80 != 0);
    cpu.set_pf(parity_even(result));
}

fn set_shift_result_flags_u16(cpu: &mut CpuState, result: u16) {
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000 != 0);
    cpu.set_pf(parity_even(result as u8));
}

fn set_shift_result_flags_u32(cpu: &mut CpuState, result: u32) {
    cpu.set_zf(result == 0);
    cpu.set_sf(result & 0x8000_0000 != 0);
    cpu.set_pf(parity_even(result as u8));
}

/// Defined result of a double-precision shift: the new destination value and
/// the last bit shifted out of the original destination (`CF`).
struct DoublePrecisionShift {
    result: u32,
    carry: bool,
}

/// Evaluate `SHLD` (`left`) or `SHRD` — Spec: Intel SDM Vol. 2 "SHLD—Double
/// Precision Shift Left" / "SHRD—Double Precision Shift Right" (Operation).
///
/// `dest` and `src` are the operand values zero-extended to 32 bits and `bits`
/// is the operand size (16 or 32). `count` must already be reduced modulo 32,
/// which is the mask the SDM applies outside 64-bit mode **regardless of the
/// operand size** — so a 16-bit operand size can legally receive a count of
/// 17–31.
///
/// Returns `None` for the two cases with no architectural result:
///
/// - `count == 0`, which the SDM specifies as "no operation": the destination is
///   unchanged and no flag is affected.
/// - `count > bits`, which the SDM calls "Bad parameters" and leaves the
///   destination *and* `CF`/`OF`/`SF`/`ZF`/`AF`/`PF` undefined. **This tree
///   commits nothing there.** That is one legal instance of the undefined
///   behavior and keeps the interpreter a deterministic reference for a future
///   JIT. It is reachable only at a 16-bit operand size, because the modulo-32
///   mask keeps a 32-bit count at or below 31.
///
/// Both cases therefore emit no destination write at all, so a memory
/// destination is read but not written back.
fn double_precision_shift(
    left: bool,
    dest: u32,
    src: u32,
    count: u32,
    bits: u32,
) -> Option<DoublePrecisionShift> {
    debug_assert!(bits == 16 || bits == 32);
    debug_assert!(count < 32);
    if count == 0 || count > bits {
        return None;
    }
    let (result, carry) = if left {
        // CF := BIT[DEST, SIZE - COUNT] — the last bit shifted out.
        let carry = (dest >> (bits - count)) & 1 != 0;
        let concat = (u64::from(dest) << bits) | u64::from(src);
        (((concat << count) >> bits) as u32, carry)
    } else {
        // CF := BIT[DEST, COUNT - 1] — the last bit shifted out.
        let carry = (dest >> (count - 1)) & 1 != 0;
        let concat = (u64::from(src) << bits) | u64::from(dest);
        ((concat >> count) as u32, carry)
    };
    let mask = if bits == 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    };
    Some(DoublePrecisionShift {
        result: result & mask,
        carry,
    })
}

/// Group 2 byte ops (D0/C0/D2). Spec: SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
/// `raw_count` is masked to 5 bits; count 0 leaves dest and flags unchanged.
fn grp2_u8(cpu: &mut CpuState, reg: u8, mut val: u8, raw_count: u8) -> Result<u8, ExecError> {
    let count = raw_count & 0x1F;
    if count == 0 {
        return Ok(val);
    }
    match reg {
        0 => {
            // ROL — tempCOUNT = COUNT mod 8; CF = LSB(result) when COUNT>0.
            let n = count % 8;
            if n != 0 {
                val = val.rotate_left(u32::from(n));
            }
            let new_cf = (val & 1) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val & 0x80) != 0) ^ new_cf);
            }
            Ok(val)
        }
        1 => {
            let n = count % 8;
            if n != 0 {
                val = val.rotate_right(u32::from(n));
            }
            let new_cf = (val & 0x80) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x80) != 0);
            }
            Ok(val)
        }
        2 => {
            // RCL — rotate through CF; tempCOUNT = COUNT mod 9.
            let n = count % 9;
            for _ in 0..n {
                let new_cf = (val & 0x80) != 0;
                val = (val << 1) | u8::from(cpu.rflags & 1 != 0);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x80) != 0) ^ cf);
            }
            Ok(val)
        }
        3 => {
            let n = count % 9;
            for _ in 0..n {
                let new_cf = (val & 1) != 0;
                val = (val >> 1) | (u8::from(cpu.rflags & 1 != 0) << 7);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x80) != 0);
            }
            Ok(val)
        }
        4 => {
            // SHL/SAL
            for _ in 0..count {
                cpu.set_cf((val & 0x80) != 0);
                val <<= 1;
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x80) != 0) ^ cf);
            }
            set_shift_result_flags_u8(cpu, val);
            Ok(val)
        }
        5 => {
            let orig = val;
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val >>= 1;
            }
            if count == 1 {
                cpu.set_of((orig & 0x80) != 0);
            }
            set_shift_result_flags_u8(cpu, val);
            Ok(val)
        }
        6 => Err(ExecError::Unsupported(0xD0)), // reserved; callers deliver #UD
        7 => {
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val = ((val as i8) >> 1) as u8;
            }
            if count == 1 {
                cpu.set_of(false);
            }
            set_shift_result_flags_u8(cpu, val);
            Ok(val)
        }
        _ => Err(ExecError::Unsupported(0xD0)),
    }
}

/// Group 2 word ops (D1/C1/D3). Spec: SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
fn grp2_u16(cpu: &mut CpuState, reg: u8, mut val: u16, raw_count: u8) -> Result<u16, ExecError> {
    let count = raw_count & 0x1F;
    if count == 0 {
        return Ok(val);
    }
    match reg {
        0 => {
            let n = count % 16;
            if n != 0 {
                val = val.rotate_left(u32::from(n));
            }
            let new_cf = (val & 1) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val & 0x8000) != 0) ^ new_cf);
            }
            Ok(val)
        }
        1 => {
            let n = count % 16;
            if n != 0 {
                val = val.rotate_right(u32::from(n));
            }
            let new_cf = (val & 0x8000) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x8000) != 0);
            }
            Ok(val)
        }
        2 => {
            let n = count % 17;
            for _ in 0..n {
                let new_cf = (val & 0x8000) != 0;
                val = (val << 1) | u16::from(cpu.rflags & 1 != 0);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x8000) != 0) ^ cf);
            }
            Ok(val)
        }
        3 => {
            let n = count % 17;
            for _ in 0..n {
                let new_cf = (val & 1) != 0;
                val = (val >> 1) | (u16::from(cpu.rflags & 1 != 0) << 15);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x8000) != 0);
            }
            Ok(val)
        }
        4 => {
            for _ in 0..count {
                cpu.set_cf((val & 0x8000) != 0);
                val <<= 1;
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x8000) != 0) ^ cf);
            }
            set_shift_result_flags_u16(cpu, val);
            Ok(val)
        }
        5 => {
            let orig = val;
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val >>= 1;
            }
            if count == 1 {
                cpu.set_of((orig & 0x8000) != 0);
            }
            set_shift_result_flags_u16(cpu, val);
            Ok(val)
        }
        6 => Err(ExecError::Unsupported(0xD1)),
        7 => {
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val = ((val as i16) >> 1) as u16;
            }
            if count == 1 {
                cpu.set_of(false);
            }
            set_shift_result_flags_u16(cpu, val);
            Ok(val)
        }
        _ => Err(ExecError::Unsupported(0xD1)),
    }
}

/// Group 2 dword ops (D1/C1/D3 under OsZ32). Spec: SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
/// COUNT masked to 5 bits; RCL/RCR use COUNT mod 33.
fn grp2_u32(cpu: &mut CpuState, reg: u8, mut val: u32, raw_count: u8) -> Result<u32, ExecError> {
    let count = raw_count & 0x1F;
    if count == 0 {
        return Ok(val);
    }
    match reg {
        0 => {
            let n = count % 32;
            if n != 0 {
                val = val.rotate_left(u32::from(n));
            }
            let new_cf = (val & 1) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val & 0x8000_0000) != 0) ^ new_cf);
            }
            Ok(val)
        }
        1 => {
            let n = count % 32;
            if n != 0 {
                val = val.rotate_right(u32::from(n));
            }
            let new_cf = (val & 0x8000_0000) != 0;
            cpu.set_cf(new_cf);
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x8000_0000) != 0);
            }
            Ok(val)
        }
        2 => {
            let n = count % 33;
            for _ in 0..n {
                let new_cf = (val & 0x8000_0000) != 0;
                val = (val << 1) | u32::from(cpu.rflags & 1 != 0);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x8000_0000) != 0) ^ cf);
            }
            Ok(val)
        }
        3 => {
            let n = count % 33;
            for _ in 0..n {
                let new_cf = (val & 1) != 0;
                val = (val >> 1) | (u32::from(cpu.rflags & 1 != 0) << 31);
                cpu.set_cf(new_cf);
            }
            if count == 1 {
                cpu.set_of(((val ^ (val << 1)) & 0x8000_0000) != 0);
            }
            Ok(val)
        }
        4 => {
            for _ in 0..count {
                cpu.set_cf((val & 0x8000_0000) != 0);
                val <<= 1;
            }
            if count == 1 {
                let cf = cpu.rflags & 1 != 0;
                cpu.set_of(((val & 0x8000_0000) != 0) ^ cf);
            }
            set_shift_result_flags_u32(cpu, val);
            Ok(val)
        }
        5 => {
            let orig = val;
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val >>= 1;
            }
            if count == 1 {
                cpu.set_of((orig & 0x8000_0000) != 0);
            }
            set_shift_result_flags_u32(cpu, val);
            Ok(val)
        }
        6 => Err(ExecError::Unsupported(0xD1)),
        7 => {
            for _ in 0..count {
                cpu.set_cf((val & 1) != 0);
                val = ((val as i32) >> 1) as u32;
            }
            if count == 1 {
                cpu.set_of(false);
            }
            set_shift_result_flags_u32(cpu, val);
            Ok(val)
        }
        _ => Err(ExecError::Unsupported(0xD1)),
    }
}

/// Highest basic `CPUID` leaf this emulator implements.
const CPUID_MAX_BASIC_LEAF: u32 = 1;

/// Highest extended `CPUID` leaf this emulator implements — the enumerator
/// itself, meaning there are no extended leaves with content.
const CPUID_MAX_EXTENDED_LEAF: u32 = 0x8000_0000;

/// Vendor identification string returned by `CPUID` leaf 0.
///
/// Deliberately neither `GenuineIntel` nor `AuthenticAMD`: software that keys
/// off a familiar vendor plus family/model would otherwise infer features this
/// emulator does not implement. `docs/cpu-profile-core2.md` asks for a
/// conservative vendor/brand string until the features exist.
const CPUID_VENDOR: [u8; 12] = *b"x86WASM Emu ";

/// `CPUID` leaf 1 version information: family 6, model 0, stepping 0.
///
/// Family 6 is the generation that introduced the two newest features reported
/// below — `PGE` and `CMOV` — so the signature and the feature bits still
/// agree. It was family 5 while `MSR` was the only bit set.
const CPUID_VERSION_INFO: u32 = 0x0000_0600;

/// `CPUID.01H:EDX[3]` — page size extensions, i.e. 4-MiB pages through
/// `CR4.PSE` (SDM Vol. 3 §4.1.4, §4.3).
const CPUID_FEATURE_PSE: u32 = 1 << 3;
/// `CPUID.01H:EDX[5]` — `RDMSR`/`WRMSR`.
const CPUID_FEATURE_MSR: u32 = 1 << 5;
/// `CPUID.01H:EDX[13]` — global pages through `CR4.PGE` (SDM Vol. 3 §4.1.4,
/// §4.10.2.4).
const CPUID_FEATURE_PGE: u32 = 1 << 13;
/// `CPUID.01H:EDX[15]` — `CMOVcc` (and `FCMOVcc` when an FPU is present, which
/// this emulator does not report).
const CPUID_FEATURE_CMOV: u32 = 1 << 15;

/// `CPUID` leaf 1 `EDX` feature bits.
///
/// Four features are implemented and therefore advertised:
///
/// * `PSE` — `CR4.PSE` and 4-MiB pages are implemented by the paging engine,
///   and §4.1.4 makes the CPUID bit the guest's licence to set `CR4.PSE`.
/// * `MSR` — `RDMSR`/`WRMSR` decode, check privilege, and raise the
///   architectural `#GP` for the MSR addresses they do not implement (except
///   the bounded `IA32_APIC_BASE` presence path).
/// * `PGE` — `CR4.PGE` and global-page TLB retention are implemented.
/// * `CMOV` — the `0F 40`–`0F 4F` conditional moves are implemented. The
///   `FCMOVcc` half of this bit's definition needs an FPU, which `FPU`
///   (`EDX[0]`) correctly reports as absent.
///
/// Deliberately still clear: `CX8` (`EDX[8]`) even though `CMPXCHG8B` executes
/// — Round 6 keeps the feature bit clear until the form is considered solid;
/// `PAE` (`EDX[6]`), `PAT` (`EDX[16]`) and `PSE-36` (`EDX[17]`) — the paging
/// engine models none of them. Everything else — `FPU`, `TSC`, `APIC`, `MTRR`,
/// `MMX`, `SSE` — stays clear because none of those are implemented.
/// Spec: Intel SDM Vol. 2 "CPUID" (Table 3-11); Vol. 3 §4.1.4; `AGENTS.md`
/// truthful-CPUID rule.
const CPUID_FEATURES_EDX: u32 =
    CPUID_FEATURE_PSE | CPUID_FEATURE_MSR | CPUID_FEATURE_PGE | CPUID_FEATURE_CMOV;

/// `CPUID` leaf 1 `ECX` feature bits: none are implemented.
const CPUID_FEATURES_ECX: u32 = 0;

/// The four registers `CPUID` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CpuidResult {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
}

/// `CPUID` output for a basic or extended leaf.
///
/// "If a value entered for CPUID.EAX is higher than the maximum input value for
/// basic or extended function for that processor, then the data for the highest
/// basic information leaf is returned" — Intel SDM Vol. 2 "CPUID". That rule
/// covers leaf `0x4000_0000`, which firmware uses to probe for a hypervisor
/// signature; this emulator is not a hypervisor and reports none.
fn cpuid_leaf(leaf: u32) -> CpuidResult {
    let basic_1 = CpuidResult {
        eax: CPUID_VERSION_INFO,
        // Brand index, CLFLUSH line size, and maximum logical processors are
        // only meaningful with feature bits this emulator does not set, and the
        // initial APIC ID of the single modeled processor is 0.
        ebx: 0,
        ecx: CPUID_FEATURES_ECX,
        edx: CPUID_FEATURES_EDX,
    };
    match leaf {
        0 => CpuidResult {
            eax: CPUID_MAX_BASIC_LEAF,
            ebx: u32::from_le_bytes([
                CPUID_VENDOR[0],
                CPUID_VENDOR[1],
                CPUID_VENDOR[2],
                CPUID_VENDOR[3],
            ]),
            edx: u32::from_le_bytes([
                CPUID_VENDOR[4],
                CPUID_VENDOR[5],
                CPUID_VENDOR[6],
                CPUID_VENDOR[7],
            ]),
            ecx: u32::from_le_bytes([
                CPUID_VENDOR[8],
                CPUID_VENDOR[9],
                CPUID_VENDOR[10],
                CPUID_VENDOR[11],
            ]),
        },
        1 => basic_1,
        CPUID_MAX_EXTENDED_LEAF => CpuidResult {
            eax: CPUID_MAX_EXTENDED_LEAF,
            ebx: 0,
            ecx: 0,
            edx: 0,
        },
        _ => basic_1,
    }
}

/// `IA32_APIC_BASE` MSR index. Spec: Intel SDM Vol. 4 MSR `1Bh`.
const MSR_IA32_APIC_BASE: u32 = 0x1B;
/// BSP flag (bit 8). Spec: SDM Vol. 3 §10.4.4.
const IA32_APIC_BASE_BSP: u64 = 1 << 8;
/// Enable x2APIC mode (bit 10 / EXTD) — unsupported; writing 1 raises `#GP(0)`.
const IA32_APIC_BASE_X2APIC: u64 = 1 << 10;
/// APIC Global Enable (bit 11 / EN). Spec: SDM Vol. 3 §10.4.4.
const IA32_APIC_BASE_ENABLE: u64 = 1 << 11;
/// Fields software may write: EN | base[35:12] (36-bit physical address model).
/// BSP is read-only and forced from the prior value; bits 0–7, 9, 10 (x2APIC),
/// and [63:36] are reserved for this tree.
const IA32_APIC_BASE_SOFTWARE_WRITABLE: u64 = IA32_APIC_BASE_ENABLE | 0x0000_000F_FFFF_F000;

/// Read a model-specific register, or `None` when the address is reserved or
/// unimplemented.
///
/// Implemented: `IA32_APIC_BASE` (`0x1B`) only. Every other address takes the
/// architectural `#GP` path rather than returning a fabricated zero.
/// Spec: Intel SDM Vol. 2 "RDMSR"; Vol. 4 (MSR listings).
fn read_msr(cpu: &CpuState, index: u32) -> Option<u64> {
    match index {
        MSR_IA32_APIC_BASE => Some(cpu.ia32_apic_base),
        _ => None,
    }
}

/// Write a model-specific register, returning `false` when the address is
/// reserved/unimplemented or the value sets a reserved bit (`#GP`).
///
/// Spec: Intel SDM Vol. 2 "WRMSR"; Vol. 3 §10.4.4 / Vol. 4 IA32_APIC_BASE.
fn write_msr(cpu: &mut CpuState, index: u32, value: u64) -> bool {
    match index {
        MSR_IA32_APIC_BASE => {
            // x2APIC EXTD (bit 10) and any other non-software-writable bit → #GP(0).
            // BSP (bit 8) is read-only: a write that changes it is also `#GP`.
            let prior_bsp = cpu.ia32_apic_base & IA32_APIC_BASE_BSP;
            if value & IA32_APIC_BASE_X2APIC != 0 {
                return false;
            }
            if value & !(IA32_APIC_BASE_SOFTWARE_WRITABLE | IA32_APIC_BASE_BSP) != 0 {
                return false;
            }
            if value & IA32_APIC_BASE_BSP != prior_bsp {
                return false;
            }
            cpu.ia32_apic_base = (value & IA32_APIC_BASE_SOFTWARE_WRITABLE) | prior_bsp;
            true
        }
        _ => false,
    }
}

/// `#GP(0)` unless the processor is at CPL 0.
///
/// Real-address mode always runs at CPL 0. Virtual-8086 mode forces CPL 3, so
/// it faults even when CS[1:0] looks like RPL 0. Spec: Intel SDM Vol. 2
/// "INVD"/"WBINVD"/"RDMSR"/"WRMSR" (Protected Mode Exceptions); Vol. 3 §5.5.
fn require_cpl0(cpu: &CpuState) -> Result<(), ExecError> {
    if architectural_cpl(cpu) != 0 {
        return Err(arch_fault_with_error_code(13, 0));
    }
    Ok(())
}

/// `CR4.VME` — Virtual-8086 Mode Extensions enable, bit 0 (SDM Vol. 3 §2.5).
///
/// Writable for the Round-12 soft-int redirect-bitmap stub. **`CPUID.01H:EDX.VME`
/// stays clear** until VIF/VIP and the rest of Table 20-2 ship (`AGENTS.md`
/// truthful-CPUID). Architecturally §4.1.4 would couple the CR4 bit to CPUID;
/// we deliberately allow the sticky CR4 bit without advertising the feature.
const CR4_VME: u64 = 1 << 0;

/// The `CR4` bits a guest may set.
///
/// SDM Vol. 3 §4.1.4 makes each paging feature's `CR4` bit conditional on its
/// `CPUID` bit ("`CR4.PSE` … can be set only if `CPUID.01H:EDX.PSE [bit 3]` is
/// 1"), and Vol. 2 "MOV—Move to/from Control Registers" raises `#GP(0)` on a
/// write of 1 to a reserved `CR4` bit. `CR4.VME` is an honesty exception: it is
/// writable for the bounded VME redirect stub while `CPUID.VME` remains clear.
/// Every other unimplemented bit — `PVI`, `TSD`, `DE`, `PAE`, `MCE`, `PCE`,
/// `OSFXSR`, `OSXMMEXCPT`, `UMIP`, `SMEP`, `SMAP`, `PKE`, and the rest — is
/// reserved and refused. In particular `CR4.PAE` is refused, which keeps a
/// guest from selecting the PAE paging mode the engine reports as unsupported.
const fn cr4_reserved_mask() -> u64 {
    let mut implemented = CR4_VME;
    if CPUID_FEATURES_EDX & CPUID_FEATURE_PSE != 0 {
        implemented |= x86_mmu::paging::CR4_PSE;
    }
    if CPUID_FEATURES_EDX & CPUID_FEATURE_PGE != 0 {
        implemented |= x86_mmu::paging::CR4_PGE;
    }
    !implemented
}

/// `CR0.PG`, bit 31 (SDM Vol. 3 §4.1.1).
const CR0_PG: u64 = x86_mmu::paging::CR0_PG;

/// Commit a control-register write and tell the memory path about it.
///
/// Every `MOV to CRn`, `LMSW` and `CLTS` routes through here so the TLB
/// invalidation hooks (SDM §4.10.4.1) cannot be forgotten on one path.
fn note_control_register_write(cpu: &CpuState, bus: &mut dyn Bus, reg: u8) {
    bus.on_mov_to_control_register(reg, cpu.cr0, cpu.cr3, cpu.cr4);
}

/// What `BT`/`BTS`/`BTR`/`BTC` do to the selected bit after copying it to CF.
/// Spec: Intel SDM Vol. 2 "BT"/"BTS"/"BTR"/"BTC".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BitOp {
    Test,
    Set,
    Reset,
    Complement,
}

impl BitOp {
    /// New value for the selected bit, or `None` when the bit is only tested.
    fn apply(self, old: bool) -> Option<bool> {
        match self {
            Self::Test => None,
            Self::Set => Some(true),
            Self::Reset => Some(false),
            Self::Complement => Some(!old),
        }
    }
}

/// `BT`/`BTS`/`BTR`/`BTC` for both the register and the immediate bit-offset
/// encodings.
///
/// A register bit base takes `BitOffset MOD OperandSize`. A memory bit base is
/// the start of a bit string: the addressed bit is `BitOffset MOD 8` inside the
/// byte at `BitBase + (BitOffset DIV 8)`, where `DIV` is signed division
/// rounding toward negative infinity and `MOD` returns a non-negative value, so
/// a register bit offset reaches far outside the nominal operand.
/// Spec: Intel SDM Vol. 2 "BT" (Operation) and §3.1.1.9 (`Bit(BitBase,
/// BitOffset)` notation); Vol. 3 §5.3 (limit checking).
///
/// `CF` receives the original bit. `OF`, `SF`, `ZF`, `AF`, and `PF` are
/// architecturally undefined; this interpreter leaves them unchanged so the
/// reference semantics stay deterministic.
fn exec_bit_op(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    op: BitOp,
    bit_offset: i32,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
    let operand_bits: i32 = if opsz32(insn) { 32 } else { 16 };

    if m.mod_ == 3 {
        let index = bit_offset.rem_euclid(operand_bits) as u32;
        let old_bit = if opsz32(insn) {
            let old = cpu.gpr_u32(m.rm as usize);
            let bit = (old >> index) & 1 != 0;
            if let Some(new_bit) = op.apply(bit) {
                let mask = 1u32 << index;
                cpu.set_gpr_u32(
                    m.rm as usize,
                    if new_bit { old | mask } else { old & !mask },
                );
            }
            bit
        } else {
            let old = cpu.gpr_u16(m.rm as usize);
            let bit = (old >> index) & 1 != 0;
            if let Some(new_bit) = op.apply(bit) {
                let mask = 1u16 << index;
                cpu.set_gpr_u16(
                    m.rm as usize,
                    if new_bit { old | mask } else { old & !mask },
                );
            }
            bit
        };
        cpu.set_cf(old_bit);
        return Ok(());
    }

    let byte_displacement = bit_offset.div_euclid(8);
    let index = bit_offset.rem_euclid(8) as u32;
    let (addr, uses_ss) = {
        let (seg, uses_ss, base_offset) = ea_parts(cpu, insn)?;
        // The bit-string byte address wraps inside the address-size window.
        let offset = if asize32(insn) {
            u64::from((base_offset as u32).wrapping_add(byte_displacement as u32))
        } else {
            u64::from((base_offset as u16).wrapping_add(byte_displacement as u16))
        };
        (seg_linear_checked(seg, offset, 1, uses_ss)?, uses_ss)
    };

    let old = bus
        .read_u8(addr)
        .map_err(|e| classify_mem_fault(e, uses_ss))?;
    let old_bit = (old >> index) & 1 != 0;
    if let Some(new_bit) = op.apply(old_bit) {
        let mask = 1u8 << index;
        let new = if new_bit { old | mask } else { old & !mask };
        bus.write_u8(addr, new)
            .map_err(|e| classify_mem_fault(e, uses_ss))?;
    }
    // CF commits only after the read-modify-write cannot fault.
    cpu.set_cf(old_bit);
    Ok(())
}

/// Evaluate an x86 condition code against the current `EFLAGS`.
///
/// `cc` is the low nibble shared by the short `Jcc` (`70`+cc), the near `Jcc`
/// (`0F 80`+cc), and `SETcc` (`0F 90`+cc) encodings, so all three forms select
/// the condition through this one helper.
/// Spec: Intel SDM Vol. 2 "Jcc"/"SETcc"; Appendix B (condition-code encodings).
fn condition_code(cpu: &CpuState, cc: u8) -> bool {
    let cf = cpu.rflags & 1 != 0;
    let pf = cpu.rflags & (1 << 2) != 0;
    let zf = cpu.rflags & (1 << 6) != 0;
    let sf = cpu.rflags & (1 << 7) != 0;
    let of = cpu.rflags & (1 << 11) != 0;
    match cc & 0x0F {
        0x0 => of,               // O
        0x1 => !of,              // NO
        0x2 => cf,               // B / C / NAE
        0x3 => !cf,              // AE / NB / NC
        0x4 => zf,               // E / Z
        0x5 => !zf,              // NE / NZ
        0x6 => cf || zf,         // BE / NA
        0x7 => !cf && !zf,       // A / NBE
        0x8 => sf,               // S
        0x9 => !sf,              // NS
        0xA => pf,               // P / PE
        0xB => !pf,              // NP / PO
        0xC => sf != of,         // L / NGE
        0xD => sf == of,         // GE / NL
        0xE => zf || (sf != of), // LE / NG
        _ => !zf && (sf == of),  // G / NLE
    }
}

/// Short Jcc condition for opcodes 0x70–0x7F (Intel SDM Vol. 2, Jcc).
fn jcc_condition(cpu: &CpuState, opcode: u8) -> bool {
    condition_code(cpu, opcode)
}

/// Primary opcodes that are architectural `#UD` in real-address mode when the
/// sparse decoder table has no entry (or rejects the opcode).
///
/// **Rule (sparse tables):** do **not** treat every `UnsupportedOpcode` as `#UD`.
/// Only opcodes the SDM classifies as invalid/unrecognized in real mode vector
/// through the IVT. Valid-but-unimplemented primaries (x87 `D8`–`DF`, `WAIT`/`9B`,
/// two-byte escape `0F` secondaries with no entry, Grp1 alias `82`, …) remain
/// host `Decode(UnsupportedOpcode)`.
///
/// Note: `D6` and `F1` are reserved/undefined but do **not** generate `#UD`
/// (Intel SDM Vol. 3 §6.15 — Invalid Opcode Exception).
///
/// Spec: Intel SDM Vol. 3 §6.15 (#UD); Vol. 2 ARPL (Real-Address Mode Exceptions).
fn real_mode_primary_opcode_is_ud(opcode: u8) -> bool {
    matches!(opcode, 0x63) // ARPL — not recognized in real-address mode
}

fn fetch_decode(cpu: &CpuState, bus: &mut dyn Bus) -> Result<x86_decode::DecodedInsn, ExecError> {
    // Grow the window until decode succeeds or we hit the 15-byte SDM limit.
    let mut buf = Vec::with_capacity(15);
    // Decode defaults follow the cached CS.D bit (Vol. 3 §3.4.5); `0x66`/`0x67`
    // invert them.
    let mode = x86_decode::DecodeMode::from_cs_default_big(cpu.cs.default_big());
    let default_big = cpu.cs.default_big();
    loop {
        if buf.len() >= 15 {
            return Err(ExecError::Decode(DecodeError::TooLong));
        }
        // `D=0` fetch wraps in the 16-bit IP window; `D=1` uses full EIP.
        // Enforce the cached CS.limit either way.
        // Spec: Intel SDM Vol. 3 §5.3; §6.15 (#GP). Bus MemoryFault → #GP (CS).
        let base = current_ip(cpu);
        let ip = if default_big {
            u64::from(base.wrapping_add(buf.len() as u32))
        } else {
            u64::from((base as u16).wrapping_add(buf.len() as u16))
        };
        let addr = seg_linear_checked(&cpu.cs, ip, 1, false)?;
        // `fetch_u8` so paging sees `AccessKind::InstructionFetch` (SDM §4.6.1).
        // Each byte is translated on its own, so an instruction that straddles
        // a page boundary faults on the byte that is actually unreachable.
        buf.push(
            bus.fetch_u8(addr)
                .map_err(|e| classify_mem_fault(e, false))?,
        );
        match decode_with_mode(&buf, mode) {
            Ok(insn) => return Ok(insn),
            Err(DecodeError::Truncated) => continue,
            Err(DecodeError::UnsupportedOpcode(op)) if real_mode_primary_opcode_is_ud(op) => {
                return Err(arch_fault(6));
            }
            Err(e) => return Err(ExecError::Decode(e)),
        }
    }
}

/// Real-mode software interrupt delivery through the IVT at `IDTR.base`.
///
/// Uses unchecked stack pushes so a delivery-time bus fault stays `MemoryFault`
/// (not re-classified as a nested `#SS` ArchFault). Spec: Intel SDM Vol. 2
/// "INT n/INTO/INT3/INT1"; Vol. 3 §6.4.
fn real_mode_software_interrupt(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    vector: u8,
    return_ip: u32,
) -> Result<(), ExecError> {
    // Real-address mode always executes with a 16-bit IP window.
    debug_assert!(!cpu.cs.default_big(), "real-mode IVT delivery needs CS.D=0");
    let flags16 = cpu.rflags as u16;
    push16_unchecked(cpu, bus, flags16)?;
    push16_unchecked(cpu, bus, cpu.cs.selector)?;
    push16_unchecked(cpu, bus, return_ip as u16)?;
    // Clear IF and TF (Vol. 2 INT n Operation, real-address mode).
    cpu.rflags &= !((1 << 9) | (1 << 8));
    let entry = cpu.idtr.base.wrapping_add(u64::from(vector) * 4);
    let offset = bus.read_u16(entry)?;
    let selector = bus.read_u16(entry.wrapping_add(2))?;
    cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
    cpu.set_ip16(offset);
    Ok(())
}

/// Return a pending exception fault for top-level real-mode IVT delivery.
///
/// Saved IP is the faulting instruction address (instruction start).
/// Spec: Intel SDM Vol. 3 §6.4 (real-address mode), §6.15 (exception reference).
/// Note: #OF from INTO is a trap (use [`real_mode_software_interrupt`] with next IP).
fn real_mode_exception(
    _cpu: &mut CpuState,
    _bus: &mut dyn Bus,
    vector: u8,
) -> Result<(), ExecError> {
    Err(arch_fault(vector))
}

/// Deliver a fault through the current real-mode IVT path.
///
/// Any protected-mode error-code payload is deliberately not pushed: the
/// existing real-mode frame remains FLAGS, CS, and faulting IP.
/// Spec: Intel SDM Vol. 3 §§6.13, 6.15.
fn deliver_real_mode_exception(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    vector: u8,
) -> Result<(), ExecError> {
    let return_ip = current_ip(cpu);
    real_mode_software_interrupt(cpu, bus, vector, return_ip)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProtectedGateSource {
    Software,
    Hardware,
    /// A fault or trap, with the error code its gate pushes. The code is a
    /// doubleword because `#PF` uses one (SDM Vol. 3 §4.7); the selector-style
    /// codes of §6.13 occupy only its low word.
    Exception(Option<u32>),
}

impl ProtectedGateSource {
    fn error_code(self) -> Option<u32> {
        match self {
            Self::Exception(error_code) => error_code,
            Self::Software | Self::Hardware => None,
        }
    }
}

/// Deliver one interrupt or architectural fault through a 286 (16-bit) or 386
/// (32-bit) protected-mode interrupt or trap gate.
///
/// Gate types `0x6`/`0x7` build a 16-bit `FLAGS`/`CS`/`IP` frame and require a
/// `D=0` current and target code segment. Gate types `0xE`/`0xF` build a
/// 32-bit `EFLAGS`/`CS`/`EIP` frame (with a doubleword error code where
/// applicable), take the entry `EIP` from the gate offset high and low words,
/// and accept a `D=0` or `D=1` target. The frame element width comes from the
/// gate type while the stack-pointer width comes from the destination `SS.B`.
///
/// When the target code segment's DPL is less than CPL, delivery performs a
/// privilege-changing stack switch: `SSn:ESPn` are read from the current
/// 32-bit TSS, the new SS is validated at the inner CPL, and the outer
/// `SS:ESP` are pushed ahead of the ordinary frame (Vol. 3 §6.12.1 Figure 6-5).
/// Inner-stack accesses use supervisor mode (§4.6.1). Same-CPL delivery is
/// unchanged.
///
/// Virtual-8086 mode (`EFLAGS.VM=1`): architectural CPL is 3 (CS[1:0] is not
/// RPL). A privilege-changing 386 gate pushes the **9-dword** VM86 frame
/// GS/FS/DS/ES + SS:ESP + EFLAGS(with VM) + CS:EIP (Vol. 3 §20.2 Figure 20-2),
/// then loads DS/ES/FS/GS with null selectors. 16-bit gates and same-CPL
/// delivery from VM86 remain unsupported. Task gates, VME/PVI, and nested
/// #DF synthesis beyond the existing path remain out of scope.
///
/// Gate DPL is checked only for software INT/INT3/INTO. A violation raises
/// #GP with IDT=1 and EXT=0; faults, NMI, and external IRQs bypass gate DPL.
/// Spec: Intel SDM Vol. 2 INT n/INT3/INTO; Vol. 3
/// §§6.10, 6.11.2, 6.12.1, 6.12.3, 6.13, 20.2–20.3.
fn deliver_protected_mode_gate(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    vector: u8,
    return_ip: u32,
    source: ProtectedGateSource,
) -> Result<(), ExecError> {
    let gate_offset = u64::from(vector) * 8;
    if gate_offset + 7 > u64::from(cpu.idtr.limit) {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::IdtLimit,
        ));
    }

    let gate_addr = cpu.idtr.base.wrapping_add(gate_offset);
    let mut gate = [0u8; 8];
    for (index, byte) in gate.iter_mut().enumerate() {
        let addr = gate_addr.wrapping_add(index as u64);
        *byte = bus.read_system_u8(addr).map_err(|_| {
            protected_mode_delivery_error(vector, ProtectedModeDeliveryError::IdtRead(addr))
        })?;
    }

    let gate_access = gate[5];
    // Type field: 0x6/0x7 are 286 gates, 0xE/0xF are 386 gates. The low bit of
    // the pair selects trap (odd) vs interrupt (even).
    // Spec: Intel SDM Vol. 3 §6.11 (Table 3-2 system-descriptor types).
    let (gate32, interrupt_gate) = match gate_access & 0x1F {
        0x06 => (false, true),
        0x07 => (false, false),
        0x0E => (true, true),
        0x0F => (true, false),
        _ => {
            return Err(protected_mode_delivery_error(
                vector,
                ProtectedModeDeliveryError::GateType(gate_access),
            ));
        }
    };
    if gate_access & 0x80 == 0 {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::GateNotPresent,
        ));
    }

    // VM86 forces CPL=3; CS[1:0] is not RPL (Vol. 3 §§5.5, 20.1.1).
    let from_vm86 = eflags_vm(cpu.rflags);
    let cpl = architectural_cpl(cpu);
    if from_vm86 && !gate32 {
        // VM86 extended frame is dword-width (Figure 20-2); 286 gates unsupported.
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::GateType(gate_access),
        ));
    }
    if cpu.cs.flags & x86_core::SegmentReg::FLAG_LONG != 0 {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::CurrentPrivilege,
        ));
    }
    if !gate32 && cpu.cs.default_big() {
        // A 16-bit frame cannot carry a 32-bit return EIP; report instead of
        // silently truncating it.
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::CurrentPrivilege,
        ));
    }
    let gate_dpl = (gate_access >> 5) & 3;
    if source == ProtectedGateSource::Software && cpl > gate_dpl {
        // IDT selector error code: vector index in bits 15:3, IDT=1, EXT=0.
        // The failed software transfer has not touched the stack.
        return Err(arch_fault_with_error_code(13, (u16::from(vector) << 3) | 2));
    }

    let target_selector = u16::from_le_bytes([gate[2], gate[3]]);
    if is_null_selector(target_selector) {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::NullTargetSelector,
        ));
    }
    if target_selector & 0x4 != 0 {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::LdtTargetSelector,
        ));
    }

    let descriptor_offset = u64::from(target_selector >> 3) * 8;
    if descriptor_offset + 7 > u64::from(cpu.gdtr.limit) {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::GdtLimit,
        ));
    }
    let descriptor_addr = cpu.gdtr.base.wrapping_add(descriptor_offset);
    let mut descriptor = [0u8; 8];
    for (index, byte) in descriptor.iter_mut().enumerate() {
        let addr = descriptor_addr.wrapping_add(index as u64);
        *byte = bus.read_system_u8(addr).map_err(|_| {
            protected_mode_delivery_error(vector, ProtectedModeDeliveryError::GdtRead(addr))
        })?;
    }

    let target_access = descriptor[5];
    if target_access & 0x80 == 0 {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::TargetNotPresent,
        ));
    }
    let system = target_access & 0x10 == 0;
    let executable = target_access & 0x08 != 0;
    let conforming = executable && target_access & 0x04 != 0;
    let dpl = (target_access >> 5) & 3;
    // Same-CPL: DPL == CPL. Privilege-changing: nonconforming DPL < CPL.
    // Spec: Intel SDM Vol. 3 §6.12.1.
    let privilege_change = dpl < cpl;
    if system || !executable || dpl > cpl || (privilege_change && conforming) {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::TargetCode,
        ));
    }
    // Same-CPL delivery from VM86 (handler DPL=3) is not modeled; monitors use
    // ring-0 gates. Spec honesty: Vol. 3 §20.2 privilege-changing frame.
    if from_vm86 && !privilege_change {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::CurrentPrivilege,
        ));
    }
    let new_cpl = dpl;

    let parsed_target = parse_segment_descriptor(descriptor);
    if parsed_target.flags & x86_core::SegmentReg::FLAG_LONG != 0 {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::TargetLongMode,
        ));
    }
    if !gate32 && parsed_target.flags & x86_core::SegmentReg::FLAG_DEFAULT_BIG != 0 {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::TargetNot16Bit,
        ));
    }
    // 386 gates hold the entry offset in bytes 1:0 and 7:6; 286 gates keep
    // bytes 7:6 reserved. Spec: Intel SDM Vol. 3 §6.11 (Figure 6-2).
    let target_offset = if gate32 {
        u32::from(u16::from_le_bytes([gate[0], gate[1]]))
            | (u32::from(u16::from_le_bytes([gate[6], gate[7]])) << 16)
    } else {
        u32::from(u16::from_le_bytes([gate[0], gate[1]]))
    };
    if target_offset > parsed_target.limit {
        return Err(protected_mode_delivery_error(
            vector,
            ProtectedModeDeliveryError::TargetOffsetLimit,
        ));
    }

    // Privilege-changing delivery loads SS:ESP from the TSS, then pushes the
    // outer SS:ESP before the ordinary FLAGS/CS/IP[/error] frame (Figure 6-5).
    // From VM86, also push GS/FS/DS/ES ahead of SS:ESP (Figure 20-2).
    // Same-CPL keeps the current stack. Spec: Intel SDM Vol. 3 §6.12.1 / §20.2.
    let error_code = source.error_code();
    let entry_size: usize = if gate32 { 4 } else { 2 };
    let saved_flags = if gate32 {
        cpu.rflags as u32
    } else {
        u32::from(cpu.rflags as u16)
    };
    let old_ss = cpu.ss.selector;
    let old_sp = stack_pointer(cpu);
    let old_es = cpu.es.selector;
    let old_ds = cpu.ds.selector;
    let old_fs = cpu.fs.selector;
    let old_gs = cpu.gs.selector;

    let (stack_seg, mut final_sp, system_stack_access) = if privilege_change {
        let (ss_sel, esp) = read_tss32_inner_stack(cpu, bus, new_cpl, vector)?;
        let loaded =
            prepare_ss_from_gdt_for_cpl(cpu, bus, ss_sel, new_cpl).map_err(|err| match err {
                ExecError::ArchFault { .. } | ExecError::MemoryFault(_) => {
                    protected_mode_delivery_error(
                        vector,
                        ProtectedModeDeliveryError::InnerStackSelector,
                    )
                }
                other => other,
            })?;
        (loaded, esp, true)
    } else {
        (cpu.ss.clone(), old_sp, false)
    };

    let mut frame_entries = Vec::with_capacity(if privilege_change {
        let base = if from_vm86 { 9 } else { 5 };
        if error_code.is_some() {
            base + 1
        } else {
            base
        }
    } else if error_code.is_some() {
        4
    } else {
        3
    });
    if privilege_change {
        // Push order (first = highest address): GS, FS, DS, ES, SS, ESP, then
        // EFLAGS/CS/EIP below. Spec: Vol. 3 Figure 20-2 / Figure 6-5.
        if from_vm86 {
            frame_entries.push(u32::from(old_gs));
            frame_entries.push(u32::from(old_fs));
            frame_entries.push(u32::from(old_ds));
            frame_entries.push(u32::from(old_es));
        }
        frame_entries.push(u32::from(old_ss));
        frame_entries.push(old_sp);
    }
    // The `CS.D=1` rejection above guarantees a 16-bit gate's return EIP fits.
    frame_entries.extend([saved_flags, u32::from(cpu.cs.selector), return_ip]);
    if let Some(code) = error_code {
        frame_entries.push(code);
    }

    let stack_b32 = stack_seg.default_big();
    let mut desired_bytes = Vec::with_capacity(frame_entries.len() * entry_size);
    for entry in frame_entries {
        final_sp = stack_step_width(stack_b32, final_sp, -(entry_size as i32));
        let addr = checked_linear_addr(&stack_seg, u64::from(final_sp), entry_size as u64)
            .map_err(|_| {
                protected_mode_delivery_error(vector, ProtectedModeDeliveryError::StackLimit)
            })?;
        let bytes = entry.to_le_bytes();
        for (index, byte) in bytes.iter().take(entry_size).enumerate() {
            desired_bytes.push((addr.wrapping_add(index as u64), *byte));
        }
    }

    let mut planned_writes = Vec::with_capacity(desired_bytes.len());
    for (addr, value) in desired_bytes {
        let original = if system_stack_access {
            bus.read_system_u8(addr)
        } else {
            bus.read_u8(addr)
        }
        .map_err(|_| {
            protected_mode_delivery_error(vector, ProtectedModeDeliveryError::StackRead(addr))
        })?;
        planned_writes.push((addr, original, value));
    }

    for index in 0..planned_writes.len() {
        let (addr, _, value) = planned_writes[index];
        let write_ok = if system_stack_access {
            bus.write_system_u8(addr, value)
        } else {
            bus.write_u8(addr, value)
        };
        if write_ok.is_err() {
            let mut rollback_failure = None;
            for &(restore_addr, original, _) in planned_writes[..=index].iter().rev() {
                let restore_ok = if system_stack_access {
                    bus.write_system_u8(restore_addr, original)
                } else {
                    bus.write_u8(restore_addr, original)
                };
                if restore_ok.is_err() && rollback_failure.is_none() {
                    rollback_failure = Some(restore_addr);
                }
            }
            let reason = rollback_failure.map_or(
                ProtectedModeDeliveryError::StackWrite(addr),
                ProtectedModeDeliveryError::StackRollback,
            );
            return Err(protected_mode_delivery_error(vector, reason));
        }
    }

    if privilege_change {
        cpu.ss = stack_seg;
    }
    if stack_b32 {
        cpu.set_gpr_u32(CpuState::RSP, final_sp);
    } else {
        cpu.set_gpr_u16(CpuState::RSP, final_sp as u16);
    }
    cpu.cs.load_descriptor_cache(
        (target_selector & !3) | u16::from(new_cpl),
        parsed_target.base,
        parsed_target.limit,
        parsed_target.flags,
    );
    cpu.rip = u64::from(target_offset);

    // Both gate types clear TF, NT, RF, and VM. Only interrupt gates clear IF;
    // trap gates preserve it. The saved FLAGS word above contains pre-entry IF.
    cpu.rflags &= !((1 << 8) | (1 << 14) | (1 << 16) | (1 << 17));
    if interrupt_gate {
        cpu.set_interrupt_flag(false);
    }
    // VM86 → protected: nullify data segments (Vol. 3 §20.2).
    if from_vm86 {
        cpu.es.load_null_selector(0);
        cpu.ds.load_null_selector(0);
        cpu.fs.load_null_selector(0);
        cpu.gs.load_null_selector(0);
    }
    Ok(())
}

fn deliver_software_interrupt(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    vector: u8,
    return_ip: u32,
) -> Result<(), ExecError> {
    if cr0_pe(cpu) {
        deliver_protected_mode_gate(cpu, bus, vector, return_ip, ProtectedGateSource::Software)
    } else {
        real_mode_software_interrupt(cpu, bus, vector, return_ip)
    }
}

fn deliver_hardware_interrupt(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    vector: u8,
    return_ip: u32,
) -> Result<(), ExecError> {
    if cr0_pe(cpu) {
        deliver_protected_mode_gate(cpu, bus, vector, return_ip, ProtectedGateSource::Hardware)
    } else {
        real_mode_software_interrupt(cpu, bus, vector, return_ip)
    }
}

/// `#PF` — Page-Fault Exception. Spec: Intel SDM Vol. 3 §4.7, §6.15.
const VECTOR_PAGE_FAULT: u8 = 14;
/// `#DF` — Double-Fault Exception. Spec: Intel SDM Vol. 3 §6.15.
const VECTOR_DOUBLE_FAULT: u8 = 8;

/// Deliver a pending fault through whichever path the current mode selects.
///
/// A `#PF` can only arise with `CR0.PG = 1`, which requires `CR0.PE = 1`, so
/// the real-mode branch is never reached for vector 14.
///
/// If protected-mode delivery of an exception fails, the failure escalates to
/// `#DF` (vector 8, error code 0). A failure while delivering `#DF` is reported
/// as [`ExecError::TripleFault`] rather than continuing.
/// Spec: Intel SDM Vol. 3 §6.4 (real-address mode), §6.12.1 (gates), §6.15
/// (#DF / triple fault), §4.1.1.
fn deliver_fault(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    vector: u8,
    error_code: Option<u32>,
) -> Result<(), ExecError> {
    if cr0_pe(cpu) {
        let return_ip = current_ip(cpu);
        match deliver_protected_mode_gate(
            cpu,
            bus,
            vector,
            return_ip,
            ProtectedGateSource::Exception(error_code),
        ) {
            Ok(()) => Ok(()),
            Err(ExecError::ProtectedModeExceptionDelivery { reason, .. })
                if vector == VECTOR_DOUBLE_FAULT =>
            {
                Err(ExecError::TripleFault { reason })
            }
            Err(ExecError::ProtectedModeExceptionDelivery { .. }) => {
                // Escalate: prior delivery left no architectural commits.
                match deliver_protected_mode_gate(
                    cpu,
                    bus,
                    VECTOR_DOUBLE_FAULT,
                    return_ip,
                    ProtectedGateSource::Exception(Some(0)),
                ) {
                    Ok(()) => Ok(()),
                    Err(ExecError::ProtectedModeExceptionDelivery { reason, .. }) => {
                        Err(ExecError::TripleFault { reason })
                    }
                    Err(other) => Err(other),
                }
            }
            Err(other) => Err(other),
        }
    } else {
        deliver_real_mode_exception(cpu, bus, vector)
    }
}

/// #UD — Invalid Opcode Exception (vector 6).
/// Spec: Intel SDM Vol. 3 §6.15 (#UD).
fn real_mode_ud(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<(), ExecError> {
    real_mode_exception(cpu, bus, 6)
}

/// Minimum inclusive limit of a 32-bit TSS (SDM Vol. 3 §7.2.1).
const TSS32_MIN_LIMIT: u32 = 0x67;
/// System-descriptor type: available 32-bit TSS (SDM Vol. 3 Table 3-2).
const DESC_TYPE_TSS32_AVAILABLE: u8 = 0x9;
/// System-descriptor type: busy 32-bit TSS (SDM Vol. 3 Table 3-2).
const DESC_TYPE_TSS32_BUSY: u8 = 0xB;
/// System-descriptor type: task gate (SDM Vol. 3 Table 3-2).
const DESC_TYPE_TASK_GATE: u8 = 0x5;
/// System-descriptor type: LDT (SDM Vol. 3 Table 3-2).
const DESC_TYPE_LDT: u8 = 0x2;

/// 32-bit TSS field offsets (SDM Vol. 3 §7.2.1 Figure 7-2).
const TSS32_OFF_LINK: u32 = 0;
const TSS32_OFF_CR3: u32 = 28;
const TSS32_OFF_EIP: u32 = 32;
const TSS32_OFF_EFLAGS: u32 = 36;
const TSS32_OFF_EAX: u32 = 40;
const TSS32_OFF_ECX: u32 = 44;
const TSS32_OFF_EDX: u32 = 48;
const TSS32_OFF_EBX: u32 = 52;
const TSS32_OFF_ESP: u32 = 56;
const TSS32_OFF_EBP: u32 = 60;
const TSS32_OFF_ESI: u32 = 64;
const TSS32_OFF_EDI: u32 = 68;
const TSS32_OFF_ES: u32 = 72;
const TSS32_OFF_CS: u32 = 76;
const TSS32_OFF_SS: u32 = 80;
const TSS32_OFF_DS: u32 = 84;
const TSS32_OFF_FS: u32 = 88;
const TSS32_OFF_GS: u32 = 92;
const TSS32_OFF_LDTR: u32 = 96;

/// How a hardware task switch was requested (SDM Vol. 3 §7.3 / Table 7-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskSwitchCause {
    /// Far `JMP` — clear outgoing busy, clear `NT`.
    Jmp,
    /// Far `CALL` — keep outgoing busy, set `NT`, write previous-task link.
    Call,
    /// `IRET` with `NT=1` — clear outgoing busy, clear `NT` (link from current TSS).
    Iret,
}

/// `LTR r/m16` — load TR from a present available 32-bit TSS descriptor.
///
/// Validates CPL, null/TI, type (`0x9`), present, and the §7.2.1 minimum limit,
/// then marks the GDT descriptor busy (`0xB`) and caches base/limit/AR in TR.
/// No task switch is performed.
/// Spec: Intel SDM Vol. 2 "LTR"; Vol. 3 §§7.2–7.3.
/// Unsupported here: 16-bit TSS (`type=1`), LDT-resident descriptors, hardware
/// task switches.
fn exec_ltr(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if !cr0_pe(cpu) {
        // Real-address / virtual-8086 mode: invalid opcode.
        return Err(arch_fault(6));
    }
    require_cpl0(cpu)?;
    let selector = read_rm_u16(cpu, bus, insn)?;
    if is_null_selector(selector) {
        return Err(selector_fault(13, selector));
    }
    if selector & 0x4 != 0 {
        return Err(selector_fault(13, selector));
    }

    let offset = u64::from(selector >> 3) * 8;
    if offset + 7 > u64::from(cpu.gdtr.limit) {
        return Err(selector_fault(13, selector));
    }
    let addr = cpu.gdtr.base.wrapping_add(offset);
    let mut descriptor = [0u8; 8];
    for (index, byte) in descriptor.iter_mut().enumerate() {
        *byte = bus
            .read_system_u8(addr.wrapping_add(index as u64))
            .map_err(|error| classify_mem_fault(error, false))?;
    }

    let access = descriptor[5];
    let system = access & 0x10 == 0;
    let type_field = access & 0x0F;
    if !system || type_field != DESC_TYPE_TSS32_AVAILABLE {
        return Err(selector_fault(13, selector));
    }
    if access & 0x80 == 0 {
        return Err(selector_fault(11, selector));
    }

    let parsed = parse_segment_descriptor(descriptor);
    if parsed.limit < TSS32_MIN_LIMIT {
        return Err(selector_fault(13, selector));
    }

    // Mark busy in the GDT before committing TR (SDM Vol. 3 §7.2.2 / §7.3).
    let busy_access = (access & 0xF0) | DESC_TYPE_TSS32_BUSY;
    bus.write_system_u8(addr.wrapping_add(5), busy_access)
        .map_err(|error| classify_mem_fault(error, false))?;

    cpu.tr
        .load_descriptor_cache(selector, parsed.base, parsed.limit, {
            let mut flags = parsed.flags;
            flags = (flags & !0x0F) | u16::from(DESC_TYPE_TSS32_BUSY);
            flags
        });
    Ok(())
}

/// `LLDT r/m16` — load LDTR from a present LDT system descriptor in the GDT.
///
/// Null clears the LDTR cache. Spec: Intel SDM Vol. 2 "LLDT"; Vol. 3 §§2.4.2,
/// 3.5.1. Unsupported here: LDT-resident LDT descriptors (`TI=1` on the source).
fn exec_lldt(cpu: &mut CpuState, bus: &mut dyn Bus, insn: &DecodedInsn) -> Result<(), ExecError> {
    if !cr0_pe(cpu) {
        return Err(arch_fault(6));
    }
    require_cpl0(cpu)?;
    let selector = read_rm_u16(cpu, bus, insn)?;
    if is_null_selector(selector) {
        cpu.ldtr = x86_core::SegmentReg {
            selector,
            base: 0,
            limit: 0,
            flags: 0,
        };
        return Ok(());
    }
    if selector & 0x4 != 0 {
        return Err(selector_fault(13, selector));
    }
    let desc = read_gdt_raw_descriptor(cpu, bus, selector)?;
    let access = desc[5];
    if access & 0x10 != 0 || access & 0x0F != DESC_TYPE_LDT {
        return Err(selector_fault(13, selector));
    }
    if access & 0x80 == 0 {
        return Err(selector_fault(11, selector));
    }
    let parsed = parse_segment_descriptor(desc);
    cpu.ldtr
        .load_descriptor_cache(selector, parsed.base, parsed.limit, parsed.flags);
    Ok(())
}

fn tss32_read_u32(bus: &mut dyn Bus, base: u64, offset: u32) -> Result<u32, ExecError> {
    let mut bytes = [0u8; 4];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = bus
            .read_system_u8(base.wrapping_add(u64::from(offset) + index as u64))
            .map_err(|error| classify_mem_fault(error, false))?;
    }
    Ok(u32::from_le_bytes(bytes))
}

fn tss32_write_u32(bus: &mut dyn Bus, base: u64, offset: u32, value: u32) -> Result<(), ExecError> {
    for (index, byte) in value.to_le_bytes().iter().enumerate() {
        bus.write_system_u8(base.wrapping_add(u64::from(offset) + index as u64), *byte)
            .map_err(|error| classify_mem_fault(error, false))?;
    }
    Ok(())
}

fn tss32_read_u16(bus: &mut dyn Bus, base: u64, offset: u32) -> Result<u16, ExecError> {
    let mut bytes = [0u8; 2];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = bus
            .read_system_u8(base.wrapping_add(u64::from(offset) + index as u64))
            .map_err(|error| classify_mem_fault(error, false))?;
    }
    Ok(u16::from_le_bytes(bytes))
}

fn tss32_write_u16(bus: &mut dyn Bus, base: u64, offset: u32, value: u16) -> Result<(), ExecError> {
    for (index, byte) in value.to_le_bytes().iter().enumerate() {
        bus.write_system_u8(base.wrapping_add(u64::from(offset) + index as u64), *byte)
            .map_err(|error| classify_mem_fault(error, false))?;
    }
    Ok(())
}

fn gdt_descriptor_addr(cpu: &CpuState, selector: u16) -> Result<u64, ExecError> {
    if is_null_selector(selector) {
        return Err(selector_fault(13, selector));
    }
    if selector & 0x4 != 0 {
        return Err(selector_fault(13, selector));
    }
    let offset = u64::from(selector >> 3) * 8;
    if offset + 7 > u64::from(cpu.gdtr.limit) {
        return Err(selector_fault(13, selector));
    }
    Ok(cpu.gdtr.base.wrapping_add(offset))
}

fn read_gdt_raw_descriptor(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<[u8; 8], ExecError> {
    let addr = gdt_descriptor_addr(cpu, selector)?;
    let mut descriptor = [0u8; 8];
    for (index, byte) in descriptor.iter_mut().enumerate() {
        *byte = bus
            .read_system_u8(addr.wrapping_add(index as u64))
            .map_err(|error| classify_mem_fault(error, false))?;
    }
    Ok(descriptor)
}

/// Read an 8-byte descriptor from the GDT (`TI=0`) or current LDT (`TI=1`).
///
/// Spec: Intel SDM Vol. 3 §§3.5.1–3.5.2. A null or unloaded LDTR with `TI=1`
/// raises `#GP(selector)`.
fn read_dt_raw_descriptor(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<[u8; 8], ExecError> {
    if is_null_selector(selector) {
        return Err(selector_fault(13, selector));
    }
    if selector & 0x4 == 0 {
        return read_gdt_raw_descriptor(cpu, bus, selector);
    }
    // LDT path.
    if is_null_selector(cpu.ldtr.selector) {
        return Err(selector_fault(13, selector));
    }
    let offset = u64::from(selector >> 3) * 8;
    if offset + 7 > u64::from(cpu.ldtr.limit) {
        return Err(selector_fault(13, selector));
    }
    let addr = cpu.ldtr.base.wrapping_add(offset);
    let mut descriptor = [0u8; 8];
    for (index, byte) in descriptor.iter_mut().enumerate() {
        *byte = bus
            .read_system_u8(addr.wrapping_add(index as u64))
            .map_err(|error| classify_mem_fault(error, false))?;
    }
    Ok(descriptor)
}

fn write_gdt_access_byte(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
    access: u8,
) -> Result<(), ExecError> {
    let addr = gdt_descriptor_addr(cpu, selector)?;
    bus.write_system_u8(addr.wrapping_add(5), access)
        .map_err(|error| classify_mem_fault(error, false))
}

/// Whether a GDT selector is a task-switch target (TSS or task gate).
///
/// Used by far `JMP`/`CALL` to distinguish code-segment transfers from the
/// bounded hardware task-switch path. Spec: Intel SDM Vol. 3 Table 3-2.
fn protected_far_is_task_target(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<bool, ExecError> {
    if is_null_selector(selector) || selector & 0x4 != 0 {
        return Ok(false);
    }
    let offset = u64::from(selector >> 3) * 8;
    if offset + 7 > u64::from(cpu.gdtr.limit) {
        return Ok(false);
    }
    let access = bus
        .read_system_u8(cpu.gdtr.base.wrapping_add(offset).wrapping_add(5))
        .map_err(|error| classify_mem_fault(error, false))?;
    if access & 0x10 != 0 {
        return Ok(false);
    }
    Ok(matches!(
        access & 0x0F,
        DESC_TYPE_TASK_GATE | DESC_TYPE_TSS32_AVAILABLE | DESC_TYPE_TSS32_BUSY
    ))
}

/// Resolve a far selector to an available 32-bit TSS (direct or via task gate).
///
/// Spec: Intel SDM Vol. 3 §7.3 / Figure 7-5 (privilege checks for JMP and CALL).
fn resolve_task_tss_selector(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<(u16, [u8; 8]), ExecError> {
    let cpl = (cpu.cs.selector & 3) as u8;
    let rpl = (selector & 3) as u8;
    let desc = read_gdt_raw_descriptor(cpu, bus, selector)?;
    let access = desc[5];
    if access & 0x10 != 0 {
        return Err(selector_fault(13, selector));
    }
    let type_field = access & 0x0F;
    let dpl = (access >> 5) & 3;
    let present = access & 0x80 != 0;

    let tss_selector = match type_field {
        DESC_TYPE_TASK_GATE => {
            // Gate: CPL ≤ DPL and RPL ≤ DPL.
            if cpl > dpl || rpl > dpl {
                return Err(selector_fault(13, selector));
            }
            if !present {
                return Err(selector_fault(11, selector));
            }
            u16::from_le_bytes([desc[2], desc[3]])
        }
        DESC_TYPE_TSS32_AVAILABLE | DESC_TYPE_TSS32_BUSY => {
            // Direct TSS: max(CPL, RPL) ≤ DPL.
            if cpl > dpl || rpl > dpl {
                return Err(selector_fault(13, selector));
            }
            if !present {
                return Err(selector_fault(11, selector));
            }
            selector
        }
        _ => return Err(selector_fault(13, selector)),
    };

    if is_null_selector(tss_selector) || tss_selector & 0x4 != 0 {
        return Err(selector_fault(13, tss_selector));
    }
    let tss_desc = read_gdt_raw_descriptor(cpu, bus, tss_selector)?;
    let tss_access = tss_desc[5];
    if tss_access & 0x10 != 0 || tss_access & 0x0F != DESC_TYPE_TSS32_AVAILABLE {
        // Busy or wrong type → #GP(selector). Spec: Vol. 3 §7.3.
        return Err(selector_fault(13, tss_selector));
    }
    if tss_access & 0x80 == 0 {
        return Err(selector_fault(11, tss_selector));
    }
    let parsed = parse_segment_descriptor(tss_desc);
    if parsed.limit < TSS32_MIN_LIMIT {
        return Err(selector_fault(13, tss_selector));
    }
    Ok((tss_selector, tss_desc))
}

fn prepare_task_cs(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<x86_core::SegmentReg, ExecError> {
    if is_null_selector(selector) {
        return Err(selector_fault(13, selector));
    }
    let desc = read_gdt_segment_descriptor(cpu, bus, selector)?;
    let access = desc[5];
    let s_bit = access & 0x10 != 0;
    let executable = access & 0x08 != 0;
    let conforming = executable && access & 0x04 != 0;
    let rpl = (selector & 3) as u8;
    let dpl = (access >> 5) & 3;
    if !s_bit || !executable {
        return Err(selector_fault(13, selector));
    }
    if conforming {
        if dpl > rpl {
            return Err(selector_fault(13, selector));
        }
    } else if dpl != rpl {
        return Err(selector_fault(13, selector));
    }
    if access & 0x80 == 0 {
        return Err(selector_fault(11, selector));
    }
    let parsed = parse_segment_descriptor(desc);
    if parsed.flags & x86_core::SegmentReg::FLAG_LONG != 0 {
        return Err(selector_fault(13, selector));
    }
    Ok(x86_core::SegmentReg {
        selector,
        base: parsed.base,
        limit: parsed.limit,
        flags: parsed.flags,
    })
}

fn prepare_task_data_sreg(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
    cpl: u8,
) -> Result<x86_core::SegmentReg, ExecError> {
    if is_null_selector(selector) {
        return Ok(x86_core::SegmentReg {
            selector,
            base: 0,
            limit: 0,
            flags: 0,
        });
    }
    let descriptor = read_gdt_segment_descriptor(cpu, bus, selector)?;
    let (base, limit, flags) = parse_data_segment_descriptor(descriptor, selector, cpl)?;
    Ok(x86_core::SegmentReg {
        selector,
        base,
        limit,
        flags,
    })
}

fn prepare_task_ldtr(
    cpu: &CpuState,
    bus: &mut dyn Bus,
    selector: u16,
) -> Result<x86_core::SegmentReg, ExecError> {
    if is_null_selector(selector) {
        return Ok(x86_core::SegmentReg {
            selector,
            base: 0,
            limit: 0,
            flags: 0,
        });
    }
    if selector & 0x4 != 0 {
        return Err(selector_fault(13, selector));
    }
    let desc = read_gdt_raw_descriptor(cpu, bus, selector)?;
    let access = desc[5];
    if access & 0x10 != 0 || access & 0x0F != DESC_TYPE_LDT {
        return Err(selector_fault(13, selector));
    }
    if access & 0x80 == 0 {
        return Err(selector_fault(11, selector));
    }
    let parsed = parse_segment_descriptor(desc);
    Ok(x86_core::SegmentReg {
        selector,
        base: parsed.base,
        limit: parsed.limit,
        flags: parsed.flags,
    })
}

/// Hardware task switch via far `JMP` to a 32-bit TSS or task gate.
///
/// Spec: Intel SDM Vol. 2 "JMP"; Vol. 3 §§7.2–7.3.
fn task_switch_jmp(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    selector: u16,
    next_ip: u32,
) -> Result<(), ExecError> {
    task_switch(cpu, bus, TaskSwitchCause::Jmp, Some(selector), next_ip)
}

/// Hardware task switch via far `CALL` to a 32-bit TSS or task gate.
///
/// Keeps the outgoing TSS busy, writes the previous-task link, and sets `NT`.
/// Spec: Intel SDM Vol. 2 "CALL"; Vol. 3 §§7.2–7.3 Table 7-1.
fn task_switch_call(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    selector: u16,
    next_ip: u32,
) -> Result<(), ExecError> {
    task_switch(cpu, bus, TaskSwitchCause::Call, Some(selector), next_ip)
}

/// Shared 32-bit TSS task-switch engine (JMP / CALL / IRET forms).
///
/// Spec: Intel SDM Vol. 3 §§7.2–7.3.
///
/// Unsupported here: `EFLAGS.VM=1` targets, 16-bit TSS, IDT task-gate delivery,
/// and LDT-resident TSS/gate descriptors.
fn task_switch(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    cause: TaskSwitchCause,
    selector: Option<u16>,
    next_ip: u32,
) -> Result<(), ExecError> {
    let old_type = (cpu.tr.flags & 0x0F) as u8;
    if old_type != DESC_TYPE_TSS32_BUSY || cpu.tr.limit < TSS32_MIN_LIMIT {
        return Err(selector_fault(13, cpu.tr.selector));
    }

    let (new_sel, new_desc) = match cause {
        TaskSwitchCause::Jmp | TaskSwitchCause::Call => {
            let sel = selector.expect("JMP/CALL require a far selector");
            resolve_task_tss_selector(cpu, bus, sel)?
        }
        TaskSwitchCause::Iret => {
            // Nested-task return: destination is the previous-task link and must
            // already be busy. Spec: Vol. 3 §7.3 / Vol. 2 IRET.
            let link = tss32_read_u16(bus, cpu.tr.base, TSS32_OFF_LINK)?;
            if is_null_selector(link) || link & 0x4 != 0 {
                return Err(selector_fault(13, link));
            }
            let tss_desc = read_gdt_raw_descriptor(cpu, bus, link)?;
            let tss_access = tss_desc[5];
            if tss_access & 0x10 != 0 || tss_access & 0x0F != DESC_TYPE_TSS32_BUSY {
                return Err(selector_fault(13, link));
            }
            if tss_access & 0x80 == 0 {
                return Err(selector_fault(11, link));
            }
            let parsed = parse_segment_descriptor(tss_desc);
            if parsed.limit < TSS32_MIN_LIMIT {
                return Err(selector_fault(13, link));
            }
            (link, tss_desc)
        }
    };
    let new_parsed = parse_segment_descriptor(new_desc);
    let new_base = new_parsed.base;

    // Reject VM86 targets before mutating either TSS. Spec: Vol. 3 §7.3;
    // VM86 task entry is a later slice.
    let new_eflags = tss32_read_u32(bus, new_base, TSS32_OFF_EFLAGS)?;
    if new_eflags & (1 << 17) != 0 {
        return Err(ExecError::Unsupported(match cause {
            TaskSwitchCause::Call => 0x9A,
            TaskSwitchCause::Iret => 0xCF,
            TaskSwitchCause::Jmp => 0xEA,
        }));
    }
    let new_eip = tss32_read_u32(bus, new_base, TSS32_OFF_EIP)?;
    let new_cr3 = tss32_read_u32(bus, new_base, TSS32_OFF_CR3)?;
    let new_eax = tss32_read_u32(bus, new_base, TSS32_OFF_EAX)?;
    let new_ecx = tss32_read_u32(bus, new_base, TSS32_OFF_ECX)?;
    let new_edx = tss32_read_u32(bus, new_base, TSS32_OFF_EDX)?;
    let new_ebx = tss32_read_u32(bus, new_base, TSS32_OFF_EBX)?;
    let new_esp = tss32_read_u32(bus, new_base, TSS32_OFF_ESP)?;
    let new_ebp = tss32_read_u32(bus, new_base, TSS32_OFF_EBP)?;
    let new_esi = tss32_read_u32(bus, new_base, TSS32_OFF_ESI)?;
    let new_edi = tss32_read_u32(bus, new_base, TSS32_OFF_EDI)?;
    let new_es = tss32_read_u16(bus, new_base, TSS32_OFF_ES)?;
    let new_cs_sel = tss32_read_u16(bus, new_base, TSS32_OFF_CS)?;
    let new_ss_sel = tss32_read_u16(bus, new_base, TSS32_OFF_SS)?;
    let new_ds = tss32_read_u16(bus, new_base, TSS32_OFF_DS)?;
    let new_fs = tss32_read_u16(bus, new_base, TSS32_OFF_FS)?;
    let new_gs = tss32_read_u16(bus, new_base, TSS32_OFF_GS)?;
    let new_ldtr_sel = tss32_read_u16(bus, new_base, TSS32_OFF_LDTR)?;

    // Validate incoming segments before committing the outgoing save.
    let cs_loaded = prepare_task_cs(cpu, bus, new_cs_sel)?;
    let new_cpl = (new_cs_sel & 3) as u8;
    let ss_loaded = prepare_ss_from_gdt_for_cpl(cpu, bus, new_ss_sel, new_cpl)?;
    let es_loaded = prepare_task_data_sreg(cpu, bus, new_es, new_cpl)?;
    let ds_loaded = prepare_task_data_sreg(cpu, bus, new_ds, new_cpl)?;
    let fs_loaded = prepare_task_data_sreg(cpu, bus, new_fs, new_cpl)?;
    let gs_loaded = prepare_task_data_sreg(cpu, bus, new_gs, new_cpl)?;
    let ldtr_loaded = prepare_task_ldtr(cpu, bus, new_ldtr_sel)?;
    if new_eip > cs_loaded.limit {
        return Err(arch_fault_with_error_code(13, 0));
    }

    // Save outgoing architectural state into the current TSS.
    let old_base = cpu.tr.base;
    let old_sel = cpu.tr.selector;
    tss32_write_u32(bus, old_base, TSS32_OFF_EIP, next_ip)?;
    tss32_write_u32(bus, old_base, TSS32_OFF_EFLAGS, cpu.rflags as u32)?;
    tss32_write_u32(bus, old_base, TSS32_OFF_EAX, cpu.gpr_u32(CpuState::RAX))?;
    tss32_write_u32(bus, old_base, TSS32_OFF_ECX, cpu.gpr_u32(CpuState::RCX))?;
    tss32_write_u32(bus, old_base, TSS32_OFF_EDX, cpu.gpr_u32(CpuState::RDX))?;
    tss32_write_u32(bus, old_base, TSS32_OFF_EBX, cpu.gpr_u32(CpuState::RBX))?;
    tss32_write_u32(bus, old_base, TSS32_OFF_ESP, cpu.gpr_u32(CpuState::RSP))?;
    tss32_write_u32(bus, old_base, TSS32_OFF_EBP, cpu.gpr_u32(CpuState::RBP))?;
    tss32_write_u32(bus, old_base, TSS32_OFF_ESI, cpu.gpr_u32(CpuState::RSI))?;
    tss32_write_u32(bus, old_base, TSS32_OFF_EDI, cpu.gpr_u32(CpuState::RDI))?;
    tss32_write_u16(bus, old_base, TSS32_OFF_ES, cpu.es.selector)?;
    tss32_write_u16(bus, old_base, TSS32_OFF_CS, cpu.cs.selector)?;
    tss32_write_u16(bus, old_base, TSS32_OFF_SS, cpu.ss.selector)?;
    tss32_write_u16(bus, old_base, TSS32_OFF_DS, cpu.ds.selector)?;
    tss32_write_u16(bus, old_base, TSS32_OFF_FS, cpu.fs.selector)?;
    tss32_write_u16(bus, old_base, TSS32_OFF_GS, cpu.gs.selector)?;
    tss32_write_u16(bus, old_base, TSS32_OFF_LDTR, cpu.ldtr.selector)?;
    tss32_write_u32(bus, old_base, TSS32_OFF_CR3, cpu.cr3 as u32)?;

    match cause {
        TaskSwitchCause::Call => {
            // Nested CALL: old stays busy; write previous-task link; set NT.
            tss32_write_u16(bus, new_base, TSS32_OFF_LINK, old_sel)?;
            let new_access = (new_desc[5] & 0xF0) | DESC_TYPE_TSS32_BUSY;
            write_gdt_access_byte(cpu, bus, new_sel, new_access)?;
        }
        TaskSwitchCause::Jmp | TaskSwitchCause::Iret => {
            // JMP / IRET: clear busy on the outgoing TSS.
            let old_access = ((cpu.tr.flags & 0xFF) as u8 & 0xF0) | DESC_TYPE_TSS32_AVAILABLE;
            write_gdt_access_byte(cpu, bus, old_sel, old_access)?;
            if cause == TaskSwitchCause::Jmp {
                let new_access = (new_desc[5] & 0xF0) | DESC_TYPE_TSS32_BUSY;
                write_gdt_access_byte(cpu, bus, new_sel, new_access)?;
            }
            // IRET returns to an already-busy TSS; leave its busy bit set.
        }
    }

    cpu.tr
        .load_descriptor_cache(new_sel, new_base, new_parsed.limit, {
            let mut flags = new_parsed.flags;
            flags = (flags & !0x0F) | u16::from(DESC_TYPE_TSS32_BUSY);
            flags
        });

    cpu.cr3 = u64::from(new_cr3);
    note_control_register_write(cpu, bus, 3);
    cpu.rip = u64::from(new_eip);
    // JMP/IRET clear NT; CALL sets NT. Spec: Vol. 3 §7.3 Table 7-1.
    let mut loaded_flags = new_eflags;
    match cause {
        TaskSwitchCause::Call => loaded_flags |= 1 << 14,
        TaskSwitchCause::Jmp | TaskSwitchCause::Iret => loaded_flags &= !(1 << 14),
    }
    cpu.rflags = u64::from(loaded_flags);
    cpu.set_gpr_u32(CpuState::RAX, new_eax);
    cpu.set_gpr_u32(CpuState::RCX, new_ecx);
    cpu.set_gpr_u32(CpuState::RDX, new_edx);
    cpu.set_gpr_u32(CpuState::RBX, new_ebx);
    cpu.set_gpr_u32(CpuState::RSP, new_esp);
    cpu.set_gpr_u32(CpuState::RBP, new_ebp);
    cpu.set_gpr_u32(CpuState::RSI, new_esi);
    cpu.set_gpr_u32(CpuState::RDI, new_edi);
    cpu.cs = cs_loaded;
    cpu.ss = ss_loaded;
    cpu.es = es_loaded;
    cpu.ds = ds_loaded;
    cpu.fs = fs_loaded;
    cpu.gs = gs_loaded;
    cpu.ldtr = ldtr_loaded;
    cpu.cr0 |= 1u64 << 3; // CR0.TS
    note_control_register_write(cpu, bus, 0);
    Ok(())
}

/// Load/store GDTR/IDTR pseudo-descriptor `m16&32` (limit16 + base32).
/// Spec: Intel SDM Vol. 2 "LGDT/SGDT" / "LIDT/SIDT"; Vol. 3 §2.4.1 / §2.4.3.
///
/// Operand-size 16: base uses bits 23:0 (bits 31:24 ignored on load; stored 0 on store).
/// Operand-size 32 (`0x66`): full 32-bit base. Memory form only (mod=11 → `#UD`).
fn dtr_pseudo_desc(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    load: bool,
    idtr: bool,
) -> Result<(), ExecError> {
    let m = insn.modrm.ok_or(ExecError::Unsupported(0x01))?;
    if m.mod_ == 3 {
        // Spec: SDM Vol. 2 LGDT/SGDT / LIDT/SIDT — register form #UD
        return Err(arch_fault(6));
    }
    let (addr, _, uses_ss) = ea(cpu, insn, 6)?;
    let dtr = if idtr { &mut cpu.idtr } else { &mut cpu.gdtr };
    if load {
        let limit = bus
            .read_u16(addr)
            .map_err(|e| classify_mem_fault(e, uses_ss))?;
        let mut base = u64::from(
            bus.read_u32(addr.wrapping_add(2))
                .map_err(|e| classify_mem_fault(e, uses_ss))?,
        );
        if !opsz32(insn) {
            // Spec: SDM Vol. 2 LGDT/LIDT — 16-bit operand-size uses 24-bit base.
            base &= 0x00FF_FFFF;
        }
        dtr.limit = limit;
        dtr.base = base;
    } else {
        bus.write_u16(addr, dtr.limit)
            .map_err(|e| classify_mem_fault(e, uses_ss))?;
        let mut base = dtr.base as u32;
        if !opsz32(insn) {
            // Spec: SDM Vol. 2 SGDT/SIDT — 16-bit operand-size stores base[31:24]=0.
            base &= 0x00FF_FFFF;
        }
        bus.write_u32(addr.wrapping_add(2), base)
            .map_err(|e| classify_mem_fault(e, uses_ss))?;
    }
    Ok(())
}

/// Two-byte opcode map (0F xx). Spec: Intel SDM Vol. 2 Chapter 2; "LGDT"/"SGDT"/"IMUL".
fn step_two_byte(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    insn: &DecodedInsn,
    next_ip: u32,
) -> Result<(), ExecError> {
    match insn.opcode {
        0x06 => {
            // CLTS — Spec: Intel SDM Vol. 2 "CLTS—Clear Task-Switched Flag in
            // CR0"; Vol. 3 §2.5 (CR0.TS = bit 3). Clears TS only; all other
            // CR0 bits (including PE) are unchanged. Real-mode path only —
            // protected-mode CPL=0 / #GP(0) checks are out of scope here.
            cpu.cr0 &= !(1u64 << 3);
            note_control_register_write(cpu, bus, 0);
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0x00 => {
            // Group 6 — Spec: Intel SDM Vol. 2 opcode map 2; "SLDT"/"STR"/
            // "LLDT"/"LTR"/"VERR"/"VERW"; Vol. 3 §§2.4.2, 7.2–7.3.
            // Unsupported here: 16-bit TSS descriptors and nested-task CALL.
            let m = insn.modrm.ok_or(ExecError::Unsupported(0x00))?;
            match m.reg {
                0 => {
                    // SLDT r/m16 — store the visible LDTR selector. Spec: SDM
                    // Vol. 2 "SLDT". Valid in real-address mode as well.
                    write_rm_u16(cpu, bus, insn, cpu.ldtr.selector)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                1 => {
                    // STR r/m16 — store the visible TR selector. No privilege
                    // check; valid in real-address mode as well (stores the
                    // cached selector). Spec: SDM Vol. 2 "STR".
                    write_rm_u16(cpu, bus, insn, cpu.tr.selector)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                2 => {
                    // LLDT r/m16 — load LDTR from a GDT LDT descriptor.
                    exec_lldt(cpu, bus, insn)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                3 => {
                    // LTR r/m16 — load TR from a 32-bit available TSS.
                    exec_ltr(cpu, bus, insn)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                4 => {
                    // VERR r/m16 — Spec: Intel SDM Vol. 2 "VERR".
                    exec_verr_verw(cpu, bus, insn, false)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                5 => {
                    // VERW r/m16 — Spec: Intel SDM Vol. 2 "VERW".
                    exec_verr_verw(cpu, bus, insn, true)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                _ => Err(ExecError::Unsupported(0x00)),
            }
        }
        0x02 => {
            // LAR r, r/m16 — Spec: Intel SDM Vol. 2 "LAR".
            exec_lar_lsl(cpu, bus, insn, false)?;
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0x03 => {
            // LSL r, r/m16 — Spec: Intel SDM Vol. 2 "LSL".
            exec_lar_lsl(cpu, bus, insn, true)?;
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0x01 => {
            // Group 7 — Spec: Intel SDM Vol. 2 opcode map 2;
            // "SGDT"/"SIDT"/"LGDT"/"LIDT"/"SMSW"/"LMSW"/"INVLPG".
            // Unsupported here: /5 (extensions); protected-mode entry from PE;
            // paging/TLB invalidate side effects (real-mode INVLPG is a NOP).
            let m = insn.modrm.ok_or(ExecError::Unsupported(0x01))?;
            match m.reg {
                0 => {
                    // SGDT m16&32
                    dtr_pseudo_desc(cpu, bus, insn, false, false)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                1 => {
                    // SIDT m16&32
                    dtr_pseudo_desc(cpu, bus, insn, false, true)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                2 => {
                    // LGDT m16&32
                    dtr_pseudo_desc(cpu, bus, insn, true, false)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                3 => {
                    // LIDT m16&32
                    dtr_pseudo_desc(cpu, bus, insn, true, true)?;
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                4 => {
                    // SMSW r/m16 — Spec: SDM Vol. 2 "SMSW"; stores CR0[15:0].
                    // Memory destination is always 16-bit; register + opsize32
                    // zero-extends into r32 (deterministic; upper bits undefined in SDM).
                    let msw = cpu.cr0 as u16;
                    if m.mod_ == 3 && opsz32(insn) {
                        cpu.set_gpr_u32(m.rm as usize, u32::from(msw));
                    } else {
                        write_rm_u16(cpu, bus, insn, msw)?;
                    }
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                6 => {
                    // LMSW r/m16 — Spec: SDM Vol. 2 "LMSW"; Vol. 3 §2.5 (CR0.PE).
                    // Loads CR0[15:0]. Cannot clear PE once set. Setting PE=1
                    // enables protected-mode GDT descriptor loads for MOV
                    // DS/ES/FS/GS/SS and bounded direct far JMP16 transfers.
                    let src = read_rm_u16(cpu, bus, insn)?;
                    let pe_was = cpu.cr0 & 1 != 0;
                    let mut low = u64::from(src);
                    if pe_was {
                        low |= 1; // Spec: LMSW cannot clear PE
                    }
                    cpu.cr0 = (cpu.cr0 & !0xFFFF) | low;
                    // LMSW reaches only CR0[15:0], so it can change neither PG
                    // (bit 31) nor WP (bit 16) and implies no invalidation; the
                    // hook still runs so the memory path's shadow of CR0 stays
                    // exact. Spec: SDM Vol. 3 §4.10.4.1.
                    note_control_register_write(cpu, bus, 0);
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                7 => {
                    // INVLPG m — Spec: Intel SDM Vol. 2 "INVLPG—Invalidate TLB
                    // Entries"; Vol. 3 §4.10.4.1. Register form (mod=11) → #UD,
                    // and CPL != 0 → #GP(0).
                    //
                    // The operand is an address, not an operand the processor
                    // reads: the effective address is formed and limit-checked
                    // (§5.3) but no byte is loaded or stored, so `INVLPG` can
                    // raise `#GP`/`#SS` for a limit violation and never `#PF`.
                    // With no translation cached — which includes every
                    // real-address-mode execution — invalidating nothing is
                    // still the architectural result, so this stays a NOP by
                    // consequence rather than by special case.
                    if m.mod_ == 3 {
                        return Err(arch_fault(6));
                    }
                    require_cpl0(cpu)?;
                    let (addr, _, _) = ea(cpu, insn, 1)?;
                    bus.invalidate_page(addr);
                    set_current_ip(cpu, next_ip);
                    Ok(())
                }
                _ => Err(ExecError::Unsupported(0x01)),
            }
        }
        0x20 => {
            // MOV r32, CR0 — Spec: Intel SDM Vol. 2 "MOV—Move to/from Control
            // Registers"; Vol. 3 §2.5 (CR0). ModRM.reg selects the control
            // register; the mod field is architecturally ignored (decoder
            // never populates SIB/displacement for this opcode). Operand
            // size is always 32 bits regardless of any 0x66 prefix.
            // CR1 and CR5-CR7 → #UD. CPL != 0 → #GP(0) (Vol. 3 §5.5).
            // Unsupported here: CR8 (`REX.R` in 64-bit mode).
            let m = insn.modrm.ok_or(ExecError::Unsupported(0x20))?;
            let value = match m.reg {
                0 => cpu.cr0,
                2 => cpu.cr2,
                3 => cpu.cr3,
                4 => cpu.cr4,
                _ => return real_mode_ud(cpu, bus),
            };
            require_cpl0(cpu)?;
            cpu.set_gpr_u32(m.rm as usize, value as u32);
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0x22 => {
            // MOV CR0, r32 — Spec: Intel SDM Vol. 2 "MOV—Move to/from Control
            // Registers"; Vol. 3 §2.5 (CR0). Unlike LMSW, this instruction
            // MAY clear PE. PE=1 enables GDT descriptor loads for MOV
            // DS/ES/FS/GS/SS and bounded direct far JMP16 transfers. Clearing PE
            // restores the sticky-unreal data-segment and real-mode far-JMP paths.
            //
            // CR1 and CR5-CR7 → #UD; CPL != 0 → #GP(0). Writing 1 to a reserved
            // bit of CR0 or CR4 → #GP(0). Outside 64-bit mode the write is 32
            // bits wide and clears the register's upper doubleword.
            // Unsupported here: CR8, and the CR0 reserved-bit and
            // NW/CD/PE/PG-combination checks other than the PG-without-PE one
            // below.
            let m = insn.modrm.ok_or(ExecError::Unsupported(0x22))?;
            if !matches!(m.reg, 0 | 2 | 3 | 4) {
                return real_mode_ud(cpu, bus);
            }
            require_cpl0(cpu)?;
            let src = u64::from(cpu.gpr_u32(m.rm as usize));
            match m.reg {
                0 => {
                    // Spec: SDM Vol. 2 MOV CRn — "#GP(0) if an attempt is made
                    // to set CR0.PG when CR0.PE is clear"; Vol. 3 §4.1.1 makes
                    // protected mode a precondition of paging.
                    if src & CR0_PG != 0 && src & 1 == 0 {
                        return Err(arch_fault_with_error_code(13, 0));
                    }
                    cpu.cr0 = src;
                }
                2 => {
                    // CR2 holds the linear address of the last page fault
                    // (SDM Vol. 3 §4.7); it has no reserved bits.
                    cpu.cr2 = src;
                }
                3 => {
                    // Spec: SDM Vol. 3 Table 4-3 — with 32-bit paging, CR3 bits
                    // 2:0 and 11:5 are *ignored*, not reserved, so no bit of a
                    // 32-bit write can raise #GP. Bits 31:12 locate the page
                    // directory; PWT/PCD are stored and inert.
                    cpu.cr3 = src;
                }
                _ => {
                    if src & cr4_reserved_mask() != 0 {
                        return Err(arch_fault_with_error_code(13, 0));
                    }
                    cpu.cr4 = src;
                }
            }
            note_control_register_write(cpu, bus, m.reg);
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0x80..=0x8F => {
            // Jcc rel16/rel32 (near) — Spec: Intel SDM Vol. 2 "Jcc—Jump if
            // Condition Is Met". The displacement is relative to the next
            // instruction and follows the operand-size attribute; a 16-bit
            // operand size clears `EIP[31:16]` (shared `near_branch_target`).
            // Flags are not modified. The `CS`-limit check for the target
            // happens on the next instruction fetch.
            // Unsupported here: `rel32` under a `D=0` code segment commits only
            // `IP` (the shared `set_current_ip` window), and 64-bit mode.
            if condition_code(cpu, insn.opcode) {
                set_current_ip(
                    cpu,
                    near_branch_target(next_ip, insn.immediate, opsz32(insn)),
                );
            } else {
                set_current_ip(cpu, next_ip);
            }
            Ok(())
        }
        0x40..=0x4F => {
            // CMOVcc r, r/m — Spec: Intel SDM Vol. 2 "CMOVcc—Conditional Move":
            // `IF condition THEN DEST := SRC`. The condition comes from the
            // shared low-nibble evaluator, so `CMOVcc` cannot disagree with
            // `Jcc` or `SETcc`. There is no byte form; the width follows the
            // operand-size attribute, and no flags are written.
            //
            // The source operand is read *before* the condition is evaluated.
            // The SDM allows the processor to read a memory source regardless of
            // the condition, so a source the segment limit or the bus rejects
            // faults whether or not the move happens. Nothing is committed when
            // the read faults.
            //
            // Unsupported here: the REX.W `r64` form, and CPUID leaf 1 EDX bit 15
            // (`CMOV`) deliberately stays clear — ADR-0007 governs CPUID, and
            // under-reporting an implemented feature is safe.
            let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
            if opsz32(insn) {
                let src = read_rm_u32(cpu, bus, insn)?;
                if condition_code(cpu, insn.opcode) {
                    cpu.set_gpr_u32(m.reg as usize, src);
                }
            } else {
                let src = read_rm_u16(cpu, bus, insn)?;
                if condition_code(cpu, insn.opcode) {
                    cpu.set_gpr_u16(m.reg as usize, src);
                }
            }
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0x90..=0x9F => {
            // SETcc r/m8 — Spec: Intel SDM Vol. 2 "SETcc—Set Byte on
            // Condition": `IF condition THEN DEST := 1 ELSE DEST := 0`.
            // The destination is always a byte (register form covers the
            // legacy high-byte encodings AH/CH/DH/BH); ModR/M.reg is not used
            // and no flags are affected.
            let value = u8::from(condition_code(cpu, insn.opcode));
            write_rm_u8(cpu, bus, insn, value)?;
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xA0 | 0xA8 => {
            // PUSH FS / PUSH GS — Spec: Intel SDM Vol. 2 "PUSH". A 32-bit
            // operand size pushes the zero-extended selector in a doubleword
            // slot; the stack-pointer width itself follows `SS.B`.
            let selector = if insn.opcode == 0xA0 {
                cpu.fs.selector
            } else {
                cpu.gs.selector
            };
            push_sreg(cpu, bus, selector, opsz32(insn))?;
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xA1 | 0xA9 => {
            // POP FS / POP GS — Spec: Intel SDM Vol. 2 "POP"; Vol. 3 §§3.5.1,
            // 5.4.1. In protected mode the selector is validated through the
            // shared DS/ES data-descriptor path (a null selector is allowed and
            // clears the cache) before the stack pointer or the cache commits.
            let sreg = if insn.opcode == 0xA1 { 4 } else { 5 };
            pop_sreg(cpu, bus, sreg, opsz32(insn))?;
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xB2 => {
            // LSS r16/r32, m16:16/m16:32 — Spec: Intel SDM Vol. 2
            // "LDS/LES/LFS/LGS/LSS". Loads SS through the same stack-segment
            // descriptor rules as `MOV SS`/`POP SS`, and like them inhibits
            // maskable interrupts through the following instruction
            // (Vol. 3 §6.8.3).
            load_far_pointer(cpu, bus, insn, 2)?;
            cpu.arm_maskable_interrupt_shadow();
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xB4 | 0xB5 => {
            // LFS / LGS r16/r32, m16:16/m16:32 — Spec: Intel SDM Vol. 2
            // "LDS/LES/LFS/LGS/LSS". FS/GS follow the DS/ES data rules, so a
            // null selector loads and clears the cache without faulting.
            let sreg = if insn.opcode == 0xB4 { 4 } else { 5 };
            load_far_pointer(cpu, bus, insn, sreg)?;
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xB6 | 0xB7 | 0xBE | 0xBF => {
            // MOVZX / MOVSX Gv, Eb|Ew — Spec: Intel SDM Vol. 2 "MOVZX—Move
            // with Zero-Extend" / "MOVSX—Move with Sign-Extension". The opcode
            // fixes the source width (`B6`/`BE` byte, `B7`/`BF` word) while the
            // destination width follows the operand-size attribute, so a 16-bit
            // operand size with a word source is an ordinary word move. No
            // flags are affected. Byte sources use the legacy `AL..BH` register
            // encodings in the `mod=11` form.
            // Unsupported here: the REX.W `r64` destinations and `MOVSXD`.
            let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
            let sign_extend = matches!(insn.opcode, 0xBE | 0xBF);
            let value = if matches!(insn.opcode, 0xB6 | 0xBE) {
                let src = read_rm_u8(cpu, bus, insn)?;
                if sign_extend {
                    src as i8 as i32 as u32
                } else {
                    u32::from(src)
                }
            } else {
                let src = read_rm_u16(cpu, bus, insn)?;
                if sign_extend {
                    src as i16 as i32 as u32
                } else {
                    u32::from(src)
                }
            };
            if opsz32(insn) {
                cpu.set_gpr_u32(m.reg as usize, value);
            } else {
                cpu.set_gpr_u16(m.reg as usize, value as u16);
            }
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0x0B => {
            // UD2 — Spec: Intel SDM Vol. 2 "UD2—Undefined Instruction":
            // raises `#UD` in every operating mode. It is the architecturally
            // guaranteed invalid opcode, so it must not be reported as a host
            // decode gap.
            Err(arch_fault(6))
        }
        0x08 | 0x09 => {
            // INVD / WBINVD — Spec: Intel SDM Vol. 2 "INVD"/"WBINVD". This
            // emulator models no processor caches, so both are architectural
            // no-ops; only the CPL 0 requirement is observable.
            // Unsupported here: any external write-back cycle or cache-coherence
            // effect, and the `#UD` for a `LOCK` prefix.
            require_cpl0(cpu)?;
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xA2 => {
            // CPUID — Spec: Intel SDM Vol. 2 "CPUID". The leaf comes from EAX
            // and the result replaces EAX/EBX/ECX/EDX. No flags are affected.
            // See `cpuid_leaf` for what this emulator honestly reports.
            // Unsupported here: `ECX` sub-leaf selection (no leaf that uses it
            // is implemented) and the `#UD` for a `LOCK` prefix.
            let result = cpuid_leaf(cpu.eax());
            cpu.set_gpr_u32(CpuState::RAX, result.eax);
            cpu.set_gpr_u32(CpuState::RBX, result.ebx);
            cpu.set_gpr_u32(CpuState::RCX, result.ecx);
            cpu.set_gpr_u32(CpuState::RDX, result.edx);
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0x32 => {
            // RDMSR — Spec: Intel SDM Vol. 2 "RDMSR". ECX selects the MSR and
            // the 64-bit value is returned in EDX:EAX. A reserved or
            // unimplemented address is `#GP`, and outside real-address mode the
            // instruction requires CPL 0.
            require_cpl0(cpu)?;
            let index = cpu.gpr_u32(CpuState::RCX);
            let value = read_msr(cpu, index).ok_or_else(|| arch_fault_with_error_code(13, 0))?;
            cpu.set_gpr_u32(CpuState::RAX, value as u32);
            cpu.set_gpr_u32(CpuState::RDX, (value >> 32) as u32);
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0x30 => {
            // WRMSR — Spec: Intel SDM Vol. 2 "WRMSR". ECX selects the MSR and
            // EDX:EAX supplies the 64-bit value; a reserved or unimplemented
            // address is `#GP`, and outside real-address mode the instruction
            // requires CPL 0.
            require_cpl0(cpu)?;
            let index = cpu.gpr_u32(CpuState::RCX);
            let value = (u64::from(cpu.gpr_u32(CpuState::RDX)) << 32)
                | u64::from(cpu.gpr_u32(CpuState::RAX));
            if !write_msr(cpu, index, value) {
                return Err(arch_fault_with_error_code(13, 0));
            }
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xA3 | 0xAB | 0xB3 | 0xBB => {
            // BT / BTS / BTR / BTC r/m, r — Spec: Intel SDM Vol. 2. The bit
            // offset is the full signed ModR/M.reg register, so a memory bit
            // base can address a bit far outside the nominal operand.
            let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
            let bit_offset = if opsz32(insn) {
                cpu.gpr_u32(m.reg as usize) as i32
            } else {
                i32::from(cpu.gpr_u16(m.reg as usize) as i16)
            };
            let op = match insn.opcode {
                0xA3 => BitOp::Test,
                0xAB => BitOp::Set,
                0xB3 => BitOp::Reset,
                _ => BitOp::Complement,
            };
            exec_bit_op(cpu, bus, insn, op, bit_offset)?;
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xBA => {
            // Group 8: BT/BTS/BTR/BTC r/m, imm8 — Spec: Intel SDM Vol. 2
            // opcode map 2, Group 8. `/0`–`/3` are reserved → `#UD`.
            //
            // The SDM defines the immediate bit offset over `0..OperandSize-1`;
            // this interpreter reduces the imm8 modulo the operand size, which
            // is exact over that documented range. Immediates above it are
            // outside the defined domain and are masked rather than extending
            // the bit-string address.
            let m = insn.modrm.ok_or(ExecError::Unsupported(0xBA))?;
            let op = match m.reg {
                4 => BitOp::Test,
                5 => BitOp::Set,
                6 => BitOp::Reset,
                7 => BitOp::Complement,
                _ => return Err(arch_fault(6)),
            };
            let operand_bits = if opsz32(insn) { 32 } else { 16 };
            let bit_offset = (insn.immediate & 0xFF) % operand_bits;
            exec_bit_op(cpu, bus, insn, op, bit_offset)?;
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xA4 | 0xA5 | 0xAC | 0xAD => {
            // SHLD / SHRD r/m, r, imm8|CL — Spec: Intel SDM Vol. 2 "SHLD—Double
            // Precision Shift Left" / "SHRD—Double Precision Shift Right".
            // The destination is `r/m`, the bits shifted in come from
            // `ModR/M.reg`, and the source register is never modified.
            //
            // The count is reduced modulo 32 outside 64-bit mode *independently
            // of the operand size*, so a 16-bit operand size can receive a count
            // of 17–31. See `double_precision_shift` for what this tree does
            // with that architecturally undefined case and with a zero count.
            //
            // Flags: `CF` is the last bit shifted out of the destination and
            // `SF`/`ZF`/`PF` follow the result. `OF` is defined only for a
            // 1-bit shift (set when the sign changed) and undefined above that,
            // so it is left unchanged for larger counts — the same deterministic
            // choice the Group 2 shifts make. `AF` is undefined and is left
            // unchanged. The destination is written before any flag commits, so
            // a faulting memory write leaves the flags alone.
            //
            // Unsupported here: the REX.W 64-bit forms and the `#UD` a `LOCK`
            // prefix should raise.
            let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
            let left = matches!(insn.opcode, 0xA4 | 0xA5);
            let raw_count = if matches!(insn.opcode, 0xA4 | 0xAC) {
                (insn.immediate as u32) & 0xFF
            } else {
                u32::from(cpu.gpr_u8_low(CpuState::RCX))
            };
            let count = raw_count % 32;
            if opsz32(insn) {
                let dest = read_rm_u32(cpu, bus, insn)?;
                let src = cpu.gpr_u32(m.reg as usize);
                if let Some(shift) = double_precision_shift(left, dest, src, count, 32) {
                    write_rm_u32(cpu, bus, insn, shift.result)?;
                    cpu.set_cf(shift.carry);
                    if count == 1 {
                        cpu.set_of((dest ^ shift.result) & 0x8000_0000 != 0);
                    }
                    set_shift_result_flags_u32(cpu, shift.result);
                }
            } else {
                let dest = read_rm_u16(cpu, bus, insn)?;
                let src = cpu.gpr_u16(m.reg as usize);
                if let Some(shift) =
                    double_precision_shift(left, u32::from(dest), u32::from(src), count, 16)
                {
                    let result = shift.result as u16;
                    write_rm_u16(cpu, bus, insn, result)?;
                    cpu.set_cf(shift.carry);
                    if count == 1 {
                        cpu.set_of((dest ^ result) & 0x8000 != 0);
                    }
                    set_shift_result_flags_u16(cpu, result);
                }
            }
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xBC | 0xBD => {
            // BSF / BSR r, r/m — Spec: Intel SDM Vol. 2 "BSF"/"BSR". A zero
            // source sets ZF and leaves the destination architecturally
            // undefined; this interpreter leaves it unchanged so the reference
            // semantics stay deterministic. CF/OF/SF/AF/PF are undefined and
            // are likewise left unchanged.
            let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
            let forward = insn.opcode == 0xBC;
            if opsz32(insn) {
                let src = read_rm_u32(cpu, bus, insn)?;
                if src == 0 {
                    cpu.set_zf(true);
                } else {
                    cpu.set_zf(false);
                    let index = if forward {
                        src.trailing_zeros()
                    } else {
                        31 - src.leading_zeros()
                    };
                    cpu.set_gpr_u32(m.reg as usize, index);
                }
            } else {
                let src = read_rm_u16(cpu, bus, insn)?;
                if src == 0 {
                    cpu.set_zf(true);
                } else {
                    cpu.set_zf(false);
                    let index = if forward {
                        src.trailing_zeros()
                    } else {
                        15 - src.leading_zeros()
                    };
                    cpu.set_gpr_u16(m.reg as usize, index as u16);
                }
            }
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xC8..=0xCF => {
            // BSWAP r32 — Spec: Intel SDM Vol. 2 "BSWAP". Reverses the four
            // bytes of a doubleword register; no flags are affected.
            // The 16-bit operand-size form is architecturally undefined; this
            // interpreter performs the same 32-bit byte reversal so the
            // reference semantics stay deterministic.
            // Unsupported here: the REX.W `r64` form.
            let idx = (insn.opcode - 0xC8) as usize;
            cpu.set_gpr_u32(idx, cpu.gpr_u32(idx).swap_bytes());
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xC0 | 0xC1 => {
            // XADD r/m, r — Spec: Intel SDM Vol. 2 "XADD":
            // `TEMP := SRC + DEST; SRC := DEST; DEST := TEMP`. Flags follow
            // ADD. The destination write happens before the register exchange
            // so a faulting memory write leaves nothing committed.
            // Unsupported here: `LOCK` atomicity (single-processor model) and
            // the `#UD` for `LOCK` with a register destination.
            let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
            if insn.opcode == 0xC0 {
                let dest = read_rm_u8(cpu, bus, insn)?;
                let src = read_reg_u8(cpu, m.reg);
                let sum = dest.wrapping_add(src);
                write_rm_u8(cpu, bus, insn, sum)?;
                write_reg_u8(cpu, m.reg, dest);
                set_add_flags_u8(cpu, dest, src, sum);
            } else if opsz32(insn) {
                let dest = read_rm_u32(cpu, bus, insn)?;
                let src = cpu.gpr_u32(m.reg as usize);
                let sum = dest.wrapping_add(src);
                write_rm_u32(cpu, bus, insn, sum)?;
                cpu.set_gpr_u32(m.reg as usize, dest);
                set_add_flags_u32(cpu, dest, src, sum);
            } else {
                let dest = read_rm_u16(cpu, bus, insn)?;
                let src = cpu.gpr_u16(m.reg as usize);
                let sum = dest.wrapping_add(src);
                write_rm_u16(cpu, bus, insn, sum)?;
                cpu.set_gpr_u16(m.reg as usize, dest);
                set_add_flags_u16(cpu, dest, src, sum);
            }
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xB0 | 0xB1 => {
            // CMPXCHG r/m, r — Spec: Intel SDM Vol. 2 "CMPXCHG":
            // `TEMP := DEST; IF accumulator = TEMP THEN ZF := 1; DEST := SRC
            // ELSE ZF := 0; accumulator := TEMP; DEST := TEMP`. The unequal
            // case still writes the destination back. ZF comes from the
            // comparison and CF/PF/AF/SF/OF follow the same `accumulator - TEMP`
            // subtraction, so the shared SUB flag helpers set all six.
            // Unsupported here: `LOCK` atomicity (single-processor model).
            let m = insn.modrm.ok_or(ExecError::Unsupported(insn.opcode))?;
            if insn.opcode == 0xB0 {
                let temp = read_rm_u8(cpu, bus, insn)?;
                let accumulator = cpu.al();
                let equal = accumulator == temp;
                let new_dest = if equal { read_reg_u8(cpu, m.reg) } else { temp };
                write_rm_u8(cpu, bus, insn, new_dest)?;
                if !equal {
                    cpu.set_al(temp);
                }
                set_sub_flags_u8(cpu, accumulator, temp, accumulator.wrapping_sub(temp));
            } else if opsz32(insn) {
                let temp = read_rm_u32(cpu, bus, insn)?;
                let accumulator = cpu.eax();
                let equal = accumulator == temp;
                let new_dest = if equal {
                    cpu.gpr_u32(m.reg as usize)
                } else {
                    temp
                };
                write_rm_u32(cpu, bus, insn, new_dest)?;
                if !equal {
                    cpu.set_eax(temp);
                }
                set_sub_flags_u32(cpu, accumulator, temp, accumulator.wrapping_sub(temp));
            } else {
                let temp = read_rm_u16(cpu, bus, insn)?;
                let accumulator = cpu.ax();
                let equal = accumulator == temp;
                let new_dest = if equal {
                    cpu.gpr_u16(m.reg as usize)
                } else {
                    temp
                };
                write_rm_u16(cpu, bus, insn, new_dest)?;
                if !equal {
                    cpu.set_ax(temp);
                }
                set_sub_flags_u16(cpu, accumulator, temp, accumulator.wrapping_sub(temp));
            }
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xC7 => {
            // Group 9 — Spec: Intel SDM Vol. 2 "CMPXCHG8B/CMPXCHG16B"; opcode
            // map Group 9 (`0F C7`). Implemented: /1 CMPXCHG8B m64.
            // `TEMP64 := DEST; IF EDX:EAX = TEMP64 THEN ZF := 1; DEST := ECX:EBX
            // ELSE ZF := 0; EDX:EAX := TEMP64; DEST := TEMP64`. Only ZF is
            // written. Register destination is `#UD`. LOCK may be decoded; this
            // single-processor model does not enforce multi-processor atomicity.
            // Unsupported here: other /r forms, CMPXCHG16B / REX.W.
            let m = insn.modrm.ok_or(ExecError::Unsupported(0xC7))?;
            if m.reg != 1 {
                return Err(ExecError::Unsupported(0xC7));
            }
            let temp = read_mem_u64(cpu, bus, insn)?;
            let edx_eax = (u64::from(cpu.gpr_u32(CpuState::RDX)) << 32)
                | u64::from(cpu.gpr_u32(CpuState::RAX));
            let equal = edx_eax == temp;
            let new_dest = if equal {
                (u64::from(cpu.gpr_u32(CpuState::RCX)) << 32)
                    | u64::from(cpu.gpr_u32(CpuState::RBX))
            } else {
                temp
            };
            // Memory write commits before register/flag updates so a write
            // fault leaves EDX:EAX and ZF unchanged (precise exceptions).
            write_mem_u64(cpu, bus, insn, new_dest)?;
            if !equal {
                cpu.set_gpr_u32(CpuState::RAX, temp as u32);
                cpu.set_gpr_u32(CpuState::RDX, (temp >> 32) as u32);
            }
            cpu.set_zf(equal);
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        0xAF => {
            // IMUL r16, r/m16 / IMUL r32, r/m32 — Spec: Intel SDM Vol. 2 "IMUL".
            // Dest = ModRM.reg := ModRM.reg * r/m (signed).
            // Unsupported here: REX.W r64 form; LOCK #UD.
            let m = insn.modrm.ok_or(ExecError::Unsupported(0xAF))?;
            if opsz32(insn) {
                let src = read_rm_u32(cpu, bus, insn)?;
                let dst = cpu.gpr_u32(m.reg as usize);
                let prod = i64::from(dst as i32).wrapping_mul(i64::from(src as i32));
                cpu.set_gpr_u32(m.reg as usize, prod as u32);
                set_imul_flags_i32(cpu, prod);
            } else {
                let src = read_rm_u16(cpu, bus, insn)?;
                let dst = cpu.gpr_u16(m.reg as usize);
                let prod = i32::from(dst as i16).wrapping_mul(i32::from(src as i16));
                cpu.set_gpr_u16(m.reg as usize, prod as u16);
                set_imul_flags_i16(cpu, prod);
            }
            set_current_ip(cpu, next_ip);
            Ok(())
        }
        op => Err(ExecError::Unsupported(op)),
    }
}

/// Execute a single instruction at CS:IP.
///
/// Services latched `#NMI` (vector 2, not gated by `IF`) before maskable IRQs,
/// then when `IF=1` services a latched/polled external IRQ before fetch/decode
/// so non-REP instructions are interruptible (REP also polls between iterations).
/// A successful instruction (including one whose architectural fault is entered)
/// retires one MOV/POP SS shadow boundary. A bounded nested-delivery failure does
/// not consume the shadow because no recoverable architectural boundary commits.
/// Spec: Intel SDM Vol. 3 §6.3.3 / §6.7 (NMI); §§6.8.1, 6.8.3.
pub fn step(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<(), ExecError> {
    // No caller-owned MMU: the TLB lives for exactly this instruction. §4.10.2.2
    // permits that ("Processors need not implement any TLBs"), so translation
    // stays correct, but software that edits a paging-structure entry without
    // invalidating will see the new entry rather than the stale translation
    // real hardware may keep. Use [`step_with_mmu`] to model that.
    let mut mmu = Mmu::new();
    step_with_mmu(cpu, bus, &mut mmu)
}

/// One instruction with a caller-owned [`Mmu`], so the TLB survives across
/// instructions and `INVLPG` / `MOV to CR3` are observable.
///
/// This is the entry point a machine integration should use: it is the only
/// way a guest that forgets an invalidation misbehaves here the way it would on
/// silicon (SDM Vol. 3 §4.10.2, §4.10.4.1).
pub fn step_with_mmu(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    mmu: &mut Mmu,
) -> Result<(), ExecError> {
    let mut paged = PagedBus::new(bus, mmu, cpu);
    step_paged(cpu, &mut paged)
}

fn step_paged(cpu: &mut CpuState, bus: &mut PagedBus<'_>) -> Result<(), ExecError> {
    // Platform `#NMI` outranks maskable IRQs and can wake HLT.
    if service_pending_nmi(cpu, bus)? {
        return Ok(());
    }
    // Per-instruction external IRQ poll (PIC stub via pending_irq / Bus).
    if service_pending_external_interrupt(cpu, bus)? {
        return Ok(());
    }
    if cpu.halted {
        return Ok(());
    }
    bus.arm_restart_point(cpu);
    let result = match step_inner(cpu, bus) {
        Err(ExecError::ArchFault { vector, error_code }) => {
            deliver_fault(cpu, bus, vector, error_code.map(u32::from))
        }
        Err(ExecError::PageFault { linear, error_code }) => {
            // A fault re-executes the instruction, so undo everything the
            // partially executed instruction committed (SDM Vol. 3 §6.5).
            if let Some(restart) = bus.take_restart_point() {
                *cpu = restart;
            }
            // SDM Vol. 3 §4.7: `CR2` receives the faulting linear address, and
            // it is loaded whether or not delivery itself then succeeds.
            cpu.cr2 = linear;
            deliver_fault(cpu, bus, VECTOR_PAGE_FAULT, Some(error_code))
        }
        other => other,
    };
    if result.is_ok() {
        cpu.retire_maskable_interrupt_shadow();
    }
    result
}

fn step_inner(cpu: &mut CpuState, bus: &mut dyn Bus) -> Result<(), ExecError> {
    let insn = fetch_decode(cpu, bus)?;
    let next_ip = next_ip_after(cpu, insn.length);
    let op = insn.opcode;

    if insn.two_byte {
        return step_two_byte(cpu, bus, &insn, next_ip);
    }

    match op {
        0x06 | 0x0E | 0x16 | 0x1E => {
            // PUSH ES / CS / SS / DS — Spec: Intel SDM Vol. 2 "PUSH"; Vol. 1
            // §6.2. The stack slot follows the operand-size attribute exactly
            // as the two-byte `PUSH FS`/`GS` forms do; the stack-pointer width
            // itself follows `SS.B`.
            let selector = match op {
                0x06 => cpu.es.selector,
                0x0E => cpu.cs.selector,
                0x16 => cpu.ss.selector,
                _ => cpu.ds.selector,
            };
            push_sreg(cpu, bus, selector, opsz32(&insn))?;
            set_current_ip(cpu, next_ip);
        }
        0x07 | 0x17 | 0x1F => {
            // POP ES / SS / DS — Spec: Intel SDM Vol. 2 "POP"; Vol. 3 §§3.5.1,
            // 5.4.1, 6.8.3. A 32-bit operand size releases a doubleword slot
            // and takes the selector from its low word. `POP CS` does not
            // exist, so `0x0F` is the two-byte escape rather than a POP.
            let sreg = match op {
                0x07 => 0,
                0x17 => 2,
                _ => 3,
            };
            pop_sreg(cpu, bus, sreg, opsz32(&insn))?;
            if op == 0x17 {
                cpu.arm_maskable_interrupt_shadow();
            }
            set_current_ip(cpu, next_ip);
        }
        0xF4 => {
            cpu.halted = true;
            set_current_ip(cpu, next_ip);
        }
        0xFA => {
            // CLI — Spec: Intel SDM Vol. 2 "CLI" Table 3-7; Vol. 3 §20.2.1.
            // Unsupported: VME/PVI (`CR4` reserved; CPUID clear) → no VIF path.
            require_iopl_for_cli_sti(cpu)?;
            cpu.set_interrupt_flag(false);
            set_current_ip(cpu, next_ip);
        }
        0xFB => {
            // STI — Spec: Intel SDM Vol. 2 "STI" Table 3-8; Vol. 3 §20.2.1.
            // Unsupported: VME/PVI VIF path; interrupt-shadow delay after STI.
            require_iopl_for_cli_sti(cpu)?;
            cpu.set_interrupt_flag(true);
            set_current_ip(cpu, next_ip);
        }
        0x90 => set_current_ip(cpu, next_ip),
        0x91..=0x97 => {
            // XCHG AX/EAX, r16/r32 — Spec: Intel SDM Vol. 2 "XCHG"; Ch. 2 (66H).
            // Opcode 90 is NOP (XCHG AX/EAX,AX/EAX). Unsupported: REX.W (XCHG RAX,r64).
            let idx = (op - 0x90) as usize;
            if opsz32(&insn) {
                let eax = cpu.eax();
                let other = cpu.gpr_u32(idx);
                cpu.set_eax(other);
                cpu.set_gpr_u32(idx, eax);
            } else {
                let ax = cpu.ax();
                let other = cpu.gpr_u16(idx);
                cpu.set_ax(other);
                cpu.set_gpr_u16(idx, ax);
            }
            set_current_ip(cpu, next_ip);
        }
        0x98 => {
            // CBW/CWDE — Spec: Intel SDM Vol. 2 "CBW/CWDE/CDQE"; Ch. 2 (66H).
            // Unsupported here: CDQE (REX.W).
            if opsz32(&insn) {
                // CWDE: sign-extend AX into EAX.
                let eax = cpu.ax() as i16 as i32 as u32;
                cpu.set_eax(eax);
            } else {
                // CBW: sign-extend AL into AX.
                let al = cpu.al() as i8 as i16 as u16;
                cpu.set_ax(al);
            }
            set_current_ip(cpu, next_ip);
        }
        0x99 => {
            // CWD/CDQ — Spec: Intel SDM Vol. 2 "CWD/CDQ/CQO"; Ch. 2 (66H).
            // Unsupported here: CQO (REX.W).
            if opsz32(&insn) {
                // CDQ: sign-extend EAX into EDX:EAX.
                let edx = if cpu.eax() & 0x8000_0000 != 0 {
                    0xFFFF_FFFFu32
                } else {
                    0
                };
                cpu.set_gpr_u32(CpuState::RDX, edx);
            } else {
                let dx = if cpu.ax() & 0x8000 != 0 { 0xFFFFu16 } else { 0 };
                cpu.set_gpr_u16(CpuState::RDX, dx);
            }
            set_current_ip(cpu, next_ip);
        }
        0xF5 => {
            let cf = cpu.rflags & 1 != 0;
            cpu.set_cf(!cf);
            set_current_ip(cpu, next_ip);
        }
        0xF8 => {
            // CLC — Spec: Intel SDM Vol. 2 "CLC".
            cpu.set_cf(false);
            set_current_ip(cpu, next_ip);
        }
        0xF9 => {
            // STC — Spec: Intel SDM Vol. 2 "STC".
            cpu.set_cf(true);
            set_current_ip(cpu, next_ip);
        }
        0xFC => {
            // CLD — Spec: Intel SDM Vol. 2 "CLD".
            cpu.set_direction_flag(false);
            set_current_ip(cpu, next_ip);
        }
        0xFD => {
            // STD — Spec: Intel SDM Vol. 2 "STD".
            cpu.set_direction_flag(true);
            set_current_ip(cpu, next_ip);
        }
        0xEC | 0xED => {
            // IN AL, DX / IN eAX, DX — Spec: Intel SDM Vol. 2 "IN—Input from
            // Port". `DX` supplies the full 16-bit port number in every mode.
            // Unsupported here: the protected-mode IOPL / TSS I/O-permission
            // `#GP(0)` (CPL 0 only, no TSS) and the VM86 I/O bitmap.
            let port = cpu.gpr_u16(CpuState::RDX);
            port_in_accumulator(cpu, bus, &insn, port, op == 0xEC)?;
            set_current_ip(cpu, next_ip);
        }
        0xEE | 0xEF => {
            // OUT DX, AL / OUT DX, eAX — Spec: Intel SDM Vol. 2 "OUT—Output to
            // Port".
            let port = cpu.gpr_u16(CpuState::RDX);
            port_out_accumulator(cpu, bus, &insn, port, op == 0xEE)?;
            set_current_ip(cpu, next_ip);
        }
        0xE4 | 0xE5 => {
            // IN AL, imm8 / IN eAX, imm8 — the imm8 is the port number, so only
            // ports 0x00–0xFF are reachable through this form.
            // Spec: Intel SDM Vol. 2 "IN—Input from Port".
            let port = insn.immediate as u16;
            port_in_accumulator(cpu, bus, &insn, port, op == 0xE4)?;
            set_current_ip(cpu, next_ip);
        }
        0xE6 | 0xE7 => {
            // OUT imm8, AL / OUT imm8, eAX — Spec: Intel SDM Vol. 2 "OUT".
            let port = insn.immediate as u16;
            port_out_accumulator(cpu, bus, &insn, port, op == 0xE6)?;
            set_current_ip(cpu, next_ip);
        }
        0xE0..=0xE2 => {
            // LOOPNE/LOOPE/LOOP rel8 — Spec: Intel SDM Vol. 2 "LOOP/LOOPcc".
            // Address-size selects CX (16) or ECX (32). Unsupported: asize 64 (RCX).
            let zf = cpu.rflags & (1 << 6) != 0;
            let take = if asize32(&insn) {
                let ecx = cpu.gpr_u32(CpuState::RCX).wrapping_sub(1);
                cpu.set_gpr_u32(CpuState::RCX, ecx);
                match op {
                    0xE0 => ecx != 0 && !zf, // LOOPNE / LOOPNZ
                    0xE1 => ecx != 0 && zf,  // LOOPE / LOOPZ
                    0xE2 => ecx != 0,        // LOOP
                    _ => unreachable!("matched 0xE0..=0xE2"),
                }
            } else {
                let cx = cpu.gpr_u16(CpuState::RCX).wrapping_sub(1);
                cpu.set_gpr_u16(CpuState::RCX, cx);
                match op {
                    0xE0 => cx != 0 && !zf,
                    0xE1 => cx != 0 && zf,
                    0xE2 => cx != 0,
                    _ => unreachable!("matched 0xE0..=0xE2"),
                }
            };
            if take {
                // LOOP takes a rel8; the target follows the operand size.
                set_current_ip(
                    cpu,
                    near_branch_target(next_ip, insn.immediate, opsz32(&insn)),
                );
            } else {
                set_current_ip(cpu, next_ip);
            }
        }
        0xE3 => {
            // JCXZ/JECXZ rel8 — Spec: Intel SDM Vol. 2 "JCXZ/JECXZ/JRCXZ".
            // Address-size selects CX (16) or ECX (32). Unsupported: JRCXZ (asize 64).
            let zero = if asize32(&insn) {
                cpu.gpr_u32(CpuState::RCX) == 0
            } else {
                cpu.gpr_u16(CpuState::RCX) == 0
            };
            if zero {
                set_current_ip(
                    cpu,
                    near_branch_target(next_ip, insn.immediate, opsz32(&insn)),
                );
            } else {
                set_current_ip(cpu, next_ip);
            }
        }
        0xEB => {
            // JMP short rel8 — Spec: Intel SDM Vol. 2 "JMP".
            set_current_ip(
                cpu,
                near_branch_target(next_ip, insn.immediate, opsz32(&insn)),
            );
        }
        0xE9 => {
            // JMP near rel16/rel32 — Spec: Intel SDM Vol. 2 "JMP"; Ch. 2 (66H).
            // Operand size 16 clears EIP[31:16]; CS.D=1 executes full EIP.
            set_current_ip(
                cpu,
                near_branch_target(next_ip, insn.immediate, opsz32(&insn)),
            );
        }
        0xEA => {
            // JMP far ptr16:16 / ptr16:32.
            // Spec: Intel SDM Vol. 2 "JMP"; Ch. 2 (66H); Vol. 3 §5.8.1 / §20.1.
            // Protected mode (VM=0) is bounded to GDT code / task targets.
            // Virtual-8086 / real-address: reload CS:IP; opsize-32 truncates the
            // offset to IP16 (Vol. 2 JMP real-address note). Stay VM=1 when set.
            let offset = if opsz32(&insn) {
                insn.immediate as u32
            } else {
                u32::from(insn.immediate as u16)
            };
            let selector = insn.displacement as u16;
            if cr0_pe(cpu) && !eflags_vm(cpu.rflags) {
                protected_far_jump(cpu, bus, offset, selector, next_ip)?;
            } else {
                // Real-address / VM86: code fetch still uses IP16; ptr16:32 truncated.
                cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                cpu.set_ip16(offset as u16);
            }
        }
        0xE8 => {
            // CALL near rel16/rel32 — Spec: Intel SDM Vol. 2 "CALL"; Ch. 2 (66H).
            // Opsize 32 pushes a 32-bit return EIP; opsize 16 pushes IP and
            // clears EIP[31:16] of the target.
            if opsz32(&insn) {
                push32(cpu, bus, next_ip)?;
            } else {
                push16(cpu, bus, next_ip as u16)?;
            }
            set_current_ip(
                cpu,
                near_branch_target(next_ip, insn.immediate, opsz32(&insn)),
            );
        }
        0xC2 => {
            // RET iw — near return with stack release.
            // Spec: Intel SDM Vol. 2 "RET" (near, imm16). Imm16 release always;
            // opsize selects pop IP16 vs EIP32.
            let release = insn.immediate as u16;
            let target = if opsz32(&insn) {
                pop32(cpu, bus)?
            } else {
                u32::from(pop16(cpu, bus)?)
            };
            stack_release(cpu, release);
            set_current_ip(cpu, near_absolute_target(target, opsz32(&insn)));
        }
        0xC3 => {
            // RET near — Spec: Intel SDM Vol. 2 "RET".
            let target = if opsz32(&insn) {
                pop32(cpu, bus)?
            } else {
                u32::from(pop16(cpu, bus)?)
            };
            set_current_ip(cpu, near_absolute_target(target, opsz32(&insn)));
        }
        0xC4 => {
            // LES r16/r32, m16:16/m16:32 — load offset into r and selector into ES.
            // In protected mode, validate/load ES through the shared DS/ES
            // descriptor path only after the complete pointer is readable.
            // Spec: Intel SDM Vol. 2 "LES" (Operation, Protected Mode
            // Exceptions); Vol. 3 §§3.4.2–3.4.5, 5.3–5.6.
            // Register form (mod=11) → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.mod_ == 3 {
                return real_mode_ud(cpu, bus);
            }
            // Protected (`PE=1`, `VM=0`) uses GDT; real-address / VM86 use
            // `selector << 4` (Vol. 3 §20.1.1).
            let use_gdt = cr0_pe(cpu) && !eflags_vm(cpu.rflags);
            if opsz32(&insn) {
                let (offset, selector) = read_far_ptr32(cpu, bus, &insn)?;
                let protected_es = if use_gdt {
                    Some(prepare_data_sreg_from_gdt(cpu, bus, selector)?)
                } else {
                    None
                };
                // All fallible pointer/descriptor work precedes this commit.
                cpu.set_gpr_u32(m.reg as usize, offset);
                if let Some(loaded) = protected_es {
                    cpu.es = loaded;
                } else {
                    cpu.es.load_real_mode_selector(selector);
                }
            } else {
                let (offset, selector) = read_far_ptr16(cpu, bus, &insn)?;
                let protected_es = if use_gdt {
                    Some(prepare_data_sreg_from_gdt(cpu, bus, selector)?)
                } else {
                    None
                };
                // All fallible pointer/descriptor work precedes this commit.
                cpu.set_gpr_u16(m.reg as usize, offset);
                if let Some(loaded) = protected_es {
                    cpu.es = loaded;
                } else {
                    cpu.es.load_real_mode_selector(selector);
                }
            }
            set_current_ip(cpu, next_ip);
        }
        0xC5 => {
            // LDS r16/r32, m16:16/m16:32 — load offset into r and selector into DS.
            // In protected mode, validate/load DS through the shared DS/ES
            // descriptor path only after the complete pointer is readable.
            // Spec: Intel SDM Vol. 2 "LDS" (Operation, Protected Mode
            // Exceptions); Vol. 3 §§3.4.2–3.4.5, 5.3–5.6, 20.1.1.
            // Register form (mod=11) → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.mod_ == 3 {
                return real_mode_ud(cpu, bus);
            }
            let use_gdt = cr0_pe(cpu) && !eflags_vm(cpu.rflags);
            if opsz32(&insn) {
                let (offset, selector) = read_far_ptr32(cpu, bus, &insn)?;
                let protected_ds = if use_gdt {
                    Some(prepare_data_sreg_from_gdt(cpu, bus, selector)?)
                } else {
                    None
                };
                // All fallible pointer/descriptor work precedes this commit.
                cpu.set_gpr_u32(m.reg as usize, offset);
                if let Some(loaded) = protected_ds {
                    cpu.ds = loaded;
                } else {
                    cpu.ds.load_real_mode_selector(selector);
                }
            } else {
                let (offset, selector) = read_far_ptr16(cpu, bus, &insn)?;
                let protected_ds = if use_gdt {
                    Some(prepare_data_sreg_from_gdt(cpu, bus, selector)?)
                } else {
                    None
                };
                // All fallible pointer/descriptor work precedes this commit.
                cpu.set_gpr_u16(m.reg as usize, offset);
                if let Some(loaded) = protected_ds {
                    cpu.ds = loaded;
                } else {
                    cpu.ds.load_real_mode_selector(selector);
                }
            }
            set_current_ip(cpu, next_ip);
        }
        0x9A => {
            // CALL far ptr16:16 / ptr16:32.
            // Spec: Intel SDM Vol. 2 "CALL"; Ch. 2 (66H); Vol. 3 §5.8.1 / §20.1.
            // Protected (VM=0): same-CPL GDT code / call gate / task.
            // Virtual-8086 / real-address: push CS:IP (opsize-32 → EIP32 then
            // CS16 = 6-byte frame); truncate target offset to IP16; stay VM=1.
            // Unsupported from VM86: privilege-changing call gates.
            let selector = insn.displacement as u16;
            let offset = if opsz32(&insn) {
                insn.immediate as u32
            } else {
                u32::from(insn.immediate as u16)
            };
            if cr0_pe(cpu) && !eflags_vm(cpu.rflags) {
                protected_far_call(cpu, bus, offset, selector, next_ip, opsz32(&insn))?;
            } else if opsz32(&insn) {
                push16(cpu, bus, cpu.cs.selector)?;
                push32(cpu, bus, next_ip)?;
                cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                cpu.set_ip16(offset as u16);
            } else {
                push16(cpu, bus, cpu.cs.selector)?;
                push16(cpu, bus, next_ip as u16)?;
                cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                cpu.set_ip16(offset as u16);
            }
        }
        0xCA => {
            // RETF iw — far return with stack release.
            // Spec: Intel SDM Vol. 2 "RET" (far, imm16); Ch. 2 (66H); §20.1.
            // Opsize 32: pop EIP32 then CS16 (truncate EIP→IP16); Imm16 release.
            // Real-address / VM86 path (protected privilege-changing RETF out).
            let release = insn.immediate as u16;
            if opsz32(&insn) {
                let eip = pop32(cpu, bus)?;
                let cs_sel = pop16(cpu, bus)?;
                stack_release(cpu, release);
                cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
                cpu.set_ip16(eip as u16);
            } else {
                let ip = pop16(cpu, bus)?;
                let cs_sel = pop16(cpu, bus)?;
                stack_release(cpu, release);
                cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
                cpu.set_ip16(ip);
            }
        }
        0xCB => {
            // RETF — far return.
            // Spec: Intel SDM Vol. 2 "RET" (far); Ch. 2 (66H); Vol. 3 §20.1.
            // Opsize 16: pop IP then CS; opsize 32: pop EIP then CS (6-byte
            // frame, EIP truncated to IP16). VM86 stays in VM86.
            if opsz32(&insn) {
                let eip = pop32(cpu, bus)?;
                let cs_sel = pop16(cpu, bus)?;
                cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
                cpu.set_ip16(eip as u16);
            } else {
                let ip = pop16(cpu, bus)?;
                let cs_sel = pop16(cpu, bus)?;
                cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
                cpu.set_ip16(ip);
            }
        }
        0xC8 => {
            // ENTER/ENTERD iw, ib — nesting level = imm8 mod 32.
            // Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §§6.5, 6.2.2; Ch. 2 (66H).
            // The operand size selects the pushed width and the BP/EBP frame
            // register; the stack pointer follows `SS.B` (the `0x67`
            // address-size prefix does not change the stack address size).
            // Unsupported here: 64-bit stacks (RBP/RSP).
            let alloc = insn.immediate as u16;
            let nesting = (insn.displacement as u8) & 0x1F;
            let op32 = opsz32(&insn);
            if op32 {
                push32(cpu, bus, cpu.gpr_u32(CpuState::RBP))?;
            } else {
                push16(cpu, bus, cpu.gpr_u16(CpuState::RBP))?;
            }
            let frame_temp = stack_pointer(cpu);
            if nesting > 0 {
                // Copy nesting-1 display pointers from the caller's frame, then
                // push frame_temp (current procedure's frame pointer for LEAVE).
                let step = if op32 { 4u64 } else { 2 };
                for _ in 1..nesting {
                    let bp = if op32 {
                        let bp = cpu.gpr_u32(CpuState::RBP).wrapping_sub(step as u32);
                        cpu.set_gpr_u32(CpuState::RBP, bp);
                        u64::from(bp)
                    } else {
                        let bp = cpu.gpr_u16(CpuState::RBP).wrapping_sub(step as u16);
                        cpu.set_gpr_u16(CpuState::RBP, bp);
                        u64::from(bp)
                    };
                    let addr = seg_linear_checked(&cpu.ss, bp, step, true)?;
                    if op32 {
                        let display = bus
                            .read_u32(addr)
                            .map_err(|e| classify_mem_fault(e, true))?;
                        push32(cpu, bus, display)?;
                    } else {
                        let display = bus
                            .read_u16(addr)
                            .map_err(|e| classify_mem_fault(e, true))?;
                        push16(cpu, bus, display)?;
                    }
                }
                if op32 {
                    push32(cpu, bus, frame_temp)?;
                } else {
                    push16(cpu, bus, frame_temp as u16)?;
                }
            }
            if op32 {
                cpu.set_gpr_u32(CpuState::RBP, frame_temp);
            } else {
                cpu.set_gpr_u16(CpuState::RBP, frame_temp as u16);
            }
            let sp = stack_step(cpu, stack_pointer(cpu), -i32::from(alloc));
            set_stack_pointer(cpu, sp);
            set_current_ip(cpu, next_ip);
        }
        0xC9 => {
            // LEAVE — Spec: Intel SDM Vol. 2 "LEAVE"; Vol. 1 §6.2.2; Ch. 2 (66H).
            // `SS.B` selects `ESP ← EBP` vs `SP ← BP`; the operand size selects
            // the popped frame-pointer width. Unsupported: 64-bit stacks.
            let bp = if stack_addr_size_32(cpu) {
                cpu.gpr_u32(CpuState::RBP)
            } else {
                u32::from(cpu.gpr_u16(CpuState::RBP))
            };
            set_stack_pointer(cpu, bp);
            if opsz32(&insn) {
                let v = pop32(cpu, bus)?;
                cpu.set_gpr_u32(CpuState::RBP, v);
            } else {
                let v = pop16(cpu, bus)?;
                cpu.set_gpr_u16(CpuState::RBP, v);
            }
            set_current_ip(cpu, next_ip);
        }
        0x9C => {
            // PUSHF/PUSHFD — Spec: Intel SDM Vol. 2 "PUSHF/PUSHFD/PUSHFQ".
            // VM86 without VME: IOPL < 3 → #GP(0) (Vol. 3 §20.2.2).
            // Unsupported: PUSHFQ; VME VIP/VIF push masking.
            if cr0_pe(cpu) && eflags_vm(cpu.rflags) && eflags_iopl(cpu.rflags) < 3 {
                return Err(arch_fault_with_error_code(13, 0));
            }
            if opsz32(&insn) {
                push32(cpu, bus, cpu.rflags as u32)?;
            } else {
                push16(cpu, bus, cpu.rflags as u16)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x9D => {
            // POPF/POPFD — Spec: Intel SDM Vol. 2 "POPF/POPFD/POPFQ"; Vol. 3
            // §20.2.2. Privilege masks IOPL/IF; VM/RF never loaded from image.
            // Unsupported: POPFQ; VME 16-bit POPF with IOPL<3.
            popf_execute(cpu, bus, opsz32(&insn))?;
            set_current_ip(cpu, next_ip);
        }
        0x9E => {
            // SAHF — load SF,ZF,AF,PF,CF from AH. Spec: Intel SDM Vol. 2 "SAHF".
            // Unsupported here: none for real-mode 16-bit; OF unaffected.
            let ah = cpu.ah();
            cpu.set_cf(ah & 1 != 0);
            cpu.set_pf(ah & (1 << 2) != 0);
            cpu.set_af(ah & (1 << 4) != 0);
            cpu.set_zf(ah & (1 << 6) != 0);
            cpu.set_sf(ah & (1 << 7) != 0);
            set_current_ip(cpu, next_ip);
        }
        0x9F => {
            // LAHF — AH = SF:ZF:0:AF:0:PF:1:CF. Spec: Intel SDM Vol. 2 "LAHF".
            let mut ah = 1u8 << 1; // reserved bit 1 always set in the transferred image
            if cpu.rflags & 1 != 0 {
                ah |= 1;
            }
            if cpu.rflags & (1 << 2) != 0 {
                ah |= 1 << 2;
            }
            if cpu.rflags & (1 << 4) != 0 {
                ah |= 1 << 4;
            }
            if cpu.rflags & (1 << 6) != 0 {
                ah |= 1 << 6;
            }
            if cpu.rflags & (1 << 7) != 0 {
                ah |= 1 << 7;
            }
            cpu.set_ah(ah);
            set_current_ip(cpu, next_ip);
        }
        0xCC => {
            // INT3 — one-byte breakpoint; saved return IP is the following byte.
            // Spec: Intel SDM Vol. 2 "INT3"; Vol. 3 §§6.4, 6.12.1, 20.2.2.
            // Not IOPL-sensitive in VM86 (Table 20-1). Uses the VM86→CPL0
            // 9-dword frame when VM=1. Unsupported: ICEBP/INT1 (`F1`) — remains
            // a sparse-table decode miss (not silent #DB).
            deliver_software_interrupt(cpu, bus, 3, next_ip)?;
        }
        0xCD => {
            // INT imm8 — saved return IP is the following instruction.
            // Spec: Intel SDM Vol. 2 "INT n"; Vol. 3 §§6.4, 6.12.1, 20.2.2.
            // VM86 without VME: IOPL < 3 → #GP(0).
            require_vm86_iopl_for_soft_int(cpu)?;
            deliver_software_interrupt(cpu, bus, insn.immediate as u8, next_ip)?;
        }
        0xCE => {
            // INTO — if OF=1, #OF (vector 4) trap; else fall through.
            // Spec: Intel SDM Vol. 2 "INT n/INTO/INT3/INT1";
            // Vol. 3 §§6.12.1, 6.15 (#OF — trap), 20.2.2.
            // INTO is not IOPL-sensitive in VM86 (unlike INT n).
            // Saved IP is the following instruction (trap class).
            // Unsupported here: 64-bit mode (#UD); VME redirect.
            if cpu.rflags & (1 << 11) != 0 {
                deliver_software_interrupt(cpu, bus, 4, next_ip)?;
            } else {
                set_current_ip(cpu, next_ip);
            }
        }
        0xCF => {
            // IRET/IRETD — Spec: Intel SDM Vol. 2 "IRET/IRETD/IRETQ".
            // The effective operand size selects the 6-byte or 12-byte frame.
            if cr0_pe(cpu) {
                if eflags_vm(cpu.rflags) {
                    vm86_iret(cpu, bus, opsz32(&insn))?;
                } else {
                    protected_iret(cpu, bus, opsz32(&insn), next_ip)?;
                }
            } else {
                // Preserve the existing real-address 16-bit stack-frame path.
                // Real-address `IRETD` (`0x66 CF`, 12-byte frame) is not modeled.
                let ip = pop16(cpu, bus)?;
                let cs_sel = pop16(cpu, bus)?;
                let flags = pop16(cpu, bus)?;
                cpu.cs = x86_core::SegmentReg::real_mode_code(cs_sel);
                cpu.set_ip16(ip);
                // Preserve high RFLAGS; bit 1 of FLAGS is reserved-1.
                cpu.rflags = (cpu.rflags & !0xFFFF) | u64::from(flags) | 2;
            }
        }
        0xD4 => {
            // AAM — ASCII Adjust AX After Multiply. Spec: Intel SDM Vol. 2 "AAM".
            // imm8=0 → #DE (Vol. 3 §6.15). OF/AF/CF undefined (left unchanged).
            // Unsupported here: 64-bit mode (#UD).
            let base = insn.immediate as u8;
            if base == 0 {
                return real_mode_exception(cpu, bus, 0);
            }
            let temp_al = cpu.al();
            cpu.set_ah(temp_al / base);
            let al = temp_al % base;
            cpu.set_al(al);
            set_bcd_szp_flags_u8(cpu, al);
            set_current_ip(cpu, next_ip);
        }
        0xD5 => {
            // AAD — ASCII Adjust AX Before Division. Spec: Intel SDM Vol. 2 "AAD".
            // OF/AF/CF undefined (left unchanged). Unsupported here: 64-bit mode (#UD).
            let base = insn.immediate as u8;
            let temp_al = cpu.al();
            let temp_ah = cpu.ah();
            let al = temp_al.wrapping_add(temp_ah.wrapping_mul(base));
            cpu.set_al(al);
            cpu.set_ah(0);
            set_bcd_szp_flags_u8(cpu, al);
            set_current_ip(cpu, next_ip);
        }
        0xD7 => {
            // XLAT/XLATB — AL ← [rBX + AL] (segment overrideable).
            // Spec: Intel SDM Vol. 2 "XLAT/XLATB"; Vol. 1 §3.6 (address-size).
            // Address-size 16 → BX; 0x67 → EBX. Opsize does not apply. Unsupported: asize 64.
            let off = if asize32(&insn) {
                u64::from(cpu.gpr_u32(CpuState::RBX).wrapping_add(u32::from(cpu.al())))
            } else {
                u64::from(cpu.gpr_u16(CpuState::RBX).wrapping_add(u16::from(cpu.al())))
            };
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = matches!(insn.prefixes.segment_override, Some(0x36));
            let addr = seg_linear_checked(seg, off, 1, uses_ss)?;
            let v = bus
                .read_u8(addr)
                .map_err(|e| classify_mem_fault(e, uses_ss))?;
            cpu.set_al(v);
            set_current_ip(cpu, next_ip);
        }
        0xD0 => {
            // Group 2 r/m8, 1 — Spec: Intel SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR.
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            let v = read_rm_u8(cpu, bus, &insn)?;
            let r = grp2_u8(cpu, m.reg, v, 1)?;
            write_rm_u8(cpu, bus, &insn, r)?;
            set_current_ip(cpu, next_ip);
        }
        0xD1 => {
            // Group 2 r/m16|32, 1 — Spec: Intel SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR; Ch. 2.
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                let r = grp2_u32(cpu, m.reg, v, 1)?;
                write_rm_u32(cpu, bus, &insn, r)?;
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                let r = grp2_u16(cpu, m.reg, v, 1)?;
                write_rm_u16(cpu, bus, &insn, r)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x80 => {
            // Group 1 r/m8, imm8 — Spec: Intel SDM Vol. 2 opcode map / ADD…CMP.
            // Unsupported here: opcode 82 alias; LOCK.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = insn.immediate as u8;
            if let Some(r) = grp1_u8(cpu, m.reg, a, b)? {
                write_rm_u8(cpu, bus, &insn, r)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x81 => {
            // Group 1 r/m16|32, imm16|32 — Spec: Intel SDM Vol. 2; Ch. 2 (66H).
            // Unsupported here: LOCK.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = insn.immediate as u32;
                if let Some(r) = grp1_u32(cpu, m.reg, a, b)? {
                    write_rm_u32(cpu, bus, &insn, r)?;
                }
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = insn.immediate as u16;
                if let Some(r) = grp1_u16(cpu, m.reg, a, b)? {
                    write_rm_u16(cpu, bus, &insn, r)?;
                }
            }
            set_current_ip(cpu, next_ip);
        }
        0x83 => {
            // Group 1 r/m16|32, imm8 (sign-extended) — Spec: Intel SDM Vol. 2; Ch. 2.
            // Unsupported here: LOCK.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = insn.immediate as i8 as i32 as u32;
                if let Some(r) = grp1_u32(cpu, m.reg, a, b)? {
                    write_rm_u32(cpu, bus, &insn, r)?;
                }
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = insn.immediate as i8 as i16 as u16;
                if let Some(r) = grp1_u16(cpu, m.reg, a, b)? {
                    write_rm_u16(cpu, bus, &insn, r)?;
                }
            }
            set_current_ip(cpu, next_ip);
        }
        0xC0 => {
            // Group 2 r/m8, imm8 — Spec: Intel SDM Vol. 2 (COUNT masked to 5 bits).
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            let v = read_rm_u8(cpu, bus, &insn)?;
            let r = grp2_u8(cpu, m.reg, v, insn.immediate as u8)?;
            write_rm_u8(cpu, bus, &insn, r)?;
            set_current_ip(cpu, next_ip);
        }
        0xC1 => {
            // Group 2 r/m16|32, imm8 — Spec: Intel SDM Vol. 2 (COUNT masked to 5 bits); Ch. 2.
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                let r = grp2_u32(cpu, m.reg, v, insn.immediate as u8)?;
                write_rm_u32(cpu, bus, &insn, r)?;
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                let r = grp2_u16(cpu, m.reg, v, insn.immediate as u8)?;
                write_rm_u16(cpu, bus, &insn, r)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0xD2 => {
            // Group 2 r/m8, CL — Spec: Intel SDM Vol. 2 (COUNT = CL, masked to 5 bits).
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            let v = read_rm_u8(cpu, bus, &insn)?;
            let r = grp2_u8(cpu, m.reg, v, cpu.gpr_u8_low(CpuState::RCX))?;
            write_rm_u8(cpu, bus, &insn, r)?;
            set_current_ip(cpu, next_ip);
        }
        0xD3 => {
            // Group 2 r/m16|32, CL — Spec: Intel SDM Vol. 2 (COUNT = CL, masked to 5 bits); Ch. 2.
            // /6 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg == 6 {
                return real_mode_ud(cpu, bus);
            }
            let count = cpu.gpr_u8_low(CpuState::RCX);
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                let r = grp2_u32(cpu, m.reg, v, count)?;
                write_rm_u32(cpu, bus, &insn, r)?;
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                let r = grp2_u16(cpu, m.reg, v, count)?;
                write_rm_u16(cpu, bus, &insn, r)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0xF6 => {
            // Group 3 r/m8 — TEST/NOT/NEG/MUL/IMUL/DIV/IDIV (/0–/7).
            // Spec: Intel SDM Vol. 2 "TEST"/"NOT"/"NEG"/"MUL"/"IMUL"/"DIV"/"IDIV"; opcode map Group 3.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u8(cpu, bus, &insn)?;
            match m.reg {
                0 | 1 => {
                    // TEST r/m8, imm8 — AND; result discarded. Flags like AND.
                    set_logic_flags_u8(cpu, v & (insn.immediate as u8));
                }
                2 => {
                    // NOT — one's complement; flags unaffected.
                    write_rm_u8(cpu, bus, &insn, !v)?;
                }
                3 => {
                    // NEG — two's complement; flags as SUB from 0 (CF cleared iff operand was 0).
                    let r = v.wrapping_neg();
                    write_rm_u8(cpu, bus, &insn, r)?;
                    set_sub_flags_u8(cpu, 0, v, r);
                }
                4 => {
                    // MUL r/m8 — AX = AL * r/m8. CF=OF=1 iff AH != 0; SF/ZF/AF/PF undefined.
                    let prod = u16::from(cpu.al()).wrapping_mul(u16::from(v));
                    cpu.set_ax(prod);
                    let hi_nz = (prod >> 8) != 0;
                    cpu.set_cf(hi_nz);
                    cpu.set_of(hi_nz);
                }
                5 => {
                    // IMUL r/m8 — AX = AL * r/m8 (signed). CF=OF=1 iff result not in AL.
                    let prod = i16::from(cpu.al() as i8).wrapping_mul(i16::from(v as i8));
                    cpu.set_ax(prod as u16);
                    let fits = prod == i16::from(prod as i8);
                    cpu.set_cf(!fits);
                    cpu.set_of(!fits);
                }
                6 => {
                    // DIV r/m8 — AX / r/m8 → AL=quot, AH=rem. #DE if divisor=0 or quot>0xFF.
                    // Spec: Intel SDM Vol. 2 "DIV"; Vol. 3 §6.15 (#DE). Faulting IP = insn start.
                    if v == 0 {
                        return real_mode_exception(cpu, bus, 0);
                    }
                    let dividend = u32::from(cpu.ax());
                    let quot = dividend / u32::from(v);
                    let rem = dividend % u32::from(v);
                    if quot > 0xFF {
                        return real_mode_exception(cpu, bus, 0);
                    }
                    cpu.set_ax(((rem as u16) << 8) | (quot as u16));
                }
                7 => {
                    // IDIV r/m8 — signed AX / r/m8 → AL=quot, AH=rem. #DE on 0 or quot∉i8.
                    if v == 0 {
                        return real_mode_exception(cpu, bus, 0);
                    }
                    let dividend = cpu.ax() as i16;
                    let divisor = i16::from(v as i8);
                    let Some(quot) = dividend.checked_div(divisor) else {
                        return real_mode_exception(cpu, bus, 0);
                    };
                    if !(i16::from(i8::MIN)..=i16::from(i8::MAX)).contains(&quot) {
                        return real_mode_exception(cpu, bus, 0);
                    }
                    // Safe: checked_div already rejected i16::MIN / -1.
                    let rem = dividend.wrapping_rem(divisor);
                    cpu.set_ax(((rem as u16) << 8) | ((quot as u8) as u16));
                }
                _ => return Err(ExecError::Unsupported(op)),
            }
            set_current_ip(cpu, next_ip);
        }
        0xF7 => {
            // Group 3 r/m16|32 — TEST/NOT/NEG/MUL/IMUL/DIV/IDIV (/0–/7).
            // Spec: Intel SDM Vol. 2 "TEST"/"NOT"/"NEG"/"MUL"/"IMUL"/"DIV"/"IDIV"; Ch. 2 (66H).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                match m.reg {
                    0 | 1 => {
                        set_logic_flags_u32(cpu, v & (insn.immediate as u32));
                    }
                    2 => {
                        write_rm_u32(cpu, bus, &insn, !v)?;
                    }
                    3 => {
                        let r = v.wrapping_neg();
                        write_rm_u32(cpu, bus, &insn, r)?;
                        set_sub_flags_u32(cpu, 0, v, r);
                    }
                    4 => {
                        // MUL r/m32 — EDX:EAX = EAX * r/m32. CF=OF=1 iff EDX != 0.
                        let prod = u64::from(cpu.eax()).wrapping_mul(u64::from(v));
                        cpu.set_eax(prod as u32);
                        cpu.set_gpr_u32(CpuState::RDX, (prod >> 32) as u32);
                        let hi_nz = (prod >> 32) != 0;
                        cpu.set_cf(hi_nz);
                        cpu.set_of(hi_nz);
                    }
                    5 => {
                        // IMUL r/m32 — EDX:EAX = EAX * r/m32 (signed). CF=OF=1 iff not in EAX.
                        let prod = i64::from(cpu.eax() as i32).wrapping_mul(i64::from(v as i32));
                        cpu.set_eax(prod as u32);
                        cpu.set_gpr_u32(CpuState::RDX, (prod >> 32) as u32);
                        set_imul_flags_i32(cpu, prod);
                    }
                    6 => {
                        // DIV r/m32 — EDX:EAX / r/m32 → EAX=quot, EDX=rem. #DE on 0 or quot>u32::MAX.
                        if v == 0 {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let dividend =
                            (u64::from(cpu.gpr_u32(CpuState::RDX)) << 32) | u64::from(cpu.eax());
                        let quot = dividend / u64::from(v);
                        let rem = dividend % u64::from(v);
                        if quot > u64::from(u32::MAX) {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        cpu.set_eax(quot as u32);
                        cpu.set_gpr_u32(CpuState::RDX, rem as u32);
                    }
                    7 => {
                        // IDIV r/m32 — signed EDX:EAX / r/m32 → EAX=quot, EDX=rem.
                        if v == 0 {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let dividend = ((u64::from(cpu.gpr_u32(CpuState::RDX)) << 32)
                            | u64::from(cpu.eax())) as i64;
                        let divisor = i64::from(v as i32);
                        let Some(quot) = dividend.checked_div(divisor) else {
                            return real_mode_exception(cpu, bus, 0);
                        };
                        if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&quot) {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let rem = dividend.wrapping_rem(divisor);
                        cpu.set_eax(quot as u32);
                        cpu.set_gpr_u32(CpuState::RDX, rem as u32);
                    }
                    _ => return Err(ExecError::Unsupported(op)),
                }
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                match m.reg {
                    0 | 1 => {
                        set_logic_flags_u16(cpu, v & (insn.immediate as u16));
                    }
                    2 => {
                        write_rm_u16(cpu, bus, &insn, !v)?;
                    }
                    3 => {
                        let r = v.wrapping_neg();
                        write_rm_u16(cpu, bus, &insn, r)?;
                        set_sub_flags_u16(cpu, 0, v, r);
                    }
                    4 => {
                        let prod = u32::from(cpu.ax()).wrapping_mul(u32::from(v));
                        cpu.set_ax(prod as u16);
                        cpu.set_gpr_u16(CpuState::RDX, (prod >> 16) as u16);
                        let hi_nz = (prod >> 16) != 0;
                        cpu.set_cf(hi_nz);
                        cpu.set_of(hi_nz);
                    }
                    5 => {
                        let prod = i32::from(cpu.ax() as i16).wrapping_mul(i32::from(v as i16));
                        cpu.set_ax(prod as u16);
                        cpu.set_gpr_u16(CpuState::RDX, (prod >> 16) as u16);
                        set_imul_flags_i16(cpu, prod);
                    }
                    6 => {
                        if v == 0 {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let dividend =
                            (u32::from(cpu.gpr_u16(CpuState::RDX)) << 16) | u32::from(cpu.ax());
                        let quot = dividend / u32::from(v);
                        let rem = dividend % u32::from(v);
                        if quot > 0xFFFF {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        cpu.set_ax(quot as u16);
                        cpu.set_gpr_u16(CpuState::RDX, rem as u16);
                    }
                    7 => {
                        if v == 0 {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let dividend = ((u32::from(cpu.gpr_u16(CpuState::RDX)) << 16)
                            | u32::from(cpu.ax())) as i32;
                        let divisor = i32::from(v as i16);
                        let Some(quot) = dividend.checked_div(divisor) else {
                            return real_mode_exception(cpu, bus, 0);
                        };
                        if !(i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&quot) {
                            return real_mode_exception(cpu, bus, 0);
                        }
                        let rem = dividend.wrapping_rem(divisor);
                        cpu.set_ax(quot as u16);
                        cpu.set_gpr_u16(CpuState::RDX, rem as u16);
                    }
                    _ => return Err(ExecError::Unsupported(op)),
                }
            }
            set_current_ip(cpu, next_ip);
        }
        0xFE => {
            // Group 4 r/m8 — INC (/0) / DEC (/1). Spec: Intel SDM Vol. 2 "INC"/"DEC".
            // /2–/7 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg > 1 {
                return real_mode_ud(cpu, bus);
            }
            let v = read_rm_u8(cpu, bus, &insn)?;
            let saved_cf = cpu.rflags & 1 != 0;
            if m.reg == 0 {
                let r = v.wrapping_add(1);
                write_rm_u8(cpu, bus, &insn, r)?;
                set_add_flags_u8(cpu, v, 1, r);
            } else {
                let r = v.wrapping_sub(1);
                write_rm_u8(cpu, bus, &insn, r)?;
                set_sub_flags_u8(cpu, v, 1, r);
            }
            // INC/DEC do not modify CF (Intel SDM Vol. 2, INC/DEC).
            cpu.set_cf(saved_cf);
            set_current_ip(cpu, next_ip);
        }
        0xFF => {
            // Group 5 r/m16|32 — INC/DEC/CALL/JMP/PUSH.
            // Spec: Intel SDM Vol. 2 "INC"/"DEC"/"CALL"/"JMP"/"PUSH"; opcode map Group 5;
            // Ch. 2 (66H). /7 reserved and far CALL/JMP register forms → #UD (Vol. 3 §6.15).
            // Protected-mode far CALL/JMP are same-CPL GDT code only (no call gates).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let op32 = opsz32(&insn);
            match m.reg {
                0 | 1 => {
                    let saved_cf = cpu.rflags & 1 != 0;
                    if op32 {
                        let v = read_rm_u32(cpu, bus, &insn)?;
                        if m.reg == 0 {
                            let r = v.wrapping_add(1);
                            write_rm_u32(cpu, bus, &insn, r)?;
                            set_add_flags_u32(cpu, v, 1, r);
                        } else {
                            let r = v.wrapping_sub(1);
                            write_rm_u32(cpu, bus, &insn, r)?;
                            set_sub_flags_u32(cpu, v, 1, r);
                        }
                    } else {
                        let v = read_rm_u16(cpu, bus, &insn)?;
                        if m.reg == 0 {
                            let r = v.wrapping_add(1);
                            write_rm_u16(cpu, bus, &insn, r)?;
                            set_add_flags_u16(cpu, v, 1, r);
                        } else {
                            let r = v.wrapping_sub(1);
                            write_rm_u16(cpu, bus, &insn, r)?;
                            set_sub_flags_u16(cpu, v, 1, r);
                        }
                    }
                    // INC/DEC do not modify CF (Intel SDM Vol. 2, INC/DEC).
                    cpu.set_cf(saved_cf);
                    set_current_ip(cpu, next_ip);
                }
                2 => {
                    // CALL r/m16|32 near absolute indirect.
                    let target = if op32 {
                        let target = read_rm_u32(cpu, bus, &insn)?;
                        push32(cpu, bus, next_ip)?;
                        target
                    } else {
                        let target = read_rm_u16(cpu, bus, &insn)?;
                        push16(cpu, bus, next_ip as u16)?;
                        u32::from(target)
                    };
                    set_current_ip(cpu, near_absolute_target(target, op32));
                }
                3 => {
                    // CALL FAR m16:16 / m16:32 — absolute indirect far (memory only).
                    // Spec: Intel SDM Vol. 2 "CALL"; opcode map Group 5 /3; Ch. 2 (66H).
                    // Register form is invalid (#UD). Protected: same-CPL GDT code.
                    if m.mod_ == 3 {
                        return real_mode_ud(cpu, bus);
                    }
                    let (offset, selector) = if op32 {
                        let (addr, _, uses_ss) = ea(cpu, &insn, 6)?;
                        let offset = bus
                            .read_u32(addr)
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        let selector = bus
                            .read_u16(addr.wrapping_add(4))
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        (offset, selector)
                    } else {
                        let (addr, _, uses_ss) = ea(cpu, &insn, 4)?;
                        let offset = bus
                            .read_u16(addr)
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        let selector = bus
                            .read_u16(addr.wrapping_add(2))
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        (u32::from(offset), selector)
                    };
                    if cr0_pe(cpu) && !eflags_vm(cpu.rflags) {
                        protected_far_call(cpu, bus, offset, selector, next_ip, op32)?;
                    } else if op32 {
                        push16(cpu, bus, cpu.cs.selector)?;
                        push32(cpu, bus, next_ip)?;
                        cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                        cpu.set_ip16(offset as u16);
                    } else {
                        push16(cpu, bus, cpu.cs.selector)?;
                        push16(cpu, bus, next_ip as u16)?;
                        cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                        cpu.set_ip16(offset as u16);
                    }
                }
                4 => {
                    // JMP r/m16|32 near absolute indirect.
                    let target = if op32 {
                        read_rm_u32(cpu, bus, &insn)?
                    } else {
                        u32::from(read_rm_u16(cpu, bus, &insn)?)
                    };
                    set_current_ip(cpu, near_absolute_target(target, op32));
                }
                5 => {
                    // JMP FAR m16:16 / m16:32 — absolute indirect far (memory only).
                    // Spec: Intel SDM Vol. 2 "JMP"; opcode map Group 5 /5; Ch. 2
                    // (66H); Vol. 3 §5.8.1. Register form is invalid (#UD).
                    // Protected mode is bounded to m16:16 through a same-level
                    // nonconforming D=0 GDT code segment.
                    if m.mod_ == 3 {
                        return real_mode_ud(cpu, bus);
                    }
                    let (offset, selector) = if op32 {
                        let (addr, _, uses_ss) = ea(cpu, &insn, 6)?;
                        let offset = bus
                            .read_u32(addr)
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        let selector = bus
                            .read_u16(addr.wrapping_add(4))
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        (offset, selector)
                    } else {
                        let (addr, _, uses_ss) = ea(cpu, &insn, 4)?;
                        let offset = bus
                            .read_u16(addr)
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        let selector = bus
                            .read_u16(addr.wrapping_add(2))
                            .map_err(|e| classify_mem_fault(e, uses_ss))?;
                        (u32::from(offset), selector)
                    };
                    if cr0_pe(cpu) && !eflags_vm(cpu.rflags) {
                        protected_far_jump(cpu, bus, offset, selector, next_ip)?;
                    } else {
                        // Real-address / VM86 (Vol. 3 §20.1): stay in current mode.
                        cpu.cs = x86_core::SegmentReg::real_mode_code(selector);
                        cpu.set_ip16(offset as u16);
                    }
                }
                6 => {
                    // PUSH r/m16|32 — value is read before SP decrement (incl. PUSH SP).
                    if op32 {
                        let v = read_rm_u32(cpu, bus, &insn)?;
                        push32(cpu, bus, v)?;
                    } else {
                        let v = read_rm_u16(cpu, bus, &insn)?;
                        push16(cpu, bus, v)?;
                    }
                    set_current_ip(cpu, next_ip);
                }
                _ => return real_mode_ud(cpu, bus), // /7 reserved
            }
        }
        0x70..=0x7F => {
            // Jcc rel8 — Spec: Intel SDM Vol. 2 "Jcc".
            // Unsupported here: near rel16/rel32 forms (0F 8x).
            if jcc_condition(cpu, op) {
                set_current_ip(
                    cpu,
                    near_branch_target(next_ip, insn.immediate, opsz32(&insn)),
                );
            } else {
                set_current_ip(cpu, next_ip);
            }
        }
        0x40..=0x47 => {
            // INC r16/r32 — Spec: Intel SDM Vol. 2 "INC"; Ch. 2 (66H).
            let idx = (op - 0x40) as usize;
            let saved_cf = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let old = cpu.gpr_u32(idx);
                let v = old.wrapping_add(1);
                cpu.set_gpr_u32(idx, v);
                set_add_flags_u32(cpu, old, 1, v);
            } else {
                let old = cpu.gpr_u16(idx);
                let v = old.wrapping_add(1);
                cpu.set_gpr_u16(idx, v);
                set_add_flags_u16(cpu, old, 1, v);
            }
            // INC does not modify CF (Intel SDM Vol. 2, INC).
            cpu.set_cf(saved_cf);
            set_current_ip(cpu, next_ip);
        }
        0x48..=0x4F => {
            // DEC r16/r32 — Spec: Intel SDM Vol. 2 "DEC"; Ch. 2 (66H).
            let idx = (op - 0x48) as usize;
            let saved_cf = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let old = cpu.gpr_u32(idx);
                let v = old.wrapping_sub(1);
                cpu.set_gpr_u32(idx, v);
                set_sub_flags_u32(cpu, old, 1, v);
            } else {
                let old = cpu.gpr_u16(idx);
                let v = old.wrapping_sub(1);
                cpu.set_gpr_u16(idx, v);
                set_sub_flags_u16(cpu, old, 1, v);
            }
            // DEC does not modify CF (Intel SDM Vol. 2, DEC).
            cpu.set_cf(saved_cf);
            set_current_ip(cpu, next_ip);
        }
        0x50..=0x57 => {
            // PUSH r16/r32 — Spec: Intel SDM Vol. 2 "PUSH"; Ch. 2 (66H).
            let idx = (op - 0x50) as usize;
            if opsz32(&insn) {
                push32(cpu, bus, cpu.gpr_u32(idx))?;
            } else {
                push16(cpu, bus, cpu.gpr_u16(idx))?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x58..=0x5F => {
            // POP r16/r32 — Spec: Intel SDM Vol. 2 "POP"; Ch. 2 (66H).
            let idx = (op - 0x58) as usize;
            if opsz32(&insn) {
                let v = pop32(cpu, bus)?;
                cpu.set_gpr_u32(idx, v);
            } else {
                let v = pop16(cpu, bus)?;
                cpu.set_gpr_u16(idx, v);
            }
            set_current_ip(cpu, next_ip);
        }
        0x60 => {
            // PUSHA/PUSHAD — push AX…DI / EAX…EDI; Temp = SP/ESP before pushes.
            // Spec: Intel SDM Vol. 2 "PUSHA/PUSHAD"; Ch. 2 (66H).
            // `Temp` is the stack pointer before the first push: ESP when
            // `SS.B=1`, otherwise SP (zero-extended into the PUSHAD slot).
            // The `0x67` address-size prefix does not change the stack address
            // size. Unsupported here: 64-bit stacks (RSP).
            if opsz32(&insn) {
                let temp = stack_pointer(cpu);
                push32(cpu, bus, cpu.gpr_u32(CpuState::RAX))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RCX))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RDX))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RBX))?;
                push32(cpu, bus, temp)?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RBP))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RSI))?;
                push32(cpu, bus, cpu.gpr_u32(CpuState::RDI))?;
            } else {
                let sp0 = stack_pointer(cpu) as u16;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RAX))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RCX))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RDX))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RBX))?;
                push16(cpu, bus, sp0)?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RBP))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RSI))?;
                push16(cpu, bus, cpu.gpr_u16(CpuState::RDI))?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x61 => {
            // POPA/POPAD — pop DI…AX / EDI…EAX; discard saved SP/ESP slot.
            // Spec: Intel SDM Vol. 2 "POPA/POPAD"; Vol. 1 §6.2.2; Ch. 2 (66H).
            // The stack pointer follows `SS.B`; `0x67` does not change it.
            // Unsupported here: 64-bit stacks (RSP).
            if opsz32(&insn) {
                let di = pop32(cpu, bus)?;
                let si = pop32(cpu, bus)?;
                let bp = pop32(cpu, bus)?;
                let _discard_esp = pop32(cpu, bus)?;
                let bx = pop32(cpu, bus)?;
                let dx = pop32(cpu, bus)?;
                let cx = pop32(cpu, bus)?;
                let ax = pop32(cpu, bus)?;
                cpu.set_gpr_u32(CpuState::RDI, di);
                cpu.set_gpr_u32(CpuState::RSI, si);
                cpu.set_gpr_u32(CpuState::RBP, bp);
                cpu.set_gpr_u32(CpuState::RBX, bx);
                cpu.set_gpr_u32(CpuState::RDX, dx);
                cpu.set_gpr_u32(CpuState::RCX, cx);
                cpu.set_gpr_u32(CpuState::RAX, ax);
            } else {
                let di = pop16(cpu, bus)?;
                let si = pop16(cpu, bus)?;
                let bp = pop16(cpu, bus)?;
                let _discard_sp = pop16(cpu, bus)?;
                let bx = pop16(cpu, bus)?;
                let dx = pop16(cpu, bus)?;
                let cx = pop16(cpu, bus)?;
                let ax = pop16(cpu, bus)?;
                cpu.set_gpr_u16(CpuState::RDI, di);
                cpu.set_gpr_u16(CpuState::RSI, si);
                cpu.set_gpr_u16(CpuState::RBP, bp);
                cpu.set_gpr_u16(CpuState::RBX, bx);
                cpu.set_gpr_u16(CpuState::RDX, dx);
                cpu.set_gpr_u16(CpuState::RCX, cx);
                cpu.set_gpr_u16(CpuState::RAX, ax);
            }
            set_current_ip(cpu, next_ip);
        }
        0x62 => {
            // BOUND r16/r32, m16&16 / m32&32 — signed index vs lower/upper bounds.
            // Spec: Intel SDM Vol. 2 "BOUND"; Vol. 3 §6.15 (#BR — fault, vector 5); Ch. 2.
            // Register form (mod=11) → #UD. #BR saved IP = BOUND instruction.
            // Unsupported here: protected mode; 64-bit (#UD).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.mod_ == 3 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let (addr, _, uses_ss) = ea(cpu, &insn, 8)?;
                let lower =
                    bus.read_u32(addr)
                        .map_err(|e| classify_mem_fault(e, uses_ss))? as i32;
                let upper =
                    bus.read_u32(addr.wrapping_add(4))
                        .map_err(|e| classify_mem_fault(e, uses_ss))? as i32;
                let index = cpu.gpr_u32(m.reg as usize) as i32;
                if index < lower || index > upper {
                    return real_mode_exception(cpu, bus, 5);
                }
            } else {
                let (addr, _, uses_ss) = ea(cpu, &insn, 4)?;
                let lower =
                    bus.read_u16(addr)
                        .map_err(|e| classify_mem_fault(e, uses_ss))? as i16;
                let upper =
                    bus.read_u16(addr.wrapping_add(2))
                        .map_err(|e| classify_mem_fault(e, uses_ss))? as i16;
                let index = cpu.gpr_u16(m.reg as usize) as i16;
                if index < lower || index > upper {
                    return real_mode_exception(cpu, bus, 5);
                }
            }
            set_current_ip(cpu, next_ip);
        }
        0x63 => {
            // ARPL r/m16, r16 — Spec: Intel SDM Vol. 2 "ARPL".
            // Real-address / virtual-8086 mode → #UD. Always 16-bit operands.
            // If DEST.RPL < SRC.RPL, set DEST.RPL = SRC.RPL and ZF=1; else ZF=0.
            if !cr0_pe(cpu) {
                return Err(arch_fault(6));
            }
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let dest = read_rm_u16(cpu, bus, &insn)?;
            let src = cpu.gpr_u16(m.reg as usize);
            let dest_rpl = dest & 3;
            let src_rpl = src & 3;
            if dest_rpl < src_rpl {
                write_rm_u16(cpu, bus, &insn, (dest & !3) | src_rpl)?;
                cpu.set_zf(true);
            } else {
                cpu.set_zf(false);
            }
            set_current_ip(cpu, next_ip);
        }
        0x8F => {
            // POP r/m16|32 — Group /0 only.
            // Spec: Intel SDM Vol. 2 "POP"; Ch. 2 (66H).
            // /1–/7 reserved → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg != 0 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                let v = pop32(cpu, bus)?;
                write_rm_u32(cpu, bus, &insn, v)?;
            } else {
                let v = pop16(cpu, bus)?;
                write_rm_u16(cpu, bus, &insn, v)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x68 => {
            // PUSH imm16/imm32 — Spec: Intel SDM Vol. 2 "PUSH"; Ch. 2 (66H).
            if opsz32(&insn) {
                push32(cpu, bus, insn.immediate as u32)?;
            } else {
                push16(cpu, bus, insn.immediate as u16)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x69 | 0x6B => {
            // IMUL r16/r32, r/m16/32, imm — Spec: Intel SDM Vol. 2 "IMUL"; Ch. 2 (66H).
            // Dest = ModRM.reg; src = r/m; 6B imm8 sign-extended; 69 imm follows OsZ.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let src = read_rm_u32(cpu, bus, &insn)?;
                let imm = if op == 0x6B {
                    i32::from(insn.immediate as i8)
                } else {
                    insn.immediate
                };
                let prod = i64::from(src as i32).wrapping_mul(i64::from(imm));
                cpu.set_gpr_u32(m.reg as usize, prod as u32);
                set_imul_flags_i32(cpu, prod);
            } else {
                let src = read_rm_u16(cpu, bus, &insn)?;
                let imm = if op == 0x6B {
                    i32::from(insn.immediate as i8)
                } else {
                    i32::from(insn.immediate as u16 as i16)
                };
                let prod = i32::from(src as i16).wrapping_mul(imm);
                cpu.set_gpr_u16(m.reg as usize, prod as u16);
                set_imul_flags_i16(cpu, prod);
            }
            set_current_ip(cpu, next_ip);
        }
        0x6A => {
            // PUSH imm8 (sign-extended to opsize) — Spec: Intel SDM Vol. 2 "PUSH".
            if opsz32(&insn) {
                let v = insn.immediate as i8 as i32 as u32;
                push32(cpu, bus, v)?;
            } else {
                let v = insn.immediate as i8 as i16 as u16;
                push16(cpu, bus, v)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x6C => {
            // INSB — Spec: Intel SDM Vol. 2 "INS/INSB/INSW/INSD"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Port = DX; dest = ES:DI. F2/F3 act as unconditional REP (count = CX).
            // Unsupported here: asize 64; IOPL/CPL checks.
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                insb_once(cpu, bus, insn)
            })?;
        }
        0x6D => {
            // INSW/INSD — Spec: Intel SDM Vol. 2 "INS/INSB/INSW/INSD"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Operand-size 16 → word; 0x66 → dword. Unsupported: asize 64; IOPL.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    insd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    insw_once(cpu, bus, insn)
                })?;
            }
        }
        0x6E => {
            // OUTSB — Spec: Intel SDM Vol. 2 "OUTS/OUTSB/OUTSW/OUTSD"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Port = DX; src = DS:SI (segment override allowed).
            // Unsupported here: asize 64; IOPL/CPL checks.
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                outsb_once(cpu, bus, insn)
            })?;
        }
        0x6F => {
            // OUTSW/OUTSD — Spec: Intel SDM Vol. 2 "OUTS/OUTSB/OUTSW/OUTSD"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Operand-size 16 → word; 0x66 → dword. Unsupported: asize 64; IOPL.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    outsd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    outsw_once(cpu, bus, insn)
                })?;
            }
        }
        0xA4 => {
            // MOVSB — Spec: Intel SDM Vol. 2 "MOVS/MOVSB/MOVSW/MOVSD/MOVSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: MOVSQ; asize 64.
            // F2/F3 both act as unconditional REP for MOVS (count = (E)CX).
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                movsb_once(cpu, bus, insn)
            })?;
        }
        0xA5 => {
            // MOVSW/MOVSD — Spec: Intel SDM Vol. 2 "MOVS/MOVSB/MOVSW/MOVSD/MOVSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Operand-size 16 → word; 0x66 → dword. Unsupported: MOVSQ; asize 64.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    movsd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    movsw_once(cpu, bus, insn)
                })?;
            }
        }
        0xAA => {
            // STOSB — Spec: Intel SDM Vol. 2 "STOS/STOSB/STOSW/STOSD/STOSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: STOSQ; asize 64.
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                stosb_once(cpu, bus, insn)
            })?;
        }
        0xAB => {
            // STOSW/STOSD — Spec: Intel SDM Vol. 2 "STOS/STOSB/STOSW/STOSD/STOSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: STOSQ; asize 64.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    stosd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    stosw_once(cpu, bus, insn)
                })?;
            }
        }
        0xAC => {
            // LODSB — Spec: Intel SDM Vol. 2 "LODS/LODSB/LODSW/LODSD/LODSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: LODSQ; asize 64.
            exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                lodsb_once(cpu, bus, insn)
            })?;
        }
        0xAD => {
            // LODSW/LODSD — Spec: Intel SDM Vol. 2 "LODS/LODSB/LODSW/LODSD/LODSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // Unsupported here: LODSQ; asize 64.
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    lodsd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, None, |cpu, bus, insn| {
                    lodsw_once(cpu, bus, insn)
                })?;
            }
        }
        0xA6 => {
            // CMPSB — Spec: Intel SDM Vol. 2 "CMPS/CMPSB/CMPSW/CMPSD/CMPSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // F3 = REPE (while ZF=1); F2 = REPNE (while ZF=0).
            // Unsupported here: CMPSQ; asize 64.
            let zf_term = if insn.prefixes.repne {
                Some(false)
            } else if insn.prefixes.rep {
                Some(true)
            } else {
                None
            };
            exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                cmpsb_once(cpu, bus, insn)
            })?;
        }
        0xA7 => {
            // CMPSW/CMPSD — Spec: Intel SDM Vol. 2 "CMPS/CMPSB/CMPSW/CMPSD/CMPSQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // F3 = REPE; F2 = REPNE. Unsupported: CMPSQ; asize 64.
            let zf_term = if insn.prefixes.repne {
                Some(false)
            } else if insn.prefixes.rep {
                Some(true)
            } else {
                None
            };
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                    cmpsd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                    cmpsw_once(cpu, bus, insn)
                })?;
            }
        }
        0xAE => {
            // SCASB — Spec: Intel SDM Vol. 2 "SCAS/SCASB/SCASW/SCASD/SCASQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // F3 = REPE (while ZF=1); F2 = REPNE (while ZF=0).
            // Unsupported here: SCASQ; asize 64.
            let zf_term = if insn.prefixes.repne {
                Some(false)
            } else if insn.prefixes.rep {
                Some(true)
            } else {
                None
            };
            exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                scasb_once(cpu, bus, insn)
            })?;
        }
        0xAF => {
            // SCASW/SCASD — Spec: Intel SDM Vol. 2 "SCAS/SCASB/SCASW/SCASD/SCASQ"
            // and "REP/REPE/REPNE/REPZ/REPNZ".
            // F3 = REPE; F2 = REPNE. Unsupported: SCASQ; asize 64.
            let zf_term = if insn.prefixes.repne {
                Some(false)
            } else if insn.prefixes.rep {
                Some(true)
            } else {
                None
            };
            if opsz32(&insn) {
                exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                    scasd_once(cpu, bus, insn)
                })?;
            } else {
                exec_string_op(cpu, bus, &insn, next_ip, zf_term, |cpu, bus, insn| {
                    scasw_once(cpu, bus, insn)
                })?;
            }
        }
        0xA0 => {
            // MOV AL, moffs8 — Spec: Intel SDM Vol. 2 "MOV": "the address-size
            // attribute of the instruction determines the size of the offset".
            let off = moffs_offset(&insn);
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = string_src_uses_ss(&insn);
            let addr = seg_linear_checked(seg, off, 1, uses_ss)?;
            let v = bus
                .read_u8(addr)
                .map_err(|e| classify_mem_fault(e, uses_ss))?;
            cpu.set_al(v);
            set_current_ip(cpu, next_ip);
        }
        0xA1 => {
            // MOV AX/EAX, moffs — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            // Address-size selects moffs16/moffs32; operand-size selects AX/EAX.
            let off = moffs_offset(&insn);
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = string_src_uses_ss(&insn);
            if opsz32(&insn) {
                let addr = seg_linear_checked(seg, off, 4, uses_ss)?;
                let v = bus
                    .read_u32(addr)
                    .map_err(|e| classify_mem_fault(e, uses_ss))?;
                cpu.set_eax(v);
            } else {
                let addr = seg_linear_checked(seg, off, 2, uses_ss)?;
                let v = bus
                    .read_u16(addr)
                    .map_err(|e| classify_mem_fault(e, uses_ss))?;
                cpu.set_ax(v);
            }
            set_current_ip(cpu, next_ip);
        }
        0xA2 => {
            // MOV moffs8, AL — Spec: Intel SDM Vol. 2 "MOV".
            let off = moffs_offset(&insn);
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = string_src_uses_ss(&insn);
            let addr = seg_linear_checked(seg, off, 1, uses_ss)?;
            bus.write_u8(addr, cpu.al())
                .map_err(|e| classify_mem_fault(e, uses_ss))?;
            set_current_ip(cpu, next_ip);
        }
        0xA3 => {
            // MOV moffs, AX/EAX — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            let off = moffs_offset(&insn);
            let seg = data_seg_for_string_src(cpu, &insn);
            let uses_ss = string_src_uses_ss(&insn);
            if opsz32(&insn) {
                let addr = seg_linear_checked(seg, off, 4, uses_ss)?;
                bus.write_u32(addr, cpu.eax())
                    .map_err(|e| classify_mem_fault(e, uses_ss))?;
            } else {
                let addr = seg_linear_checked(seg, off, 2, uses_ss)?;
                bus.write_u16(addr, cpu.ax())
                    .map_err(|e| classify_mem_fault(e, uses_ss))?;
            }
            set_current_ip(cpu, next_ip);
        }
        0xA8 => {
            // TEST AL, imm8 — Spec: Intel SDM Vol. 2 "TEST".
            // Flags: CF=OF=0; SF/ZF/PF from (AL & imm); AF undefined (cleared).
            set_logic_flags_u8(cpu, cpu.al() & insn.immediate as u8);
            set_current_ip(cpu, next_ip);
        }
        0xA9 => {
            // TEST AX/EAX, imm16/imm32 — Spec: Intel SDM Vol. 2 "TEST"; Ch. 2 (66H).
            // Flags: CF=OF=0; SF/ZF/PF from (AX/EAX & imm); AF undefined (cleared).
            if opsz32(&insn) {
                set_logic_flags_u32(cpu, cpu.eax() & insn.immediate as u32);
            } else {
                set_logic_flags_u16(cpu, cpu.ax() & insn.immediate as u16);
            }
            set_current_ip(cpu, next_ip);
        }
        0xC6 => {
            // Group 11 MOV r/m8, imm8 — Spec: Intel SDM Vol. 2 "MOV" / opcode map.
            // Only /0 is defined; /1–/7 → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg != 0 {
                return real_mode_ud(cpu, bus);
            }
            write_rm_u8(cpu, bus, &insn, insn.immediate as u8)?;
            set_current_ip(cpu, next_ip);
        }
        0xC7 => {
            // Group 11 MOV r/m16|32, imm16|32 — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2.
            // Only /0 is defined; /1–/7 → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.reg != 0 {
                return real_mode_ud(cpu, bus);
            }
            if opsz32(&insn) {
                write_rm_u32(cpu, bus, &insn, insn.immediate as u32)?;
            } else {
                write_rm_u16(cpu, bus, &insn, insn.immediate as u16)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0xB0..=0xB7 => {
            // MOV r8, imm8 - B0-B3 AL/CL/DL/BL; B4-B7 AH/CH/DH/BH (SDM Vol. 2 MOV).
            write_reg_u8(cpu, op - 0xB0, insn.immediate as u8);
            set_current_ip(cpu, next_ip);
        }
        0xB8..=0xBF => {
            // MOV r16/r32, imm16/imm32 — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            let idx = (op - 0xB8) as usize;
            if opsz32(&insn) {
                cpu.set_gpr_u32(idx, insn.immediate as u32);
            } else {
                cpu.set_gpr_u16(idx, insn.immediate as u16);
            }
            set_current_ip(cpu, next_ip);
        }
        0x8A => {
            // MOV r8, r/m8
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_rm_u8(cpu, bus, &insn)?;
            write_reg_u8(cpu, m.reg, v);
            set_current_ip(cpu, next_ip);
        }
        0x88 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let v = read_reg_u8(cpu, m.reg);
            write_rm_u8(cpu, bus, &insn, v)?;
            set_current_ip(cpu, next_ip);
        }
        0x8B => {
            // MOV r16/r32, r/m16|32 — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let v = read_rm_u32(cpu, bus, &insn)?;
                cpu.set_gpr_u32(m.reg as usize, v);
            } else {
                let v = read_rm_u16(cpu, bus, &insn)?;
                cpu.set_gpr_u16(m.reg as usize, v);
            }
            set_current_ip(cpu, next_ip);
        }
        0x89 => {
            // MOV r/m16|32, r16/r32 — Spec: Intel SDM Vol. 2 "MOV"; Ch. 2 (66H).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let v = cpu.gpr_u32(m.reg as usize);
                write_rm_u32(cpu, bus, &insn, v)?;
            } else {
                let v = cpu.gpr_u16(m.reg as usize);
                write_rm_u16(cpu, bus, &insn, v)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x8C => {
            // MOV r/m16|r32, Sreg — Spec: Intel SDM Vol. 2 "MOV" (r/m16, Sreg); Ch. 2.
            // OsZ32 + register dest: zero-extend selector into r32.
            // Memory dest always stores 16 bits (selector width), even with 0x66.
            // Reserved Sreg encodings (reg=6,7) → #UD (Vol. 3 §6.15).
            // Unsupported here: protected-mode side effects.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let Some(sreg) = sreg_from_modrm_reg(m.reg) else {
                return real_mode_ud(cpu, bus);
            };
            let v = read_sreg_selector(cpu, sreg);
            if opsz32(&insn) && m.mod_ == 3 {
                cpu.set_gpr_u32(m.rm as usize, u32::from(v));
            } else {
                write_rm_u16(cpu, bus, &insn, v)?;
            }
            set_current_ip(cpu, next_ip);
        }
        0x8D => {
            // LEA r16/r32, m — load effective address (offset only; no memory read).
            // Spec: Intel SDM Vol. 2 "LEA"; Vol. 1 §3.6 (address-/operand-size).
            // Register source (mod=11) → #UD (Vol. 3 §6.15).
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if m.mod_ == 3 {
                return real_mode_ud(cpu, bus);
            }
            if asize32(&insn) {
                let off = calc_ea32(cpu, &insn)?;
                if opsz32(&insn) {
                    cpu.set_gpr_u32(m.reg as usize, off);
                } else {
                    cpu.set_gpr_u16(m.reg as usize, off as u16);
                }
            } else {
                let off = calc_ea16(cpu, m.mod_, m.rm, insn.displacement)?;
                if opsz32(&insn) {
                    cpu.set_gpr_u32(m.reg as usize, u32::from(off));
                } else {
                    cpu.set_gpr_u16(m.reg as usize, off);
                }
            }
            set_current_ip(cpu, next_ip);
        }
        0x8E => {
            // MOV Sreg, r/m16 — Spec: Intel SDM Vol. 2 "MOV" (Sreg, r/m16).
            // PE=0: real-address load (base = selector << 4); sticky unreal limit/AR.
            // PE=1: DS/ES/FS/GS/SS load hidden cache from GDT (DS/ES/FS/GS null
            // clears; SS null → #GP; P=0 → #NP for data / #SS for SS; invalid
            // type / out of limit → #GP; SS requires writable data).
            // MOV to CS and reserved Sreg encodings → #UD (Vol. 3 §6.15).
            // Unsupported here: LDT resolution.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let Some(sreg) = sreg_from_modrm_reg(m.reg) else {
                return real_mode_ud(cpu, bus);
            };
            if sreg == 1 {
                // MOV to CS is invalid (#UD). Spec: Intel SDM Vol. 2 "MOV".
                return real_mode_ud(cpu, bus);
            }
            let v = read_rm_u16(cpu, bus, &insn)?;
            write_sreg(cpu, bus, sreg, v)?;
            if sreg == 2 {
                cpu.arm_maskable_interrupt_shadow();
            }
            set_current_ip(cpu, next_ip);
        }
        0x86 => {
            // XCHG r8, r/m8 — Spec: Intel SDM Vol. 2 "XCHG".
            // Flags unchanged. Unsupported here: LOCK bus-lock.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let rm = read_rm_u8(cpu, bus, &insn)?;
            let reg = read_reg_u8(cpu, m.reg);
            write_rm_u8(cpu, bus, &insn, reg)?;
            write_reg_u8(cpu, m.reg, rm);
            set_current_ip(cpu, next_ip);
        }
        0x87 => {
            // XCHG r16, r/m16 — Spec: Intel SDM Vol. 2 "XCHG".
            // Flags unchanged. Unsupported here: opsize 32; LOCK bus-lock.
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let rm = read_rm_u16(cpu, bus, &insn)?;
            let reg = cpu.gpr_u16(m.reg as usize);
            write_rm_u16(cpu, bus, &insn, reg)?;
            cpu.set_gpr_u16(m.reg as usize, rm);
            set_current_ip(cpu, next_ip);
        }
        0x84 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            set_logic_flags_u8(cpu, a & b);
            set_current_ip(cpu, next_ip);
        }
        0x85 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u16(cpu, bus, &insn)?;
            let b = cpu.gpr_u16(m.reg as usize);
            set_logic_flags_u16(cpu, a & b);
            set_current_ip(cpu, next_ip);
        }
        // XOR ModRM — Spec: Intel SDM Vol. 2 "XOR".
        // Flags: CF=OF=0; SF/ZF/PF from result; AF undefined (cleared here).
        // Unsupported here: opsize 32; LOCK; AH/CH/DH/BH high-byte GPRs; segment-limit faults.
        0x30 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a ^ b;
            write_rm_u8(cpu, bus, &insn, r)?;
            set_logic_flags_u8(cpu, r);
            set_current_ip(cpu, next_ip);
        }
        0x32 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a ^ b;
            write_reg_u8(cpu, m.reg, r);
            set_logic_flags_u8(cpu, r);
            set_current_ip(cpu, next_ip);
        }
        0x31 | 0x33 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                if op == 0x31 {
                    let a = read_rm_u32(cpu, bus, &insn)?;
                    let b = cpu.gpr_u32(m.reg as usize);
                    let r = a ^ b;
                    write_rm_u32(cpu, bus, &insn, r)?;
                    set_logic_flags_u32(cpu, r);
                } else {
                    let a = cpu.gpr_u32(m.reg as usize);
                    let b = read_rm_u32(cpu, bus, &insn)?;
                    let r = a ^ b;
                    cpu.set_gpr_u32(m.reg as usize, r);
                    set_logic_flags_u32(cpu, r);
                }
            } else if op == 0x31 {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a ^ b;
                write_rm_u16(cpu, bus, &insn, r)?;
                set_logic_flags_u16(cpu, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a ^ b;
                cpu.set_gpr_u16(m.reg as usize, r);
                set_logic_flags_u16(cpu, r);
            }
            set_current_ip(cpu, next_ip);
        }
        // ADD/SUB ModRM — Spec: Intel SDM Vol. 2 "ADD" / "SUB".
        // Flags via set_add_flags_* / set_sub_flags_* (CF/OF/AF/ZF/SF/PF).
        // Unsupported here: opsize 32; LOCK; AH/CH/DH/BH high-byte GPRs; segment-limit faults.
        0x00 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a.wrapping_add(b);
            write_rm_u8(cpu, bus, &insn, r)?;
            set_add_flags_u8(cpu, a, b, r);
            set_current_ip(cpu, next_ip);
        }
        0x02 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a.wrapping_add(b);
            write_reg_u8(cpu, m.reg, r);
            set_add_flags_u8(cpu, a, b, r);
            set_current_ip(cpu, next_ip);
        }
        0x01 | 0x03 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                if op == 0x01 {
                    let a = read_rm_u32(cpu, bus, &insn)?;
                    let b = cpu.gpr_u32(m.reg as usize);
                    let r = a.wrapping_add(b);
                    write_rm_u32(cpu, bus, &insn, r)?;
                    set_add_flags_u32(cpu, a, b, r);
                } else {
                    let a = cpu.gpr_u32(m.reg as usize);
                    let b = read_rm_u32(cpu, bus, &insn)?;
                    let r = a.wrapping_add(b);
                    cpu.set_gpr_u32(m.reg as usize, r);
                    set_add_flags_u32(cpu, a, b, r);
                }
            } else if op == 0x01 {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a.wrapping_add(b);
                write_rm_u16(cpu, bus, &insn, r)?;
                set_add_flags_u16(cpu, a, b, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a.wrapping_add(b);
                cpu.set_gpr_u16(m.reg as usize, r);
                set_add_flags_u16(cpu, a, b, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x28 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a.wrapping_sub(b);
            write_rm_u8(cpu, bus, &insn, r)?;
            set_sub_flags_u8(cpu, a, b, r);
            set_current_ip(cpu, next_ip);
        }
        0x2A => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a.wrapping_sub(b);
            write_reg_u8(cpu, m.reg, r);
            set_sub_flags_u8(cpu, a, b, r);
            set_current_ip(cpu, next_ip);
        }
        0x29 | 0x2B => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                if op == 0x29 {
                    let a = read_rm_u32(cpu, bus, &insn)?;
                    let b = cpu.gpr_u32(m.reg as usize);
                    let r = a.wrapping_sub(b);
                    write_rm_u32(cpu, bus, &insn, r)?;
                    set_sub_flags_u32(cpu, a, b, r);
                } else {
                    let a = cpu.gpr_u32(m.reg as usize);
                    let b = read_rm_u32(cpu, bus, &insn)?;
                    let r = a.wrapping_sub(b);
                    cpu.set_gpr_u32(m.reg as usize, r);
                    set_sub_flags_u32(cpu, a, b, r);
                }
            } else if op == 0x29 {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a.wrapping_sub(b);
                write_rm_u16(cpu, bus, &insn, r)?;
                set_sub_flags_u16(cpu, a, b, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a.wrapping_sub(b);
                cpu.set_gpr_u16(m.reg as usize, r);
                set_sub_flags_u16(cpu, a, b, r);
            }
            set_current_ip(cpu, next_ip);
        }
        // CMP ModRM — Spec: Intel SDM Vol. 2 "CMP".
        // Flags via set_sub_flags_* (same as SUB); operands unchanged.
        // Unsupported here: opsize 32; LOCK; AH/CH/DH/BH high-byte GPRs; segment-limit faults.
        0x38 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            set_sub_flags_u8(cpu, a, b, a.wrapping_sub(b));
            set_current_ip(cpu, next_ip);
        }
        0x3A => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            set_sub_flags_u8(cpu, a, b, a.wrapping_sub(b));
            set_current_ip(cpu, next_ip);
        }
        0x39 | 0x3B => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                if op == 0x39 {
                    let a = read_rm_u32(cpu, bus, &insn)?;
                    let b = cpu.gpr_u32(m.reg as usize);
                    set_sub_flags_u32(cpu, a, b, a.wrapping_sub(b));
                } else {
                    let a = cpu.gpr_u32(m.reg as usize);
                    let b = read_rm_u32(cpu, bus, &insn)?;
                    set_sub_flags_u32(cpu, a, b, a.wrapping_sub(b));
                }
            } else if op == 0x39 {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                set_sub_flags_u16(cpu, a, b, a.wrapping_sub(b));
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                set_sub_flags_u16(cpu, a, b, a.wrapping_sub(b));
            }
            set_current_ip(cpu, next_ip);
        }
        0x04 => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            let r = a.wrapping_add(b);
            cpu.set_al(r);
            // minimal flags for 8-bit add
            cpu.set_cf((a as u16) + (b as u16) > 0xFF);
            cpu.set_zf(r == 0);
            cpu.set_sf(r & 0x80 != 0);
            cpu.set_pf(parity_even(r));
            set_current_ip(cpu, next_ip);
        }
        // ADD AX/EAX,imm — Spec: Intel SDM Vol. 2 "ADD" (05 iw/id); Ch. 2 (66H).
        0x05 => {
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                let r = a.wrapping_add(b);
                cpu.set_eax(r);
                set_add_flags_u32(cpu, a, b, r);
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                let r = a.wrapping_add(b);
                cpu.set_ax(r);
                set_add_flags_u16(cpu, a, b, r);
            }
            set_current_ip(cpu, next_ip);
        }
        // OR/AND AL/AX,imm — Spec: Intel SDM Vol. 2 "OR" / "AND" (accumulator forms).
        // Flags: CF=OF=0; SF/ZF/PF from result; AF undefined (cleared here).
        // Unsupported here: opsize 32 (imm32 into EAX).
        0x0C => {
            let r = cpu.al() | (insn.immediate as u8);
            cpu.set_al(r);
            set_logic_flags_u8(cpu, r);
            set_current_ip(cpu, next_ip);
        }
        0x0D => {
            if opsz32(&insn) {
                let r = cpu.eax() | (insn.immediate as u32);
                cpu.set_eax(r);
                set_logic_flags_u32(cpu, r);
            } else {
                let r = cpu.ax() | (insn.immediate as u16);
                cpu.set_ax(r);
                set_logic_flags_u16(cpu, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x24 => {
            let r = cpu.al() & (insn.immediate as u8);
            cpu.set_al(r);
            set_logic_flags_u8(cpu, r);
            set_current_ip(cpu, next_ip);
        }
        0x25 => {
            if opsz32(&insn) {
                let r = cpu.eax() & (insn.immediate as u32);
                cpu.set_eax(r);
                set_logic_flags_u32(cpu, r);
            } else {
                let r = cpu.ax() & (insn.immediate as u16);
                cpu.set_ax(r);
                set_logic_flags_u16(cpu, r);
            }
            set_current_ip(cpu, next_ip);
        }
        // ADC/SBB AL/AX,imm — Spec: Intel SDM Vol. 2 "ADC" / "SBB" (accumulator forms).
        // dest ← dest ± imm ± CF; flags via set_adc_flags_* / set_sbb_flags_*.
        // Unsupported here: opsize 32 (imm32 into EAX).
        0x14 => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_add(b).wrapping_add(u8::from(cf_in));
            cpu.set_al(r);
            set_adc_flags_u8(cpu, a, b, cf_in, r);
            set_current_ip(cpu, next_ip);
        }
        0x15 => {
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                let r = a.wrapping_add(b).wrapping_add(u32::from(cf_in));
                cpu.set_eax(r);
                set_adc_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
                cpu.set_ax(r);
                set_adc_flags_u16(cpu, a, b, cf_in, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x1C => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_sub(b).wrapping_sub(u8::from(cf_in));
            cpu.set_al(r);
            set_sbb_flags_u8(cpu, a, b, cf_in, r);
            set_current_ip(cpu, next_ip);
        }
        0x1D => {
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                let r = a.wrapping_sub(b).wrapping_sub(u32::from(cf_in));
                cpu.set_eax(r);
                set_sbb_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
                cpu.set_ax(r);
                set_sbb_flags_u16(cpu, a, b, cf_in, r);
            }
            set_current_ip(cpu, next_ip);
        }
        // SUB/XOR/CMP AL/AX/EAX,imm — Spec: Intel SDM Vol. 2 accumulator forms; Ch. 2.
        // BCD adjust — Spec: Intel SDM Vol. 2 DAA/DAS/AAA/AAS.
        // Unsupported here: 64-bit mode (#UD); INTO/BOUND (separate opcodes).
        0x27 => {
            exec_daa(cpu);
            set_current_ip(cpu, next_ip);
        }
        0x2F => {
            exec_das(cpu);
            set_current_ip(cpu, next_ip);
        }
        0x37 => {
            exec_aaa(cpu);
            set_current_ip(cpu, next_ip);
        }
        0x3F => {
            exec_aas(cpu);
            set_current_ip(cpu, next_ip);
        }
        0x2C => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            let r = a.wrapping_sub(b);
            cpu.set_al(r);
            set_sub_flags_u8(cpu, a, b, r);
            set_current_ip(cpu, next_ip);
        }
        0x2D => {
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                let r = a.wrapping_sub(b);
                cpu.set_eax(r);
                set_sub_flags_u32(cpu, a, b, r);
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                let r = a.wrapping_sub(b);
                cpu.set_ax(r);
                set_sub_flags_u16(cpu, a, b, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x34 => {
            let r = cpu.al() ^ (insn.immediate as u8);
            cpu.set_al(r);
            set_logic_flags_u8(cpu, r);
            set_current_ip(cpu, next_ip);
        }
        0x35 => {
            if opsz32(&insn) {
                let r = cpu.eax() ^ (insn.immediate as u32);
                cpu.set_eax(r);
                set_logic_flags_u32(cpu, r);
            } else {
                let r = cpu.ax() ^ (insn.immediate as u16);
                cpu.set_ax(r);
                set_logic_flags_u16(cpu, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x3C => {
            let a = cpu.al();
            let b = insn.immediate as u8;
            set_sub_flags_u8(cpu, a, b, a.wrapping_sub(b));
            set_current_ip(cpu, next_ip);
        }
        0x3D => {
            if opsz32(&insn) {
                let a = cpu.eax();
                let b = insn.immediate as u32;
                set_sub_flags_u32(cpu, a, b, a.wrapping_sub(b));
            } else {
                let a = cpu.ax();
                let b = insn.immediate as u16;
                set_sub_flags_u16(cpu, a, b, a.wrapping_sub(b));
            }
            set_current_ip(cpu, next_ip);
        }
        // ADC/SBB ModRM — Spec: Intel SDM Vol. 2 "ADC" / "SBB"; Ch. 2 (66H).
        // Unsupported here: LOCK; segment-limit faults.
        0x10 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_add(b).wrapping_add(u8::from(cf_in));
            write_rm_u8(cpu, bus, &insn, r)?;
            set_adc_flags_u8(cpu, a, b, cf_in, r);
            set_current_ip(cpu, next_ip);
        }
        0x11 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = cpu.gpr_u32(m.reg as usize);
                let r = a.wrapping_add(b).wrapping_add(u32::from(cf_in));
                write_rm_u32(cpu, bus, &insn, r)?;
                set_adc_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
                write_rm_u16(cpu, bus, &insn, r)?;
                set_adc_flags_u16(cpu, a, b, cf_in, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x12 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_add(b).wrapping_add(u8::from(cf_in));
            write_reg_u8(cpu, m.reg, r);
            set_adc_flags_u8(cpu, a, b, cf_in, r);
            set_current_ip(cpu, next_ip);
        }
        0x13 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = cpu.gpr_u32(m.reg as usize);
                let b = read_rm_u32(cpu, bus, &insn)?;
                let r = a.wrapping_add(b).wrapping_add(u32::from(cf_in));
                cpu.set_gpr_u32(m.reg as usize, r);
                set_adc_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a.wrapping_add(b).wrapping_add(u16::from(cf_in));
                cpu.set_gpr_u16(m.reg as usize, r);
                set_adc_flags_u16(cpu, a, b, cf_in, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x18 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_sub(b).wrapping_sub(u8::from(cf_in));
            write_rm_u8(cpu, bus, &insn, r)?;
            set_sbb_flags_u8(cpu, a, b, cf_in, r);
            set_current_ip(cpu, next_ip);
        }
        0x19 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = cpu.gpr_u32(m.reg as usize);
                let r = a.wrapping_sub(b).wrapping_sub(u32::from(cf_in));
                write_rm_u32(cpu, bus, &insn, r)?;
                set_sbb_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
                write_rm_u16(cpu, bus, &insn, r)?;
                set_sbb_flags_u16(cpu, a, b, cf_in, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x1A => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let cf_in = cpu.rflags & 1 != 0;
            let r = a.wrapping_sub(b).wrapping_sub(u8::from(cf_in));
            write_reg_u8(cpu, m.reg, r);
            set_sbb_flags_u8(cpu, a, b, cf_in, r);
            set_current_ip(cpu, next_ip);
        }
        0x1B => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let cf_in = cpu.rflags & 1 != 0;
            if opsz32(&insn) {
                let a = cpu.gpr_u32(m.reg as usize);
                let b = read_rm_u32(cpu, bus, &insn)?;
                let r = a.wrapping_sub(b).wrapping_sub(u32::from(cf_in));
                cpu.set_gpr_u32(m.reg as usize, r);
                set_sbb_flags_u32(cpu, a, b, cf_in, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a.wrapping_sub(b).wrapping_sub(u16::from(cf_in));
                cpu.set_gpr_u16(m.reg as usize, r);
                set_sbb_flags_u16(cpu, a, b, cf_in, r);
            }
            set_current_ip(cpu, next_ip);
        }
        // OR/AND ModRM — Spec: Intel SDM Vol. 2 "OR" / "AND"; Ch. 2 (66H).
        // Unsupported here: LOCK; segment-limit faults.
        0x08 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a | b;
            write_rm_u8(cpu, bus, &insn, r)?;
            set_logic_flags_u8(cpu, r);
            set_current_ip(cpu, next_ip);
        }
        0x09 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = cpu.gpr_u32(m.reg as usize);
                let r = a | b;
                write_rm_u32(cpu, bus, &insn, r)?;
                set_logic_flags_u32(cpu, r);
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a | b;
                write_rm_u16(cpu, bus, &insn, r)?;
                set_logic_flags_u16(cpu, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x0A => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a | b;
            write_reg_u8(cpu, m.reg, r);
            set_logic_flags_u8(cpu, r);
            set_current_ip(cpu, next_ip);
        }
        0x0B => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let a = cpu.gpr_u32(m.reg as usize);
                let b = read_rm_u32(cpu, bus, &insn)?;
                let r = a | b;
                cpu.set_gpr_u32(m.reg as usize, r);
                set_logic_flags_u32(cpu, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a | b;
                cpu.set_gpr_u16(m.reg as usize, r);
                set_logic_flags_u16(cpu, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x20 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_rm_u8(cpu, bus, &insn)?;
            let b = read_reg_u8(cpu, m.reg);
            let r = a & b;
            write_rm_u8(cpu, bus, &insn, r)?;
            set_logic_flags_u8(cpu, r);
            set_current_ip(cpu, next_ip);
        }
        0x21 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let a = read_rm_u32(cpu, bus, &insn)?;
                let b = cpu.gpr_u32(m.reg as usize);
                let r = a & b;
                write_rm_u32(cpu, bus, &insn, r)?;
                set_logic_flags_u32(cpu, r);
            } else {
                let a = read_rm_u16(cpu, bus, &insn)?;
                let b = cpu.gpr_u16(m.reg as usize);
                let r = a & b;
                write_rm_u16(cpu, bus, &insn, r)?;
                set_logic_flags_u16(cpu, r);
            }
            set_current_ip(cpu, next_ip);
        }
        0x22 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            let a = read_reg_u8(cpu, m.reg);
            let b = read_rm_u8(cpu, bus, &insn)?;
            let r = a & b;
            write_reg_u8(cpu, m.reg, r);
            set_logic_flags_u8(cpu, r);
            set_current_ip(cpu, next_ip);
        }
        0x23 => {
            let m = insn.modrm.ok_or(ExecError::Unsupported(op))?;
            if opsz32(&insn) {
                let a = cpu.gpr_u32(m.reg as usize);
                let b = read_rm_u32(cpu, bus, &insn)?;
                let r = a & b;
                cpu.set_gpr_u32(m.reg as usize, r);
                set_logic_flags_u32(cpu, r);
            } else {
                let a = cpu.gpr_u16(m.reg as usize);
                let b = read_rm_u16(cpu, bus, &insn)?;
                let r = a & b;
                cpu.set_gpr_u16(m.reg as usize, r);
                set_logic_flags_u16(cpu, r);
            }
            set_current_ip(cpu, next_ip);
        }
        _ => return Err(ExecError::Unsupported(op)),
    }

    Ok(())
}

/// Run until HLT or `max_steps`.
pub fn run(cpu: &mut CpuState, bus: &mut dyn Bus, max_steps: u64) -> Result<u64, ExecError> {
    let mut mmu = Mmu::new();
    run_with_mmu(cpu, bus, &mut mmu, max_steps)
}

/// Run until HLT or `max_steps` with a caller-owned [`Mmu`]; see
/// [`step_with_mmu`] for why that matters.
pub fn run_with_mmu(
    cpu: &mut CpuState,
    bus: &mut dyn Bus,
    mmu: &mut Mmu,
    max_steps: u64,
) -> Result<u64, ExecError> {
    let mut n = 0u64;
    while n < max_steps && !cpu.halted {
        step_with_mmu(cpu, bus, mmu)?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VecBus {
        mem: Vec<u8>,
        ports: Vec<u8>,
    }

    impl Bus for VecBus {
        fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            Ok(self.mem[i])
        }
        fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            self.mem[i] = val;
            Ok(())
        }
        fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
            Ok(0xFF)
        }
        fn port_out_u8(&mut self, _port: u16, val: u8) -> Result<(), ExecError> {
            self.ports.push(val);
            Ok(())
        }
    }

    fn assert_arch_fault(result: Result<(), ExecError>, vector: u8, error_code: Option<u16>) {
        assert_eq!(result, Err(ExecError::ArchFault { vector, error_code }));
    }

    #[derive(Clone, Copy, Debug)]
    enum ProtectedFarJumpForm {
        Immediate,
        Memory,
    }

    impl ProtectedFarJumpForm {
        const ALL: [Self; 2] = [Self::Immediate, Self::Memory];

        fn name(self) -> &'static str {
            match self {
                Self::Immediate => "EA ptr16:16",
                Self::Memory => "FF /5 m16:16",
            }
        }

        fn instruction_len(self) -> usize {
            match self {
                Self::Immediate => 5,
                Self::Memory => 4,
            }
        }

        fn write(self, mem: &mut [u8], offset: u16, selector: u16) {
            match self {
                Self::Immediate => {
                    let offset = offset.to_le_bytes();
                    let selector = selector.to_le_bytes();
                    mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + 5].copy_from_slice(&[
                        0xEA,
                        offset[0],
                        offset[1],
                        selector[0],
                        selector[1],
                    ]);
                }
                Self::Memory => {
                    // FF /5, mod=00 r/m=110: JMP FAR [DS:0x3000].
                    mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + 4]
                        .copy_from_slice(&[0xFF, 0x2E, 0x00, 0x30]);
                    mem[0x3000..0x3002].copy_from_slice(&offset.to_le_bytes());
                    mem[0x3002..0x3004].copy_from_slice(&selector.to_le_bytes());
                }
            }
        }
    }

    const PROTECTED_TEST_CODE: usize = 0x1000;
    const PROTECTED_TEST_GDT: usize = 0x4000;
    const PROTECTED_TEST_IDT: usize = 0x5000;
    const PROTECTED_TEST_HANDLER: u16 = 0x1234;
    const PROTECTED_TEST_TARGET_CS: u16 = 0x0008;
    const PROTECTED_COMPAT_IDT: usize = 0x7000;
    const PROTECTED_COMPAT_TARGET_CS: u16 = 0x0078;
    const PROTECTED_FAR_JUMP_TARGET_CS: u16 = 0x0020;
    const PROTECTED_FAR_JUMP_TARGET_BASE: u32 = 0x0000_6000;
    const PROTECTED_IRET_FRAME_SP: u16 = 0x8000;
    const PROTECTED_IRET_RETURN_CS: u16 = 0x0020;
    const PROTECTED_IRET_RETURN_BASE: u32 = 0x0000_6000;

    fn encode_idt_gate(offset: u16, selector: u16, access: u8) -> [u8; 8] {
        let offset = offset.to_le_bytes();
        let selector = selector.to_le_bytes();
        [
            offset[0],
            offset[1],
            selector[0],
            selector[1],
            0,
            access,
            0,
            0,
        ]
    }

    fn write_protected_test_gate(
        mem: &mut [u8],
        vector: u8,
        offset: u16,
        selector: u16,
        access: u8,
    ) {
        let entry = PROTECTED_TEST_IDT + usize::from(vector) * 8;
        mem[entry..entry + 8].copy_from_slice(&encode_idt_gate(offset, selector, access));
    }

    fn protected_fault_fixture(vector: u8, gate_access: u8) -> (CpuState, VecBus) {
        let mut mem = vec![0u8; 0x10000];
        // Group 2 /6 is reserved and raises #UD at the instruction start.
        mem[PROTECTED_TEST_CODE] = 0xD0;
        mem[PROTECTED_TEST_CODE + 1] = 0xF0;
        let target = encode_seg_desc(0x0000_2000, 0xFFFF, 0x9A, 0);
        mem[PROTECTED_TEST_GDT + 8..PROTECTED_TEST_GDT + 16].copy_from_slice(&target);
        write_protected_test_gate(
            &mut mem,
            vector,
            PROTECTED_TEST_HANDLER,
            PROTECTED_TEST_TARGET_CS,
            gate_access,
        );

        let mut cpu = CpuState::reset();
        cpu.cr0 |= 1;
        cpu.cs = x86_core::SegmentReg {
            selector: 0x0010,
            base: 0,
            limit: 0xFFFF,
            flags: 0x009A,
        };
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0018,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        cpu.gdtr.base = PROTECTED_TEST_GDT as u64;
        cpu.gdtr.limit = 15;
        cpu.idtr.base = PROTECTED_TEST_IDT as u64;
        cpu.idtr.limit = u16::from(vector) * 8 + 7;
        cpu.rip = PROTECTED_TEST_CODE as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        (cpu, VecBus { mem, ports: vec![] })
    }

    fn protected_interrupt_fixture(vector: u8, gate_access: u8, cpl: u8) -> (CpuState, VecBus) {
        let (mut cpu, mut bus) = protected_fault_fixture(vector, gate_access);
        let code_access = 0x9A | (cpl << 5);
        let data_access = 0x93 | (cpl << 5);

        bus.mem[PROTECTED_TEST_GDT + 8..PROTECTED_TEST_GDT + 16].copy_from_slice(&encode_seg_desc(
            0x0000_2000,
            0xFFFF,
            code_access,
            0,
        ));
        bus.mem[PROTECTED_TEST_GDT + 16..PROTECTED_TEST_GDT + 24]
            .copy_from_slice(&encode_seg_desc(0, 0xFFFF, code_access, 0));
        cpu.gdtr.limit = 23;
        cpu.idtr.limit = 0x07FF;
        cpu.cs = x86_core::SegmentReg {
            selector: 0x0010 | u16::from(cpl),
            base: 0,
            limit: 0xFFFF,
            flags: u16::from(code_access),
        };
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0018 | u16::from(cpl),
            base: 0,
            limit: 0xFFFF,
            flags: u16::from(data_access),
        };
        (cpu, bus)
    }

    /// Give pre-existing PE=1 descriptor-fault tests a valid protected-mode
    /// delivery target. Index 15 leaves their source descriptors at low GDT
    /// indices undisturbed; limit-fault tests use index 16.
    fn install_protected_test_exception_gate(
        mem: &mut [u8],
        cpu: &mut CpuState,
        vector: u8,
        handler: u16,
    ) {
        let descriptor_offset = usize::from(PROTECTED_COMPAT_TARGET_CS);
        let descriptor_addr = cpu.gdtr.base as usize + descriptor_offset;
        mem[descriptor_addr..descriptor_addr + 8]
            .copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x9A, 0));
        cpu.gdtr.limit = cpu.gdtr.limit.max((descriptor_offset + 7) as u16);

        let entry = PROTECTED_COMPAT_IDT + usize::from(vector) * 8;
        mem[entry..entry + 8].copy_from_slice(&encode_idt_gate(
            handler,
            PROTECTED_COMPAT_TARGET_CS,
            0x86,
        ));
        cpu.idtr.base = PROTECTED_COMPAT_IDT as u64;
        cpu.idtr.limit = u16::from(vector) * 8 + 7;
    }

    fn protected_far_jump_fixture(
        form: ProtectedFarJumpForm,
        offset: u16,
        selector: u16,
        descriptor: [u8; 8],
    ) -> (CpuState, VecBus) {
        let mut mem = vec![0u8; 0x10000];
        form.write(&mut mem, offset, selector);
        let descriptor_addr = PROTECTED_TEST_GDT + usize::from(selector >> 3) * 8;
        mem[descriptor_addr..descriptor_addr + 8].copy_from_slice(&descriptor);

        let mut cpu = CpuState::reset();
        cpu.cr0 |= 1;
        cpu.cs = x86_core::SegmentReg {
            selector: 0x0010,
            base: 0,
            limit: 0xFFFF,
            flags: 0x009A,
        };
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0018,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        cpu.ds = x86_core::SegmentReg {
            selector: 0x0018,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        cpu.gdtr.base = PROTECTED_TEST_GDT as u64;
        cpu.rip = PROTECTED_TEST_CODE as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_gpr_u16(CpuState::RAX, 0xA55A);
        cpu.set_gpr_u16(CpuState::RBX, 0x5AA5);

        // Install #NP before #GP so the final IDTR.limit includes both vectors.
        install_protected_test_exception_gate(&mut mem, &mut cpu, 11, 0x0B00);
        install_protected_test_exception_gate(&mut mem, &mut cpu, 13, 0x0D00);
        (cpu, VecBus { mem, ports: vec![] })
    }

    fn protected_iret_fixture(
        return_ip: u16,
        return_selector: u16,
        return_flags: u16,
        descriptor: [u8; 8],
    ) -> (CpuState, VecBus) {
        let mut mem = vec![0u8; 0x10000];
        mem[PROTECTED_TEST_CODE] = 0xCF;
        let descriptor_addr =
            PROTECTED_TEST_GDT + usize::from(return_selector >> 3).saturating_mul(8);
        mem[descriptor_addr..descriptor_addr + 8].copy_from_slice(&descriptor);
        let frame = usize::from(PROTECTED_IRET_FRAME_SP);
        mem[frame..frame + 2].copy_from_slice(&return_ip.to_le_bytes());
        mem[frame + 2..frame + 4].copy_from_slice(&return_selector.to_le_bytes());
        mem[frame + 4..frame + 6].copy_from_slice(&return_flags.to_le_bytes());

        let mut cpu = CpuState::reset();
        cpu.cr0 |= 1;
        cpu.cs = x86_core::SegmentReg {
            selector: 0x0010,
            base: 0,
            limit: 0xFFFF,
            flags: 0x009A,
        };
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0018,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        cpu.gdtr.base = PROTECTED_TEST_GDT as u64;
        cpu.gdtr.limit = 0x00FF;
        cpu.rip = PROTECTED_TEST_CODE as u64;
        cpu.set_gpr_u16(CpuState::RSP, PROTECTED_IRET_FRAME_SP);
        (cpu, VecBus { mem, ports: vec![] })
    }

    fn assert_bounded_protected_delivery_failure(
        cpu: &mut CpuState,
        bus: &mut dyn Bus,
        expected_detail: &str,
    ) {
        let error = step(cpu, bus).expect_err("invalid protected delivery must be bounded");
        // Exception-delivery failures escalate to `#DF`; when that also fails
        // the host sees `TripleFault`. Software/IRQ paths still report
        // `ProtectedModeExceptionDelivery` directly (and keep the detail).
        match &error {
            ExecError::ProtectedModeExceptionDelivery { .. } => {
                let message = error.to_string();
                assert!(
                    message.contains(expected_detail),
                    "unexpected delivery error: {message}"
                );
            }
            ExecError::TripleFault { reason } => {
                let _ = expected_detail;
                let _ = reason;
            }
            other => panic!("delivery failure escaped through the wrong error variant: {other:?}"),
        }
    }

    struct FailOnceWriteBus {
        mem: Vec<u8>,
        fail_addr: u64,
        failed: bool,
    }

    impl Bus for FailOnceWriteBus {
        fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
            self.mem
                .get(addr as usize)
                .copied()
                .ok_or(ExecError::MemoryFault(addr))
        }

        fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
            if addr == self.fail_addr && !self.failed {
                self.failed = true;
                return Err(ExecError::MemoryFault(addr));
            }
            let byte = self
                .mem
                .get_mut(addr as usize)
                .ok_or(ExecError::MemoryFault(addr))?;
            *byte = val;
            Ok(())
        }

        fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
            Ok(0xFF)
        }

        fn port_out_u8(&mut self, _port: u16, _val: u8) -> Result<(), ExecError> {
            Ok(())
        }
    }

    struct FailOnceReadBus {
        mem: Vec<u8>,
        fail_addr: u64,
        failed: bool,
    }

    impl Bus for FailOnceReadBus {
        fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
            if addr == self.fail_addr && !self.failed {
                self.failed = true;
                return Err(ExecError::MemoryFault(addr));
            }
            self.mem
                .get(addr as usize)
                .copied()
                .ok_or(ExecError::MemoryFault(addr))
        }

        fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
            let byte = self
                .mem
                .get_mut(addr as usize)
                .ok_or(ExecError::MemoryFault(addr))?;
            *byte = val;
            Ok(())
        }

        fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
            Ok(0xFF)
        }

        fn port_out_u8(&mut self, _port: u16, _val: u8) -> Result<(), ExecError> {
            Ok(())
        }
    }

    /// #UD, #DE, and #BR are faults without error codes.
    /// Spec: Intel SDM Vol. 3 §§6.13, 6.15.
    #[test]
    fn architectural_fault_payload_omits_error_codes_for_ud_de_br() {
        let cases: &[(u8, &[u8])] = &[
            (6, &[0xD0, 0xF0]),       // Group 2 /6 → #UD
            (0, &[0xD4, 0x00]),       // AAM 0 → #DE
            (5, &[0x62, 0x06, 0, 2]), // BOUND AX,[0x0200] → #BR
        ];

        for &(vector, code) in cases {
            let mut mem = vec![0u8; 0x10000];
            mem[..code.len()].copy_from_slice(code);
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_ax(1);
            let mut bus = VecBus { mem, ports: vec![] };

            assert_arch_fault(step_inner(&mut cpu, &mut bus), vector, None);
        }
    }

    /// A 16-bit interrupt gate pushes FLAGS, CS, and the faulting IP, then
    /// clears IF only after the protected-mode transfer succeeds.
    /// Spec: Intel SDM Vol. 3 §§6.11.2, 6.12.1, 6.15 (#UD).
    #[test]
    fn protected_fault_ud_interrupt_gate_frame_and_if() {
        let (mut cpu, mut bus) = protected_fault_fixture(6, 0x86);
        cpu.rflags |= (1 << 9) | 1;
        let saved_flags = cpu.rflags as u16;
        bus.mem[0xFFF6..0xFFF8].copy_from_slice(&0x5AA5u16.to_le_bytes());

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, PROTECTED_TEST_TARGET_CS);
        assert_eq!(cpu.cs.base, 0x2000);
        assert_eq!(cpu.cs.limit, 0xFFFF);
        assert_eq!(cpu.cs.flags, 0x009A);
        assert_eq!(cpu.ip16(), PROTECTED_TEST_HANDLER);
        assert!(!cpu.interrupt_flag(), "interrupt gate must clear IF");
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), PROTECTED_TEST_CODE as u16);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0010);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
        assert_eq!(
            bus.read_u16(0xFFF6).unwrap(),
            0x5AA5,
            "#UD must not push an error code"
        );
    }

    /// A selector #GP uses its imported payload as the final 16-bit frame word;
    /// a trap gate preserves IF. Final stack order is error, IP, CS, FLAGS.
    /// Spec: Intel SDM Vol. 3 §§6.11.2, 6.12.1, 6.13, 6.15 (#GP).
    #[test]
    fn protected_fault_gp_trap_gate_error_code_and_if() {
        let (mut cpu, mut bus) = protected_fault_fixture(13, 0x87);
        // MOV DS,AX with RPL 3 naming DPL-0 nonconforming code → #GP(0x10).
        bus.mem[PROTECTED_TEST_CODE] = 0x8E;
        bus.mem[PROTECTED_TEST_CODE + 1] = 0xD8;
        let invalid_data_target = encode_seg_desc(0, 0xFFFF, 0x9A, 0);
        bus.mem[PROTECTED_TEST_GDT + 16..PROTECTED_TEST_GDT + 24]
            .copy_from_slice(&invalid_data_target);
        cpu.gdtr.limit = 23;
        cpu.set_ax(0x0013);
        cpu.ds = x86_core::SegmentReg {
            selector: 0x0020,
            base: 0xABCD_0000,
            limit: 0x7FFF,
            flags: 0x0093,
        };
        let ds_before = cpu.ds.clone();
        cpu.set_interrupt_flag(true);
        cpu.rflags |= 1;
        let saved_flags = cpu.rflags as u16;

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, PROTECTED_TEST_TARGET_CS);
        assert_eq!(cpu.ip16(), PROTECTED_TEST_HANDLER);
        assert!(cpu.interrupt_flag(), "trap gate must preserve IF");
        assert_eq!(cpu.ds, ds_before);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF6);
        assert_eq!(bus.read_u16(0xFFF6).unwrap(), 0x0010);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), PROTECTED_TEST_CODE as u16);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0010);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
    }

    /// INT imm8, INT3, and taken INTO are software-generated interrupts. Their
    /// protected-mode frames save the following instruction, contain no error
    /// code, and use the selected 16-bit interrupt/trap gate semantics.
    ///
    /// Spec: Intel SDM Vol. 2 INT n/INT3/INTO (Operation); Vol. 3
    /// §§6.11.2, 6.12.1, 6.13.
    #[test]
    fn protected_software_interrupt_forms_save_next_ip_and_gate_flags() {
        let cases: [(&str, &[u8], u8, u8, bool); 3] = [
            ("INT imm8", &[0xCD, 0x30], 0x30, 0xE6, false),
            ("INT3", &[0xCC], 3, 0xE7, false),
            ("INTO", &[0xCE], 4, 0xE6, true),
        ];

        for (name, code, vector, gate_access, overflow) in cases {
            let (mut cpu, mut bus) = protected_interrupt_fixture(vector, gate_access, 3);
            bus.mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + code.len()].copy_from_slice(code);
            bus.mem[0xFFF6..0xFFF8].copy_from_slice(&0xA55Au16.to_le_bytes());
            cpu.rflags = 0x0203;
            cpu.set_of(overflow);
            let saved_flags = cpu.rflags as u16;

            step(&mut cpu, &mut bus).unwrap();

            assert_eq!(cpu.cs.selector, PROTECTED_TEST_TARGET_CS | 3, "{name}");
            assert_eq!(cpu.cs.base, 0x2000, "{name}");
            assert_eq!(cpu.ip16(), PROTECTED_TEST_HANDLER, "{name}");
            assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8, "{name}");
            assert_eq!(
                bus.read_u16(0xFFF8).unwrap(),
                (PROTECTED_TEST_CODE + code.len()) as u16,
                "{name}: saved IP"
            );
            assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0013, "{name}");
            assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags, "{name}");
            assert_eq!(
                bus.read_u16(0xFFF6).unwrap(),
                0xA55A,
                "{name}: software interrupt must not push an error code"
            );
            assert_eq!(
                cpu.interrupt_flag(),
                gate_access & 1 != 0,
                "{name}: interrupt/trap gate IF behavior"
            );
        }
    }

    /// Untaken INTO does not consult the IDT and changes only IP.
    /// Spec: Intel SDM Vol. 2 INTO (Operation).
    #[test]
    fn protected_into_without_overflow_falls_through_without_gate_access() {
        let (mut cpu, mut bus) = protected_interrupt_fixture(4, 0x06, 0);
        bus.mem[PROTECTED_TEST_CODE] = 0xCE;
        cpu.set_of(false);
        let cs_before = cpu.cs.clone();
        let flags_before = cpu.rflags;
        let sp_before = cpu.gpr_u16(CpuState::RSP);

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), PROTECTED_TEST_CODE as u16 + 1);
        assert_eq!(cpu.cs, cs_before);
        assert_eq!(cpu.rflags, flags_before);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp_before);
    }

    /// The IDT-gate DPL check applies to all three software forms. A CPL-3
    /// violation raises #GP with IDT=1, EXT=0, and the software vector as the
    /// index; the failed transfer writes no frame before #GP is delivered.
    ///
    /// Spec: Intel SDM Vol. 2 INT n/INT3/INTO (Protected Mode Exceptions);
    /// Vol. 3 §§6.12.1, 6.13.
    #[test]
    fn protected_software_gate_dpl_violation_delivers_gp_idt_payload_atomically() {
        let cases: [(&str, &[u8], u8, bool); 3] = [
            ("INT imm8", &[0xCD, 0x30], 0x30, false),
            ("INT3", &[0xCC], 3, false),
            ("INTO", &[0xCE], 4, true),
        ];

        for (name, code, vector, overflow) in cases {
            let (mut cpu, mut bus) = protected_interrupt_fixture(vector, 0x86, 3);
            bus.mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + code.len()].copy_from_slice(code);
            write_protected_test_gate(&mut bus.mem, 13, 0x0D00, PROTECTED_TEST_TARGET_CS, 0x87);
            cpu.rflags = 0x0203;
            cpu.set_of(overflow);
            let saved_flags = cpu.rflags as u16;

            step(&mut cpu, &mut bus).unwrap();

            assert_eq!(cpu.cs.selector, PROTECTED_TEST_TARGET_CS | 3, "{name}");
            assert_eq!(cpu.ip16(), 0x0D00, "{name}: #GP handler");
            assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF6, "{name}");
            assert_eq!(
                bus.read_u16(0xFFF6).unwrap(),
                (u16::from(vector) << 3) | 2,
                "{name}: #GP IDT error code"
            );
            assert_eq!(
                bus.read_u16(0xFFF8).unwrap(),
                PROTECTED_TEST_CODE as u16,
                "{name}: #GP must restart at the software instruction"
            );
            assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0013, "{name}");
            assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags, "{name}");
        }
    }

    /// Gate DPL is ignored for NMI and maskable hardware interrupts. Both can
    /// enter a same-CPL D=0 target even when CPL=3 and gate DPL=0.
    ///
    /// Spec: Intel SDM Vol. 3 §§6.3.3, 6.8.1, 6.12.1.
    #[test]
    fn protected_hardware_interrupts_ignore_gate_dpl() {
        let (mut nmi_cpu, mut nmi_bus) = protected_interrupt_fixture(2, 0x86, 3);
        nmi_cpu.set_interrupt_flag(false);
        nmi_cpu.request_nmi();
        step(&mut nmi_cpu, &mut nmi_bus).unwrap();
        assert_eq!(nmi_cpu.cs.selector, PROTECTED_TEST_TARGET_CS | 3);
        assert_eq!(nmi_cpu.ip16(), PROTECTED_TEST_HANDLER);
        assert_eq!(nmi_cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(
            nmi_bus.read_u16(0xFFF8).unwrap(),
            PROTECTED_TEST_CODE as u16
        );

        let (mut irq_cpu, irq_fixture) = protected_interrupt_fixture(0x20, 0x86, 3);
        let mut irq_bus = IrqAfterWritesBus {
            mem: irq_fixture.mem,
            ports: vec![],
            writes: 0,
            inject_after_writes: usize::MAX,
            inject_vector: 0x20,
            latched: Some(0x20),
        };
        irq_cpu.set_interrupt_flag(true);
        step(&mut irq_cpu, &mut irq_bus).unwrap();
        assert_eq!(irq_cpu.cs.selector, PROTECTED_TEST_TARGET_CS | 3);
        assert_eq!(irq_cpu.ip16(), PROTECTED_TEST_HANDLER);
        assert_eq!(irq_cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(
            irq_bus.read_u16(0xFFF8).unwrap(),
            PROTECTED_TEST_CODE as u16
        );
    }

    /// NMI wins over a simultaneous maskable IRQ. The NMI interrupt gate clears
    /// IF, IRET restores it, and the still-pending IRQ then enters its trap gate.
    /// Both hardware frames save the current interrupted IP and no error code.
    ///
    /// Spec: Intel SDM Vol. 2 IRET; Vol. 3 §§6.3.3, 6.7, 6.8.1, 6.12.1.
    #[test]
    fn protected_nmi_precedes_irq_and_both_iret_to_interrupted_ip() {
        const NMI_HANDLER: u16 = 0x0200;
        const IRQ_HANDLER: u16 = 0x0300;
        let (mut cpu, mut bus) = protected_interrupt_fixture(2, 0x86, 0);
        write_protected_test_gate(&mut bus.mem, 2, NMI_HANDLER, PROTECTED_TEST_TARGET_CS, 0x86);
        write_protected_test_gate(
            &mut bus.mem,
            0x20,
            IRQ_HANDLER,
            PROTECTED_TEST_TARGET_CS,
            0x87,
        );
        bus.mem[0x2000 + usize::from(NMI_HANDLER)] = 0xCF;
        bus.mem[0x2000 + usize::from(IRQ_HANDLER)] = 0xCF;
        bus.mem[PROTECTED_TEST_CODE] = 0x90;
        bus.mem[0xFFF6..0xFFF8].copy_from_slice(&0x5AA5u16.to_le_bytes());
        cpu.rflags = 0x0203;
        cpu.request_interrupt(0x20);
        cpu.request_nmi();

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), NMI_HANDLER);
        assert!(!cpu.pending_nmi);
        assert_eq!(cpu.pending_irq, Some(0x20));
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), PROTECTED_TEST_CODE as u16);
        assert_eq!(bus.read_u16(0xFFF6).unwrap(), 0x5AA5);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0x0010);
        assert_eq!(cpu.ip16(), PROTECTED_TEST_CODE as u16);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
        assert!(cpu.interrupt_flag());
        assert_eq!(cpu.pending_irq, Some(0x20));

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), IRQ_HANDLER);
        assert_eq!(cpu.pending_irq, None);
        assert!(cpu.interrupt_flag(), "trap gate must preserve IF");
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), PROTECTED_TEST_CODE as u16);
        assert_eq!(bus.read_u16(0xFFF6).unwrap(), 0x5AA5);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0x0010);
        assert_eq!(cpu.ip16(), PROTECTED_TEST_CODE as u16);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
        assert!(cpu.interrupt_flag());
    }

    /// IF=0 leaves a pending PIC vector latched while the current instruction
    /// executes. Once IF is set, delivery saves the then-current IP.
    /// Spec: Intel SDM Vol. 3 §§6.8.1, 6.12.1.
    #[test]
    fn protected_irq_respects_if_and_saves_current_ip() {
        let (mut cpu, mut bus) = protected_interrupt_fixture(0x20, 0x86, 0);
        bus.mem[PROTECTED_TEST_CODE] = 0x90;
        bus.mem[PROTECTED_TEST_CODE + 1] = 0x90;
        cpu.set_interrupt_flag(false);
        cpu.request_interrupt(0x20);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), PROTECTED_TEST_CODE as u16 + 1);
        assert_eq!(cpu.pending_irq, Some(0x20));
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        cpu.set_interrupt_flag(true);
        let saved_flags = cpu.rflags as u16;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), PROTECTED_TEST_HANDLER);
        assert_eq!(cpu.pending_irq, None);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(
            bus.read_u16(0xFFF8).unwrap(),
            PROTECTED_TEST_CODE as u16 + 1
        );
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
    }

    /// A valid NMI or IF-enabled IRQ wakes HLT, saves the post-HLT IP, and can
    /// return through the imported same-CPL IRET16 path.
    ///
    /// Spec: Intel SDM Vol. 2 HLT/IRET; Vol. 3 §§6.3.3, 6.8.1, 6.12.1.
    #[test]
    fn protected_nmi_and_irq_wake_hlt_and_iret_after_hlt() {
        for (name, vector, nmi) in [("NMI", 2, true), ("IRQ", 0x20, false)] {
            let (mut cpu, mut bus) = protected_interrupt_fixture(vector, 0x86, 0);
            bus.mem[PROTECTED_TEST_CODE] = 0xF4;
            bus.mem[PROTECTED_TEST_CODE + 1] = 0x90;
            bus.mem[0x2000 + usize::from(PROTECTED_TEST_HANDLER)] = 0xCF;
            cpu.set_interrupt_flag(!nmi);

            step(&mut cpu, &mut bus).unwrap();
            assert!(cpu.halted, "{name}");
            assert_eq!(cpu.ip16(), PROTECTED_TEST_CODE as u16 + 1, "{name}");
            let saved_flags = cpu.rflags as u16;
            if nmi {
                cpu.request_nmi();
            } else {
                cpu.request_interrupt(vector);
            }

            step(&mut cpu, &mut bus).unwrap();
            assert!(!cpu.halted, "{name}: valid delivery must wake HLT");
            assert_eq!(cpu.ip16(), PROTECTED_TEST_HANDLER, "{name}");
            assert_eq!(
                bus.read_u16(0xFFF8).unwrap(),
                PROTECTED_TEST_CODE as u16 + 1,
                "{name}: saved post-HLT IP"
            );
            assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags, "{name}");

            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.cs.selector, 0x0010, "{name}");
            assert_eq!(cpu.ip16(), PROTECTED_TEST_CODE as u16 + 1, "{name}");
            assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE, "{name}");
            assert_eq!(cpu.interrupt_flag(), !nmi, "{name}");
        }
    }

    /// Newly routed software/NMI/IRQ callers retain the imported helper's
    /// transactional behavior for invalid gate, target, and stack cases.
    /// Pending hardware requests and HLT are committed only after valid entry.
    ///
    /// Spec: Intel SDM Vol. 3 §§6.11.2, 6.12.1.
    #[test]
    fn protected_interrupt_delivery_failures_remain_atomic() {
        // 0x5 is a task gate — task-based delivery is out of scope.
        let (mut software_cpu, mut software_bus) = protected_interrupt_fixture(0x30, 0x85, 0);
        software_bus.mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + 2]
            .copy_from_slice(&[0xCD, 0x30]);
        let software_before = software_cpu.clone();
        let software_mem_before = software_bus.mem.clone();
        let error = step(&mut software_cpu, &mut software_bus).unwrap_err();
        assert!(matches!(
            error,
            ExecError::ProtectedModeExceptionDelivery {
                vector: 0x30,
                reason: ProtectedModeDeliveryError::GateType(0x85)
            }
        ));
        assert_eq!(software_cpu, software_before);
        assert_eq!(software_bus.mem, software_mem_before);

        let (mut nmi_cpu, mut nmi_bus) = protected_interrupt_fixture(2, 0x86, 0);
        nmi_bus.mem[PROTECTED_TEST_GDT + 8..PROTECTED_TEST_GDT + 16]
            .copy_from_slice(&encode_seg_desc(0x2000, 0xFFFF, 0x1A, 0));
        nmi_cpu.halted = true;
        nmi_cpu.request_nmi();
        let nmi_before = nmi_cpu.clone();
        let nmi_mem_before = nmi_bus.mem.clone();
        let error = step(&mut nmi_cpu, &mut nmi_bus).unwrap_err();
        assert!(matches!(
            error,
            ExecError::ProtectedModeExceptionDelivery {
                vector: 2,
                reason: ProtectedModeDeliveryError::TargetNotPresent
            }
        ));
        assert_eq!(nmi_cpu, nmi_before);
        assert_eq!(nmi_bus.mem, nmi_mem_before);

        let (mut irq_cpu, mut irq_bus) = protected_interrupt_fixture(0x20, 0x86, 0);
        irq_cpu.set_interrupt_flag(true);
        irq_cpu.ss.limit = 0xFFFC;
        irq_cpu.halted = true;
        irq_cpu.request_interrupt(0x20);
        let irq_before = irq_cpu.clone();
        let irq_mem_before = irq_bus.mem.clone();
        let error = step(&mut irq_cpu, &mut irq_bus).unwrap_err();
        assert!(matches!(
            error,
            ExecError::ProtectedModeExceptionDelivery {
                vector: 0x20,
                reason: ProtectedModeDeliveryError::StackLimit
            }
        ));
        assert_eq!(irq_cpu, irq_before);
        assert_eq!(irq_bus.mem, irq_mem_before);
    }

    /// Both 16-bit gate types round-trip a software INT frame through IRET.
    /// Spec: Intel SDM Vol. 2 INT n/IRET; Vol. 3 §§6.11.2, 6.12.1.
    #[test]
    fn protected_software_interrupt_gate_types_round_trip_through_iret16() {
        for (name, gate_access) in [("interrupt", 0x86), ("trap", 0x87)] {
            let (mut cpu, mut bus) = protected_interrupt_fixture(0x30, gate_access, 0);
            bus.mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + 2].copy_from_slice(&[0xCD, 0x30]);
            bus.mem[0x2000 + usize::from(PROTECTED_TEST_HANDLER)] = 0xCF;
            cpu.rflags = 0x0AD7;
            let mut expected = cpu.clone();
            expected.rip = PROTECTED_TEST_CODE as u64 + 2;

            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.interrupt_flag(), gate_access & 1 != 0, "{name} gate");
            step(&mut cpu, &mut bus).unwrap();

            assert_eq!(cpu, expected, "{name} gate round trip");
        }
    }

    /// IDT bounds, 16-bit gate type, and P are checked before any frame write.
    /// Spec: Intel SDM Vol. 3 §§6.10, 6.11.2, 6.12.1.
    #[test]
    fn protected_fault_gate_validation_is_atomic() {
        let cases = [
            ("IDT limit", 0x86, Some(6 * 8 + 6), "IDT limit"),
            // 0x5 is a task gate; task-based delivery is out of scope.
            ("task gate type", 0x85, None, "interrupt/trap gate"),
            ("not-present gate", 0x06, None, "gate is not present"),
        ];

        for (name, access, idt_limit, detail) in cases {
            let (mut cpu, mut bus) = protected_fault_fixture(6, access);
            if let Some(limit) = idt_limit {
                cpu.idtr.limit = limit;
            }
            let cpu_before = cpu.clone();
            let mem_before = bus.mem.clone();

            assert_bounded_protected_delivery_failure(&mut cpu, &mut bus, detail);
            assert_eq!(cpu, cpu_before, "{name}: CPU state changed");
            assert_eq!(bus.mem, mem_before, "{name}: guest memory changed");
        }
    }

    /// This bounded same-CPL model accepts only a non-null GDT selector for a
    /// present ring-0, D=0 code segment containing the gate offset.
    /// Spec: Intel SDM Vol. 3 §§6.11.2, 6.12.1, 6.12.3.
    #[test]
    fn protected_fault_target_descriptor_validation_is_atomic() {
        let valid = encode_seg_desc(0x2000, 0xFFFF, 0x9A, 0);
        let cases = [
            (
                "null selector",
                0x0000,
                valid,
                15,
                PROTECTED_TEST_HANDLER,
                "null target selector",
            ),
            (
                "LDT selector",
                0x000C,
                valid,
                15,
                PROTECTED_TEST_HANDLER,
                "LDT target selector",
            ),
            (
                "GDT limit",
                0x0008,
                valid,
                7,
                PROTECTED_TEST_HANDLER,
                "GDT limit",
            ),
            (
                "not present",
                0x0008,
                encode_seg_desc(0x2000, 0xFFFF, 0x1A, 0),
                15,
                PROTECTED_TEST_HANDLER,
                "target code segment is not present",
            ),
            (
                "data segment",
                0x0008,
                encode_seg_desc(0x2000, 0xFFFF, 0x92, 0),
                15,
                PROTECTED_TEST_HANDLER,
                "usable executable code",
            ),
            (
                "ring 1 code",
                0x0008,
                encode_seg_desc(0x2000, 0xFFFF, 0xBA, 0),
                15,
                PROTECTED_TEST_HANDLER,
                "usable executable code",
            ),
            (
                "default-32 code",
                0x0008,
                encode_seg_desc(0x2000, 0xFFFF, 0x9A, 0x40),
                15,
                PROTECTED_TEST_HANDLER,
                "16-bit gate target descriptor is not a 16-bit code segment",
            ),
            (
                "offset past limit",
                0x0008,
                encode_seg_desc(0x2000, 0x00FF, 0x9A, 0),
                15,
                0x0100,
                "target offset exceeds",
            ),
        ];

        for (name, selector, descriptor, gdt_limit, offset, detail) in cases {
            let (mut cpu, mut bus) = protected_fault_fixture(6, 0x86);
            let descriptor_addr = PROTECTED_TEST_GDT + usize::from(selector >> 3) * 8;
            bus.mem[descriptor_addr..descriptor_addr + 8].copy_from_slice(&descriptor);
            write_protected_test_gate(&mut bus.mem, 6, offset, selector, 0x86);
            cpu.gdtr.limit = gdt_limit;
            let cpu_before = cpu.clone();
            let mem_before = bus.mem.clone();

            assert_bounded_protected_delivery_failure(&mut cpu, &mut bus, detail);
            assert_eq!(cpu, cpu_before, "{name}: CPU state changed");
            assert_eq!(bus.mem, mem_before, "{name}: guest memory changed");
        }
    }

    /// A same-stack frame that exceeds SS.limit fails before changing CPU or RAM.
    /// Spec: Intel SDM Vol. 3 §6.12.1; bounded nested-fault policy for this slice.
    #[test]
    fn protected_fault_stack_limit_failure_is_atomic() {
        let (mut cpu, mut bus) = protected_fault_fixture(6, 0x86);
        cpu.ss.limit = 0xFFFC;
        let cpu_before = cpu.clone();
        let mem_before = bus.mem.clone();

        assert_bounded_protected_delivery_failure(&mut cpu, &mut bus, "stack limit");
        assert_eq!(cpu, cpu_before);
        assert_eq!(bus.mem, mem_before);
    }

    /// If a later stack byte write fails, earlier bytes are restored and no
    /// architectural state is committed. This slice does not synthesize #DF.
    /// Spec: Intel SDM Vol. 3 §6.12.1; bounded nested-fault policy for this slice.
    #[test]
    fn protected_fault_stack_write_failure_rolls_back() {
        let (mut cpu, fixture) = protected_fault_fixture(6, 0x86);
        let mut bus = FailOnceWriteBus {
            mem: fixture.mem,
            // FLAGS at FFFC succeeds; the first CS byte then fails once.
            fail_addr: 0xFFFA,
            failed: false,
        };
        let cpu_before = cpu.clone();
        let mem_before = bus.mem.clone();

        assert_bounded_protected_delivery_failure(&mut cpu, &mut bus, "stack write");
        assert_eq!(cpu, cpu_before);
        assert_eq!(
            bus.mem, mem_before,
            "partial frame bytes were not rolled back"
        );
    }

    /// Same-CPL ring-0 IRET with a 16-bit operand restores the complete target
    /// CS cache and SP only after validating the full frame and descriptor.
    /// At CPL 0, IF and IOPL are writable; FLAGS bits 1/3/5/15 retain their
    /// architectural fixed values while EFLAGS[31:16] remain unchanged.
    ///
    /// Spec: Intel SDM Vol. 2 IRET/IRETD/IRETQ (Operation, protected mode);
    /// Vol. 1 §3.4.3; Vol. 3 §§2.3.1, 3.4.2–3.4.5, 6.12.1.
    #[test]
    fn protected_iret16_same_cpl_restores_cache_sp_and_flags() {
        let descriptor = encode_seg_desc(
            PROTECTED_IRET_RETURN_BASE,
            0,
            0x9A,
            0x90, // G=1, D/B=0, L=0, AVL=1
        );
        let frame_flags = 0xBFFD; // IF+IOPL=3; reserved bits 3/5/15 deliberately set.
        let (mut cpu, mut bus) =
            protected_iret_fixture(0x0FFE, PROTECTED_IRET_RETURN_CS, frame_flags, descriptor);
        let high_flags = 0x0000_0000_003D_0000; // RF/AC/VIF/VIP/ID, VM=0.
        cpu.rflags = high_flags | 0x802A; // NT=0; low reserved bits start noncanonical.

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 0x0FFE);
        assert_eq!(cpu.cs.selector, PROTECTED_IRET_RETURN_CS);
        assert_eq!(cpu.cs.base, u64::from(PROTECTED_IRET_RETURN_BASE));
        assert_eq!(cpu.cs.limit, 0x0FFF);
        assert_eq!(cpu.cs.flags, 0x909A);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), PROTECTED_IRET_FRAME_SP + 6);
        assert_eq!(cpu.rflags, high_flags | 0x3FD7);
        assert!(cpu.interrupt_flag(), "CPL-0 IRET must restore IF");
    }

    /// A 16-bit interrupt gate clears IF on entry; IRET restores the saved
    /// same-CPL frame, including IF and the original CS descriptor cache.
    ///
    /// Spec: Intel SDM Vol. 2 IRET/IRETD/IRETQ; Vol. 3 §§6.11.2, 6.12.1.
    #[test]
    fn protected_iret16_round_trips_interrupt_gate_and_if() {
        let (mut cpu, mut bus) = protected_fault_fixture(6, 0x86);
        bus.mem[PROTECTED_TEST_GDT + 16..PROTECTED_TEST_GDT + 24]
            .copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x9A, 0));
        cpu.gdtr.limit = 23;
        let handler = 0x2000usize + usize::from(PROTECTED_TEST_HANDLER);
        bus.mem[handler] = 0xCF;
        cpu.rflags = 0x0AD7;
        let expected = cpu.clone();

        step(&mut cpu, &mut bus).unwrap();
        assert!(!cpu.interrupt_flag(), "interrupt gate did not clear IF");
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu, expected);
    }

    /// Error-code frames are not special-cased by IRET. The trap handler
    /// explicitly executes ADD SP,2 before IRET, then returns to the faulting
    /// instruction with the original CS:IP, SP, FLAGS, and segment cache.
    ///
    /// Spec: Intel SDM Vol. 2 IRET/IRETD/IRETQ; Vol. 3 §§6.12.1, 6.13.
    #[test]
    fn protected_iret16_round_trips_trap_gate_after_error_discard() {
        let (mut cpu, mut bus) = protected_fault_fixture(13, 0x87);
        // MOV DS,AX with an execute-only code descriptor raises #GP(0x18).
        bus.mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + 2].copy_from_slice(&[0x8E, 0xD8]);
        bus.mem[PROTECTED_TEST_GDT + 16..PROTECTED_TEST_GDT + 24]
            .copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x9A, 0));
        bus.mem[PROTECTED_TEST_GDT + 24..PROTECTED_TEST_GDT + 32]
            .copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x98, 0));
        cpu.gdtr.limit = 31;
        cpu.set_ax(0x0018);
        cpu.rflags = 0x0AD7;
        let handler = 0x2000usize + usize::from(PROTECTED_TEST_HANDLER);
        // ADD SP,2 discards #GP's error code; IRET consumes only IP, CS, FLAGS.
        bus.mem[handler..handler + 4].copy_from_slice(&[0x83, 0xC4, 0x02, 0xCF]);
        let expected = cpu.clone();

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF6);
        assert_eq!(bus.read_u16(0xFFF6).unwrap(), 0x0018);
        assert!(cpu.interrupt_flag(), "trap gate must preserve IF");

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu, expected);
    }

    /// Selector, table, type, presence, privilege, width, and target-limit
    /// checks fault before any architectural return state is committed.
    /// Selector errors preserve TI/index and clear RPL; target-limit #GP uses 0.
    ///
    /// Spec: Intel SDM Vol. 2 IRET/IRETD/IRETQ (Protected Mode Exceptions);
    /// Vol. 3 §§3.4.2–3.4.5, 5.5, 6.13, 6.15.
    #[test]
    fn protected_iret16_validation_faults_are_atomic() {
        let valid = encode_seg_desc(PROTECTED_IRET_RETURN_BASE, 0xFFFF, 0x9A, 0);
        let cases = [
            ("null selector", 0x0003, valid, 0x00FF, 0x0100, 13, 0x0000),
            ("LDT selector", 0x0024, valid, 0x00FF, 0x0100, 13, 0x0024),
            ("GDT limit", 0x0083, valid, 0x007F, 0x0100, 13, 0x0080),
            (
                "data descriptor",
                0x0020,
                encode_seg_desc(PROTECTED_IRET_RETURN_BASE, 0xFFFF, 0x92, 0),
                0x00FF,
                0x0100,
                13,
                0x0020,
            ),
            (
                "system descriptor",
                0x0020,
                encode_seg_desc(PROTECTED_IRET_RETURN_BASE, 0xFFFF, 0x8A, 0),
                0x00FF,
                0x0100,
                13,
                0x0020,
            ),
            (
                "not present",
                0x0020,
                encode_seg_desc(PROTECTED_IRET_RETURN_BASE, 0xFFFF, 0x1A, 0),
                0x00FF,
                0x0100,
                11,
                0x0020,
            ),
            (
                "DPL mismatch",
                0x0020,
                encode_seg_desc(PROTECTED_IRET_RETURN_BASE, 0xFFFF, 0xBA, 0),
                0x00FF,
                0x0100,
                13,
                0x0020,
            ),
            ("RPL mismatch", 0x0021, valid, 0x00FF, 0x0100, 13, 0x0020),
            (
                "conforming code",
                0x0020,
                encode_seg_desc(PROTECTED_IRET_RETURN_BASE, 0xFFFF, 0x9E, 0),
                0x00FF,
                0x0100,
                13,
                0x0020,
            ),
            // `D=1` return segments are accepted since the IRETD slice; `L=1`
            // (64-bit) code segments remain rejected here.
            (
                "long-mode code",
                0x0020,
                encode_seg_desc(PROTECTED_IRET_RETURN_BASE, 0xFFFF, 0x9A, 0x20),
                0x00FF,
                0x0100,
                13,
                0x0020,
            ),
            (
                "effective limit",
                0x0020,
                encode_seg_desc(PROTECTED_IRET_RETURN_BASE, 0, 0x9A, 0x80),
                0x00FF,
                0x1000,
                13,
                0x0000,
            ),
        ];

        for &(name, selector, descriptor, gdt_limit, ip, vector, error_code) in &cases {
            let (mut cpu, mut bus) = protected_iret_fixture(ip, selector, 0x0203, descriptor);
            cpu.gdtr.limit = gdt_limit;
            let cpu_before = cpu.clone();

            assert_arch_fault(step_inner(&mut cpu, &mut bus), vector, Some(error_code));
            assert_eq!(cpu, cpu_before, "{name}: IRET state partially committed");
        }
    }

    /// Every IP/CS/FLAGS byte is read from the old SS:SP before SP or any
    /// return state changes. A limit failure, truncated frame, or late bus
    /// failure therefore raises #SS(0) with the complete CPU state unchanged.
    ///
    /// Spec: Intel SDM Vol. 2 IRET/IRETD/IRETQ (Protected Mode Exceptions);
    /// Vol. 3 §§5.3, 6.12.1, 6.15.
    #[test]
    fn protected_iret16_stack_frame_reads_are_atomic() {
        let valid = encode_seg_desc(PROTECTED_IRET_RETURN_BASE, 0xFFFF, 0x9A, 0);

        let (mut cpu, mut bus) =
            protected_iret_fixture(0x0100, PROTECTED_IRET_RETURN_CS, 0x0203, valid);
        cpu.ss.limit = u32::from(PROTECTED_IRET_FRAME_SP) + 4;
        let cpu_before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 12, Some(0));
        assert_eq!(cpu, cpu_before, "stack-limit fault changed CPU state");

        let (mut cpu, mut bus) = protected_iret_fixture(0x0100, 0, 0x0203, valid);
        bus.mem.truncate(usize::from(PROTECTED_IRET_FRAME_SP) + 5);
        let cpu_before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 12, Some(0));
        assert_eq!(
            cpu, cpu_before,
            "truncated FLAGS read changed CPU state or validated CS too early"
        );

        let (mut cpu, fixture) =
            protected_iret_fixture(0x0100, PROTECTED_IRET_RETURN_CS, 0x0203, valid);
        let mut bus = FailOnceReadBus {
            mem: fixture.mem,
            fail_addr: u64::from(PROTECTED_IRET_FRAME_SP) + 5,
            failed: false,
        };
        let cpu_before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 12, Some(0));
        assert_eq!(cpu, cpu_before, "late FLAGS bus fault changed CPU state");
    }

    /// Both direct far-JMP16 encodings load a same-level, present,
    /// nonconforming D=0 GDT code descriptor. G makes raw limit 0 effective as
    /// 0xFFF, and the complete cached attributes are preserved. JMP leaves
    /// FLAGS and every unrelated field unchanged.
    ///
    /// Spec: Intel SDM Vol. 2 JMP (far direct forms, Operation); Vol. 3
    /// §§3.4.5, 5.8.1.
    #[test]
    fn protected_far_jump16_direct_forms_load_cs_cache_atomically() {
        let descriptor = encode_seg_desc(
            PROTECTED_FAR_JUMP_TARGET_BASE,
            0,
            0x9A,
            0x90, // G=1, D/B=0, L=0, AVL=1
        );

        for form in ProtectedFarJumpForm::ALL {
            let (mut cpu, mut bus) =
                protected_far_jump_fixture(form, 0x0FFE, PROTECTED_FAR_JUMP_TARGET_CS, descriptor);
            cpu.rflags = 0x0000_0000_0000_A5D7;
            let mut expected = cpu.clone();
            expected.cs.load_descriptor_cache(
                PROTECTED_FAR_JUMP_TARGET_CS,
                u64::from(PROTECTED_FAR_JUMP_TARGET_BASE),
                0x0FFF,
                0x909A,
            );
            expected.rip = 0x0FFE;

            step(&mut cpu, &mut bus).unwrap();

            assert_eq!(cpu, expected, "{} changed unrelated state", form.name());
        }
    }

    /// Selector/table/type/presence/privilege checks precede CS:IP commit for
    /// both direct forms. Selector-derived #GP/#NP codes clear RPL bits but
    /// retain TI/index; an offset beyond the effective segment limit is #GP(0).
    /// D=1 and conforming segments are deliberately rejected by this bounded
    /// D=0 nonconforming slice.
    ///
    /// Spec: Intel SDM Vol. 2 JMP, Protected Mode Exceptions; Vol. 3
    /// §§3.4.5, 5.8.1, 6.13.
    #[test]
    fn protected_far_jump16_validation_faults_are_atomic_and_delivered() {
        let valid = encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0xFFFF, 0x9A, 0);
        let cases = [
            ("null selector", 0x0000, valid, 0x007F, 0x0100, 13, 0x0000),
            ("LDT selector", 0x0024, valid, 0x007F, 0x0100, 13, 0x0024),
            ("GDT limit", 0x0080, valid, 0x007F, 0x0100, 13, 0x0080),
            (
                "data descriptor",
                0x0020,
                encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0xFFFF, 0x92, 0),
                0x007F,
                0x0100,
                13,
                0x0020,
            ),
            (
                "system descriptor",
                0x0020,
                encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0xFFFF, 0x82, 0),
                0x007F,
                0x0100,
                13,
                0x0020,
            ),
            (
                "not present",
                0x0020,
                encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0xFFFF, 0x1A, 0),
                0x007F,
                0x0100,
                11,
                0x0020,
            ),
            (
                "DPL mismatch",
                0x0020,
                encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0xFFFF, 0xBA, 0),
                0x007F,
                0x0100,
                13,
                0x0020,
            ),
            ("RPL mismatch", 0x0021, valid, 0x007F, 0x0100, 13, 0x0020),
            (
                "conforming code",
                0x0020,
                encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0xFFFF, 0x9E, 0),
                0x007F,
                0x0100,
                13,
                0x0020,
            ),
            // `D=1` targets are accepted since the default-32 execution slice;
            // `L=1` (64-bit) code segments remain rejected here.
            (
                "long-mode code",
                0x0020,
                encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0xFFFF, 0x9A, 0x20),
                0x007F,
                0x0100,
                13,
                0x0020,
            ),
            (
                "effective limit",
                0x0020,
                encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0, 0x9A, 0x80),
                0x007F,
                0x1000,
                13,
                0x0000,
            ),
        ];

        for form in ProtectedFarJumpForm::ALL {
            for &(name, selector, descriptor, gdt_limit, offset, vector, error_code) in &cases {
                let (mut cpu, mut bus) =
                    protected_far_jump_fixture(form, offset, selector, descriptor);
                cpu.gdtr.limit = gdt_limit;
                let cpu_before = cpu.clone();

                assert_arch_fault(step_inner(&mut cpu, &mut bus), vector, Some(error_code));
                assert_eq!(
                    cpu,
                    cpu_before,
                    "{} {name}: target state committed before fault",
                    form.name()
                );

                let (mut cpu, mut bus) =
                    protected_far_jump_fixture(form, offset, selector, descriptor);
                cpu.gdtr.limit = gdt_limit;
                cpu.rflags = 0x0000_0000_0000_8A57;
                let saved_flags = cpu.rflags as u16;

                step(&mut cpu, &mut bus).unwrap();

                assert_eq!(
                    cpu.cs.selector,
                    PROTECTED_COMPAT_TARGET_CS,
                    "{} {name}: protected fault gate was not entered",
                    form.name()
                );
                assert_eq!(cpu.cs.base, 0);
                assert_eq!(cpu.cs.limit, 0xFFFF);
                assert_eq!(cpu.cs.flags, 0x009A);
                assert_eq!(cpu.ip16(), if vector == 11 { 0x0B00 } else { 0x0D00 });
                assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF6);
                assert_eq!(bus.read_u16(0xFFF6).unwrap(), error_code);
                assert_eq!(bus.read_u16(0xFFF8).unwrap(), PROTECTED_TEST_CODE as u16);
                assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0010);
                assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
            }
        }
    }

    /// Fetch truncation and a partial m16:16 operand read fault before CS:IP
    /// commit. These tests use `step_inner` so the faulting architectural state
    /// can be compared directly, before the protected #GP gate changes CS:IP.
    ///
    /// Spec: Intel SDM Vol. 2 JMP, Protected Mode Exceptions; Vol. 3 §6.13.
    #[test]
    fn protected_far_jump16_fetch_and_pointer_truncation_are_atomic() {
        let descriptor = encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0xFFFF, 0x9A, 0);

        for form in ProtectedFarJumpForm::ALL {
            let (mut cpu, mut bus) =
                protected_far_jump_fixture(form, 0x0100, PROTECTED_FAR_JUMP_TARGET_CS, descriptor);
            bus.mem
                .truncate(PROTECTED_TEST_CODE + form.instruction_len() - 1);
            let cpu_before = cpu.clone();

            assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
            assert_eq!(
                cpu,
                cpu_before,
                "{} fetch truncation changed CPU state",
                form.name()
            );
        }

        let (mut cpu, mut bus) = protected_far_jump_fixture(
            ProtectedFarJumpForm::Memory,
            0x0100,
            PROTECTED_FAR_JUMP_TARGET_CS,
            descriptor,
        );
        bus.mem.truncate(0x3003); // selector high byte is missing
        let cpu_before = cpu.clone();

        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, cpu_before, "partial far pointer changed CPU state");
    }

    /// A late pointer-byte or descriptor-byte read failure becomes #GP(0) only
    /// after the complete target transfer has remained uncommitted. The
    /// fail-once bus then permits the imported protected fault gate to deliver
    /// that payload and expose the original CS:IP in its frame.
    ///
    /// Spec: Intel SDM Vol. 2 JMP, Protected Mode Exceptions; Vol. 3
    /// §§5.8.1, 6.13.
    #[test]
    fn protected_far_jump16_memory_read_faults_do_not_partially_commit() {
        let descriptor = encode_seg_desc(PROTECTED_FAR_JUMP_TARGET_BASE, 0xFFFF, 0x9A, 0);

        let (mut cpu, fixture) = protected_far_jump_fixture(
            ProtectedFarJumpForm::Memory,
            0x0100,
            PROTECTED_FAR_JUMP_TARGET_CS,
            descriptor,
        );
        let mut bus = FailOnceReadBus {
            mem: fixture.mem,
            fail_addr: 0x3003,
            failed: false,
        };
        let saved_flags = cpu.rflags as u16;

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(cpu.ip16(), 0x0D00);
        assert_eq!(bus.read_u16(0xFFF6).unwrap(), 0);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), PROTECTED_TEST_CODE as u16);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0010);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);

        for form in ProtectedFarJumpForm::ALL {
            let (mut cpu, fixture) =
                protected_far_jump_fixture(form, 0x0100, PROTECTED_FAR_JUMP_TARGET_CS, descriptor);
            let mut bus = FailOnceReadBus {
                mem: fixture.mem,
                fail_addr: (PROTECTED_TEST_GDT + usize::from(PROTECTED_FAR_JUMP_TARGET_CS) + 7)
                    as u64,
                failed: false,
            };
            let saved_flags = cpu.rflags as u16;

            step(&mut cpu, &mut bus).unwrap();

            assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
            assert_eq!(cpu.ip16(), 0x0D00);
            assert_eq!(bus.read_u16(0xFFF6).unwrap(), 0);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), PROTECTED_TEST_CODE as u16);
            assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0010);
            assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
        }
    }

    /// Test bus: latch an external IRQ after N successful `write_u8` calls.
    /// Used to exercise REP interruptibility between iterations (PIC stub).
    struct IrqAfterWritesBus {
        mem: Vec<u8>,
        ports: Vec<u8>,
        writes: usize,
        inject_after_writes: usize,
        inject_vector: u8,
        latched: Option<u8>,
    }

    impl Bus for IrqAfterWritesBus {
        fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            Ok(self.mem[i])
        }
        fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            self.mem[i] = val;
            self.writes = self.writes.saturating_add(1);
            if self.writes == self.inject_after_writes {
                self.latched = Some(self.inject_vector);
            }
            Ok(())
        }
        fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
            Ok(0xFF)
        }
        fn port_out_u8(&mut self, _port: u16, val: u8) -> Result<(), ExecError> {
            self.ports.push(val);
            Ok(())
        }
        fn poll_external_irq(&mut self) -> Option<u8> {
            self.latched.take()
        }
    }

    #[test]
    fn xor_reg_clears_and_sets_zf() {
        let mut cpu = CpuState::reset();
        cpu.cs.base = 0;
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RAX, 0x1234);
        // 31 C0  xor ax, ax
        let mut bus = VecBus {
            mem: vec![0x31, 0xC0, 0xF4],
            ports: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0);
    }

    #[test]
    fn out_dx_al_writes_port() {
        let mut cpu = CpuState::reset();
        cpu.cs.base = 0;
        cpu.rip = 0;
        cpu.set_al(b'Z');
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        let mut bus = VecBus {
            mem: vec![0xEE, 0xF4],
            ports: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.ports, b"Z");
    }

    /// `#NMI` vector 2 via IVT; not gated by IF (SDM Vol. 3 §6.3.3 / §6.7 / §6.4).
    #[test]
    fn nmi_delivers_vector_2_ignoring_if() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[2] at offset 8 → handler 0000:0x0800
        mem[8] = 0x00;
        mem[9] = 0x08;
        mem[10] = 0x00;
        mem[11] = 0x00;
        mem[0x800] = 0xF4; // HLT
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0x1000;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(false);
        cpu.request_nmi();
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert!(!cpu.pending_nmi);
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0800);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0x1000); // return IP
    }

    /// INT n: push FLAGS/CS/IP, clear IF+TF, load IVT[vector] (SDM Vol. 2 / Vol. 3 §6.4).
    #[test]
    fn int_imm8_real_mode_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0x21] at 0x84: offset 0x2000, segment 0x1000 → linear 0x12000 (out of this bus).
        // Use segment 0x0000 offset 0x0800 so handler is in the same 64 KiB image.
        mem[0x84] = 0x00;
        mem[0x85] = 0x08; // offset 0x0800
        mem[0x86] = 0x00;
        mem[0x87] = 0x00; // segment 0x0000
                          // Code at CS:IP = 0:0 — INT 21h
        mem[0] = 0xCD;
        mem[1] = 0x21;
        // Handler at 0x800: HLT
        mem[0x800] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.rflags |= 1 << 8; // TF set so we can observe clear
        cpu.rflags |= 1; // CF sticky so FLAGS round-trip is visible
        let saved_flags = cpu.rflags as u16;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0800);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.rflags & (1 << 8), 0);
        // Stack: FLAGS, CS, IP (top)
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 2); // return IP after INT
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0); // CS
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
    }

    /// IRET restores IP/CS/FLAGS from the 16-bit real-mode interrupt frame.
    #[test]
    fn iret_restores_real_mode_frame() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x100] = 0xCF; // IRET
                           // Pre-built frame at SS:SP = 0:0xFFF8 — IP, CS, FLAGS
        mem[0xFFF8] = 0x34;
        mem[0xFFF9] = 0x12; // IP 0x1234
        mem[0xFFFA] = 0x00;
        mem[0xFFFB] = 0x20; // CS 0x2000
        mem[0xFFFC] = 0x03; // FLAGS: CF+reserved1 (IF clear)
        mem[0xFFFD] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0x100;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFF8);
        cpu.set_interrupt_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 0x1234);
        assert_eq!(cpu.cs.selector, 0x2000);
        assert_eq!(cpu.cs.base, 0x2000u64 << 4);
        assert!(!cpu.interrupt_flag());
        assert_ne!(cpu.rflags & 1, 0); // CF restored
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    #[test]
    fn int_then_iret_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0x10] → handler at 0000:0900
        mem[0x40] = 0x00;
        mem[0x41] = 0x09;
        mem[0x42] = 0x00;
        mem[0x43] = 0x00;
        // 0: INT 10h; HLT (return target)
        mem[0] = 0xCD;
        mem[1] = 0x10;
        mem[2] = 0xF4;
        // Handler: IRET
        mem[0x900] = 0xCF;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let flags_before = cpu.rflags;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // INT
        step(&mut cpu, &mut bus).unwrap(); // IRET

        assert_eq!(cpu.ip16(), 2);
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.rflags & 0xFFFF, flags_before & 0xFFFF);
        assert!(cpu.interrupt_flag());
    }

    /// PUSHF pushes 16-bit FLAGS (SDM Vol. 2 PUSHF/PUSHFD/PUSHFQ, real-address mode).
    #[test]
    fn pushf_pushes_flags16() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x9C; // PUSHF
        mem[1] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.rflags |= 1; // CF
        let flags16 = cpu.rflags as u16;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), flags16);
        assert_eq!(cpu.ip16(), 1);
    }

    /// POPF restores 16-bit FLAGS; reserved bit 1 stays set (SDM Vol. 2 POPF).
    #[test]
    fn popf_restores_flags16() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x9D; // POPF
        mem[1] = 0xF4;
        mem[0xFFFC] = 0x03; // CF + reserved1; IF clear
        mem[0xFFFD] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFC);
        cpu.set_interrupt_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert!(!cpu.interrupt_flag());
        assert_ne!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & 2, 2);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
        assert_eq!(cpu.ip16(), 1);
    }

    #[test]
    fn pushf_popf_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x9C; // PUSHF
        mem[1] = 0xFA; // CLI (clear IF in live flags)
        mem[2] = 0x9D; // POPF (restore)
        mem[3] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.rflags |= 1;
        let flags_before = cpu.rflags & 0xFFFF;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // PUSHF
        step(&mut cpu, &mut bus).unwrap(); // CLI
        assert!(!cpu.interrupt_flag());
        step(&mut cpu, &mut bus).unwrap(); // POPF

        assert_eq!(cpu.rflags & 0xFFFF, flags_before);
        assert!(cpu.interrupt_flag());
    }

    /// CALL far: push CS/IP, load ptr16:16 (SDM Vol. 2 CALL).
    #[test]
    fn call_far_pushes_cs_ip_and_loads_target() {
        let mut mem = vec![0u8; 0x10000];
        // CALL 0000:0800 — encoding 9A 00 08 00 00
        mem[0] = 0x9A;
        mem[1] = 0x00;
        mem[2] = 0x08;
        mem[3] = 0x00;
        mem[4] = 0x00;
        mem[0x800] = 0xF4; // HLT at target

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0800);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 5); // return IP
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0); // return CS
    }

    /// RETF restores IP/CS from the far-call frame.
    #[test]
    fn retf_restores_cs_ip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x100] = 0xCB; // RETF
        mem[0xFFFA] = 0x34;
        mem[0xFFFB] = 0x12; // IP
        mem[0xFFFC] = 0x00;
        mem[0xFFFD] = 0x20; // CS 0x2000

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0x100;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFA);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 0x1234);
        assert_eq!(cpu.cs.selector, 0x2000);
        assert_eq!(cpu.cs.base, 0x2000u64 << 4);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    #[test]
    fn call_far_then_retf_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        // 0: CALL 0000:0900; HLT
        mem[0] = 0x9A;
        mem[1] = 0x00;
        mem[2] = 0x09;
        mem[3] = 0x00;
        mem[4] = 0x00;
        mem[5] = 0xF4;
        // Handler: RETF
        mem[0x900] = 0xCB;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // CALL far
        step(&mut cpu, &mut bus).unwrap(); // RETF

        assert_eq!(cpu.ip16(), 5);
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    /// PUSH/POP DS updates selector and real-mode base (SDM Vol. 2 PUSH/POP; Vol. 3 §3.4.2).
    #[test]
    fn push_pop_ds_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x1E; // PUSH DS
        mem[1] = 0x1F; // POP DS
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0x1234);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // PUSH DS
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0x1234);
        cpu.ds = x86_core::SegmentReg::real_mode(0); // clobber
        step(&mut cpu, &mut bus).unwrap(); // POP DS
        assert_eq!(cpu.ds.selector, 0x1234);
        assert_eq!(cpu.ds.base, 0x1234u64 << 4);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    #[test]
    fn push_cs_and_pop_es() {
        // Code lives at F000:0000 (linear 0xF0000); stack still uses SS=0.
        let mut mem = vec![0u8; 0x100000];
        mem[0xF0000] = 0x0E; // PUSH CS
        mem[0xF0001] = 0x07; // POP ES
        mem[0xF0002] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0xF000);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.es.selector, 0xF000);
        assert_eq!(cpu.es.base, 0xF000u64 << 4);
    }

    /// JMP far loads CS:IP from ptr16:16 without touching the stack (SDM Vol. 2 JMP).
    #[test]
    fn jmp_far_loads_cs_ip() {
        let mut mem = vec![0u8; 0x20000];
        // At 0000:0000 — JMP 1000:0200
        mem[0] = 0xEA;
        mem[1] = 0x00;
        mem[2] = 0x02;
        mem[3] = 0x00;
        mem[4] = 0x10;
        // Target linear = 0x1000<<4 + 0x200 = 0x10200
        mem[0x10200] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.cs.base, 0x1000u64 << 4);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE); // stack unchanged
    }

    /// MOV AX, DS / MOV ES, AX — reg forms (SDM Vol. 2 MOV r/m16,Sreg / Sreg,r/m16).
    #[test]
    fn mov_sreg_reg_forms() {
        let mut mem = vec![0u8; 0x10000];
        // 8C D8 = MOV AX, DS; 8E C0 = MOV ES, AX
        mem[0] = 0x8C;
        mem[1] = 0xD8;
        mem[2] = 0x8E;
        mem[3] = 0xC0;
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0x1234);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RAX, 0);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1234);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.es.selector, 0x1234);
        assert_eq!(cpu.es.base, 0x1234u64 << 4);
    }

    /// MOV r/m16, Sreg and MOV Sreg, r/m16 memory forms (SDM Vol. 2 MOV).
    #[test]
    fn mov_sreg_mem_forms() {
        let mut mem = vec![0u8; 0x10000];
        // Use ES as Sreg so DS remains 0 for the EA default segment.
        // 8C 06 00 20 = MOV [0x2000], ES
        // 8E 06 00 20 = MOV ES, [0x2000]
        mem[0] = 0x8C;
        mem[1] = 0x06;
        mem[2] = 0x00;
        mem[3] = 0x20;
        mem[4] = 0x8E;
        mem[5] = 0x06;
        mem[6] = 0x00;
        mem[7] = 0x20;
        mem[8] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0xABCD);
        cpu.rip = 0;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x2000).unwrap(), 0xABCD);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.es.selector, 0xABCD);
        assert_eq!(cpu.es.base, 0xABCDu64 << 4);
    }

    /// MOV CS, r/m16 is invalid (#UD) — delivered via IVT vector 6 (SDM Vol. 2 MOV; Vol. 3 §6.15).
    #[test]
    fn mov_to_cs_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[24] = 0x00;
        mem[25] = 0x0B;
        mem[26] = 0x00;
        mem[27] = 0x00;
        // 8E C8 = MOV CS, AX
        mem[0] = 0x8E;
        mem[1] = 0xC8;
        mem[0xB00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_gpr_u16(CpuState::RAX, 0x1000);
        cpu.set_interrupt_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0B00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
    }

    /// LODSB/STOSB/MOVSB advance SI/DI by DF (SDM Vol. 2 LODS/STOS/MOVS).
    #[test]
    fn string_byte_ops_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xAC; // LODSB
        mem[1] = 0xAA; // STOSB
        mem[2] = 0xA4; // MOVSB
        mem[3] = 0xF4;
        mem[0x1000] = b'X';
        mem[0x1001] = b'Y';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_direction_flag(false);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // LODSB
        assert_eq!(cpu.al(), b'X');
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1001);

        step(&mut cpu, &mut bus).unwrap(); // STOSB
        assert_eq!(bus.read_u8(0x2000).unwrap(), b'X');
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2001);

        // MOVSB: DS:[SI]=Y → ES:[DI]
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2001).unwrap(), b'Y');
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1002);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2002);
    }

    #[test]
    fn lodsb_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xAC;
        mem[1] = 0xF4;
        mem[0x1000] = 0xAB;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_direction_flag(true);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xAB);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x0FFF);
    }

    /// REP/REPE/REPNE on string byte ops (SDM Vol. 2 REP/REPE/REPNE + MOVS/STOS/LODS/SCAS/CMPS).
    #[test]
    fn rep_stosb_cx_zero_is_nop() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[2] = 0xF4;
        mem[0x2000] = 0x55;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0xAA);
        cpu.set_gpr_u16(CpuState::RCX, 0);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2000).unwrap(), 0x55); // unchanged
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2000);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_stosb_fills_and_clears_cx() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Z');
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x3000).unwrap(), b'Z');
        assert_eq!(bus.read_u8(0x3001).unwrap(), b'Z');
        assert_eq!(bus.read_u8(0x3002).unwrap(), b'Z');
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3003);
        assert_eq!(cpu.ip16(), 2);
    }

    /// REP is interruptible between iterations when IF=1.
    /// Spec: Intel SDM Vol. 2 "REP/REPE/REPNE" — service pending interrupts
    /// before each string iteration; saved IP points at the string insn;
    /// CX/SI/DI reflect the last completed iteration.
    #[test]
    fn rep_stosb_external_irq_before_first_iteration() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0x20] → 0000:0E00
        mem[0x20 * 4] = 0x00;
        mem[0x20 * 4 + 1] = 0x0E;
        mem[0x20 * 4 + 2] = 0x00;
        mem[0x20 * 4 + 3] = 0x00;
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[0xE00] = 0xF4; // handler HLT
        mem[0x3000] = 0x11;
        mem[0x3001] = 0x22;
        mem[0x3002] = 0x33;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Z');
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_direction_flag(false);
        cpu.set_interrupt_flag(true);
        cpu.request_interrupt(0x20);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        // Interrupted before any store (SDM poll at iteration start).
        assert_eq!(bus.read_u8(0x3000).unwrap(), 0x11);
        assert_eq!(bus.read_u8(0x3001).unwrap(), 0x22);
        assert_eq!(bus.read_u8(0x3002).unwrap(), 0x33);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 3);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3000);
        assert_eq!(cpu.ip16(), 0x0E00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.pending_irq, None);
        // Saved IP = REP STOSB start.
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
    }

    /// IF=0: pending IRQ stays latched; REP runs to completion.
    #[test]
    fn rep_stosb_pending_irq_ignored_when_if_clear() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x20 * 4] = 0x00;
        mem[0x20 * 4 + 1] = 0x0E;
        mem[0x20 * 4 + 2] = 0x00;
        mem[0x20 * 4 + 3] = 0x00;
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[2] = 0xF4;
        mem[0xE00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Q');
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_direction_flag(false);
        cpu.set_interrupt_flag(false);
        cpu.request_interrupt(0x20);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(bus.read_u8(0x3000).unwrap(), b'Q');
        assert_eq!(bus.read_u8(0x3001).unwrap(), b'Q');
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 2);
        assert_eq!(cpu.pending_irq, Some(0x20));
    }

    /// Bus-latched IRQ after first STOS write → suspend before second iteration.
    /// Spec: SDM Vol. 2 REP — CX/DI reflect last successful iteration; IP = string insn.
    #[test]
    fn rep_stosb_irq_between_iterations_via_bus_poll() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x21 * 4] = 0x00;
        mem[0x21 * 4 + 1] = 0x0F;
        mem[0x21 * 4 + 2] = 0x00;
        mem[0x21 * 4 + 3] = 0x00;
        mem[0] = 0xF3;
        mem[1] = 0xAA; // REP STOSB
        mem[0xF00] = 0xCF; // IRET — resume REP

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Z');
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_direction_flag(false);
        cpu.set_interrupt_flag(true);

        let mut bus = IrqAfterWritesBus {
            mem,
            ports: vec![],
            writes: 0,
            inject_after_writes: 1,
            inject_vector: 0x21,
            latched: None,
        };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(bus.mem[0x3000], b'Z');
        assert_eq!(bus.mem[0x3001], 0); // not yet
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 2);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3001);
        assert_eq!(cpu.ip16(), 0x0F00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // resume at REP STOSB

        // IRET then finish remaining two stores.
        step(&mut cpu, &mut bus).unwrap(); // IRET
        assert!(cpu.interrupt_flag());
        assert_eq!(cpu.ip16(), 0);
        step(&mut cpu, &mut bus).unwrap(); // remaining REP
        assert_eq!(bus.mem[0x3001], b'Z');
        assert_eq!(bus.mem[0x3002], b'Z');
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3003);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_movsb_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xA4; // REP MOVSB
        mem[2] = 0xF4;
        mem[0x1010] = b'A';
        mem[0x100F] = b'B';
        mem[0x100E] = b'C';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x1010);
        cpu.set_gpr_u16(CpuState::RDI, 0x2010);
        cpu.set_direction_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2010).unwrap(), b'A');
        assert_eq!(bus.read_u8(0x200F).unwrap(), b'B');
        assert_eq!(bus.read_u8(0x200E).unwrap(), b'C');
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x100D);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x200D);
    }

    #[test]
    fn rep_lodsb_loads_last_byte_into_al() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAC; // REP LODSB
        mem[2] = 0xF4;
        mem[0x4000] = 0x11;
        mem[0x4001] = 0x22;
        mem[0x4002] = 0x33;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x4000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x33);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x4003);
    }

    #[test]
    fn repe_scasb_stops_on_mismatch() {
        // REPE SCASB: repeat while ZF=1; stop early on first mismatch.
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAE; // REPE SCASB
        mem[2] = 0xF4;
        mem[0x5000] = b'x';
        mem[0x5001] = b'x';
        mem[0x5002] = b'y'; // mismatch
        mem[0x5003] = b'x';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'x');
        cpu.set_gpr_u16(CpuState::RCX, 4);
        cpu.set_gpr_u16(CpuState::RDI, 0x5000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1); // 4→3→2→1 after mismatch at third
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x5003);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0
    }

    #[test]
    fn repne_scasb_stops_on_match() {
        // REPNE SCASB: repeat while ZF=0; stop when equal found.
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF2;
        mem[1] = 0xAE; // REPNE SCASB
        mem[2] = 0xF4;
        mem[0x6000] = b'a';
        mem[0x6001] = b'b';
        mem[0x6002] = b'Q'; // match AL
        mem[0x6003] = b'c';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(b'Q');
        cpu.set_gpr_u16(CpuState::RCX, 4);
        cpu.set_gpr_u16(CpuState::RDI, 0x6000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x6003);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF=1
    }

    #[test]
    fn repe_cmpsb_compares_strings() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xA6; // REPE CMPSB
        mem[2] = 0xF4;
        mem[0x7000] = 1;
        mem[0x7001] = 2;
        mem[0x7002] = 3;
        mem[0x8000] = 1;
        mem[0x8001] = 2;
        mem[0x8002] = 9; // mismatch

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x7000);
        cpu.set_gpr_u16(CpuState::RDI, 0x8000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x7003);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x8003);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0 after mismatch
    }

    /// LODSW/STOSW/MOVSW advance SI/DI by ±2 per DF (SDM Vol. 2 LODS/STOS/MOVS).
    #[test]
    fn string_word_ops_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xAD; // LODSW
        mem[1] = 0xAB; // STOSW
        mem[2] = 0xA5; // MOVSW
        mem[3] = 0xF4;
        // little-endian words at DS:1000
        mem[0x1000] = 0x34;
        mem[0x1001] = 0x12; // 0x1234
        mem[0x1002] = 0x78;
        mem[0x1003] = 0x56; // 0x5678

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_direction_flag(false);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // LODSW
        assert_eq!(cpu.ax(), 0x1234);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1002);

        step(&mut cpu, &mut bus).unwrap(); // STOSW
        assert_eq!(bus.read_u16(0x2000).unwrap(), 0x1234);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2002);

        // MOVSW: DS:[SI]=0x5678 → ES:[DI]
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x2002).unwrap(), 0x5678);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1004);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2004);
    }

    #[test]
    fn lodsw_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xAD;
        mem[1] = 0xF4;
        mem[0x1000] = 0xCD;
        mem[0x1001] = 0xAB; // 0xABCD

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_direction_flag(true);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xABCD);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x0FFE);
    }

    #[test]
    fn rep_stosw_fills_and_clears_cx() {
        // Spec: Intel SDM Vol. 2 STOS + REP/REPE/REPNE
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAB; // REP STOSW
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_ax(0xBEEF);
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x3000).unwrap(), 0xBEEF);
        assert_eq!(bus.read_u16(0x3002).unwrap(), 0xBEEF);
        assert_eq!(bus.read_u16(0x3004).unwrap(), 0xBEEF);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3006);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_movsw_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xA5; // REP MOVSW
        mem[2] = 0xF4;
        // Words at SI=0x1010, 0x100E, 0x100C (DF=1 steps −2).
        mem[0x1010] = 0xAA;
        mem[0x1011] = 0x11; // 0x11AA
        mem[0x100E] = 0xBB;
        mem[0x100F] = 0x22; // 0x22BB
        mem[0x100C] = 0xCC;
        mem[0x100D] = 0x33; // 0x33CC

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x1010);
        cpu.set_gpr_u16(CpuState::RDI, 0x2010);
        cpu.set_direction_flag(true);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x2010).unwrap(), 0x11AA);
        assert_eq!(bus.read_u16(0x200E).unwrap(), 0x22BB);
        assert_eq!(bus.read_u16(0x200C).unwrap(), 0x33CC);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x100A);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x200A);
    }

    #[test]
    fn rep_lodsw_loads_last_word_into_ax() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAD; // REP LODSW
        mem[2] = 0xF4;
        mem[0x4000] = 0x11;
        mem[0x4001] = 0x11;
        mem[0x4002] = 0x22;
        mem[0x4003] = 0x22;
        mem[0x4004] = 0x33;
        mem[0x4005] = 0x33;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x4000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x3333);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x4006);
    }

    #[test]
    fn repe_scasw_stops_on_mismatch() {
        // Spec: Intel SDM Vol. 2 SCAS + REPE
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xAF; // REPE SCASW
        mem[2] = 0xF4;
        mem[0x5000] = 0x78;
        mem[0x5001] = 0x56; // 0x5678 match
        mem[0x5002] = 0x78;
        mem[0x5003] = 0x56; // match
        mem[0x5004] = 0x00;
        mem[0x5005] = 0x00; // mismatch
        mem[0x5006] = 0x78;
        mem[0x5007] = 0x56;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_ax(0x5678);
        cpu.set_gpr_u16(CpuState::RCX, 4);
        cpu.set_gpr_u16(CpuState::RDI, 0x5000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x5006);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0
    }

    #[test]
    fn repne_scasw_stops_on_match() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF2;
        mem[1] = 0xAF; // REPNE SCASW
        mem[2] = 0xF4;
        mem[0x6000] = 0x01;
        mem[0x6001] = 0x00;
        mem[0x6002] = 0x02;
        mem[0x6003] = 0x00;
        mem[0x6004] = 0x51;
        mem[0x6005] = 0x51; // match AX
        mem[0x6006] = 0x03;
        mem[0x6007] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_ax(0x5151);
        cpu.set_gpr_u16(CpuState::RCX, 4);
        cpu.set_gpr_u16(CpuState::RDI, 0x6000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x6006);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF=1
    }

    #[test]
    fn repe_cmpsw_compares_words() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0xA7; // REPE CMPSW
        mem[2] = 0xF4;
        mem[0x7000] = 0x01;
        mem[0x7001] = 0x00;
        mem[0x7002] = 0x02;
        mem[0x7003] = 0x00;
        mem[0x7004] = 0x03;
        mem[0x7005] = 0x00;
        mem[0x8000] = 0x01;
        mem[0x8001] = 0x00;
        mem[0x8002] = 0x02;
        mem[0x8003] = 0x00;
        mem[0x8004] = 0x09;
        mem[0x8005] = 0x00; // mismatch

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x7000);
        cpu.set_gpr_u16(CpuState::RDI, 0x8000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x7006);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x8006);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0 after mismatch
    }

    /// 0x66 A5 = MOVSD — dword element, SI/DI ±4 (SDM Vol. 2 MOVS + opsize).
    #[test]
    fn rep_movsd_opsize32() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x66;
        mem[2] = 0xA5; // REP MOVSD
        mem[3] = 0xF4;
        // two dwords at 0x4000
        mem[0x4000] = 0x01;
        mem[0x4001] = 0x02;
        mem[0x4002] = 0x03;
        mem[0x4003] = 0x04; // 0x04030201
        mem[0x4004] = 0x11;
        mem[0x4005] = 0x22;
        mem[0x4006] = 0x33;
        mem[0x4007] = 0x44; // 0x44332211

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_gpr_u16(CpuState::RSI, 0x4000);
        cpu.set_gpr_u16(CpuState::RDI, 0x5000);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x5000).unwrap(), 0x0403_0201);
        assert_eq!(bus.read_u32(0x5004).unwrap(), 0x4433_2211);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x4008);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x5008);
    }

    #[test]
    fn stosd_opsize32_writes_eax() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x66;
        mem[1] = 0xAB; // STOSD
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_eax(0xDEAD_BEEF);
        cpu.set_gpr_u16(CpuState::RDI, 0x2100);
        cpu.set_direction_flag(false);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x2100).unwrap(), 0xDEAD_BEEF);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2104);
    }

    /// Port bus with sequenced IN bytes and recorded OUT traffic for INS/OUTS tests.
    struct PortSeqBus {
        mem: Vec<u8>,
        in_bytes: Vec<u8>,
        in_idx: usize,
        /// Recorded (port, size, value) outs.
        outs: Vec<(u16, u8, u32)>,
    }

    impl Bus for PortSeqBus {
        fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            Ok(self.mem[i])
        }
        fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            self.mem[i] = val;
            Ok(())
        }
        fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
            if self.in_idx >= self.in_bytes.len() {
                return Ok(0xFF);
            }
            let v = self.in_bytes[self.in_idx];
            self.in_idx += 1;
            Ok(v)
        }
        fn port_out_u8(&mut self, port: u16, val: u8) -> Result<(), ExecError> {
            self.outs.push((port, 1, u32::from(val)));
            Ok(())
        }
        fn port_in_u16(&mut self, port: u16) -> Result<u16, ExecError> {
            let lo = self.port_in_u8(port)?;
            let hi = self.port_in_u8(port.wrapping_add(1))?;
            Ok(u16::from_le_bytes([lo, hi]))
        }
        fn port_out_u16(&mut self, port: u16, val: u16) -> Result<(), ExecError> {
            self.outs.push((port, 2, u32::from(val)));
            Ok(())
        }
        fn port_in_u32(&mut self, port: u16) -> Result<u32, ExecError> {
            let lo = u32::from(self.port_in_u16(port)?);
            let hi = u32::from(self.port_in_u16(port.wrapping_add(2))?);
            Ok(lo | (hi << 16))
        }
        fn port_out_u32(&mut self, port: u16, val: u32) -> Result<(), ExecError> {
            self.outs.push((port, 4, val));
            Ok(())
        }
    }

    /// INSB: DX port → ES:[DI], DI ±1 by DF (SDM Vol. 2 INS/INSB/INSW/INSD).
    #[test]
    fn insb_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x6C; // INSB
        mem[1] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0x41],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2000).unwrap(), 0x41);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2001);
        assert_eq!(cpu.ip16(), 1);
    }

    #[test]
    fn insb_df_backward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x6C;
        mem[1] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x60);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);
        cpu.set_direction_flag(true);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0xAB],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x2000).unwrap(), 0xAB);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x1FFF);
    }

    /// OUTSB: DS:[SI] → DX port, SI ±1 by DF (SDM Vol. 2 OUTS/OUTSB/OUTSW/OUTSD).
    #[test]
    fn outsb_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x6E; // OUTSB
        mem[1] = 0xF4;
        mem[0x1000] = b'Z';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x3F8, 1, u32::from(b'Z'))]);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1001);
        assert_eq!(cpu.ip16(), 1);
    }

    #[test]
    fn outsb_segment_override_es() {
        // Spec: SDM Vol. 2 OUTS — source may use segment override; dest port is DX.
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x26; // ES:
        mem[1] = 0x6E; // OUTSB
        mem[2] = 0xF4;
        mem[0x3000] = 0x55;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        // Put source data under ES base ≠ DS: use es.base via selector.
        cpu.es = x86_core::SegmentReg::real_mode(0x0300); // base 0x3000
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x402);
        cpu.set_gpr_u16(CpuState::RSI, 0); // ES:0 → linear 0x3000
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x402, 1, 0x55)]);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 1);
    }

    #[test]
    fn rep_insb_fills_and_clears_cx() {
        // Spec: SDM Vol. 2 INS + REP/REPE/REPNE (count = CX in asize 16).
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x6C; // REP INSB
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x60);
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RDI, 0x4000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0x11, 0x22, 0x33],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x11);
        assert_eq!(bus.read_u8(0x4001).unwrap(), 0x22);
        assert_eq!(bus.read_u8(0x4002).unwrap(), 0x33);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x4003);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_outsb_cx_zero_is_nop() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x6E; // REP OUTSB
        mem[2] = 0xF4;
        mem[0x1000] = 0x99;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        cpu.set_gpr_u16(CpuState::RCX, 0);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert!(bus.outs.is_empty());
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1000);
        assert_eq!(cpu.ip16(), 2);
    }

    #[test]
    fn rep_outsb_writes_and_clears_cx() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x6E; // REP OUTSB
        mem[2] = 0xF4;
        mem[0x1000] = b'A';
        mem[0x1001] = b'B';
        mem[0x1002] = b'C';

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x3F8);
        cpu.set_gpr_u16(CpuState::RCX, 3);
        cpu.set_gpr_u16(CpuState::RSI, 0x1000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(
            bus.outs,
            [
                (0x3F8, 1, u32::from(b'A')),
                (0x3F8, 1, u32::from(b'B')),
                (0x3F8, 1, u32::from(b'C')),
            ]
        );
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x1003);
    }

    /// INSW/OUTSW: word port I/O, SI/DI ±2 (SDM Vol. 2 INS/OUTS).
    #[test]
    fn insw_outsw_df_forward() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x6D; // INSW
        mem[1] = 0x6F; // OUTSW
        mem[2] = 0xF4;
        // OUTSW source after INSW wrote 0x1234 at ES:2000; point SI there.
        // We'll set SI=0x2000 before OUTSW via separate setup — run step by step.

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x1F0);
        cpu.set_gpr_u16(CpuState::RDI, 0x2000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            // little-endian word 0x1234 via default port_in_u16 (port, port+1)
            in_bytes: vec![0x34, 0x12],
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap(); // INSW
        assert_eq!(bus.read_u16(0x2000).unwrap(), 0x1234);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2002);

        cpu.set_gpr_u16(CpuState::RSI, 0x2000);
        step(&mut cpu, &mut bus).unwrap(); // OUTSW
        assert_eq!(bus.outs, [(0x1F0, 2, 0x1234)]);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x2002);
    }

    #[test]
    fn rep_insw_fills_words() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x6D; // REP INSW
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x1F0);
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_gpr_u16(CpuState::RDI, 0x3000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0xEE, 0xBE, 0xAD, 0xDE], // 0xBEEE, 0xDEAD
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x3000).unwrap(), 0xBEEE);
        assert_eq!(bus.read_u16(0x3002).unwrap(), 0xDEAD);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3004);
    }

    /// 0x66 6D/6F = INSD/OUTSD — dword element, DI/SI ±4 (SDM Vol. 2 INS/OUTS + opsize).
    #[test]
    fn rep_insd_outsd_opsize32() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF3;
        mem[1] = 0x66;
        mem[2] = 0x6D; // REP INSD
        mem[3] = 0x66;
        mem[4] = 0x6F; // OUTSD (single)
        mem[5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RDX, 0x1F0);
        cpu.set_gpr_u16(CpuState::RCX, 1);
        cpu.set_gpr_u16(CpuState::RDI, 0x5000);
        cpu.set_direction_flag(false);

        let mut bus = PortSeqBus {
            mem,
            in_bytes: vec![0x01, 0x02, 0x03, 0x04], // 0x04030201
            in_idx: 0,
            outs: vec![],
        };
        step(&mut cpu, &mut bus).unwrap(); // REP INSD
        assert_eq!(bus.read_u32(0x5000).unwrap(), 0x0403_0201);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x5004);

        cpu.set_gpr_u16(CpuState::RSI, 0x5000);
        step(&mut cpu, &mut bus).unwrap(); // OUTSD
        assert_eq!(bus.outs, [(0x1F0, 4, 0x0403_0201)]);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x5004);
    }

    /// Real-mode fixture over the size-recording port bus used by the
    /// accumulator `IN`/`OUT` tests. `in_bytes` feeds successive `port_in_u8`
    /// calls, so a word read consumes two and a doubleword read four.
    fn port_fixture(code: &[u8], in_bytes: Vec<u8>) -> (CpuState, PortSeqBus) {
        let mut mem = vec![0u8; 0x10000];
        mem[..code.len()].copy_from_slice(code);
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        (
            cpu,
            PortSeqBus {
                mem,
                in_bytes,
                in_idx: 0,
                outs: vec![],
            },
        )
    }

    /// Intel SDM Vol. 2 "IN—Input from Port": `EC` always loads `AL`, while
    /// `ED` loads `AX` or `EAX` from the operand-size attribute. A 16-bit
    /// destination leaves the upper half of `EAX` untouched (Vol. 1 §3.4.1.1),
    /// and no form writes flags.
    #[test]
    fn accumulator_in_destination_width_follows_operand_size() {
        // EC = IN AL, DX — one byte into AL only.
        let (mut cpu, mut bus) = port_fixture(&[0xEC], vec![0x5A]);
        cpu.set_gpr_u16(CpuState::RDX, 0x0CFC);
        cpu.gpr[CpuState::RAX] = 0x1111_2222_3333_4444;
        let flags = cpu.rflags;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr[CpuState::RAX], 0x1111_2222_3333_445A);
        assert_eq!(cpu.rflags, flags, "IN must not write flags");
        assert_eq!(cpu.ip16(), 1);

        // ED = IN AX, DX under a 16-bit operand size.
        let (mut cpu, mut bus) = port_fixture(&[0xED], vec![0x34, 0x12]);
        cpu.set_gpr_u16(CpuState::RDX, 0x0CFC);
        cpu.gpr[CpuState::RAX] = 0x1111_2222_3333_4444;
        let flags = cpu.rflags;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr[CpuState::RAX], 0x1111_2222_3333_1234);
        assert_eq!(cpu.rflags, flags);
        assert_eq!(cpu.ip16(), 1);

        // 66 ED = IN EAX, DX.
        let (mut cpu, mut bus) = port_fixture(&[0x66, 0xED], vec![0x01, 0x02, 0x03, 0x04]);
        cpu.set_gpr_u16(CpuState::RDX, 0x0CFC);
        cpu.gpr[CpuState::RAX] = 0x1111_2222_3333_4444;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr[CpuState::RAX], 0x1111_2222_0403_0201);
        assert_eq!(cpu.ip16(), 2);
    }

    /// Intel SDM Vol. 2 "OUT—Output to Port": `EE` always writes `AL`, while
    /// `EF` writes `AX` or `EAX` in one access of that width through the
    /// size-aware bus accessors, not as a sequence of byte writes.
    #[test]
    fn accumulator_out_source_width_follows_operand_size() {
        // EE = OUT DX, AL.
        let (mut cpu, mut bus) = port_fixture(&[0xEE], vec![]);
        cpu.set_gpr_u16(CpuState::RDX, 0x0CF8);
        cpu.set_gpr_u32(CpuState::RAX, 0x1234_5678);
        let flags = cpu.rflags;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x0CF8, 1, 0x78)]);
        assert_eq!(cpu.rflags, flags, "OUT must not write flags");

        // EF = OUT DX, AX under a 16-bit operand size.
        let (mut cpu, mut bus) = port_fixture(&[0xEF], vec![]);
        cpu.set_gpr_u16(CpuState::RDX, 0x0CF8);
        cpu.set_gpr_u32(CpuState::RAX, 0x1234_5678);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x0CF8, 2, 0x5678)]);
        assert_eq!(cpu.ip16(), 1);

        // 66 EF = OUT DX, EAX.
        let (mut cpu, mut bus) = port_fixture(&[0x66, 0xEF], vec![]);
        cpu.set_gpr_u16(CpuState::RDX, 0x0CF8);
        cpu.set_gpr_u32(CpuState::RAX, 0x1234_5678);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x0CF8, 4, 0x1234_5678)]);
        assert_eq!(cpu.ip16(), 2);
    }

    /// Intel SDM Vol. 2 "IN"/"OUT": the `E5`/`E7` port number is an `imm8` at
    /// every operand size, and only the accumulator width changes.
    #[test]
    fn accumulator_port_imm8_forms_keep_a_byte_port_number() {
        // E5 70 = IN AX, 0x70.
        let (mut cpu, mut bus) = port_fixture(&[0xE5, 0x70], vec![0xCD, 0xAB]);
        cpu.gpr[CpuState::RAX] = 0x0000_0000_FFFF_FFFF;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xFFFF_ABCD);
        assert_eq!(cpu.ip16(), 2);

        // 66 E5 70 = IN EAX, 0x70.
        let (mut cpu, mut bus) = port_fixture(&[0x66, 0xE5, 0x70], vec![0x11, 0x22, 0x33, 0x44]);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x4433_2211);
        assert_eq!(cpu.ip16(), 3);

        // E7 71 = OUT 0x71, AX.
        let (mut cpu, mut bus) = port_fixture(&[0xE7, 0x71], vec![]);
        cpu.set_gpr_u32(CpuState::RAX, 0xDEAD_BEEF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x71, 2, 0xBEEF)]);
        assert_eq!(cpu.ip16(), 2);

        // 66 E7 71 = OUT 0x71, EAX.
        let (mut cpu, mut bus) = port_fixture(&[0x66, 0xE7, 0x71], vec![]);
        cpu.set_gpr_u32(CpuState::RAX, 0xDEAD_BEEF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x71, 4, 0xDEAD_BEEF)]);
        assert_eq!(cpu.ip16(), 3);
    }

    /// Intel SDM Vol. 2 "IN"/"OUT"; Vol. 1 §3.6 Table 3-4: in a `CS.D=1` code
    /// segment `EF` is `OUT DX, EAX` and `66 ED` is `IN AX, DX`. This is the
    /// exact byte sequence SeaBIOS uses to drive PCI configuration Mechanism #1
    /// at `0xCF8`/`0xCFC`, so the two widths appear back to back.
    #[test]
    fn accumulator_port_io_default_sizes_invert_under_cs_d1() {
        // EF = OUT DX, EAX (32-bit default), then 66 ED = IN AX, DX.
        let (mut cpu, bus) = pm32_fixture(&[0xEF, 0x66, 0xED], PM32_CODE, true);
        let mut bus = PortSeqBus {
            mem: bus.mem,
            in_bytes: vec![0x77, 0x66],
            in_idx: 0,
            outs: vec![],
        };
        cpu.set_gpr_u16(CpuState::RDX, 0x0CF8);
        cpu.set_gpr_u32(CpuState::RAX, 0x8000_0000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.outs, [(0x0CF8, 4, 0x8000_0000)]);
        assert_eq!(cpu.rip, (PM32_CODE + 1) as u64);

        cpu.set_gpr_u16(CpuState::RDX, 0x0CFC);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x8000_6677);
        assert_eq!(cpu.rip, (PM32_CODE + 3) as u64);
    }

    /// The accumulator forms and the `INS`/`OUTS` string forms must reach the
    /// bus through the same width-specific accessors, so a word or doubleword
    /// transfer is one access of that width either way.
    /// Spec: Intel SDM Vol. 2 "IN"/"OUT"/"INS"/"OUTS".
    #[test]
    fn accumulator_and_string_port_io_use_the_same_access_widths() {
        for (accumulator, string_op, width, value) in [
            (vec![0xEEu8], vec![0x6Eu8], 1u8, 0x5Au32),
            (vec![0xEF], vec![0x6F], 2, 0xBEEF),
            (vec![0x66, 0xEF], vec![0x66, 0x6F], 4, 0xDEAD_BEEF),
        ] {
            let (mut cpu, mut bus) = port_fixture(&accumulator, vec![]);
            cpu.set_gpr_u16(CpuState::RDX, 0x1F0);
            cpu.set_gpr_u32(CpuState::RAX, value);
            step(&mut cpu, &mut bus).unwrap();
            let accumulator_outs = bus.outs.clone();

            let (mut cpu, mut bus) = port_fixture(&string_op, vec![]);
            cpu.set_gpr_u16(CpuState::RDX, 0x1F0);
            cpu.set_gpr_u16(CpuState::RSI, 0x2000);
            bus.mem[0x2000..0x2004].copy_from_slice(&value.to_le_bytes());
            step(&mut cpu, &mut bus).unwrap();

            assert_eq!(
                accumulator_outs, bus.outs,
                "width {width} accumulator and string OUT must match"
            );
            assert_eq!(accumulator_outs[0].1, width);
        }
    }

    /// Short Jcc take/not-take for unsigned and signed conditions (SDM Vol. 2 Jcc).
    #[test]
    fn jcc_short_conditions() {
        // Layout: JA +2 → target HLT at ip=4; fall-through HLT at ip=2.
        // 77 02 = JA +2; F4; F4
        let run = |opcode: u8, flags: u64, expect_taken: bool| {
            let mut mem = vec![0u8; 0x10000];
            mem[0] = opcode;
            mem[1] = 0x02; // rel8 = +2 → land on second HLT
            mem[2] = 0xF4;
            mem[3] = 0x90;
            mem[4] = 0xF4;

            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.rip = 0;
            cpu.rflags = 0x2 | flags;
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            if expect_taken {
                assert_eq!(cpu.ip16(), 4, "op {opcode:#x} flags {flags:#x} should take");
            } else {
                assert_eq!(
                    cpu.ip16(),
                    2,
                    "op {opcode:#x} flags {flags:#x} should fall through"
                );
            }
        };

        // JA (77): CF=0 and ZF=0
        run(0x77, 0, true);
        run(0x77, 1, false); // CF
        run(0x77, 1 << 6, false); // ZF
                                  // JAE (73): CF=0
        run(0x73, 0, true);
        run(0x73, 1, false);
        // JBE (76): CF|ZF
        run(0x76, 0, false);
        run(0x76, 1, true);
        run(0x76, 1 << 6, true);
        // JL (7C): SF != OF
        run(0x7C, 0, false);
        run(0x7C, 1 << 7, true); // SF
        run(0x7C, (1 << 7) | (1 << 11), false); // SF+OF
                                                // JG (7F): ZF=0 and SF==OF
        run(0x7F, 0, true);
        run(0x7F, 1 << 6, false);
        run(0x7F, 1 << 7, false);
        // JO (70) / JS (78) / JP (7A)
        run(0x70, 1 << 11, true);
        run(0x70, 0, false);
        run(0x78, 1 << 7, true);
        run(0x7A, 1 << 2, true);
        // JGE (7D) / JLE (7E) / JNO (71) / JNS (79) / JNP (7B)
        run(0x7D, 0, true);
        run(0x7E, 1 << 6, true);
        run(0x71, 0, true);
        run(0x79, 0, true);
        run(0x7B, 0, true);
    }

    /// INT3 delivers vector 3 through the IVT like INT 3 (SDM Vol. 2 INT3; Vol. 3 §6.4).
    #[test]
    fn int3_real_mode_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[3] at linear 0x0C: offset 0x0900, segment 0x0000
        mem[0x0C] = 0x00;
        mem[0x0D] = 0x09;
        mem[0x0E] = 0x00;
        mem[0x0F] = 0x00;
        mem[0] = 0xCC; // INT3
        mem[0x900] = 0xF4; // handler HLT

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let saved_flags = cpu.rflags as u16;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0900);
        assert!(!cpu.interrupt_flag());
        // Stack top→: return IP (=1 after CC), CS, FLAGS
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 1);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
    }

    /// INTO: OF=0 falls through; OF=1 delivers #OF (vector 4) as a trap (return IP = next).
    /// Spec: Intel SDM Vol. 2 "INT n/INTO/INT3/INT1"; Vol. 3 §6.15 (#OF — trap).
    #[test]
    fn into_overflow_trap_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[4] → 0000:0A00
        mem[0x10] = 0x00;
        mem[0x11] = 0x0A;
        mem[0x12] = 0x00;
        mem[0x13] = 0x00;
        mem[0] = 0xCE; // INTO
        mem[1] = 0xF4; // fall-through HLT when OF clear
        mem[0xA00] = 0xF4; // #OF handler

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_of(false);
        cpu.set_interrupt_flag(true);
        let flags_clear = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        // OF clear → no vectoring; IP advances past INTO
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 1);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
        assert_eq!(cpu.rflags, flags_clear);
        assert!(cpu.interrupt_flag());

        // OF set → vector 4; saved IP = next (trap), IF cleared
        cpu.rip = 0;
        cpu.set_of(true);
        cpu.set_interrupt_flag(true);
        let saved_flags = cpu.rflags as u16;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0A00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 1); // return IP after INTO
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
    }

    /// BOUND checks signed index against m16&16; #BR (vector 5) is a fault (IP = BOUND).
    /// Spec: Intel SDM Vol. 2 "BOUND"; Vol. 3 §6.15 (#BR — fault).
    #[test]
    fn bound_index_check_and_br_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[5] → 0000:0B00
        mem[0x14] = 0x00;
        mem[0x15] = 0x0B;
        mem[0x16] = 0x00;
        mem[0x17] = 0x00;
        // Bounds at DS:0x2000 — lower=0x0010, upper=0x0020 (signed)
        mem[0x2000] = 0x10;
        mem[0x2001] = 0x00;
        mem[0x2002] = 0x20;
        mem[0x2003] = 0x00;
        // 62 06 00 20 = BOUND AX, [0x2000]
        mem[0] = 0x62;
        mem[1] = 0x06;
        mem[2] = 0x00;
        mem[3] = 0x20;
        mem[4] = 0xF4;
        mem[0xB00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_ax(0x0015); // inside [0x10, 0x20]
        cpu.rflags = 0x246;
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 4);
        assert_eq!(cpu.ax(), 0x0015);
        assert_eq!(cpu.rflags, flags_before);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        // Below lower bound → #BR; fault IP = 0
        cpu.rip = 0;
        cpu.set_ax(0x000F);
        cpu.set_interrupt_flag(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        assert_eq!(cpu.ax(), 0x000F); // index unchanged

        // Above upper bound → #BR
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_ax(0x0021);
        cpu.set_interrupt_flag(true);
        cpu.halted = false;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);

        // Inclusive endpoints succeed
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_ax(0x0010);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 4);
        cpu.rip = 0;
        cpu.set_ax(0x0020);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 4);
    }

    /// BOUND register form is #UD via IVT (SDM Vol. 2 BOUND; Vol. 3 §6.15).
    #[test]
    fn bound_register_source_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0C00
        mem[24] = 0x00;
        mem[25] = 0x0C;
        mem[26] = 0x00;
        mem[27] = 0x00;
        mem[0] = 0x62;
        mem[1] = 0xC0; // BOUND AX, AX — mod=11 → #UD
        mem[0xC00] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0C00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
    }

    /// CLC/STC toggle CF only; CLD/STD toggle DF only (SDM Vol. 2).
    #[test]
    fn clc_stc_cld_std() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xF9; // STC
        mem[1] = 0xF8; // CLC
        mem[2] = 0xFD; // STD
        mem[3] = 0xFC; // CLD
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_cf(false);
        cpu.set_direction_flag(false);
        let other = cpu.rflags;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap(); // STC
        assert_ne!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & !1, other & !1);

        step(&mut cpu, &mut bus).unwrap(); // CLC
        assert_eq!(cpu.rflags & 1, 0);

        step(&mut cpu, &mut bus).unwrap(); // STD
        assert!(cpu.direction_flag());
        assert_eq!(cpu.rflags & 1, 0); // CF untouched

        step(&mut cpu, &mut bus).unwrap(); // CLD
        assert!(!cpu.direction_flag());
    }

    /// MOV AX, CS is valid (read CS selector).
    #[test]
    fn mov_from_cs_to_ax() {
        // Code at 1000:0000 → linear 0x10000
        let mut mem = vec![0u8; 0x20000];
        // 8C C8 = MOV AX, CS
        mem[0x10000] = 0x8C;
        mem[0x10001] = 0xC8;
        mem[0x10002] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x1000);
        cpu.rip = 0;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1000);
    }

    /// Group 2 D0/D1 count=1: ROL/ROR/RCL/RCR/SHL/SHR/SAR (SDM Vol. 2).
    #[test]
    fn grp2_shift_rotate_by1_reg() {
        let run8 = |modrm_reg: u8, al: u8, cf_in: bool| -> (u8, bool, bool) {
            let mut mem = vec![0u8; 0x10000];
            // D0 C0+8*reg = op AL, 1
            mem[0] = 0xD0;
            mem[1] = 0xC0 | (modrm_reg << 3);
            mem[2] = 0xF4;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.rip = 0;
            cpu.set_al(al);
            cpu.set_cf(cf_in);
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            (cpu.al(), cpu.rflags & 1 != 0, cpu.rflags & (1 << 11) != 0)
        };

        // ROL AL,1: 0x81 → 0x03, CF=1, OF=MSB xor CF = 0 xor 1 = 1
        let (r, cf, of) = run8(0, 0x81, false);
        assert_eq!((r, cf, of), (0x03, true, true));

        // ROR AL,1: 0x03 → 0x81, CF=1, OF = two MSBs differ
        let (r, cf, of) = run8(1, 0x03, false);
        assert_eq!((r, cf), (0x81, true));
        assert!(of);

        // RCL AL,1 with CF=1: 0x40 → 0x81, CF=0, OF=1 xor 0 = 1
        let (r, cf, of) = run8(2, 0x40, true);
        assert_eq!((r, cf, of), (0x81, false, true));

        // RCR AL,1 with CF=1: 0x02 → 0x81, CF=0
        let (r, cf, _) = run8(3, 0x02, true);
        assert_eq!((r, cf), (0x81, false));

        // SHL AL,1: 0x40 → 0x80, CF=0, OF=1, SF=1, ZF=0
        let (r, cf, of) = run8(4, 0x40, false);
        assert_eq!((r, cf, of), (0x80, false, true));

        // SHR AL,1: 0x81 → 0x40, CF=1, OF=original MSB=1
        let (r, cf, of) = run8(5, 0x81, false);
        assert_eq!((r, cf, of), (0x40, true, true));

        // SAR AL,1: 0x81 → 0xC0, CF=1, OF=0, SF=1
        let (r, cf, of) = run8(7, 0x81, false);
        assert_eq!((r, cf, of), (0xC0, true, false));

        // Word SHL AX,1
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xD1;
        mem[1] = 0xE0; // SHL AX,1
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x4000);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x8000);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    #[test]
    fn grp2_shl_mem8_and_flags() {
        let mut mem = vec![0u8; 0x10000];
        // D0 26 00 30 = SHL byte [0x3000], 1
        mem[0] = 0xD0;
        mem[1] = 0x26;
        mem[2] = 0x00;
        mem[3] = 0x30;
        mem[4] = 0xF4;
        mem[0x3000] = 0x01;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x3000).unwrap(), 0x02);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF clear
    }

    #[test]
    fn grp2_reserved_slash6_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[24] = 0x00;
        mem[25] = 0x0B;
        mem[26] = 0x00;
        mem[27] = 0x00;
        mem[0] = 0xD0;
        mem[1] = 0xF0; // /6 AL
        mem[0xB00] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
    }

    #[test]
    fn grp2_rol_does_not_touch_zf() {
        // Rotates leave SF/ZF/AF/PF unchanged (SDM Vol. 2 ROL — Flags Affected).
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xD0;
        mem[1] = 0xC0; // ROL AL,1
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x01);
        cpu.set_zf(true);
        cpu.set_sf(true);
        cpu.set_pf(false);
        let zf_sf_pf = cpu.rflags & ((1 << 6) | (1 << 7) | (1 << 2));
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x02);
        assert_eq!(cpu.rflags & ((1 << 6) | (1 << 7) | (1 << 2)), zf_sf_pf);
    }

    /// Group 2 C0/C1 imm8 count (masked to 5 bits). Spec: SDM Vol. 2.
    #[test]
    fn grp2_imm8_shl_shr_count0() {
        let mut mem = vec![0u8; 0x10000];
        // C0 E0 03 = SHL AL, 3; C1 E8 04 = SHR AX, 4; C0 E0 00 = SHL AL, 0 (no-op)
        mem[0] = 0xC0;
        mem[1] = 0xE0;
        mem[2] = 0x03;
        mem[3] = 0xC1;
        mem[4] = 0xE8;
        mem[5] = 0x04;
        mem[6] = 0xC0;
        mem[7] = 0xE0;
        mem[8] = 0x00;
        mem[9] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x01);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // SHL AL,3 → 0x08
        assert_eq!(cpu.al(), 0x08);
        assert_eq!(cpu.rflags & 1, 0);

        cpu.set_ax(0x8000);
        step(&mut cpu, &mut bus).unwrap(); // SHR AX,4 → 0x0800
        assert_eq!(cpu.ax(), 0x0800);

        let flags_before = cpu.rflags;
        cpu.set_al(0x55);
        step(&mut cpu, &mut bus).unwrap(); // SHL AL,0 — unchanged
        assert_eq!(cpu.al(), 0x55);
        assert_eq!(cpu.rflags, flags_before);
    }

    #[test]
    fn grp2_imm8_count_masked_to_5_bits() {
        // COUNT & 0x1F: imm=0x21 → count 1 (SDM Vol. 2).
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0xC0;
        mem[1] = 0xE0;
        mem[2] = 0x21; // SHL AL, 0x21 → effective 1
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x40);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x80);
    }

    /// BCD adjust: DAA/DAS/AAA/AAS/AAM/AAD results + flags (Intel SDM Vol. 2).
    #[test]
    fn bcd_adjust_daa_das_aaa_aas_aam_aad_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: DAA  1: DAS  2: AAA  3: AAS  4-5: AAM 0Ah  6-7: AAD 0Ah  8-9: AAM 10h  10-11: AAD 10h
        mem[0] = 0x27;
        mem[1] = 0x2F;
        mem[2] = 0x37;
        mem[3] = 0x3F;
        mem[4] = 0xD4;
        mem[5] = 0x0A;
        mem[6] = 0xD5;
        mem[7] = 0x0A;
        mem[8] = 0xD4;
        mem[9] = 0x10;
        mem[10] = 0xD5;
        mem[11] = 0x10;
        mem[12] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // DAA: AL=0x0A, AF=0, CF=0 → AL=0x10, AF=1, CF=0; SF/ZF/PF from AL.
        // Spec: Intel SDM Vol. 2 "DAA".
        cpu.set_al(0x0A);
        cpu.set_af(false);
        cpu.set_cf(false);
        cpu.set_of(true); // OF undefined — left unchanged
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x10);
        assert!(cpu.rflags & (1 << 4) != 0); // AF
        assert!(cpu.rflags & 1 == 0); // CF
        assert!(cpu.rflags & (1 << 6) == 0); // ZF
        assert!(cpu.rflags & (1 << 7) == 0); // SF
                                             // PF: 0x10 has one set bit (odd) → PF clear
        assert!(cpu.rflags & (1 << 2) == 0);
        assert!(cpu.rflags & (1 << 11) != 0); // OF preserved

        // DAA: AL=0x9A → low adjust then +60H → AL=0x00, AF=1, CF=1, ZF=1.
        cpu.rip = 0;
        cpu.set_al(0x9A);
        cpu.set_af(false);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x00);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 != 0);
        assert!(cpu.rflags & (1 << 6) != 0);

        // DAA: AL=0x15, AF=1 → +6 → 0x1B; no high adjust.
        cpu.rip = 0;
        cpu.set_al(0x15);
        cpu.set_af(true);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x1B);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 == 0);

        // DAS: AL=0x10, AF=0, CF=0 → no adjust (nibble ok, high ok).
        // Spec: Intel SDM Vol. 2 "DAS".
        cpu.rip = 1;
        cpu.set_al(0x10);
        cpu.set_af(false);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x10);
        assert!(cpu.rflags & (1 << 4) == 0);
        assert!(cpu.rflags & 1 == 0);

        // DAS: AL=0x05, AF=1 → AL−6 = 0xFF, AF=1; high adjust off → CF=0.
        cpu.rip = 1;
        cpu.set_al(0x05);
        cpu.set_af(true);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 == 0);
        assert!(cpu.rflags & (1 << 7) != 0); // SF

        // DAS: AL=0xA0 → high adjust −60H → AL=0x40, CF=1.
        cpu.rip = 1;
        cpu.set_al(0xA0);
        cpu.set_af(false);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x40);
        assert!(cpu.rflags & 1 != 0);

        // AAA: AL=0x0A → AX+=0x106, AL&=0x0F → AX=0x0100, AF=CF=1.
        // Spec: Intel SDM Vol. 2 "AAA". OF/SF/ZF/PF undefined (left unchanged).
        cpu.rip = 2;
        cpu.set_ax(0x000A);
        cpu.set_af(false);
        cpu.set_cf(false);
        cpu.set_zf(true);
        cpu.set_sf(true);
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0100);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 != 0);
        assert!(cpu.rflags & (1 << 6) != 0); // ZF preserved
        assert!(cpu.rflags & (1 << 7) != 0); // SF preserved
        assert!(cpu.rflags & (1 << 11) != 0); // OF preserved

        // AAA: AL=0x05, AF=0 → no adjust; AL&=0x0F stays 5; AF=CF=0.
        cpu.rip = 2;
        cpu.set_ax(0x1205);
        cpu.set_af(false);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1205);
        assert!(cpu.rflags & (1 << 4) == 0);
        assert!(cpu.rflags & 1 == 0);

        // AAS: AL=0x0A → AX−=0x106, AL&=0x0F → AX=0xFF04? Wait: 0x000A - 0x106 = 0xFF04, then AL&=0x0F → 0xFF04.
        // Spec: Intel SDM Vol. 2 "AAS".
        cpu.rip = 3;
        cpu.set_ax(0x000A);
        cpu.set_af(false);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFF04);
        assert!(cpu.rflags & (1 << 4) != 0);
        assert!(cpu.rflags & 1 != 0);

        // AAS: AL=0x03, AF=0 → AL&=0x0F; AF=CF=0.
        cpu.rip = 3;
        cpu.set_ax(0x5503);
        cpu.set_af(false);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x5503);
        assert!(cpu.rflags & (1 << 4) == 0);
        assert!(cpu.rflags & 1 == 0);

        // AAM base 10: AL=0x0F → AH=1, AL=5; SF/ZF/PF from AL.
        // Spec: Intel SDM Vol. 2 "AAM".
        cpu.rip = 4;
        cpu.set_ax(0x000F);
        cpu.set_cf(true); // undefined — left unchanged
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0105);
        assert!(cpu.rflags & (1 << 6) == 0); // ZF
        assert!(cpu.rflags & (1 << 7) == 0); // SF
        assert!(cpu.rflags & (1 << 2) != 0); // PF even(5)=true? 5=101b two bits → even → PF=1
        assert!(cpu.rflags & 1 != 0); // CF preserved
        assert!(cpu.rflags & (1 << 11) != 0); // OF preserved

        // AAD base 10: AH=2, AL=3 → AL=23=0x17, AH=0.
        // Spec: Intel SDM Vol. 2 "AAD".
        cpu.rip = 6;
        cpu.set_ax(0x0203);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0017);
        assert!(cpu.rflags & (1 << 6) == 0);
        assert!(cpu.rflags & (1 << 7) == 0);

        // AAM base 16: AL=0x2A → AH=2, AL=0x0A.
        cpu.rip = 8;
        cpu.set_ax(0x002A);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x020A);

        // AAD base 16: AH=1, AL=5 → AL = 5 + 16 = 0x15.
        cpu.rip = 10;
        cpu.set_ax(0x0105);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0015);
        // PF for 0x15: three set bits → odd → PF clear
        assert!(cpu.rflags & (1 << 2) == 0);
    }

    /// AAM imm8=0 raises #DE via IVT vector 0 (SDM Vol. 2 AAM; Vol. 3 §6.15).
    #[test]
    fn aam_base_zero_de_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0] → 0000:0900
        mem[0] = 0x00;
        mem[1] = 0x09;
        mem[2] = 0x00;
        mem[3] = 0x00;
        mem[0x1000] = 0xD4;
        mem[0x1001] = 0x00; // AAM 0
        mem[0x900] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.set_ax(0x0010);
        let ax_before = cpu.ax();
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0900);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0100);
        assert_eq!(cpu.ax(), ax_before); // no partial update
    }

    /// CBW/CWD sign-extend AL→AX and AX→DX:AX (SDM Vol. 2).
    #[test]
    fn cbw_cwd_sign_extend() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x98; // CBW
        mem[1] = 0x99; // CWD
        mem[2] = 0x98;
        mem[3] = 0x99;
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x0080); // AL negative as i8
        cpu.set_gpr_u16(CpuState::RDX, 0x1234);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // CBW → AX=0xFF80
        assert_eq!(cpu.ax(), 0xFF80);
        step(&mut cpu, &mut bus).unwrap(); // CWD → DX=0xFFFF
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0xFFFF);

        cpu.set_ax(0x007F);
        step(&mut cpu, &mut bus).unwrap(); // CBW → 0x007F
        assert_eq!(cpu.ax(), 0x007F);
        step(&mut cpu, &mut bus).unwrap(); // CWD → DX=0
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0);
    }

    /// LEA loads 16-bit EA offset into reg (SDM Vol. 2 LEA).
    #[test]
    fn lea_disp16_and_bx_si() {
        let mut mem = vec![0u8; 0x10000];
        // 8D 06 34 12 = LEA AX, [0x1234]
        // 8D 18 = LEA BX, [BX+SI]  (mod=00 rm=000)
        mem[0] = 0x8D;
        mem[1] = 0x06;
        mem[2] = 0x34;
        mem[3] = 0x12;
        mem[4] = 0x8D;
        mem[5] = 0x18;
        mem[6] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0x9999); // must not affect LEA
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RBX, 0x0100);
        cpu.set_gpr_u16(CpuState::RSI, 0x0020);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1234);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x0120);
    }

    #[test]
    fn lea_register_source_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[24] = 0x00;
        mem[25] = 0x0B;
        mem[26] = 0x00;
        mem[27] = 0x00;
        mem[0] = 0x8D;
        mem[1] = 0xC0; // LEA AX, AX — mod=11 → #UD
        mem[0xB00] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
    }

    /// Group 3 NOT/NEG (F6/F7 /2 /3). Spec: SDM Vol. 2 NOT/NEG.
    #[test]
    fn grp3_not_neg() {
        let mut mem = vec![0u8; 0x10000];
        // F6 D0 = NOT AL; F6 D8 = NEG AL; F7 D0 = NOT AX; F7 D8 = NEG AX
        mem[0] = 0xF6;
        mem[1] = 0xD0;
        mem[2] = 0xF6;
        mem[3] = 0xD8;
        mem[4] = 0xF7;
        mem[5] = 0xD0;
        mem[6] = 0xF7;
        mem[7] = 0xD8;
        mem[8] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x0F);
        cpu.set_zf(true);
        let flags_before_not = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // NOT AL
        assert_eq!(cpu.al(), 0xF0);
        assert_eq!(cpu.rflags, flags_before_not); // NOT: flags unaffected

        cpu.set_al(0x01);
        step(&mut cpu, &mut bus).unwrap(); // NEG AL → 0xFF, CF=1, SF=1
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 7), 0);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF clear

        cpu.set_ax(0x00FF);
        let flags_before = cpu.rflags;
        step(&mut cpu, &mut bus).unwrap(); // NOT AX
        assert_eq!(cpu.ax(), 0xFF00);
        assert_eq!(cpu.rflags, flags_before);

        cpu.set_ax(0);
        step(&mut cpu, &mut bus).unwrap(); // NEG AX 0 → 0, CF=0, ZF=1
        assert_eq!(cpu.ax(), 0);
        assert_eq!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 6), 0);
    }

    #[test]
    fn grp3_neg_mem8() {
        let mut mem = vec![0u8; 0x10000];
        // F6 1E 00 40 = NEG byte [0x4000]
        mem[0] = 0xF6;
        mem[1] = 0x1E;
        mem[2] = 0x00;
        mem[3] = 0x40;
        mem[0x4000] = 0x10;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0xF0); // −0x10
    }

    /// Group 3 TEST/MUL (F6/F7 /0,/1,/4). Spec: SDM Vol. 2 TEST/MUL.
    #[test]
    fn grp3_test_mul_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: F6 C0 0F       TEST AL, 0x0F
        // 3: F6 C8 01       TEST AL, 1 (/1 alias)
        // 6: F7 C0 34 12    TEST AX, 0x1234
        // A: F6 E3          MUL BL
        // C: F7 E3          MUL BX
        // E: F6 06 00 40 FF TEST byte [0x4000], 0xFF
        // 13: F4            HLT
        mem[0] = 0xF6;
        mem[1] = 0xC0;
        mem[2] = 0x0F;
        mem[3] = 0xF6;
        mem[4] = 0xC8;
        mem[5] = 0x01;
        mem[6] = 0xF7;
        mem[7] = 0xC0;
        mem[8] = 0x34;
        mem[9] = 0x12;
        mem[0xA] = 0xF6;
        mem[0xB] = 0xE3;
        mem[0xC] = 0xF7;
        mem[0xD] = 0xE3;
        mem[0xE] = 0xF6;
        mem[0xF] = 0x06;
        mem[0x10] = 0x00;
        mem[0x11] = 0x40;
        mem[0x12] = 0xFF;
        mem[0x13] = 0xF4;
        mem[0x4000] = 0xF0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0xF0);
        cpu.set_cf(true);
        cpu.set_of(true);
        let mut bus = VecBus { mem, ports: vec![] };

        // TEST AL, 0x0F → 0xF0 & 0x0F = 0; ZF=1, CF=OF=0
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xF0); // unchanged
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        // TEST AL, 1 → 0xF0 & 1 = 0; ZF=1
        step(&mut cpu, &mut bus).unwrap();
        assert_ne!(cpu.rflags & (1 << 6), 0);

        // TEST AX, 0x1234 with AX=0 → 0; ZF=1
        cpu.set_ax(0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0);

        // MUL BL: AL=0x10, BL=0x10 → AX=0x0100; AH!=0 → CF=OF=1
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0100);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // MUL BX: AX=0x0002, BX=0x0003 → DX:AX=0:6; DX=0 → CF=OF=0
        cpu.set_ax(0x0002);
        cpu.set_gpr_u16(CpuState::RBX, 0x0003);
        cpu.set_gpr_u16(CpuState::RDX, 0xFFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 6);
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // TEST byte [0x4000], 0xFF → 0xF0; SF=1, ZF=0
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0xF0); // unchanged
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// Group 3 IMUL/DIV/IDIV (F6/F7 /5–/7). Spec: SDM Vol. 2 IMUL/DIV/IDIV; Vol. 3 §6.15 (#DE).
    #[test]
    fn grp3_imul_div_idiv_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: F6 EB          IMUL BL
        // 2: F7 EB          IMUL BX
        // 4: F6 F3          DIV BL
        // 6: F7 F3          DIV BX
        // 8: F6 FB          IDIV BL
        // A: F7 FB          IDIV BX
        // C: F6 36 00 40    DIV byte [0x4000]
        // 10: F4            HLT
        mem[0] = 0xF6;
        mem[1] = 0xEB;
        mem[2] = 0xF7;
        mem[3] = 0xEB;
        mem[4] = 0xF6;
        mem[5] = 0xF3;
        mem[6] = 0xF7;
        mem[7] = 0xF3;
        mem[8] = 0xF6;
        mem[9] = 0xFB;
        mem[0xA] = 0xF7;
        mem[0xB] = 0xFB;
        mem[0xC] = 0xF6;
        mem[0xD] = 0x36;
        mem[0xE] = 0x00;
        mem[0xF] = 0x40;
        mem[0x10] = 0xF4;
        mem[0x4000] = 5;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // IMUL BL: AL=-2 (0xFE), BL=-3 (0xFD) → AX=6; fits in AL → CF=OF=0
        cpu.set_al(0xFE);
        cpu.set_gpr_u8_low(CpuState::RBX, 0xFD);
        cpu.set_cf(true);
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 6);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX: AX=0x0100, BX=0x0100 → DX:AX=0x0001_0000; does not fit in AX → CF=OF=1
        cpu.set_ax(0x0100);
        cpu.set_gpr_u16(CpuState::RBX, 0x0100);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 1);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // DIV BL: AX=0x0105 / BL=3 → AL=0x57, AH=0
        cpu.set_ax(0x0105);
        cpu.set_gpr_u8_low(CpuState::RBX, 3);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0057);

        // DIV BX: DX:AX=0:1000 / BX=7 → AX=142 (0x8E), DX=6
        cpu.set_ax(1000);
        cpu.set_gpr_u16(CpuState::RDX, 0);
        cpu.set_gpr_u16(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 142);
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 6);

        // IDIV BL: AX=-25 (0xFFE7) / BL=7 → AL=-3 (0xFD), AH=-4 (0xFC)
        cpu.set_ax(0xFFE7);
        cpu.set_gpr_u8_low(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFCFD);

        // IDIV BX: DX:AX=-1000 / BX=7 → AX=-142, DX=-6
        // -1000 as i32 = 0xFFFF_FC18 → DX=0xFFFF, AX=0xFC18
        cpu.set_ax(0xFC18);
        cpu.set_gpr_u16(CpuState::RDX, 0xFFFF);
        cpu.set_gpr_u16(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax() as i16, -142);
        assert_eq!(cpu.gpr_u16(CpuState::RDX) as i16, -6);

        // DIV byte [0x4000]: AX=26 / 5 → AL=5, AH=1
        cpu.set_ax(26);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0105);
    }

    /// DIV/IDIV #DE (vector 0): divisor 0 or quotient overflow; fault IP = insn start.
    #[test]
    fn grp3_div_idiv_de_fault() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[0] → handler at 0000:0900
        mem[0] = 0x00;
        mem[1] = 0x09;
        mem[2] = 0x00;
        mem[3] = 0x00;
        // Place code away from IVT: CS base 0x1000 (selector 0x0100), IP 0
        // linear 0x1000: F6 F3 = DIV BL (divisor 0)
        // linear 0x1002: F6 F3 = DIV BL (quot overflow)
        // linear 0x1004: F7 FB = IDIV BX (i32::MIN / -1)
        mem[0x1000] = 0xF6;
        mem[0x1001] = 0xF3;
        mem[0x1002] = 0xF6;
        mem[0x1003] = 0xF3;
        mem[0x1004] = 0xF7;
        mem[0x1005] = 0xFB;
        mem[0x900] = 0xF4; // handler HLT

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };

        // DIV BL with BL=0 → #DE; saved IP = 0 (faulting insn)
        cpu.set_ax(0x0100);
        cpu.set_gpr_u8_low(CpuState::RBX, 0);
        let ax_before = cpu.ax();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0900);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0100); // CS
        assert_eq!(cpu.ax(), ax_before); // no partial update

        // Resume at overflow DIV: AX=0x0200 / BL=1 → quot 0x200 > 0xFF → #DE
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.rip = 2;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.set_ax(0x0200);
        cpu.set_gpr_u8_low(CpuState::RBX, 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 2);

        // IDIV BX: DX:AX = i32::MIN / -1 → #DE (quot overflow)
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.rip = 4;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.set_ax(0);
        cpu.set_gpr_u16(CpuState::RDX, 0x8000);
        cpu.set_gpr_u16(CpuState::RBX, 0xFFFF); // -1
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 4);
    }

    /// Decode-miss #UD policy (sparse primary table).
    ///
    /// - Architecturally invalid in real-address mode (e.g. ARPL 0x63) → IVT vector 6.
    /// - Valid-but-unimplemented (x87, WAIT, unimplemented 0F map, …) stay host Decode errors.
    /// - D6/F1 are reserved/undefined but do **not** generate #UD (SDM Vol. 3 §6.15).
    ///
    /// Spec: Intel SDM Vol. 3 §6.15 (#UD); Vol. 2 ARPL (real-address mode).
    #[test]
    fn decode_miss_ud_via_ivt_only_for_architectural_ud() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        mem[0x1000] = 0x63; // ARPL — #UD in real-address mode
        mem[0x1001] = 0xC0;
        mem[0xB00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0B00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP = insn start
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0100);

        // Sparse-table misses that are valid-but-unimplemented must NOT become #UD.
        // `0xED` used to stand in for the unimplemented accumulator port I/O
        // forms; it now decodes and executes, so another x87 escape (`0xDF`)
        // covers the same policy.
        for &op in &[0x9Bu8, 0xD8, 0xDF, 0xD6, 0xF1] {
            let mut mem = vec![0u8; 0x10000];
            mem[6 * 4] = 0x00;
            mem[6 * 4 + 1] = 0x0B;
            mem[6 * 4 + 2] = 0x00;
            mem[6 * 4 + 3] = 0x00;
            mem[0] = op;
            mem[1] = 0x90;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            let mut bus = VecBus { mem, ports: vec![] };
            let err = step(&mut cpu, &mut bus).unwrap_err();
            assert!(
                matches!(err, ExecError::Decode(DecodeError::UnsupportedOpcode(o)) if o == op),
                "opcode {op:#x} should remain Decode/UnsupportedOpcode, got {err:?}"
            );
            assert_eq!(cpu.ip16(), 0, "IP must not advance on host decode miss");
            assert_eq!(cpu.cs.selector, 0);
        }

        // 0F is a real escape (IMUL 0F AF is implemented); unimplemented secondaries
        // report UnsupportedOpcode(secondary) and must not vector #UD.
        {
            let mut mem = vec![0u8; 0x10000];
            mem[6 * 4] = 0x00;
            mem[6 * 4 + 1] = 0x0B;
            mem[6 * 4 + 2] = 0x00;
            mem[6 * 4 + 3] = 0x00;
            mem[0] = 0x0F;
            mem[1] = 0x10; // MOVUPS — valid SSE encoding, not in this 0F map
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            let mut bus = VecBus { mem, ports: vec![] };
            let err = step(&mut cpu, &mut bus).unwrap_err();
            assert!(
                matches!(err, ExecError::Decode(DecodeError::UnsupportedOpcode(0x10))),
                "unimplemented 0F map entry should remain Decode/UnsupportedOpcode(secondary), got {err:?}"
            );
            assert_eq!(cpu.ip16(), 0, "IP must not advance on host decode miss");
            assert_eq!(cpu.cs.selector, 0);
        }
    }

    /// Bus that returns MemoryFault once for a poisoned linear address, then allows it.
    /// Needed when the faulting stack write address overlaps the later IVT frame pushes.
    struct PoisonBus {
        mem: Vec<u8>,
        poison: u64,
        tripped: bool,
    }

    impl Bus for PoisonBus {
        fn read_u8(&mut self, addr: u64) -> Result<u8, ExecError> {
            if addr == self.poison && !self.tripped {
                self.tripped = true;
                return Err(ExecError::MemoryFault(addr));
            }
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            Ok(self.mem[i])
        }
        fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), ExecError> {
            if addr == self.poison && !self.tripped {
                self.tripped = true;
                return Err(ExecError::MemoryFault(addr));
            }
            let i = addr as usize;
            if i >= self.mem.len() {
                return Err(ExecError::MemoryFault(addr));
            }
            self.mem[i] = val;
            Ok(())
        }
        fn port_in_u8(&mut self, _port: u16) -> Result<u8, ExecError> {
            Ok(0xFF)
        }
        fn port_out_u8(&mut self, _port: u16, _val: u8) -> Result<(), ExecError> {
            Ok(())
        }
    }

    /// Real-mode MemoryFault → #SS (vector 12) when the access uses SS; #GP (13) otherwise.
    /// Spec: Intel SDM Vol. 3 §6.4, §6.15 (#SS/#GP).
    /// Remaining host MemoryFault: IVT delivery stack/IVT bus errors (unchecked pushes).
    #[test]
    fn memory_fault_ss_gp_via_ivt() {
        // --- #SS: PUSH AX writes SS:SP-2 at poisoned linear address ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            mem[0] = 0x50; // PUSH AX
            mem[0xC00] = 0xF4;
            let poison = 0xFFFC; // first byte of PUSH write at SP=0xFFFE → SP-2=0xFFFC
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.cs.selector, 0);
            assert_eq!(cpu.ip16(), 0x0C00);
            assert!(!cpu.interrupt_flag());
            // After SP restore + 3× push16: SP = 0xFFFE - 6 = 0xFFF8; saved IP at 0xFFF8
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
            assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        }

        // --- #GP: MOV AX,[BX] DS-relative read at poisoned address ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            // 8B 07 = MOV AX, [BX]
            mem[0] = 0x8B;
            mem[1] = 0x07;
            mem[0xD00] = 0xF4;
            let poison = 0x3000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RBX, 0x3000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
            assert!(!cpu.interrupt_flag());
        }

        // --- #SS: MOV AX,[BP] default segment is SS ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            mem[0] = 0x8B;
            mem[1] = 0x46;
            mem[2] = 0x00; // MOV AX, [BP+0]
            mem[0xC00] = 0xF4;
            let poison = 0x4000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RBP, 0x4000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0C00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        }
    }

    /// String / moffs MemoryFault → #GP/#SS via IVT (same classify as ModRM).
    /// Spec: Intel SDM Vol. 3 §6.15 (#SS/#GP); Vol. 2 MOVS/STOS/LODS/MOV moffs.
    #[test]
    fn string_moffs_memory_fault_ss_gp_via_ivt() {
        // --- #GP: STOSB ES:DI write at poisoned address ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            mem[0] = 0xAA; // STOSB
            mem[0xD00] = 0xF4;
            let poison = 0x3000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.es = x86_core::SegmentReg::real_mode(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_al(0x5A);
            cpu.set_gpr_u16(CpuState::RDI, 0x3000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP = STOSB
            assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x3000); // no index update on fault
            assert!(!cpu.interrupt_flag());
        }

        // --- #SS: LODSB with SS override, SI at poison ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            // 36 AC = SS: LODSB
            mem[0] = 0x36;
            mem[1] = 0xAC;
            mem[0xC00] = 0xF4;
            let poison = 0x4000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSI, 0x4000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0C00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
            assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x4000);
        }

        // --- #GP: MOV AL, moffs8 (A0) DS-relative ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            // A0 00 50 = MOV AL, [0x5000]
            mem[0] = 0xA0;
            mem[1] = 0x00;
            mem[2] = 0x50;
            mem[0xD00] = 0xF4;
            let poison = 0x5000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        }

        // --- #SS: MOV AL, moffs8 with SS override ---
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            // 36 A0 00 60 = SS: MOV AL, [0x6000]
            mem[0] = 0x36;
            mem[1] = 0xA0;
            mem[2] = 0x00;
            mem[3] = 0x60;
            mem[0xC00] = 0xF4;
            let poison = 0x6000;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = PoisonBus {
                mem,
                poison,
                tripped: false,
            };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0C00);
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        }
    }

    /// #UD (vector 6) via real-mode IVT for reserved / invalid encodings.
    /// Spec: Intel SDM Vol. 3 §6.15 (#UD); Vol. 2 opcode map (Group 2 /6, Group 5 /7, …).
    /// Faulting IP = instruction start (same frame shape as software INT / #DE).
    #[test]
    fn ud_exception_via_ivt_reserved_encodings() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → handler at 0000:0A00
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0A;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        // CS base 0x1000 (selector 0x0100), IP 0:
        // 0: D0 F0         Group 2 /6 AL (reserved)
        // 2: FF F8         Group 5 /7 AX (reserved)
        // 4: 8D C0         LEA AX, AX (register source)
        // 6: 8E C8         MOV CS, AX
        // 8: C6 C8 00      MOV r/m8,imm /1 (Group 11 reserved)
        // B: FE D0         Group 4 /2 AL (reserved)
        // D: FF D8         Group 5 /3 CALL far reg (#UD)
        // F: 8F C0         POP r/m /0 would be valid; 8F C8 = /1 AX (#UD)
        mem[0x1000] = 0xD0;
        mem[0x1001] = 0xF0;
        mem[0x1002] = 0xFF;
        mem[0x1003] = 0xF8;
        mem[0x1004] = 0x8D;
        mem[0x1005] = 0xC0;
        mem[0x1006] = 0x8E;
        mem[0x1007] = 0xC8;
        mem[0x1008] = 0xC6;
        mem[0x1009] = 0xC8;
        mem[0x100A] = 0x00;
        mem[0x100B] = 0xFE;
        mem[0x100C] = 0xD0;
        mem[0x100D] = 0xFF;
        mem[0x100E] = 0xD8;
        mem[0x100F] = 0x8F;
        mem[0x1010] = 0xC8; // POP /1 AX
        mem[0xA00] = 0xF4; // handler HLT

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };

        let cases: &[(u16, u16)] = &[
            (0, 0),     // Group 2 /6
            (2, 2),     // Group 5 /7
            (4, 4),     // LEA reg
            (6, 6),     // MOV CS
            (8, 8),     // C6 /1
            (0xB, 0xB), // FE /2
            (0xD, 0xD), // FF /3 far CALL reg
            (0xF, 0xF), // 8F /1
        ];
        for &(ip, expect_saved_ip) in cases {
            cpu.cs = x86_core::SegmentReg::real_mode_code(0x0100);
            cpu.rip = u64::from(ip);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            cpu.halted = false;
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.cs.selector, 0, "handler CS at IP {ip:#x}");
            assert_eq!(cpu.ip16(), 0x0A00, "handler IP at fault IP {ip:#x}");
            assert!(!cpu.interrupt_flag());
            assert_eq!(
                bus.read_u16(0xFFF8).unwrap(),
                expect_saved_ip,
                "saved fault IP"
            );
            assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x0100); // CS
        }
    }

    /// Group 2 D2/D3 count = CL (SDM Vol. 2).
    #[test]
    fn grp2_cl_shl_sar() {
        let mut mem = vec![0u8; 0x10000];
        // D2 E0 = SHL AL, CL; D3 F8 = SAR AX, CL
        mem[0] = 0xD2;
        mem[1] = 0xE0;
        mem[2] = 0xD3;
        mem[3] = 0xF8;
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x01);
        cpu.set_gpr_u8_low(CpuState::RCX, 3);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x08);

        cpu.set_ax(0x8000);
        cpu.set_gpr_u8_low(CpuState::RCX, 4);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xF800); // SAR sign-extends
        assert_eq!(cpu.rflags & 1, 0); // last shifted bit was 0
    }

    /// XCHG r/m↔reg and XCHG AX,r16; flags unchanged (SDM Vol. 2 XCHG).
    #[test]
    fn xchg_reg_mem_and_ax_forms() {
        let mut mem = vec![0u8; 0x10000];
        // 86 C3 = XCHG AL, BL
        // 87 06 00 30 = XCHG AX, [0x3000]
        // 91 = XCHG AX, CX
        // 97 = XCHG AX, DI
        mem[0] = 0x86;
        mem[1] = 0xC3;
        mem[2] = 0x87;
        mem[3] = 0x06;
        mem[4] = 0x00;
        mem[5] = 0x30;
        mem[6] = 0x91;
        mem[7] = 0x97;
        mem[8] = 0xF4;
        mem[0x3000] = 0x34;
        mem[0x3001] = 0x12;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0xAA);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x55);
        cpu.rflags = 0x246; // arbitrary non-zero flags; must be preserved
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x55);
        assert_eq!(cpu.gpr_u8_low(CpuState::RBX), 0xAA);
        assert_eq!(cpu.rflags, flags_before);

        cpu.set_ax(0xABCD);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1234);
        assert_eq!(bus.read_u16(0x3000).unwrap(), 0xABCD);
        assert_eq!(cpu.rflags, flags_before);

        cpu.set_ax(0x1111);
        cpu.set_gpr_u16(CpuState::RCX, 0x2222);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x2222);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x1111);

        cpu.set_gpr_u16(CpuState::RDI, 0x3333);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x3333);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2222);
        assert_eq!(cpu.rflags, flags_before);
    }

    #[test]
    fn xchg_reg16_reg16_modrm() {
        // 87 D8 = XCHG AX, BX (mod=11)
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x87;
        mem[1] = 0xD8;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x1000);
        cpu.set_gpr_u16(CpuState::RBX, 0x2000);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x2000);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1000);
    }

    /// PUSH imm16 / sign-extended imm8 (SDM Vol. 2 PUSH).
    #[test]
    fn push_imm16_and_imm8() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x68;
        mem[1] = 0x34;
        mem[2] = 0x12; // PUSH 0x1234
        mem[3] = 0x6A;
        mem[4] = 0xFE; // PUSH -2 → 0xFFFE
        mem[5] = 0x58; // POP AX
        mem[6] = 0x5B; // POP BX
        mem[7] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0x1234);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0xFFFE);

        step(&mut cpu, &mut bus).unwrap(); // POP AX ← 0xFFFE
        assert_eq!(cpu.ax(), 0xFFFE);
        step(&mut cpu, &mut bus).unwrap(); // POP BX ← 0x1234
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1234);
    }

    /// LAHF/SAHF transfer SF ZF AF PF CF via AH (SDM Vol. 2).
    #[test]
    fn lahf_sahf_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x9F; // LAHF
        mem[1] = 0x9E; // SAHF
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        // SF ZF AF PF CF = 1 0 1 0 1 → AH pattern 1x0x0x0x with bit1=1 → 0b1001_0011 = 0x93
        cpu.set_sf(true);
        cpu.set_zf(false);
        cpu.set_af(true);
        cpu.set_pf(false);
        cpu.set_cf(true);
        cpu.set_of(true); // must survive SAHF
        cpu.set_ax(0x0000);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!((cpu.ax() >> 8) as u8, 0x93);

        // Clear status flags then restore via SAHF; OF stays set
        cpu.set_sf(false);
        cpu.set_zf(true);
        cpu.set_af(false);
        cpu.set_pf(true);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert!(cpu.rflags & (1 << 7) != 0); // SF
        assert!(cpu.rflags & (1 << 6) == 0); // ZF
        assert!(cpu.rflags & (1 << 4) != 0); // AF
        assert!(cpu.rflags & (1 << 2) == 0); // PF
        assert!(cpu.rflags & 1 != 0); // CF
        assert!(cpu.rflags & (1 << 11) != 0); // OF preserved
    }

    /// DEC r16: result/flags; CF preserved (SDM Vol. 2 DEC).
    #[test]
    fn dec_r16_preserves_cf() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x48; // DEC AX
        mem[1] = 0x4B; // DEC BX
        mem[2] = 0x4F; // DEC DI
        mem[3] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(1);
        cpu.set_gpr_u16(CpuState::RBX, 0);
        cpu.set_gpr_u16(CpuState::RDI, 0x8000);
        cpu.set_cf(true);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_ne!(cpu.rflags & 1, 0); // CF preserved

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xFFFF);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_ne!(cpu.rflags & 1, 0); // CF still set

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x7FFF);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF: 0x8000-1
        assert_ne!(cpu.rflags & 1, 0);
    }

    /// Group 1 80/81/83 imm ALU — results and flags (SDM Vol. 2).
    #[test]
    fn grp1_imm_alu() {
        let mut mem = vec![0u8; 0x10000];
        // 80 C0 01 = ADD AL,1
        // 80 E0 0F = AND AL,0x0F
        // 80 F8 05 = CMP AL,5
        // 81 C3 00 10 = ADD BX,0x1000
        // 83 EB 01 = SUB BX,1 (imm8 sign-ext)
        // 83 D8 FF = SBB AX,-1 with CF
        mem[0] = 0x80;
        mem[1] = 0xC0;
        mem[2] = 0x01;
        mem[3] = 0x80;
        mem[4] = 0xE0;
        mem[5] = 0x0F;
        mem[6] = 0x80;
        mem[7] = 0xF8;
        mem[8] = 0x05;
        mem[9] = 0x81;
        mem[10] = 0xC3;
        mem[11] = 0x00;
        mem[12] = 0x10;
        mem[13] = 0x83;
        mem[14] = 0xEB;
        mem[15] = 0x01;
        mem[16] = 0x83;
        mem[17] = 0xD8;
        mem[18] = 0xFF;
        mem[19] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0x10);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // ADD AL,1
        assert_eq!(cpu.al(), 0x11);
        assert_eq!(cpu.rflags & 1, 0);

        step(&mut cpu, &mut bus).unwrap(); // AND AL,0x0F
        assert_eq!(cpu.al(), 0x01);
        assert_eq!(cpu.rflags & 1, 0); // logic clears CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0

        let al_before = cpu.al();
        step(&mut cpu, &mut bus).unwrap(); // CMP AL,5 → 1-5
        assert_eq!(cpu.al(), al_before); // CMP no write
        assert_ne!(cpu.rflags & 1, 0); // CF=1 (borrow)
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0

        cpu.set_gpr_u16(CpuState::RBX, 0x0200);
        step(&mut cpu, &mut bus).unwrap(); // ADD BX,0x1000
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1200);

        step(&mut cpu, &mut bus).unwrap(); // SUB BX,1
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x11FF);

        cpu.set_ax(0x0001);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap(); // SBB AX, -1 (=0xFFFF): 1 - (-1) - 1 = 1
        assert_eq!(cpu.ax(), 0x0001);
    }

    #[test]
    fn grp1_adc_or_xor_mem() {
        // 80 06 00 40 7F = ADD byte [0x4000], 0x7F
        // 80 0E 00 40 01 = OR  byte [0x4000], 1
        // 80 36 00 40 FF = XOR byte [0x4000], 0xFF
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x80;
        mem[1] = 0x06;
        mem[2] = 0x00;
        mem[3] = 0x40;
        mem[4] = 0x7F;
        mem[5] = 0x80;
        mem[6] = 0x0E;
        mem[7] = 0x00;
        mem[8] = 0x40;
        mem[9] = 0x01;
        mem[10] = 0x80;
        mem[11] = 0x36;
        mem[12] = 0x00;
        mem[13] = 0x40;
        mem[14] = 0xFF;
        mem[0x4000] = 0x01;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x80);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF: 0x01+0x7F → 0x80

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x81);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x7E);
    }

    /// LOOP/LOOPcc decrement CX then branch; JCXZ tests CX (SDM Vol. 2).
    #[test]
    fn loop_loopcc_jcxz() {
        let mut mem = vec![0u8; 0x10000];
        // E2 FE = LOOP $-0 (rel8=-2) → branch back to self while CX≠0 after dec
        // After CX hits 0, fall through.
        mem[0] = 0xE2;
        mem[1] = 0xFE;
        // E0 02 = LOOPNE +2; E1 02 = LOOPE +2; padding HLTs
        mem[2] = 0xE0;
        mem[3] = 0x02;
        mem[4] = 0xF4; // skip target when not taken
        mem[5] = 0xF4;
        mem[6] = 0x90; // taken landing
        mem[7] = 0xE1;
        mem[8] = 0x02;
        mem[9] = 0xF4;
        mem[10] = 0xF4;
        mem[11] = 0x90;
        // E3 02 = JCXZ +2
        mem[12] = 0xE3;
        mem[13] = 0x02;
        mem[14] = 0xF4;
        mem[15] = 0xF4;
        mem[16] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RCX, 3);
        let mut bus = VecBus { mem, ports: vec![] };

        // LOOP three times: CX 3→2→1→0, then fall through to IP=2
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 2);
        assert_eq!(cpu.ip16(), 0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 2);

        // LOOPNE: CX=2, ZF=0 → take; then CX=1, ZF=1 → no take
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_zf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.ip16(), 6); // taken → next_ip(4)+2

        cpu.rip = 2;
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_zf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 1);
        assert_eq!(cpu.ip16(), 4); // not taken

        // LOOPE: ZF=1 and CX after dec ≠0 → take
        cpu.rip = 7;
        cpu.set_gpr_u16(CpuState::RCX, 1);
        cpu.set_zf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 9); // CX became 0 → not taken

        cpu.rip = 7;
        cpu.set_gpr_u16(CpuState::RCX, 2);
        cpu.set_zf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 11); // taken

        // JCXZ: CX==0 takes; CX!=0 falls through
        cpu.rip = 12;
        cpu.set_gpr_u16(CpuState::RCX, 0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 16);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0); // unchanged

        cpu.rip = 12;
        cpu.set_gpr_u16(CpuState::RCX, 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 14);
    }

    /// OR/AND ModRM 08–0B / 20–23 — results and logic flags (SDM Vol. 2 OR/AND).
    #[test]
    fn and_or_modrm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 08 D8 = OR  AL, BL
        // 0A C3 = OR  AL, BL  (reg ← r/m; same regs after first)
        // 09 D8 = OR  AX, BX
        // 0B C3 = OR  AX, BX
        // 20 D8 = AND AL, BL
        // 22 C3 = AND AL, BL
        // 21 D8 = AND AX, BX
        // 23 C3 = AND AX, BX
        // 09 06 00 40 = OR  word [0x4000], AX
        // 23 06 00 40 = AND AX, word [0x4000]
        mem[0] = 0x08;
        mem[1] = 0xD8;
        mem[2] = 0x0A;
        mem[3] = 0xC3;
        mem[4] = 0x09;
        mem[5] = 0xD8;
        mem[6] = 0x0B;
        mem[7] = 0xC3;
        mem[8] = 0x20;
        mem[9] = 0xD8;
        mem[10] = 0x22;
        mem[11] = 0xC3;
        mem[12] = 0x21;
        mem[13] = 0xD8;
        mem[14] = 0x23;
        mem[15] = 0xC3;
        mem[16] = 0x09;
        mem[17] = 0x06;
        mem[18] = 0x00;
        mem[19] = 0x40;
        mem[20] = 0x23;
        mem[21] = 0x06;
        mem[22] = 0x00;
        mem[23] = 0x40;
        mem[24] = 0xF4;
        mem[0x4000] = 0x0F;
        mem[0x4001] = 0xF0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0xF0);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x0F);
        cpu.set_cf(true);
        cpu.set_of(true);
        let mut bus = VecBus { mem, ports: vec![] };

        // OR AL, BL (08): r/m ← r/m | reg → AL |= BL
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_eq!(cpu.rflags & 1, 0); // CF cleared
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF cleared
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // OR AL, BL (0A): reg ← reg | r/m
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x01);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x11);

        // OR AX, BX (09): r/m ← r/m | reg
        cpu.set_ax(0xF000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0FFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFFFF);

        // OR AX, BX (0B): reg ← reg | r/m
        cpu.set_ax(0x1000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0200);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1200);

        // AND AL, BL (20)
        cpu.set_al(0xF3);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x0F);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x03);
        assert_eq!(cpu.rflags & 1, 0);

        // AND AL, BL (22)
        cpu.set_al(0xAA);
        cpu.set_gpr_u8_low(CpuState::RBX, 0xF0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xA0);

        // AND AX, BX (21)
        cpu.set_ax(0xFF00);
        cpu.set_gpr_u16(CpuState::RBX, 0x0FF0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0F00);

        // AND AX, BX (23)
        cpu.set_ax(0x1234);
        cpu.set_gpr_u16(CpuState::RBX, 0x00FF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0034);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0

        // OR [0x4000], AX — mem destination
        cpu.set_ax(0x00F0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0xF0FF);

        // AND AX, [0x4000]
        cpu.set_ax(0xFFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xF0FF);
    }

    /// ADC/SBB ModRM 10–13 / 18–1B — results and flags with CF in (SDM Vol. 2 ADC/SBB).
    #[test]
    fn adc_sbb_modrm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 10 D8 = ADC AL, BL
        // 12 C3 = ADC AL, BL  (reg ← r/m)
        // 11 D8 = ADC AX, BX
        // 13 C3 = ADC AX, BX
        // 18 D8 = SBB AL, BL
        // 1A C3 = SBB AL, BL
        // 19 D8 = SBB AX, BX
        // 1B C3 = SBB AX, BX
        // 11 06 00 40 = ADC word [0x4000], AX
        // 1B 06 00 40 = SBB AX, word [0x4000]
        mem[0] = 0x10;
        mem[1] = 0xD8;
        mem[2] = 0x12;
        mem[3] = 0xC3;
        mem[4] = 0x11;
        mem[5] = 0xD8;
        mem[6] = 0x13;
        mem[7] = 0xC3;
        mem[8] = 0x18;
        mem[9] = 0xD8;
        mem[10] = 0x1A;
        mem[11] = 0xC3;
        mem[12] = 0x19;
        mem[13] = 0xD8;
        mem[14] = 0x1B;
        mem[15] = 0xC3;
        mem[16] = 0x11;
        mem[17] = 0x06;
        mem[18] = 0x00;
        mem[19] = 0x40;
        mem[20] = 0x1B;
        mem[21] = 0x06;
        mem[22] = 0x00;
        mem[23] = 0x40;
        mem[24] = 0xF4;
        mem[0x4000] = 0x00;
        mem[0x4001] = 0x80; // 0x8000

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // ADC AL, BL (10): 0x10 + 0x20 + CF1 = 0x31
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x20);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x31);
        assert_eq!(cpu.rflags & 1, 0); // CF clear

        // ADC AL, BL (12): reg ← reg + r/m + CF; 0x7F + 0 + CF1 → 0x80, OF set
        cpu.set_al(0x7F);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x00);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x80);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // ADC AX, BX (11): 0x1000 + 0x0200 + CF0 = 0x1200
        cpu.set_ax(0x1000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0200);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1200);

        // ADC AX, BX (13): 0xFFFF + 1 + CF0 → 0, CF set, ZF set
        cpu.set_ax(0xFFFF);
        cpu.set_gpr_u16(CpuState::RBX, 0x0001);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0000);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        // SBB AL, BL (18): 0x05 - 0x02 - CF1 = 0x02
        cpu.set_al(0x05);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x02);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x02);
        assert_eq!(cpu.rflags & 1, 0);

        // SBB AL, BL (1A): 0x00 - 0x00 - CF1 = 0xFF, CF set
        cpu.set_al(0x00);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x00);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // SBB AX, BX (19): 0x1000 - 0x0001 - CF0 = 0x0FFF
        cpu.set_ax(0x1000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0001);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0FFF);

        // SBB AX, BX (1B): 0x0000 - 0x0001 - CF0 = 0xFFFF, CF set
        cpu.set_ax(0x0000);
        cpu.set_gpr_u16(CpuState::RBX, 0x0001);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFFFF);
        assert_ne!(cpu.rflags & 1, 0);

        // ADC [0x4000], AX — mem dest: 0x8000 + 0x0001 + CF1 = 0x8002
        cpu.set_ax(0x0001);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x8002);

        // SBB AX, [0x4000]: 0x8003 - 0x8002 - CF0 = 0x0001
        cpu.set_ax(0x8003);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0001);
        assert_eq!(cpu.rflags & 1, 0);
    }

    /// ADC/SBB AL/AX,imm — 14/15/1C/1D (SDM Vol. 2 ADC/SBB accumulator forms).
    #[test]
    fn adc_sbb_al_ax_imm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 14 01       ADC AL, 0x01
        // 15 00 10    ADC AX, 0x1000
        // 1C 02       SBB AL, 0x02
        // 1D 01 00    SBB AX, 0x0001
        // 14 FF       ADC AL, 0xFF  (CF+wrap)
        // 1C 00       SBB AL, 0     (with CF)
        mem[0] = 0x14;
        mem[1] = 0x01;
        mem[2] = 0x15;
        mem[3] = 0x00;
        mem[4] = 0x10;
        mem[5] = 0x1C;
        mem[6] = 0x02;
        mem[7] = 0x1D;
        mem[8] = 0x01;
        mem[9] = 0x00;
        mem[10] = 0x14;
        mem[11] = 0xFF;
        mem[12] = 0x1C;
        mem[13] = 0x00;
        mem[14] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // ADC AL, 1 with CF=1: 0x10 + 0x01 + 1 = 0x12; AH preserved
        cpu.set_ax(0xAB10);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x12);
        assert_eq!(cpu.ax(), 0xAB12);
        assert_eq!(cpu.rflags & 1, 0);

        // ADC AX, 0x1000 with CF=0: 0x0200 + 0x1000 = 0x1200
        cpu.set_ax(0x0200);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1200);

        // SBB AL, 2 with CF=1: 0x05 - 0x02 - 1 = 0x02
        cpu.set_ax(0xCD05);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x02);
        assert_eq!(cpu.ax(), 0xCD02);
        assert_eq!(cpu.rflags & 1, 0);

        // SBB AX, 1 with CF=0: 0x1000 - 1 = 0x0FFF
        cpu.set_ax(0x1000);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0FFF);

        // ADC AL, 0xFF with CF=0: 0x01 + 0xFF = 0x00, CF set, ZF set
        cpu.set_al(0x01);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x00);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        // SBB AL, 0 with CF=1: 0x00 - 0 - 1 = 0xFF, CF set, SF set
        cpu.set_al(0x00);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    /// OR/AND AL/AX,imm — 0C/0D/24/25 (SDM Vol. 2 OR/AND accumulator forms).
    #[test]
    fn and_or_al_ax_imm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0C 0F       OR  AL, 0x0F
        // 0D F0 0F    OR  AX, 0x0FF0
        // 24 F0       AND AL, 0xF0
        // 25 FF 00    AND AX, 0x00FF
        // 0C 00       OR  AL, 0     (ZF)
        mem[0] = 0x0C;
        mem[1] = 0x0F;
        mem[2] = 0x0D;
        mem[3] = 0xF0;
        mem[4] = 0x0F;
        mem[5] = 0x24;
        mem[6] = 0xF0;
        mem[7] = 0x25;
        mem[8] = 0xFF;
        mem[9] = 0x00;
        mem[10] = 0x0C;
        mem[11] = 0x00;
        mem[12] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x12F0); // AH=0x12 must survive AL ops
        cpu.set_cf(true);
        cpu.set_of(true);
        let mut bus = VecBus { mem, ports: vec![] };

        // OR AL, 0x0F → AL = 0xFF; CF/OF cleared; SF set
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_eq!(cpu.ax(), 0x12FF);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // OR AX, 0x0FF0
        cpu.set_ax(0xF000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xFFF0);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // AND AL, 0xF0
        cpu.set_ax(0x34AB);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xA0);
        assert_eq!(cpu.ax(), 0x34A0);
        assert_eq!(cpu.rflags & 1, 0);

        // AND AX, 0x00FF
        cpu.set_ax(0x1234);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0034);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0

        // OR AL, 0 → ZF
        cpu.set_al(0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// XOR ModRM byte 30/32 — results and logic flags (SDM Vol. 2 XOR).
    #[test]
    fn xor_modrm_byte_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 30 D8 = XOR AL, BL  (r/m ← r/m ^ reg)
        // 32 C3 = XOR AL, BL  (reg ← reg ^ r/m)
        // 30 06 00 40 = XOR byte [0x4000], AL
        // 32 06 00 40 = XOR AL, byte [0x4000]
        mem[0] = 0x30;
        mem[1] = 0xD8;
        mem[2] = 0x32;
        mem[3] = 0xC3;
        mem[4] = 0x30;
        mem[5] = 0x06;
        mem[6] = 0x00;
        mem[7] = 0x40;
        mem[8] = 0x32;
        mem[9] = 0x06;
        mem[10] = 0x00;
        mem[11] = 0x40;
        mem[12] = 0xF4;
        mem[0x4000] = 0xF0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // XOR AL, BL (30): 0xF0 ^ 0x0F = 0xFF; CF/OF cleared; SF set
        cpu.set_al(0xF0);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x0F);
        cpu.set_cf(true);
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_eq!(cpu.rflags & 1, 0); // CF cleared
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF cleared
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // XOR AL, BL (32): 0xAA ^ 0x55 = 0xFF
        cpu.set_al(0xAA);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x55);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);

        // XOR [0x4000], AL (30): 0xF0 ^ 0x0F = 0xFF
        cpu.set_al(0x0F);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0xFF);

        // XOR AL, [0x4000] (32): 0x11 ^ 0xFF = 0xEE; ZF clear
        cpu.set_al(0x11);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xEE);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    /// ADD/SUB ModRM byte 00/02/28/2A — results and arithmetic flags (SDM Vol. 2 ADD/SUB).
    #[test]
    fn add_sub_modrm_byte_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 00 D8 = ADD AL, BL  (r/m ← r/m + reg)
        // 02 C3 = ADD AL, BL  (reg ← reg + r/m)
        // 28 D8 = SUB AL, BL
        // 2A C3 = SUB AL, BL
        // 00 06 00 40 = ADD byte [0x4000], AL
        // 2A 06 00 40 = SUB AL, byte [0x4000]
        mem[0] = 0x00;
        mem[1] = 0xD8;
        mem[2] = 0x02;
        mem[3] = 0xC3;
        mem[4] = 0x28;
        mem[5] = 0xD8;
        mem[6] = 0x2A;
        mem[7] = 0xC3;
        mem[8] = 0x00;
        mem[9] = 0x06;
        mem[10] = 0x00;
        mem[11] = 0x40;
        mem[12] = 0x2A;
        mem[13] = 0x06;
        mem[14] = 0x00;
        mem[15] = 0x40;
        mem[16] = 0xF4;
        mem[0x4000] = 0x10;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // ADD AL, BL (00): 0x70 + 0x10 = 0x80; CF=0; SF set; OF set (signed overflow)
        cpu.set_al(0x70);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x80);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF

        // ADD AL, BL (02): 0x01 + 0x02 = 0x03; ZF clear
        cpu.set_al(0x01);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x02);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x03);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // SUB AL, BL (28): 0x05 - 0x10 = 0xF5; CF set; SF set
        cpu.set_al(0x05);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xF5);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // SUB AL, BL (2A): 0x10 - 0x10 = 0; ZF set; CF clear
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & 1, 0); // CF

        // ADD [0x4000], AL (00): 0x10 + 0x05 = 0x15
        cpu.set_al(0x05);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x15);

        // SUB AL, [0x4000] (2A): 0x20 - 0x15 = 0x0B
        cpu.set_al(0x20);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x0B);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// CMP ModRM byte 38/3A — flags only, operands unchanged (SDM Vol. 2 CMP).
    #[test]
    fn cmp_modrm_byte_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 38 D8 = CMP AL, BL  (r/m − reg → flags)
        // 3A C3 = CMP AL, BL  (reg − r/m → flags)
        // 38 06 00 40 = CMP byte [0x4000], AL
        // 3A 06 00 40 = CMP AL, byte [0x4000]
        mem[0] = 0x38;
        mem[1] = 0xD8;
        mem[2] = 0x3A;
        mem[3] = 0xC3;
        mem[4] = 0x38;
        mem[5] = 0x06;
        mem[6] = 0x00;
        mem[7] = 0x40;
        mem[8] = 0x3A;
        mem[9] = 0x06;
        mem[10] = 0x00;
        mem[11] = 0x40;
        mem[12] = 0xF4;
        mem[0x4000] = 0x10;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // CMP AL, BL (38): 0x05 − 0x10 → CF/SF set; AL unchanged
        cpu.set_al(0x05);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x05);
        assert_eq!(cpu.gpr_u8_low(CpuState::RBX), 0x10);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // CMP AL, BL (3A): 0x10 − 0x10 → ZF; CF clear; AL unchanged
        cpu.set_al(0x10);
        cpu.set_gpr_u8_low(CpuState::RBX, 0x10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x10);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & 1, 0); // CF

        // CMP [0x4000], AL (38): 0x10 − 0x05 → CF clear; mem unchanged
        cpu.set_al(0x05);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x10);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // CMP AL, [0x4000] (3A): 0x05 − 0x10 → CF/SF; AL unchanged
        cpu.set_al(0x05);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x05);
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x10);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    /// SUB/XOR/CMP AL/AX,imm — 2C/2D/34/35/3C/3D (SDM Vol. 2 accumulator forms).
    #[test]
    fn sub_xor_cmp_al_ax_imm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 2C 01       SUB AL, 0x01
        // 2D 00 10    SUB AX, 0x1000
        // 34 0F       XOR AL, 0x0F
        // 35 FF 00    XOR AX, 0x00FF
        // 3C 05       CMP AL, 0x05
        // 3D 34 12    CMP AX, 0x1234
        // 2C 01       SUB AL, 1  (borrow → CF)
        mem[0] = 0x2C;
        mem[1] = 0x01;
        mem[2] = 0x2D;
        mem[3] = 0x00;
        mem[4] = 0x10;
        mem[5] = 0x34;
        mem[6] = 0x0F;
        mem[7] = 0x35;
        mem[8] = 0xFF;
        mem[9] = 0x00;
        mem[10] = 0x3C;
        mem[11] = 0x05;
        mem[12] = 0x3D;
        mem[13] = 0x34;
        mem[14] = 0x12;
        mem[15] = 0x2C;
        mem[16] = 0x01;
        mem[17] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // SUB AL, 1: 0x10 - 1 = 0x0F; AH preserved
        cpu.set_ax(0xAB10);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x0F);
        assert_eq!(cpu.ax(), 0xAB0F);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // SUB AX, 0x1000: 0x2000 - 0x1000 = 0x1000
        cpu.set_ax(0x2000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1000);
        assert_eq!(cpu.rflags & 1, 0);

        // XOR AL, 0x0F: 0xF0 ^ 0x0F = 0xFF; CF/OF cleared; SF set
        cpu.set_ax(0x12F0);
        cpu.set_cf(true);
        cpu.set_of(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_eq!(cpu.ax(), 0x12FF);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // XOR AX, 0x00FF: 0x1234 ^ 0x00FF = 0x12CB
        cpu.set_ax(0x1234);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x12CB);

        // CMP AL, 5: 5 - 5 → ZF; AL unchanged
        cpu.set_ax(0xCD05);
        let al_before = cpu.al();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), al_before);
        assert_eq!(cpu.ax(), 0xCD05);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & 1, 0); // CF

        // CMP AX, 0x1234: 0x1000 - 0x1234 → CF set; AX unchanged
        cpu.set_ax(0x1000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1000);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF

        // SUB AL, 1: 0x00 - 1 = 0xFF, CF set, SF set
        cpu.set_al(0x00);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
    }

    /// ADD AX,imm16 — 05 iw (SDM Vol. 2 ADD accumulator form).
    #[test]
    fn add_ax_imm_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 05 34 12    ADD AX, 0x1234
        // 05 01 00    ADD AX, 0x0001  (carry from 0xFFFF)
        // 05 00 80    ADD AX, 0x8000  (signed overflow)
        mem[0] = 0x05;
        mem[1] = 0x34;
        mem[2] = 0x12;
        mem[3] = 0x05;
        mem[4] = 0x01;
        mem[5] = 0x00;
        mem[6] = 0x05;
        mem[7] = 0x00;
        mem[8] = 0x80;
        mem[9] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // ADD AX, 0x1234: 0x1000 + 0x1234 = 0x2234; CF/OF/ZF clear
        cpu.set_ax(0x1000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x2234);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF

        // ADD AX, 1: 0xFFFF + 1 = 0; CF and ZF set
        cpu.set_ax(0xFFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0000);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        // ADD AX, 0x8000: 0x8000 + 0x8000 = 0; CF and OF set
        cpu.set_ax(0x8000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0000);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// FE/FF Group 4/5 INC/DEC r/m — /0 INC, /1 DEC; CF preserved (SDM Vol. 2 INC/DEC).
    #[test]
    fn grp4_grp5_inc_dec_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // FE C0          INC AL
        // FE C3          INC BL
        // FE C8          DEC AL
        // FE 06 00 40    INC byte [0x4000]
        // FE 0E 00 40    DEC byte [0x4000]
        // FF C0          INC AX
        // FF C8          DEC AX
        // FF 06 00 40    INC word [0x4000]
        // FF 0E 00 40    DEC word [0x4000]
        // FE /2 #UD covered by ud_exception_via_ivt_reserved_encodings
        mem[0] = 0xFE;
        mem[1] = 0xC0;
        mem[2] = 0xFE;
        mem[3] = 0xC3;
        mem[4] = 0xFE;
        mem[5] = 0xC8;
        mem[6] = 0xFE;
        mem[7] = 0x06;
        mem[8] = 0x00;
        mem[9] = 0x40;
        mem[10] = 0xFE;
        mem[11] = 0x0E;
        mem[12] = 0x00;
        mem[13] = 0x40;
        mem[14] = 0xFF;
        mem[15] = 0xC0;
        mem[16] = 0xFF;
        mem[17] = 0xC8;
        mem[18] = 0xFF;
        mem[19] = 0x06;
        mem[20] = 0x00;
        mem[21] = 0x40;
        mem[22] = 0xFF;
        mem[23] = 0x0E;
        mem[24] = 0x00;
        mem[25] = 0x40;
        mem[26] = 0xF4;
        mem[0x4000] = 0x7F;
        mem[0x4001] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // INC AL: 0xFF → 0; ZF; CF preserved
        cpu.set_al(0xFF);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x00);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_ne!(cpu.rflags & 1, 0); // CF preserved

        // INC BL: 0x7F → 0x80; OF; CF preserved clear
        cpu.set_gpr_u8_low(CpuState::RBX, 0x7F);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u8_low(CpuState::RBX), 0x80);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_eq!(cpu.rflags & 1, 0); // CF preserved

        // DEC AL: 0x00 → 0xFF; SF; CF preserved
        cpu.set_al(0x00);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xFF);
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_ne!(cpu.rflags & 1, 0); // CF preserved

        // INC byte [0x4000]: 0x7F → 0x80
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x80);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF

        // DEC byte [0x4000]: 0x80 → 0x7F
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x7F);

        // INC AX: 0x7FFF → 0x8000; OF; CF preserved
        cpu.set_ax(0x7FFF);
        cpu.set_cf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x8000);
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & 1, 0); // CF preserved

        // DEC AX: 0x0001 → 0; ZF; CF preserved clear
        cpu.set_ax(0x0001);
        cpu.set_cf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x0000);
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & 1, 0); // CF preserved

        // INC word [0x4000]: 0x007F → 0x0080
        bus.write_u16(0x4000, 0x007F).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x0080);

        // DEC word [0x4000]: 0x0080 → 0x007F
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x007F);
    }

    /// FF Group 5 CALL/JMP/PUSH r/m — /2 CALL near, /4 JMP near, /6 PUSH (SDM Vol. 2).
    #[test]
    fn grp5_call_jmp_push_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: FF D0          CALL AX
        // 2: F4             HLT (return landing)
        // 3: FF 16 00 40    CALL word [0x4000]
        // 7: F4             HLT
        // 8: FF E3          JMP BX
        // A: F4             HLT (should not reach)
        // B: FF 26 00 40    JMP word [0x4000]
        // F: F4             HLT (should not reach)
        // 10: FF F0         PUSH AX
        // 12: FF 36 00 40   PUSH word [0x4000]
        // FF /3 far CALL reg #UD covered by ud_exception_via_ivt_reserved_encodings
        mem[0] = 0xFF;
        mem[1] = 0xD0;
        mem[2] = 0xF4;
        mem[3] = 0xFF;
        mem[4] = 0x16;
        mem[5] = 0x00;
        mem[6] = 0x40;
        mem[7] = 0xF4;
        mem[8] = 0xFF;
        mem[9] = 0xE3;
        mem[0xA] = 0xF4;
        mem[0xB] = 0xFF;
        mem[0xC] = 0x26;
        mem[0xD] = 0x00;
        mem[0xE] = 0x40;
        mem[0xF] = 0xF4;
        mem[0x10] = 0xFF;
        mem[0x11] = 0xF0;
        mem[0x12] = 0xFF;
        mem[0x13] = 0x36;
        mem[0x14] = 0x00;
        mem[0x15] = 0x40;
        mem[0x16] = 0xFF;
        mem[0x17] = 0xD8;
        mem[0x18] = 0xF4;
        // Call/jmp targets
        mem[0x800] = 0xC3; // RET (near)
        mem[0x900] = 0xF4; // HLT
        mem[0x4000] = 0x00;
        mem[0x4001] = 0x09; // word 0x0900
        mem[0x4002] = 0x34;
        mem[0x4003] = 0x12; // word 0x1234 for PUSH mem

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        // CALL AX → 0x800: push return IP 2, jump
        cpu.set_ax(0x0800);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0800);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 2);

        // RET back to HLT at 2
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 2);
        step(&mut cpu, &mut bus).unwrap(); // HLT
        assert!(cpu.halted);

        // CALL word [0x4000] → 0x900
        cpu.halted = false;
        cpu.rip = 3;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 7); // return after CALL mem

        // JMP BX → 0x900
        cpu.rip = 8;
        cpu.set_gpr_u16(CpuState::RBX, 0x0900);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE); // no stack change

        // JMP word [0x4000] → 0x900
        cpu.rip = 0xB;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0900);

        // PUSH AX
        cpu.rip = 0x10;
        cpu.set_ax(0xABCD);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x12);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0xABCD);

        // PUSH word [0x4000] — use 0x1234 at 0x4002 via displacement change:
        // encoding still [0x4000]; overwrite target word for this step.
        bus.write_u16(0x4000, 0x1234).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x16);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0x1234);
    }

    /// FF Group 5 far CALL/JMP m16:16 — /3 CALL far, /5 JMP far (SDM Vol. 2).
    #[test]
    fn grp5_call_jmp_far_real_mode() {
        let mut mem = vec![0u8; 0x20000];
        // 0: FF 1E 00 40    CALL FAR [0x4000]
        // 4: F4             HLT (return landing after RETF)
        // 5: FF 2E 00 40    JMP FAR [0x4000]
        // 9: F4             HLT (should not reach after JMP)
        // Far CALL/JMP register #UD covered by ud_exception_via_ivt_reserved_encodings
        mem[0] = 0xFF;
        mem[1] = 0x1E;
        mem[2] = 0x00;
        mem[3] = 0x40;
        mem[4] = 0xF4;
        mem[5] = 0xFF;
        mem[6] = 0x2E;
        mem[7] = 0x00;
        mem[8] = 0x40;
        mem[9] = 0xF4;
        // Far pointer at DS:0x4000 → CS:IP = 0x1000:0x0200 → linear 0x10200
        mem[0x4000] = 0x00;
        mem[0x4001] = 0x02; // offset 0x0200
        mem[0x4002] = 0x00;
        mem[0x4003] = 0x10; // selector 0x1000
                            // Target: RETF then HLT at 0x1000:0x0200
        let target = (0x1000u64 << 4) + 0x0200;
        mem[target as usize] = 0xCB; // RETF
        mem[target as usize + 1] = 0xF4; // HLT (JMP landing)

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        // CALL FAR [0x4000]: push CS/IP, load 0x1000:0x0200
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.cs.base, 0x1000u64 << 4);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 4); // return IP
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0); // return CS

        // RETF back to HLT at 4
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 4);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
        step(&mut cpu, &mut bus).unwrap(); // HLT
        assert!(cpu.halted);

        // JMP FAR [0x4000] → 0x1000:0x0200 (HLT after we overwrite RETF)
        cpu.halted = false;
        cpu.rip = 5;
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        bus.write_u8(target, 0xF4).unwrap(); // HLT at far target
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE); // no stack change
        step(&mut cpu, &mut bus).unwrap(); // HLT
        assert!(cpu.halted);
    }

    /// AH/CH/DH/BH via ModR/M reg and r/m for MOV and OR (SDM Vol. 1 ┬º3.4.1.1; Vol. 2 MOV/OR).
    #[test]
    fn high_byte_modrm_mov_or_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 88 E0 = MOV AL, AH   (r/m=AL, reg=AH)
        // 8A E3 = MOV AH, BL   (reg=AH, r/m=BL)
        // 88 FD = MOV CH, BH   (r/m=CH, reg=BH)
        // 8A F9 = MOV BH, CL   (reg=BH, r/m=CL)
        // 08 E5 = OR  CH, AH   (r/m=CH, reg=AH)
        // 0A F1 = OR  DH, CL   (reg=DH, r/m=CL)
        // B4 77 = MOV AH, 0x77
        // B7 88 = MOV BH, 0x88
        // 80 E4 0F = AND AH, 0x0F  (Group 1 /4, r/m=AH)
        mem[0] = 0x88;
        mem[1] = 0xE0;
        mem[2] = 0x8A;
        mem[3] = 0xE3;
        mem[4] = 0x88;
        mem[5] = 0xFD;
        mem[6] = 0x8A;
        mem[7] = 0xF9;
        mem[8] = 0x08;
        mem[9] = 0xE5;
        mem[10] = 0x0A;
        mem[11] = 0xF1;
        mem[12] = 0xB4;
        mem[13] = 0x77;
        mem[14] = 0xB7;
        mem[15] = 0x88;
        mem[16] = 0x80;
        mem[17] = 0xE4; // mod=3,reg=4(AND),rm=4(AH)
        mem[18] = 0x0F;
        mem[19] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        // AX=0xABCD, BX=0x1234, CX=0x5678, DX=0x9ABC
        cpu.set_ax(0xABCD);
        cpu.set_gpr_u16(CpuState::RBX, 0x1234);
        cpu.set_gpr_u16(CpuState::RCX, 0x5678);
        cpu.set_gpr_u16(CpuState::RDX, 0x9ABC);
        let mut bus = VecBus { mem, ports: vec![] };

        // MOV AL, AH ΓåÆ AL=0xAB, AH unchanged ΓåÆ AX=0xABAB
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0xABAB);

        // MOV AH, BL ΓåÆ AH=0x34, AL preserved ΓåÆ AX=0x34AB
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x34AB);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1234);

        // MOV CH, BH ΓåÆ CH=0x12, CL preserved ΓåÆ CX=0x1278
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x1278);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1234);

        // MOV BH, CL ΓåÆ BH=0x78, BL preserved ΓåÆ BX=0x7834
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x7834);

        // OR CH, AH ΓåÆ CH |= AH = 0x12 | 0x34 = 0x36 ΓåÆ CX=0x3678
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x3678);
        assert_eq!(cpu.ax(), 0x34AB);

        // OR DH, CL ΓåÆ DH |= CL = 0x9A | 0x78 = 0xFA ΓåÆ DX=0xFABC
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0xFABC);

        // MOV AH, 0x77 ΓåÆ AX=0x77AB
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x77AB);

        // MOV BH, 0x88 ΓåÆ BX=0x8834
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x8834);

        // AND AH, 0x0F ΓåÆ AH=0x07, AL preserved ΓåÆ AX=0x07AB
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x07AB);
        assert_eq!(cpu.rflags & 1, 0); // CF cleared by AND
    }

    /// XCHG high-byte reg Γåö r/m (SDM Vol. 2 XCHG).
    #[test]
    fn high_byte_xchg_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 86 E3 = XCHG AH, BL
        mem[0] = 0x86;
        mem[1] = 0xE3;
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_ax(0x11AA);
        cpu.set_gpr_u16(CpuState::RBX, 0x22BB);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        // AHΓåöBL: AH=0xBB, BL=0x11; AL/BH preserved
        assert_eq!(cpu.ax(), 0xBBAA);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x2211);
    }

    /// MOV C6/C7 r/m,imm — Spec: Intel SDM Vol. 2 MOV.
    #[test]
    fn mov_rm_imm_c6_c7_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // C6 C0 5A = MOV AL, 0x5A
        // C6 06 00 40 99 = MOV byte [0x4000], 0x99
        // C7 C3 34 12 = MOV BX, 0x1234
        // C7 06 00 30 CD AB = MOV word [0x3000], 0xABCD
        // C6 /1 #UD covered by ud_exception_via_ivt_reserved_encodings
        mem[0] = 0xC6;
        mem[1] = 0xC0;
        mem[2] = 0x5A;
        mem[3] = 0xC6;
        mem[4] = 0x06;
        mem[5] = 0x00;
        mem[6] = 0x40;
        mem[7] = 0x99;
        mem[8] = 0xC7;
        mem[9] = 0xC3;
        mem[10] = 0x34;
        mem[11] = 0x12;
        mem[12] = 0xC7;
        mem[13] = 0x06;
        mem[14] = 0x00;
        mem[15] = 0x30;
        mem[16] = 0xCD;
        mem[17] = 0xAB;
        mem[18] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_al(0);
        cpu.rflags = 0x246;
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x5A);
        assert_eq!(cpu.rflags, flags_before); // MOV does not touch flags

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x99);
        assert_eq!(cpu.rflags, flags_before);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1234);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x3000).unwrap(), 0xABCD);
    }

    /// MOV A0–A3 AL/AX ↔ moffs — Spec: Intel SDM Vol. 2 MOV.
    #[test]
    fn mov_moffs_a0_a3_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // A0 00 40 = MOV AL, [0x4000]
        // A2 00 50 = MOV [0x5000], AL
        // A1 00 30 = MOV AX, [0x3000]
        // A3 00 60 = MOV [0x6000], AX
        // 2E A0 00 10 = MOV AL, CS:[0x1000]
        mem[0] = 0xA0;
        mem[1] = 0x00;
        mem[2] = 0x40;
        mem[3] = 0xA2;
        mem[4] = 0x00;
        mem[5] = 0x50;
        mem[6] = 0xA1;
        mem[7] = 0x00;
        mem[8] = 0x30;
        mem[9] = 0xA3;
        mem[10] = 0x00;
        mem[11] = 0x60;
        mem[12] = 0x2E;
        mem[13] = 0xA0;
        mem[14] = 0x00;
        mem[15] = 0x10;
        mem[16] = 0xF4;
        mem[0x4000] = 0xAB;
        mem[0x3000] = 0x34;
        mem[0x3001] = 0x12;
        mem[0x1000] = 0xCD; // CS=0 → linear 0x1000

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xAB);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x5000).unwrap(), 0xAB);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x1234);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x6000).unwrap(), 0x1234);

        cpu.set_al(0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xCD);
    }

    /// TEST A8/A9 AL/AX,imm — Spec: Intel SDM Vol. 2 TEST.
    #[test]
    fn test_al_ax_imm_a8_a9_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // A8 0F = TEST AL, 0x0F
        // A9 FF 00 = TEST AX, 0x00FF
        // A8 00 = TEST AL, 0 (ZF)
        mem[0] = 0xA8;
        mem[1] = 0x0F;
        mem[2] = 0xA9;
        mem[3] = 0xFF;
        mem[4] = 0x00;
        mem[5] = 0xA8;
        mem[6] = 0x00;
        mem[7] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_al(0xF0);
        cpu.set_cf(true);
        cpu.set_of(true);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xF0); // unchanged
        assert_eq!(cpu.rflags & 1, 0); // CF cleared
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF cleared
        assert_eq!(cpu.rflags & (1 << 6), 1 << 6); // ZF (0xF0 & 0x0F == 0)

        cpu.set_ax(0x12F0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x12F0);
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF=0 (0x12F0 & 0x00FF == 0x00F0)
        assert_eq!(cpu.rflags & (1 << 7), 0); // SF from 16-bit result (bit 15 clear)

        cpu.set_al(0x55);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x55);
        assert_eq!(cpu.rflags & (1 << 6), 1 << 6); // ZF
    }

    /// PUSHA stack layout then POPA restores GPRs (except SP from the saved slot).
    /// Spec: Intel SDM Vol. 2 "PUSHA/PUSHAD", "POPA/POPAD".
    #[test]
    fn pusha_popa_stack_layout_and_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x60; // PUSHA
        mem[1] = 0x61; // POPA
        mem[2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u16(CpuState::RAX, 0x1111);
        cpu.set_gpr_u16(CpuState::RCX, 0x2222);
        cpu.set_gpr_u16(CpuState::RDX, 0x3333);
        cpu.set_gpr_u16(CpuState::RBX, 0x4444);
        cpu.set_gpr_u16(CpuState::RBP, 0x5555);
        cpu.set_gpr_u16(CpuState::RSI, 0x6666);
        cpu.set_gpr_u16(CpuState::RDI, 0x7777);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0.wrapping_sub(16));
        // Highest addresses first: AX at sp0-2 … DI at sp0-16.
        assert_eq!(bus.read_u16(u64::from(sp0 - 2)).unwrap(), 0x1111); // AX
        assert_eq!(bus.read_u16(u64::from(sp0 - 4)).unwrap(), 0x2222); // CX
        assert_eq!(bus.read_u16(u64::from(sp0 - 6)).unwrap(), 0x3333); // DX
        assert_eq!(bus.read_u16(u64::from(sp0 - 8)).unwrap(), 0x4444); // BX
        assert_eq!(bus.read_u16(u64::from(sp0 - 10)).unwrap(), sp0); // original SP
        assert_eq!(bus.read_u16(u64::from(sp0 - 12)).unwrap(), 0x5555); // BP
        assert_eq!(bus.read_u16(u64::from(sp0 - 14)).unwrap(), 0x6666); // SI
        assert_eq!(bus.read_u16(u64::from(sp0 - 16)).unwrap(), 0x7777); // DI

        // Clobber GPRs (leave SP as after PUSHA).
        cpu.set_gpr_u16(CpuState::RAX, 0);
        cpu.set_gpr_u16(CpuState::RCX, 0);
        cpu.set_gpr_u16(CpuState::RDX, 0);
        cpu.set_gpr_u16(CpuState::RBX, 0);
        cpu.set_gpr_u16(CpuState::RBP, 0);
        cpu.set_gpr_u16(CpuState::RSI, 0);
        cpu.set_gpr_u16(CpuState::RDI, 0);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x1111);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x2222);
        assert_eq!(cpu.gpr_u16(CpuState::RDX), 0x3333);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x4444);
        assert_eq!(cpu.gpr_u16(CpuState::RBP), 0x5555);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x6666);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x7777);
        // POPA discards the saved SP; SP ends at the pre-PUSHA value.
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
    }

    /// ENTER nesting level 0 + LEAVE round-trip (SDM Vol. 2 ENTER/LEAVE).
    #[test]
    fn enter_level0_leave_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        // ENTER 8, 0
        mem[0] = 0xC8;
        mem[1] = 0x08;
        mem[2] = 0x00;
        mem[3] = 0x00;
        mem[4] = 0xC9; // LEAVE
        mem[5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u16(CpuState::RBP, 0xABCD);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        // After ENTER 8,0: PUSH old BP; BP = new frame; SP = BP - 8.
        assert_eq!(bus.read_u16(u64::from(sp0 - 2)).unwrap(), 0xABCD);
        let frame = sp0 - 2;
        assert_eq!(cpu.gpr_u16(CpuState::RBP), frame);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), frame.wrapping_sub(8));

        step(&mut cpu, &mut bus).unwrap(); // LEAVE
        assert_eq!(cpu.gpr_u16(CpuState::RBP), 0xABCD);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
    }

    /// ENTER nesting level 1 pushes old BP and the new frame pointer (display).
    /// Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §6.5.
    #[test]
    fn enter_nesting_level1_display() {
        let mut mem = vec![0u8; 0x10000];
        // ENTER 4, 1
        mem[0] = 0xC8;
        mem[1] = 0x04;
        mem[2] = 0x00;
        mem[3] = 0x01;
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u16(CpuState::RBP, 0xABCD);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        // Push old BP; frame_temp = SP; Push(frame_temp); BP = frame_temp; SP -= 4.
        assert_eq!(bus.read_u16(u64::from(sp0 - 2)).unwrap(), 0xABCD);
        let frame = sp0 - 2;
        assert_eq!(bus.read_u16(u64::from(sp0 - 4)).unwrap(), frame);
        assert_eq!(cpu.gpr_u16(CpuState::RBP), frame);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), frame.wrapping_sub(2 + 4));
    }

    /// ENTER nesting level 2 copies one display word from the caller's frame.
    /// Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §6.5.
    #[test]
    fn enter_nesting_level2_copies_display() {
        let mut mem = vec![0u8; 0x10000];
        // ENTER 0, 1 then ENTER 0, 2
        mem[0] = 0xC8;
        mem[1] = 0x00;
        mem[2] = 0x00;
        mem[3] = 0x01;
        mem[4] = 0xC8;
        mem[5] = 0x00;
        mem[6] = 0x00;
        mem[7] = 0x02;
        mem[8] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u16(CpuState::RBP, 0x1111);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // ENTER 0,1
        let parent_bp = cpu.gpr_u16(CpuState::RBP);
        assert_eq!(parent_bp, sp0 - 2);
        // Parent display: [BP]=old BP, [BP-2]=frame_temp (= parent_bp).
        assert_eq!(bus.read_u16(u64::from(parent_bp)).unwrap(), 0x1111);
        assert_eq!(bus.read_u16(u64::from(parent_bp - 2)).unwrap(), parent_bp);

        step(&mut cpu, &mut bus).unwrap(); // ENTER 0,2
        let child_bp = cpu.gpr_u16(CpuState::RBP);
        // [BP] = parent frame pointer (pushed at start).
        assert_eq!(bus.read_u16(u64::from(child_bp)).unwrap(), parent_bp);
        // [BP-2] = copied display entry from parent [parent_bp-2] (= parent_bp).
        assert_eq!(bus.read_u16(u64::from(child_bp - 2)).unwrap(), parent_bp);
        // [BP-4] = child's frame_temp (= child_bp).
        assert_eq!(bus.read_u16(u64::from(child_bp - 4)).unwrap(), child_bp);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), child_bp - 4);
    }

    /// ENTERD (0x66 ENTER) nesting 0 + LEAVE opsize-32 round-trip.
    /// Spec: Intel SDM Vol. 2 "ENTER"/"LEAVE"; Ch. 2 (66H); Vol. 1 §3.6 / §6.5.
    #[test]
    fn enterd_level0_leave_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        // 66 C8 08 00 00 = ENTERD 8, 0
        mem[0] = 0x66;
        mem[1] = 0xC8;
        mem[2] = 0x08;
        mem[3] = 0x00;
        mem[4] = 0x00;
        // 66 C9 = LEAVE (opsize 32)
        mem[5] = 0x66;
        mem[6] = 0xC9;
        mem[7] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u32(CpuState::RBP, 0xAAAA_ABCD);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        // Push EBP (4); frame = SP; EBP = frame; SP = frame - 8.
        assert_eq!(bus.read_u32(u64::from(sp0 - 4)).unwrap(), 0xAAAA_ABCD);
        let frame = u32::from(sp0 - 4);
        assert_eq!(cpu.gpr_u32(CpuState::RBP), frame);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), (frame as u16).wrapping_sub(8));

        step(&mut cpu, &mut bus).unwrap(); // LEAVE opsize32
        assert_eq!(cpu.gpr_u32(CpuState::RBP), 0xAAAA_ABCD);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
    }

    /// ENTERD nesting level 1: push EBP, push frame_temp (dword display).
    /// Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §6.5; Ch. 2 (66H).
    #[test]
    fn enterd_nesting_level1_display() {
        let mut mem = vec![0u8; 0x10000];
        // 66 C8 04 00 01 = ENTERD 4, 1
        mem[0] = 0x66;
        mem[1] = 0xC8;
        mem[2] = 0x04;
        mem[3] = 0x00;
        mem[4] = 0x01;
        mem[5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u32(CpuState::RBP, 0x1111_ABCD);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(u64::from(sp0 - 4)).unwrap(), 0x1111_ABCD);
        let frame = u32::from(sp0 - 4);
        assert_eq!(bus.read_u32(u64::from(sp0 - 8)).unwrap(), frame);
        assert_eq!(cpu.gpr_u32(CpuState::RBP), frame);
        // frame_temp push (4) + alloc 4.
        assert_eq!(
            cpu.gpr_u16(CpuState::RSP),
            (frame as u16).wrapping_sub(4 + 4)
        );
    }

    /// PUSHAD stack layout then POPAD restores GPRs (discards saved ESP).
    /// Spec: Intel SDM Vol. 2 "PUSHA/PUSHAD", "POPA/POPAD"; Ch. 2 (66H).
    #[test]
    fn pushad_popad_stack_layout_and_round_trip() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x66;
        mem[1] = 0x60; // PUSHAD
        mem[2] = 0x66;
        mem[3] = 0x61; // POPAD
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        cpu.set_gpr_u32(CpuState::RAX, 0x1111_1111);
        cpu.set_gpr_u32(CpuState::RCX, 0x2222_2222);
        cpu.set_gpr_u32(CpuState::RDX, 0x3333_3333);
        cpu.set_gpr_u32(CpuState::RBX, 0x4444_4444);
        cpu.set_gpr_u32(CpuState::RBP, 0x5555_5555);
        cpu.set_gpr_u32(CpuState::RSI, 0x6666_6666);
        cpu.set_gpr_u32(CpuState::RDI, 0x7777_7777);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0.wrapping_sub(32));
        assert_eq!(bus.read_u32(u64::from(sp0 - 4)).unwrap(), 0x1111_1111); // EAX
        assert_eq!(bus.read_u32(u64::from(sp0 - 8)).unwrap(), 0x2222_2222); // ECX
        assert_eq!(bus.read_u32(u64::from(sp0 - 12)).unwrap(), 0x3333_3333); // EDX
        assert_eq!(bus.read_u32(u64::from(sp0 - 16)).unwrap(), 0x4444_4444); // EBX
        assert_eq!(bus.read_u32(u64::from(sp0 - 20)).unwrap(), u32::from(sp0)); // orig ESP
        assert_eq!(bus.read_u32(u64::from(sp0 - 24)).unwrap(), 0x5555_5555); // EBP
        assert_eq!(bus.read_u32(u64::from(sp0 - 28)).unwrap(), 0x6666_6666); // ESI
        assert_eq!(bus.read_u32(u64::from(sp0 - 32)).unwrap(), 0x7777_7777); // EDI

        cpu.set_gpr_u32(CpuState::RAX, 0);
        cpu.set_gpr_u32(CpuState::RCX, 0);
        cpu.set_gpr_u32(CpuState::RDX, 0);
        cpu.set_gpr_u32(CpuState::RBX, 0);
        cpu.set_gpr_u32(CpuState::RBP, 0);
        cpu.set_gpr_u32(CpuState::RSI, 0);
        cpu.set_gpr_u32(CpuState::RDI, 0);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x1111_1111);
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0x2222_2222);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0x3333_3333);
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x4444_4444);
        assert_eq!(cpu.gpr_u32(CpuState::RBP), 0x5555_5555);
        assert_eq!(cpu.gpr_u32(CpuState::RSI), 0x6666_6666);
        assert_eq!(cpu.gpr_u32(CpuState::RDI), 0x7777_7777);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
    }

    /// PUSHFD/POPFD round-trip in real-address mode (opsize 32).
    /// Spec: Intel SDM Vol. 2 "PUSHF/PUSHFD/PUSHFQ", "POPF/POPFD/POPFQ"; Ch. 2 (66H).
    /// `VM` is unaffected by POPFD; `RF` is cleared after POPF (Vol. 2 POPF note).
    #[test]
    fn pushfd_popfd_round_trip_preserves_vm_clears_rf() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x66;
        mem[1] = 0x9C; // PUSHFD
        mem[2] = 0x66;
        mem[3] = 0x9D; // POPFD
        mem[4] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let sp0 = 0xFFFE_u16;
        cpu.set_gpr_u16(CpuState::RSP, sp0);
        // CF+PF+AF+ZF+SF+IF+OF + synthetic VM/RF.
        cpu.rflags = 0x0002_0AD7 | (1 << 16) | (1 << 17);
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // PUSHFD
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0.wrapping_sub(4));
        assert_eq!(
            bus.read_u32(u64::from(sp0 - 4)).unwrap(),
            (flags_before as u32)
        );

        // Clobber writable flags but keep VM/RF set for the POPFD preserve check.
        cpu.rflags = (1 << 16) | (1 << 17) | 2;
        step(&mut cpu, &mut bus).unwrap(); // POPFD
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp0);
        // Lower image restored (bit 1 forced); VM sticky; RF cleared.
        assert_eq!(cpu.rflags & 0xFFFF, u64::from(flags_before as u16 | 2));
        assert_eq!(cpu.rflags & (1 << 16), 0); // RF cleared
        assert_ne!(cpu.rflags & (1 << 17), 0); // VM preserved
    }

    /// RET iw / RETF iw release stack bytes after the return frame.
    /// Spec: Intel SDM Vol. 2 "RET".
    #[test]
    fn ret_retf_imm16_release_stack() {
        let mut mem = vec![0u8; 0x10000];
        // Near: RET 4 with IP on stack and 4 dummy bytes below the frame.
        mem[0] = 0xC2;
        mem[1] = 0x04;
        mem[2] = 0x00;
        // Far: RETF 2 at 0x100
        mem[0x100] = 0xCA;
        mem[0x101] = 0x02;
        mem[0x102] = 0x00;

        // Near frame at SP=0xFFF0: IP=0x2000, then 4 pad bytes, then marker 0xBEEF at 0xFFF6.
        mem[0xFFF0] = 0x00;
        mem[0xFFF1] = 0x20; // return IP
        mem[0xFFF2] = 0x11;
        mem[0xFFF3] = 0x11;
        mem[0xFFF4] = 0x22;
        mem[0xFFF5] = 0x22;
        mem[0xFFF6] = 0xEF;
        mem[0xFFF7] = 0xBE;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFF0);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x2000);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF6);
        assert_eq!(bus.read_u16(0xFFF6).unwrap(), 0xBEEF);

        // Far frame at SP=0xFFF0: IP, CS, then 2 pad bytes, marker at 0xFFF6.
        bus.mem[0xFFF0] = 0x34;
        bus.mem[0xFFF1] = 0x12; // IP
        bus.mem[0xFFF2] = 0x00;
        bus.mem[0xFFF3] = 0x30; // CS 0x3000
        bus.mem[0xFFF4] = 0xAA;
        bus.mem[0xFFF5] = 0xAA;
        bus.mem[0xFFF6] = 0xEF;
        bus.mem[0xFFF7] = 0xBE;
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0x100;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFF0);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x1234);
        assert_eq!(cpu.cs.selector, 0x3000);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF6);
    }

    /// POP r/m16 (8F /0) reg and mem forms.
    /// Spec: Intel SDM Vol. 2 "POP". /1–/7 #UD covered by ud_exception_via_ivt_reserved_encodings.
    #[test]
    fn pop_rm16_reg_mem() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x8F;
        mem[1] = 0xC3; // POP BX
        mem[2] = 0x8F;
        mem[3] = 0x06;
        mem[4] = 0x00;
        mem[5] = 0x40; // POP [0x4000]
        mem[6] = 0xF4;

        // Stack: 0xAAAA then 0xBBBB
        mem[0xFFFA] = 0xBB;
        mem[0xFFFB] = 0xBB;
        mem[0xFFFC] = 0xAA;
        mem[0xFFFD] = 0xAA;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFC);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xAAAA);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        cpu.set_gpr_u16(CpuState::RSP, 0xFFFA);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0xBBBB);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFC);
    }

    /// LES/LDS load m16:16 into r16 + ES/DS (SDM Vol. 2 LES/LDS). Real mode only.
    #[test]
    fn les_lds_load_far_pointer() {
        let mut mem = vec![0u8; 0x10000];
        // Far pointer at DS:0x2000 — offset 0x5678, segment 0x1234
        mem[0x2000] = 0x78;
        mem[0x2001] = 0x56;
        mem[0x2002] = 0x34;
        mem[0x2003] = 0x12;
        // Far pointer at DS:0x3000 — offset 0xABCD, segment 0xF000
        mem[0x3000] = 0xCD;
        mem[0x3001] = 0xAB;
        mem[0x3002] = 0x00;
        mem[0x3003] = 0xF0;
        // C4 06 00 20 = LES AX, [0x2000]
        // C5 1E 00 30 = LDS BX, [0x3000]
        mem[0] = 0xC4;
        mem[1] = 0x06;
        mem[2] = 0x00;
        mem[3] = 0x20;
        mem[4] = 0xC5;
        mem[5] = 0x1E;
        mem[6] = 0x00;
        mem[7] = 0x30;
        mem[8] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0x9999);
        cpu.rip = 0;
        cpu.rflags = 0x246; // IF+reserved; sticky pattern for "flags unchanged"
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 0x5678);
        assert_eq!(cpu.es.selector, 0x1234);
        assert_eq!(cpu.es.base, 0x1234u64 << 4);
        assert_eq!(cpu.rflags, flags_before);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xABCD);
        assert_eq!(cpu.ds.selector, 0xF000);
        assert_eq!(cpu.ds.base, 0xF000u64 << 4);
        assert_eq!(cpu.rflags, flags_before);
    }

    /// LES/LDS register form is #UD via IVT (SDM Vol. 2 LES/LDS; Vol. 3 §6.15).
    #[test]
    fn les_lds_register_source_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT[6] → 0000:0B00
        mem[24] = 0x00;
        mem[25] = 0x0B;
        mem[26] = 0x00;
        mem[27] = 0x00;
        mem[0] = 0xC4;
        mem[1] = 0xC0; // LES AX, AX — mod=11 → #UD
        mem[2] = 0xC5;
        mem[3] = 0xDB; // LDS BX, BX — mod=11 → #UD
        mem[0xB00] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        // Second case after returning to next insn via fresh RIP setup
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 2;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.halted = false;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 2);
    }

    /// XLATB: AL ← DS:[BX+AL] (SDM Vol. 2 XLAT/XLATB); segment override honored.
    #[test]
    fn xlat_table_lookup_and_segment_override() {
        let mut mem = vec![0u8; 0x20000];
        // DS=0 table at BX=0x1000: index AL=0x05 → 0xAB
        mem[0x1005] = 0xAB;
        // ES=0x1000 table at BX=0x0200: index AL=0x03 → linear 0x10203
        mem[0x10203] = 0xCD;
        // D7; 26 D7; F4
        mem[0] = 0xD7;
        mem[1] = 0x26;
        mem[2] = 0xD7;
        mem[3] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0x1000);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RBX, 0x1000);
        cpu.set_al(0x05);
        cpu.rflags = 0x246;
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xAB);
        assert_eq!(cpu.rflags, flags_before);
        cpu.set_gpr_u16(CpuState::RBX, 0x0200);
        cpu.set_al(0x03);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xCD);
        assert_eq!(cpu.rflags, flags_before);
    }

    /// IMUL r16, r/m16, imm — opcodes 69/6B (SDM Vol. 2 "IMUL").
    /// CF=OF set iff signed product does not fit in r16; SF/ZF/AF/PF undefined.
    #[test]
    fn imul_imm_69_6b_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: 69 D8 02 00     IMUL BX, AX, 2
        // 4: 69 D8 00 01     IMUL BX, AX, 0x100
        // 8: 6B D8 FD        IMUL BX, AX, -3 (imm8)
        // B: 6B DB FF        IMUL BX, BX, -1 (two-op sugar)
        // E: 69 1E 00 40 03 00  IMUL BX, [0x4000], 3
        mem[0] = 0x69;
        mem[1] = 0xD8;
        mem[2] = 0x02;
        mem[3] = 0x00;
        mem[4] = 0x69;
        mem[5] = 0xD8;
        mem[6] = 0x00;
        mem[7] = 0x01;
        mem[8] = 0x6B;
        mem[9] = 0xD8;
        mem[10] = 0xFD;
        mem[11] = 0x6B;
        mem[12] = 0xDB;
        mem[13] = 0xFF;
        mem[14] = 0x69;
        mem[15] = 0x1E;
        mem[16] = 0x00;
        mem[17] = 0x40;
        mem[18] = 0x03;
        mem[19] = 0x00;
        mem[20] = 0xF4;
        mem[0x4000] = 0x05;
        mem[0x4001] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // IMUL BX, AX, 2: 3*2=6 fits → CF=OF=0; AX unchanged
        cpu.set_ax(3);
        cpu.set_gpr_u16(CpuState::RBX, 0xDEAD);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 6);
        assert_eq!(cpu.ax(), 3);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, AX, 0x100: 0x100*0x100=0x10000 does not fit in i16 → CF=OF=1
        cpu.set_ax(0x0100);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, AX, -3: (-2)*(-3)=6 fits
        cpu.set_ax(0xFFFE); // -2
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 6);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, BX, -1: 6*(-1)=-6 fits
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xFFFA); // -6
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, [0x4000], 3: 5*3=15; memory unchanged
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 15);
        assert_eq!(bus.read_u16(0x4000).unwrap(), 5);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);
    }

    /// IMUL r16/r32, r/m16/r/m32 — opcode 0F AF (SDM Vol. 2 "IMUL").
    /// Dest = ModRM.reg * r/m; CF=OF iff signed product does not fit in dest width.
    #[test]
    fn imul_0f_af_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 0: 0F AF D8          IMUL BX, AX
        // 3: 0F AF D8          IMUL BX, AX (overflow)
        // 6: 0F AF 1E 00 40    IMUL BX, [0x4000]
        // B: 66 0F AF C3       IMUL EAX, EBX
        // F: 66 0F AF C3       IMUL EAX, EBX (overflow)
        // 13: F4               HLT
        mem[0] = 0x0F;
        mem[1] = 0xAF;
        mem[2] = 0xD8;
        mem[3] = 0x0F;
        mem[4] = 0xAF;
        mem[5] = 0xD8;
        mem[6] = 0x0F;
        mem[7] = 0xAF;
        mem[8] = 0x1E;
        mem[9] = 0x00;
        mem[10] = 0x40;
        mem[11] = 0x66;
        mem[12] = 0x0F;
        mem[13] = 0xAF;
        mem[14] = 0xC3;
        mem[15] = 0x66;
        mem[16] = 0x0F;
        mem[17] = 0xAF;
        mem[18] = 0xC3;
        mem[19] = 0xF4;
        mem[0x4000] = 0x05;
        mem[0x4001] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // IMUL BX, AX: 3*2=6 fits → CF=OF=0; AX unchanged
        cpu.set_ax(2);
        cpu.set_gpr_u16(CpuState::RBX, 3);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 6);
        assert_eq!(cpu.ax(), 2);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, AX: 0x100*0x100=0x10000 does not fit in i16 → CF=OF=1
        cpu.set_ax(0x0100);
        cpu.set_gpr_u16(CpuState::RBX, 0x0100);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // IMUL BX, [0x4000]: 7*5=35; memory unchanged
        cpu.set_gpr_u16(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 35);
        assert_eq!(bus.read_u16(0x4000).unwrap(), 5);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EAX, EBX: 0x10 * 0x20 = 0x200 fits → CF=OF=0
        cpu.set_gpr_u32(CpuState::RAX, 0x10);
        cpu.set_gpr_u32(CpuState::RBX, 0x20);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x200);
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x20);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EAX, EBX: 0x10000 * 0x10000 = 0x1_0000_0000 does not fit in i32
        cpu.set_gpr_u32(CpuState::RAX, 0x0001_0000);
        cpu.set_gpr_u32(CpuState::RBX, 0x0001_0000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);
    }

    /// SMSW/LMSW — opcode 0F 01 /4 and /6 (SDM Vol. 2 SMSW/LMSW; Vol. 3 CR0).
    /// SMSW stores CR0[15:0]; LMSW loads CR0[15:0] and cannot clear PE.
    /// PE bit updates do not enter protected-mode execution here.
    #[test]
    fn smsw_lmsw_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // +0: 0F 01 E0         SMSW AX          (mod=11, /4, rm=AX)
        // +3: 0F 01 26 00 40   SMSW [0x4000]    (mem always 16-bit)
        // +8: 66 0F 01 E3      SMSW EBX         (opsize32 zero-extend)
        // +C: B8 01 00         MOV AX, 1        (PE=1)
        // +F: 0F 01 F0         LMSW AX
        // +12: B8 00 00        MOV AX, 0
        // +15: 0F 01 F0        LMSW AX          (must not clear PE)
        // +18: B8 10 00        MOV AX, 0x10     (ET)
        // +1B: 0F 01 F0        LMSW AX          (from CR0 with PE still set → PE stays)
        // +1E: F4              HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0xE0; // 11_100_000 SMSW AX
        mem[code + 3] = 0x0F;
        mem[code + 4] = 0x01;
        mem[code + 5] = 0x26; // 00_100_110 SMSW [disp16]
        mem[code + 6] = 0x00;
        mem[code + 7] = 0x40;
        mem[code + 8] = 0x66;
        mem[code + 9] = 0x0F;
        mem[code + 10] = 0x01;
        mem[code + 11] = 0xE3; // SMSW EBX
        mem[code + 12] = 0xB8;
        mem[code + 13] = 0x01;
        mem[code + 14] = 0x00;
        mem[code + 15] = 0x0F;
        mem[code + 16] = 0x01;
        mem[code + 17] = 0xF0; // 11_110_000 LMSW AX
        mem[code + 18] = 0xB8;
        mem[code + 19] = 0x00;
        mem[code + 20] = 0x00;
        mem[code + 21] = 0x0F;
        mem[code + 22] = 0x01;
        mem[code + 23] = 0xF0;
        mem[code + 24] = 0xB8;
        mem[code + 25] = 0x10;
        mem[code + 26] = 0x00;
        mem[code + 27] = 0x0F;
        mem[code + 28] = 0x01;
        mem[code + 29] = 0xF0;
        mem[code + 30] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        // Reset CR0 low = 0x0010 (ET). Spec: typical real-mode after RESET.
        assert_eq!(cpu.cr0 as u16, 0x0010);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x0010);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x0010);

        cpu.set_gpr_u32(CpuState::RBX, 0xFFFF_FFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x0000_0010);

        step(&mut cpu, &mut bus).unwrap(); // MOV AX,1
        step(&mut cpu, &mut bus).unwrap(); // LMSW AX → set PE
        assert_eq!(cpu.cr0 & 0xFFFF, 0x0001);
        assert_eq!(cpu.cr0 & 1, 1, "PE set in CR0");

        step(&mut cpu, &mut bus).unwrap(); // MOV AX,0
        step(&mut cpu, &mut bus).unwrap(); // LMSW AX — must not clear PE
        assert_eq!(cpu.cr0 & 1, 1, "LMSW cannot clear PE");
        assert_eq!(cpu.cr0 & 0xFFFF, 0x0001);

        step(&mut cpu, &mut bus).unwrap(); // MOV AX,0x10
        step(&mut cpu, &mut bus).unwrap(); // LMSW AX with PE sticky
        assert_eq!(cpu.cr0 & 0xFFFF, 0x0011, "ET loaded; PE remains set");
    }

    /// LMSW PE=1 enables both MOV DS and direct far JMP GDT cache loads.
    /// Spec: Intel SDM Vol. 2 LMSW / MOV Sreg / JMP; Vol. 3
    /// §§2.5, 3.4.3–3.5.1, 5.8.1.
    #[test]
    fn lmsw_pe_enables_mov_ds_and_far_jmp_gdt_loads() {
        let mut mem = vec![0u8; 0x30000];
        let gdt = 0x8000usize;
        // GDT[0]=null; GDT[1] selector 0x08: data base=0x0002_0000 limit=0xFFFF access=0x92
        let desc = encode_seg_desc(0x0002_0000, 0xFFFF, 0x92, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);
        // GDT[2] selector 0x10: D=0 code base=0x0001_0000, limit=0xFFFF.
        let code_desc = encode_seg_desc(0x0001_0000, 0xFFFF, 0x9A, 0x00);
        mem[gdt + 16..gdt + 24].copy_from_slice(&code_desc);

        let code = 0x1000usize;
        // +0: B8 01 00         MOV AX, 1
        // +3: 0F 01 F0         LMSW AX            (CR0.PE ← 1)
        // +6: B8 08 00         MOV AX, 0x08
        // +9: 8E D8            MOV DS, AX         (GDT load)
        // +B: EA 00 02 10 00   JMP 0010:0200      (GDT code load)
        mem[code] = 0xB8;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x0F;
        mem[code + 4] = 0x01;
        mem[code + 5] = 0xF0;
        mem[code + 6] = 0xB8;
        mem[code + 7] = 0x08;
        mem[code + 8] = 0x00;
        mem[code + 9] = 0x8E;
        mem[code + 10] = 0xD8;
        mem[code + 11] = 0xEA;
        mem[code + 12] = 0x00;
        mem[code + 13] = 0x02;
        mem[code + 14] = 0x10;
        mem[code + 15] = 0x00;
        mem[0x10200] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0xFFFF_FFFF; // prior unreal cache must be replaced by GDT load
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 23; // null + data + code
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // MOV AX, 1
        step(&mut cpu, &mut bus).unwrap(); // LMSW AX → set PE
        assert_eq!(cpu.cr0 & 1, 1, "LMSW sets CR0.PE");

        step(&mut cpu, &mut bus).unwrap(); // MOV AX, 0x08
        step(&mut cpu, &mut bus).unwrap(); // MOV DS, AX
        assert_eq!(cpu.ds.selector, 0x08);
        assert_eq!(cpu.ds.base, 0x0002_0000, "PE=1 MOV DS loads GDT base");
        assert_eq!(cpu.ds.limit, 0xFFFF, "PE=1 MOV DS loads GDT limit");
        assert_eq!(cpu.ds.flags, 0x0092);

        step(&mut cpu, &mut bus).unwrap(); // JMP far
        assert_eq!(cpu.cs.selector, 0x0010);
        assert_eq!(cpu.cs.base, 0x0001_0000);
        assert_eq!(cpu.cs.limit, 0xFFFF);
        assert_eq!(cpu.cs.flags, 0x009A);
        assert_eq!(cpu.ip16(), 0x0200);
    }

    /// CLTS — opcode 0F 06. Clears CR0.TS (bit 3) only; all other CR0 bits preserved.
    /// Spec: Intel SDM Vol. 2 "CLTS—Clear Task-Switched Flag in CR0"; Vol. 3 §2.5 (CR0.TS).
    /// Real-mode only here; PM CPL/#GP checks are out of scope.
    #[test]
    fn clts_clears_only_cr0_ts() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // +0: 0F 06   CLTS
        // +2: F4      HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x06;
        mem[code + 2] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        // CD|NW|ET|TS|PE — TS (bit 3) and PE (bit 0) both set; CLTS must clear only TS.
        // Spec: CR0.TS = bit 3; CLTS clears TS without modifying other CR0 bits.
        cpu.cr0 = 0x6000_0019;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // CLTS
        assert_eq!(cpu.cr0 & (1 << 3), 0, "CR0.TS must be cleared");
        assert_eq!(cpu.cr0 & 1, 1, "PE must be preserved");
        assert_eq!(
            cpu.cr0, 0x6000_0011,
            "only TS (bit 3) cleared; CD|NW|ET|PE remain"
        );
        assert_eq!(cpu.rip, (code + 2) as u64);
    }

    /// MOV r32, CR0 — opcode 0F 20 /r (SDM Vol. 2 "MOV—Move to/from Control
    /// Registers"; Vol. 3 §2.5 CR0). Reads the full 32-bit CR0 into a GPR.
    #[test]
    fn mov_r32_cr0_reads_reset_value() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // +0: 0F 20 C0   MOV EAX, CR0
        // +3: F4         HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x20;
        mem[code + 2] = 0xC0; // 11_000_000: reg=0 (CR0), rm=0 (EAX)
        mem[code + 3] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        // Reset CR0 = CD|NW|ET (Spec: typical real-mode after RESET).
        assert_eq!(cpu.cr0, 0x6000_0010);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x6000_0010);
        assert_eq!(cpu.gpr_u32(CpuState::RAX) & 0xFFFF, 0x0010, "ET set");
    }

    /// MOV CR0 sets/clears PE; PE=1 enables DS GDT loads; clearing PE restores <<4.
    /// Spec: Intel SDM Vol. 2 "MOV—Move to/from Control Registers" / "MOV" (Sreg);
    /// Vol. 3 §2.5, §3.4.2–§3.5.1.
    #[test]
    fn mov_cr0_r32_sets_and_clears_pe_ds_gdt_then_real() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x8000usize;
        let desc = encode_seg_desc(0x0003_0000, 0x0FFF, 0x93, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        // +0:  66 B8 11 00 00 60   MOV EAX, 0x60000011  (PE=1)
        // +6:  0F 22 C0            MOV CR0, EAX
        // +9:  B8 08 00            MOV AX, 0x08
        // +C:  8E D8               MOV DS, AX           (GDT load while PE=1)
        // +E:  66 B8 10 00 00 60   MOV EAX, 0x60000010  (PE=0)
        // +14: 0F 22 C0            MOV CR0, EAX
        // +17: B8 34 12            MOV AX, 0x1234
        // +1A: 8E D8               MOV DS, AX           (real-mode <<4 after PE clear)
        // +1C: F4                  HLT
        mem[code] = 0x66;
        mem[code + 1] = 0xB8;
        mem[code + 2] = 0x11;
        mem[code + 3] = 0x00;
        mem[code + 4] = 0x00;
        mem[code + 5] = 0x60;
        mem[code + 6] = 0x0F;
        mem[code + 7] = 0x22;
        mem[code + 8] = 0xC0;
        mem[code + 9] = 0xB8;
        mem[code + 10] = 0x08;
        mem[code + 11] = 0x00;
        mem[code + 12] = 0x8E;
        mem[code + 13] = 0xD8;
        mem[code + 14] = 0x66;
        mem[code + 15] = 0xB8;
        mem[code + 16] = 0x10;
        mem[code + 17] = 0x00;
        mem[code + 18] = 0x00;
        mem[code + 19] = 0x60;
        mem[code + 20] = 0x0F;
        mem[code + 21] = 0x22;
        mem[code + 22] = 0xC0;
        mem[code + 23] = 0xB8;
        mem[code + 24] = 0x34;
        mem[code + 25] = 0x12;
        mem[code + 26] = 0x8E;
        mem[code + 27] = 0xD8;
        mem[code + 28] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // MOV EAX, 0x60000011
        step(&mut cpu, &mut bus).unwrap(); // MOV CR0, EAX
        assert_eq!(cpu.cr0 & 1, 1, "PE set via MOV CR0");

        step(&mut cpu, &mut bus).unwrap(); // MOV AX, 0x08
        step(&mut cpu, &mut bus).unwrap(); // MOV DS, AX
        assert_eq!(cpu.ds.base, 0x0003_0000, "PE=1 MOV DS loads GDT");
        assert_eq!(cpu.ds.limit, 0x0FFF);

        step(&mut cpu, &mut bus).unwrap(); // MOV EAX, 0x60000010
        step(&mut cpu, &mut bus).unwrap(); // MOV CR0, EAX — clears PE
        assert_eq!(cpu.cr0 & 1, 0, "MOV CR0 (unlike LMSW) can clear PE");

        step(&mut cpu, &mut bus).unwrap(); // MOV AX, 0x1234
        step(&mut cpu, &mut bus).unwrap(); // MOV DS, AX
        assert_eq!(cpu.ds.selector, 0x1234);
        assert_eq!(
            cpu.ds.base,
            0x1234u64 << 4,
            "PE=0 restores real-mode selector<<4 DS load"
        );
        // Sticky unreal: GDT-loaded limit survives real-mode selector update.
        assert_eq!(cpu.ds.limit, 0x0FFF);
    }

    /// Parsed descriptors retain the access byte and AVL/L/D-B/G nibble in
    /// their architectural positions, for both byte and page granularity.
    /// Spec: Intel SDM Vol. 3 §§3.4.3–3.4.5.
    #[test]
    fn parsed_segment_descriptors_preserve_full_attributes() {
        let byte_granular = encode_seg_desc(0x1234_5000, 0xA_BCDE, 0x97, 0x70);
        let (base, limit, flags) = parse_data_segment_descriptor(byte_granular, 0x0008, 0).unwrap();
        assert_eq!(base, 0x1234_5000);
        assert_eq!(limit, 0xA_BCDE);
        assert_eq!(flags, 0x7097, "access + AVL/L/D-B must be cached");

        let page_granular = encode_seg_desc(0x5678_9000, 0x1_2345, 0x96, 0xD0);
        let (base, limit, flags) =
            parse_stack_segment_descriptor(page_granular, 0x0010, 0).unwrap();
        assert_eq!(base, 0x5678_9000);
        assert_eq!(limit, 0x1234_5FFF);
        assert_eq!(flags, 0xD096, "access + AVL/D-B/G must be cached");
    }

    /// PE=1 MOV DS loads data-segment descriptor from GDT (base/limit/AR + G-bit).
    /// Spec: Intel SDM Vol. 2 MOV (Sreg, r/m16); Vol. 3 §3.4.5 / §3.5.1.
    #[test]
    fn mov_ds_pe1_loads_gdt_data_descriptor() {
        let mut mem = vec![0u8; 0x10000];
        // IVT unused; GDT at 0x5000
        let gdt = 0x5000usize;
        // Selector 0x08: base=0x0011_2200, limit20=0xF_FFFF, G=1
        // → effective limit 0xFFFF_FFFF, cached attributes 0x8092.
        let desc = encode_seg_desc(0x0011_2200, 0xF_FFFF, 0x92, 0x80);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        // B8 08 00  MOV AX, 0x08
        // 8E D8     MOV DS, AX
        // F4        HLT
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD8;
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0xABCD);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1; // PE=1
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ds.selector, 0x08);
        assert_eq!(cpu.ds.base, 0x0011_2200);
        assert_eq!(cpu.ds.limit, 0xFFFF_FFFF, "G=1 expands limit");
        assert_eq!(cpu.ds.flags, 0x8092);
        assert_eq!(cpu.ip16(), (code + 5) as u16);
    }

    /// PE=1 MOV ES also loads from GDT (same data-segment path as DS).
    /// Spec: Intel SDM Vol. 2 MOV (Sreg, r/m16); Vol. 3 §3.5.1.
    #[test]
    fn mov_es_pe1_loads_gdt_data_descriptor() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        // Byte-granular descriptor with AVL=1 and L=1; cache the attributes
        // exactly even though neither changes this slice's execution behavior.
        let desc = encode_seg_desc(0x0000_4000, 0x1FFF, 0x93, 0x30);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        // B8 08 00  MOV AX, 0x08
        // 8E C0     MOV ES, AX
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xC0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.es.selector, 0x08);
        assert_eq!(cpu.es.base, 0x0000_4000);
        assert_eq!(cpu.es.limit, 0x1FFF);
        assert_eq!(cpu.es.flags, 0x3093);
        assert!(!cpu.es.default_big(), "AVL/L do not imply D/B");
    }

    /// PE=1 MOV DS with not-present data descriptor → #NP via a 16-bit IDT gate.
    /// Spec: Intel SDM Vol. 2 MOV; Vol. 3 §§6.11.2, 6.12.1, 6.13.
    #[test]
    fn mov_ds_pe1_not_present_np_via_idt() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        // P=0 data segment (access 0x12)
        let desc = encode_seg_desc(0x1000, 0xFFFF, 0x12, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD8; // MOV DS, AX → #NP
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0x1111);
        let ds_before = cpu.ds.clone();
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 11, 0x0C00);

        step(&mut cpu, &mut bus).unwrap(); // MOV AX
        step(&mut cpu, &mut bus).unwrap(); // MOV DS → #NP
        assert_eq!(cpu.ip16(), 0x0C00, "#NP handler");
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(
            cpu.ds, ds_before,
            "failed MOV DS must not update segment cache"
        );
    }

    /// Descriptor faults carry selector error codes with RPL cleared, TI preserved,
    /// and leave the full destination cache unchanged.
    /// Spec: Intel SDM Vol. 3 §§6.13, 6.15; Vol. 2 MOV Sreg.
    #[test]
    fn data_segment_fault_payloads_mask_selector_and_preserve_state() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        let not_present = encode_seg_desc(0x1000, 0xFFFF, 0x72, 0);
        let execute_only_code = encode_seg_desc(0x2000, 0xFFFF, 0x98, 0);
        mem[gdt + 8..gdt + 16].copy_from_slice(&not_present);
        mem[gdt + 16..gdt + 24].copy_from_slice(&execute_only_code);

        let mut cpu = CpuState::reset();
        cpu.ds = x86_core::SegmentReg {
            selector: 0x0023,
            base: 0x1234_0000,
            limit: 0xABCD,
            flags: 0x0093,
        };
        let ds_before = cpu.ds.clone();
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 23;
        let mut bus = VecBus { mem, ports: vec![] };

        assert_arch_fault(
            load_data_sreg_from_gdt(&mut cpu, &mut bus, 3, 0x000B),
            11,
            Some(0x0008),
        );
        assert_eq!(cpu.ds, ds_before, "#NP must not partially load DS");

        assert_arch_fault(
            load_data_sreg_from_gdt(&mut cpu, &mut bus, 3, 0x0013),
            13,
            Some(0x0010),
        );
        assert_eq!(cpu.ds, ds_before, "type #GP must not partially load DS");

        assert_arch_fault(
            load_data_sreg_from_gdt(&mut cpu, &mut bus, 3, 0x001B),
            13,
            Some(0x0018),
        );
        assert_eq!(cpu.ds, ds_before, "limit #GP must not partially load DS");

        assert_arch_fault(
            load_data_sreg_from_gdt(&mut cpu, &mut bus, 3, 0x000F),
            13,
            Some(0x000C),
        );
        assert_eq!(cpu.ds, ds_before, "LDT #GP must not partially load DS");
    }

    /// SS descriptor faults use #GP(0), #GP(selector), or #SS(selector) as
    /// appropriate and leave the full SS cache unchanged.
    /// Spec: Intel SDM Vol. 3 §§6.13, 6.15; Vol. 2 MOV Sreg.
    #[test]
    fn stack_segment_fault_payloads_mask_selector_and_preserve_state() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        let not_present = encode_seg_desc(0x1000, 0xFFFF, 0x12, 0);
        let read_only = encode_seg_desc(0x2000, 0xFFFF, 0x90, 0);
        mem[gdt + 8..gdt + 16].copy_from_slice(&not_present);
        mem[gdt + 16..gdt + 24].copy_from_slice(&read_only);

        let mut cpu = CpuState::reset();
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0020,
            base: 0x5678_0000,
            limit: 0xCDEF,
            flags: 0x0093,
        };
        let ss_before = cpu.ss.clone();
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 23;
        let mut bus = VecBus { mem, ports: vec![] };

        assert_arch_fault(load_ss_from_gdt(&mut cpu, &mut bus, 0x0003), 13, Some(0));
        assert_eq!(cpu.ss, ss_before, "null #GP must not partially load SS");

        assert_arch_fault(
            load_ss_from_gdt(&mut cpu, &mut bus, 0x0008),
            12,
            Some(0x0008),
        );
        assert_eq!(cpu.ss, ss_before, "#SS must not partially load SS");

        assert_arch_fault(
            load_ss_from_gdt(&mut cpu, &mut bus, 0x0013),
            13,
            Some(0x0010),
        );
        assert_eq!(cpu.ss, ss_before, "type #GP must not partially load SS");

        assert_arch_fault(
            load_ss_from_gdt(&mut cpu, &mut bus, 0x001B),
            13,
            Some(0x0018),
        );
        assert_eq!(cpu.ss, ss_before, "limit #GP must not partially load SS");

        assert_arch_fault(
            load_ss_from_gdt(&mut cpu, &mut bus, 0x000F),
            13,
            Some(0x000C),
        );
        assert_eq!(cpu.ss, ss_before, "LDT #GP must not partially load SS");
    }

    /// Selector payloads become the final word of a protected-mode exception
    /// frame, below faulting IP, CS, and FLAGS.
    /// Spec: Intel SDM Vol. 3 §§6.12.1, 6.13, 6.15; Vol. 2 MOV Sreg.
    #[test]
    fn selector_fault_payload_is_pushed_by_protected_mode_delivery() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        let not_present = encode_seg_desc(0x1000, 0xFFFF, 0x72, 0);
        mem[gdt + 8..gdt + 16].copy_from_slice(&not_present);

        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x0B;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD8;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0x1111);
        let ds_before = cpu.ds.clone();
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let saved_flags = cpu.rflags as u16;
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 11, 0x0C00);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 0x0C00);
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF6);
        assert_eq!(bus.read_u16(0xFFF6).unwrap(), 0x0008);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), (code + 3) as u16);
        assert_eq!(bus.read_u16(0xFFFA).unwrap(), 0);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), saved_flags);
        assert_eq!(cpu.ds, ds_before);
    }

    /// PE=1 null selector into DS clears hidden cache (no #GP). Spec: SDM Vol. 3 §5.4.1.
    #[test]
    fn mov_ds_pe1_null_selector_clears_cache() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // B8 00 00  MOV AX, 0
        // 8E D8     MOV DS, AX
        mem[code] = 0xB8;
        mem[code + 1] = 0x00;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD8;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg {
            selector: 0x08,
            base: 0x0010_0000,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.limit = 0; // empty GDT — null must not consult it
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ds.selector, 0);
        assert_eq!(cpu.ds.base, 0);
        assert_eq!(cpu.ds.limit, 0);
        assert_eq!(cpu.ds.flags, 0);
    }

    /// PE=1 MOV DS with index past GDTR.limit → #GP via a 16-bit IDT gate.
    /// Spec: Intel SDM Vol. 2 MOV; Vol. 3 §§6.11.2, 6.12.1, 6.13.
    #[test]
    fn mov_ds_pe1_gdt_limit_gp_via_idt() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x80;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD8; // selector 0x80 lies beyond gate CS at limit 0x7F
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0x2222);
        let ds_before = cpu.ds.clone();
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = 0x5000;
        cpu.gdtr.limit = 0x7F; // gate CS at index 15; source selector 0x80 is index 16
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 13, 0x0D00);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00, "#GP handler");
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(cpu.ds, ds_before, "failed MOV DS must not update cache");
    }

    /// PE=0 MOV DS unchanged: base = selector<<4, sticky limit/AR.
    /// Spec: Intel SDM Vol. 3 §3.4.2–§3.4.3.
    #[test]
    fn mov_ds_pe0_still_selector_shift4() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x34;
        mem[code + 2] = 0x12;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD8;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0xFFFF_FFFF;
        cpu.ds.flags = 0x0093;
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        assert_eq!(cpu.cr0 & 1, 0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ds.selector, 0x1234);
        assert_eq!(cpu.ds.base, 0x1234u64 << 4);
        assert_eq!(cpu.ds.limit, 0xFFFF_FFFF);
        assert_eq!(cpu.ds.flags, 0x0093);
    }

    /// PE=1 MOV FS loads data-segment descriptor from GDT (same rules as DS/ES).
    /// Spec: Intel SDM Vol. 2 MOV (Sreg, r/m16); Vol. 3 §3.4.5 / §3.5.1.
    #[test]
    fn mov_fs_pe1_loads_gdt_data_descriptor() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        // Selector 0x08: base=0x0022_3300, limit20=0xF_FFFF, G=1
        // → effective limit 0xFFFF_FFFF, cached attributes 0x8092.
        let desc = encode_seg_desc(0x0022_3300, 0xF_FFFF, 0x92, 0x80);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        // B8 08 00  MOV AX, 0x08
        // 8E E0     MOV FS, AX
        // F4        HLT
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xE0;
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.fs = x86_core::SegmentReg::real_mode(0xABCD);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.fs.selector, 0x08);
        assert_eq!(cpu.fs.base, 0x0022_3300);
        assert_eq!(cpu.fs.limit, 0xFFFF_FFFF, "G=1 expands limit");
        assert_eq!(cpu.fs.flags, 0x8092);
        assert_eq!(cpu.ip16(), (code + 5) as u16);
    }

    /// PE=1 MOV GS loads from GDT (same data-segment path as DS/ES/FS).
    /// Spec: Intel SDM Vol. 2 MOV (Sreg, r/m16); Vol. 3 §3.5.1.
    #[test]
    fn mov_gs_pe1_loads_gdt_data_descriptor() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        let desc = encode_seg_desc(0x0000_5000, 0x2FFF, 0x93, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        // B8 08 00  MOV AX, 0x08
        // 8E E8     MOV GS, AX
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xE8;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.gs = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gs.selector, 0x08);
        assert_eq!(cpu.gs.base, 0x0000_5000);
        assert_eq!(cpu.gs.limit, 0x2FFF);
        assert_eq!(cpu.gs.flags, 0x0093);
    }

    /// PE=1 MOV FS, r/m16 memory form loads GDT descriptor.
    /// Spec: Intel SDM Vol. 2 MOV (Sreg, r/m16).
    #[test]
    fn mov_fs_pe1_loads_gdt_from_mem() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        let desc = encode_seg_desc(0x0000_6000, 0x0FFF, 0x92, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);
        // Selector word at DS:BX = 0:0x2000
        mem[0x2000] = 0x08;
        mem[0x2001] = 0x00;

        let code = 0x1000usize;
        // BB 00 20  MOV BX, 0x2000
        // 8E 27     MOV FS, [BX]
        mem[code] = 0xBB;
        mem[code + 1] = 0x00;
        mem[code + 2] = 0x20;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0x27;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.fs = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.fs.selector, 0x08);
        assert_eq!(cpu.fs.base, 0x0000_6000);
        assert_eq!(cpu.fs.limit, 0x0FFF);
        assert_eq!(cpu.fs.flags, 0x0092);
    }

    /// PE=1 MOV FS with not-present data descriptor → #NP via a 16-bit IDT gate.
    /// Spec: Intel SDM Vol. 2 MOV; Vol. 3 §§6.11.2, 6.12.1, 6.13.
    #[test]
    fn mov_fs_pe1_not_present_np_via_idt() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        let desc = encode_seg_desc(0x1000, 0xFFFF, 0x12, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xE0; // MOV FS, AX → #NP
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.fs = x86_core::SegmentReg::real_mode(0x1111);
        let fs_before = cpu.fs.clone();
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 11, 0x0C00);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0C00, "#NP handler");
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(
            cpu.fs, fs_before,
            "failed MOV FS must not update segment cache"
        );
    }

    /// PE=1 null selector into GS clears hidden cache (no #GP). Spec: SDM Vol. 3 §5.4.1.
    #[test]
    fn mov_gs_pe1_null_selector_clears_cache() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // B8 00 00  MOV AX, 0
        // 8E E8     MOV GS, AX
        mem[code] = 0xB8;
        mem[code + 1] = 0x00;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xE8;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.gs = x86_core::SegmentReg {
            selector: 0x08,
            base: 0x0010_0000,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.limit = 0;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gs.selector, 0);
        assert_eq!(cpu.gs.base, 0);
        assert_eq!(cpu.gs.limit, 0);
        assert_eq!(cpu.gs.flags, 0);
    }

    /// PE=1 MOV GS with index past GDTR.limit → #GP via a 16-bit IDT gate.
    /// Spec: Intel SDM Vol. 2 MOV; Vol. 3 §§6.11.2, 6.12.1, 6.13.
    #[test]
    fn mov_gs_pe1_gdt_limit_gp_via_idt() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x80;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xE8; // selector 0x80 lies beyond gate CS at limit 0x7F
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.gs = x86_core::SegmentReg::real_mode(0x2222);
        let gs_before = cpu.gs.clone();
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = 0x5000;
        cpu.gdtr.limit = 0x7F; // gate CS at index 15; source selector 0x80 is index 16
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 13, 0x0D00);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00, "#GP handler");
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(cpu.gs, gs_before, "failed MOV GS must not update cache");
    }

    /// PE=1 MOV FS with code-segment descriptor → #GP via a 16-bit IDT gate.
    /// Spec: Intel SDM Vol. 2 MOV; Vol. 3 §§6.11.2, 6.12.1, 6.13.
    #[test]
    fn mov_fs_pe1_execute_only_code_descriptor_gp_via_idt() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        // Execute-only code segment (access 0x98) — not valid for FS load.
        let desc = encode_seg_desc(0x1000, 0xFFFF, 0x98, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xE0;
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.fs = x86_core::SegmentReg::real_mode(0x3333);
        let fs_before = cpu.fs.clone();
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 13, 0x0D00);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00, "#GP handler");
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(cpu.fs, fs_before, "failed MOV FS must not update cache");
    }

    /// PE=0 MOV FS unchanged: base = selector<<4, sticky limit/AR.
    /// Spec: Intel SDM Vol. 3 §3.4.2–§3.4.3.
    #[test]
    fn mov_fs_pe0_still_selector_shift4() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x34;
        mem[code + 2] = 0x12;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xE0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.fs = x86_core::SegmentReg::real_mode(0);
        cpu.fs.limit = 0xFFFF_FFFF;
        cpu.fs.flags = 0x0093;
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        assert_eq!(cpu.cr0 & 1, 0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.fs.selector, 0x1234);
        assert_eq!(cpu.fs.base, 0x1234u64 << 4);
        assert_eq!(cpu.fs.limit, 0xFFFF_FFFF);
        assert_eq!(cpu.fs.flags, 0x0093);
    }

    /// PE=1 MOV SS loads writable data-segment descriptor from GDT.
    /// Spec: Intel SDM Vol. 2 MOV (SS, r/m16); Vol. 3 §3.4.5 / §3.5.1.
    #[test]
    fn mov_ss_pe1_loads_gdt_writable_data_descriptor() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        // Selector 0x08: writable data, base=0x0003_0000, limit=0x7FFF,
        // access=0x92, B=1 (32-bit stack-pointer width).
        let desc = encode_seg_desc(0x0003_0000, 0x7FFF, 0x92, 0x40);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        // B8 08 00  MOV AX, 0x08
        // 8E D0     MOV SS, AX
        // F4        HLT
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD0;
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0xABCD);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ss.selector, 0x08);
        assert_eq!(cpu.ss.base, 0x0003_0000);
        assert_eq!(cpu.ss.limit, 0x7FFF);
        assert_eq!(cpu.ss.flags, 0x4092);
        assert_eq!(cpu.ss.stack_width(), 32);
        assert_eq!(cpu.ip16(), (code + 5) as u16);
    }

    /// PE=1 MOV SS accepts expand-down writable stack segment (type 6).
    /// Spec: Intel SDM Vol. 2 MOV — SS writable data or expand-down data.
    #[test]
    fn mov_ss_pe1_loads_expand_down_stack_descriptor() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        // Access 0x96: P=1 S=1 type=6 (expand-down RW)
        let desc = encode_seg_desc(0x0000_8000, 0x1000, 0x96, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ss.selector, 0x08);
        assert_eq!(cpu.ss.base, 0x0000_8000);
        assert_eq!(cpu.ss.limit, 0x1000);
        assert_eq!(cpu.ss.flags, 0x0096);
    }

    /// PE=1 MOV SS, r/m16 memory form loads GDT writable descriptor.
    /// Spec: Intel SDM Vol. 2 MOV (SS, r/m16).
    #[test]
    fn mov_ss_pe1_loads_gdt_from_mem() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        let desc = encode_seg_desc(0x0000_9000, 0x0FFF, 0x93, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);
        mem[0x2000] = 0x08;
        mem[0x2001] = 0x00;

        let code = 0x1000usize;
        // BB 00 20  MOV BX, 0x2000
        // 8E 17     MOV SS, [BX]
        mem[code] = 0xBB;
        mem[code + 1] = 0x00;
        mem[code + 2] = 0x20;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0x17;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ss.selector, 0x08);
        assert_eq!(cpu.ss.base, 0x0000_9000);
        assert_eq!(cpu.ss.limit, 0x0FFF);
        assert_eq!(cpu.ss.flags, 0x0093);
    }

    /// PE=1 null selector into SS → #GP via a 16-bit IDT gate.
    /// Spec: Intel SDM Vol. 3 §§5.4.1, 6.11.2, 6.12.1, 6.13.
    #[test]
    fn mov_ss_pe1_null_selector_gp_via_idt() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // B8 00 00  MOV AX, 0
        // 8E D0     MOV SS, AX → #GP
        mem[code] = 0xB8;
        mem[code + 1] = 0x00;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD0;
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        // Keep SS base=0 so IVT #GP push stays inside the 64KiB test image.
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0010,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        let ss_before = cpu.ss.clone();
        cpu.cr0 |= 1;
        cpu.gdtr.limit = 0;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 13, 0x0D00);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00, "#GP handler");
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(cpu.ss, ss_before, "failed MOV SS must not update cache");
    }

    /// PE=1 MOV SS with not-present writable data → #SS via a 16-bit IDT gate.
    /// Spec: Intel SDM Vol. 2 MOV; Vol. 3 §§6.11.2, 6.12.1, 6.13.
    #[test]
    fn mov_ss_pe1_not_present_ss_via_idt() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        // P=0 writable data (access 0x12)
        let desc = encode_seg_desc(0x1000, 0xFFFF, 0x12, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD0; // MOV SS, AX → #SS
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0020,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        let ss_before = cpu.ss.clone();
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 12, 0x0C00);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0C00, "#SS handler");
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(
            cpu.ss, ss_before,
            "failed MOV SS must not update segment cache"
        );
    }

    /// PE=1 MOV SS with read-only data descriptor → #GP via a 16-bit IDT gate.
    /// Spec: Intel SDM Vol. 2 MOV; Vol. 3 §§6.11.2, 6.12.1, 6.13.
    #[test]
    fn mov_ss_pe1_readonly_data_gp_via_idt() {
        let mut mem = vec![0u8; 0x10000];
        let gdt = 0x5000usize;
        // Access 0x90: P=1 S=1 type=0 (data RO) — not valid for SS
        let desc = encode_seg_desc(0x1000, 0xFFFF, 0x90, 0x00);
        mem[gdt + 8..gdt + 16].copy_from_slice(&desc);

        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x08;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD0;
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0030,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        let ss_before = cpu.ss.clone();
        cpu.cr0 |= 1;
        cpu.gdtr.base = gdt as u64;
        cpu.gdtr.limit = 15;
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 13, 0x0D00);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00, "#GP handler");
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(cpu.ss, ss_before, "failed MOV SS must not update cache");
    }

    /// PE=1 MOV SS with index past GDTR.limit → #GP via a 16-bit IDT gate.
    /// Spec: Intel SDM Vol. 2 MOV; Vol. 3 §§6.11.2, 6.12.1, 6.13.
    #[test]
    fn mov_ss_pe1_gdt_limit_gp_via_idt() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x80;
        mem[code + 2] = 0x00;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD0;
        mem[code + 5] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0040,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        let ss_before = cpu.ss.clone();
        cpu.cr0 |= 1;
        cpu.gdtr.base = 0x5000;
        cpu.gdtr.limit = 0x7F; // gate CS at index 15; source selector 0x80 is index 16
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        install_protected_test_exception_gate(&mut bus.mem, &mut cpu, 13, 0x0D00);

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00, "#GP handler");
        assert_eq!(cpu.cs.selector, PROTECTED_COMPAT_TARGET_CS);
        assert_eq!(cpu.ss, ss_before, "failed MOV SS must not update cache");
    }

    /// PE=0 MOV SS unchanged: base = selector<<4, sticky limit/AR.
    /// Spec: Intel SDM Vol. 3 §3.4.2–§3.4.3.
    #[test]
    fn mov_ss_pe0_still_selector_shift4() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        mem[code] = 0xB8;
        mem[code + 1] = 0x34;
        mem[code + 2] = 0x12;
        mem[code + 3] = 0x8E;
        mem[code + 4] = 0xD0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ss.limit = 0xFFFF_FFFF;
        cpu.ss.flags = 0x0093;
        assert_eq!(cpu.cr0 & 1, 0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ss.selector, 0x1234);
        assert_eq!(cpu.ss.base, 0x1234u64 << 4);
        assert_eq!(cpu.ss.limit, 0xFFFF_FFFF);
        assert_eq!(cpu.ss.flags, 0x0093);
    }

    /// MOV to/from CR1 is architecturally undefined — #UD via the real-mode IVT.
    /// Spec: Intel SDM Vol. 2 "MOV—Move to/from Control Registers"
    /// ("Attempts to reference CR1 ... result in undefined opcode (#UD)").
    #[test]
    fn mov_cr1_is_ud_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        // IVT #UD vector 6 → handler at 0x0B00.
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        let code = 0x1000usize;
        // +0: 0F 20 C8   MOV EAX, CR1  (reg=1) → #UD
        // +3: F4         HLT (unreached)
        mem[code] = 0x0F;
        mem[code + 1] = 0x20;
        mem[code + 2] = 0xC8; // 11_001_000: reg=1 (CR1), rm=0 (EAX)
        mem[code + 3] = 0xF4;
        mem[0xB00] = 0xF4; // handler HLT

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x0B00);
    }

    /// MOV to/from CR2/CR3/CR4 are implemented as of round 4; they were
    /// `Unsupported` while the paging engine was unwired.
    /// Spec: Intel SDM Vol. 2 "MOV—Move to/from Control Registers"; Vol. 3
    /// §2.5, §4.7 (CR2), Table 4-3 (CR3), §4.1.4 (CR4).
    /// Behavioral coverage lives in `tests/cpu_r4_control_registers.rs`.
    #[test]
    fn mov_cr2_cr3_cr4_are_implemented() {
        let mut mem = vec![0u8; 0x10000];
        let code = 0x1000usize;
        // +0: 0F 20 D0   MOV EAX, CR2  (reg=2)
        // +3: 0F 22 D8   MOV CR3, EAX  (reg=3)
        // +6: 0F 20 E0   MOV EAX, CR4  (reg=4)
        mem[code] = 0x0F;
        mem[code + 1] = 0x20;
        mem[code + 2] = 0xD0;
        mem[code + 3] = 0x0F;
        mem[code + 4] = 0x22;
        mem[code + 5] = 0xD8;
        mem[code + 6] = 0x0F;
        mem[code + 7] = 0x20;
        mem[code + 8] = 0xE0;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.cr2 = 0x1234_5678;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x1234_5678, "EAX ← CR2");
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cr3, 0x1234_5678, "CR3 ← EAX");
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0, "EAX ← CR4 (still zero)");
    }

    /// LIDT/SIDT m16&32 — opcode 0F 01 /3 and /1 (SDM Vol. 2 LIDT/SIDT; Vol. 3 §2.4.3).
    /// Mirrors LGDT/SGDT opsize and mod=11 #UD rules for IDTR.
    #[test]
    fn lidt_sidt_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // IVT #UD vector 6 → handler at 0x0B00
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        let code = 0x1000usize;
        // +0: 0F 01 1E 00 40    LIDT [0x4000]  (opsize 16 → 24-bit base)
        // +5: 0F 01 0E 00 50    SIDT [0x5000]
        // +A: 66 0F 01 1E 00 60 LIDT [0x6000] (opsize 32 → 32-bit base)
        // +10: 66 0F 01 0E 00 70 SIDT [0x7000]
        // +16: 0F 01 C9         SIDT ECX (mod=11, /1) → #UD
        // +19: F4               HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0x1E;
        mem[code + 3] = 0x00;
        mem[code + 4] = 0x40;
        mem[code + 5] = 0x0F;
        mem[code + 6] = 0x01;
        mem[code + 7] = 0x0E;
        mem[code + 8] = 0x00;
        mem[code + 9] = 0x50;
        mem[code + 10] = 0x66;
        mem[code + 11] = 0x0F;
        mem[code + 12] = 0x01;
        mem[code + 13] = 0x1E;
        mem[code + 14] = 0x00;
        mem[code + 15] = 0x60;
        mem[code + 16] = 0x66;
        mem[code + 17] = 0x0F;
        mem[code + 18] = 0x01;
        mem[code + 19] = 0x0E;
        mem[code + 20] = 0x00;
        mem[code + 21] = 0x70;
        mem[code + 22] = 0x0F;
        mem[code + 23] = 0x01;
        mem[code + 24] = 0xC9; // mod=11, reg=1 (SIDT r/m) — #UD
        mem[code + 25] = 0xF4;

        // Pseudo-descriptor at 0x4000: limit=0x03FF, base=0x12ABCDEF (high byte ignored)
        mem[0x4000] = 0xFF;
        mem[0x4001] = 0x03;
        mem[0x4002] = 0xEF;
        mem[0x4003] = 0xCD;
        mem[0x4004] = 0xAB;
        mem[0x4005] = 0x12;

        // Pseudo-descriptor at 0x6000: limit=0x07FF, base=0xCAFEBABE
        mem[0x6000] = 0xFF;
        mem[0x6001] = 0x07;
        mem[0x6002] = 0xBE;
        mem[0x6003] = 0xBA;
        mem[0x6004] = 0xFE;
        mem[0x6005] = 0xCA;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.idtr.limit, 0x03FF);
        assert_eq!(
            cpu.idtr.base, 0x00AB_CDEF,
            "16-bit opsize truncates base to 24 bits"
        );

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x5000).unwrap(), 0x03FF);
        assert_eq!(bus.read_u32(0x5002).unwrap(), 0x00AB_CDEF);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.idtr.limit, 0x07FF);
        assert_eq!(cpu.idtr.base, 0xCAFE_BABE);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x7000).unwrap(), 0x07FF);
        assert_eq!(bus.read_u32(0x7002).unwrap(), 0xCAFE_BABE);

        // Register form → #UD via IVT (IDTR still at 0xCAFEBABE would miss;
        // restore IVT base first so delivery uses low memory).
        cpu.idtr.base = 0;
        cpu.idtr.limit = 0x03FF;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(cpu.cs.selector, 0);
    }

    /// INVLPG m — opcode 0F 01 /7 (SDM Vol. 2 "INVLPG—Invalidate TLB Entries").
    /// Real-address mode: memory form is an architectural NOP (TLB-less; no paging).
    /// Register form (mod=11) → #UD via IVT. Does not modify GPRs or CR0 / enable PE/PM.
    #[test]
    fn invlpg_real_mode_nop_and_reg_ud() {
        let mut mem = vec![0u8; 0x10000];
        // IVT #UD vector 6 → handler at 0x0B00
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        let code = 0x1000usize;
        // +0: 0F 01 3E 00 40    INVLPG [0x4000]  (memory form → NOP)
        // +5: 0F 01 F8         INVLPG EAX        (mod=11 → #UD)
        // +8: F4               HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0x3E;
        mem[code + 3] = 0x00;
        mem[code + 4] = 0x40;
        mem[code + 5] = 0x0F;
        mem[code + 6] = 0x01;
        mem[code + 7] = 0xF8; // mod=11, reg=7, rm=EAX
        mem[code + 8] = 0xF4;
        // Sentinel at operand address — INVLPG must not read or write it.
        mem[0x4000] = 0xA5;
        mem[0x4001] = 0x5A;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_gpr_u32(CpuState::RAX, 0x1122_3344);
        cpu.set_gpr_u32(CpuState::RBX, 0x5566_7788);
        let cr0_before = cpu.cr0;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // INVLPG [0x4000]
        assert_eq!(cpu.rip, (code + 5) as u64, "memory INVLPG advances IP");
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x1122_3344, "GPRs unchanged");
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x5566_7788, "GPRs unchanged");
        assert_eq!(cpu.cr0, cr0_before, "CR0 unchanged (no PE/PM side effects)");
        assert_eq!(bus.mem[0x4000], 0xA5, "operand memory not accessed");
        assert_eq!(bus.mem[0x4001], 0x5A, "operand memory not accessed");

        // Register form → #UD via IVT
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.cr0, cr0_before, "CR0 still unchanged after #UD path");
    }

    /// LGDT/SGDT m16&32 — opcode 0F 01 /2 and /0 (SDM Vol. 2 LGDT/SGDT; Vol. 3 §2.4.1).
    /// Real-mode opsize-16 uses 24-bit base; 0x66 uses full 32-bit base. mod=11 → #UD.
    #[test]
    fn lgdt_sgdt_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // IVT #UD vector 6 → handler at 0x0B00 (keep code out of IVT bytes 0..0x400)
        mem[6 * 4] = 0x00;
        mem[6 * 4 + 1] = 0x0B;
        mem[6 * 4 + 2] = 0x00;
        mem[6 * 4 + 3] = 0x00;
        let code = 0x1000usize;
        // +0: 0F 01 16 00 40    LGDT [0x4000]  (opsize 16 → 24-bit base)
        // +5: 0F 01 06 00 50    SGDT [0x5000]
        // +A: 66 0F 01 16 00 60 LGDT [0x6000] (opsize 32 → 32-bit base)
        // +10: 66 0F 01 06 00 70 SGDT [0x7000]
        // +16: 0F 01 C0         SGDT EAX (mod=11) → #UD
        // +19: F4               HLT
        mem[code] = 0x0F;
        mem[code + 1] = 0x01;
        mem[code + 2] = 0x16;
        mem[code + 3] = 0x00;
        mem[code + 4] = 0x40;
        mem[code + 5] = 0x0F;
        mem[code + 6] = 0x01;
        mem[code + 7] = 0x06;
        mem[code + 8] = 0x00;
        mem[code + 9] = 0x50;
        mem[code + 10] = 0x66;
        mem[code + 11] = 0x0F;
        mem[code + 12] = 0x01;
        mem[code + 13] = 0x16;
        mem[code + 14] = 0x00;
        mem[code + 15] = 0x60;
        mem[code + 16] = 0x66;
        mem[code + 17] = 0x0F;
        mem[code + 18] = 0x01;
        mem[code + 19] = 0x06;
        mem[code + 20] = 0x00;
        mem[code + 21] = 0x70;
        mem[code + 22] = 0x0F;
        mem[code + 23] = 0x01;
        mem[code + 24] = 0xC0; // mod=11, reg=0 (SGDT r/m) — #UD
        mem[code + 25] = 0xF4;

        // Pseudo-descriptor at 0x4000: limit=0x0027, base=0x12ABCDEF (high byte ignored)
        mem[0x4000] = 0x27;
        mem[0x4001] = 0x00;
        mem[0x4002] = 0xEF;
        mem[0x4003] = 0xCD;
        mem[0x4004] = 0xAB;
        mem[0x4005] = 0x12;

        // Pseudo-descriptor at 0x6000: limit=0xFFFF, base=0xDEADBEEF
        mem[0x6000] = 0xFF;
        mem[0x6001] = 0xFF;
        mem[0x6002] = 0xEF;
        mem[0x6003] = 0xBE;
        mem[0x6004] = 0xAD;
        mem[0x6005] = 0xDE;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = code as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gdtr.limit, 0x0027);
        assert_eq!(
            cpu.gdtr.base, 0x00AB_CDEF,
            "16-bit opsize truncates base to 24 bits"
        );

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x5000).unwrap(), 0x0027);
        assert_eq!(bus.read_u32(0x5002).unwrap(), 0x00AB_CDEF);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gdtr.limit, 0xFFFF);
        assert_eq!(cpu.gdtr.base, 0xDEAD_BEEF);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x7000).unwrap(), 0xFFFF);
        assert_eq!(bus.read_u32(0x7002).unwrap(), 0xDEAD_BEEF);

        // Register form → #UD via IVT
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(cpu.cs.selector, 0);
    }

    /// Operand-size override 0x66: MOV/PUSH/POP/ALU 32-bit in real mode.
    /// Spec: Intel SDM Vol. 2 Ch. 2 (66H); Vol. 1 §3.6; instruction pages MOV/PUSH/POP/ADD.
    /// Segment model remains real-mode (selector<<4); without 0x66 stays 16-bit.
    #[test]
    fn opsize32_mov_push_pop_alu_real_mode() {
        let mut mem = vec![0u8; 0x10000];
        // 66 B8 78 56 34 12  = MOV EAX, 0x12345678
        mem[0] = 0x66;
        mem[1] = 0xB8;
        mem[2] = 0x78;
        mem[3] = 0x56;
        mem[4] = 0x34;
        mem[5] = 0x12;
        // 66 BB 01 00 00 00  = MOV EBX, 1
        mem[6] = 0x66;
        mem[7] = 0xBB;
        mem[8] = 0x01;
        mem[9] = 0x00;
        mem[10] = 0x00;
        mem[11] = 0x00;
        // 66 01 D8          = ADD EAX, EBX
        mem[12] = 0x66;
        mem[13] = 0x01;
        mem[14] = 0xD8;
        // 66 50             = PUSH EAX
        mem[15] = 0x66;
        mem[16] = 0x50;
        // 66 5A             = POP EDX
        mem[17] = 0x66;
        mem[18] = 0x5A;
        // 66 3D 79 56 34 12 = CMP EAX, 0x12345679
        mem[19] = 0x66;
        mem[20] = 0x3D;
        mem[21] = 0x79;
        mem[22] = 0x56;
        mem[23] = 0x34;
        mem[24] = 0x12;
        // B8 CD AB          = MOV AX, 0xABCD (no 0x66 → 16-bit)
        mem[25] = 0xB8;
        mem[26] = 0xCD;
        mem[27] = 0xAB;
        mem[28] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        // Prove real-mode segment base = selector<<4 (unchanged by opsize).
        assert_eq!(cpu.ds.base, 0);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // MOV EAX
        assert_eq!(cpu.eax(), 0x1234_5678);
        step(&mut cpu, &mut bus).unwrap(); // MOV EBX
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 1);
        step(&mut cpu, &mut bus).unwrap(); // ADD EAX, EBX
        assert_eq!(cpu.eax(), 0x1234_5679);
        assert_eq!(cpu.rflags & 1, 0); // CF clear
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF clear

        step(&mut cpu, &mut bus).unwrap(); // PUSH EAX
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u32(0xFFFA).unwrap(), 0x1234_5679);

        step(&mut cpu, &mut bus).unwrap(); // POP EDX
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0x1234_5679);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        step(&mut cpu, &mut bus).unwrap(); // CMP EAX, imm32
        assert_eq!(cpu.eax(), 0x1234_5679); // unchanged
        assert_ne!(cpu.rflags & (1 << 6), 0); // ZF

        step(&mut cpu, &mut bus).unwrap(); // MOV AX,imm16 without 0x66
        assert_eq!(cpu.ax(), 0xABCD);
        // set_gpr_u16 preserves bits 31:16 of EAX.
        assert_eq!(cpu.eax(), 0x1234_ABCD);
        assert_eq!(cpu.ds.base, 0); // still real-mode flat DS
    }

    /// 0x66 ALU memory form + near CALL/RET with opsize 32.
    /// Spec: Intel SDM Vol. 2 ADD/XOR; "CALL"/"RET" near; Ch. 2 (66H).
    #[test]
    fn opsize32_alu_mem_and_near_call_ret() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x4000] = 0x10;
        mem[0x4001] = 0x00;
        mem[0x4002] = 0x00;
        mem[0x4003] = 0x00;

        // 66 81 06 00 40 EF BE AD DE = ADD dword [0x4000], 0xDEADBEEF
        mem[0] = 0x66;
        mem[1] = 0x81;
        mem[2] = 0x06;
        mem[3] = 0x00;
        mem[4] = 0x40;
        mem[5] = 0xEF;
        mem[6] = 0xBE;
        mem[7] = 0xAD;
        mem[8] = 0xDE;
        // 66 31 C0 = XOR EAX, EAX
        mem[9] = 0x66;
        mem[10] = 0x31;
        mem[11] = 0xC0;
        // 66 E8 08 00 00 00 = CALL rel32; next=18, target=26 (RET)
        mem[12] = 0x66;
        mem[13] = 0xE8;
        mem[14] = 0x08;
        mem[15] = 0x00;
        mem[16] = 0x00;
        mem[17] = 0x00;
        // return site: 66 05 01 00 00 00 = ADD EAX, 1
        mem[18] = 0x66;
        mem[19] = 0x05;
        mem[20] = 0x01;
        mem[21] = 0x00;
        mem[22] = 0x00;
        mem[23] = 0x00;
        mem[24] = 0xF4; // HLT
                        // subroutine: 66 C3 = RET
        mem[26] = 0x66;
        mem[27] = 0xC3;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // ADD [mem], imm32
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0xDEAD_BEFF);
        assert_eq!(cpu.rflags & 1, 0);

        step(&mut cpu, &mut bus).unwrap(); // XOR EAX,EAX
        assert_eq!(cpu.eax(), 0);
        assert_ne!(cpu.rflags & (1 << 6), 0);

        step(&mut cpu, &mut bus).unwrap(); // CALL → RET at 26
        assert_eq!(cpu.ip16(), 26);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
        assert_eq!(bus.read_u32(0xFFFA).unwrap(), 18);

        step(&mut cpu, &mut bus).unwrap(); // RET → 18
        assert_eq!(cpu.ip16(), 18);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        step(&mut cpu, &mut bus).unwrap(); // ADD EAX, 1
        assert_eq!(cpu.eax(), 1);
    }

    /// 0x66 tranche-3: INC/DEC r32, XCHG EAX,r32, CWDE/CDQ, TEST EAX,imm32.
    /// Spec: Intel SDM Vol. 2 INC/DEC/XCHG/CBW/CWDE/CWD/CDQ/TEST; Ch. 2 (66H).
    #[test]
    fn opsize32_inc_dec_xchg_cwde_cdq_test_eax() {
        let mut mem = vec![0u8; 0x10000];
        // 66 40 = INC EAX
        mem[0] = 0x66;
        mem[1] = 0x40;
        // 66 48 = DEC EAX
        mem[2] = 0x66;
        mem[3] = 0x48;
        // 66 FF C3 = INC EBX (Group5 /0 r32)
        mem[4] = 0x66;
        mem[5] = 0xFF;
        mem[6] = 0xC3;
        // 66 FF CB = DEC EBX (Group5 /1 r32)
        mem[7] = 0x66;
        mem[8] = 0xFF;
        mem[9] = 0xCB;
        // 66 93 = XCHG EAX, EBX
        mem[10] = 0x66;
        mem[11] = 0x93;
        // 66 98 = CWDE
        mem[12] = 0x66;
        mem[13] = 0x98;
        // 66 99 = CDQ
        mem[14] = 0x66;
        mem[15] = 0x99;
        // 66 A9 EF BE AD DE = TEST EAX, 0xDEADBEEF
        mem[16] = 0x66;
        mem[17] = 0xA9;
        mem[18] = 0xEF;
        mem[19] = 0xBE;
        mem[20] = 0xAD;
        mem[21] = 0xDE;
        mem[22] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_eax(0x0FFF_FFFF);
        cpu.set_gpr_u32(CpuState::RBX, 0x10);
        cpu.set_cf(true); // INC/DEC must preserve CF
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // INC EAX
        assert_eq!(cpu.eax(), 0x1000_0000);
        assert!(cpu.rflags & 1 != 0); // CF preserved
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF clear
        assert_eq!(cpu.rflags & (1 << 7), 0); // SF clear

        step(&mut cpu, &mut bus).unwrap(); // DEC EAX
        assert_eq!(cpu.eax(), 0x0FFF_FFFF);
        assert!(cpu.rflags & 1 != 0);

        step(&mut cpu, &mut bus).unwrap(); // INC EBX
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x11);
        step(&mut cpu, &mut bus).unwrap(); // DEC EBX
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x10);

        step(&mut cpu, &mut bus).unwrap(); // XCHG EAX, EBX
        assert_eq!(cpu.eax(), 0x10);
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x0FFF_FFFF);

        // AX = 0x8000 → CWDE → EAX = 0xFFFF_8000
        cpu.set_eax(0x0000_8000);
        step(&mut cpu, &mut bus).unwrap(); // CWDE
        assert_eq!(cpu.eax(), 0xFFFF_8000);

        step(&mut cpu, &mut bus).unwrap(); // CDQ
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0xFFFF_FFFF);
        assert_eq!(cpu.eax(), 0xFFFF_8000);

        // TEST EAX, 0xDEADBEEF → EAX & imm = 0xDEAD_8000; SF=1 ZF=0
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0xFFFF_8000); // unchanged
        assert_eq!(cpu.rflags & 1, 0); // CF cleared
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF cleared
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
    }

    /// 0x66 LES/LDS r32,m16:32 — Spec: Intel SDM Vol. 2 LES/LDS; Ch. 2 (66H).
    #[test]
    fn opsize32_les_lds_r32() {
        let mut mem = vec![0u8; 0x10000];
        // Far ptr32 at 0x2000: offset 0x12345678, selector 0x1000
        mem[0x2000] = 0x78;
        mem[0x2001] = 0x56;
        mem[0x2002] = 0x34;
        mem[0x2003] = 0x12;
        mem[0x2004] = 0x00;
        mem[0x2005] = 0x10;
        // Far ptr32 at 0x3000: offset 0xABCDEF01, selector 0xF000
        mem[0x3000] = 0x01;
        mem[0x3001] = 0xEF;
        mem[0x3002] = 0xCD;
        mem[0x3003] = 0xAB;
        mem[0x3004] = 0x00;
        mem[0x3005] = 0xF0;
        // 66 C4 06 00 20 = LES EAX, [0x2000]
        mem[0] = 0x66;
        mem[1] = 0xC4;
        mem[2] = 0x06;
        mem[3] = 0x00;
        mem[4] = 0x20;
        // 66 C5 1E 00 30 = LDS EBX, [0x3000]
        mem[5] = 0x66;
        mem[6] = 0xC5;
        mem[7] = 0x1E;
        mem[8] = 0x00;
        mem[9] = 0x30;
        mem[10] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0x9999);
        cpu.rip = 0;
        cpu.rflags = 0x246;
        let flags_before = cpu.rflags;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x1234_5678);
        assert_eq!(cpu.es.selector, 0x1000);
        assert_eq!(cpu.es.base, 0x1000u64 << 4);
        assert_eq!(cpu.rflags, flags_before);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0xABCD_EF01);
        assert_eq!(cpu.ds.selector, 0xF000);
        assert_eq!(cpu.ds.base, 0xF000u64 << 4);
        assert_eq!(cpu.rflags, flags_before);
    }

    /// 0x66 BOUND r32,m32&32 — Spec: Intel SDM Vol. 2 BOUND; Vol. 3 §6.15 (#BR).
    #[test]
    fn opsize32_bound_r32() {
        let mut mem = vec![0u8; 0x10000];
        // Bounds at 0x2000: lower=0x10, upper=0x20
        mem[0x2000] = 0x10;
        mem[0x2001] = 0x00;
        mem[0x2002] = 0x00;
        mem[0x2003] = 0x00;
        mem[0x2004] = 0x20;
        mem[0x2005] = 0x00;
        mem[0x2006] = 0x00;
        mem[0x2007] = 0x00;
        // IVT[5] → 0000:0B00
        mem[20] = 0x00;
        mem[21] = 0x0B;
        mem[22] = 0x00;
        mem[23] = 0x00;
        // 66 62 06 00 20 = BOUND EAX, [0x2000]
        mem[0] = 0x66;
        mem[1] = 0x62;
        mem[2] = 0x06;
        mem[3] = 0x00;
        mem[4] = 0x20;
        mem[0xB00] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_eax(0x0000_000F); // below lower → #BR
        cpu.set_interrupt_flag(true);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0B00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP

        // Inclusive endpoints succeed
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_eax(0x10);
        cpu.halted = false;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 5);
        cpu.rip = 0;
        cpu.set_eax(0x20);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 5);
    }

    /// 0x66 far CALL/JMP/RETF ptr16:32 and Group5 m16:32.
    /// Spec: Intel SDM Vol. 2 CALL/JMP/RET; Ch. 2 (66H). Real-mode OsZ32 → 6-byte frame.
    #[test]
    fn opsize32_far_call_jmp_retf_ptr16_32() {
        let mut mem = vec![0u8; 0x20000];
        // Far pointer memory at DS:0x4000 → 0x1000:0x0200
        mem[0x4000] = 0x00;
        mem[0x4001] = 0x02;
        mem[0x4002] = 0x00;
        mem[0x4003] = 0x00;
        mem[0x4004] = 0x00;
        mem[0x4005] = 0x10;
        // Target at 0x1000:0x0200 = linear 0x10200: 66 CB RETF
        let target = (0x1000u32 << 4) + 0x0200;
        mem[target as usize] = 0x66;
        mem[target as usize + 1] = 0xCB;
        // Landing: HLT
        mem[0x20] = 0xF4;

        // 66 9A 00 02 00 00 00 10 = CALL FAR 1000:00000200
        mem[0] = 0x66;
        mem[1] = 0x9A;
        mem[2] = 0x00;
        mem[3] = 0x02;
        mem[4] = 0x00;
        mem[5] = 0x00;
        mem[6] = 0x00;
        mem[7] = 0x10;
        // After RETF lands here (IP=8): NOP pad then JMP FAR mem
        // 66 FF 2E 00 40 = JMP FAR dword [0x4000]
        mem[8] = 0x66;
        mem[9] = 0xFF;
        mem[10] = 0x2E;
        mem[11] = 0x00;
        mem[12] = 0x40;
        // After second RETF would be HLT at 0x20 — rewrite target after first return
        // Also exercise Group5 CALL FAR: place at 0x30
        // 66 FF 1E 00 40 = CALL FAR [0x4000]
        mem[0x30] = 0x66;
        mem[0x31] = 0xFF;
        mem[0x32] = 0x1E;
        mem[0x33] = 0x00;
        mem[0x34] = 0x40;
        mem[0x35] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // CALL FAR ptr16:32
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.ip16(), 0x0200);
        // 6-byte frame: EIP32 then CS16 above it on stack growth down
        // SP was FFFE; push CS (−2→FFFC), push EIP (−4→FFF8)
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u32(0xFFF8).unwrap(), 8); // return EIP
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0); // saved CS

        step(&mut cpu, &mut bus).unwrap(); // RETF opsize32
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 8);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        // JMP FAR m16:32 to same target (no stack) — overwrite RETF with HLT for landing
        bus.mem[target as usize] = 0xF4;
        step(&mut cpu, &mut bus).unwrap(); // JMP FAR [0x4000]
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE); // unchanged

        // Group5 CALL FAR m16:32
        bus.mem[target as usize] = 0x66;
        bus.mem[target as usize + 1] = 0xCB;
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0x30;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.ip16(), 0x0200);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
        assert_eq!(bus.read_u32(0xFFF8).unwrap(), 0x35); // next after CALL
        step(&mut cpu, &mut bus).unwrap(); // RETF
        assert_eq!(cpu.cs.selector, 0);
        assert_eq!(cpu.ip16(), 0x35);
    }

    /// 0x66 tranche-4: MOV moffs EAX (A1/A3), POP r/m32 (8F), MOV r32←Sreg (8C).
    /// Spec: Intel SDM Vol. 2 MOV/POP; Ch. 2 (66H); Vol. 1 §3.6.
    #[test]
    fn opsize32_moffs_eax_pop_rm32_mov_sreg_r32() {
        let mut mem = vec![0u8; 0x10000];
        // moffs dword at DS:0x3000
        mem[0x3000] = 0x78;
        mem[0x3001] = 0x56;
        mem[0x3002] = 0x34;
        mem[0x3003] = 0x12;
        // 66 A1 00 30 = MOV EAX, moffs16 0x3000
        mem[0] = 0x66;
        mem[1] = 0xA1;
        mem[2] = 0x00;
        mem[3] = 0x30;
        // 66 A3 00 40 = MOV moffs16 0x4000, EAX
        mem[4] = 0x66;
        mem[5] = 0xA3;
        mem[6] = 0x00;
        mem[7] = 0x40;
        // 66 8C D8 = MOV EAX, DS (zero-extend selector)
        mem[8] = 0x66;
        mem[9] = 0x8C;
        mem[10] = 0xD8;
        // 66 8C 06 00 50 = MOV [0x5000], ES — memory dest still 16-bit store
        mem[11] = 0x66;
        mem[12] = 0x8C;
        mem[13] = 0x06;
        mem[14] = 0x00;
        mem[15] = 0x50;
        // 66 8F C3 = POP EBX
        mem[16] = 0x66;
        mem[17] = 0x8F;
        mem[18] = 0xC3;
        // 66 8F 06 00 60 = POP dword [0x6000]
        mem[19] = 0x66;
        mem[20] = 0x8F;
        mem[21] = 0x06;
        mem[22] = 0x00;
        mem[23] = 0x60;
        mem[24] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0xABCD);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFA);
        // Stack: dword 0x11111111 at SP=0xFFFA; dword 0x22222222 at SP=0xFFF6
        mem[0xFFFA] = 0x11;
        mem[0xFFFB] = 0x11;
        mem[0xFFFC] = 0x11;
        mem[0xFFFD] = 0x11;
        mem[0xFFF6] = 0x22;
        mem[0xFFF7] = 0x22;
        mem[0xFFF8] = 0x22;
        mem[0xFFF9] = 0x22;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap(); // MOV EAX, moffs
        assert_eq!(cpu.eax(), 0x1234_5678);

        step(&mut cpu, &mut bus).unwrap(); // MOV moffs, EAX
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0x1234_5678);

        cpu.set_eax(0xDEAD_BEEF);
        cpu.ds = x86_core::SegmentReg::real_mode(0x1234);
        step(&mut cpu, &mut bus).unwrap(); // MOV EAX, DS
        assert_eq!(cpu.eax(), 0x0000_1234);

        // Poison high word of memory so 16-bit store is observable.
        bus.mem[0x5000] = 0xFF;
        bus.mem[0x5001] = 0xFF;
        bus.mem[0x5002] = 0xEE;
        bus.mem[0x5003] = 0xEE;
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        step(&mut cpu, &mut bus).unwrap(); // MOV [0x5000], ES
        assert_eq!(bus.read_u16(0x5000).unwrap(), 0xABCD);
        assert_eq!(bus.mem[0x5002], 0xEE); // upper bytes untouched
        assert_eq!(bus.mem[0x5003], 0xEE);

        step(&mut cpu, &mut bus).unwrap(); // POP EBX
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x1111_1111);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);

        cpu.set_gpr_u16(CpuState::RSP, 0xFFF6);
        step(&mut cpu, &mut bus).unwrap(); // POP dword [0x6000]
        assert_eq!(bus.read_u32(0x6000).unwrap(), 0x2222_2222);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFA);
    }

    /// 0x66 Group 2 D1/C1 and Group 3 F7 dword forms.
    /// Spec: Intel SDM Vol. 2 ROL/ROR/RCL/RCR/SHL/SHR/SAR; TEST/NOT/NEG/MUL/IMUL/DIV/IDIV; Ch. 2.
    #[test]
    fn opsize32_grp2_d1_c1_and_grp3_f7() {
        let mut mem = vec![0u8; 0x10000];
        // 66 D1 E0       = SHL EAX, 1
        mem[0] = 0x66;
        mem[1] = 0xD1;
        mem[2] = 0xE0;
        // 66 C1 E8 04    = SHR EAX, 4
        mem[3] = 0x66;
        mem[4] = 0xC1;
        mem[5] = 0xE8;
        mem[6] = 0x04;
        // 66 D1 C0       = ROL EAX, 1
        mem[7] = 0x66;
        mem[8] = 0xD1;
        mem[9] = 0xC0;
        // 66 F7 D0       = NOT EAX
        mem[10] = 0x66;
        mem[11] = 0xF7;
        mem[12] = 0xD0;
        // 66 F7 D8       = NEG EAX
        mem[13] = 0x66;
        mem[14] = 0xF7;
        mem[15] = 0xD8;
        // 66 F7 C0 EF BE AD DE = TEST EAX, 0xDEADBEEF
        mem[16] = 0x66;
        mem[17] = 0xF7;
        mem[18] = 0xC0;
        mem[19] = 0xEF;
        mem[20] = 0xBE;
        mem[21] = 0xAD;
        mem[22] = 0xDE;
        // 66 F7 E3       = MUL EBX
        mem[23] = 0x66;
        mem[24] = 0xF7;
        mem[25] = 0xE3;
        // 66 F7 EB       = IMUL EBX
        mem[26] = 0x66;
        mem[27] = 0xF7;
        mem[28] = 0xEB;
        // 66 F7 F3       = DIV EBX
        mem[29] = 0x66;
        mem[30] = 0xF7;
        mem[31] = 0xF3;
        // 66 F7 FB       = IDIV EBX
        mem[32] = 0x66;
        mem[33] = 0xF7;
        mem[34] = 0xFB;
        // 66 F7 06 00 40 = NOT dword [0x4000]
        mem[35] = 0x66;
        mem[36] = 0xF7;
        mem[37] = 0x16;
        mem[38] = 0x00;
        mem[39] = 0x40; // /2 NOT mem — ModRM 0x16 = mod=00 reg=2 rm=6 → [disp16]
        mem[40] = 0xF4;
        mem[0x4000] = 0x0F;
        mem[0x4001] = 0x0F;
        mem[0x4002] = 0x0F;
        mem[0x4003] = 0x0F;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        cpu.set_eax(0x4000_0000);
        step(&mut cpu, &mut bus).unwrap(); // SHL EAX,1
        assert_eq!(cpu.eax(), 0x8000_0000);
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 11), 0); // OF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        step(&mut cpu, &mut bus).unwrap(); // SHR EAX,4
        assert_eq!(cpu.eax(), 0x0800_0000);

        cpu.set_eax(0x8000_0000);
        step(&mut cpu, &mut bus).unwrap(); // ROL EAX,1
        assert_eq!(cpu.eax(), 0x0000_0001);
        assert_ne!(cpu.rflags & 1, 0); // CF=1

        cpu.set_eax(0x0F0F_0F0F);
        let flags_before = cpu.rflags;
        step(&mut cpu, &mut bus).unwrap(); // NOT EAX
        assert_eq!(cpu.eax(), 0xF0F0_F0F0);
        assert_eq!(cpu.rflags, flags_before);

        cpu.set_eax(1);
        step(&mut cpu, &mut bus).unwrap(); // NEG EAX
        assert_eq!(cpu.eax(), 0xFFFF_FFFF);
        assert_ne!(cpu.rflags & 1, 0); // CF
        assert_ne!(cpu.rflags & (1 << 7), 0); // SF

        // Imm high half must participate: 0x12345678 & 0xFFFF0000 = 0x12340000 (ZF clear).
        // A mistaken imm16 decode (0x0000) would yield ZF set — catch length too (IP += 7).
        cpu.set_eax(0x1234_5678);
        let ip_before_test = cpu.ip16();
        step(&mut cpu, &mut bus).unwrap(); // TEST EAX, 0xDEADBEEF
        assert_eq!(cpu.ip16(), ip_before_test + 7);
        assert_eq!(cpu.eax(), 0x1234_5678); // unchanged
        assert_eq!(cpu.rflags & 1, 0); // CF
        assert_eq!(cpu.rflags & (1 << 11), 0); // OF
                                               // Result 0x12341668: ZF clear, SF clear.
        assert_eq!(cpu.rflags & (1 << 6), 0); // ZF
        assert_eq!(cpu.rflags & (1 << 7), 0); // SF

        // MUL EBX: EAX=2, EBX=3 → EDX:EAX = 0:6; CF=OF=0
        cpu.set_eax(2);
        cpu.set_gpr_u32(CpuState::RBX, 3);
        cpu.set_gpr_u32(CpuState::RDX, 0xFFFF_FFFF);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 6);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX: EAX=-2, EBX=-3 → 6; fits in i32 → CF=OF=0
        cpu.set_eax(0xFFFF_FFFE);
        cpu.set_gpr_u32(CpuState::RBX, 0xFFFF_FFFD);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 6);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // DIV EBX: EDX:EAX = 0:100 / 7 → quot=14 rem=2
        cpu.set_eax(100);
        cpu.set_gpr_u32(CpuState::RDX, 0);
        cpu.set_gpr_u32(CpuState::RBX, 7);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 14);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 2);

        // IDIV EBX: EDX:EAX = -20 / 3 → quot=-6 rem=-2
        cpu.set_eax((-20i32) as u32);
        cpu.set_gpr_u32(CpuState::RDX, 0xFFFF_FFFF); // sign-extend
        cpu.set_gpr_u32(CpuState::RBX, 3);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), (-6i32) as u32);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), (-2i32) as u32);

        step(&mut cpu, &mut bus).unwrap(); // NOT dword [0x4000]
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0xF0F0_F0F0);
    }

    /// 0x66 Group 2 D3 r/m32,CL and IMUL 69/6B r32,r/m32,imm.
    /// Spec: Intel SDM Vol. 2 SHL/IMUL; Ch. 2 (66H).
    #[test]
    fn opsize32_grp2_d3_cl_and_imul_69_6b() {
        let mut mem = vec![0u8; 0x10000];
        // 66 D3 E0                   = SHL EAX, CL
        mem[0] = 0x66;
        mem[1] = 0xD3;
        mem[2] = 0xE0;
        // 66 69 D8 02 00 00 00       = IMUL EBX, EAX, 2
        mem[3] = 0x66;
        mem[4] = 0x69;
        mem[5] = 0xD8;
        mem[6] = 0x02;
        mem[7] = 0x00;
        mem[8] = 0x00;
        mem[9] = 0x00;
        // 66 69 D8 00 00 01 00       = IMUL EBX, EAX, 0x00010000
        mem[10] = 0x66;
        mem[11] = 0x69;
        mem[12] = 0xD8;
        mem[13] = 0x00;
        mem[14] = 0x00;
        mem[15] = 0x01;
        mem[16] = 0x00;
        // 66 6B D8 FD                = IMUL EBX, EAX, -3
        mem[17] = 0x66;
        mem[18] = 0x6B;
        mem[19] = 0xD8;
        mem[20] = 0xFD;
        // 66 69 1E 00 40 03 00 00 00 = IMUL EBX, [0x4000], 3
        mem[21] = 0x66;
        mem[22] = 0x69;
        mem[23] = 0x1E;
        mem[24] = 0x00;
        mem[25] = 0x40;
        mem[26] = 0x03;
        mem[27] = 0x00;
        mem[28] = 0x00;
        mem[29] = 0x00;
        mem[30] = 0xF4;
        mem[0x4000] = 0x05;
        mem[0x4001] = 0x00;
        mem[0x4002] = 0x00;
        mem[0x4003] = 0x00;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        // SHL EAX, CL: 0x4000_0000 << 1 = 0x8000_0000; CF=0, OF=1
        cpu.set_eax(0x4000_0000);
        cpu.set_gpr_u8_low(CpuState::RCX, 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x8000_0000);
        assert_eq!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX, EAX, 2: 3*2=6 fits → CF=OF=0; EAX unchanged
        cpu.set_eax(3);
        cpu.set_gpr_u32(CpuState::RBX, 0xDEAD_BEEF);
        let ip_before = cpu.ip16();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), ip_before + 7); // 66 + 69 + modrm + imm32
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 6);
        assert_eq!(cpu.eax(), 3);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX, EAX, 0x10000: 0x10000*0x10000 = 0x1_0000_0000 does not fit in i32
        cpu.set_eax(0x0001_0000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX, EAX, -3: (-2)*(-3)=6 fits
        cpu.set_eax(0xFFFF_FFFE);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 6);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);

        // IMUL EBX, [0x4000], 3: 5*3=15; memory unchanged
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 15);
        assert_eq!(bus.read_u32(0x4000).unwrap(), 5);
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(cpu.rflags & (1 << 11), 0);
    }

    /// Real-mode 0x67: 32-bit ModRM effective addresses (selector<<4 + EA32).
    /// Spec: Intel SDM Vol. 1 §3.6; Vol. 2 Chapter 2 (address-size attribute).
    #[test]
    fn asize32_modrm_ea_mov_and_lea() {
        let mut mem = vec![0u8; 0x20000];
        // 67 8B 03 = MOV AX, [EBX]
        mem[0] = 0x67;
        mem[1] = 0x8B;
        mem[2] = 0x03;
        // 67 8D 4B 10 = LEA CX, [EBX+0x10]
        mem[3] = 0x67;
        mem[4] = 0x8D;
        mem[5] = 0x4B;
        mem[6] = 0x10;
        // 67 8B 44 24 04 = MOV AX, [ESP+4]
        mem[7] = 0x67;
        mem[8] = 0x8B;
        mem[9] = 0x44;
        mem[10] = 0x24;
        mem[11] = 0x04;
        mem[12] = 0xF4;

        // DS:EBX → linear 0x1000; payload 0xBEEF
        mem[0x1000] = 0xEF;
        mem[0x1001] = 0xBE;
        // SS:ESP+4 → linear 0x3004; payload 0xCAFE
        mem[0x3004] = 0xFE;
        mem[0x3005] = 0xCA;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u32(CpuState::RBX, 0x1000);
        cpu.set_gpr_u32(CpuState::RSP, 0x3000);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0xBEEF);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x1010);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0xCAFE);
    }

    /// Absolute disp32 under 0x67 uses DS:(disp32), not EBP.
    /// Spec: Intel SDM Vol. 2 Chapter 2 — mod=00 rm=101 → disp32.
    #[test]
    fn asize32_modrm_disp32_absolute() {
        let mut mem = vec![0u8; 0x20000];
        // 67 8A 05 00 40 00 00 = MOV AL, [0x4000]
        mem[0] = 0x67;
        mem[1] = 0x8A;
        mem[2] = 0x05;
        mem[3] = 0x00;
        mem[4] = 0x40;
        mem[5] = 0x00;
        mem[6] = 0x00;
        mem[7] = 0xF4;
        mem[0x4000] = 0x5A;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u32(CpuState::RBP, 0xFFFF_FFFF); // must not participate
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x5A);
    }

    /// String ops with 0x67 use ESI/EDI (and ECX for REP), not SI/DI/CX.
    /// Spec: Intel SDM Vol. 1 §3.6; Vol. 2 MOVS / REP (address-size attribute).
    #[test]
    fn asize32_movsb_uses_esi_edi_and_rep_ecx() {
        let mut mem = vec![0u8; 0x20000];
        // F3 67 A4 = REP MOVSB
        mem[0] = 0xF3;
        mem[1] = 0x67;
        mem[2] = 0xA4;
        mem[3] = 0xF4;
        mem[0x5000] = 0x11;
        mem[0x5001] = 0x22;
        mem[0x5002] = 0x33;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        // High halves must participate under asize32 (would be ignored with SI/DI/CX).
        cpu.set_gpr_u32(CpuState::RSI, 0x0000_5000);
        cpu.set_gpr_u32(CpuState::RDI, 0x0000_6000);
        cpu.set_gpr_u32(CpuState::RCX, 0x0000_0003);
        cpu.set_direction_flag(false);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x6000).unwrap(), 0x11);
        assert_eq!(bus.read_u8(0x6001).unwrap(), 0x22);
        assert_eq!(bus.read_u8(0x6002).unwrap(), 0x33);
        assert_eq!(cpu.gpr_u32(CpuState::RSI), 0x5003);
        assert_eq!(cpu.gpr_u32(CpuState::RDI), 0x6003);
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0);
    }

    /// Default real-mode DS limit 64KiB: accesses within 0..=FFFF succeed; 16-bit EA wrap.
    /// Spec: SDM Vol. 3 §3.4.2–§3.4.3, §5.3; docs/cpu-profile-core2.md.
    #[test]
    fn real_mode_default_segment_limit_unchanged() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x1234] = 0xAB;
        mem[0x2000] = 0x5A;
        // A0 34 12 = MOV AL, [0x1234]
        mem[0] = 0xA0;
        mem[1] = 0x34;
        mem[2] = 0x12;
        // 8A 87 FE 1F = MOV AL, [BX+0x1FFE] with BX=2 → EA 0x2000 (16-bit wrap add)
        mem[3] = 0x8A;
        mem[4] = 0x87;
        mem[5] = 0xFE;
        mem[6] = 0x1F;
        mem[7] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        assert_eq!(cpu.ds.limit, 0xFFFF);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RBX, 2);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xAB);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x5A);
        assert_eq!(cpu.ds.limit, 0xFFFF);
    }

    /// A `moffs` offset follows the *effective address-size attribute*, so
    /// under `CS.D=1` it is 32 bits with no prefix and 16 bits with `0x67` —
    /// the inverse of the `D=0` case. Keying on the prefix instead truncated
    /// every 32-bit `moffs` reference to its low word, which sent SeaBIOS's
    /// POST writes to `CS.base + offset16` in the ROM window.
    /// Spec: Intel SDM Vol. 2 "MOV" (moffs8/moffs16/moffs32); Vol. 1 §3.6.
    #[test]
    fn moffs_offset_width_follows_the_address_size_attribute() {
        let mut mem = vec![0u8; 0x3_0000];
        mem[0x2_1234] = 0x5A;
        mem[0x1234] = 0xA5;
        // A0 34 12 02 00      MOV AL, moffs32 0x21234   (D=1, no prefix)
        // 67 A0 34 12         MOV AL, moffs16 0x1234    (D=1, 0x67 → 16-bit)
        mem[0..5].copy_from_slice(&[0xA0, 0x34, 0x12, 0x02, 0x00]);
        mem[5..9].copy_from_slice(&[0x67, 0xA0, 0x34, 0x12]);

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg {
            selector: 0x08,
            base: 0,
            limit: 0xFFFF_FFFF,
            flags: 0xC09B,
        };
        cpu.ds = x86_core::SegmentReg {
            selector: 0x10,
            base: 0,
            limit: 0xFFFF_FFFF,
            flags: 0xC093,
        };
        cpu.ss = cpu.ds.clone();
        cpu.cr0 = 1;
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x5A, "no prefix under D=1 is a 32-bit offset");
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0xA5, "0x67 under D=1 is a 16-bit offset");
    }

    /// Expanded DS limit (unreal): moffs32 beyond 64KiB succeeds; beyond limit → #GP via IVT.
    /// Spec: SDM Vol. 3 §3.4.3 (cached limit), §5.3, §6.15 (#GP); Vol. 2 MOV moffs.
    #[test]
    fn unreal_expanded_ds_limit_moffs32_and_gp() {
        // --- success path: limit=4GiB-1, read [0x10000] ---
        {
            let mut mem = vec![0u8; 0x20000];
            mem[0x10000] = 0xC3;
            // 67 A0 00 00 01 00 = MOV AL, moffs32 0x10000
            mem[0] = 0x67;
            mem[1] = 0xA0;
            mem[2] = 0x00;
            mem[3] = 0x00;
            mem[4] = 0x01;
            mem[5] = 0x00;
            mem[6] = 0xF4;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.ds.limit = 0xFFFF_FFFF;
            cpu.rip = 0;
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.al(), 0xC3);
            assert_eq!(cpu.ip16(), 6);
        }

        // --- #GP when offset past cached limit (still >64KiB) ---
        {
            let mut mem = vec![0u8; 0x20000];
            // IVT[13] → 0000:0D00
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            mem[0xD00] = 0xF4;
            // 67 A0 00 80 01 00 = MOV AL, [0x18000]
            mem[0] = 0x67;
            mem[1] = 0xA0;
            mem[2] = 0x00;
            mem[3] = 0x80;
            mem[4] = 0x01;
            mem[5] = 0x00;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.ds.limit = 0x1_7FFF; // allows 0x10000, not 0x18000
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            cpu.set_interrupt_flag(true);
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert!(!cpu.interrupt_flag());
            assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // faulting IP
        }
    }

    /// Real-mode MOV DS keeps expanded cached limit (sticky unreal descriptor cache).
    /// Spec: SDM Vol. 3 §3.4.2–§3.4.3.
    #[test]
    fn unreal_mov_ds_preserves_expanded_limit() {
        let mut mem = vec![0u8; 0x10000];
        // B8 34 12 = MOV AX, 0x1234; 8E D8 = MOV DS, AX
        mem[0] = 0xB8;
        mem[1] = 0x34;
        mem[2] = 0x12;
        mem[3] = 0x8E;
        mem[4] = 0xD8;
        mem[5] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0xFFFF_FFFF;
        cpu.ds.flags = 0x0093;
        cpu.rip = 0;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ds.selector, 0x1234);
        assert_eq!(cpu.ds.base, 0x1234u64 << 4);
        assert_eq!(cpu.ds.limit, 0xFFFF_FFFF);
        assert_eq!(cpu.ds.flags, 0x0093);
    }

    /// Reduced limit with 16-bit ModRM EA → #GP via IVT (no asize32 required).
    /// Spec: SDM Vol. 3 §5.3, §6.15 (#GP); Vol. 2 MOV.
    #[test]
    fn segment_limit_gp_modrm_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        mem[13 * 4] = 0x00;
        mem[13 * 4 + 1] = 0x0D;
        mem[13 * 4 + 2] = 0x00;
        mem[13 * 4 + 3] = 0x00;
        mem[0xD00] = 0xF4;
        // 8A 87 00 90 = MOV AL, [BX+0x9000] with BX=0
        mem[0] = 0x8A;
        mem[1] = 0x87;
        mem[2] = 0x00;
        mem[3] = 0x90;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0x7FFF;
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RBX, 0);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
    }

    /// Address-size 0x67: LOOP/LOOPcc use ECX; JECXZ tests ECX (SDM Vol. 2 LOOP / JCXZ).
    /// High half of ECX participates (asize16 would only see CX=0 when ECX=0x10000).
    #[test]
    fn asize32_loop_jecxz_uses_ecx() {
        let mut mem = vec![0u8; 0x10000];
        // 0: 67 E2 FD = LOOP $-3 (self; 3-byte insn with 0x67)
        // 3: 67 E3 02 = JECXZ +2
        // 6: F4 F4 F4
        mem[0] = 0x67;
        mem[1] = 0xE2;
        mem[2] = 0xFD;
        mem[3] = 0x67;
        mem[4] = 0xE3;
        mem[5] = 0x02;
        mem[6] = 0xF4;
        mem[7] = 0xF4;
        mem[8] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        // ECX=0x10000 → after dec 0xFFFF ≠ 0 → take; CX alone would already be 0.
        cpu.set_gpr_u32(CpuState::RCX, 0x1_0000);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0xFFFF);
        assert_eq!(cpu.ip16(), 0);

        // Fall-through when ECX becomes 0 (short path; high-half case covered above).
        cpu.set_gpr_u32(CpuState::RCX, 1);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0);
        assert_eq!(cpu.ip16(), 3);

        // JECXZ: ECX=0 takes
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 8);
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0);

        // JECXZ: ECX=0x10000 (CX=0) must NOT take under asize32
        cpu.rip = 3;
        cpu.set_gpr_u32(CpuState::RCX, 0x1_0000);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 6);
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0x1_0000);
    }

    /// Address-size 0x67: XLAT uses EBX+AL (SDM Vol. 2 XLAT/XLATB; Vol. 1 §3.6).
    #[test]
    fn asize32_xlat_uses_ebx() {
        let mut mem = vec![0u8; 0x20000];
        mem[0x10005] = 0x5A;
        // 67 D7 = XLAT (asize32)
        mem[0] = 0x67;
        mem[1] = 0xD7;
        mem[2] = 0xF4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0xFFFF_FFFF;
        cpu.rip = 0;
        // BX=0 would miss; EBX high half required.
        cpu.set_gpr_u32(CpuState::RBX, 0x1_0000);
        cpu.set_al(0x05);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x5A);
        assert_eq!(cpu.ip16(), 2);
    }

    /// String ops enforce cached SegmentReg.limit before bus access (parity with ModRM).
    /// Spec: SDM Vol. 3 §5.3 / §6.15; Vol. 2 MOVS.
    #[test]
    fn string_op_segment_limit_gp_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        mem[13 * 4] = 0x00;
        mem[13 * 4 + 1] = 0x0D;
        mem[13 * 4 + 2] = 0x00;
        mem[13 * 4 + 3] = 0x00;
        mem[0xD00] = 0xF4;
        // A4 = MOVSB; SI=0x9000 past DS.limit=0x7FFF
        mem[0] = 0xA4;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ds.limit = 0x7FFF;
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSI, 0x9000);
        cpu.set_gpr_u16(CpuState::RDI, 0x1000);
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00);
        // Faulting IP; indices must not advance on limit fault.
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0);
        assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x9000);
        assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x1000);
    }

    /// STOSB ES limit → #GP; SS override on LODSB source → #SS.
    /// Spec: SDM Vol. 3 §5.3, §6.15 (#GP/#SS); Vol. 2 STOS/LODS.
    #[test]
    fn string_op_es_limit_gp_and_ss_override_ss() {
        // STOSB past ES.limit → #GP
        {
            let mut mem = vec![0u8; 0x10000];
            mem[13 * 4] = 0x00;
            mem[13 * 4 + 1] = 0x0D;
            mem[13 * 4 + 2] = 0x00;
            mem[13 * 4 + 3] = 0x00;
            mem[0xD00] = 0xF4;
            mem[0] = 0xAA; // STOSB
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.es = x86_core::SegmentReg::real_mode(0);
            cpu.es.limit = 0x0FFF;
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RDI, 0x2000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00);
            assert_eq!(cpu.gpr_u16(CpuState::RDI), 0x2000);
        }
        // LODSB with SS override past SS.limit → #SS
        {
            let mut mem = vec![0u8; 0x10000];
            mem[12 * 4] = 0x00;
            mem[12 * 4 + 1] = 0x0C;
            mem[12 * 4 + 2] = 0x00;
            mem[12 * 4 + 3] = 0x00;
            mem[0xC00] = 0xF4;
            // 36 AC = LODSB SS:
            mem[0] = 0x36;
            mem[1] = 0xAC;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.ss.limit = 0x0FFF;
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSI, 0x2000);
            cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0C00);
            assert_eq!(cpu.gpr_u16(CpuState::RSI), 0x2000);
        }
    }

    /// CS instruction-fetch past cached limit → #GP via IVT.
    /// Spec: SDM Vol. 3 §5.3, §6.15 (#GP); Vol. 1 §3.3.4 (CS:IP fetch).
    #[test]
    fn cs_fetch_limit_gp_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        mem[13 * 4] = 0x00;
        mem[13 * 4 + 1] = 0x0D;
        mem[13 * 4 + 2] = 0x00;
        mem[13 * 4 + 3] = 0x00;
        mem[0xD00] = 0xF4;
        mem[0x2000] = 0xF4; // would be HLT if fetch succeeded
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.cs.limit = 0x1FFF;
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0x2000;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0x2000);
    }

    /// Non-REP instruction: external IRQ when IF=1 is serviced before fetch/execute.
    /// Spec: Intel SDM Vol. 3 §6.8.1 — saved IP is the interrupted instruction.
    #[test]
    fn non_rep_external_irq_before_instruction() {
        let mut mem = vec![0u8; 0x10000];
        mem[0x20 * 4] = 0x00;
        mem[0x20 * 4 + 1] = 0x0E;
        mem[0x20 * 4 + 2] = 0x00;
        mem[0x20 * 4 + 3] = 0x00;
        mem[0] = 0x90; // NOP — must not execute
        mem[0xE00] = 0xF4;
        mem[0x1000] = 0x00; // sentinel; NOP must not touch memory

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        cpu.request_interrupt(0x20);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 0x0E00);
        assert!(!cpu.interrupt_flag());
        assert_eq!(cpu.pending_irq, None);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // saved IP = NOP
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF8);
    }

    /// IF=0: pending IRQ stays latched; non-REP instruction runs normally.
    #[test]
    fn non_rep_external_irq_ignored_when_if_clear() {
        let mut mem = vec![0u8; 0x10000];
        mem[0] = 0x90; // NOP
        mem[1] = 0xF4;

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(false);
        cpu.request_interrupt(0x20);

        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 1);
        assert_eq!(cpu.pending_irq, Some(0x20));
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFFE);
    }

    /// Code-fetch bus MemoryFault → #GP via IVT (same classify as CS limit fault).
    /// Spec: Intel SDM Vol. 3 §6.15 (#GP); Vol. 1 §3.3.4 (instruction fetch).
    #[test]
    fn code_fetch_memory_fault_gp_via_ivt() {
        let mut mem = vec![0u8; 0x10000];
        mem[13 * 4] = 0x00;
        mem[13 * 4 + 1] = 0x0D;
        mem[13 * 4 + 2] = 0x00;
        mem[13 * 4 + 3] = 0x00;
        mem[0] = 0x90; // NOP at poisoned fetch address
        mem[0xD00] = 0xF4;
        let poison = 0u64; // CS.base=0, IP=0 → linear 0
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_interrupt_flag(true);
        let mut bus = PoisonBus {
            mem,
            poison,
            tripped: false,
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x0D00);
        assert_eq!(bus.read_u16(0xFFF8).unwrap(), 0); // fault IP
        assert!(!cpu.interrupt_flag());
    }

    /// The `0x67` address-size override applies to memory operands, not to the
    /// stack address size, so ENTER on a `B=0` stack still uses SP/BP.
    /// Spec: Intel SDM Vol. 2 "ENTER"; Vol. 1 §§3.6, 6.2.2; Vol. 3 §3.4.5.1.
    #[test]
    fn enter_ignores_address_size_override_on_16_bit_stack() {
        let mut mem = vec![0u8; 0x10000];
        // 67 C8 08 00 00 = ENTER 8, 0 with asize32
        mem[0] = 0x67;
        mem[1] = 0xC8;
        mem[2] = 0x08;
        mem[3] = 0x00;
        mem[4] = 0x00;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_gpr_u32(CpuState::RBP, 0x1111_ABCD);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 5);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0xABCD);
        assert_eq!(cpu.gpr_u32(CpuState::RBP), 0x1111_FFFC);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF4);
    }

    /// The `0x67` address-size override does not change the stack address
    /// size, so PUSHA on a `B=0` stack still steps SP.
    /// Spec: Intel SDM Vol. 2 "PUSHA/PUSHAD"; Vol. 1 §6.2.2; Vol. 3 §3.4.5.1.
    #[test]
    fn pusha_ignores_address_size_override_on_16_bit_stack() {
        let mut mem = vec![0u8; 0x10000];
        // 67 60 = PUSHA with asize32
        mem[0] = 0x67;
        mem[1] = 0x60;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFFE);
        cpu.set_gpr_u16(CpuState::RAX, 0x1234);
        let mut bus = VecBus { mem, ports: vec![] };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 2);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFEE);
        assert_eq!(bus.read_u16(0xFFFC).unwrap(), 0x1234);
        assert_eq!(bus.read_u16(0xFFF4).unwrap(), 0xFFFE); // Temp = SP
    }

    const POP_SEG_STACK_BASE: usize = 0x2000;
    const POP_SEG_STACK_SP: u16 = 0x0100;
    const POP_SEG_SELECTOR: u16 = 0x0020;

    fn protected_pop_segment_fixture(
        opcode: u8,
        selector: u16,
        descriptor: Option<[u8; 8]>,
        cpl: u8,
    ) -> (CpuState, VecBus) {
        let mut mem = vec![0u8; 0x10000];
        mem[PROTECTED_TEST_CODE] = opcode;
        let descriptor_offset = usize::from(selector >> 3) * 8;
        if let Some(descriptor) = descriptor {
            let descriptor_addr = PROTECTED_TEST_GDT + descriptor_offset;
            mem[descriptor_addr..descriptor_addr + 8].copy_from_slice(&descriptor);
        }
        let stack_addr = POP_SEG_STACK_BASE + usize::from(POP_SEG_STACK_SP);
        mem[stack_addr..stack_addr + 2].copy_from_slice(&selector.to_le_bytes());
        // Poison the unbased offset so this also proves that POP reads old SS:SP.
        mem[usize::from(POP_SEG_STACK_SP)..usize::from(POP_SEG_STACK_SP) + 2]
            .copy_from_slice(&0x0000u16.to_le_bytes());

        let code_access = 0x9A | (cpl << 5);
        let data_access = 0x93 | (cpl << 5);
        let mut cpu = CpuState::reset();
        cpu.cr0 |= 1;
        cpu.cs = x86_core::SegmentReg {
            selector: 0x0010 | u16::from(cpl),
            base: 0,
            limit: 0xFFFF,
            flags: u16::from(code_access),
        };
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0018 | u16::from(cpl),
            base: POP_SEG_STACK_BASE as u64,
            limit: 0xFFFF,
            flags: u16::from(data_access),
        };
        cpu.ds = x86_core::SegmentReg {
            selector: 0x0030 | u16::from(cpl),
            base: 0x3333_0000,
            limit: 0x3333,
            flags: u16::from(data_access),
        };
        cpu.es = x86_core::SegmentReg {
            selector: 0x0038 | u16::from(cpl),
            base: 0x4444_0000,
            limit: 0x4444,
            flags: u16::from(data_access),
        };
        cpu.gdtr.base = PROTECTED_TEST_GDT as u64;
        cpu.gdtr.limit = if descriptor.is_some() {
            (descriptor_offset + 7) as u16
        } else {
            0
        };
        cpu.rip = PROTECTED_TEST_CODE as u64;
        cpu.rflags = 0x0AD7;
        cpu.gpr[CpuState::RSP] = 0xA5A5_5A5A_0000_0000 | u64::from(POP_SEG_STACK_SP);
        cpu.gpr[CpuState::RAX] = 0x1111_2222_3333_4444;
        cpu.gpr[CpuState::RBX] = 0x5555_6666_7777_8888;
        (cpu, VecBus { mem, ports: vec![] })
    }

    fn real_interrupt_shadow_fixture(code: &[u8]) -> (CpuState, VecBus) {
        let mut mem = vec![0u8; 0x10000];
        mem[..code.len()].copy_from_slice(code);
        mem[usize::from(VECTOR_NMI) * 4..usize::from(VECTOR_NMI) * 4 + 4]
            .copy_from_slice(&[0x00, 0x0D, 0x00, 0x00]);
        mem[0x20 * 4..0x20 * 4 + 4].copy_from_slice(&[0x00, 0x0E, 0x00, 0x00]);

        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.rflags = 0x0203;
        cpu.set_ax(0);
        cpu.set_gpr_u16(CpuState::RSP, 0x8000);
        (cpu, VecBus { mem, ports: vec![] })
    }

    /// POP ES/DS/SS reads through the old 16-bit SS:SP, validates all descriptor
    /// bytes, then commits the cache and advances SP exactly once.
    ///
    /// Spec: Intel SDM Vol. 2 POP (Protected Mode Exceptions); Vol. 3
    /// §§3.4.3–3.4.5, 5.4.1.
    #[test]
    fn protected_pop_segments_load_gdt_caches_atomically() {
        let cases = [
            ("POP ES data", 0x07, 0, 0, 0x92, 0x10),
            ("POP DS readable code", 0x1F, 3, 0, 0x9A, 0x80),
            ("POP SS writable data", 0x17, 2, 0, 0x93, 0x10),
            ("POP DS ring-3 readable code", 0x1F, 3, 3, 0xFA, 0x00),
        ];

        for (name, opcode, target, cpl, access, gran) in cases {
            let selector = POP_SEG_SELECTOR | u16::from(cpl);
            let base = 0x1234_5000 + u32::from(opcode);
            let raw_limit = 0x1_2345;
            let descriptor = encode_seg_desc(base, raw_limit, access, gran);
            let (mut cpu, mut bus) =
                protected_pop_segment_fixture(opcode, selector, Some(descriptor), cpl);
            let before = cpu.clone();

            step(&mut cpu, &mut bus).unwrap();

            let loaded = match target {
                0 => &cpu.es,
                2 => &cpu.ss,
                3 => &cpu.ds,
                _ => unreachable!(),
            };
            assert_eq!(loaded.selector, selector, "{name}: selector");
            assert_eq!(loaded.base, u64::from(base), "{name}: base");
            let expected_limit = if gran & 0x80 != 0 {
                (raw_limit << 12) | 0xFFF
            } else {
                raw_limit
            };
            assert_eq!(loaded.limit, expected_limit, "{name}: limit");
            assert_eq!(
                loaded.flags,
                u16::from(access) | (u16::from(gran & 0xF0) << 8),
                "{name}: cached attributes"
            );
            assert_eq!(cpu.ip16(), PROTECTED_TEST_CODE as u16 + 1, "{name}");
            assert_eq!(
                cpu.gpr[CpuState::RSP],
                (before.gpr[CpuState::RSP] & !0xFFFF) | u64::from(POP_SEG_STACK_SP + 2),
                "{name}: bounded 16-bit SP advances once"
            );
            assert_eq!(cpu.gpr[CpuState::RAX], before.gpr[CpuState::RAX], "{name}");
            assert_eq!(cpu.gpr[CpuState::RBX], before.gpr[CpuState::RBX], "{name}");
            assert_eq!(cpu.rflags, before.rflags, "{name}: FLAGS");
            if target != 0 {
                assert_eq!(cpu.es, before.es, "{name}: unrelated ES");
            }
            if target != 2 {
                assert_eq!(cpu.ss, before.ss, "{name}: unrelated SS");
            }
            if target != 3 {
                assert_eq!(cpu.ds, before.ds, "{name}: unrelated DS");
            }
        }
    }

    /// Null selectors (index zero, including nonzero RPL) are legal for DS/ES
    /// and use the repository's cleared/unusable cache contract.
    /// Spec: Intel SDM Vol. 2 POP; Vol. 3 §5.4.1.
    #[test]
    fn protected_pop_data_segments_accept_null_selectors() {
        for (name, opcode, selector) in [("POP ES", 0x07, 0x0003), ("POP DS", 0x1F, 0x0000)] {
            let (mut cpu, mut bus) = protected_pop_segment_fixture(opcode, selector, None, 0);
            let old_sp = cpu.gpr_u16(CpuState::RSP);

            step_inner(&mut cpu, &mut bus).unwrap();

            let loaded = if opcode == 0x07 { &cpu.es } else { &cpu.ds };
            assert_eq!(loaded.selector, selector, "{name}");
            assert_eq!(loaded.base, 0, "{name}");
            assert_eq!(loaded.limit, 0, "{name}");
            assert_eq!(loaded.flags, 0, "{name}");
            assert_eq!(cpu.gpr_u16(CpuState::RSP), old_sp + 2, "{name}");
        }
    }

    /// DS/ES accept data or readable code, enforce CPL/RPL versus DPL for data
    /// and nonconforming code, and check type/privilege before presence.
    ///
    /// Spec: Intel SDM Vol. 2 POP (Protected Mode Exceptions); Vol. 3
    /// §§3.4.5, 5.4.1, 5.5, 5.6, 6.13.
    #[test]
    fn protected_pop_data_segment_fault_matrix_is_atomic() {
        let cases = [
            ("system", 0x0020, 0x80, 0, None, 13),
            ("execute-only code", 0x0020, 0x98, 0, None, 13),
            ("RPL above DPL", 0x0023, 0x92, 0, None, 13),
            ("CPL above DPL", 0x0020, 0xD2, 3, None, 13),
            ("not present", 0x0020, 0x12, 0, None, 11),
            ("GDT limit", 0x0020, 0x92, 0, Some(38), 13),
            ("LDT selector", 0x0024, 0x92, 0, None, 13),
        ];

        for (name, selector, access, cpl, gdt_limit, vector) in cases {
            let descriptor = encode_seg_desc(0x1234_0000, 0xFFFF, access, 0);
            let (mut cpu, mut bus) =
                protected_pop_segment_fixture(0x1F, selector, Some(descriptor), cpl);
            if let Some(limit) = gdt_limit {
                cpu.gdtr.limit = limit;
            }
            let before = cpu.clone();

            assert_arch_fault(step_inner(&mut cpu, &mut bus), vector, Some(selector & !3));
            assert_eq!(cpu, before, "{name}: POP DS partially committed");
        }
    }

    /// SS must be non-null writable data with RPL=CPL=DPL. Type and privilege
    /// failures are #GP; only a valid-but-not-present stack descriptor is #SS.
    ///
    /// Spec: Intel SDM Vol. 2 POP (Protected Mode Exceptions); Vol. 3
    /// §§3.4.5, 5.4.1, 5.5, 5.7, 6.13.
    #[test]
    fn protected_pop_ss_fault_matrix_is_atomic() {
        let cases = [
            ("null", 0x0003, 0x93, 0, None, 13, 0),
            ("read-only data", 0x0020, 0x90, 0, None, 13, 0x20),
            ("code", 0x0020, 0x9A, 0, None, 13, 0x20),
            ("RPL differs from CPL", 0x0023, 0x92, 0, None, 13, 0x20),
            ("DPL differs from CPL", 0x0023, 0xD2, 3, None, 13, 0x20),
            ("not present", 0x0020, 0x12, 0, None, 12, 0x20),
            (
                "not-present privilege mismatch",
                0x0023,
                0x12,
                0,
                None,
                13,
                0x20,
            ),
            ("GDT limit", 0x0020, 0x92, 0, Some(38), 13, 0x20),
            ("LDT selector", 0x0024, 0x92, 0, None, 13, 0x24),
        ];

        for (name, selector, access, cpl, gdt_limit, vector, error_code) in cases {
            let descriptor = encode_seg_desc(0x5678_0000, 0xFFFF, access, 0);
            let (mut cpu, mut bus) =
                protected_pop_segment_fixture(0x17, selector, Some(descriptor), cpl);
            if let Some(limit) = gdt_limit {
                cpu.gdtr.limit = limit;
            }
            let before = cpu.clone();

            assert_arch_fault(step_inner(&mut cpu, &mut bus), vector, Some(error_code));
            assert_eq!(cpu, before, "{name}: POP SS partially committed");
        }
    }

    /// Stack limit/read faults and a late descriptor-byte fault leave SP and
    /// the destination cache unchanged.
    ///
    /// Spec: Intel SDM Vol. 2 POP (Protected Mode Exceptions); Vol. 3
    /// §§5.3, 6.13, 6.15.
    #[test]
    fn protected_pop_segment_stack_and_descriptor_faults_are_atomic() {
        let descriptor = encode_seg_desc(0x1234_0000, 0xFFFF, 0x92, 0);

        let (mut cpu, mut bus) =
            protected_pop_segment_fixture(0x1F, POP_SEG_SELECTOR, Some(descriptor), 0);
        cpu.ss.limit = u32::from(POP_SEG_STACK_SP);
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 12, Some(0));
        assert_eq!(cpu, before, "stack-limit #SS changed POP DS state");

        let (mut cpu, fixture) =
            protected_pop_segment_fixture(0x07, POP_SEG_SELECTOR, Some(descriptor), 0);
        let mut bus = FailOnceReadBus {
            mem: fixture.mem,
            fail_addr: POP_SEG_STACK_BASE as u64 + u64::from(POP_SEG_STACK_SP) + 1,
            failed: false,
        };
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 12, Some(0));
        assert!(bus.failed);
        assert_eq!(cpu, before, "stack bus #SS changed POP ES state");

        let (mut cpu, fixture) =
            protected_pop_segment_fixture(0x1F, POP_SEG_SELECTOR, Some(descriptor), 0);
        let mut bus = FailOnceReadBus {
            mem: fixture.mem,
            fail_addr: cpu.gdtr.base + u64::from(POP_SEG_SELECTOR >> 3) * 8 + 7,
            failed: false,
        };
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert!(bus.failed, "the final descriptor byte must be read");
        assert_eq!(cpu, before, "descriptor bus #GP changed POP DS state");
    }

    /// MOV SS inhibits a pending maskable IRQ through the immediately following
    /// HLT. The next boundary recognizes the IRQ and wakes the halted CPU.
    ///
    /// Spec: Intel SDM Vol. 2 MOV/HLT; Vol. 3 §6.8.3.
    #[test]
    fn mov_ss_shadow_delays_irq_across_hlt_in_real_mode() {
        let (mut cpu, mut bus) = real_interrupt_shadow_fixture(&[0x8E, 0xD0, 0xF4]);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 2);
        assert_eq!(cpu.ss.selector, 0);
        cpu.request_interrupt(0x20);

        step(&mut cpu, &mut bus).unwrap();
        assert!(cpu.halted, "HLT immediately after MOV SS must execute");
        assert_eq!(cpu.ip16(), 3);
        assert_eq!(cpu.pending_irq, Some(0x20));

        step(&mut cpu, &mut bus).unwrap();
        assert!(!cpu.halted, "delayed IRQ must wake HLT");
        assert_eq!(cpu.ip16(), 0x0E00);
        assert_eq!(cpu.pending_irq, None);
        assert_eq!(bus.read_u16(0x7FFA).unwrap(), 3, "saved post-HLT IP");
    }

    /// Protected-mode POP SS creates the same exact one-instruction shadow.
    /// Spec: Intel SDM Vol. 2 POP; Vol. 3 §6.8.3.
    #[test]
    fn pop_ss_shadow_delays_irq_one_instruction_in_protected_mode() {
        let (mut cpu, mut bus) = protected_interrupt_fixture(0x20, 0x87, 0);
        bus.mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + 2].copy_from_slice(&[0x17, 0x90]);
        bus.mem[PROTECTED_TEST_GDT + 32..PROTECTED_TEST_GDT + 40]
            .copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x93, 0));
        cpu.gdtr.limit = 39;
        cpu.set_gpr_u16(CpuState::RSP, 0xF000);
        bus.mem[0xF000..0xF002].copy_from_slice(&POP_SEG_SELECTOR.to_le_bytes());
        cpu.set_interrupt_flag(true);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ss.selector, POP_SEG_SELECTOR);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xF002);
        cpu.request_interrupt(0x20);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(
            cpu.ip16(),
            PROTECTED_TEST_CODE as u16 + 2,
            "following NOP must execute before IRQ"
        );
        assert_eq!(cpu.pending_irq, Some(0x20));
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xF002);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), PROTECTED_TEST_HANDLER);
        assert_eq!(cpu.pending_irq, None);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xEFFC);
        assert_eq!(
            bus.read_u16(0xEFFC).unwrap(),
            PROTECTED_TEST_CODE as u16 + 2
        );
    }

    /// The SS shadow masks external IRQ recognition, not NMI delivery.
    /// Spec: Intel SDM Vol. 3 §§6.3.3, 6.7, 6.8.3.
    #[test]
    fn ss_shadow_does_not_block_nmi() {
        let (mut cpu, mut bus) = real_interrupt_shadow_fixture(&[0x8E, 0xD0, 0x90]);

        step(&mut cpu, &mut bus).unwrap();
        cpu.request_nmi();
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.ip16(), 0x0D00);
        assert!(!cpu.pending_nmi);
        assert_eq!(
            bus.read_u16(0x7FFA).unwrap(),
            2,
            "NMI saves following NOP IP"
        );
    }

    /// Descriptor failures do not create an SS interrupt shadow: an IF-enabled
    /// IRQ is recognized before the faulting MOV/POP can be retried.
    ///
    /// Spec: Intel SDM Vol. 2 MOV/POP (Protected Mode Exceptions); Vol. 3 §6.8.3.
    #[test]
    fn failed_mov_and_pop_ss_do_not_arm_interrupt_shadow() {
        for (name, opcode) in [("MOV SS", 0x8E), ("POP SS", 0x17)] {
            let (mut cpu, mut bus) = protected_interrupt_fixture(0x20, 0x87, 0);
            bus.mem[PROTECTED_TEST_GDT + 32..PROTECTED_TEST_GDT + 40]
                .copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x90, 0));
            cpu.gdtr.limit = 39;
            cpu.set_interrupt_flag(true);
            if opcode == 0x8E {
                bus.mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + 2]
                    .copy_from_slice(&[0x8E, 0xD0]);
                cpu.set_ax(POP_SEG_SELECTOR);
            } else {
                bus.mem[PROTECTED_TEST_CODE] = 0x17;
                cpu.set_gpr_u16(CpuState::RSP, 0xF000);
                bus.mem[0xF000..0xF002].copy_from_slice(&POP_SEG_SELECTOR.to_le_bytes());
            }
            let before = cpu.clone();

            assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(POP_SEG_SELECTOR));
            assert_eq!(cpu, before, "{name}: fault changed state");

            cpu.request_interrupt(0x20);
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(
                cpu.ip16(),
                PROTECTED_TEST_HANDLER,
                "{name}: failed load incorrectly delayed IRQ"
            );
        }
    }

    /// A fault from the instruction after MOV SS is delivered normally and
    /// consumes the one-instruction shadow; a pending IRQ can then preempt the
    /// trap handler before its first instruction.
    ///
    /// Spec: Intel SDM Vol. 3 §§6.8.3, 6.11.2, 6.12.1. This bounded model clears
    /// the shadow only after successful fault-gate entry; a failed nested delivery
    /// remains an emulator error without partially retiring the boundary.
    #[test]
    fn ss_shadow_expires_when_following_instruction_faults() {
        const FAULT_HANDLER: u16 = 0x0200;
        const IRQ_HANDLER: u16 = 0x0300;
        let (mut cpu, mut bus) = protected_interrupt_fixture(6, 0x87, 0);
        write_protected_test_gate(
            &mut bus.mem,
            6,
            FAULT_HANDLER,
            PROTECTED_TEST_TARGET_CS,
            0x87,
        );
        write_protected_test_gate(
            &mut bus.mem,
            0x20,
            IRQ_HANDLER,
            PROTECTED_TEST_TARGET_CS,
            0x87,
        );
        cpu.idtr.limit = 0x20 * 8 + 7;
        bus.mem[PROTECTED_TEST_GDT + 32..PROTECTED_TEST_GDT + 40]
            .copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x93, 0));
        cpu.gdtr.limit = 39;
        bus.mem[PROTECTED_TEST_CODE..PROTECTED_TEST_CODE + 4]
            .copy_from_slice(&[0x8E, 0xD0, 0xD0, 0xF0]);
        cpu.set_ax(POP_SEG_SELECTOR);
        cpu.set_interrupt_flag(true);

        step(&mut cpu, &mut bus).unwrap();
        cpu.request_interrupt(0x20);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), FAULT_HANDLER);
        assert_eq!(cpu.pending_irq, Some(0x20));

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), IRQ_HANDLER);
        assert_eq!(cpu.pending_irq, None);
        assert_eq!(bus.read_u16(0xFFF2).unwrap(), FAULT_HANDLER);
    }

    #[derive(Clone, Copy)]
    struct ProtectedFarDataLoadForm {
        opcode: u8,
        op32: bool,
        reg: usize,
        uses_ss: bool,
    }

    fn protected_far_data_load_fixture(
        form: ProtectedFarDataLoadForm,
        pointer_offset: u16,
        offset: u32,
        selector: u16,
        descriptor: Option<[u8; 8]>,
        cpl: u8,
    ) -> (CpuState, VecBus) {
        let ProtectedFarDataLoadForm {
            opcode,
            op32,
            reg,
            uses_ss,
        } = form;
        let mut mem = vec![0u8; 0x10000];
        let mut code_index = PROTECTED_TEST_CODE;
        if op32 {
            mem[code_index] = 0x66;
            code_index += 1;
        }
        mem[code_index] = opcode;
        mem[code_index + 1] = (if uses_ss { 0x80 } else { 0 }) | ((reg as u8) << 3) | 0x06;
        let displacement = if uses_ss { 0 } else { pointer_offset };
        mem[code_index + 2..code_index + 4].copy_from_slice(&displacement.to_le_bytes());

        let pointer_index = usize::from(pointer_offset);
        if op32 {
            mem[pointer_index..pointer_index + 4].copy_from_slice(&offset.to_le_bytes());
            mem[pointer_index + 4..pointer_index + 6].copy_from_slice(&selector.to_le_bytes());
        } else {
            mem[pointer_index..pointer_index + 2].copy_from_slice(&(offset as u16).to_le_bytes());
            mem[pointer_index + 2..pointer_index + 4].copy_from_slice(&selector.to_le_bytes());
        }

        let descriptor_offset = usize::from(selector >> 3) * 8;
        if let Some(descriptor) = descriptor {
            let descriptor_addr = PROTECTED_TEST_GDT + descriptor_offset;
            mem[descriptor_addr..descriptor_addr + 8].copy_from_slice(&descriptor);
        }

        let code_access = 0x9A | (cpl << 5);
        let data_access = 0x93 | (cpl << 5);
        let mut cpu = CpuState::reset();
        cpu.cr0 |= 1;
        cpu.cs = x86_core::SegmentReg {
            selector: 0x0010 | u16::from(cpl),
            base: 0,
            limit: 0xFFFF,
            flags: u16::from(code_access),
        };
        cpu.ss = x86_core::SegmentReg {
            selector: 0x0018 | u16::from(cpl),
            base: 0,
            limit: 0xFFFF,
            flags: u16::from(data_access),
        };
        cpu.ds = x86_core::SegmentReg {
            selector: 0x0030 | u16::from(cpl),
            base: 0,
            limit: 0xFFFF,
            flags: u16::from(data_access),
        };
        cpu.es = x86_core::SegmentReg {
            selector: 0x0038 | u16::from(cpl),
            base: 0x4444_0000,
            limit: 0x4444,
            flags: u16::from(data_access),
        };
        cpu.gdtr.base = PROTECTED_TEST_GDT as u64;
        cpu.gdtr.limit = descriptor
            .map(|_| (descriptor_offset + 7) as u16)
            .unwrap_or(0);
        cpu.rip = PROTECTED_TEST_CODE as u64;
        cpu.rflags = 0x0AD7;
        for (index, value) in cpu.gpr.iter_mut().enumerate() {
            *value = 0xA5A5_5A5A_DEAD_0000 | index as u64;
        }
        if uses_ss {
            cpu.set_gpr_u16(CpuState::RBP, pointer_offset);
        }
        (cpu, VecBus { mem, ports: vec![] })
    }

    /// Protected LDS/LES read the complete far pointer and descriptor before
    /// committing either the destination GPR or the DS/ES visible+hidden state.
    /// The cases cover both operand widths, DS and SS addressing, exact segment
    /// end boundaries, data/readable-code descriptors, and multiple registers.
    ///
    /// Spec: Intel SDM Vol. 2 LDS/LES (Operation, Protected Mode Exceptions);
    /// Vol. 3 §§3.4.3–3.4.5, 5.3–5.6.
    #[test]
    fn protected_les_lds_load_offsets_and_gdt_caches_atomically() {
        let cases = [
            (
                "LES AX data at DS limit",
                0xC4,
                false,
                CpuState::RAX,
                0xFFFC,
                false,
                0x0020,
                0x92,
                0xD0,
                0,
                0x0000_BEEF,
            ),
            (
                "LDS DI readable code through SS",
                0xC5,
                false,
                CpuState::RDI,
                0x2800,
                true,
                0x0020,
                0x9A,
                0x10,
                0,
                0x0000_1234,
            ),
            (
                "LES ECX conforming readable code through SS",
                0xC4,
                true,
                CpuState::RCX,
                0x3000,
                true,
                0x0023,
                0x9E,
                0x80,
                3,
                0x89AB_CDEF,
            ),
            (
                "LDS EBX ring-3 data at DS limit",
                0xC5,
                true,
                CpuState::RBX,
                0xFFFA,
                false,
                0x0023,
                0xF2,
                0x00,
                3,
                0x0123_4567,
            ),
        ];

        for (name, opcode, op32, reg, pointer, uses_ss, selector, access, gran, cpl, offset) in
            cases
        {
            let base = 0x1234_5000u32
                .wrapping_add(u32::from(opcode) << 8)
                .wrapping_add(reg as u32);
            let raw_limit = 0x1_2345;
            let descriptor = encode_seg_desc(base, raw_limit, access, gran);
            let (mut cpu, mut bus) = protected_far_data_load_fixture(
                ProtectedFarDataLoadForm {
                    opcode,
                    op32,
                    reg,
                    uses_ss,
                },
                pointer,
                offset,
                selector,
                Some(descriptor),
                cpl,
            );
            let before = cpu.clone();

            step_inner(&mut cpu, &mut bus).unwrap();

            let loaded = if opcode == 0xC4 { &cpu.es } else { &cpu.ds };
            assert_eq!(loaded.selector, selector, "{name}: selector");
            assert_eq!(loaded.base, u64::from(base), "{name}: base");
            let expected_limit = if gran & 0x80 != 0 {
                (raw_limit << 12) | 0xFFF
            } else {
                raw_limit
            };
            assert_eq!(loaded.limit, expected_limit, "{name}: effective limit");
            assert_eq!(
                loaded.flags,
                u16::from(access) | (u16::from(gran & 0xF0) << 8),
                "{name}: cached attributes"
            );

            let write_mask = if op32 { 0xFFFF_FFFF } else { 0xFFFF };
            let expected_gpr = (before.gpr[reg] & !write_mask) | (u64::from(offset) & write_mask);
            assert_eq!(cpu.gpr[reg], expected_gpr, "{name}: destination width");
            for index in 0..cpu.gpr.len() {
                if index != reg {
                    assert_eq!(cpu.gpr[index], before.gpr[index], "{name}: GPR {index}");
                }
            }
            assert_eq!(cpu.rflags, before.rflags, "{name}: FLAGS");
            assert_eq!(cpu.ss, before.ss, "{name}: unrelated SS");
            if opcode == 0xC4 {
                assert_eq!(cpu.ds, before.ds, "{name}: unrelated DS");
            } else {
                assert_eq!(cpu.es, before.es, "{name}: unrelated ES");
            }
            assert_eq!(
                cpu.ip16(),
                PROTECTED_TEST_CODE as u16 + if op32 { 5 } else { 4 },
                "{name}: IP"
            );
        }
    }

    /// A null selector (index zero, including a nonzero RPL) is legal for
    /// LDS→DS and LES→ES. It loads the offset and the repository's cleared,
    /// unusable data-segment cache without consulting a descriptor.
    ///
    /// Spec: Intel SDM Vol. 2 LDS/LES (Operation, Protected Mode Exceptions);
    /// Vol. 3 §§3.4.2–3.4.3, 5.4.1.
    #[test]
    fn protected_les_lds_accept_null_selectors() {
        let cases = [
            (
                "LES DX null+RPL",
                0xC4,
                false,
                CpuState::RDX,
                0x0003,
                0xCAFEu32,
            ),
            (
                "LDS ESI null",
                0xC5,
                true,
                CpuState::RSI,
                0x0000,
                0x7654_3210,
            ),
        ];

        for (name, opcode, op32, reg, selector, offset) in cases {
            let (mut cpu, mut bus) = protected_far_data_load_fixture(
                ProtectedFarDataLoadForm {
                    opcode,
                    op32,
                    reg,
                    uses_ss: false,
                },
                0x3000,
                offset,
                selector,
                None,
                0,
            );
            let before = cpu.clone();

            step_inner(&mut cpu, &mut bus).unwrap();

            let loaded = if opcode == 0xC4 { &cpu.es } else { &cpu.ds };
            assert_eq!(loaded.selector, selector, "{name}: visible selector");
            assert_eq!(loaded.base, 0, "{name}: base");
            assert_eq!(loaded.limit, 0, "{name}: limit");
            assert_eq!(loaded.flags, 0, "{name}: attributes");
            let write_mask = if op32 { 0xFFFF_FFFF } else { 0xFFFF };
            assert_eq!(
                cpu.gpr[reg],
                (before.gpr[reg] & !write_mask) | (u64::from(offset) & write_mask),
                "{name}: destination"
            );
            assert_eq!(cpu.rflags, before.rflags, "{name}: FLAGS");
        }
    }

    /// LDS/LES use the DS/ES data-segment load rules: system and execute-only
    /// descriptors fail, valid-but-not-present descriptors raise #NP, data and
    /// nonconforming readable code enforce CPL/RPL≤DPL, and only GDT selectors
    /// within the table limit are supported. Every failure is atomic.
    ///
    /// Spec: Intel SDM Vol. 2 LDS/LES (Protected Mode Exceptions); Vol. 3
    /// §§3.4.5, 5.4.1, 5.5–5.6, 6.13.
    #[test]
    fn protected_les_lds_descriptor_fault_matrix_is_atomic() {
        let cases = [
            (
                "LES system",
                0xC4,
                false,
                CpuState::RAX,
                0x0020,
                0x80,
                0,
                None,
                13,
                0x20,
            ),
            (
                "LDS execute-only code",
                0xC5,
                true,
                CpuState::RBX,
                0x0020,
                0x98,
                0,
                None,
                13,
                0x20,
            ),
            (
                "LES not-present data",
                0xC4,
                true,
                CpuState::RCX,
                0x0020,
                0x12,
                0,
                None,
                11,
                0x20,
            ),
            (
                "LDS readable-code RPL above DPL",
                0xC5,
                false,
                CpuState::RDI,
                0x0023,
                0x9A,
                0,
                None,
                13,
                0x20,
            ),
            (
                "LES CPL above data DPL",
                0xC4,
                false,
                CpuState::RDX,
                0x0020,
                0xD2,
                3,
                None,
                13,
                0x20,
            ),
            (
                "LDS LDT selector",
                0xC5,
                true,
                CpuState::RSI,
                0x0024,
                0x92,
                0,
                None,
                13,
                0x24,
            ),
            (
                "LES GDT limit",
                0xC4,
                true,
                CpuState::RBP,
                0x0020,
                0x92,
                0,
                Some(38),
                13,
                0x20,
            ),
        ];

        for (name, opcode, op32, reg, selector, access, cpl, gdt_limit, vector, error_code) in cases
        {
            let descriptor = encode_seg_desc(0x1234_0000, 0xFFFF, access, 0);
            let (mut cpu, mut bus) = protected_far_data_load_fixture(
                ProtectedFarDataLoadForm {
                    opcode,
                    op32,
                    reg,
                    uses_ss: false,
                },
                0x3000,
                0x89AB_CDEF,
                selector,
                Some(descriptor),
                cpl,
            );
            if let Some(limit) = gdt_limit {
                cpu.gdtr.limit = limit;
            }
            let before = cpu.clone();

            assert_arch_fault(step_inner(&mut cpu, &mut bus), vector, Some(error_code));
            assert_eq!(cpu, before, "{name}: partial GPR or segment-cache update");
        }
    }

    /// The full m16:16/m16:32 operand must fit in the old source segment and be
    /// readable before descriptor validation. Pointer and descriptor read
    /// failures retain the destination GPR, DS/ES cache, IP, and FLAGS.
    ///
    /// Spec: Intel SDM Vol. 2 LDS/LES (Protected Mode Exceptions); Vol. 3
    /// §§5.3, 6.13, 6.15.
    #[test]
    fn protected_les_lds_pointer_and_descriptor_read_faults_are_atomic() {
        let descriptor = encode_seg_desc(0x1234_0000, 0xFFFF, 0x92, 0);

        let (mut cpu, mut bus) = protected_far_data_load_fixture(
            ProtectedFarDataLoadForm {
                opcode: 0xC4,
                op32: false,
                reg: CpuState::RAX,
                uses_ss: false,
            },
            0x3000,
            0xBEEF,
            0x0020,
            Some(descriptor),
            0,
        );
        cpu.ds.limit = 0x3002;
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before, "m16:16 DS-limit failure changed state");

        let (mut cpu, mut bus) = protected_far_data_load_fixture(
            ProtectedFarDataLoadForm {
                opcode: 0xC5,
                op32: true,
                reg: CpuState::RBX,
                uses_ss: true,
            },
            0x3000,
            0x89AB_CDEF,
            0x0020,
            Some(descriptor),
            0,
        );
        cpu.ss.limit = 0x3004;
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 12, Some(0));
        assert_eq!(cpu, before, "m16:32 SS-limit failure changed state");

        let (mut cpu, mut bus) = protected_far_data_load_fixture(
            ProtectedFarDataLoadForm {
                opcode: 0xC4,
                op32: true,
                reg: CpuState::RCX,
                uses_ss: false,
            },
            0xF000,
            0x0123_4567,
            0x0020,
            Some(descriptor),
            0,
        );
        bus.mem.truncate(0xF005); // selector high byte is absent
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before, "truncated m16:32 changed state");

        let (mut cpu, fixture) = protected_far_data_load_fixture(
            ProtectedFarDataLoadForm {
                opcode: 0xC5,
                op32: false,
                reg: CpuState::RDI,
                uses_ss: true,
            },
            0x3000,
            0xCAFE,
            0x0020,
            Some(descriptor),
            0,
        );
        let mut bus = FailOnceReadBus {
            mem: fixture.mem,
            fail_addr: 0x3003,
            failed: false,
        };
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 12, Some(0));
        assert!(bus.failed, "selector high byte must be read through old SS");
        assert_eq!(cpu, before, "late m16:16 read failure changed state");

        for (name, opcode, op32, reg) in [
            ("LES descriptor byte 7", 0xC4, false, CpuState::RDX),
            ("LDS descriptor byte 7", 0xC5, true, CpuState::RSI),
        ] {
            let (mut cpu, fixture) = protected_far_data_load_fixture(
                ProtectedFarDataLoadForm {
                    opcode,
                    op32,
                    reg,
                    uses_ss: false,
                },
                0x3000,
                0x7654_3210,
                0x0020,
                Some(descriptor),
                0,
            );
            let mut bus = FailOnceReadBus {
                mem: fixture.mem,
                fail_addr: PROTECTED_TEST_GDT as u64 + 0x20 + 7,
                failed: false,
            };
            let before = cpu.clone();

            assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
            assert!(bus.failed, "{name}: final descriptor byte was not read");
            assert_eq!(cpu, before, "{name}: partial update");
        }
    }

    /// ModRM.mod=11 is invalid for LDS/LES in protected mode as in real mode.
    /// Spec: Intel SDM Vol. 2 LDS/LES; Vol. 3 §6.15 (#UD).
    #[test]
    fn protected_les_lds_register_source_is_ud_and_atomic() {
        for (name, opcode, op32, reg) in [
            ("LES r16,r16", 0xC4, false, CpuState::RAX),
            ("LDS r32,r32", 0xC5, true, CpuState::RBX),
        ] {
            let (mut cpu, mut bus) = protected_far_data_load_fixture(
                ProtectedFarDataLoadForm {
                    opcode,
                    op32,
                    reg,
                    uses_ss: false,
                },
                0x3000,
                0,
                0x0020,
                Some(encode_seg_desc(0, 0xFFFF, 0x92, 0)),
                0,
            );
            let opcode_index = PROTECTED_TEST_CODE + usize::from(op32);
            bus.mem[opcode_index + 1] = 0xC0 | ((reg as u8) << 3) | 1;
            let before = cpu.clone();

            assert_arch_fault(step_inner(&mut cpu, &mut bus), 6, None);
            assert_eq!(cpu, before, "{name}: #UD changed state");
        }
    }

    // ----------------------------------------------------------------------
    // Milestone 2 round 1 — same-CPL 32-bit protected mode (`docs/pm32-*.md`).
    // ----------------------------------------------------------------------

    const PM32_MEM_LEN: usize = 0x3_0000;
    const PM32_CODE: usize = 0x1000;
    const PM32_HIGH_CODE: usize = 0x2_1000;
    const PM32_DATA: usize = 0x2000;
    const PM32_GDT: usize = 0x4000;
    /// Ring-0 `D=1` code (base 0, G=1, 4 GiB limit).
    const PM32_CS32: u16 = 0x0008;
    /// Ring-0 `D=0` code (base 0, 64 KiB limit).
    const PM32_CS16: u16 = 0x0010;
    /// Ring-0 writable data / `B=0` stack (base 0, 64 KiB limit).
    const PM32_DS: u16 = 0x0018;

    /// Protected-mode fixture with both a `D=1` and a `D=0` ring-0 code
    /// segment in the GDT. `default_big` selects the segment CS starts in.
    ///
    /// Spec: Intel SDM Vol. 3 §§3.4.5, 5.8.1.
    fn pm32_fixture(code: &[u8], entry: usize, default_big: bool) -> (CpuState, VecBus) {
        let mut mem = vec![0u8; PM32_MEM_LEN];
        mem[entry..entry + code.len()].copy_from_slice(code);
        mem[PM32_GDT + 8..PM32_GDT + 16].copy_from_slice(&encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0));
        mem[PM32_GDT + 16..PM32_GDT + 24].copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x9A, 0));
        mem[PM32_GDT + 24..PM32_GDT + 32].copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x92, 0));

        let mut cpu = CpuState::reset();
        cpu.cr0 |= 1;
        cpu.gdtr.base = PM32_GDT as u64;
        cpu.gdtr.limit = 31;
        cpu.cs = if default_big {
            x86_core::SegmentReg {
                selector: PM32_CS32,
                base: 0,
                limit: 0xFFFF_FFFF,
                flags: 0xC09A,
            }
        } else {
            x86_core::SegmentReg {
                selector: PM32_CS16,
                base: 0,
                limit: 0xFFFF,
                flags: 0x009A,
            }
        };
        let data = x86_core::SegmentReg {
            selector: PM32_DS,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        cpu.ds = data.clone();
        cpu.es = data.clone();
        cpu.ss = data;
        cpu.rip = entry as u64;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFF0);
        (cpu, VecBus { mem, ports: vec![] })
    }

    /// Intel SDM Vol. 1 §3.6 (Table 3-4); Vol. 3 §3.4.5: with `CS.D=1` the
    /// default operand size is 32 and `0x66` selects 16. The identical opcode
    /// bytes must execute differently under `D=0` and `D=1`.
    #[test]
    fn cs_default_big_selects_32_bit_default_operand_size() {
        // B8 id / B8 iw — MOV EAX, imm32 vs MOV AX, imm16.
        let bytes = [0xB8, 0x78, 0x56, 0x34, 0x12];

        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RAX, 0xAAAA_BBBB);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x1234_5678);
        assert_eq!(cpu.rip, (PM32_CODE + 5) as u64);

        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_CODE, false);
        cpu.set_gpr_u32(CpuState::RAX, 0xAAAA_BBBB);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0xAAAA_5678, "D=0 must keep EAX[31:16]");
        assert_eq!(cpu.ip16(), (PM32_CODE + 3) as u16);

        // 66 B8 iw under D=1 — the override selects the 16-bit operand size.
        let (mut cpu, mut bus) = pm32_fixture(&[0x66, 0xB8, 0x34, 0x12], PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RAX, 0xAAAA_BBBB);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0xAAAA_1234);
        assert_eq!(cpu.rip, (PM32_CODE + 4) as u64);
    }

    /// Intel SDM Vol. 2 "ADD"/"INC"/"PUSH": ModR/M ALU and register forms pick
    /// their width from the same effective operand-size attribute.
    #[test]
    fn cs_default_big_widens_alu_and_register_forms() {
        // 01 D8 = ADD EAX, EBX (D=1) / ADD AX, BX (D=0).
        for (default_big, expected) in [(true, 0x0001_0000u32), (false, 0x0000_0000)] {
            let (mut cpu, mut bus) = pm32_fixture(&[0x01, 0xD8], PM32_CODE, default_big);
            cpu.set_gpr_u32(CpuState::RAX, 0x0000_FFFF);
            cpu.set_gpr_u32(CpuState::RBX, 0x0000_0001);
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.eax(), expected, "default_big={default_big}");
        }

        // 40 = INC EAX (D=1) / INC AX (D=0).
        for (default_big, expected) in [(true, 0x0001_0000u32), (false, 0x0000_0000)] {
            let (mut cpu, mut bus) = pm32_fixture(&[0x40], PM32_CODE, default_big);
            cpu.set_gpr_u32(CpuState::RAX, 0x0000_FFFF);
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.eax(), expected, "default_big={default_big}");
        }
    }

    /// Intel SDM Vol. 2 Chapter 2 (ModR/M, SIB, displacement): `CS.D=1` makes
    /// 32-bit addressing the default and `0x67` restores the 16-bit forms.
    #[test]
    fn cs_default_big_selects_32_bit_default_address_size() {
        // 8B 05 id = MOV EAX, [disp32] — no 0x67 needed under D=1.
        let bytes = [0x8B, 0x05, 0x00, 0x20, 0x00, 0x00];
        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_CODE, true);
        bus.mem[PM32_DATA..PM32_DATA + 4].copy_from_slice(&0x1122_3344u32.to_le_bytes());
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x1122_3344);
        assert_eq!(cpu.rip, (PM32_CODE + 6) as u64);

        // 67 8B 06 iw = the 16-bit [disp16] ModR/M form (operand size stays 32).
        let bytes = [0x67, 0x8B, 0x06, 0x00, 0x20];
        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_CODE, true);
        bus.mem[PM32_DATA..PM32_DATA + 4].copy_from_slice(&0x1122_3344u32.to_le_bytes());
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x1122_3344);
        assert_eq!(cpu.rip, (PM32_CODE + 5) as u64);

        // 8B 44 24 04 = MOV EAX, [ESP+4] — SIB base ESP defaults to SS.
        let bytes = [0x8B, 0x44, 0x24, 0x04];
        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RSP, 0x0000_1F00);
        bus.mem[0x1F04..0x1F08].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0xDEAD_BEEF);
        assert_eq!(cpu.rip, (PM32_CODE + 4) as u64);
    }

    /// Intel SDM Vol. 2 "JMP" (near relative, Operation): `CS.D=1` executes a
    /// full 32-bit EIP, while a 16-bit operand size clears `EIP[31:16]`.
    #[test]
    fn cs_default_big_near_jmp_uses_32_bit_eip() {
        // E9 cd from 0x1000 to 0x21000.
        let rel = (PM32_HIGH_CODE as i64 - (PM32_CODE + 5) as i64) as i32;
        let mut bytes = vec![0xE9];
        bytes.extend_from_slice(&rel.to_le_bytes());
        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_CODE, true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, PM32_HIGH_CODE as u64);

        // 66 E9 cw at EIP 0x21000: tempEIP = 0x21014 AND 0000FFFFH = 0x1014.
        let bytes = [0x66, 0xE9, 0x10, 0x00];
        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_HIGH_CODE, true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, 0x1014);

        // EB cb short jumps stay within the same 32-bit window.
        let (mut cpu, mut bus) = pm32_fixture(&[0xEB, 0x02], PM32_HIGH_CODE, true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, (PM32_HIGH_CODE + 4) as u64);
    }

    /// Intel SDM Vol. 2 "CALL"/"RET" (near): under `CS.D=1` the default
    /// operand size pushes and pops a 32-bit return EIP. The stack itself is
    /// still `SS.B=0` here (16-bit SP) — 32-bit stacks are a separate slice.
    #[test]
    fn cs_default_big_near_call_ret_round_trips_32_bit_return_eip() {
        let rel = (PM32_HIGH_CODE as i64 - (PM32_CODE + 5) as i64) as i32;
        let mut bytes = vec![0xE8];
        bytes.extend_from_slice(&rel.to_le_bytes());
        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_CODE, true);
        bus.mem[PM32_HIGH_CODE] = 0xC3; // RET near
        let sp_before = cpu.gpr_u16(CpuState::RSP);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, PM32_HIGH_CODE as u64);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp_before.wrapping_sub(4));
        assert_eq!(
            bus.read_u32(u64::from(cpu.gpr_u16(CpuState::RSP))).unwrap(),
            (PM32_CODE + 5) as u32
        );

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, (PM32_CODE + 5) as u64);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp_before);
    }

    /// Intel SDM Vol. 2 "JMP" (far, Protected Mode); Vol. 3 §§3.4.5, 5.8.1:
    /// a same-CPL direct far jump can enter a `D=1` code segment (switching
    /// the execution window to 32-bit) and return to a `D=0` segment.
    #[test]
    fn protected_far_jump_enters_and_leaves_default_32_code_segment() {
        // EA ptr16:16 from D=0 code into the D=1 code segment.
        let bytes = [0xEA, 0x00, 0x10, PM32_CS32 as u8, 0x00];
        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_CODE, false);
        // MOV EAX, imm32 at the D=1 entry proves the new default operand size.
        bus.mem[0x1000 + 5..0x1000 + 10].copy_from_slice(&[0xB8, 0x78, 0x56, 0x34, 0x12]);
        bus.mem[0x1000..0x1005].copy_from_slice(&bytes);
        cpu.rip = PM32_CODE as u64;

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, PM32_CS32);
        assert!(cpu.cs.default_big());
        assert_eq!(cpu.cs.flags, 0xC09A);
        assert_eq!(cpu.cs.limit, 0xFFFF_FFFF);
        assert_eq!(cpu.rip, 0x1000);

        // EA ptr16:32 (7 bytes) is the D=1 default far-pointer form; go back
        // to the D=0 segment, whose 64 KiB limit bounds the offset.
        let mut back = vec![0xEA];
        back.extend_from_slice(&(PM32_DATA as u32).to_le_bytes());
        back.extend_from_slice(&PM32_CS16.to_le_bytes());
        assert_eq!(back.len(), 7);
        let (mut cpu, mut bus) = pm32_fixture(&back, PM32_HIGH_CODE, true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, PM32_CS16);
        assert!(!cpu.cs.default_big());
        assert_eq!(cpu.cs.flags, 0x009A);
        assert_eq!(cpu.cs.limit, 0xFFFF);
        assert_eq!(cpu.ip16(), PM32_DATA as u16);
    }

    /// Intel SDM Vol. 2 "JMP" (far, Protected Mode Exceptions): a far jump
    /// whose 32-bit offset exceeds the target segment limit raises `#GP(0)`
    /// and commits nothing.
    #[test]
    fn protected_far_jump_offset_beyond_target_limit_faults_atomically() {
        let mut bytes = vec![0xEA];
        bytes.extend_from_slice(&0x0001_0000u32.to_le_bytes());
        bytes.extend_from_slice(&PM32_CS16.to_le_bytes());
        let (mut cpu, mut bus) = pm32_fixture(&bytes, PM32_CODE, true);
        let before = cpu.clone();

        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before, "far JMP committed state before #GP");
    }

    /// Intel SDM Vol. 2 "CALL" (far); Vol. 3 §5.8.1: same-CPL far CALL into a
    /// nonconforming GDT code segment pushes the return link then loads CS.
    /// Call gates and privilege changes remain unsupported.
    #[test]
    fn protected_far_call_direct_and_indirect_push_return_link() {
        // 9A ptr16:32 — D=1 default form into the D=0 code segment at PM32_DATA.
        let mut direct = vec![0x9A];
        direct.extend_from_slice(&(PM32_DATA as u32).to_le_bytes());
        direct.extend_from_slice(&PM32_CS16.to_le_bytes());
        let (mut cpu, mut bus) = pm32_fixture(&direct, PM32_CODE, true);
        let return_ip = (PM32_CODE + direct.len()) as u32;
        let sp_before = cpu.gpr_u16(CpuState::RSP);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, PM32_CS16);
        assert!(!cpu.cs.default_big());
        assert_eq!(cpu.cs.flags, 0x009A);
        assert_eq!(cpu.rip, PM32_DATA as u64);
        // 6-byte frame: CS then EIP (SS.B=0 wraps SP).
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp_before.wrapping_sub(6));
        let sp = cpu.gpr_u16(CpuState::RSP) as usize;
        assert_eq!(
            u32::from_le_bytes(bus.mem[sp..sp + 4].try_into().unwrap()),
            return_ip
        );
        assert_eq!(
            u16::from_le_bytes(bus.mem[sp + 4..sp + 6].try_into().unwrap()),
            PM32_CS32
        );

        // FF /3 m16:16 under D=0: call into D=1 code.
        // ModRM 0x1E = mod=00,reg=3,rm=6 → [disp16]; pointer at 0x3000.
        let indirect = [0xFF, 0x1E, 0x00, 0x30];
        let (mut cpu, mut bus) = pm32_fixture(&indirect, PM32_CODE, false);
        bus.mem[0x3000..0x3002].copy_from_slice(&(0x1000u16).to_le_bytes());
        bus.mem[0x3002..0x3004].copy_from_slice(&PM32_CS32.to_le_bytes());
        let return_ip = (PM32_CODE + indirect.len()) as u16;
        let sp_before = cpu.gpr_u16(CpuState::RSP);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.cs.selector, PM32_CS32);
        assert!(cpu.cs.default_big());
        assert_eq!(cpu.rip, 0x1000);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), sp_before.wrapping_sub(4));
        let sp = cpu.gpr_u16(CpuState::RSP) as usize;
        assert_eq!(
            u16::from_le_bytes(bus.mem[sp..sp + 2].try_into().unwrap()),
            return_ip
        );
        assert_eq!(
            u16::from_le_bytes(bus.mem[sp + 2..sp + 4].try_into().unwrap()),
            PM32_CS16
        );

        // Null selector → #GP(0), stack and CS unchanged.
        let mut bad = vec![0x9A];
        bad.extend_from_slice(&0u32.to_le_bytes());
        bad.extend_from_slice(&0u16.to_le_bytes());
        let (mut cpu, mut bus) = pm32_fixture(&bad, PM32_CODE, true);
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before, "far CALL null selector must be atomic");
        assert_eq!(cpu.gpr_u16(CpuState::RSP), before.gpr_u16(CpuState::RSP));
    }

    /// A 286 (16-bit) gate cannot carry a 32-bit return EIP, so a fault raised
    /// while `CS.D=1` cannot enter that gate. Delivery fails, escalates to
    /// `#DF`, and with no usable `#DF` gate becomes a triple fault.
    ///
    /// Spec: Intel SDM Vol. 3 §6.11 (gate types); §6.12.1; §6.15.
    #[test]
    fn default_32_execution_cannot_yet_enter_16_bit_idt_gates() {
        // D0 F0 = Group 2 /6 (reserved) → #UD.
        let (mut cpu, mut bus) = pm32_fixture(&[0xD0, 0xF0], PM32_CODE, true);
        // A valid 386 16-bit interrupt gate (type 0x6) for vector 6.
        bus.mem[0x5000 + 6 * 8..0x5000 + 6 * 8 + 8]
            .copy_from_slice(&encode_idt_gate(0x0800, PM32_CS16, 0x86));
        cpu.idtr.base = 0x5000;
        cpu.idtr.limit = 0x07FF;

        let err = step(&mut cpu, &mut bus).expect_err("16-bit gate under CS.D=1");
        assert!(
            matches!(
                err,
                ExecError::TripleFault {
                    reason: ProtectedModeDeliveryError::GateType(0)
                        | ProtectedModeDeliveryError::CurrentPrivilege
                        | ProtectedModeDeliveryError::IdtLimit
                        | ProtectedModeDeliveryError::GateNotPresent
                }
            ),
            "expected triple fault after #DF escalation, got {err:?}"
        );
    }

    /// Ring-0 writable `B=1` stack selector used by the 32-bit stack tests.
    const PM32_SS32: u16 = 0x0020;
    const PM32_TEST_ESP: u32 = 0x0002_0000;

    /// `CS.D=1` fixture whose stack segment has `B=1` and a 4 GiB limit.
    ///
    /// Spec: Intel SDM Vol. 3 §3.4.5.1 (B flag); Vol. 1 §6.2.
    fn pm32_big_stack_fixture(code: &[u8], entry: usize, esp: u32) -> (CpuState, VecBus) {
        let (mut cpu, bus) = pm32_fixture(code, entry, true);
        cpu.ss = x86_core::SegmentReg {
            selector: PM32_SS32,
            base: 0,
            limit: 0xFFFF_FFFF,
            flags: 0xC092,
        };
        cpu.set_gpr_u32(CpuState::RSP, esp);
        (cpu, bus)
    }

    /// Intel SDM Vol. 1 §6.2.2; Vol. 2 "PUSH"/"POP" (Operation, `StackAddrSize`):
    /// with `SS.B=1` the stack pointer is the full 32-bit ESP; with `SS.B=0`
    /// only SP changes and `ESP[31:16]` is preserved.
    #[test]
    fn stack_b_flag_selects_esp_or_sp() {
        // 68 id = PUSH imm32; 58 = POP EAX.
        let code = [0x68, 0x78, 0x56, 0x34, 0x12, 0x58];

        let (mut cpu, mut bus) = pm32_big_stack_fixture(&code, PM32_CODE, PM32_TEST_ESP);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 4);
        assert_eq!(
            bus.read_u32(u64::from(PM32_TEST_ESP - 4)).unwrap(),
            0x1234_5678
        );
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x1234_5678);
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);

        // Same code, same ESP, but the `B=0` stack of `pm32_fixture` wraps SP.
        let (mut cpu, mut bus) = pm32_fixture(&code, PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RSP, PM32_TEST_ESP);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), 0x0002_FFFC);
        assert_eq!(bus.read_u32(0xFFFC).unwrap(), 0x1234_5678);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x1234_5678);
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
    }

    /// Intel SDM Vol. 3 §5.3 (limit checking) with a 32-bit stack: the pointer
    /// wraps modulo 2^32, so a push through offset 0 leaves the segment limit
    /// and raises `#SS(0)` without committing ESP.
    #[test]
    fn stack_b1_wraps_at_32_bits_and_faults_outside_the_limit() {
        let (mut cpu, mut bus) =
            pm32_big_stack_fixture(&[0x68, 0x78, 0x56, 0x34, 0x12], PM32_CODE, 2);
        cpu.ss.limit = 0x0001_FFFF;
        let before = cpu.clone();

        assert_arch_fault(step_inner(&mut cpu, &mut bus), 12, Some(0));
        assert_eq!(cpu, before, "faulting push committed ESP");
    }

    /// Intel SDM Vol. 2 "PUSHF/PUSHFD" and "POPF/POPFD": the pushed width
    /// follows the operand size while the pointer follows `SS.B`.
    #[test]
    fn stack_b1_pushfd_popfd_use_esp() {
        // 9C = PUSHFD (D=1); 9D = POPFD.
        let (mut cpu, mut bus) = pm32_big_stack_fixture(&[0x9C, 0x9D], PM32_CODE, PM32_TEST_ESP);
        cpu.rflags = 0x0000_0000_0000_0A57;

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 4);
        assert_eq!(bus.read_u32(u64::from(PM32_TEST_ESP - 4)).unwrap(), 0x0A57);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
        assert_eq!(cpu.rflags & 0xFFFF, 0x0A57);
    }

    /// Intel SDM Vol. 2 "PUSHA/PUSHAD" and "POPA/POPAD": `Temp` is the stack
    /// pointer before the first push and the saved slot is discarded on pop.
    /// `StackAddrSize` (SS.B), not the `0x67` address-size prefix, selects ESP.
    #[test]
    fn stack_b1_pushad_popad_use_esp_and_ignore_address_size_prefix() {
        for prefix in [None, Some(0x67u8)] {
            let mut code = Vec::new();
            if let Some(p) = prefix {
                code.push(p);
            }
            code.push(0x60); // PUSHAD
            let pushad_len = code.len();
            if let Some(p) = prefix {
                code.push(p);
            }
            code.push(0x61); // POPAD

            let (mut cpu, mut bus) = pm32_big_stack_fixture(&code, PM32_CODE, PM32_TEST_ESP);
            for (index, value) in [
                0x1111_1111u32,
                0x2222_2222,
                0x3333_3333,
                0x4444_4444,
                0,
                0x6666_6666,
                0x7777_7777,
                0x8888_8888,
            ]
            .into_iter()
            .enumerate()
            {
                if index != CpuState::RSP {
                    cpu.set_gpr_u32(index, value);
                }
            }
            let expected = cpu.gpr;

            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 32);
            assert_eq!(cpu.rip, (PM32_CODE + pushad_len) as u64);
            // EAX…EBX, then Temp = ESP before the first push.
            assert_eq!(
                bus.read_u32(u64::from(PM32_TEST_ESP - 4)).unwrap(),
                0x1111_1111
            );
            assert_eq!(
                bus.read_u32(u64::from(PM32_TEST_ESP - 20)).unwrap(),
                PM32_TEST_ESP
            );
            assert_eq!(
                bus.read_u32(u64::from(PM32_TEST_ESP - 32)).unwrap(),
                0x8888_8888
            );

            // Scramble everything so POPAD has to restore it.
            for index in 0..8 {
                if index != CpuState::RSP {
                    cpu.set_gpr_u32(index, 0xDEAD_0000 + index as u32);
                }
            }
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
            for (index, (&actual, &want)) in cpu.gpr.iter().zip(expected.iter()).enumerate().take(8)
            {
                if index != CpuState::RSP {
                    assert_eq!(actual, want, "gpr {index}");
                }
            }
        }
    }

    /// Intel SDM Vol. 2 "CALL"/"RET" (near) with a 32-bit stack: the 32-bit
    /// return EIP is pushed and popped through ESP.
    #[test]
    fn stack_b1_near_call_ret_use_esp() {
        let rel = (PM32_HIGH_CODE as i64 - (PM32_CODE + 5) as i64) as i32;
        let mut code = vec![0xE8];
        code.extend_from_slice(&rel.to_le_bytes());
        let (mut cpu, mut bus) = pm32_big_stack_fixture(&code, PM32_CODE, PM32_TEST_ESP);
        bus.mem[PM32_HIGH_CODE] = 0xC3;

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, PM32_HIGH_CODE as u64);
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 4);
        assert_eq!(
            bus.read_u32(u64::from(PM32_TEST_ESP - 4)).unwrap(),
            (PM32_CODE + 5) as u32
        );

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, (PM32_CODE + 5) as u64);
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
    }

    /// Intel SDM Vol. 2 "RET" (near, imm16) with a 32-bit stack: the release
    /// count is added to ESP, not SP.
    #[test]
    fn stack_b1_ret_imm16_releases_through_esp() {
        // C2 iw = RET 8.
        let (mut cpu, mut bus) =
            pm32_big_stack_fixture(&[0xC2, 0x08, 0x00], PM32_CODE, PM32_TEST_ESP - 4);
        bus.mem[(PM32_TEST_ESP - 4) as usize..(PM32_TEST_ESP) as usize]
            .copy_from_slice(&(PM32_HIGH_CODE as u32).to_le_bytes());

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, PM32_HIGH_CODE as u64);
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP + 8);
    }

    /// Intel SDM Vol. 2 "ENTER"/"LEAVE"; Vol. 1 §6.5: with `StackAddrSize=32`
    /// the frame pointer and allocation use ESP/EBP. The `0x67` address-size
    /// prefix does not change the stack address size.
    #[test]
    fn stack_b1_enter_leave_use_esp_and_ebp() {
        for prefix in [None, Some(0x67u8)] {
            let mut code = Vec::new();
            if let Some(p) = prefix {
                code.push(p);
            }
            code.extend_from_slice(&[0xC8, 0x08, 0x00, 0x00]); // ENTER 8, 0
            let enter_len = code.len();
            if let Some(p) = prefix {
                code.push(p);
            }
            code.push(0xC9); // LEAVE

            let (mut cpu, mut bus) = pm32_big_stack_fixture(&code, PM32_CODE, PM32_TEST_ESP);
            cpu.set_gpr_u32(CpuState::RBP, 0xAAAA_BBBB);

            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.rip, (PM32_CODE + enter_len) as u64);
            assert_eq!(cpu.gpr_u32(CpuState::RBP), PM32_TEST_ESP - 4);
            assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 12);
            assert_eq!(
                bus.read_u32(u64::from(PM32_TEST_ESP - 4)).unwrap(),
                0xAAAA_BBBB
            );

            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.gpr_u32(CpuState::RBP), 0xAAAA_BBBB);
            assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
        }
    }

    /// Intel SDM Vol. 2 "ENTER" (nesting level > 0): the display pointers are
    /// walked through EBP with the 32-bit stack pointer.
    #[test]
    fn stack_b1_enter_nesting_walks_display_through_ebp() {
        // C8 iw ib = ENTER 4, 2.
        let (mut cpu, mut bus) =
            pm32_big_stack_fixture(&[0xC8, 0x04, 0x00, 0x02], PM32_CODE, PM32_TEST_ESP);
        let caller_bp = PM32_TEST_ESP - 0x40;
        cpu.set_gpr_u32(CpuState::RBP, caller_bp);
        bus.mem[(caller_bp - 4) as usize..caller_bp as usize]
            .copy_from_slice(&0x0BAD_F00Du32.to_le_bytes());

        step(&mut cpu, &mut bus).unwrap();
        // Saved EBP, one copied display pointer, the new frame pointer, then 4
        // bytes of locals.
        let frame = PM32_TEST_ESP - 4;
        assert_eq!(cpu.gpr_u32(CpuState::RBP), frame);
        assert_eq!(cpu.gpr_u32(CpuState::RSP), frame - 8 - 4);
        assert_eq!(bus.read_u32(u64::from(frame)).unwrap(), caller_bp);
        assert_eq!(bus.read_u32(u64::from(frame - 4)).unwrap(), 0x0BAD_F00D);
        assert_eq!(bus.read_u32(u64::from(frame - 8)).unwrap(), frame);
    }

    /// Intel SDM Vol. 2 "PUSH"/"POP" (r/m and +rd forms) on a 32-bit stack.
    #[test]
    fn stack_b1_push_pop_rm_and_reg_forms_use_esp() {
        // FF 35 id = PUSH dword [disp32]; 8F 05 id = POP dword [disp32].
        let mut code = vec![0xFF, 0x35];
        code.extend_from_slice(&(PM32_DATA as u32).to_le_bytes());
        code.extend_from_slice(&[0x8F, 0x05]);
        code.extend_from_slice(&((PM32_DATA + 4) as u32).to_le_bytes());
        code.push(0x53); // PUSH EBX
        code.push(0x5A); // POP EDX

        let (mut cpu, mut bus) = pm32_big_stack_fixture(&code, PM32_CODE, PM32_TEST_ESP);
        bus.mem[PM32_DATA..PM32_DATA + 4].copy_from_slice(&0xCAFE_BABEu32.to_le_bytes());
        cpu.set_gpr_u32(CpuState::RBX, 0x0F0F_0F0F);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 4);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
        assert_eq!(bus.read_u32((PM32_DATA + 4) as u64).unwrap(), 0xCAFE_BABE);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 4);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0x0F0F_0F0F);
    }

    /// Intel SDM Vol. 2 "PUSH"/"POP": a `0x66` operand-size override on a
    /// 32-bit stack pushes a word while the pointer still steps through ESP.
    #[test]
    fn stack_b1_word_push_pop_steps_esp_by_two() {
        // 66 50 = PUSH AX; 66 5B = POP BX.
        let (mut cpu, mut bus) =
            pm32_big_stack_fixture(&[0x66, 0x50, 0x66, 0x5B], PM32_CODE, PM32_TEST_ESP);
        cpu.set_gpr_u32(CpuState::RAX, 0x1111_2222);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 2);
        assert_eq!(bus.read_u16(u64::from(PM32_TEST_ESP - 2)).unwrap(), 0x2222);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x2222);
    }

    const PM32_IDT: usize = 0x5000;
    const PM32_HANDLER: u32 = 0x0002_2000;
    /// 386 32-bit interrupt gate, present, DPL 0 (SDM Vol. 3 Figure 6-2).
    const PM32_INTERRUPT_GATE32: u8 = 0x8E;
    /// 386 32-bit trap gate, present, DPL 0.
    const PM32_TRAP_GATE32: u8 = 0x8F;

    /// 386 IDT gate descriptor: offset 15:0, selector, 0, access, offset 31:16.
    /// Spec: Intel SDM Vol. 3 §6.11 (Figure 6-2).
    fn encode_idt_gate32(offset: u32, selector: u16, access: u8) -> [u8; 8] {
        let low = (offset as u16).to_le_bytes();
        let high = ((offset >> 16) as u16).to_le_bytes();
        let selector = selector.to_le_bytes();
        [
            low[0],
            low[1],
            selector[0],
            selector[1],
            0,
            access,
            high[0],
            high[1],
        ]
    }

    /// `CS.D=1` fixture with a 386 IDT gate for `vector` and a same-CPL
    /// handler code segment. `big_stack` selects `SS.B=1` or `SS.B=0`.
    ///
    /// Spec: Intel SDM Vol. 3 §§6.11, 6.12.1.
    fn pm32_gate_fixture(
        code: &[u8],
        vector: u8,
        gate_access: u8,
        cpl: u8,
        big_stack: bool,
    ) -> (CpuState, VecBus) {
        let (mut cpu, mut bus) = pm32_fixture(code, PM32_CODE, true);
        let code_access = 0x9A | (cpl << 5);
        let data_access = 0x92 | (cpl << 5);
        bus.mem[PM32_GDT + 8..PM32_GDT + 16].copy_from_slice(&encode_seg_desc(
            0,
            0xF_FFFF,
            code_access,
            0xC0,
        ));
        bus.mem[PM32_GDT + 24..PM32_GDT + 32].copy_from_slice(&encode_seg_desc(
            0,
            0xFFFF,
            data_access,
            0,
        ));
        bus.mem[PM32_GDT + 32..PM32_GDT + 40].copy_from_slice(&encode_seg_desc(
            0,
            0xF_FFFF,
            data_access,
            0xC0,
        ));
        cpu.gdtr.limit = 39;

        let entry = PM32_IDT + usize::from(vector) * 8;
        bus.mem[entry..entry + 8].copy_from_slice(&encode_idt_gate32(
            PM32_HANDLER,
            PM32_CS32 | u16::from(cpl),
            gate_access,
        ));
        cpu.idtr.base = PM32_IDT as u64;
        cpu.idtr.limit = 0x07FF;

        cpu.cs = x86_core::SegmentReg {
            selector: PM32_CS32 | u16::from(cpl),
            base: 0,
            limit: 0xFFFF_FFFF,
            flags: 0xC000 | u16::from(code_access),
        };
        let stack = if big_stack {
            x86_core::SegmentReg {
                selector: PM32_SS32 | u16::from(cpl),
                base: 0,
                limit: 0xFFFF_FFFF,
                flags: 0xC000 | u16::from(data_access),
            }
        } else {
            x86_core::SegmentReg {
                selector: PM32_DS | u16::from(cpl),
                base: 0,
                limit: 0xFFFF,
                flags: u16::from(data_access),
            }
        };
        cpu.ss = stack;
        if big_stack {
            cpu.set_gpr_u32(CpuState::RSP, PM32_TEST_ESP);
        } else {
            cpu.set_gpr_u32(CpuState::RSP, 0x0000_FFF0);
        }
        (cpu, bus)
    }

    /// Intel SDM Vol. 3 §6.12.1 (Figure 6-4) and Vol. 2 "INT n": a same-CPL
    /// 386 interrupt gate builds a 32-bit EFLAGS/CS/EIP frame, takes EIP from
    /// the gate offset high and low words, and clears IF.
    #[test]
    fn protected_interrupt_gate32_delivers_32_bit_frame_and_clears_if() {
        // CC = INT3.
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32, 0, true);
        cpu.rflags = 0x0000_0000_0001_0A57 | (1 << 9);
        let saved_flags = cpu.rflags as u32;

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.rip, u64::from(PM32_HANDLER));
        assert_eq!(cpu.cs.selector, PM32_CS32);
        assert!(cpu.cs.default_big());
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 12);
        let sp = u64::from(PM32_TEST_ESP - 12);
        assert_eq!(bus.read_u32(sp).unwrap(), (PM32_CODE + 1) as u32);
        assert_eq!(bus.read_u32(sp + 4).unwrap(), u32::from(PM32_CS32));
        assert_eq!(bus.read_u32(sp + 8).unwrap(), saved_flags);
        assert!(!cpu.interrupt_flag(), "interrupt gate must clear IF");
    }

    /// Intel SDM Vol. 3 §6.12.1: a trap gate leaves IF unchanged. Both gate
    /// types clear TF, NT, RF, and VM.
    #[test]
    fn protected_trap_gate32_preserves_if() {
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, PM32_TRAP_GATE32, 0, true);
        cpu.rflags = 0x0000_0000_0001_4102 | (1 << 9) | (1 << 8);

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.rip, u64::from(PM32_HANDLER));
        assert!(cpu.interrupt_flag(), "trap gate must preserve IF");
        assert_eq!(cpu.rflags & (1 << 8), 0, "TF must be cleared");
        assert_eq!(cpu.rflags & (1 << 14), 0, "NT must be cleared");
        assert_eq!(cpu.rflags & (1 << 16), 0, "RF must be cleared");
    }

    /// Intel SDM Vol. 3 §§6.12.1, 6.13: an exception with an error code pushes
    /// it as a doubleword below the 32-bit EFLAGS/CS/EIP frame, and the saved
    /// EIP is the faulting instruction.
    #[test]
    fn protected_gate32_pushes_doubleword_error_code() {
        // 8E D8 = MOV DS, AX with a selector past the GDT limit → #GP(sel).
        let (mut cpu, mut bus) =
            pm32_gate_fixture(&[0x8E, 0xD8], 13, PM32_INTERRUPT_GATE32, 0, true);
        cpu.set_gpr_u16(CpuState::RAX, 0x0084);

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.rip, u64::from(PM32_HANDLER));
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 16);
        let sp = u64::from(PM32_TEST_ESP - 16);
        assert_eq!(bus.read_u32(sp).unwrap(), 0x0084);
        assert_eq!(bus.read_u32(sp + 4).unwrap(), PM32_CODE as u32);
        assert_eq!(bus.read_u32(sp + 8).unwrap(), u32::from(PM32_CS32));
    }

    /// Intel SDM Vol. 1 §6.2.2: the gate type selects the frame width, the
    /// cached `SS.B` bit selects the stack-pointer width. A 386 gate on a
    /// `B=0` stack pushes doublewords through SP.
    #[test]
    fn protected_gate32_frame_width_is_independent_of_stack_width() {
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32, 0, false);

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFE4);
        assert_eq!(bus.read_u32(0xFFE4).unwrap(), (PM32_CODE + 1) as u32);
        assert_eq!(cpu.rip, u64::from(PM32_HANDLER));
    }

    /// Intel SDM Vol. 3 §6.12.1: a 386 gate taken from a `D=0` code segment
    /// still builds a 32-bit frame, so 16-bit and 32-bit gates coexist.
    #[test]
    fn protected_gate32_from_16_bit_code_still_builds_a_32_bit_frame() {
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32, 0, true);
        cpu.cs = x86_core::SegmentReg {
            selector: PM32_CS16,
            base: 0,
            limit: 0xFFFF,
            flags: 0x009A,
        };
        cpu.rflags = 0x0000_0000_0000_0202;

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.rip, u64::from(PM32_HANDLER));
        assert!(cpu.cs.default_big(), "handler runs in the D=1 segment");
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 12);
        let sp = u64::from(PM32_TEST_ESP - 12);
        assert_eq!(bus.read_u32(sp).unwrap(), (PM32_CODE + 1) as u32);
        assert_eq!(bus.read_u32(sp + 4).unwrap(), u32::from(PM32_CS16));
        assert_eq!(bus.read_u32(sp + 8).unwrap(), 0x0202);
    }

    /// Intel SDM Vol. 3 §§6.11, 6.12.1: 32-bit gates coexist with the 16-bit
    /// gate types, and other descriptor types (task gate, data, LDT) are
    /// rejected deterministically instead of synthesizing a nested fault.
    #[test]
    fn protected_gate32_rejects_unsupported_gate_types() {
        for access in [0x85u8, 0x89, 0x82, 0x8C, 0x00] {
            let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, access, 0, true);
            let before = cpu.clone();
            let error = step(&mut cpu, &mut bus).expect_err("unsupported gate type");
            assert!(
                matches!(
                    error,
                    ExecError::ProtectedModeExceptionDelivery {
                        vector: 3,
                        reason: ProtectedModeDeliveryError::GateType(_)
                            | ProtectedModeDeliveryError::GateNotPresent,
                    }
                ),
                "access {access:#04x}: {error:?}"
            );
            assert_eq!(cpu, before, "access {access:#04x}: state committed");
        }
    }

    /// Intel SDM Vol. 3 §6.11.2 / §6.12.1.2: gate DPL is checked for software
    /// `INT n` / `INT3` / `INTO` and raises `#GP(vector*8 + IDT)` without
    /// touching the stack; hardware sources (NMI, external IRQ) bypass it.
    #[test]
    fn protected_gate32_dpl_applies_to_software_interrupts_only() {
        // CPL 3 with a DPL 0 gate: the software form is rejected.
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32, 3, true);
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some((3 << 3) | 2));
        assert_eq!(cpu, before, "rejected software INT touched state");

        // Raising the gate DPL to 3 lets the same instruction through.
        let (mut cpu, mut bus) =
            pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32 | 0x60, 3, true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, u64::from(PM32_HANDLER));
        assert_eq!(cpu.cs.selector, PM32_CS32 | 3);

        // NMI ignores the DPL 0 gate at CPL 3.
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0x90], 2, PM32_INTERRUPT_GATE32, 3, true);
        cpu.request_nmi();
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, u64::from(PM32_HANDLER));
        assert_eq!(
            bus.read_u32(u64::from(PM32_TEST_ESP - 12)).unwrap(),
            PM32_CODE as u32
        );

        // External IRQ likewise ignores the DPL 0 gate at CPL 3.
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0x90], 0x21, PM32_INTERRUPT_GATE32, 3, true);
        cpu.set_interrupt_flag(true);
        cpu.request_interrupt(0x21);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, u64::from(PM32_HANDLER));
        assert!(!cpu.interrupt_flag());
    }

    /// Intel SDM Vol. 3 §6.12.1: a 386 gate may target a `D=1` or a `D=0`
    /// code segment; `L=1` remains unsupported. A 16-bit gate still requires a
    /// `D=0` target and a `D=0` current code segment.
    #[test]
    fn protected_gate32_target_code_segment_d_bit_rules() {
        // A 32-bit gate into a D=0 handler switches the execution window back.
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32, 0, true);
        bus.mem[PM32_GDT + 8..PM32_GDT + 16].copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x9A, 0));
        let entry = PM32_IDT + 3 * 8;
        bus.mem[entry..entry + 8].copy_from_slice(&encode_idt_gate32(
            0x0000_0800,
            PM32_CS32,
            PM32_INTERRUPT_GATE32,
        ));
        step(&mut cpu, &mut bus).unwrap();
        assert!(!cpu.cs.default_big());
        assert_eq!(cpu.ip16(), 0x0800);

        // L=1 targets are rejected.
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32, 0, true);
        bus.mem[PM32_GDT + 8..PM32_GDT + 16]
            .copy_from_slice(&encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xA0));
        let before = cpu.clone();
        assert_eq!(
            step(&mut cpu, &mut bus),
            Err(ExecError::ProtectedModeExceptionDelivery {
                vector: 3,
                reason: ProtectedModeDeliveryError::TargetLongMode,
            })
        );
        assert_eq!(cpu, before);
    }

    /// Intel SDM Vol. 3 §6.12.1: a 386 gate whose 32-bit offset is beyond the
    /// target code-segment limit is rejected before any stack write.
    #[test]
    fn protected_gate32_offset_beyond_target_limit_is_rejected() {
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32, 0, true);
        bus.mem[PM32_GDT + 8..PM32_GDT + 16].copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x9A, 0));
        let before = cpu.clone();

        assert_eq!(
            step(&mut cpu, &mut bus),
            Err(ExecError::ProtectedModeExceptionDelivery {
                vector: 3,
                reason: ProtectedModeDeliveryError::TargetOffsetLimit,
            })
        );
        assert_eq!(cpu, before);
    }

    /// The 32-bit frame is committed atomically: a stack write failure rolls
    /// back every byte and leaves CS:EIP, ESP, and EFLAGS untouched.
    /// Spec: Intel SDM Vol. 3 §6.12.1 (nested #DF/triple fault not modeled).
    #[test]
    fn protected_gate32_stack_write_failure_rolls_back() {
        let (cpu_template, bus_template) =
            pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32, 0, true);
        // The last frame byte written is the high byte of the saved EIP.
        let fail_addr = u64::from(PM32_TEST_ESP - 12 + 3);
        let mut cpu = cpu_template.clone();
        let mut bus = FailOnceWriteBus {
            mem: bus_template.mem.clone(),
            fail_addr,
            failed: false,
        };
        let before = cpu.clone();

        let error = step(&mut cpu, &mut bus).expect_err("stack write must fail");
        assert!(
            matches!(
                error,
                ExecError::ProtectedModeExceptionDelivery {
                    vector: 3,
                    reason: ProtectedModeDeliveryError::StackWrite(_),
                }
            ),
            "{error:?}"
        );
        assert_eq!(cpu, before);
        assert_eq!(
            bus.mem, bus_template.mem,
            "frame bytes were not rolled back"
        );
    }

    /// `IRETD` fixture: a 32-bit frame at `PM32_TEST_ESP - 12` and a return
    /// code descriptor written at `return_selector`'s GDT index.
    ///
    /// Spec: Intel SDM Vol. 2 "IRET/IRETD" (Protected Mode, same privilege).
    fn pm32_iretd_fixture(
        return_eip: u32,
        return_selector: u16,
        return_eflags: u32,
        descriptor: [u8; 8],
    ) -> (CpuState, VecBus) {
        // CF = IRET; under CS.D=1 the default operand size makes it IRETD.
        let (mut cpu, mut bus) = pm32_fixture(&[0xCF], PM32_CODE, true);
        let index = usize::from(return_selector >> 3);
        bus.mem[PM32_GDT + index * 8..PM32_GDT + index * 8 + 8].copy_from_slice(&descriptor);
        cpu.gdtr.limit = 0x00FF;
        cpu.ss = x86_core::SegmentReg {
            selector: PM32_SS32,
            base: 0,
            limit: 0xFFFF_FFFF,
            flags: 0xC092,
        };
        let frame = (PM32_TEST_ESP - 12) as usize;
        bus.mem[frame..frame + 4].copy_from_slice(&return_eip.to_le_bytes());
        bus.mem[frame + 4..frame + 8].copy_from_slice(&u32::from(return_selector).to_le_bytes());
        bus.mem[frame + 8..frame + 12].copy_from_slice(&return_eflags.to_le_bytes());
        cpu.set_gpr_u32(CpuState::RSP, PM32_TEST_ESP - 12);
        (cpu, bus)
    }

    /// Intel SDM Vol. 2 "IRET/IRETD" (Operation, same-privilege return);
    /// Vol. 3 §§3.4.5, 6.12.1: a 32-bit frame restores EIP, CS with its full
    /// cached attributes, EFLAGS, and steps ESP by 12.
    #[test]
    fn protected_iretd_restores_32_bit_frame_and_cs_cache() {
        // Return CS: ring-0, nonconforming, present, G=1, D=1, AVL=1.
        let descriptor = encode_seg_desc(0x0000_1000, 0xF_FFFF, 0x9A, 0xD0);
        let (mut cpu, mut bus) =
            pm32_iretd_fixture(0x0002_1234, PM32_CS32, 0x0000_0A57, descriptor);

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.rip, 0x0002_1234);
        assert_eq!(cpu.cs.selector, PM32_CS32);
        assert_eq!(cpu.cs.base, 0x0000_1000);
        assert_eq!(cpu.cs.limit, 0xFFFF_FFFF);
        assert_eq!(cpu.cs.flags, 0xD09A);
        assert!(cpu.cs.default_big());
        assert_eq!(cpu.rflags, 0x0000_0A57);
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
    }

    /// Intel SDM Vol. 1 §3.4.3; Vol. 2 "IRET/IRETD": at CPL 0 the 32-bit frame
    /// restores the defined EFLAGS bits (through ID). Reserved bits 3, 5, and
    /// 15 stay clear, bit 1 stays set, and `RFLAGS[63:32]` is unchanged.
    #[test]
    fn protected_iretd_restores_defined_eflags_only() {
        let descriptor = encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0);
        let (mut cpu, mut bus) =
            pm32_iretd_fixture(0x0000_2000, PM32_CS32, !(1u32 << 17), descriptor);
        cpu.rflags = 0xDEAD_BEEF_0000_0002;

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.rflags >> 32, 0xDEAD_BEEF);
        assert_eq!(cpu.rflags as u32, 0x003D_7FD7);
    }

    /// Intel SDM Vol. 2 "IRET/IRETD": a `0x66` override on `CS.D=1` code takes
    /// the 16-bit frame, clears `EIP[31:16]`, steps ESP by 6, and leaves
    /// `EFLAGS[31:16]` untouched.
    #[test]
    fn protected_iret16_on_a_32_bit_stack_uses_a_six_byte_frame() {
        let descriptor = encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0);
        let (mut cpu, mut bus) = pm32_iretd_fixture(0, PM32_CS32, 0, descriptor);
        bus.mem[PM32_CODE..PM32_CODE + 2].copy_from_slice(&[0x66, 0xCF]);
        let frame = (PM32_TEST_ESP - 6) as usize;
        bus.mem[frame..frame + 2].copy_from_slice(&0x2468u16.to_le_bytes());
        bus.mem[frame + 2..frame + 4].copy_from_slice(&PM32_CS32.to_le_bytes());
        bus.mem[frame + 4..frame + 6].copy_from_slice(&0x0A57u16.to_le_bytes());
        cpu.set_gpr_u32(CpuState::RSP, PM32_TEST_ESP - 6);
        cpu.rflags = 0x0000_0000_00A5_0002;

        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.rip, 0x2468);
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
        assert_eq!(cpu.rflags, 0x0000_0000_00A5_0A57);
    }

    /// Intel SDM Vol. 3 §6.12.1: entering a 386 interrupt gate and returning
    /// with `IRETD` restores the exact architectural state, including `IF`.
    #[test]
    fn protected_iretd_round_trips_a_32_bit_interrupt_gate() {
        let (mut cpu, mut bus) = pm32_gate_fixture(&[0xCC], 3, PM32_INTERRUPT_GATE32, 0, true);
        bus.mem[PM32_HANDLER as usize] = 0xCF;
        cpu.rflags = 0x0000_0000_0000_0A57 | (1 << 9);
        let before = cpu.clone();

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, u64::from(PM32_HANDLER));
        assert!(!cpu.interrupt_flag());

        step(&mut cpu, &mut bus).unwrap();
        let mut expected = before.clone();
        // INT3 saves the address of the following instruction.
        expected.rip = (PM32_CODE + 1) as u64;
        assert_eq!(cpu, expected);
    }

    /// Intel SDM Vol. 2 "IRET/IRETD" (Protected Mode Exceptions): every frame
    /// and descriptor check happens before any architectural commit.
    #[test]
    fn protected_iretd_validation_faults_are_atomic() {
        /// name, return selector, descriptor, return EIP, vector, error code.
        type IretdFaultCase = (&'static str, u16, [u8; 8], u32, u8, u16);

        let valid = encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0);
        let cases: [IretdFaultCase; 8] = [
            ("null selector", 0x0000, valid, 0x2000, 13, 0x0000),
            ("LDT selector", 0x000C, valid, 0x2000, 13, 0x000C),
            (
                "data descriptor",
                PM32_CS32,
                encode_seg_desc(0, 0xF_FFFF, 0x92, 0xC0),
                0x2000,
                13,
                PM32_CS32,
            ),
            (
                "conforming code",
                PM32_CS32,
                encode_seg_desc(0, 0xF_FFFF, 0x9E, 0xC0),
                0x2000,
                13,
                PM32_CS32,
            ),
            ("outer RPL", 0x000B, valid, 0x2000, 13, 0x0008),
            (
                "ring 1 DPL",
                PM32_CS32,
                encode_seg_desc(0, 0xF_FFFF, 0xBA, 0xC0),
                0x2000,
                13,
                PM32_CS32,
            ),
            (
                "not present",
                PM32_CS32,
                encode_seg_desc(0, 0xF_FFFF, 0x1A, 0xC0),
                0x2000,
                11,
                PM32_CS32,
            ),
            (
                "long-mode code",
                PM32_CS32,
                encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xE0),
                0x2000,
                13,
                PM32_CS32,
            ),
        ];

        for (name, selector, descriptor, eip, vector, error_code) in cases {
            let (mut cpu, mut bus) = pm32_iretd_fixture(eip, selector, 0x0002, descriptor);
            let before = cpu.clone();
            assert_arch_fault(step_inner(&mut cpu, &mut bus), vector, Some(error_code));
            assert_eq!(
                cpu, before,
                "{name}: IRETD committed state before the fault"
            );
        }

        // A 32-bit EIP beyond the return segment limit is #GP(0).
        let (mut cpu, mut bus) = pm32_iretd_fixture(
            0x0001_0000,
            PM32_CS32,
            0x0002,
            encode_seg_desc(0, 0xFFFF, 0x9A, 0x40),
        );
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before, "EIP past limit committed state");
    }

    /// Returning to virtual-8086 mode is covered by `cpu_r9_vm86_enter`.
    /// A 3-dword frame with `VM=1` is incomplete (needs ESP/SS/ES/DS/FS/GS) and
    /// must not silently ignore the VM bit. Spec: Intel SDM Vol. 2 "IRET/IRETD"
    /// RETURN-TO-VIRTUAL-8086-MODE; Vol. 3 §20.2.
    #[test]
    fn protected_iretd_vm86_image_requires_nine_dword_frame() {
        let descriptor = encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0);
        // Only three dwords are on the stack; slots 3..8 read as zero and would
        // form a vacuous VM86 state — ensure ESP advances past a full 9-dword
        // frame when VM is set (36 bytes), not the protected 3-dword path.
        let (mut cpu, mut bus) = pm32_iretd_fixture(0x0100, 0x1000, 0x0002 | (1 << 17), descriptor);
        // Extend the stack image with the remaining six dwords of a VM86 frame.
        let frame = (PM32_TEST_ESP - 36) as usize;
        let eip = 0x0100u32.to_le_bytes();
        let cs = 0x1000u32.to_le_bytes();
        let flags = (0x0002u32 | (1 << 17)).to_le_bytes();
        bus.mem[frame..frame + 4].copy_from_slice(&eip);
        bus.mem[frame + 4..frame + 8].copy_from_slice(&cs);
        bus.mem[frame + 8..frame + 12].copy_from_slice(&flags);
        for i in 0..6 {
            bus.mem[frame + 12 + i * 4..frame + 16 + i * 4]
                .copy_from_slice(&(0x2000u32 + i as u32 * 0x1000).to_le_bytes());
        }
        // SS/ESP for VM86: selector 0x2000, SP 0xFFFE (encoded in first extra dword).
        bus.mem[frame + 12..frame + 16].copy_from_slice(&0xFFFEu32.to_le_bytes());
        bus.mem[frame + 16..frame + 20].copy_from_slice(&0x2000u32.to_le_bytes());
        cpu.set_gpr_u32(CpuState::RSP, PM32_TEST_ESP - 36);

        step(&mut cpu, &mut bus).unwrap();
        assert_ne!(cpu.rflags & (1 << 17), 0);
        assert_eq!(cpu.cs.selector, 0x1000);
        assert_eq!(cpu.cs.base, 0x1000 << 4);
        assert_eq!(cpu.rip, 0x0100);
    }

    /// A stack-limit fault during the 32-bit frame load leaves the CPU
    /// untouched. Spec: Intel SDM Vol. 2 "IRET/IRETD" (Protected Mode
    /// Exceptions); Vol. 3 §5.3.
    #[test]
    fn protected_iretd_frame_reads_are_atomic() {
        let descriptor = encode_seg_desc(0, 0xF_FFFF, 0x9A, 0xC0);
        let (mut cpu, mut bus) = pm32_iretd_fixture(0x2000, PM32_CS32, 0x0002, descriptor);
        // Only the first two frame doublewords stay inside the stack limit.
        cpu.ss.limit = PM32_TEST_ESP - 5;
        let before = cpu.clone();

        assert_arch_fault(step_inner(&mut cpu, &mut bus), 12, Some(0));
        assert_eq!(cpu, before);
    }

    // ----------------------------------------------------------------------
    // Milestone 2 round 2 — two-byte `0F` map (`docs/cpu-0f-map.md`).
    // ----------------------------------------------------------------------

    /// Execute one real-mode instruction from `code` at `CS:0000` with the
    /// given `EFLAGS`, returning the CPU and bus afterwards.
    fn run_real_mode_once(code: &[u8], flags: u64) -> (CpuState, VecBus) {
        let mut mem = vec![0u8; 0x10000];
        mem[..code.len()].copy_from_slice(code);
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.rflags = flags;
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        (cpu, bus)
    }

    /// The 32 architecturally meaningful `EFLAGS` combinations of CF/PF/ZF/SF/OF,
    /// with bit 1 reserved-one set. Spec: Intel SDM Vol. 1 §3.4.3.
    fn condition_flag_combinations() -> impl Iterator<Item = u64> {
        (0u64..32).map(|bits| {
            0x0002
                | (bits & 1)
                | ((bits >> 1) & 1) << 2
                | ((bits >> 2) & 1) << 6
                | ((bits >> 3) & 1) << 7
                | ((bits >> 4) & 1) << 11
        })
    }

    /// Intel SDM Vol. 2 "Jcc" and Appendix B (condition-code encodings): the
    /// near `0F 80`+cc map selects exactly the same condition as the
    /// already-validated short `70`+cc form for every condition code and every
    /// CF/PF/ZF/SF/OF combination, and neither form writes flags.
    #[test]
    fn jcc_near_condition_matches_short_form_for_every_flag_combination() {
        for cc in 0u8..16 {
            for flags in condition_flag_combinations() {
                // 70+cc 10 — short form, +0x10 from the 2-byte next IP.
                let (short_cpu, _) = run_real_mode_once(&[0x70 | cc, 0x10], flags);
                let taken = match short_cpu.ip16() {
                    0x12 => true,
                    0x02 => false,
                    other => panic!("cc {cc:#x} flags {flags:#x}: short IP {other:#06X}"),
                };

                // 0F 80+cc 10 00 — near rel16, +0x10 from the 4-byte next IP.
                let (near_cpu, _) = run_real_mode_once(&[0x0F, 0x80 | cc, 0x10, 0x00], flags);
                assert_eq!(
                    near_cpu.ip16(),
                    if taken { 0x14 } else { 0x04 },
                    "cc {cc:#x} flags {flags:#x}"
                );
                assert_eq!(near_cpu.rflags, flags, "Jcc must not write flags");
            }
        }
    }

    /// Intel SDM Vol. 2 "Jcc" (Operation): a negative rel16 wraps inside the
    /// 16-bit `IP` window under a `D=0` code segment.
    #[test]
    fn jcc_near_rel16_backward_displacement_wraps_in_16_bit_window() {
        let mut mem = vec![0u8; 0x10000];
        // 0F 85 8E F9 = JNZ -1650 — the SeaBIOS reset-vector branch shape.
        mem[0x1000..0x1004].copy_from_slice(&[0x0F, 0x85, 0x8E, 0xF9]);
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0x1000;
        cpu.set_zf(false);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ip16(), 0x1004u16.wrapping_sub(1650));

        // Taken-with-wrap below zero stays inside the 16-bit window.
        let (cpu, _) = run_real_mode_once(&[0x0F, 0x85, 0xF0, 0xFF], 0x0002);
        assert_eq!(cpu.ip16(), 0xFFF4);
    }

    /// Intel SDM Vol. 2 "Jcc" (Operation): "If the operand-size attribute is
    /// 16, the upper two bytes of the EIP register are cleared." Under `CS.D=1`
    /// a `0x66`-prefixed near Jcc must therefore drop `EIP[31:16]`.
    #[test]
    fn jcc_near_rel16_clears_eip_high_bits_under_default32() {
        // 66 0F 85 00 00 = JNZ rel16 +0 (5 bytes) at EIP 0x0002_1000.
        let (mut cpu, mut bus) =
            pm32_fixture(&[0x66, 0x0F, 0x85, 0x00, 0x00], PM32_HIGH_CODE, true);
        cpu.set_zf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, u64::from((PM32_HIGH_CODE as u32 + 5) & 0xFFFF));

        // Not taken: the sequential next EIP keeps its high bits.
        let (mut cpu, mut bus) =
            pm32_fixture(&[0x66, 0x0F, 0x85, 0x00, 0x00], PM32_HIGH_CODE, true);
        cpu.set_zf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, (PM32_HIGH_CODE + 5) as u64);
    }

    /// Intel SDM Vol. 2 "Jcc" (near, rel32): under `CS.D=1` the default
    /// displacement is 32 bits and reaches targets outside the 16-bit window.
    #[test]
    fn jcc_near_rel32_reaches_beyond_16_bit_window_under_default32() {
        // 0F 84 cd = JZ rel32 (6 bytes) from PM32_CODE to PM32_HIGH_CODE.
        let disp = (PM32_HIGH_CODE as i32) - (PM32_CODE as i32 + 6);
        let mut code = vec![0x0F, 0x84];
        code.extend_from_slice(&disp.to_le_bytes());

        let (mut cpu, mut bus) = pm32_fixture(&code, PM32_CODE, true);
        cpu.set_zf(true);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, PM32_HIGH_CODE as u64);

        let (mut cpu, mut bus) = pm32_fixture(&code, PM32_CODE, true);
        cpu.set_zf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rip, (PM32_CODE + 6) as u64);
    }

    /// Intel SDM Vol. 2 "SETcc": `DEST := 1` when the condition holds and `0`
    /// otherwise, for every condition code and flag combination, with the
    /// condition agreeing with the short `Jcc` form and no flags written.
    #[test]
    fn setcc_condition_matches_short_jcc_for_every_flag_combination() {
        for cc in 0u8..16 {
            for flags in condition_flag_combinations() {
                let (short_cpu, _) = run_real_mode_once(&[0x70 | cc, 0x10], flags);
                let taken = short_cpu.ip16() == 0x12;

                // 0F 90+cc C0 = SETcc AL.
                let (cpu, _) = run_real_mode_once(&[0x0F, 0x90 | cc, 0xC0], flags);
                assert_eq!(cpu.al(), u8::from(taken), "cc {cc:#x} flags {flags:#x}");
                assert_eq!(cpu.rflags, flags, "SETcc must not write flags");
                assert_eq!(cpu.ip16(), 3);
            }
        }
    }

    /// Intel SDM Vol. 2 "SETcc": the byte destination may be any legacy 8-bit
    /// register, including the high-byte encodings AH/CH/DH/BH, and only that
    /// byte changes.
    #[test]
    fn setcc_writes_legacy_low_and_high_byte_registers() {
        // 0F 94 C7 = SETE BH (mod=11, rm=7).
        let mut mem = vec![0u8; 0x10000];
        mem[..3].copy_from_slice(&[0x0F, 0x94, 0xC7]);
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.gpr[CpuState::RBX] = 0x1111_2222_3333_4455;
        cpu.set_zf(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr[CpuState::RBX], 0x1111_2222_3333_0155);

        // 0F 95 C1 = SETNE CL (mod=11, rm=1) — low byte only.
        let mut mem = vec![0u8; 0x10000];
        mem[..3].copy_from_slice(&[0x0F, 0x95, 0xC1]);
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.rip = 0;
        cpu.gpr[CpuState::RCX] = 0xAAAA_BBBB_CCCC_DDEE;
        cpu.set_zf(true);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr[CpuState::RCX], 0xAAAA_BBBB_CCCC_DD00);
    }

    /// Intel SDM Vol. 2 "SETcc": the memory form writes exactly one byte at the
    /// effective address, in both the 16-bit and the `D=1` 32-bit addressing
    /// modes.
    #[test]
    fn setcc_memory_form_writes_one_byte() {
        // 0F 95 06 00 40 = SETNE byte [0x4000] (16-bit addressing).
        let mut mem = vec![0u8; 0x10000];
        mem[..5].copy_from_slice(&[0x0F, 0x95, 0x06, 0x00, 0x40]);
        mem[0x4000] = 0xAA;
        mem[0x4001] = 0xBB;
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_zf(false);
        let mut bus = VecBus { mem, ports: vec![] };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x4000).unwrap(), 1);
        assert_eq!(bus.read_u8(0x4001).unwrap(), 0xBB);
        assert_eq!(cpu.ip16(), 5);

        // 0F 94 05 disp32 = SETE byte [disp32] under CS.D=1.
        let mut code = vec![0x0F, 0x94, 0x05];
        code.extend_from_slice(&(PM32_DATA as u32).to_le_bytes());
        let (mut cpu, mut bus) = pm32_fixture(&code, PM32_CODE, true);
        cpu.set_zf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(PM32_DATA as u64).unwrap(), 0);
        assert_eq!(cpu.rip, (PM32_CODE + 7) as u64);
    }

    /// Intel SDM Vol. 2 "SHLD"/"SHRD" (Operation): the destination receives
    /// `COUNT` bits shifted in from `ModR/M.reg`, and `CF` is the last bit
    /// shifted out of the destination. The `imm8` and `CL` count forms must
    /// agree, and the bit source is never modified.
    #[test]
    fn shld_shrd_results_and_carry_at_both_operand_sizes() {
        struct Case {
            imm_op: u8,
            cl_op: u8,
            opsize32: bool,
            dest: u32,
            src: u32,
            count: u8,
            expected: u32,
            carry: bool,
        }
        let cases = [
            Case {
                imm_op: 0xA4,
                cl_op: 0xA5,
                opsize32: false,
                dest: 0x1234,
                src: 0xABCD,
                count: 4,
                expected: 0x234A,
                carry: true,
            },
            Case {
                imm_op: 0xA4,
                cl_op: 0xA5,
                opsize32: true,
                dest: 0x1234_5678,
                src: 0x9ABC_DEF0,
                count: 8,
                expected: 0x3456_789A,
                carry: false,
            },
            Case {
                imm_op: 0xAC,
                cl_op: 0xAD,
                opsize32: false,
                dest: 0x1234,
                src: 0xABCD,
                count: 4,
                expected: 0xD123,
                carry: false,
            },
            Case {
                imm_op: 0xAC,
                cl_op: 0xAD,
                opsize32: true,
                dest: 0x1234_5678,
                src: 0x9ABC_DEF0,
                count: 8,
                expected: 0xF012_3456,
                carry: false,
            },
        ];
        for case in cases {
            // ModR/M D0 = mod 11, reg = (E)DX, rm = (E)AX.
            for use_cl in [false, true] {
                let mut code = Vec::new();
                if case.opsize32 {
                    code.push(0x66);
                }
                if use_cl {
                    code.extend_from_slice(&[0x0F, case.cl_op, 0xD0]);
                } else {
                    code.extend_from_slice(&[0x0F, case.imm_op, 0xD0, case.count]);
                }
                let (mut cpu, mut bus) = real_mode_fixture(&code, |cpu, _| {
                    cpu.set_gpr_u32(CpuState::RAX, case.dest);
                    cpu.set_gpr_u32(CpuState::RDX, case.src);
                    if use_cl {
                        cpu.set_gpr_u8_low(CpuState::RCX, case.count);
                    }
                });
                step(&mut cpu, &mut bus).unwrap();
                let got = if case.opsize32 {
                    cpu.gpr_u32(CpuState::RAX)
                } else {
                    u32::from(cpu.gpr_u16(CpuState::RAX))
                };
                let op = case.imm_op;
                let opsize32 = case.opsize32;
                assert_eq!(
                    got, case.expected,
                    "op {op:#04X} opsize32={opsize32} use_cl={use_cl}"
                );
                assert_eq!(cpu.rflags & 1 != 0, case.carry, "CF op {op:#04X}");
                assert_eq!(
                    cpu.gpr_u32(CpuState::RDX),
                    case.src,
                    "source must not change"
                );
                assert_eq!(cpu.ip16(), code.len() as u16);
            }
        }
    }

    /// Intel SDM Vol. 2 "SHLD"/"SHRD" (Operation): `COUNT := COUNT MOD 32`
    /// outside 64-bit mode, independently of the operand size, and a resulting
    /// count of zero is an explicit no-operation that leaves every flag alone.
    #[test]
    fn shld_shrd_count_is_masked_modulo_32_and_zero_is_a_no_op() {
        // 0F A4 D0 24 = SHLD AX, DX, 0x24; 0x24 MOD 32 = 4.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA4, 0xD0, 0x24], |cpu, _| {
            cpu.set_gpr_u16(CpuState::RAX, 0x1234);
            cpu.set_gpr_u16(CpuState::RDX, 0xABCD);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x234A);

        // A count of 32 masks to zero: no destination change and no flag change.
        for (opcode, count) in [(0xA4u8, 0x20u8), (0xAC, 0x20), (0xA4, 0x00), (0xAC, 0x00)] {
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, opcode, 0xD0, count], |cpu, _| {
                cpu.set_gpr_u32(CpuState::RAX, 0xAAAA_1234);
                cpu.set_gpr_u16(CpuState::RDX, 0xABCD);
                cpu.rflags = 0x0002 | (1 << 0) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
            });
            let flags = cpu.rflags;
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(
                cpu.gpr_u32(CpuState::RAX),
                0xAAAA_1234,
                "0F {opcode:02X} count {count:#04X}"
            );
            assert_eq!(cpu.rflags, flags, "0F {opcode:02X} count {count:#04X}");
        }
    }

    /// Intel SDM Vol. 2 "SHLD"/"SHRD" (Operation): a count greater than the
    /// operand size is "Bad parameters" — the destination and every flag are
    /// architecturally undefined. Reachable only at a 16-bit operand size, since
    /// the modulo-32 mask caps a 32-bit count at 31.
    ///
    /// **Deterministic model choice:** this tree commits nothing, leaving the
    /// destination and all six flags unchanged.
    #[test]
    fn shld_shrd_count_above_the_operand_size_commits_nothing() {
        for opcode in [0xA4u8, 0xAC] {
            for count in [17u8, 31] {
                let (mut cpu, mut bus) =
                    real_mode_fixture(&[0x0F, opcode, 0xD0, count], |cpu, _| {
                        cpu.set_gpr_u32(CpuState::RAX, 0xAAAA_1234);
                        cpu.set_gpr_u16(CpuState::RDX, 0xABCD);
                        cpu.rflags = 0x0002 | (1 << 4) | (1 << 11);
                    });
                let flags = cpu.rflags;
                step(&mut cpu, &mut bus).unwrap();
                assert_eq!(
                    cpu.gpr_u32(CpuState::RAX),
                    0xAAAA_1234,
                    "0F {opcode:02X} count {count}"
                );
                assert_eq!(cpu.rflags, flags, "0F {opcode:02X} count {count}");
                assert_eq!(cpu.ip16(), 4);
            }
        }
    }

    /// Intel SDM Vol. 2 "SHLD"/"SHRD" (Operation): a count *equal* to the
    /// operand size is defined — every destination bit comes from the source,
    /// and `CF` is still the last destination bit shifted out.
    #[test]
    fn shld_shrd_count_equal_to_the_operand_size_replaces_the_destination() {
        // 0F A4 D0 10 = SHLD AX, DX, 16 — CF := BIT[DEST, 0].
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA4, 0xD0, 0x10], |cpu, _| {
            cpu.set_gpr_u16(CpuState::RAX, 0x1235);
            cpu.set_gpr_u16(CpuState::RDX, 0xABCD);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0xABCD);
        assert!(cpu.rflags & 1 != 0, "CF := BIT[DEST, 0] = 1");

        // 0F AC D0 10 = SHRD AX, DX, 16 — CF := BIT[DEST, 15].
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xAC, 0xD0, 0x10], |cpu, _| {
            cpu.set_gpr_u16(CpuState::RAX, 0x8234);
            cpu.set_gpr_u16(CpuState::RDX, 0xABCD);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0xABCD);
        assert!(cpu.rflags & 1 != 0, "CF := BIT[DEST, 15] = 1");
    }

    /// Intel SDM Vol. 2 "SHLD"/"SHRD" — Flags Affected: `SF`/`ZF`/`PF` follow
    /// the result; `OF` is set from a sign change for a 1-bit shift and is
    /// undefined above that.
    ///
    /// **Deterministic model choices:** `OF` is left unchanged for counts above
    /// one, and `AF` — undefined in every case — is left unchanged too. Both
    /// match what the Group 2 shifts already do in this tree.
    #[test]
    fn shld_shrd_flag_results_including_the_undefined_of_and_af() {
        // SHLD AX, DX, 1 with AX = 0x4000 → 0x8000: sign changed, so OF = 1.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA4, 0xD0, 0x01], |cpu, _| {
            cpu.set_gpr_u16(CpuState::RAX, 0x4000);
            cpu.set_gpr_u16(CpuState::RDX, 0x0000);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x8000);
        assert_eq!(cpu.rflags & 1, 0, "CF := BIT[DEST, 15] = 0");
        assert_ne!(cpu.rflags & (1 << 11), 0, "OF set on a 1-bit sign change");
        assert_ne!(cpu.rflags & (1 << 7), 0, "SF from the result");
        assert_eq!(cpu.rflags & (1 << 6), 0, "ZF from the result");
        assert_ne!(cpu.rflags & (1 << 2), 0, "PF from the low byte 0x00");

        // SHLD AX, DX, 1 with AX = 0x8000 → 0x0000: sign changed again, ZF set.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA4, 0xD0, 0x01], |cpu, _| {
            cpu.set_gpr_u16(CpuState::RAX, 0x8000);
            cpu.set_gpr_u16(CpuState::RDX, 0x0000);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x0000);
        assert_ne!(cpu.rflags & 1, 0, "CF := BIT[DEST, 15] = 1");
        assert_ne!(cpu.rflags & (1 << 11), 0, "OF set on a 1-bit sign change");
        assert_ne!(cpu.rflags & (1 << 6), 0, "ZF from the zero result");

        // A count above one leaves the undefined OF exactly as it was, in both
        // directions, and the undefined AF likewise.
        for preset_of in [false, true] {
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA4, 0xD0, 0x02], |cpu, _| {
                cpu.set_gpr_u16(CpuState::RAX, 0x4000);
                cpu.set_gpr_u16(CpuState::RDX, 0x0000);
            });
            cpu.set_of(preset_of);
            cpu.set_af(true);
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.gpr_u16(CpuState::RAX), 0x0000);
            assert_eq!(
                cpu.rflags & (1 << 11) != 0,
                preset_of,
                "OF undefined above 1 bit → unchanged"
            );
            assert_ne!(cpu.rflags & (1 << 4), 0, "AF undefined → unchanged");
        }
    }

    /// Intel SDM Vol. 2 "SHLD"/"SHRD": the destination may be a memory operand
    /// at either operand size, with the bits shifted in from `ModR/M.reg`.
    #[test]
    fn shld_shrd_memory_destination_forms() {
        // 0F A4 1E 00 40 04 = SHLD word [0x4000], BX, 4.
        let (mut cpu, mut bus) =
            real_mode_fixture(&[0x0F, 0xA4, 0x1E, 0x00, 0x40, 0x04], |cpu, mem| {
                mem[0x4000..0x4002].copy_from_slice(&0x1234u16.to_le_bytes());
                cpu.set_gpr_u16(CpuState::RBX, 0xABCD);
            });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x234A);
        assert!(cpu.rflags & 1 != 0);
        assert_eq!(cpu.ip16(), 6);

        // 66 0F AC 1E 00 40 08 = SHRD dword [0x4000], EBX, 8.
        let (mut cpu, mut bus) =
            real_mode_fixture(&[0x66, 0x0F, 0xAC, 0x1E, 0x00, 0x40, 0x08], |cpu, mem| {
                mem[0x4000..0x4004].copy_from_slice(&0x1234_5678u32.to_le_bytes());
                cpu.set_gpr_u32(CpuState::RBX, 0x9ABC_DEF0);
            });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0xF012_3456);
        assert_eq!(cpu.ip16(), 7);
    }

    /// The 64-bit-shift idiom SeaBIOS uses, and the instruction POST stopped on
    /// after slice 1: under `CS.D=1`, `0F AC D0 10` is `SHRD EAX, EDX, 16`.
    /// Spec: Intel SDM Vol. 2 "SHRD"; Vol. 1 §3.6 Table 3-4.
    #[test]
    fn shrd_default32_matches_the_firmware_64_bit_shift_idiom() {
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0xAC, 0xD0, 0x10], PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RAX, 0x1234_5678);
        cpu.set_gpr_u32(CpuState::RDX, 0x9ABC_DEF0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xDEF0_1234);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0x9ABC_DEF0);
        assert_eq!(cpu.rflags & 1, 0, "CF := BIT[DEST, 15] = 0");
        assert_eq!(cpu.rip, (PM32_CODE + 4) as u64);

        // 66 0F AC D0 10 selects the 16-bit operand size again under `D=1`.
        let (mut cpu, mut bus) = pm32_fixture(&[0x66, 0x0F, 0xAC, 0xD0, 0x10], PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RAX, 0x1234_5678);
        cpu.set_gpr_u32(CpuState::RDX, 0x9ABC_DEF0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x1234_DEF0);
    }

    /// Intel SDM Vol. 2 "CMOVcc—Conditional Move": `IF condition THEN
    /// DEST := SRC`. The condition must agree with the short `Jcc` form for
    /// every condition code and every flag combination, and `CMOVcc` writes no
    /// flags.
    #[test]
    fn cmovcc_condition_matches_short_jcc_for_every_flag_combination() {
        for cc in 0u8..16 {
            for flags in condition_flag_combinations() {
                let (short_cpu, _) = run_real_mode_once(&[0x70 | cc, 0x10], flags);
                let taken = short_cpu.ip16() == 0x12;

                // 0F 40+cc C3 = CMOVcc AX, BX (mod=11, reg=AX, rm=BX).
                let mut mem = vec![0u8; 0x10000];
                mem[..3].copy_from_slice(&[0x0F, 0x40 | cc, 0xC3]);
                let mut cpu = CpuState::reset();
                cpu.cs = x86_core::SegmentReg::real_mode_code(0);
                cpu.rip = 0;
                cpu.rflags = flags;
                cpu.set_gpr_u16(CpuState::RAX, 0x1111);
                cpu.set_gpr_u16(CpuState::RBX, 0x2222);
                let mut bus = VecBus { mem, ports: vec![] };
                step(&mut cpu, &mut bus).unwrap();
                assert_eq!(
                    cpu.gpr_u16(CpuState::RAX),
                    if taken { 0x2222 } else { 0x1111 },
                    "cc {cc:#x} flags {flags:#x}"
                );
                assert_eq!(cpu.rflags, flags, "CMOVcc must not write flags");
                assert_eq!(cpu.ip16(), 3);
            }
        }
    }

    /// Intel SDM Vol. 2 "CMOVcc": the destination width follows the operand-size
    /// attribute. A taken 16-bit move leaves the upper half of the 32-bit
    /// destination untouched (Vol. 1 §3.4.1.1), and an untaken move of either
    /// width leaves the whole destination unchanged.
    #[test]
    fn cmovcc_destination_width_follows_operand_size() {
        // (code, ZF, expected EAX) for 0F 44 C3 = CMOVE (E)AX, (E)BX.
        let cases: [(&[u8], bool, u32); 4] = [
            (&[0x0F, 0x44, 0xC3], true, 0x1111_BBBB),
            (&[0x0F, 0x44, 0xC3], false, 0x1111_2222),
            (&[0x66, 0x0F, 0x44, 0xC3], true, 0xAAAA_BBBB),
            (&[0x66, 0x0F, 0x44, 0xC3], false, 0x1111_2222),
        ];
        for (code, zf, expected) in cases {
            let (mut cpu, mut bus) = real_mode_fixture(code, |cpu, _| {
                cpu.set_gpr_u32(CpuState::RAX, 0x1111_2222);
                cpu.set_gpr_u32(CpuState::RBX, 0xAAAA_BBBB);
            });
            cpu.set_zf(zf);
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(
                cpu.gpr_u32(CpuState::RAX),
                expected,
                "zf={zf} code={code:?}"
            );
            assert_eq!(cpu.ip16(), code.len() as u16);
        }
    }

    /// Intel SDM Vol. 2 "CMOVcc": a memory source is read whether or not the
    /// condition holds, so a source the segment limit forbids faults either way.
    /// This model always reads the source before evaluating the condition.
    #[test]
    fn cmovcc_reads_the_memory_source_regardless_of_the_condition() {
        for zf in [true, false] {
            // 0F 44 1E 00 90 = CMOVE BX, word [0x9000], past DS.limit = 0x7FFF.
            let mut mem = vec![0u8; 0x10000];
            mem[..5].copy_from_slice(&[0x0F, 0x44, 0x1E, 0x00, 0x90]);
            // IVT[13] (#GP) → 0000:0D00.
            mem[13 * 4..13 * 4 + 4].copy_from_slice(&0x0000_0D00u32.to_le_bytes());
            mem[0xD00] = 0xF4;
            let mut cpu = CpuState::reset();
            cpu.cs = x86_core::SegmentReg::real_mode_code(0);
            cpu.ds = x86_core::SegmentReg::real_mode(0);
            cpu.ds.limit = 0x7FFF;
            cpu.ss = x86_core::SegmentReg::real_mode(0);
            cpu.rip = 0;
            cpu.set_gpr_u16(CpuState::RSP, 0xFFF0);
            cpu.set_gpr_u16(CpuState::RBX, 0x1234);
            cpu.set_zf(zf);
            let mut bus = VecBus { mem, ports: vec![] };
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 0x0D00, "zf={zf} must still take #GP");
            assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x1234, "no partial commit");
        }
    }

    /// Intel SDM Vol. 2 "CMOVcc": a taken move reads from memory in both 16-
    /// and 32-bit addressing, including under `CS.D=1`.
    #[test]
    fn cmovcc_memory_source_forms() {
        // 66 0F 45 1E 00 40 = CMOVNE EBX, dword [0x4000].
        let (mut cpu, mut bus) =
            real_mode_fixture(&[0x66, 0x0F, 0x45, 0x1E, 0x00, 0x40], |cpu, mem| {
                mem[0x4000..0x4004].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
                cpu.set_gpr_u32(CpuState::RBX, 0);
            });
        cpu.set_zf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0xDEAD_BEEF);

        // 0F 45 1D disp32 = CMOVNE EBX, dword [disp32] under CS.D=1.
        let mut code = vec![0x0F, 0x45, 0x1D];
        code.extend_from_slice(&(PM32_DATA as u32).to_le_bytes());
        let (mut cpu, mut bus) = pm32_fixture(&code, PM32_CODE, true);
        bus.mem[PM32_DATA..PM32_DATA + 4].copy_from_slice(&0x0BAD_F00Du32.to_le_bytes());
        cpu.set_zf(false);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x0BAD_F00D);
        assert_eq!(cpu.rip, (PM32_CODE + 7) as u64);
    }

    /// Build a real-mode single-step fixture and let the caller seed registers
    /// and memory before the instruction runs.
    fn real_mode_fixture<F>(code: &[u8], setup: F) -> (CpuState, VecBus)
    where
        F: FnOnce(&mut CpuState, &mut [u8]),
    {
        let mut mem = vec![0u8; 0x10000];
        mem[..code.len()].copy_from_slice(code);
        let mut cpu = CpuState::reset();
        cpu.cs = x86_core::SegmentReg::real_mode_code(0);
        cpu.ds = x86_core::SegmentReg::real_mode(0);
        cpu.es = x86_core::SegmentReg::real_mode(0);
        cpu.ss = x86_core::SegmentReg::real_mode(0);
        cpu.rip = 0;
        cpu.set_gpr_u16(CpuState::RSP, 0xFFF0);
        setup(&mut cpu, &mut mem);
        (cpu, VecBus { mem, ports: vec![] })
    }

    /// Intel SDM Vol. 2 "MOVZX"/"MOVSX": the opcode fixes the source width and
    /// the operand-size attribute fixes the destination width, giving eight
    /// source/destination combinations. A 16-bit destination leaves the upper
    /// half of the 32-bit register untouched, and no flags are written.
    #[test]
    fn movzx_movsx_cover_every_source_and_destination_width() {
        // ModR/M D8 = mod 11, reg = BX/EBX, rm = AX/EAX.
        // (opcode, opsize-32, seeded EAX, expected EBX)
        let cases: [(u8, bool, u32, u32); 8] = [
            (0xB6, false, 0x1111_1180, 0xAAAA_0080), // MOVZX BX, AL
            (0xB6, true, 0x1111_1180, 0x0000_0080),  // MOVZX EBX, AL
            (0xB7, false, 0x1111_8000, 0xAAAA_8000), // MOVZX BX, AX (plain move)
            (0xB7, true, 0x1111_8000, 0x0000_8000),  // MOVZX EBX, AX
            (0xBE, false, 0x1111_1180, 0xAAAA_FF80), // MOVSX BX, AL
            (0xBE, true, 0x1111_1180, 0xFFFF_FF80),  // MOVSX EBX, AL
            (0xBF, false, 0x1111_8000, 0xAAAA_8000), // MOVSX BX, AX (plain move)
            (0xBF, true, 0x1111_8000, 0xFFFF_8000),  // MOVSX EBX, AX
        ];
        for (opcode, opsize32, eax, expected_ebx) in cases {
            let mut code = Vec::new();
            if opsize32 {
                code.push(0x66);
            }
            code.extend_from_slice(&[0x0F, opcode, 0xD8]);
            let flags = 0x0002 | (1 << 0) | (1 << 6) | (1 << 7) | (1 << 11);
            let (mut cpu, mut bus) = real_mode_fixture(&code, |cpu, _| {
                cpu.set_gpr_u32(CpuState::RAX, eax);
                cpu.set_gpr_u32(CpuState::RBX, 0xAAAA_BBBB);
                cpu.rflags = flags;
            });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(
                cpu.gpr_u32(CpuState::RBX),
                expected_ebx,
                "0F {opcode:02X} opsize32={opsize32}"
            );
            assert_eq!(cpu.rflags, flags, "0F {opcode:02X} must not write flags");
            assert_eq!(cpu.ip16(), code.len() as u16);
        }
    }

    /// Intel SDM Vol. 2 "MOVZX"/"MOVSX": the byte source may be a memory
    /// operand or any legacy 8-bit register, including AH/CH/DH/BH.
    #[test]
    fn movzx_movsx_byte_sources_cover_memory_and_high_byte_registers() {
        // 0F B6 1E 00 40 = MOVZX BX, byte [0x4000]
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xB6, 0x1E, 0x00, 0x40], |_, mem| {
            mem[0x4000] = 0x9C;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x009C);

        // 0F BE 1E 00 40 = MOVSX BX, byte [0x4000]
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xBE, 0x1E, 0x00, 0x40], |_, mem| {
            mem[0x4000] = 0x9C;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xFF9C);

        // 0F BE CC = MOVSX CX, AH (mod 11, reg = CX, rm = 4 = AH).
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xBE, 0xCC], |cpu, _| {
            cpu.gpr[CpuState::RAX] = 0x0000_0000_0000_F011;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0xFFF0);

        // 0F B6 CC = MOVZX CX, AH
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xB6, 0xCC], |cpu, _| {
            cpu.gpr[CpuState::RAX] = 0x0000_0000_0000_F011;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x00F0);

        // 0F B7 1E 00 40 = MOVZX BX, word [0x4000]
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xB7, 0x1E, 0x00, 0x40], |_, mem| {
            mem[0x4000] = 0x34;
            mem[0x4001] = 0x82;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x8234);
    }

    /// Intel SDM Vol. 2 "PUSH"/"POP" (`0F A0`/`A1`/`A8`/`A9`): FS and GS round
    /// trip through a 16-bit stack slot in real-address mode, where the load is
    /// the `selector << 4` base update.
    #[test]
    fn push_pop_fs_gs_round_trip_in_real_mode() {
        // 0F A0 = PUSH FS; 0F A9 = POP GS.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA0, 0x0F, 0xA9], |cpu, _| {
            cpu.fs.load_real_mode_selector(0x1234);
            cpu.gs.load_real_mode_selector(0x0000);
        });

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFEE);
        assert_eq!(bus.read_u16(0xFFEE).unwrap(), 0x1234);

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF0);
        assert_eq!(cpu.gs.selector, 0x1234);
        assert_eq!(cpu.gs.base, 0x1_2340);
        assert_eq!(cpu.fs.selector, 0x1234);
        assert_eq!(cpu.ip16(), 4);

        // 0F A8 = PUSH GS; 0F A1 = POP FS.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA8, 0x0F, 0xA1], |cpu, _| {
            cpu.gs.load_real_mode_selector(0xB800);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0xFFEE).unwrap(), 0xB800);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.fs.selector, 0xB800);
        assert_eq!(cpu.fs.base, 0xB_8000);
    }

    /// Intel SDM Vol. 2 "PUSH"/"POP" (Operation): a 32-bit operand size uses a
    /// doubleword stack slot holding the zero-extended selector, and `0x66`
    /// selects the 16-bit slot again under `CS.D=1`.
    #[test]
    fn push_pop_fs_gs_slot_width_follows_operand_size() {
        // 0F A0 = PUSH FS; 0F A1 = POP FS, both 32-bit under CS.D=1.
        let (mut cpu, mut bus) =
            pm32_big_stack_fixture(&[0x0F, 0xA0, 0x0F, 0xA1], PM32_CODE, PM32_TEST_ESP);
        cpu.fs = x86_core::SegmentReg {
            selector: PM32_DS,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };

        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 4);
        assert_eq!(
            bus.read_u32(u64::from(PM32_TEST_ESP - 4)).unwrap(),
            u32::from(PM32_DS)
        );

        cpu.fs = x86_core::SegmentReg::real_mode(0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP);
        assert_eq!(cpu.fs.selector, PM32_DS);
        assert_eq!(cpu.fs.limit, 0xFFFF);

        // 66 0F A8 = PUSH GS with a 16-bit slot under CS.D=1.
        let (mut cpu, mut bus) =
            pm32_big_stack_fixture(&[0x66, 0x0F, 0xA8], PM32_CODE, PM32_TEST_ESP);
        cpu.gs.selector = 0x1234;
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP - 2);
        assert_eq!(bus.read_u16(u64::from(PM32_TEST_ESP - 2)).unwrap(), 0x1234);
    }

    /// Intel SDM Vol. 2 "PUSH" (Operation, segment-register source); Vol. 1
    /// §6.2: the stack slot of a primary-map segment `PUSH` follows the
    /// operand-size attribute — a word by default in a 16-bit code segment and
    /// a doubleword holding the zero-extended selector with `0x66`.
    #[test]
    fn primary_map_segment_push_slot_width_follows_operand_size() {
        for (opcode, selector) in [
            (0x06u8, 0x1111u16),
            (0x0E, 0x2222),
            (0x16, 0x3333),
            (0x1E, 0x4444),
        ] {
            // 16-bit operand size: a word slot.
            let (mut cpu, mut bus) = real_mode_fixture(&[opcode], |cpu, _| {
                cpu.es.selector = selector;
                cpu.cs.selector = selector;
                cpu.ss.selector = selector;
                cpu.ds.selector = selector;
            });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(
                cpu.gpr_u16(CpuState::RSP),
                0xFFEE,
                "{opcode:#04X} word slot"
            );
            assert_eq!(bus.read_u16(0xFFEE).unwrap(), selector);

            // 32-bit operand size: a doubleword slot with the selector
            // zero-extended.
            let (mut cpu, mut bus) = real_mode_fixture(&[0x66, opcode], |cpu, _| {
                cpu.es.selector = selector;
                cpu.cs.selector = selector;
                cpu.ss.selector = selector;
                cpu.ds.selector = selector;
            });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(
                cpu.gpr_u16(CpuState::RSP),
                0xFFEC,
                "{opcode:#04X} doubleword slot"
            );
            assert_eq!(bus.read_u32(0xFFEC).unwrap(), u32::from(selector));
        }
    }

    /// Intel SDM Vol. 2 "POP" (Operation): the operand-size attribute selects
    /// how much the stack pointer is released; only the low word of a
    /// doubleword slot loads into the segment register.
    #[test]
    fn primary_map_segment_pop_slot_width_follows_operand_size() {
        // 1F = POP DS from a word slot.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x1F], |_, mem| {
            mem[0xFFF0..0xFFF4].copy_from_slice(&0xAAAA_1234u32.to_le_bytes());
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ds.selector, 0x1234);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF2);

        // 66 1F = POP DS from a doubleword slot; the upper half is discarded.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x66, 0x1F], |_, mem| {
            mem[0xFFF0..0xFFF4].copy_from_slice(&0xAAAA_1234u32.to_le_bytes());
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ds.selector, 0x1234);
        assert_eq!(cpu.ds.base, 0x1_2340);
        assert_eq!(cpu.gpr_u16(CpuState::RSP), 0xFFF4);
    }

    /// The primary-map (`06`/`0E`/`16`/`1E`, `07`/`17`/`1F`) and two-byte
    /// (`0F A0`/`A1`/`A8`/`A9`) segment stack forms must size the slot by the
    /// same rule; round 2 left the primary map always 16-bit, which corrupts a
    /// 32-bit stack when firmware mixes the two encodings.
    /// Spec: Intel SDM Vol. 2 "PUSH"/"POP"; Vol. 1 §6.2.
    #[test]
    fn primary_and_two_byte_segment_stack_slots_agree() {
        for prefix in [Vec::new(), vec![0x66u8]] {
            let expected_delta = if prefix.is_empty() { 4 } else { 2 };

            // PUSH DS versus PUSH FS with the same selector in both registers.
            let mut primary = prefix.clone();
            primary.push(0x1E);
            let mut two_byte = prefix.clone();
            two_byte.extend_from_slice(&[0x0F, 0xA0]);

            let (mut cpu, mut bus) = pm32_big_stack_fixture(&primary, PM32_CODE, PM32_TEST_ESP);
            step(&mut cpu, &mut bus).unwrap();
            let primary_esp = cpu.gpr_u32(CpuState::RSP);
            let primary_image = bus.mem[primary_esp as usize..PM32_TEST_ESP as usize].to_vec();

            let (mut cpu, mut bus) = pm32_big_stack_fixture(&two_byte, PM32_CODE, PM32_TEST_ESP);
            cpu.fs = cpu.ds.clone();
            step(&mut cpu, &mut bus).unwrap();
            let two_byte_esp = cpu.gpr_u32(CpuState::RSP);
            let two_byte_image = bus.mem[two_byte_esp as usize..PM32_TEST_ESP as usize].to_vec();

            assert_eq!(primary_esp, two_byte_esp, "PUSH prefix {prefix:?}");
            assert_eq!(primary_image, two_byte_image, "PUSH prefix {prefix:?}");
            assert_eq!(PM32_TEST_ESP - primary_esp, expected_delta);

            // POP DS versus POP FS from the same stack image.
            let mut primary = prefix.clone();
            primary.push(0x1F);
            let mut two_byte = prefix.clone();
            two_byte.extend_from_slice(&[0x0F, 0xA1]);

            let (mut cpu, mut bus) = pm32_big_stack_fixture(&primary, PM32_CODE, PM32_TEST_ESP);
            bus.mem[PM32_TEST_ESP as usize..PM32_TEST_ESP as usize + 4]
                .copy_from_slice(&u32::from(PM32_DS).to_le_bytes());
            step(&mut cpu, &mut bus).unwrap();
            let primary_esp = cpu.gpr_u32(CpuState::RSP);

            let (mut cpu, mut bus) = pm32_big_stack_fixture(&two_byte, PM32_CODE, PM32_TEST_ESP);
            bus.mem[PM32_TEST_ESP as usize..PM32_TEST_ESP as usize + 4]
                .copy_from_slice(&u32::from(PM32_DS).to_le_bytes());
            step(&mut cpu, &mut bus).unwrap();

            assert_eq!(
                primary_esp,
                cpu.gpr_u32(CpuState::RSP),
                "POP prefix {prefix:?}"
            );
            assert_eq!(primary_esp - PM32_TEST_ESP, expected_delta);
        }
    }

    /// Intel SDM Vol. 3 §6.8.3: `POP SS` inhibits maskable interrupts through
    /// the following instruction, and widening its stack slot does not change
    /// that. The descriptor is validated before the pointer commits.
    #[test]
    fn pop_ss_with_a_32_bit_slot_still_arms_the_interrupt_shadow() {
        let (mut cpu, mut bus) = pm32_big_stack_fixture(&[0x17], PM32_CODE, PM32_TEST_ESP);
        bus.mem[PM32_TEST_ESP as usize..PM32_TEST_ESP as usize + 4]
            .copy_from_slice(&u32::from(PM32_DS).to_le_bytes());
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ss.selector, PM32_DS);
        assert!(cpu.maskable_interrupts_inhibited());
        // The pointer width came from the *old* `SS.B=1`, so ESP moved by four.
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP + 4);
    }

    /// Intel SDM Vol. 3 §5.4.1: `POP FS`/`POP GS` use the DS/ES data-segment
    /// rules, so a null selector loads and clears the cache while an invalid
    /// selector raises `#GP(selector)` without committing the stack pointer.
    #[test]
    fn pop_fs_gs_protected_mode_null_and_invalid_selectors() {
        // 0F A1 = POP FS with a null selector on the stack.
        let (mut cpu, mut bus) = pm32_big_stack_fixture(&[0x0F, 0xA1], PM32_CODE, PM32_TEST_ESP);
        bus.mem[PM32_TEST_ESP as usize..PM32_TEST_ESP as usize + 4]
            .copy_from_slice(&0u32.to_le_bytes());
        cpu.fs = x86_core::SegmentReg {
            selector: PM32_DS,
            base: 0,
            limit: 0xFFFF,
            flags: 0x0093,
        };
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.fs.selector, 0);
        assert_eq!(cpu.fs.limit, 0);
        assert_eq!(cpu.fs.flags, 0);
        assert_eq!(cpu.gpr_u32(CpuState::RSP), PM32_TEST_ESP + 4);

        // A selector past the GDT limit is #GP(selector) and commits nothing.
        let (mut cpu, mut bus) = pm32_big_stack_fixture(&[0x0F, 0xA9], PM32_CODE, PM32_TEST_ESP);
        bus.mem[PM32_TEST_ESP as usize..PM32_TEST_ESP as usize + 4]
            .copy_from_slice(&0x0030u32.to_le_bytes());
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0x0030));
        assert_eq!(cpu, before, "POP GS committed state before #GP");
    }

    /// Intel SDM Vol. 2 "LDS/LES/LFS/LGS/LSS": each form loads the offset into
    /// the ModR/M.reg register and the selector into its own segment register,
    /// with the pointer width following the operand-size attribute.
    #[test]
    fn lss_lfs_lgs_load_far_pointers_in_real_mode() {
        for (opcode, sreg_name) in [(0xB2u8, "SS"), (0xB4, "FS"), (0xB5, "GS")] {
            // 0F op 06 00 40 = Lxx AX, [0x4000]
            let (mut cpu, mut bus) =
                real_mode_fixture(&[0x0F, opcode, 0x06, 0x00, 0x40], |_, mem| {
                    mem[0x4000] = 0x34;
                    mem[0x4001] = 0x12;
                    mem[0x4002] = 0x00;
                    mem[0x4003] = 0x20;
                });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ax(), 0x1234, "{sreg_name} offset");
            let loaded = match opcode {
                0xB2 => &cpu.ss,
                0xB4 => &cpu.fs,
                _ => &cpu.gs,
            };
            assert_eq!(loaded.selector, 0x2000, "{sreg_name} selector");
            assert_eq!(loaded.base, 0x2_0000, "{sreg_name} base");
            assert_eq!(cpu.ip16(), 5);

            // 66 0F op 06 00 40 = Lxx EAX, m16:32
            let (mut cpu, mut bus) =
                real_mode_fixture(&[0x66, 0x0F, opcode, 0x06, 0x00, 0x40], |_, mem| {
                    mem[0x4000..0x4004].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
                    mem[0x4004..0x4006].copy_from_slice(&0x3000u16.to_le_bytes());
                });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.eax(), 0xDEAD_BEEF, "{sreg_name} offset32");
            let loaded = match opcode {
                0xB2 => &cpu.ss,
                0xB4 => &cpu.fs,
                _ => &cpu.gs,
            };
            assert_eq!(loaded.selector, 0x3000);
        }
    }

    /// Intel SDM Vol. 3 §6.8.3: loading SS with `LSS` inhibits maskable
    /// interrupts through the following instruction, exactly like `MOV SS` and
    /// `POP SS`. `LFS`/`LGS` do not.
    #[test]
    fn lss_arms_the_maskable_interrupt_shadow_and_lfs_lgs_do_not() {
        for (opcode, expected) in [(0xB2u8, true), (0xB4, false), (0xB5, false)] {
            let (mut cpu, mut bus) =
                real_mode_fixture(&[0x0F, opcode, 0x06, 0x00, 0x40], |_, mem| {
                    mem[0x4002] = 0x00;
                    mem[0x4003] = 0x20;
                });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(
                cpu.maskable_interrupts_inhibited(),
                expected,
                "0F {opcode:02X} interrupt shadow"
            );
        }
    }

    /// Intel SDM Vol. 2 "LDS/LES/LFS/LGS/LSS"; Vol. 3 §6.15: the register form
    /// (`mod=11`) has no far pointer to load and is `#UD`.
    #[test]
    fn lss_lfs_lgs_register_form_is_ud() {
        for opcode in [0xB2u8, 0xB4, 0xB5] {
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, opcode, 0xC0], |_, _| {});
            let before = cpu.clone();
            assert_arch_fault(step_inner(&mut cpu, &mut bus), 6, None);
            assert_eq!(cpu, before, "0F {opcode:02X} register form mutated state");
        }
    }

    /// Intel SDM Vol. 2 "LSS" (Protected Mode Exceptions); Vol. 3 §5.4.1: `LSS`
    /// uses the stack-segment descriptor rules — a null selector is `#GP(0)`
    /// and a non-writable descriptor is `#GP(selector)` — and commits nothing
    /// on failure. `LFS`/`LGS` accept a null selector like `MOV FS`/`MOV GS`.
    #[test]
    fn lss_protected_mode_uses_stack_descriptor_rules_atomically() {
        // 0F B2 05 disp32 = LSS EAX, [disp32] under CS.D=1.
        let far_pointer_code = |opcode: u8| {
            let mut code = vec![0x0F, opcode, 0x05];
            code.extend_from_slice(&(PM32_DATA as u32).to_le_bytes());
            code
        };
        let seed_pointer = |bus: &mut VecBus, selector: u16| {
            bus.mem[PM32_DATA..PM32_DATA + 4].copy_from_slice(&0x0001_2345u32.to_le_bytes());
            bus.mem[PM32_DATA + 4..PM32_DATA + 6].copy_from_slice(&selector.to_le_bytes());
        };

        // Writable ring-0 data selector: SS loads and EAX takes the offset.
        let (mut cpu, mut bus) = pm32_fixture(&far_pointer_code(0xB2), PM32_CODE, true);
        seed_pointer(&mut bus, PM32_DS);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x0001_2345);
        assert_eq!(cpu.ss.selector, PM32_DS);
        assert_eq!(cpu.ss.limit, 0xFFFF);

        // Null selector into SS is #GP(0).
        let (mut cpu, mut bus) = pm32_fixture(&far_pointer_code(0xB2), PM32_CODE, true);
        seed_pointer(&mut bus, 0);
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before, "LSS committed state before #GP(0)");

        // A readable code descriptor is not a writable stack segment.
        let (mut cpu, mut bus) = pm32_fixture(&far_pointer_code(0xB2), PM32_CODE, true);
        seed_pointer(&mut bus, PM32_CS32);
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(PM32_CS32));
        assert_eq!(cpu, before, "LSS committed state before #GP(selector)");

        // LFS accepts the null selector and clears the FS cache.
        let (mut cpu, mut bus) = pm32_fixture(&far_pointer_code(0xB4), PM32_CODE, true);
        seed_pointer(&mut bus, 0);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.eax(), 0x0001_2345);
        assert_eq!(cpu.fs.selector, 0);
        assert_eq!(cpu.fs.flags, 0);
    }

    /// Intel SDM Vol. 2 "BT"/"BTS"/"BTR"/"BTC": with a register bit base the
    /// offset is taken modulo the operand size, `CF` receives the original bit,
    /// and only `BTS`/`BTR`/`BTC` change it.
    #[test]
    fn bt_family_register_bit_base_uses_offset_modulo_operand_size() {
        // 0F op C8 = xx AX, CX (mod 11, reg = CX, rm = AX).
        // (opcode, CX, seeded AX, expected AX, expected CF)
        let cases: [(u8, u16, u16, u16, bool); 8] = [
            (0xA3, 1, 0x0002, 0x0002, true),   // BT AX, 1
            (0xA3, 17, 0x0002, 0x0002, true),  // offset 17 MOD 16 = 1
            (0xA3, 16, 0x0002, 0x0002, false), // offset 16 MOD 16 = 0
            (0xAB, 4, 0x0000, 0x0010, false),  // BTS sets bit 4
            (0xAB, 4, 0x0010, 0x0010, true),   // already set
            (0xB3, 4, 0x0010, 0x0000, true),   // BTR clears bit 4
            (0xBB, 4, 0x0010, 0x0000, true),   // BTC toggles a set bit
            (0xBB, 4, 0x0000, 0x0010, false),  // BTC toggles a clear bit
        ];
        for (opcode, offset, ax, expected_ax, expected_cf) in cases {
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, opcode, 0xC8], |cpu, _| {
                cpu.set_ax(ax);
                cpu.set_gpr_u16(CpuState::RCX, offset);
            });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ax(), expected_ax, "0F {opcode:02X} offset {offset}");
            assert_eq!(
                cpu.rflags & 1 != 0,
                expected_cf,
                "0F {opcode:02X} offset {offset} CF"
            );
        }

        // 66 0F A3 C8 = BT EAX, ECX — offset 33 MOD 32 = 1.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x66, 0x0F, 0xA3, 0xC8], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RAX, 0x0000_0002);
            cpu.set_gpr_u32(CpuState::RCX, 33);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert!(cpu.rflags & 1 != 0);

        // A negative register offset is still reduced to a bit inside the
        // register: -1 MOD 32 = 31.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x66, 0x0F, 0xA3, 0xC8], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RAX, 0x8000_0000);
            cpu.set_gpr_u32(CpuState::RCX, (-1i32) as u32);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert!(cpu.rflags & 1 != 0);
    }

    /// Intel SDM Vol. 2 §3.1.1.9 (`Bit(BitBase, BitOffset)`): with a memory bit
    /// base the addressed bit is `BitOffset MOD 8` inside the byte at
    /// `BitBase + (BitOffset DIV 8)`, using signed division that rounds toward
    /// negative infinity, so a register bit offset reaches far outside — and
    /// below — the nominal operand.
    #[test]
    fn bt_memory_bit_string_addresses_bits_outside_the_operand() {
        // 0F A3 0E 00 40 = BT [0x4000], CX
        // Offset 100 → byte 0x4000 + 12, bit 4.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA3, 0x0E, 0x00, 0x40], |cpu, mem| {
            cpu.set_gpr_u16(CpuState::RCX, 100);
            mem[0x400C] = 1 << 4;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert!(cpu.rflags & 1 != 0, "bit 100 of the string must be set");

        // The same offset with the bit clear.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA3, 0x0E, 0x00, 0x40], |cpu, mem| {
            cpu.set_gpr_u16(CpuState::RCX, 100);
            mem[0x400C] = !(1 << 4);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rflags & 1, 0);

        // Offset -1 → byte 0x3FFF, bit 7 (DIV rounds toward negative infinity).
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA3, 0x0E, 0x00, 0x40], |cpu, mem| {
            cpu.set_gpr_u16(CpuState::RCX, (-1i16) as u16);
            mem[0x3FFF] = 0x80;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert!(cpu.rflags & 1 != 0, "bit -1 lives in the preceding byte");

        // BTS at offset 100 writes only the addressed byte.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xAB, 0x0E, 0x00, 0x40], |cpu, mem| {
            cpu.set_gpr_u16(CpuState::RCX, 100);
            mem[0x4000] = 0x00;
            mem[0x400C] = 0x01;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x400C).unwrap(), 0x11);
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0x00);
        assert_eq!(cpu.rflags & 1, 0);
    }

    /// Intel SDM Vol. 3 §5.3: the segment-limit check applies to the byte the
    /// bit offset actually selects, not to the bit base, and a failing access
    /// commits neither `CF` nor memory.
    #[test]
    fn bt_memory_limit_check_follows_the_displaced_byte_address() {
        // 0F AB 0E FF 3F = BTS [0x3FFF], CX with CX = 8 → byte 0x4000.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xAB, 0x0E, 0xFF, 0x3F], |cpu, _| {
            cpu.set_gpr_u16(CpuState::RCX, 8);
            cpu.ds.limit = 0x3FFF;
            cpu.set_cf(false);
        });
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before, "BTS committed state before #GP");
        assert_eq!(bus.read_u8(0x4000).unwrap(), 0);

        // The bit base itself is inside the limit, so offset 0 succeeds.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xAB, 0x0E, 0xFF, 0x3F], |cpu, _| {
            cpu.set_gpr_u16(CpuState::RCX, 0);
            cpu.ds.limit = 0x3FFF;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u8(0x3FFF).unwrap(), 0x01);
    }

    /// Intel SDM Vol. 2 "BT" family (Flags Affected): only `CF` is defined.
    /// This interpreter leaves the undefined `OF`/`SF`/`ZF`/`AF`/`PF` unchanged
    /// so the reference semantics stay deterministic.
    #[test]
    fn bt_family_leaves_undefined_flags_unchanged() {
        let flags = 0x0002 | (1 << 2) | (1 << 4) | (1 << 6) | (1 << 7) | (1 << 11);
        for opcode in [0xA3u8, 0xAB, 0xB3, 0xBB] {
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, opcode, 0xC8], |cpu, _| {
                cpu.rflags = flags;
                cpu.set_ax(0x0002);
                cpu.set_gpr_u16(CpuState::RCX, 1);
            });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(
                cpu.rflags & !1,
                flags & !1,
                "0F {opcode:02X} changed an undefined flag"
            );
            assert!(cpu.rflags & 1 != 0);
        }
    }

    /// Intel SDM Vol. 2 opcode map 2, Group 8: `0F BA /4`–`/7` are the
    /// immediate bit-offset forms and `/0`–`/3` are reserved (`#UD`).
    #[test]
    fn grp8_immediate_bit_forms_and_reserved_encodings() {
        // (reg, seeded AX, expected AX, expected CF) with imm8 = 4.
        let cases: [(u8, u16, u16, bool); 4] = [
            (4, 0x0010, 0x0010, true),  // BT
            (5, 0x0000, 0x0010, false), // BTS
            (6, 0x0010, 0x0000, true),  // BTR
            (7, 0x0010, 0x0000, true),  // BTC
        ];
        for (reg, ax, expected_ax, expected_cf) in cases {
            let modrm = 0xC0 | (reg << 3);
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xBA, modrm, 0x04], |cpu, _| {
                cpu.set_ax(ax);
            });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ax(), expected_ax, "0F BA /{reg}");
            assert_eq!(cpu.rflags & 1 != 0, expected_cf, "0F BA /{reg} CF");
            assert_eq!(cpu.ip16(), 4);
        }

        // Memory form: 0F BA 26 00 40 09 = BT word [0x4000], 9 → byte 0x4001 bit 1.
        let (mut cpu, mut bus) =
            real_mode_fixture(&[0x0F, 0xBA, 0x26, 0x00, 0x40, 0x09], |_, mem| {
                mem[0x4001] = 0x02;
            });
        step(&mut cpu, &mut bus).unwrap();
        assert!(cpu.rflags & 1 != 0);

        // Reserved /0–/3 are #UD and commit nothing.
        for reg in 0u8..4 {
            let modrm = 0xC0 | (reg << 3);
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xBA, modrm, 0x04], |_, _| {});
            let before = cpu.clone();
            assert_arch_fault(step_inner(&mut cpu, &mut bus), 6, None);
            assert_eq!(cpu, before, "0F BA /{reg} mutated state");
        }
    }

    /// Intel SDM Vol. 2 "BSF"/"BSR": the destination gets the index of the
    /// least/most significant set bit and `ZF` is clear; a zero source sets `ZF`
    /// and leaves the destination architecturally undefined, which this
    /// interpreter models as unchanged.
    #[test]
    fn bsf_bsr_index_and_zero_source_rule() {
        // 0F BC D8 = BSF BX, AX; 0F BD D8 = BSR BX, AX.
        for (opcode, ax, expected_bx) in [
            (0xBCu8, 0x0100u16, 8u16),
            (0xBC, 0x8001, 0),
            (0xBD, 0x0100, 8),
            (0xBD, 0x8001, 15),
        ] {
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, opcode, 0xD8], |cpu, _| {
                cpu.set_ax(ax);
                cpu.set_gpr_u16(CpuState::RBX, 0xDEAD);
            });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.gpr_u16(CpuState::RBX), expected_bx, "0F {opcode:02X}");
            assert_eq!(cpu.rflags & (1 << 6), 0, "0F {opcode:02X} ZF");
        }

        // Zero source: ZF set, destination left as it was.
        for opcode in [0xBCu8, 0xBD] {
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, opcode, 0xD8], |cpu, _| {
                cpu.set_ax(0);
                cpu.set_gpr_u16(CpuState::RBX, 0xDEAD);
            });
            step(&mut cpu, &mut bus).unwrap();
            assert_ne!(
                cpu.rflags & (1 << 6),
                0,
                "0F {opcode:02X} ZF on zero source"
            );
            assert_eq!(cpu.gpr_u16(CpuState::RBX), 0xDEAD);
        }

        // 32-bit operand size scans the full doubleword.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x66, 0x0F, 0xBD, 0xD8], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RAX, 0x8000_0000);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 31);

        // Memory source: 0F BC 1E 00 40 = BSF BX, [0x4000]
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xBC, 0x1E, 0x00, 0x40], |_, mem| {
            mem[0x4000] = 0x00;
            mem[0x4001] = 0x04;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 10);
    }

    /// Intel SDM Vol. 2 "BSWAP": reverses the byte order of a doubleword
    /// register and affects no flags.
    #[test]
    fn bswap_reverses_doubleword_registers() {
        for reg in 0u8..8 {
            let flags = 0x0002 | 1 | (1 << 6);
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xC8 + reg], |cpu, _| {
                cpu.rflags = flags;
                cpu.set_gpr_u32(reg as usize, 0x1234_5678);
            });
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.gpr_u32(reg as usize), 0x7856_3412, "BSWAP reg {reg}");
            assert_eq!(cpu.rflags, flags, "BSWAP must not write flags");
            assert_eq!(cpu.ip16(), 2);
        }
    }

    /// Intel SDM Vol. 2 "XADD": `TEMP := SRC + DEST; SRC := DEST; DEST := TEMP`,
    /// with the ADD flag results.
    #[test]
    fn xadd_exchanges_and_adds_in_every_width() {
        // 0F C1 C8 = XADD AX, CX (dest = r/m = AX, src = reg = CX).
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xC1, 0xC8], |cpu, _| {
            cpu.set_ax(5);
            cpu.set_gpr_u16(CpuState::RCX, 3);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ax(), 8);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 5);
        assert_eq!(cpu.rflags & 1, 0);

        // Byte form with a carry out: 0F C0 C8 = XADD AL, CL.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xC0, 0xC8], |cpu, _| {
            cpu.set_al(0xFF);
            cpu.set_gpr_u8_low(CpuState::RCX, 0x02);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.al(), 0x01);
        assert_eq!(cpu.gpr_u8_low(CpuState::RCX), 0xFF);
        assert_ne!(cpu.rflags & 1, 0, "CF from the byte add");

        // Memory destination: 0F C1 0E 00 40 = XADD [0x4000], CX.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xC1, 0x0E, 0x00, 0x40], |cpu, mem| {
            cpu.set_gpr_u16(CpuState::RCX, 0x0001);
            mem[0x4000] = 0x34;
            mem[0x4001] = 0x12;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0x1235);
        assert_eq!(cpu.gpr_u16(CpuState::RCX), 0x1234);

        // 32-bit form: 66 0F C1 C8 = XADD EAX, ECX.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x66, 0x0F, 0xC1, 0xC8], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RAX, 0xFFFF_FFFF);
            cpu.set_gpr_u32(CpuState::RCX, 1);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0);
        assert_eq!(cpu.gpr_u32(CpuState::RCX), 0xFFFF_FFFF);
        assert_ne!(cpu.rflags & 1, 0);
        assert_ne!(cpu.rflags & (1 << 6), 0, "ZF from the wrapped sum");
    }

    /// Intel SDM Vol. 2 "CMPXCHG": on a match `ZF=1` and the source is written
    /// to the destination; otherwise `ZF=0`, the accumulator takes the old
    /// destination, and the destination is written back with its own value.
    /// CF/PF/AF/SF/OF follow the same comparison.
    #[test]
    fn cmpxchg_equal_and_unequal_paths() {
        // 0F B1 CB = CMPXCHG BX, CX (dest = r/m = BX, src = reg = CX).
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xB1, 0xCB], |cpu, _| {
            cpu.set_ax(0x1234);
            cpu.set_gpr_u16(CpuState::RBX, 0x1234);
            cpu.set_gpr_u16(CpuState::RCX, 0x5678);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u16(CpuState::RBX), 0x5678);
        assert_eq!(cpu.ax(), 0x1234, "accumulator unchanged on a match");
        assert_ne!(cpu.rflags & (1 << 6), 0, "ZF set on a match");
        assert_eq!(cpu.rflags & 1, 0, "CF from 0x1234 - 0x1234");

        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xB1, 0xCB], |cpu, _| {
            cpu.set_ax(0x1111);
            cpu.set_gpr_u16(CpuState::RBX, 0x1234);
            cpu.set_gpr_u16(CpuState::RCX, 0x5678);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(
            cpu.gpr_u16(CpuState::RBX),
            0x1234,
            "destination written back"
        );
        assert_eq!(cpu.ax(), 0x1234, "accumulator takes the old destination");
        assert_eq!(cpu.rflags & (1 << 6), 0, "ZF clear on a mismatch");
        assert_ne!(cpu.rflags & 1, 0, "CF from 0x1111 - 0x1234 borrow");

        // Byte form: 0F B0 CB = CMPXCHG BL, CL against AL.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xB0, 0xCB], |cpu, _| {
            cpu.set_al(0x42);
            cpu.set_gpr_u8_low(CpuState::RBX, 0x42);
            cpu.set_gpr_u8_low(CpuState::RCX, 0x99);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u8_low(CpuState::RBX), 0x99);
        assert_ne!(cpu.rflags & (1 << 6), 0);

        // Memory destination, mismatching: the old value is written back.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xB1, 0x0E, 0x00, 0x40], |cpu, mem| {
            cpu.set_ax(0x0000);
            cpu.set_gpr_u16(CpuState::RCX, 0xBEEF);
            mem[0x4000] = 0xCD;
            mem[0x4001] = 0xAB;
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u16(0x4000).unwrap(), 0xABCD);
        assert_eq!(cpu.ax(), 0xABCD);
        assert_eq!(cpu.rflags & (1 << 6), 0);

        // 32-bit form matching: 66 0F B1 CB = CMPXCHG EBX, ECX against EAX.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x66, 0x0F, 0xB1, 0xCB], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RAX, 0xDEAD_BEEF);
            cpu.set_gpr_u32(CpuState::RBX, 0xDEAD_BEEF);
            cpu.set_gpr_u32(CpuState::RCX, 0x1234_5678);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0x1234_5678);
        assert_ne!(cpu.rflags & (1 << 6), 0);
    }

    /// Intel SDM Vol. 2 "CMPXCHG8B": compare `EDX:EAX` with `m64`; on equal
    /// set ZF and store `ECX:EBX`, else clear ZF, load `m64` into `EDX:EAX`, and
    /// write the old value back. CF/PF/AF/SF/OF are unaffected. Register form
    /// is `#UD`. LOCK may prefix the memory form.
    #[test]
    fn cmpxchg8b_equal_unequal_flags_lock_and_ud() {
        // 0F C7 /1 [0x4000] — equal path.
        let flags = 0x0002 | 1 | (1 << 2) | (1 << 4) | (1 << 7) | (1 << 11);
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xC7, 0x0E, 0x00, 0x40], |cpu, mem| {
            cpu.rflags = flags;
            cpu.set_gpr_u32(CpuState::RAX, 0x1111_2222);
            cpu.set_gpr_u32(CpuState::RDX, 0x3333_4444);
            cpu.set_gpr_u32(CpuState::RBX, 0xAAAA_BBBB);
            cpu.set_gpr_u32(CpuState::RCX, 0xCCCC_DDDD);
            mem[0x4000..0x4008].copy_from_slice(&0x3333_4444_1111_2222u64.to_le_bytes());
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0xAAAA_BBBB);
        assert_eq!(bus.read_u32(0x4004).unwrap(), 0xCCCC_DDDD);
        assert_eq!(
            cpu.gpr_u32(CpuState::RAX),
            0x1111_2222,
            "accumulator unchanged"
        );
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0x3333_4444);
        assert_ne!(cpu.rflags & (1 << 6), 0, "ZF set on match");
        assert_eq!(
            cpu.rflags & !(1 << 6),
            flags & !(1 << 6),
            "only ZF may change"
        );
        assert_eq!(cpu.ip16(), 5);

        // Unequal path: memory written back, EDX:EAX takes TEMP, ZF clear.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xC7, 0x0E, 0x00, 0x40], |cpu, mem| {
            cpu.rflags = flags | (1 << 6);
            cpu.set_gpr_u32(CpuState::RAX, 0);
            cpu.set_gpr_u32(CpuState::RDX, 0);
            cpu.set_gpr_u32(CpuState::RBX, 0xAAAA_BBBB);
            cpu.set_gpr_u32(CpuState::RCX, 0xCCCC_DDDD);
            mem[0x4000..0x4008].copy_from_slice(&0xFEED_FACE_DEAD_BEEFu64.to_le_bytes());
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0xDEAD_BEEF, "write-back");
        assert_eq!(bus.read_u32(0x4004).unwrap(), 0xFEED_FACE);
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xDEAD_BEEF);
        assert_eq!(cpu.gpr_u32(CpuState::RDX), 0xFEED_FACE);
        assert_eq!(cpu.rflags & (1 << 6), 0, "ZF clear on mismatch");
        assert_eq!(cpu.rflags & !(1 << 6), flags & !(1 << 6));

        // LOCK prefix is accepted on the memory form (no multi-processor atomicity).
        let (mut cpu, mut bus) =
            real_mode_fixture(&[0xF0, 0x0F, 0xC7, 0x0E, 0x00, 0x40], |cpu, mem| {
                cpu.set_gpr_u32(CpuState::RAX, 1);
                cpu.set_gpr_u32(CpuState::RDX, 2);
                cpu.set_gpr_u32(CpuState::RBX, 3);
                cpu.set_gpr_u32(CpuState::RCX, 4);
                mem[0x4000..0x4008].copy_from_slice(&0x0000_0002_0000_0001u64.to_le_bytes());
            });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x4000).unwrap(), 3);
        assert_eq!(bus.read_u32(0x4004).unwrap(), 4);
        assert_ne!(cpu.rflags & (1 << 6), 0);

        // Register form: 0F C7 /1 CX → #UD.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xC7, 0xC9], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RAX, 1);
        });
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 6, None);
        assert_eq!(cpu, before);

        // Other Group 9 /r remain unimplemented.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xC7, 0x06, 0x00, 0x40], |_, _| {});
        assert_eq!(
            step_inner(&mut cpu, &mut bus),
            Err(ExecError::Unsupported(0xC7))
        );
    }

    /// Intel SDM Vol. 2 "CMPXCHG8B": a memory write fault after the compare
    /// leaves EDX:EAX and flags unchanged; segment-limit faults on the 8-byte
    /// span are `#GP`/`#SS` before any register update.
    #[test]
    fn cmpxchg8b_memory_faults_are_atomic() {
        // Limit ends at 0x4003 so the 8-byte access at 0x4000 fails #GP.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xC7, 0x0E, 0x00, 0x40], |cpu, mem| {
            cpu.ds.limit = 0x4003;
            cpu.set_gpr_u32(CpuState::RAX, 0x1111_2222);
            cpu.set_gpr_u32(CpuState::RDX, 0x3333_4444);
            cpu.set_gpr_u32(CpuState::RBX, 0xAAAA_BBBB);
            cpu.set_gpr_u32(CpuState::RCX, 0xCCCC_DDDD);
            cpu.rflags = 0x0002 | (1 << 6);
            mem[0x4000..0x4008].copy_from_slice(&0x3333_4444_1111_2222u64.to_le_bytes());
        });
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before);
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0x1111_2222);
        assert_eq!(bus.read_u32(0x4004).unwrap(), 0x3333_4444);

        // Protected mode: same equal/unequal ZF contract under PE=1.
        // 0F C7 /1 [disp32] — ModR/M 0x0D = mod=00, reg=1, rm=5.
        let (mut cpu, mut bus) =
            pm32_fixture(&[0x0F, 0xC7, 0x0D, 0x00, 0x40, 0x00, 0x00], PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RAX, 0x10);
        cpu.set_gpr_u32(CpuState::RDX, 0x20);
        cpu.set_gpr_u32(CpuState::RBX, 0x30);
        cpu.set_gpr_u32(CpuState::RCX, 0x40);
        bus.mem[0x4000..0x4008].copy_from_slice(&0x0000_0020_0000_0010u64.to_le_bytes());
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(bus.read_u32(0x4000).unwrap(), 0x30);
        assert_eq!(bus.read_u32(0x4004).unwrap(), 0x40);
        assert_ne!(cpu.rflags & (1 << 6), 0);
        assert_eq!(cpu.rip, (PM32_CODE + 7) as u64);
    }

    /// Intel SDM Vol. 2 "LAR"/"LSL": protected-mode soft checks set ZF and
    /// optionally load access rights / effective limit; real-address mode is `#UD`.
    #[test]
    fn lar_lsl_zf_matrices_null_type_and_privilege() {
        // Real-address mode → #UD.
        for opcode in [0x02u8, 0x03] {
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, opcode, 0xC1], |_, _| {});
            let before = cpu.clone();
            assert_arch_fault(step_inner(&mut cpu, &mut bus), 6, None);
            assert_eq!(cpu, before);
        }

        // LAR EAX, CX against the fixture ring-0 data selector 0x0018 (access 0x92).
        // Expected AR: (0x92 << 8) = 0x0000_9200.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x02, 0xC1], PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RCX, 0x0018);
        cpu.set_gpr_u32(CpuState::RAX, 0xDEAD_BEEF);
        cpu.rflags = 0x0002; // ZF clear
        step(&mut cpu, &mut bus).unwrap();
        assert_ne!(cpu.rflags & (1 << 6), 0, "LAR ZF set");
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x0000_9200);
        assert_eq!(cpu.rip, (PM32_CODE + 3) as u64);

        // LSL EAX, CX — byte-granular limit 0xFFFF.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x03, 0xC1], PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RCX, 0x0018);
        cpu.set_gpr_u32(CpuState::RAX, 0xDEAD_BEEF);
        step(&mut cpu, &mut bus).unwrap();
        assert_ne!(cpu.rflags & (1 << 6), 0, "LSL ZF set");
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xFFFF);

        // Null selector → ZF=0, destination unchanged.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x02, 0xC1], PM32_CODE, true);
        cpu.set_gpr_u32(CpuState::RCX, 0);
        cpu.set_gpr_u32(CpuState::RAX, 0x1111_2222);
        cpu.rflags = 0x0002 | (1 << 6);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rflags & (1 << 6), 0);
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x1111_2222);

        // Extend GDT: index 4 = page-granular data, index 5 = 32-bit call gate,
        // index 6 = interrupt gate (invalid for both), index 7 = conforming code DPL=0.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x03, 0xC1], PM32_CODE, true);
        cpu.gdtr.limit = 63;
        bus.mem[PM32_GDT + 32..PM32_GDT + 40]
            .copy_from_slice(&encode_seg_desc(0, 0xF_FFFF, 0x92, 0x80)); // G=1
        bus.mem[PM32_GDT + 40..PM32_GDT + 48].copy_from_slice(&encode_seg_desc(0x1000, 0, 0x8C, 0)); // type 0xC call gate-ish S=0
                                                                                                     // Force system call-gate type 0xC: access = P|DPL0|type = 0x8C.
        bus.mem[PM32_GDT + 40 + 5] = 0x8C;
        bus.mem[PM32_GDT + 48..PM32_GDT + 56].copy_from_slice(&encode_seg_desc(0, 0, 0x8E, 0)); // 32-bit interrupt gate
        bus.mem[PM32_GDT + 48 + 5] = 0x8E;
        bus.mem[PM32_GDT + 56..PM32_GDT + 64].copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x9E, 0)); // conforming code

        // LSL of G=1 data → 0xFFFF_FFFF.
        cpu.set_gpr_u32(CpuState::RCX, 0x0020);
        cpu.set_gpr_u32(CpuState::RAX, 0);
        step(&mut cpu, &mut bus).unwrap();
        assert_ne!(cpu.rflags & (1 << 6), 0);
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xFFFF_FFFF);

        // LAR accepts 32-bit call gate (type 0xC); LSL rejects it.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x02, 0xC1], PM32_CODE, true);
        cpu.gdtr.limit = 63;
        bus.mem[PM32_GDT + 40..PM32_GDT + 48].copy_from_slice(&[0u8; 8]);
        bus.mem[PM32_GDT + 40 + 5] = 0x8C; // type C call gate, P=1
        cpu.set_gpr_u32(CpuState::RCX, 0x0028);
        cpu.set_gpr_u32(CpuState::RAX, 0xAAAA_AAAA);
        step(&mut cpu, &mut bus).unwrap();
        assert_ne!(cpu.rflags & (1 << 6), 0, "LAR allows call gate");
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x0000_8C00);

        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x03, 0xC1], PM32_CODE, true);
        cpu.gdtr.limit = 63;
        bus.mem[PM32_GDT + 40 + 5] = 0x8C;
        cpu.set_gpr_u32(CpuState::RCX, 0x0028);
        cpu.set_gpr_u32(CpuState::RAX, 0xBBBB_BBBB);
        cpu.rflags = 0x0002 | (1 << 6);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rflags & (1 << 6), 0, "LSL rejects call gate");
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xBBBB_BBBB);

        // Interrupt gate invalid for LAR → ZF=0.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x02, 0xC1], PM32_CODE, true);
        cpu.gdtr.limit = 63;
        bus.mem[PM32_GDT + 48 + 5] = 0x8E;
        cpu.set_gpr_u32(CpuState::RCX, 0x0030);
        cpu.set_gpr_u32(CpuState::RAX, 0xCCCC_CCCC);
        cpu.rflags = 0x0002 | (1 << 6);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rflags & (1 << 6), 0);
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xCCCC_CCCC);

        // Privilege: CPL=3 vs DPL=0 data → ZF=0.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x02, 0xC1], PM32_CODE, true);
        cpu.cs.selector |= 3;
        cpu.set_gpr_u32(CpuState::RCX, 0x0018);
        cpu.set_gpr_u32(CpuState::RAX, 0xDDDD_DDDD);
        cpu.rflags = 0x0002 | (1 << 6);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rflags & (1 << 6), 0, "CPL > DPL clears ZF");
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xDDDD_DDDD);

        // Conforming code: CPL=3, DPL=0 still succeeds for LAR.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x02, 0xC1], PM32_CODE, true);
        cpu.gdtr.limit = 63;
        bus.mem[PM32_GDT + 56..PM32_GDT + 64].copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x9E, 0));
        cpu.cs.selector |= 3;
        cpu.set_gpr_u32(CpuState::RCX, 0x0038);
        cpu.set_gpr_u32(CpuState::RAX, 0);
        step(&mut cpu, &mut bus).unwrap();
        assert_ne!(cpu.rflags & (1 << 6), 0, "conforming skips DPL check");
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0x0000_9E00);

        // Not-present data: ZF=0, destination unchanged.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x02, 0xC1], PM32_CODE, true);
        cpu.gdtr.limit = 63;
        bus.mem[PM32_GDT + 32..PM32_GDT + 40].copy_from_slice(&encode_seg_desc(0, 0xFFFF, 0x12, 0)); // P=0 data
        cpu.set_gpr_u32(CpuState::RCX, 0x0020);
        cpu.set_gpr_u32(CpuState::RAX, 0xEEEE_EEEE);
        cpu.rflags = 0x0002 | (1 << 6);
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rflags & (1 << 6), 0, "P=0 clears ZF");
        assert_eq!(cpu.gpr_u32(CpuState::RAX), 0xEEEE_EEEE);

        // 16-bit operand size truncates LSL limit and LAR AR to low word.
        let (mut cpu, mut bus) = pm32_fixture(&[0x66, 0x0F, 0x03, 0xC1], PM32_CODE, true);
        cpu.gdtr.limit = 63;
        bus.mem[PM32_GDT + 32..PM32_GDT + 40]
            .copy_from_slice(&encode_seg_desc(0, 0xF_FFFF, 0x92, 0x80));
        cpu.set_gpr_u32(CpuState::RCX, 0x0020);
        cpu.set_gpr_u32(CpuState::RAX, 0xAAAA_BBBB);
        step(&mut cpu, &mut bus).unwrap();
        assert_ne!(cpu.rflags & (1 << 6), 0);
        assert_eq!(
            cpu.gpr_u32(CpuState::RAX),
            0xAAAA_FFFF,
            "16-bit LSL keeps EAX[31:16]"
        );
    }

    /// Intel SDM Vol. 2 "CPUID": leaf 0 reports the highest basic leaf in EAX
    /// and the vendor string in EBX:EDX:ECX. The string is deliberately not an
    /// Intel or AMD signature, so software cannot infer unimplemented features
    /// from a familiar vendor plus family/model (`docs/cpu-profile-core2.md`).
    #[test]
    fn cpuid_leaf_0_reports_max_basic_leaf_and_a_conservative_vendor() {
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA2], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RAX, 0);
        });
        step(&mut cpu, &mut bus).unwrap();

        assert_eq!(cpu.gpr_u32(CpuState::RAX), 1, "highest basic leaf");
        let mut vendor = Vec::new();
        vendor.extend_from_slice(&cpu.gpr_u32(CpuState::RBX).to_le_bytes());
        vendor.extend_from_slice(&cpu.gpr_u32(CpuState::RDX).to_le_bytes());
        vendor.extend_from_slice(&cpu.gpr_u32(CpuState::RCX).to_le_bytes());
        assert_eq!(vendor, b"x86WASM Emu ");
        assert_ne!(vendor, b"GenuineIntel");
        assert_ne!(vendor, b"AuthenticAMD");
        assert_eq!(cpu.ip16(), 2);
    }

    /// `AGENTS.md`: CPUID must never advertise an unimplemented feature. Leaf 1
    /// reports `PSE`, `MSR`, `PGE` and `CMOV`, all of which are implemented;
    /// every other enumerated feature must stay clear.
    ///
    /// Round 4 added `PSE` (bit 3) and `PGE` (bit 13) because §4.1.4 makes
    /// those bits a guest's licence to set `CR4.PSE` / `CR4.PGE`, and the
    /// paging engine implements 4-MiB pages and global pages; and `CMOV`
    /// (bit 15) because round 3 implemented `CMOVcc` and left the bit clear
    /// only for want of a reason to change it. `PAE`, `PAT` and `PSE-36` stay
    /// clear: the paging engine's default profile assumes exactly that.
    /// Spec: Intel SDM Vol. 2 "CPUID" (Table 3-11); Vol. 3 §4.1.4.
    #[test]
    fn cpuid_leaf_1_advertises_only_implemented_features() {
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA2], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RAX, 1);
        });
        step(&mut cpu, &mut bus).unwrap();

        let edx = cpu.gpr_u32(CpuState::RDX);
        let ecx = cpu.gpr_u32(CpuState::RCX);
        assert_eq!(
            edx,
            (1 << 3) | (1 << 5) | (1 << 13) | (1 << 15),
            "exactly PSE, MSR, PGE and CMOV"
        );
        assert_eq!(ecx, 0, "no ECX feature is implemented");
        assert_eq!(edx & (1 << 8), 0, "CX8 stays clear despite CMPXCHG8B");

        // Named guards for the features most likely to be assumed present.
        for (bit, name) in [
            (0u32, "FPU"),
            (1, "VME"),
            (2, "DE"),
            (4, "TSC"),
            (6, "PAE"),
            (8, "CX8"),
            (9, "APIC"),
            (11, "SEP"),
            (12, "MTRR"),
            (16, "PAT"),
            (17, "PSE-36"),
            (19, "CLFSH"),
            (23, "MMX"),
            (25, "SSE"),
            (26, "SSE2"),
            (28, "HTT"),
        ] {
            assert_eq!(edx & (1 << bit), 0, "CPUID must not advertise {name}");
        }

        // Family 6 is the generation that introduced PGE and CMOV, so the
        // version information still agrees with the feature bits reported.
        assert_eq!((cpu.eax() >> 8) & 0xF, 6, "family");
        assert_eq!(cpu.gpr_u32(CpuState::RBX), 0, "no brand/APIC-ID claims");
    }

    /// Intel SDM Vol. 2 "CPUID": a leaf above the maximum basic or extended
    /// input value returns the data for the highest basic leaf. Firmware probes
    /// `0x4000_0000` for a hypervisor signature; this emulator is not a
    /// hypervisor and must not present one.
    #[test]
    fn cpuid_out_of_range_leaves_return_the_highest_basic_leaf() {
        let leaf_1 = cpuid_leaf(1);
        for leaf in [2u32, 0x0000_000D, 0x4000_0000, 0x8000_0001, 0xFFFF_FFFF] {
            assert_eq!(cpuid_leaf(leaf), leaf_1, "leaf {leaf:#010X}");
        }

        // No hypervisor signature is spelled out by the 0x4000_0000 registers.
        let hv = cpuid_leaf(0x4000_0000);
        let mut signature = Vec::new();
        signature.extend_from_slice(&hv.ebx.to_le_bytes());
        signature.extend_from_slice(&hv.ecx.to_le_bytes());
        signature.extend_from_slice(&hv.edx.to_le_bytes());
        assert_ne!(&signature[..9], b"KVMKVMKVM");
        assert_ne!(&signature[..9], b"TCGTCGTCG");

        // The extended enumerator reports no extended leaves with content.
        let extended = cpuid_leaf(0x8000_0000);
        assert_eq!(extended.eax, 0x8000_0000);
        assert_eq!((extended.ebx, extended.ecx, extended.edx), (0, 0, 0));
    }

    /// Intel SDM Vol. 2 "CPUID": the instruction replaces EAX/EBX/ECX/EDX and
    /// affects no flags.
    #[test]
    fn cpuid_writes_four_registers_without_touching_flags() {
        let flags = 0x0002 | 1 | (1 << 6) | (1 << 11);
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0xA2], |cpu, _| {
            cpu.rflags = flags;
            cpu.set_gpr_u32(CpuState::RAX, 0);
            cpu.set_gpr_u32(CpuState::RBX, 0xDEAD_BEEF);
            cpu.set_gpr_u32(CpuState::RCX, 0xDEAD_BEEF);
            cpu.set_gpr_u32(CpuState::RDX, 0xDEAD_BEEF);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.rflags, flags);
        for idx in [CpuState::RBX, CpuState::RCX, CpuState::RDX] {
            assert_ne!(cpu.gpr_u32(idx), 0xDEAD_BEEF);
        }
    }

    /// Intel SDM Vol. 2 "RDMSR"/"WRMSR": reserved or unimplemented MSR addresses
    /// raise `#GP(0)`. `IA32_APIC_BASE` (`0x1B`) is implemented; everything else
    /// in this list still faults with CPU state unchanged.
    #[test]
    fn rdmsr_wrmsr_fault_on_every_unimplemented_msr_address() {
        for index in [
            0x0000_0010u32, // IA32_TIME_STAMP_COUNTER
            0x0000_00FE,    // IA32_MTRRCAP
            0x0000_02FF,    // IA32_MTRR_DEF_TYPE
            0xC000_0080,    // IA32_EFER
            0x0000_0000,
        ] {
            for opcode in [0x32u8, 0x30] {
                let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, opcode], |cpu, _| {
                    cpu.set_gpr_u32(CpuState::RCX, index);
                    cpu.set_gpr_u32(CpuState::RAX, 0x1111_2222);
                    cpu.set_gpr_u32(CpuState::RDX, 0x3333_4444);
                });
                let before = cpu.clone();
                assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
                assert_eq!(cpu, before, "0F {opcode:02X} MSR {index:#010X}");
            }
        }
    }

    /// Intel SDM Vol. 3 §10.4.4 / Vol. 4 MSR `1Bh` (`IA32_APIC_BASE`): reset
    /// BSP=1, EN=0 (bit 11), EXTD=0 (bit 10), base=`0xFEE0_0000`; WRMSR/RDMSR
    /// round-trip EN and the base; reserved bits (including x2APIC bit 10) and
    /// BSP changes raise `#GP(0)`.
    #[test]
    fn ia32_apic_base_msr_read_write_and_reserved_gp() {
        // RDMSR of the reset value.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0x32], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RCX, MSR_IA32_APIC_BASE);
            cpu.set_gpr_u32(CpuState::RAX, 0xDEAD_BEEF);
            cpu.set_gpr_u32(CpuState::RDX, 0xCAFE_BABE);
        });
        assert_eq!(cpu.ia32_apic_base, x86_core::IA32_APIC_BASE_RESET);
        step(&mut cpu, &mut bus).unwrap();
        let value =
            (u64::from(cpu.gpr_u32(CpuState::RDX)) << 32) | u64::from(cpu.gpr_u32(CpuState::RAX));
        assert_eq!(value, x86_core::IA32_APIC_BASE_RESET);
        assert_eq!(value & IA32_APIC_BASE_BSP, IA32_APIC_BASE_BSP);
        assert_eq!(value & IA32_APIC_BASE_ENABLE, 0);
        assert_eq!(value & IA32_APIC_BASE_X2APIC, 0);
        assert_eq!(value & !0xFFF, 0xFEE0_0000);

        // WRMSR: enable + relocate base, keep BSP.
        let new_val = 0xFEC0_0000 | IA32_APIC_BASE_BSP | IA32_APIC_BASE_ENABLE;
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0x30], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RCX, MSR_IA32_APIC_BASE);
            cpu.set_gpr_u32(CpuState::RAX, new_val as u32);
            cpu.set_gpr_u32(CpuState::RDX, (new_val >> 32) as u32);
        });
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.ia32_apic_base, new_val);

        // Round-trip RDMSR.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0x32], |cpu, _| {
            cpu.ia32_apic_base = new_val;
            cpu.set_gpr_u32(CpuState::RCX, MSR_IA32_APIC_BASE);
        });
        step(&mut cpu, &mut bus).unwrap();
        let readback =
            (u64::from(cpu.gpr_u32(CpuState::RDX)) << 32) | u64::from(cpu.gpr_u32(CpuState::RAX));
        assert_eq!(readback, new_val);

        // Reserved bit 0 → #GP(0), state unchanged.
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0x30], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RCX, MSR_IA32_APIC_BASE);
            cpu.set_gpr_u32(CpuState::RAX, (x86_core::IA32_APIC_BASE_RESET as u32) | 1);
            cpu.set_gpr_u32(CpuState::RDX, 0);
        });
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before);

        // x2APIC EXTD bit 10 → #GP(0).
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0x30], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RCX, MSR_IA32_APIC_BASE);
            cpu.set_gpr_u32(
                CpuState::RAX,
                (x86_core::IA32_APIC_BASE_RESET as u32) | IA32_APIC_BASE_X2APIC as u32,
            );
            cpu.set_gpr_u32(CpuState::RDX, 0);
        });
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before);

        // Clearing BSP (read-only) → #GP(0).
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0x30], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RCX, MSR_IA32_APIC_BASE);
            cpu.set_gpr_u32(CpuState::RAX, 0xFEE0_0000); // BSP cleared
            cpu.set_gpr_u32(CpuState::RDX, 0);
        });
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before);

        // Bit above the 36-bit phys model (bit 36) → #GP(0).
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0x30], |cpu, _| {
            cpu.set_gpr_u32(CpuState::RCX, MSR_IA32_APIC_BASE);
            cpu.set_gpr_u32(CpuState::RAX, x86_core::IA32_APIC_BASE_RESET as u32);
            cpu.set_gpr_u32(CpuState::RDX, 1 << 4); // bit 36 of the MSR
        });
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before);

        // CPL 3 still faults before the MSR is considered.
        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x32], PM32_CODE, true);
        cpu.cs.selector |= 3;
        cpu.set_gpr_u32(CpuState::RCX, MSR_IA32_APIC_BASE);
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
        assert_eq!(cpu, before);
    }

    /// Intel SDM Vol. 2 "RDMSR"/"WRMSR"/"INVD"/"WBINVD" (Protected Mode
    /// Exceptions): `#GP(0)` when the current privilege level is not 0.
    /// Real-address mode always runs at CPL 0.
    #[test]
    fn system_instructions_require_cpl0_in_protected_mode() {
        for opcode in [0x32u8, 0x30, 0x08, 0x09] {
            // CPL 0: INVD/WBINVD retire; RDMSR/WRMSR of an unimplemented index
            // still fault on the address (use TSC index so APIC_BASE is not hit).
            let (mut cpu, mut bus) = pm32_fixture(&[0x0F, opcode], PM32_CODE, true);
            cpu.set_gpr_u32(CpuState::RCX, 0x10); // IA32_TIME_STAMP_COUNTER
            let result = step_inner(&mut cpu, &mut bus);
            if matches!(opcode, 0x08 | 0x09) {
                result.unwrap();
                assert_eq!(cpu.rip, (PM32_CODE + 2) as u64);
            } else {
                assert_arch_fault(result, 13, Some(0));
            }

            // CPL 3 faults before anything else is considered.
            let (mut cpu, mut bus) = pm32_fixture(&[0x0F, opcode], PM32_CODE, true);
            cpu.cs.selector |= 3;
            let before = cpu.clone();
            assert_arch_fault(step_inner(&mut cpu, &mut bus), 13, Some(0));
            assert_eq!(cpu, before, "0F {opcode:02X} at CPL 3");
        }
    }

    /// Intel SDM Vol. 2 "UD2—Undefined Instruction": raises `#UD` in every
    /// operating mode, and must not be reported as a host decode gap.
    #[test]
    fn ud2_raises_undefined_opcode() {
        let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, 0x0B], |_, _| {});
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 6, None);
        assert_eq!(cpu, before);

        let (mut cpu, mut bus) = pm32_fixture(&[0x0F, 0x0B], PM32_CODE, true);
        let before = cpu.clone();
        assert_arch_fault(step_inner(&mut cpu, &mut bus), 6, None);
        assert_eq!(cpu, before);
    }

    /// Intel SDM Vol. 2 "INVD"/"WBINVD": cache maintenance only. This emulator
    /// models no caches, so both retire as no-ops that change nothing but the
    /// instruction pointer.
    #[test]
    fn invd_and_wbinvd_are_architectural_no_ops() {
        for opcode in [0x08u8, 0x09] {
            let flags = 0x0002 | 1 | (1 << 6);
            let (mut cpu, mut bus) = real_mode_fixture(&[0x0F, opcode], |cpu, _| {
                cpu.rflags = flags;
                cpu.set_gpr_u32(CpuState::RAX, 0x1234_5678);
            });
            let before = cpu.clone();
            step(&mut cpu, &mut bus).unwrap();
            assert_eq!(cpu.ip16(), 2);
            assert_eq!(cpu.rflags, flags);
            assert_eq!(cpu.gpr, before.gpr);
            assert_eq!(cpu.cr0, before.cr0);
        }
    }
}
