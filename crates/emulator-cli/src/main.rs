//! Native CLI: run a ROM/BIOS (default: built-in HELLO ROM) until HLT.

use emulator_cli::{
    build_machine, guest_boot_media, parse_args, run_freedos_measure, run_guest_measure,
    run_linux_serial_measure, run_machine, run_post_probe_options, usage, vga_frame_report,
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
        | CliError::GuestMeasureNeedsImage
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

    if opts.guest_freedos_measure {
        match run_freedos_measure(&mut machine, opts.max_steps) {
            Ok(measure) => {
                println!("{measure}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("{e}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return arg_exit_code(&e);
            }
        }
    }

    if opts.guest_linux_serial_measure {
        match run_linux_serial_measure(&mut machine, opts.max_steps) {
            Ok(measure) => {
                println!("{measure}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("{e}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return arg_exit_code(&e);
            }
        }
    }

    if opts.guest_freedos_measure {
        match run_freedos_measure(&mut machine, opts.max_steps) {
            Ok(measure) => {
                println!("{measure}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("{e}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return arg_exit_code(&e);
            }
        }
    }

    if opts.guest_linux_serial_measure {
        match run_linux_serial_measure(&mut machine, opts.max_steps) {
            Ok(measure) => {
                println!("{measure}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("{e}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return arg_exit_code(&e);
            }
        }
    }

    if opts.guest_measure {
        match run_guest_measure(&mut machine, guest_boot_media(&opts), opts.max_steps) {
            Ok(measure) => {
                println!("{measure}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("{e}");
                print_diagnostics(&machine, option_rom_line, &opts);
                return arg_exit_code(&e);
            }
        }
    }

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
            print_diagnostics(&machine, option_rom_line, &opts);
            arg_exit_code(&e)
        }
    }
}
