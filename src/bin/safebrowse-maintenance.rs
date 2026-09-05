//! Installer subprocess; console output is captured by the hidden installer invocation.

use std::io::{self, Write};
use std::process::ExitCode;

use safebrowse::maintenance::{MaintenanceCommand, MaintenanceError};

fn main() -> ExitCode {
    let result = std::env::args_os()
        .skip(1)
        .map(|argument| {
            argument
                .into_string()
                .map_err(|_| "Maintenance arguments must contain valid Unicode".to_string())
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|arguments| MaintenanceCommand::parse(&arguments))
        .map_err(MaintenanceError::from)
        .and_then(MaintenanceCommand::execute);
    match result {
        Ok(message) => {
            if writeln!(io::stdout().lock(), "{message}").is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            // A closed installer capture pipe must never unwind after a filesystem operation.
            let _ = writeln!(io::stderr().lock(), "{}", error.message);
            ExitCode::from(error.exit_code)
        }
    }
}
