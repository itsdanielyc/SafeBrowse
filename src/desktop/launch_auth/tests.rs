use super::*;

fn arguments(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn internal_handle_arguments_are_complete_unique_and_separate_from_user_options() {
    let mut incoming = arguments(&[
        "--worker",
        AUTH_READ_ARGUMENT,
        "12",
        "--url",
        "https://example.com/",
        AUTH_WRITE_ARGUMENT,
        "16",
        AUTH_PARENT_ARGUMENT,
        "20",
    ]);
    let authentication = extract_worker_auth_arguments(&mut incoming)
        .unwrap()
        .unwrap();
    assert_eq!(authentication.read_handle, 12);
    assert_eq!(authentication.write_handle, 16);
    assert_eq!(authentication.parent_handle, 20);
    assert_eq!(
        incoming,
        arguments(&["--worker", "--url", "https://example.com/"])
    );

    for malformed in [
        vec![AUTH_READ_ARGUMENT, "12"],
        vec![AUTH_READ_ARGUMENT],
        vec![
            AUTH_READ_ARGUMENT,
            "0",
            AUTH_WRITE_ARGUMENT,
            "16",
            AUTH_PARENT_ARGUMENT,
            "20",
        ],
        vec![
            AUTH_READ_ARGUMENT,
            "12",
            AUTH_WRITE_ARGUMENT,
            "12",
            AUTH_PARENT_ARGUMENT,
            "20",
        ],
        vec![
            AUTH_READ_ARGUMENT,
            "12",
            AUTH_READ_ARGUMENT,
            "24",
            AUTH_WRITE_ARGUMENT,
            "16",
            AUTH_PARENT_ARGUMENT,
            "20",
        ],
    ] {
        assert!(extract_worker_auth_arguments(&mut arguments(&malformed)).is_err());
    }
}

#[test]
fn session_names_are_fresh_and_reject_legacy_or_unbounded_desktop_names() {
    let first = new_session_desktop_name();
    let second = new_session_desktop_name();
    assert_ne!(first, second);
    assert!(validate_session_desktop_name(&first).is_ok());
    assert!(validate_session_desktop_name(&second).is_ok());
    for invalid in [
        SAFE_DESKTOP_NAME,
        "Default",
        "WinSta0\\Default",
        "",
        "SafeBrowseDesktop_00000000000000000000000000000000",
    ] {
        assert!(validate_session_desktop_name(invalid).is_err());
    }
}

#[test]
fn protocol_binds_both_processes_nonce_and_bounded_desktop_identity() {
    let desktop = new_session_desktop_name();
    let nonce = [0x17; NONCE_LENGTH];
    let message = hello_message(101, 202, &nonce, &desktop).unwrap();
    let header: [u8; HELLO_HEADER_LENGTH] = message[..HELLO_HEADER_LENGTH].try_into().unwrap();
    let parsed = Hello::decode_header(&header).unwrap();
    assert_eq!(parsed.supervisor_id, 101);
    assert_eq!(parsed.worker_id, 202);
    assert_eq!(parsed.nonce, nonce);
    assert_eq!(parsed.desktop_name_length, desktop.len());
    assert_eq!(&message[HELLO_HEADER_LENGTH..], desktop.as_bytes());
    assert_ne!(acknowledgement(202, &nonce), acknowledgement(203, &nonce));
    assert_ne!(
        acknowledgement(202, &nonce),
        acknowledgement(202, &[0x18; NONCE_LENGTH])
    );
    assert_ne!(
        commit_message(&nonce),
        commit_message(&[0x18; NONCE_LENGTH])
    );

    let mut malformed = header;
    malformed[8] = AUTH_VERSION as u8 + 1;
    assert!(Hello::decode_header(&malformed).is_err());
    malformed = header;
    malformed[34..36].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(Hello::decode_header(&malformed).is_err());
}

#[test]
fn anonymous_pipe_endpoints_report_the_creating_process_and_honor_inheritance() {
    let (read_pipe, write_pipe) = create_inheritable_pipe().unwrap();
    for pipe in [&read_pipe, &write_pipe] {
        let mut owner = 0;
        unsafe { GetNamedPipeServerProcessId(pipe.raw(), &mut owner) }.unwrap();
        assert_eq!(owner, unsafe { GetCurrentProcessId() });
        let mut flags = 0;
        unsafe { GetHandleInformation(pipe.raw(), &mut flags) }.unwrap();
        assert_ne!(flags & HANDLE_FLAG_INHERIT.0, 0);
        pipe.prevent_inheritance().unwrap();
        unsafe { GetHandleInformation(pipe.raw(), &mut flags) }.unwrap();
        assert_eq!(flags & HANDLE_FLAG_INHERIT.0, 0);
    }
}

#[test]
fn a_stalled_or_closed_authorization_channel_fails_without_blocking_forever() {
    let (read_pipe, write_pipe) = create_inheritable_pipe().unwrap();
    let mut byte = [0u8];
    let timeout = read_exact_before(
        read_pipe.raw(),
        &mut byte,
        unsafe { GetCurrentProcess() },
        Instant::now() + Duration::from_millis(30),
    )
    .unwrap_err();
    assert!(timeout.contains("timed out"));
    drop(write_pipe);
    assert!(read_exact_before(
        read_pipe.raw(),
        &mut byte,
        unsafe { GetCurrentProcess() },
        Instant::now() + HANDSHAKE_TIMEOUT,
    )
    .is_err());
}

#[test]
fn lifetime_job_is_unnamed_noninheritable_and_kill_on_last_close() {
    use windows::Win32::System::JobObjects::QueryInformationJobObject;
    let job = create_lifetime_job().unwrap();
    let mut flags = 0;
    unsafe { GetHandleInformation(job.raw(), &mut flags) }.unwrap();
    assert_eq!(flags & HANDLE_FLAG_INHERIT.0, 0);
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    unsafe {
        QueryInformationJobObject(
            Some(job.raw()),
            JobObjectExtendedLimitInformation,
            (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            None,
        )
    }
    .unwrap();
    assert_eq!(
        limits.BasicLimitInformation.LimitFlags,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    );
}

#[test]
fn executable_identity_uses_file_identity_instead_of_window_or_process_names() {
    let executable = std::env::current_exe().unwrap();
    assert_eq!(
        file_identity(&executable).unwrap(),
        file_identity(&executable).unwrap()
    );
    let process_image = process_image_path(unsafe { GetCurrentProcess() }).unwrap();
    assert_eq!(
        file_identity(&executable).unwrap(),
        file_identity(&process_image).unwrap()
    );
}

#[test]
fn worker_command_line_contains_only_handle_addresses_and_preserves_argument_boundaries() {
    let auth = WorkerAuthArguments {
        read_handle: 12,
        write_handle: 16,
        parent_handle: 20,
    };
    let command = build_worker_command_line(
        Path::new("C:\\Safe Browse\\browser.exe"),
        &[
            "--worker",
            "--url",
            "https://example.com/?q=a b&next=\"test\"",
        ],
        auth,
    )
    .unwrap();
    let command = String::from_utf16(&command[..command.len() - 1]).unwrap();
    assert!(command.starts_with("\"C:\\Safe Browse\\browser.exe\" \"--worker\""));
    assert!(command.contains("\"--worker-auth-read\" \"12\""));
    assert!(command.contains("next=\\\"test\\\""));
    assert!(!command.contains(SAFE_DESKTOP_NAME));

    for (argument, expected) in [
        ("", "\"\""),
        (
            "a\" --allow-screen-recording",
            "\"a\\\" --allow-screen-recording\"",
        ),
        ("C:\\some folder\\", "\"C:\\some folder\\\\\""),
    ] {
        let mut quoted = Vec::new();
        append_quoted_argument(&mut quoted, OsStr::new(argument)).unwrap();
        assert_eq!(String::from_utf16(&quoted).unwrap(), expected);
    }
    assert!(append_quoted_argument(&mut Vec::new(), OsStr::new("bad\0argument")).is_err());
}
