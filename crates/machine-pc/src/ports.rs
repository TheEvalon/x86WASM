//! Open bus for unclaimed port I/O, plus bounded POST-probe diagnostics.
//!
//! Reads of a port no device claims return all-ones (ISA open bus) and writes
//! are dropped. While probe recording is armed (see [`PortBus::set_probe`]),
//! those accesses — and CPU accesses to physical addresses outside RAM and
//! every ROM window — are logged so
//! [`crate::Machine::probe_post`] can report what firmware touched that this
//! machine does not model.

use devices::PortDevice;

/// Distinct `(port, direction, size)` records kept per probe run.
pub const UNCLAIMED_PORT_LIMIT: usize = 64;
/// Distinct `(page, direction)` records kept per probe run.
pub const UNMAPPED_MMIO_LIMIT: usize = 32;
/// Granularity used to fold unmapped physical accesses into pages.
pub const UNMAPPED_MMIO_PAGE_SIZE: u64 = 4096;

/// One unclaimed port I/O site observed during a probe run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnclaimedPortAccess {
    pub port: u16,
    /// `true` for `OUT`, `false` for `IN`.
    pub write: bool,
    /// Access width in bytes (1, 2, or 4).
    pub size: u8,
    /// Value of the first write (0 for reads).
    pub first_value: u32,
    pub count: u32,
}

/// One unmapped physical page touched during a probe run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnmappedMmioAccess {
    /// Page base address ([`UNMAPPED_MMIO_PAGE_SIZE`] granular).
    pub page: u64,
    pub write: bool,
    pub count: u32,
}

#[derive(Default)]
pub struct PortBus {
    probe: bool,
    unclaimed: Vec<UnclaimedPortAccess>,
    unclaimed_overflow: bool,
    unmapped_mmio: Vec<UnmappedMmioAccess>,
    unmapped_mmio_overflow: bool,
}

impl PortBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm or disarm diagnostic recording. Disarmed is the normal run state, so
    /// ordinary execution pays nothing for the probe.
    pub fn set_probe(&mut self, probe: bool) {
        self.probe = probe;
    }

    pub fn probe_enabled(&self) -> bool {
        self.probe
    }

    pub fn clear_diagnostics(&mut self) {
        self.unclaimed.clear();
        self.unclaimed_overflow = false;
        self.unmapped_mmio.clear();
        self.unmapped_mmio_overflow = false;
    }

    pub fn unclaimed_ports(&self) -> &[UnclaimedPortAccess] {
        &self.unclaimed
    }

    pub fn unclaimed_port_overflow(&self) -> bool {
        self.unclaimed_overflow
    }

    pub fn unmapped_mmio(&self) -> &[UnmappedMmioAccess] {
        &self.unmapped_mmio
    }

    pub fn unmapped_mmio_overflow(&self) -> bool {
        self.unmapped_mmio_overflow
    }

    /// Record a CPU access to a physical address that decoded to neither RAM
    /// nor a ROM window. No-op unless probe recording is armed.
    pub fn record_unmapped_mmio(&mut self, addr: u64, write: bool) {
        if !self.probe {
            return;
        }
        let page = addr & !(UNMAPPED_MMIO_PAGE_SIZE - 1);
        if let Some(hit) = self
            .unmapped_mmio
            .iter_mut()
            .find(|a| a.page == page && a.write == write)
        {
            hit.count = hit.count.saturating_add(1);
            return;
        }
        if self.unmapped_mmio.len() >= UNMAPPED_MMIO_LIMIT {
            self.unmapped_mmio_overflow = true;
            return;
        }
        self.unmapped_mmio.push(UnmappedMmioAccess {
            page,
            write,
            count: 1,
        });
    }

    fn record_port(&mut self, port: u16, size: u8, write: bool, value: u32) {
        if !self.probe {
            return;
        }
        if let Some(hit) = self
            .unclaimed
            .iter_mut()
            .find(|a| a.port == port && a.write == write && a.size == size)
        {
            hit.count = hit.count.saturating_add(1);
            return;
        }
        if self.unclaimed.len() >= UNCLAIMED_PORT_LIMIT {
            self.unclaimed_overflow = true;
            return;
        }
        self.unclaimed.push(UnclaimedPortAccess {
            port,
            write,
            size,
            first_value: value,
            count: 1,
        });
    }
}

impl PortDevice for PortBus {
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        self.record_port(port, size, false, 0);
        match size {
            1 => 0xFF,
            2 => 0xFFFF,
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        self.record_port(port, size, true, value);
    }
}
