//! Native CLI: run a ROM/BIOS (default: built-in HELLO ROM) until HLT.

use emulator_cli::{
    build_machine, parse_args, run_machine, run_post_probe, usage, CliError, ParsedArgs,
};
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match parse_args(env::args().skip(1)) {
        Ok(ParsedArgs::Help) => {
            eprintln!("{}", usage());
            ExitCode::SUCCESS
        }
        Ok(ParsedArgs::Run(opts)) if opts.post_probe => match build_machine(&opts) {
            Ok((mut machine, _kind)) => {
                println!("{}", run_post_probe(&mut machine, opts.max_steps));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Ok(ParsedArgs::Run(opts)) => match build_machine(&opts) {
            Ok((mut machine, kind)) => match run_machine(&mut machine, kind, opts.max_steps) {
                Ok((steps, com1, dbg)) => {
                    println!("steps={steps} halted={}", machine.cpu.halted);
                    println!("COM1:{com1}");
                    println!("DEBUG:{dbg}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                eprintln!("{e}");
                match e {
                    CliError::UnknownArgument(_) => ExitCode::from(2),
                    _ => ExitCode::FAILURE,
                }
            }
        },
        Err(e) => {
            eprintln!("{e}");
            match e {
                CliError::UnknownArgument(_) | CliError::RomAndBios => ExitCode::from(2),
                CliError::MissingValue(_) | CliError::InvalidSteps(_) => ExitCode::from(2),
                _ => ExitCode::FAILURE,
            }
        }
    }
}
