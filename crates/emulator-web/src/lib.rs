//! Wasm bindings: run the HELLO ROM (or supplied bytes) and return serial text.

use machine_pc::{build_hello_rom, Machine, EXPECTED_HELLO};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Run the built-in HELLO ROM; returns COM1 output.
#[wasm_bindgen]
pub fn run_hello_rom(max_steps: u32) -> Result<String, JsValue> {
    let mut m = Machine::new(4 * 1024 * 1024);
    m.load_rom(&build_hello_rom())
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    m.reset();
    m.run(u64::from(max_steps))
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let com1 = m.com1_text();
    let dbg = m.debug_text();
    if com1 != EXPECTED_HELLO && dbg != EXPECTED_HELLO {
        return Err(JsValue::from_str(&format!(
            "unexpected output com1={com1:?} debug={dbg:?}"
        )));
    }
    Ok(com1)
}

/// Run a guest ROM image (raw bytes) until HLT / step limit. Returns COM1 text.
#[wasm_bindgen]
pub fn run_rom(rom: &[u8], max_steps: u32) -> Result<String, JsValue> {
    let mut m = Machine::new(4 * 1024 * 1024);
    m.load_rom(rom)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    m.reset();
    m.run(u64::from(max_steps))
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(m.com1_text())
}

#[wasm_bindgen]
pub fn expected_hello() -> String {
    EXPECTED_HELLO.to_string()
}
