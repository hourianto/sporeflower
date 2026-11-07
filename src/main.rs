mod artifact_plan;
mod cli;
mod compile_inputs;
mod compiler;
mod decompile;
mod mapping_reports;
mod output;
mod project;

use clap::{CommandFactory, Parser};
use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args_os().len() == 1 {
        return match cli::Cli::command().print_help() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let cli = cli::Cli::parse();
    let result = match cli.command {
        None => decompile::run(cli.direct, None),
        Some(cli::Command::Decompile(args)) => decompile::run(args, None),
        Some(cli::Command::Init(args)) => project::init(args),
        Some(cli::Command::Remap(args)) => project::remap(args),
        Some(cli::Command::ExtractResources(args)) => {
            project::extract(&args.project).map(|()| true)
        }
        Some(cli::Command::CompileStubs(args)) => compiler::run(args),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(error) => {
            if let Some(diagnostics) =
                error.downcast_ref::<sporeflower::mappings::MappingDiagnostics>()
            {
                eprint!("{diagnostics}");
            } else {
                eprintln!("error: {error:#}");
            }
            ExitCode::FAILURE
        }
    }
}
