//! Typed IR for a future Wasm JIT. Milestone 1 keeps this as a stub so the
//! workspace layout matches `plan.md` without inventing JIT semantics.

#![forbid(unsafe_code)]

/// Placeholder IR opcode tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrOp {
    Nop,
}

/// Placeholder instruction node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IrInst {
    pub op: IrOp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_exists() {
        let i = IrInst { op: IrOp::Nop };
        assert_eq!(i.op, IrOp::Nop);
    }
}
