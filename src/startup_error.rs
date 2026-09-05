//! Preserves visible application failures for GUI, terminal, and redirected launches.

use std::ffi::OsStr;
use std::io::Write;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_UNKNOWN};
use windows::Win32::System::Console::{
    AttachConsole, GetConsoleMode, GetConsoleProcessList, GetStdHandle, SetStdHandle,
    ATTACH_PARENT_PROCESS, CONSOLE_MODE, STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND};

const CONSOLE_PROCESS_SAMPLE_SIZE: usize = 2;
const MAX_DIALOG_CHARACTERS: usize = 2_000;
const CONSOLE_DETAILS_SUFFIX: &str = "\n\nFurther details were written to the console.";
const SHORTENED_MESSAGE_SUFFIX: &str = "\n\nThe error message was shortened.";
// The top-level reporter also receives errors after browsing and during shutdown.
const APPLICATION_ERROR_DIALOG_TITLE: PCWSTR = w!("SafeBrowse error");

/// Describes whether standard error can preserve diagnostics after the process exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StandardErrorContext {
    Unavailable,
    Redirected,
    Console,
}

/// Chooses an error destination without allocating a console for release GUI launches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupErrorPresentation {
    StandardErrorOnly,
    StandardErrorAndDialog,
    DialogOnly,
}

impl StartupErrorPresentation {
    /// Captures launch context before startup can create a worker or change desktops.
    pub fn detect() -> Self {
        let (internal_launch, help_launch) = std::env::args_os().skip(1).fold(
            (false, false),
            |(internal_launch, help_launch), argument| {
                (
                    internal_launch || is_internal_argument(&argument),
                    help_launch || matches!(argument.to_str(), Some("--help" | "-h")),
                )
            },
        );
        if !internal_launch {
            attach_existing_parent_console();
        }

        let noninteractive_launch = internal_launch || help_launch;
        let mut attached_processes = [0u32; CONSOLE_PROCESS_SAMPLE_SIZE];
        let attached_count = unsafe { GetConsoleProcessList(&mut attached_processes) };
        let current_process_id = unsafe { GetCurrentProcessId() };
        Self::for_launch_context(
            noninteractive_launch,
            standard_error_context(),
            attached_count,
            attached_processes[0],
            current_process_id,
        )
    }

    /// Reports after resources unwind; a closed output pipe must not cause another failure.
    pub fn report(self, error: &str) {
        let standard_error_written =
            writeln!(std::io::stderr().lock(), "[SafeBrowse] {error}").is_ok();
        if self == Self::StandardErrorOnly {
            return;
        }

        let console_details_available =
            self == Self::StandardErrorAndDialog && standard_error_written;
        let message = dialog_message(error, console_details_available);
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(message.as_ptr()),
                APPLICATION_ERROR_DIALOG_TITLE,
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }

    /// GUI and standalone debug launches need dialogs; shells and explicit redirects retain output.
    fn for_launch_context(
        noninteractive_launch: bool,
        standard_error: StandardErrorContext,
        attached_count: u32,
        first_process_id: u32,
        current_process_id: u32,
    ) -> Self {
        if noninteractive_launch || standard_error == StandardErrorContext::Redirected {
            return Self::StandardErrorOnly;
        }
        if standard_error == StandardErrorContext::Unavailable {
            return Self::DialogOnly;
        }
        if attached_count == 0 || (attached_count == 1 && first_process_id == current_process_id) {
            return Self::StandardErrorAndDialog;
        }
        Self::StandardErrorOnly
    }
}

/// Reuses a deliberate terminal launch while preserving every inherited redirected stream.
fn attach_existing_parent_console() {
    let mut processes = [0u32; CONSOLE_PROCESS_SAMPLE_SIZE];
    if unsafe { GetConsoleProcessList(&mut processes) } != 0 {
        return;
    }

    let inherited_streams = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
        .map(|stream| (stream, usable_standard_handle(stream)));
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_err() {
        return;
    }
    // AttachConsole may replace standard handles when STARTF_USESTDHANDLES was not supplied.
    for (stream, inherited_handle) in inherited_streams {
        if let Some(handle) = inherited_handle {
            let _ = unsafe { SetStdHandle(stream, handle) };
        }
    }
}

/// Distinguishes an absent GUI stream from a deliberate file, pipe, or NUL redirect.
fn standard_error_context() -> StandardErrorContext {
    let Some(handle) = usable_standard_handle(STD_ERROR_HANDLE) else {
        return StandardErrorContext::Unavailable;
    };
    let mut mode = CONSOLE_MODE::default();
    if unsafe { GetConsoleMode(handle, &mut mode) }.is_ok() {
        StandardErrorContext::Console
    } else {
        StandardErrorContext::Redirected
    }
}

/// Borrows only valid stream handles; ownership remains with the process or its caller.
fn usable_standard_handle(stream: STD_HANDLE) -> Option<HANDLE> {
    let handle = unsafe { GetStdHandle(stream) }.ok()?;
    if handle.is_invalid() || unsafe { GetFileType(handle) } == FILE_TYPE_UNKNOWN {
        return None;
    }
    Some(handle)
}

/// A malformed private worker invocation must not block its supervisor with a modal window.
fn is_internal_argument(argument: &OsStr) -> bool {
    let Some(argument) = argument.to_str() else {
        return false;
    };
    argument == "--worker" || argument.starts_with("--worker-auth-")
}

/// Bounds native dialog size and prevents embedded NULs from hiding the rest of an error.
/// Time and space: O(min(n, limit)), where n is the number of error characters.
fn dialog_message(error: &str, console_details_available: bool) -> Vec<u16> {
    let mut characters = error.chars();
    let mut message = characters
        .by_ref()
        .take(MAX_DIALOG_CHARACTERS)
        .map(|character| {
            if character == '\0' {
                '\u{FFFD}'
            } else {
                character
            }
        })
        .collect::<String>();
    if characters.next().is_some() {
        message.push_str(if console_details_available {
            CONSOLE_DETAILS_SUFFIX
        } else {
            SHORTENED_MESSAGE_SUFFIX
        });
    }
    message.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_and_standalone_debug_launches_keep_errors_visible() {
        use StandardErrorContext::{Console, Unavailable};
        use StartupErrorPresentation::{DialogOnly, StandardErrorAndDialog};
        for (context, count, first_id, expected) in [
            (Unavailable, 0, 0, DialogOnly),
            (Unavailable, 2, 41, DialogOnly),
            (Console, 1, 42, StandardErrorAndDialog),
            (Console, 0, 0, StandardErrorAndDialog),
        ] {
            assert_eq!(
                StartupErrorPresentation::for_launch_context(false, context, count, first_id, 42),
                expected
            );
        }
    }

    #[test]
    fn redirected_and_shared_console_launches_do_not_interrupt_callers() {
        use StandardErrorContext::{Console, Redirected};
        for (context, count, first_id) in [
            (Redirected, 0, 0),
            (Redirected, 1, 42),
            (Redirected, 2, 41),
            (Console, 1, 41),
            (Console, 2, 42),
            (Console, 3, 41),
        ] {
            assert_eq!(
                StartupErrorPresentation::for_launch_context(false, context, count, first_id, 42),
                StartupErrorPresentation::StandardErrorOnly
            );
        }
    }

    #[test]
    fn worker_and_help_invocations_never_request_an_error_dialog() {
        for context in [
            StandardErrorContext::Unavailable,
            StandardErrorContext::Redirected,
            StandardErrorContext::Console,
        ] {
            assert_eq!(
                StartupErrorPresentation::for_launch_context(true, context, 1, 42, 42),
                StartupErrorPresentation::StandardErrorOnly
            );
        }
        for argument in [
            "--worker",
            "--worker-auth-read",
            "--worker-auth-write",
            "--worker-auth-parent",
            "--worker-auth-invalid",
        ] {
            assert!(is_internal_argument(OsStr::new(argument)));
        }
        for argument in [
            "--help",
            "--windowed",
            "--allow-screen-recording",
            "https://example.com/--worker",
        ] {
            assert!(!is_internal_argument(OsStr::new(argument)));
        }
    }

    #[test]
    fn error_text_is_bounded_without_splitting_unicode_or_hiding_a_suffix() {
        let message = dialog_message("Failure\0more details \u{1F512}", false);
        assert_eq!(
            message.iter().filter(|character| **character == 0).count(),
            1
        );
        assert_eq!(
            String::from_utf16(&message[..message.len() - 1]).unwrap(),
            "Failure\u{FFFD}more details \u{1F512}"
        );
        let long_error = "\u{1F512}".repeat(MAX_DIALOG_CHARACTERS + 1);
        for console_available in [false, true] {
            let message = dialog_message(&long_error, console_available);
            let message = String::from_utf16(&message[..message.len() - 1]).unwrap();
            assert_eq!(
                message
                    .chars()
                    .filter(|character| *character == '\u{1F512}')
                    .count(),
                MAX_DIALOG_CHARACTERS
            );
            let expected_suffix = if console_available {
                CONSOLE_DETAILS_SUFFIX
            } else {
                SHORTENED_MESSAGE_SUFFIX
            };
            assert!(message.ends_with(expected_suffix));
        }
    }
}
