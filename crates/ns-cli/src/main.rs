//! The `ns` binary.

use std::process::ExitCode;

use clap::Parser;
use ns_cli::cli::Cli;
use ns_cli::commands;

fn main() -> ExitCode {
    match commands::run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
