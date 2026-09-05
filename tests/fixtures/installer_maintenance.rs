//! Offline installer-test executable; never links or calls SafeBrowse maintenance.
//!
//! Its compile-time fixture root and marker confine all writes to disposable test
//! files. Different copied filenames model the app, maintenance, and prerequisite.

#![windows_subsystem = "windows"]

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const FIXTURE_ROOT: &str = env!("SAFEBROWSE_INSTALLER_FIXTURE_ROOT");
const FIXTURE_ID: &str = env!("SAFEBROWSE_INSTALLER_FIXTURE_ID");
const FIXTURE_FAILURE: i32 = 92;

/// Reads a fixture response, failing closed when the test omitted its scenario.
fn configured_exit_code(root: &Path, name: &str) -> io::Result<i32> {
    fs::read_to_string(root.join(name))?
        .trim()
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Validates the compiled root before recording or simulating any operation.
fn run_fixture() -> io::Result<i32> {
    let root = PathBuf::from(FIXTURE_ROOT).canonicalize()?;
    if fs::read_to_string(root.join("fixture-id.txt"))?.trim() != FIXTURE_ID {
        return Err(io::Error::other("Installer fixture marker mismatch"));
    }
    let executable = env::current_exe()?;
    let executable_name = executable.file_name().unwrap_or_default().to_string_lossy();
    let arguments: Vec<String> = env::args().skip(1).collect();
    let mut log = OpenOptions::new()
        .append(true)
        .create(true)
        .open(root.join("calls.log"))?;
    writeln!(log, "{} {}", executable_name, arguments.join(" "))?;
    match (executable_name.as_ref(), arguments.as_slice()) {
        ("MicrosoftEdgeWebview2Setup.exe", _) => configured_exit_code(&root, "bootstrap-exit.txt"),
        ("safebrowse-maintenance.exe", [command]) if command == "check-runtime" => {
            let code = configured_exit_code(&root, "runtime-exit.txt")?;
            if code != 0 {
                eprintln!("Installer fixture: configured runtime check failure {code}.");
            }
            Ok(code)
        }
        ("safebrowse-maintenance.exe", [command, rest @ ..]) if command == "cleanup" => {
            if !rest.is_empty() && rest != ["--remove-user-data"] {
                return Err(io::Error::other("Unexpected fixture cleanup arguments"));
            }
            let code = configured_exit_code(&root, "cleanup-exit.txt")?;
            if code != 0 {
                eprintln!("Installer fixture: configured cleanup failure {code}.");
                return Ok(code);
            }
            if !rest.is_empty() {
                fs::remove_file(root.join("user-data.txt"))?;
            }
            Ok(0)
        }
        ("safebrowse.exe", _) => Ok(0),
        _ => Err(io::Error::other("Unexpected installer fixture command")),
    }
}

fn main() {
    let code = match run_fixture() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Installer fixture failed: {error}");
            FIXTURE_FAILURE
        }
    };
    std::process::exit(code);
}
