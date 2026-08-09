//! Native CLI: run a ROM/BIOS (default: built-in HELLO ROM) until HLT.

use emulator_cli::{
    build_machine, parse_args, run_machine, run_post_probe_options, usage, vga_frame_report,
    vga_text_dump, BuiltMachine, CliError, Options, ParsedArgs,
};
use machine_pc::Machine;
use std::env;
use std::process::ExitCode;

/// Diagnostics are printed after the stable run / `--post-probe` output so the
/// existing formats stay byte-identical when the new flags are not used.
fn print_diagnostics(machine: &Machine, built_option_rom: Option<String>, opts: &Options) {
    if let Some(line) = built_option_rom {
        println!("{line}");
    }
    if opts.vga_text {
        println!("{}", vga_text_dump(machine));
    }
    if opts.vga_frame {
        println!("{}", vga_frame_report(machine, false));
    }
}

fn arg_exit_code(e: &CliError) -> ExitCode {
    match e {
        CliError::UnknownArgument(_)
        | CliError::RomAndBios
        | CliError::MissingValue(_)
        | CliError::InvalidSteps(_)
        | CliError::InvalidAddress(_) => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    }
}

fn main() -> ExitCode {
    let opts = match parse_args(env::args().skip(1)) {
        Ok(ParsedArgs::Help) => {
            eprintln!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Ok(ParsedArgs::Run(opts)) => opts,
        Err(e) => {
            eprintln!("{e}");
            return arg_exit_code(&e);
        }
    };

    let BuiltMachine {
        mut machine,
        kind,
        option_rom,
    } = match build_machine(&opts) {
        Ok(built) => built,
        Err(e) => {
            eprintln!("{e}");
            return arg_exit_code(&e);
        }
    };
    let option_rom_line = option_rom.map(|info| info.to_string());

    if opts.post_probe {
        println!(
            "{}",
            run_post_probe_options(
                &mut machine,
                opts.max_steps,
                opts.post_trace,
                opts.post_spin
            )
        );
        print_diagnostics(&machine, option_rom_line, &opts);
        return ExitCode::SUCCESS;
    }

    match run_machine(&mut machine, kind, opts.max_steps) {
        Ok((steps, com1, dbg)) => {
            println!("steps={steps} halted={}", machine.cpu.halted);
            println!("COM1:{com1}");
            println!("DEBUG:{dbg}");
            print_diagnostics(&machine, option_rom_line, &opts);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            // Still show what reached the screen; that is the point of the flag.
            print_diagnostics(&machine, option_rom_line, &opts);
            ExitCode::FAILURE
        }
    }
}
