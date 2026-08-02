//! Native CLI: run a ROM (default: built-in HELLO ROM) until HLT.

use machine_pc::{build_hello_rom, Machine, EXPECTED_HELLO};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let mut rom_path: Option<String> = None;
    let mut max_steps: u64 = 100_000;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rom" => rom_path = args.next(),
            "--steps" => {
                if let Some(v) = args.next() {
                    max_steps = v.parse().unwrap_or(max_steps);
                }
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: emulator-cli [--rom path.bin] [--steps N]\n\
                     Default ROM prints '{EXPECTED_HELLO}' via COM1 and port 0x402."
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let mut machine = Machine::new(16 * 1024 * 1024);
    let using_hello = rom_path.is_none();
    if let Some(path) = rom_path {
        match fs::read(&path) {
            Ok(data) => {
                if let Err(e) = machine.load_rom(&data) {
                    eprintln!("Failed to load ROM: {e}");
                    return ExitCode::FAILURE;
                }
            }
            Err(e) => {
                eprintln!("Failed to read ROM {path}: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else if let Err(e) = machine.load_rom(&build_hello_rom()) {
        eprintln!("Failed to load HELLO ROM: {e}");
        return ExitCode::FAILURE;
    }

    machine.reset();
    match machine.run(max_steps) {
        Ok(steps) => {
            let com1 = machine.com1_text();
            let dbg = machine.debug_text();
            println!("steps={steps} halted={}", machine.cpu.halted);
            println!("COM1:{com1}");
            println!("DEBUG:{dbg}");
            if using_hello && (com1 != EXPECTED_HELLO || dbg != EXPECTED_HELLO) {
                eprintln!("HELLO ROM output mismatch");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Execution error: {e}");
            ExitCode::FAILURE
        }
    }
}
