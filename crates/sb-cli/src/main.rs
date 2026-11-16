//! The `sb` binary.

use std::process::ExitCode;

use clap::Parser;
use sb_cli::cli::Cli;
use sb_cli::commands;

fn main() -> ExitCode {
    match commands::run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
