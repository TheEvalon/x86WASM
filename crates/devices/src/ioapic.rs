//! I/O APIC MMIO with RTE → interrupt delivery stub (classic base `0xFEC0_0000`).
//!
//! Spec: Intel 82093AA I/O Advanced Programmable Interrupt Controller:
//! - `IOREGSEL` at offset `00h` (index)
//! - `IOWIN` at offset `10h` (data window)
//! - Indirect: `IOAPICID` / `IOAPICVER` / `IOAPICARB`
//! - Redirection table from index `10h` (two dwords per IRQ; 24 entries)
//!
//! Round-7: unmasked RTE entries can deliver a Fixed-mode vector to a
//! guest-visible path via [`IoApicMmio::assert_pin`] → [`IoApicDelivery`].
//! Round-8: level-triggered Fixed deliveries set Remote IRR (RTE bit 14) and
//! suppress re-issue until [`IoApicMmio::eoi`] for that vector. Round-10:
//! non-Fixed delivery modes (SMI/NMI/ExtINT/LowestPriority/INIT) are stored for
//! probe honesty but do not deliver; see [`IoApicMmio::take_unsupported_delivery`].
//! Machine wiring latches the vector on the Local APIC when the destination
//! APIC ID matches. DualPic / ExtINT virtual-wire is **not** auto-mirrored.
//! See `docs/ioapic-r7-rte-irq.md`, `docs/ioapic-r8-eoi.md`,
//! `docs/ioapic-r10-delivery-mode.md`.

/// Classic I/O APIC physical base (PC AT / ACPI MADT convention).
pub const IOAPIC_DEFAULT_BASE: u64 = 0xFEC0_0000;

/// I/O APIC MMIO window size (4 KiB page).
pub const IOAPIC_WINDOW_SIZE: u64 = 0x1000;

/// IOREGSEL — select register offset.
pub const IOAPIC_IOREGSEL: u32 = 0x00;

/// IOWIN — window register offset.
pub const IOAPIC_IOWIN: u32 = 0x10;

/// Indirect: IOAPICID.
pub const IOAPIC_IND_ID: u8 = 0x00;

/// Indirect: IOAPICVER.
pub const IOAPIC_IND_VER: u8 = 0x01;

/// Indirect: IOAPICARB.
pub const IOAPIC_IND_ARB: u8 = 0x02;

/// First redirection-table index (low dword of entry 0).
pub const IOAPIC_IND_REDTBL0: u8 = 0x10;

/// Version field (82093AA-class = `0x11`).
pub const IOAPIC_VERSION_ID: u8 = 0x11;

/// Max Redirection Entry (bits 23:16). `0x17` → 24 entries (0..=23).
pub const IOAPIC_MAX_REDIRECTION_ENTRY: u8 = 0x17;

/// Number of redirection entries (`MaxREDTBL + 1`).
pub const IOAPIC_REDIRECTION_COUNT: usize = (IOAPIC_MAX_REDIRECTION_ENTRY as usize) + 1;

/// Composed IOAPICVER reset value.
pub const IOAPIC_VER_VALUE: u32 =
    (IOAPIC_VERSION_ID as u32) | ((IOAPIC_MAX_REDIRECTION_ENTRY as u32) << 16);

/// RTE low: interrupt vector (bits 7:0).
pub const IOAPIC_RTE_VECTOR_MASK: u32 = 0xFF;

/// RTE low: delivery mode (bits 10:8). Fixed = `000`.
pub const IOAPIC_RTE_DELIVERY_SHIFT: u32 = 8;

/// RTE low: delivery mode mask.
pub const IOAPIC_RTE_DELIVERY_MASK: u32 = 0x7 << IOAPIC_RTE_DELIVERY_SHIFT;

/// Fixed delivery mode.
pub const IOAPIC_DELIVERY_FIXED: u32 = 0;

/// Lowest Priority delivery mode (unsupported in this stub).
pub const IOAPIC_DELIVERY_LOWEST: u32 = 1;

/// SMI delivery mode (unsupported in this stub).
pub const IOAPIC_DELIVERY_SMI: u32 = 2;

/// NMI delivery mode (unsupported in this stub).
pub const IOAPIC_DELIVERY_NMI: u32 = 4;

/// INIT delivery mode (unsupported in this stub).
pub const IOAPIC_DELIVERY_INIT: u32 = 5;

/// ExtINT delivery mode (unsupported in this stub).
pub const IOAPIC_DELIVERY_EXTINT: u32 = 7;

/// True when `mode` is Fixed (the only delivery mode this stub implements).
pub fn ioapic_delivery_mode_supported(mode: u32) -> bool {
    mode == IOAPIC_DELIVERY_FIXED
}

/// Human-readable name for a known 82093AA delivery mode, if recognized.
pub fn ioapic_delivery_mode_name(mode: u32) -> Option<&'static str> {
    match mode {
        IOAPIC_DELIVERY_FIXED => Some("Fixed"),
        IOAPIC_DELIVERY_LOWEST => Some("LowestPriority"),
        IOAPIC_DELIVERY_SMI => Some("SMI"),
        IOAPIC_DELIVERY_NMI => Some("NMI"),
        IOAPIC_DELIVERY_INIT => Some("INIT"),
        IOAPIC_DELIVERY_EXTINT => Some("ExtINT"),
        _ => None,
    }
}

/// RTE low: interrupt mask (bit 16).
pub const IOAPIC_RTE_MASK: u32 = 1 << 16;

/// RTE low: trigger mode (bit 15): 0 = edge, 1 = level.
pub const IOAPIC_RTE_LEVEL: u32 = 1 << 15;

/// RTE low: Remote IRR (bit 14, RO for software). Spec: 82093AA — set when a
/// level-triggered Fixed interrupt is delivered; cleared by EOI for that vector.
pub const IOAPIC_RTE_REMOTE_IRR: u32 = 1 << 14;

/// RTE high: destination APIC ID in bits 63:56 of the 64-bit RTE
/// (bits 31:24 of the high dword) for physical destination mode.
pub const IOAPIC_RTE_DEST_SHIFT: u32 = 24;

/// Fixed-mode delivery produced by an unmasked RTE when a pin asserts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApicDelivery {
    /// Global system interrupt / input pin index.
    pub gsi: u8,
    /// Vector from the RTE low dword.
    pub vector: u8,
    /// Physical destination APIC ID from the RTE high dword.
    pub dest_apic_id: u8,
}

/// Recorded when an unmasked non-Fixed RTE would have fired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoApicUnsupportedDelivery {
    /// GSI / input pin that asserted.
    pub gsi: u8,
    /// Delivery mode field (bits 10:8 of RTE low).
    pub mode: u32,
}

/// I/O APIC MMIO with redirection-table store/readback + pin delivery stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoApicMmio {
    base: u64,
    /// IOREGSEL index (bits 7:0).
    index: u8,
    /// APIC ID in bits 27:24 of IOAPICID (82093AA).
    apic_id: u8,
    /// Redirection table: 24 entries × 2 dwords (low, high).
    redtbl: [u32; IOAPIC_REDIRECTION_COUNT * 2],
    /// Latched pin levels (for edge/level semantics).
    pin_level: [bool; IOAPIC_REDIRECTION_COUNT],
    /// Last unmasked non-Fixed assert (honesty probe); cleared by take.
    unsupported_delivery: Option<IoApicUnsupportedDelivery>,
    /// Scratch for assembling IOWIN dword writes.
    iowin_scratch: [u8; 4],
}

impl Default for IoApicMmio {
    fn default() -> Self {
        Self::new()
    }
}

impl IoApicMmio {
    pub fn new() -> Self {
        let mut redtbl = [0; IOAPIC_REDIRECTION_COUNT * 2];
        // Spec: 82093AA — RTE mask bit is set at reset.
        for i in 0..IOAPIC_REDIRECTION_COUNT {
            redtbl[i * 2] = IOAPIC_RTE_MASK;
        }
        Self {
            base: IOAPIC_DEFAULT_BASE,
            index: 0,
            apic_id: 0,
            redtbl,
            pin_level: [false; IOAPIC_REDIRECTION_COUNT],
            unsupported_delivery: None,
            iowin_scratch: [0; 4],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn index(&self) -> u8 {
        self.index
    }

    pub fn owns(&self, addr: u64) -> bool {
        (self.base..self.base.saturating_add(IOAPIC_WINDOW_SIZE)).contains(&addr)
    }

    /// Low dword of redirection entry `gsi`.
    pub fn redtbl_low(&self, gsi: u8) -> Option<u32> {
        let i = gsi as usize;
        if i >= IOAPIC_REDIRECTION_COUNT {
            return None;
        }
        Some(self.redtbl[i * 2])
    }

    /// High dword of redirection entry `gsi`.
    pub fn redtbl_high(&self, gsi: u8) -> Option<u32> {
        let i = gsi as usize;
        if i >= IOAPIC_REDIRECTION_COUNT {
            return None;
        }
        Some(self.redtbl[i * 2 + 1])
    }

    fn id_value(&self) -> u32 {
        u32::from(self.apic_id) << 24
    }

    fn indirect_read(&self) -> u32 {
        match self.index {
            IOAPIC_IND_ID => self.id_value(),
            IOAPIC_IND_VER => IOAPIC_VER_VALUE,
            IOAPIC_IND_ARB => self.id_value(),
            idx if idx >= IOAPIC_IND_REDTBL0 => {
                let entry = (idx - IOAPIC_IND_REDTBL0) as usize;
                if entry < self.redtbl.len() {
                    self.redtbl[entry]
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    fn indirect_write(&mut self, value: u32) {
        match self.index {
            IOAPIC_IND_ID => {
                // Spec: 82093AA IOAPICID — APIC ID in bits 27:24.
                self.apic_id = ((value >> 24) & 0x0F) as u8;
            }
            IOAPIC_IND_VER => {} // RO
            IOAPIC_IND_ARB => {} // RO in this stub
            idx if idx >= IOAPIC_IND_REDTBL0 => {
                let entry = (idx - IOAPIC_IND_REDTBL0) as usize;
                if entry < self.redtbl.len() {
                    // Remote IRR is hardware-owned on low dwords (even indices).
                    if entry.is_multiple_of(2) {
                        let remote = self.redtbl[entry] & IOAPIC_RTE_REMOTE_IRR;
                        self.redtbl[entry] = (value & !IOAPIC_RTE_REMOTE_IRR) | remote;
                    } else {
                        self.redtbl[entry] = value;
                    }
                }
            }
            _ => {}
        }
    }

    fn try_deliver(&mut self, gsi: u8) -> Option<IoApicDelivery> {
        let low = self.redtbl_low(gsi)?;
        let high = self.redtbl_high(gsi)?;
        if low & IOAPIC_RTE_MASK != 0 {
            return None;
        }
        let delivery = (low & IOAPIC_RTE_DELIVERY_MASK) >> IOAPIC_RTE_DELIVERY_SHIFT;
        if !ioapic_delivery_mode_supported(delivery) {
            // Spec: 82093AA defines SMI/NMI/INIT/ExtINT/LowestPriority; this
            // stub stores the RTE for probe honesty but does not invent an
            // APIC bus / dual-PIC ExtINT path. Record for hosts/tests.
            self.unsupported_delivery = Some(IoApicUnsupportedDelivery {
                gsi,
                mode: delivery,
            });
            return None;
        }
        Some(IoApicDelivery {
            gsi,
            vector: (low & IOAPIC_RTE_VECTOR_MASK) as u8,
            dest_apic_id: ((high >> IOAPIC_RTE_DEST_SHIFT) & 0xFF) as u8,
        })
    }

    /// Drive I/O APIC input pin `gsi`.
    ///
    /// Spec: 82093AA — edge triggers on rising edge; level delivers while high
    /// and unmasked, but Remote IRR suppresses further level issues until EOI.
    /// Returns a Fixed delivery when the RTE accepts the event. Level Fixed
    /// success sets Remote IRR. Non-Fixed modes latch
    /// [`Self::take_unsupported_delivery`] and return `None`. Does **not**
    /// talk to DualPic.
    pub fn assert_pin(&mut self, gsi: u8, high: bool) -> Option<IoApicDelivery> {
        let idx = gsi as usize;
        if idx >= IOAPIC_REDIRECTION_COUNT {
            return None;
        }
        let prev = self.pin_level[idx];
        self.pin_level[idx] = high;
        let low = self.redtbl[idx * 2];
        let level_trig = low & IOAPIC_RTE_LEVEL != 0;
        let should_try = if level_trig {
            high && low & IOAPIC_RTE_REMOTE_IRR == 0
        } else {
            !prev && high
        };
        if !should_try {
            return None;
        }
        let delivery = self.try_deliver(gsi)?;
        if level_trig {
            self.redtbl[idx * 2] |= IOAPIC_RTE_REMOTE_IRR;
        }
        Some(delivery)
    }

    /// Peek the last unmasked non-Fixed assert, if any.
    pub fn unsupported_delivery(&self) -> Option<IoApicUnsupportedDelivery> {
        self.unsupported_delivery
    }

    /// Take (clear) the last unmasked non-Fixed assert record.
    pub fn take_unsupported_delivery(&mut self) -> Option<IoApicUnsupportedDelivery> {
        self.unsupported_delivery.take()
    }

    /// Delivery mode field from RTE low for `gsi` (bits 10:8).
    pub fn delivery_mode(&self, gsi: u8) -> Option<u32> {
        self.redtbl_low(gsi)
            .map(|low| (low & IOAPIC_RTE_DELIVERY_MASK) >> IOAPIC_RTE_DELIVERY_SHIFT)
    }

    /// Clear Remote IRR on every level-triggered RTE matching `vector`.
    ///
    /// Spec: 82093AA — EOI for a level interrupt clears Remote IRR; the pin may
    /// then re-deliver if still asserted and unmasked. Edge RTEs are untouched.
    pub fn eoi(&mut self, vector: u8) {
        for gsi in 0..IOAPIC_REDIRECTION_COUNT {
            let low = self.redtbl[gsi * 2];
            if low & IOAPIC_RTE_LEVEL == 0 {
                continue;
            }
            if (low & IOAPIC_RTE_VECTOR_MASK) as u8 != vector {
                continue;
            }
            self.redtbl[gsi * 2] &= !IOAPIC_RTE_REMOTE_IRR;
        }
    }

    /// True when Remote IRR is set on redirection entry `gsi`.
    pub fn remote_irr(&self, gsi: u8) -> bool {
        self.redtbl_low(gsi)
            .map(|low| low & IOAPIC_RTE_REMOTE_IRR != 0)
            .unwrap_or(false)
    }

    /// Byte read within the claimed window, or `None` if unclaimed.
    pub fn mmio_read_u8(&self, addr: u64) -> Option<u8> {
        if !self.owns(addr) {
            return None;
        }
        let off = (addr - self.base) as u32;
        let dword_off = off & !3;
        let lane = (off & 3) as usize;
        let value = match dword_off {
            IOAPIC_IOREGSEL => u32::from(self.index),
            IOAPIC_IOWIN => self.indirect_read(),
            _ => 0,
        };
        Some(value.to_le_bytes()[lane])
    }

    /// Byte write within the claimed window.
    pub fn mmio_write_u8(&mut self, addr: u64, val: u8) -> bool {
        if !self.owns(addr) {
            return false;
        }
        let off = (addr - self.base) as u32;
        let dword_off = off & !3;
        let lane = (off & 3) as usize;
        match dword_off {
            IOAPIC_IOREGSEL if lane == 0 => {
                self.index = val;
            }
            IOAPIC_IOWIN => {
                self.iowin_scratch = self.indirect_read().to_le_bytes();
                self.iowin_scratch[lane] = val;
                self.indirect_write(u32::from_le_bytes(self.iowin_scratch));
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32(io: &IoApicMmio, off: u32) -> u32 {
        let mut b = [0u8; 4];
        for i in 0..4u64 {
            b[i as usize] = io
                .mmio_read_u8(IOAPIC_DEFAULT_BASE + u64::from(off) + i)
                .unwrap();
        }
        u32::from_le_bytes(b)
    }

    fn write_u32(io: &mut IoApicMmio, off: u32, value: u32) {
        for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
            assert!(io.mmio_write_u8(IOAPIC_DEFAULT_BASE + u64::from(off) + i as u64, byte));
        }
    }

    fn select(io: &mut IoApicMmio, index: u8) {
        assert!(io.mmio_write_u8(IOAPIC_DEFAULT_BASE + u64::from(IOAPIC_IOREGSEL), index));
    }

    fn write_rte(io: &mut IoApicMmio, gsi: u8, low: u32, high: u32) {
        let idx = IOAPIC_IND_REDTBL0 + gsi * 2;
        select(io, idx);
        write_u32(io, IOAPIC_IOWIN, low);
        select(io, idx + 1);
        write_u32(io, IOAPIC_IOWIN, high);
    }

    /// Spec: 82093AA — IOREGSEL/IOWIN + IOAPICVER MaxREDTBL=0x17 (24 entries).
    #[test]
    fn id_and_version_via_index_window() {
        let mut io = IoApicMmio::new();
        assert!(io.owns(IOAPIC_DEFAULT_BASE));
        assert!(!io.owns(IOAPIC_DEFAULT_BASE + IOAPIC_WINDOW_SIZE));

        select(&mut io, IOAPIC_IND_ID);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0);

        select(&mut io, IOAPIC_IND_VER);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), IOAPIC_VER_VALUE);
        assert_eq!(IOAPIC_VER_VALUE as u8, IOAPIC_VERSION_ID);
        assert_eq!((IOAPIC_VER_VALUE >> 16) as u8, IOAPIC_MAX_REDIRECTION_ENTRY);
        assert_eq!(read_u32(&io, IOAPIC_IOREGSEL) as u8, IOAPIC_IND_VER);
    }

    /// Spec: 82093AA — redirection table store/readback; reset entries masked.
    #[test]
    fn redirection_table_store_readback_masked_reset() {
        let mut io = IoApicMmio::new();
        assert_eq!(io.redtbl_low(0), Some(IOAPIC_RTE_MASK));

        write_rte(&mut io, 0, 0x0001_00EF, 0x0100_0000);
        select(&mut io, IOAPIC_IND_REDTBL0);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0x0001_00EF);
        select(&mut io, IOAPIC_IND_REDTBL0 + 1);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0x0100_0000);

        let last_low = IOAPIC_IND_REDTBL0 + (23 * 2);
        select(&mut io, last_low);
        write_u32(&mut io, IOAPIC_IOWIN, 0xDEAD_BEEF);
        select(&mut io, last_low);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0xDEAD_BEEF);
    }

    #[test]
    fn id_bits_store_and_reset() {
        let mut io = IoApicMmio::new();
        select(&mut io, IOAPIC_IND_ID);
        write_u32(&mut io, IOAPIC_IOWIN, 0x0A00_0000);
        select(&mut io, IOAPIC_IND_ID);
        assert_eq!(read_u32(&io, IOAPIC_IOWIN), 0x0A00_0000);
        io.reset();
        assert_eq!(io, IoApicMmio::new());
    }

    /// Spec: 82093AA — unmasked edge RTE delivers Fixed vector on rising edge.
    #[test]
    fn unmasked_edge_rte_delivers_on_rising_edge() {
        let mut io = IoApicMmio::new();
        // Vector 0x30, Fixed, edge, unmasked; dest APIC ID 1.
        write_rte(&mut io, 5, 0x0000_0030, 0x0100_0000);
        assert!(io.assert_pin(5, false).is_none());
        let d = io.assert_pin(5, true).expect("rising edge delivery");
        assert_eq!(
            d,
            IoApicDelivery {
                gsi: 5,
                vector: 0x30,
                dest_apic_id: 1,
            }
        );
        // Flat high does not re-fire (edge).
        assert!(io.assert_pin(5, true).is_none());
        // Masked: no delivery.
        write_rte(&mut io, 5, IOAPIC_RTE_MASK | 0x30, 0x0100_0000);
        assert!(io.assert_pin(5, false).is_none());
        assert!(io.assert_pin(5, true).is_none());
    }

    /// Spec: 82093AA — level RTE delivers while pin is high and unmasked;
    /// Remote IRR suppresses re-issue until EOI.
    #[test]
    fn level_rte_sets_remote_irr_until_eoi() {
        let mut io = IoApicMmio::new();
        write_rte(&mut io, 3, IOAPIC_RTE_LEVEL | 0x41, 0x0200_0000);
        let d = io.assert_pin(3, true).expect("level delivery");
        assert_eq!(d.vector, 0x41);
        assert_eq!(d.dest_apic_id, 2);
        assert!(io.remote_irr(3));
        // Still high → suppressed by Remote IRR.
        assert!(io.assert_pin(3, true).is_none());
        // Software cannot clear Remote IRR via RTE write.
        write_rte(&mut io, 3, IOAPIC_RTE_LEVEL | 0x41, 0x0200_0000);
        assert!(io.remote_irr(3));
        // EOI for vector clears Remote IRR; pin still high → re-delivers.
        io.eoi(0x41);
        assert!(!io.remote_irr(3));
        assert!(io.assert_pin(3, true).is_some());
        assert!(io.remote_irr(3));
        io.eoi(0x41);
        assert!(!io.remote_irr(3));
        assert!(io.assert_pin(3, false).is_none());
    }

    #[test]
    fn edge_rte_never_sets_remote_irr() {
        let mut io = IoApicMmio::new();
        write_rte(&mut io, 5, 0x0000_0030, 0x0100_0000);
        assert!(io.assert_pin(5, true).is_some());
        assert!(!io.remote_irr(5));
        io.eoi(0x30);
        assert!(!io.remote_irr(5));
    }

    /// Spec: 82093AA delivery modes — non-Fixed (SMI/NMI/ExtINT/Lowest/INIT)
    /// store/readback but do not deliver; Fixed continues to work.
    #[test]
    fn non_fixed_delivery_modes_unsupported_fixed_still_works() {
        let mut io = IoApicMmio::new();
        let modes = [
            IOAPIC_DELIVERY_LOWEST,
            IOAPIC_DELIVERY_SMI,
            IOAPIC_DELIVERY_NMI,
            IOAPIC_DELIVERY_INIT,
            IOAPIC_DELIVERY_EXTINT,
        ];
        for (i, mode) in modes.iter().enumerate() {
            let gsi = i as u8;
            let low = (mode << IOAPIC_RTE_DELIVERY_SHIFT) | 0x50;
            write_rte(&mut io, gsi, low, 0);
            assert_eq!(io.delivery_mode(gsi), Some(*mode));
            assert!(!ioapic_delivery_mode_supported(*mode));
            assert!(ioapic_delivery_mode_name(*mode).is_some());
            assert!(io.assert_pin(gsi, true).is_none());
            assert_eq!(
                io.take_unsupported_delivery(),
                Some(IoApicUnsupportedDelivery { gsi, mode: *mode })
            );
            // No Remote IRR for non-Fixed.
            assert!(!io.remote_irr(gsi));
        }

        // Fixed still delivers on another pin.
        write_rte(&mut io, 10, 0x0000_0060, 0x0300_0000);
        let d = io.assert_pin(10, true).expect("Fixed delivery");
        assert_eq!(d.vector, 0x60);
        assert_eq!(d.dest_apic_id, 3);
        assert!(io.unsupported_delivery().is_none());
    }
}
