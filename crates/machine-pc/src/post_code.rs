//! POST checkpoint (manufacturing diagnostic) port `0x80`.
//!
//! Spec: IBM PC/AT Technical Reference — the system board decodes `0x80` as the
//! manufacturing diagnostic port. POST writes a checkpoint code there before
//! each test phase so a diagnostic ("POST") card can display the code of the
//! step that failed. The board defines no read data for the port, so reads stay
//! ISA open bus here; the same port is also the classic I/O-delay target, which
//! is why the write count is tracked separately from the code history.
//!
//! This latch exists so the host can answer "how far did POST get". It has no
//! guest-visible side effects beyond claiming the port from the open-bus
//! fallback.

use devices::PortDevice;

/// Manufacturing diagnostic / POST checkpoint port.
pub const POST_DIAG_PORT: u16 = 0x80;

/// Checkpoint codes retained per run before [`PostCodePort::history_overflow`].
pub const POST_CODE_HISTORY_LIMIT: usize = 256;

/// Host-readable POST checkpoint latch.
#[derive(Clone, Debug, Default)]
pub struct PostCodePort {
    last: Option<u8>,
    history: Vec<u8>,
    writes: u64,
    overflow: bool,
}

impl PostCodePort {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the latch and history (power-on / system reset).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Most recent checkpoint code, or `None` before the first write.
    pub fn last_code(&self) -> Option<u8> {
        self.last
    }

    /// Checkpoint codes in write order, truncated at [`POST_CODE_HISTORY_LIMIT`].
    pub fn history(&self) -> &[u8] {
        &self.history
    }

    /// Total writes, including those dropped after the history filled up.
    pub fn write_count(&self) -> u64 {
        self.writes
    }

    /// Whether more codes were written than the bounded history holds.
    pub fn history_overflow(&self) -> bool {
        self.overflow
    }

    pub fn owns_port(port: u16) -> bool {
        port == POST_DIAG_PORT
    }
}

impl PortDevice for PostCodePort {
    /// Open bus — the AT system board drives no data for a `0x80` read.
    fn port_read(&mut self, _port: u16, size: u8) -> u32 {
        match size {
            1 => 0xFF,
            2 => 0xFFFF,
            _ => 0xFFFF_FFFF,
        }
    }

    /// Latch the checkpoint code. The port is byte wide, so a wider access
    /// contributes its low byte only.
    fn port_write(&mut self, port: u16, _size: u8, value: u32) {
        if !Self::owns_port(port) {
            return;
        }
        let code = value as u8;
        self.last = Some(code);
        self.writes = self.writes.saturating_add(1);
        if self.history.len() < POST_CODE_HISTORY_LIMIT {
            self.history.push(code);
        } else {
            self.overflow = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec: IBM PC/AT — `0x80` latches a POST checkpoint; reads are open bus.
    #[test]
    fn write_latches_code_and_read_is_open_bus() {
        let mut p = PostCodePort::new();
        assert_eq!(p.last_code(), None);
        assert_eq!(p.port_read(POST_DIAG_PORT, 1), 0xFF);

        p.port_write(POST_DIAG_PORT, 1, 0x2C);

        assert_eq!(p.last_code(), Some(0x2C));
        assert_eq!(p.history(), [0x2C]);
        assert_eq!(p.write_count(), 1);
        assert_eq!(p.port_read(POST_DIAG_PORT, 1), 0xFF);
    }

    #[test]
    fn ignores_ports_it_does_not_own() {
        let mut p = PostCodePort::new();
        p.port_write(0x81, 1, 0x11);
        assert_eq!(p.last_code(), None);
        assert!(!PostCodePort::owns_port(0x81));
    }

    #[test]
    fn history_is_bounded_but_last_code_and_count_keep_tracking() {
        let mut p = PostCodePort::new();
        for i in 0..(POST_CODE_HISTORY_LIMIT + 2) {
            p.port_write(POST_DIAG_PORT, 1, i as u32);
        }
        assert_eq!(p.history().len(), POST_CODE_HISTORY_LIMIT);
        assert!(p.history_overflow());
        assert_eq!(p.write_count(), POST_CODE_HISTORY_LIMIT as u64 + 2);
        assert_eq!(p.last_code(), Some((POST_CODE_HISTORY_LIMIT + 1) as u8));

        p.reset();
        assert_eq!(p.last_code(), None);
        assert!(p.history().is_empty());
        assert_eq!(p.write_count(), 0);
    }
}
