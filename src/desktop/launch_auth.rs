//! Capability-based authorization for one supervisor and one isolated browser worker.
//!
//! A worker receives only inherited, unnamed pipe and process handles. The desktop name and
//! random challenge travel inside those pipes, never in command-line arguments or logs. The
//! supervisor binds the exchange to the process it created and keeps the worker in an unnamed
//! kill-on-close job. This prevents unauthenticated launches and stale-session reuse; it is not
//! a boundary against an attacker that can inject code or duplicate handles from either process.

use std::ffi::{c_void, OsStr};
use std::fs::File;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, DuplicateHandle, GetHandleInformation, SetHandleInformation,
    DUPLICATE_HANDLE_OPTIONS, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, ReadFile, WriteFile, BY_HANDLE_FILE_INFORMATION,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::JobObjects::{
    CreateJobObjectW, IsProcessInJob, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Pipes::{CreatePipe, GetNamedPipeServerProcessId, PeekNamedPipe};
use windows::Win32::System::StationsAndDesktops::{
    GetThreadDesktop, GetUserObjectInformationW, UOI_NAME,
};
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetCurrentProcessId,
    GetCurrentThreadId, GetExitCodeProcess, GetProcessId, InitializeProcThreadAttributeList,
    QueryFullProcessImageNameW, ResumeThread, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTUPINFOEXW,
    STARTUPINFOW,
};

use crate::config::{SAFE_DESKTOP_NAME, WORKER_TERMINATION_TIMEOUT};

const AUTH_READ_ARGUMENT: &str = "--worker-auth-read";
const AUTH_WRITE_ARGUMENT: &str = "--worker-auth-write";
const AUTH_PARENT_ARGUMENT: &str = "--worker-auth-parent";
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_POLL_INTERVAL: Duration = Duration::from_millis(5);
const AUTH_PIPE_CAPACITY: u32 = 4096;
const AUTH_VERSION: u16 = 1;
const HELLO_MAGIC: &[u8; 8] = b"SBHELLO1";
const ACK_MAGIC: &[u8; 8] = b"SBACK__1";
const COMMIT_MAGIC: &[u8; 8] = b"SBCOMMIT";
const NONCE_LENGTH: usize = 16;
const HELLO_HEADER_LENGTH: usize = 8 + 2 + 4 + 4 + NONCE_LENGTH + 2;
const ACK_LENGTH: usize = 8 + 4 + NONCE_LENGTH;
const COMMIT_LENGTH: usize = 8 + NONCE_LENGTH;
const MAX_DESKTOP_NAME_BYTES: usize = 96;
const MAX_COMMAND_LINE_CODE_UNITS: usize = 32_767;
const MAX_PROCESS_IMAGE_CODE_UNITS: usize = 32_768;
const STARTUP_FAILURE_EXIT_CODE: u32 = 1;

/// Numeric inherited-handle addresses, not a reusable credential or a desktop identifier.
#[derive(Debug, PartialEq, Eq)]
pub struct WorkerAuthArguments {
    read_handle: usize,
    write_handle: usize,
    parent_handle: usize,
}

/// Removes the internal transport arguments before normal launch-option parsing.
///
/// Every handle must be present exactly once. A caller must additionally reject these
/// arguments unless `--worker` was supplied and require them whenever `--worker` is supplied.
/// Time: O(n); space: O(n), where n is the number of command-line arguments.
pub fn extract_worker_auth_arguments(
    arguments: &mut Vec<String>,
) -> Result<Option<WorkerAuthArguments>, String> {
    let mut read_handle = None;
    let mut write_handle = None;
    let mut parent_handle = None;
    let mut public_arguments = Vec::with_capacity(arguments.len());
    let mut incoming = std::mem::take(arguments).into_iter();
    while let Some(argument) = incoming.next() {
        let destination = match argument.as_str() {
            AUTH_READ_ARGUMENT => &mut read_handle,
            AUTH_WRITE_ARGUMENT => &mut write_handle,
            AUTH_PARENT_ARGUMENT => &mut parent_handle,
            _ => {
                public_arguments.push(argument);
                continue;
            }
        };
        if destination.is_some() {
            return Err("Worker authorization arguments cannot be repeated".into());
        }
        let value = incoming
            .next()
            .ok_or("Worker authorization arguments are incomplete")?;
        *destination = Some(parse_handle_address(&value)?);
    }
    *arguments = public_arguments;
    match (read_handle, write_handle, parent_handle) {
        (None, None, None) => Ok(None),
        (Some(read_handle), Some(write_handle), Some(parent_handle)) => {
            if read_handle == write_handle
                || read_handle == parent_handle
                || write_handle == parent_handle
            {
                return Err("Worker authorization handles must be distinct".into());
            }
            Ok(Some(WorkerAuthArguments {
                read_handle,
                write_handle,
                parent_handle,
            }))
        }
        _ => Err("Worker authorization arguments are incomplete".into()),
    }
}

/// A session name accepted only after a live, matching supervisor completed the exchange.
pub struct AuthenticatedWorkerSession {
    desktop_name: String,
    _supervisor: KernelHandle,
}

impl AuthenticatedWorkerSession {
    /// Returns the fresh desktop identity received through the inherited capability.
    pub fn desktop_name(&self) -> &str {
        &self.desktop_name
    }
}

/// Owns the exact worker and its non-inherited, unnamed lifetime job.
pub struct SupervisedWorkerProcess {
    process: KernelHandle,
    job: Option<KernelHandle>,
    thread_id: u32,
}

impl SupervisedWorkerProcess {
    /// Borrows the process handle for liveness monitoring and orderly shutdown.
    pub fn handle(&self) -> HANDLE {
        self.process.raw()
    }

    /// Returns the thread receiving the browser's orderly-shutdown message.
    pub fn thread_id(&self) -> u32 {
        self.thread_id
    }

    /// Verifies membership in this supervisor's exact job, including runtime descendants.
    pub fn contains_process(&self, process: HANDLE) -> Result<bool, String> {
        let job = self
            .job
            .as_ref()
            .ok_or("Worker lifetime container is closed")?;
        let mut contained = BOOL::default();
        unsafe { IsProcessInJob(process, Some(job.raw()), &mut contained) }
            .map_err(|error| format!("Could not verify worker lifetime containment: {error}"))?;
        Ok(contained.as_bool())
    }

    /// Reads the status of this exact process, without resolving a reusable PID.
    pub fn exit_code(&self) -> Result<u32, String> {
        let mut exit_code = 0;
        unsafe { GetExitCodeProcess(self.handle(), &mut exit_code) }
            .map_err(|error| format!("Could not read browser exit status: {error}"))?;
        Ok(exit_code)
    }
}

impl Drop for SupervisedWorkerProcess {
    fn drop(&mut self) {
        // The only job handle stays in the supervisor, so even OS termination closes it.
        drop(self.job.take());
        if unsafe { WaitForSingleObject(self.handle(), 0) } == WAIT_TIMEOUT {
            unsafe {
                let _ = TerminateProcess(self.handle(), STARTUP_FAILURE_EXIT_CODE);
                let _ = WaitForSingleObject(
                    self.handle(),
                    WORKER_TERMINATION_TIMEOUT.as_millis() as u32,
                );
            }
        }
    }
}

/// Rejects unauthenticated, replayed, wrong-parent, wrong-image, or orphaned worker startup.
///
/// Call before creating/opening a desktop, touching the clipboard, creating a profile, or
/// initializing any GUI. The only desktop access here is a read of the process's assignment.
pub fn authenticate_worker_launch(
    arguments: WorkerAuthArguments,
) -> Result<AuthenticatedWorkerSession, String> {
    let read_pipe = KernelHandle::from_address(arguments.read_handle)?;
    let write_pipe = KernelHandle::from_address(arguments.write_handle)?;
    let supervisor = KernelHandle::from_address(arguments.parent_handle)?;
    for handle in [&read_pipe, &write_pipe, &supervisor] {
        handle.prevent_inheritance()?;
    }
    let supervisor_id = validate_supervisor(read_pipe.raw(), write_pipe.raw(), supervisor.raw())?;
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut header = [0u8; HELLO_HEADER_LENGTH];
    read_exact_before(read_pipe.raw(), &mut header, supervisor.raw(), deadline)?;
    let hello = Hello::decode_header(&header)?;
    if hello.supervisor_id != supervisor_id || hello.worker_id != unsafe { GetCurrentProcessId() } {
        return Err("Worker authorization was issued for a different process".into());
    }
    let mut name_bytes = vec![0u8; hello.desktop_name_length];
    read_exact_before(read_pipe.raw(), &mut name_bytes, supervisor.raw(), deadline)?;
    let desktop_name = String::from_utf8(name_bytes)
        .map_err(|_| "Worker authorization contains an invalid desktop identity".to_string())?;
    validate_session_desktop_name(&desktop_name)?;
    if current_desktop_name()? != desktop_name {
        return Err("Worker was not created on its authorized desktop".into());
    }
    let acknowledgement = acknowledgement(hello.worker_id, &hello.nonce);
    write_all(write_pipe.raw(), &acknowledgement)?;
    let mut commit = [0u8; COMMIT_LENGTH];
    read_exact_before(read_pipe.raw(), &mut commit, supervisor.raw(), deadline)?;
    if commit != commit_message(&hello.nonce) {
        return Err("Supervisor did not confirm the worker authorization".into());
    }
    require_running_process(supervisor.raw())?;
    Ok(AuthenticatedWorkerSession {
        desktop_name,
        _supervisor: supervisor,
    })
}

/// Generates a new name instead of treating a well-known desktop object as an identity.
pub(crate) fn new_session_desktop_name() -> String {
    format!("{SAFE_DESKTOP_NAME}_{}", uuid::Uuid::new_v4().simple())
}

/// Accepts only the application-generated, version-four UUID desktop namespace.
pub(crate) fn validate_session_desktop_name(name: &str) -> Result<(), String> {
    let prefix = format!("{SAFE_DESKTOP_NAME}_");
    let Some(identifier) = name.strip_prefix(&prefix) else {
        return Err("Invalid isolated session desktop identity".into());
    };
    if identifier.len() != 32
        || !identifier.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !uuid::Uuid::parse_str(identifier).is_ok_and(|value| value.get_version_num() == 4)
    {
        return Err("Invalid isolated session desktop identity".into());
    }
    Ok(())
}

/// Creates, contains, and authenticates a worker without activating its desktop.
/// Time: O(n); space: O(n), where n is the total command-line length.
pub(crate) fn spawn_authenticated_worker(
    desktop_name: &str,
    worker_arguments: &[&str],
) -> Result<SupervisedWorkerProcess, String> {
    validate_session_desktop_name(desktop_name)?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the browser executable: {error}"))?;
    spawn_authenticated_worker_executable(&executable, desktop_name, worker_arguments)
}

fn spawn_authenticated_worker_executable(
    executable: &Path,
    desktop_name: &str,
    worker_arguments: &[&str],
) -> Result<SupervisedWorkerProcess, String> {
    let (child_read, supervisor_write) = create_inheritable_pipe()?;
    let (supervisor_read, child_write) = create_inheritable_pipe()?;
    supervisor_write.prevent_inheritance()?;
    supervisor_read.prevent_inheritance()?;
    let supervisor_identity = duplicate_supervisor_identity()?;
    let inherited_handles = [
        child_read.raw(),
        child_write.raw(),
        supervisor_identity.raw(),
    ];
    let job = create_lifetime_job()?;
    let jobs = [job.raw()];
    let attribute_list = HandleAttributeList::new(&inherited_handles, &jobs)?;
    let mut command_line = build_worker_command_line(
        executable,
        worker_arguments,
        WorkerAuthArguments {
            read_handle: child_read.address(),
            write_handle: child_write.address(),
            parent_handle: supervisor_identity.address(),
        },
    )?;
    let executable_wide = to_wide(executable.as_os_str());
    let mut desktop_wide = to_wide(OsStr::new(&format!("WinSta0\\{desktop_name}")));
    let startup = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: size_of::<STARTUPINFOEXW>() as u32,
            lpDesktop: PWSTR(desktop_wide.as_mut_ptr()),
            ..Default::default()
        },
        lpAttributeList: attribute_list.raw,
    };
    let mut process_information = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(executable_wide.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            None,
            PCWSTR::null(),
            &startup.StartupInfo,
            &mut process_information,
        )
    }
    .map_err(|error| format!("Could not create the authorized browser worker: {error}"))?;
    let process = KernelHandle::new(process_information.hProcess)?;
    let initial_thread = KernelHandle::new(process_information.hThread)?;
    let worker = SupervisedWorkerProcess {
        process,
        job: Some(job),
        thread_id: process_information.dwThreadId,
    };
    drop(child_read);
    drop(child_write);
    drop(supervisor_identity);

    let nonce = *uuid::Uuid::new_v4().as_bytes();
    let challenge = hello_message(
        unsafe { GetCurrentProcessId() },
        process_information.dwProcessId,
        &nonce,
        desktop_name,
    )?;
    write_all(supervisor_write.raw(), &challenge)?;
    if unsafe { ResumeThread(initial_thread.raw()) } != 1 {
        return Err("The contained browser worker had an unexpected suspension state".into());
    }
    let mut response = [0u8; ACK_LENGTH];
    read_exact_before(
        supervisor_read.raw(),
        &mut response,
        worker.handle(),
        Instant::now() + HANDSHAKE_TIMEOUT,
    )?;
    if response != acknowledgement(process_information.dwProcessId, &nonce) {
        return Err("Browser worker failed its single-use launch challenge".into());
    }
    require_running_process(worker.handle())?;
    write_all(supervisor_write.raw(), &commit_message(&nonce))?;
    Ok(worker)
}

/// Owns a valid kernel handle; addresses are used to make move-only ownership thread-safe.
struct KernelHandle(usize);

impl KernelHandle {
    fn new(handle: HANDLE) -> Result<Self, String> {
        if handle.is_invalid() {
            return Err("Worker authorization received an invalid kernel handle".into());
        }
        Ok(Self(handle.0 as usize))
    }

    fn from_address(address: usize) -> Result<Self, String> {
        let handle = HANDLE(address as *mut c_void);
        let mut flags = 0;
        unsafe { GetHandleInformation(handle, &mut flags) }
            .map_err(|_| "Worker authorization requires inherited kernel handles".to_string())?;
        Self::new(handle)
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.0 as *mut c_void)
    }

    fn address(&self) -> usize {
        self.0
    }

    fn prevent_inheritance(&self) -> Result<(), String> {
        unsafe { SetHandleInformation(self.raw(), HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }
            .map_err(|error| format!("Could not restrict worker handle inheritance: {error}"))
    }
}

impl Drop for KernelHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.raw());
        }
    }
}

/// Keeps the attribute list's aligned backing storage alive until CreateProcess has returned.
struct HandleAttributeList {
    raw: LPPROC_THREAD_ATTRIBUTE_LIST,
    _storage: Vec<usize>,
}

impl HandleAttributeList {
    fn new(handles: &[HANDLE], jobs: &[HANDLE]) -> Result<Self, String> {
        let mut byte_count = 0;
        const ATTRIBUTE_COUNT: u32 = 2;
        let _ = unsafe {
            InitializeProcThreadAttributeList(None, ATTRIBUTE_COUNT, None, &mut byte_count)
        };
        if byte_count == 0 {
            return Err("Could not size the worker inheritance policy".into());
        }
        let mut storage = vec![0usize; byte_count.div_ceil(size_of::<usize>())];
        let raw = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        unsafe {
            InitializeProcThreadAttributeList(Some(raw), ATTRIBUTE_COUNT, None, &mut byte_count)
        }
        .map_err(|error| format!("Could not initialize worker inheritance policy: {error}"))?;
        let list = Self {
            raw,
            _storage: storage,
        };
        unsafe {
            UpdateProcThreadAttribute(
                list.raw,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                std::mem::size_of_val(handles),
                None,
                None,
            )
        }
        .map_err(|error| format!("Could not restrict the worker's inherited handles: {error}"))?;
        // Atomic job assignment also covers supervisor termination during CreateProcess itself.
        unsafe {
            UpdateProcThreadAttribute(
                list.raw,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                Some(jobs.as_ptr().cast()),
                std::mem::size_of_val(jobs),
                None,
                None,
            )
        }
        .map_err(|error| {
            format!("Could not bind the worker to its supervisor lifetime: {error}")
        })?;
        Ok(list)
    }
}

impl Drop for HandleAttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.raw) };
    }
}

fn create_inheritable_pipe() -> Result<(KernelHandle, KernelHandle), String> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: BOOL(1),
        ..Default::default()
    };
    let mut read_handle = HANDLE::default();
    let mut write_handle = HANDLE::default();
    unsafe {
        CreatePipe(
            &mut read_handle,
            &mut write_handle,
            Some(&attributes),
            AUTH_PIPE_CAPACITY,
        )
    }
    .map_err(|error| format!("Could not create private worker authorization channels: {error}"))?;
    Ok((
        KernelHandle::new(read_handle)?,
        KernelHandle::new(write_handle)?,
    ))
}

fn duplicate_supervisor_identity() -> Result<KernelHandle, String> {
    let mut handle = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            GetCurrentProcess(),
            GetCurrentProcess(),
            &mut handle,
            PROCESS_QUERY_LIMITED_INFORMATION.0 | PROCESS_SYNCHRONIZE.0,
            true,
            DUPLICATE_HANDLE_OPTIONS(0),
        )
    }
    .map_err(|error| format!("Could not create supervisor identity capability: {error}"))?;
    KernelHandle::new(handle)
}

fn create_lifetime_job() -> Result<KernelHandle, String> {
    let job = KernelHandle::new(
        unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|error| format!("Could not create the worker lifetime container: {error}"))?,
    )?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(|error| format!("Could not enforce supervised worker lifetime: {error}"))?;
    Ok(job)
}

fn validate_supervisor(
    read_pipe: HANDLE,
    write_pipe: HANDLE,
    supervisor: HANDLE,
) -> Result<u32, String> {
    let supervisor_id = unsafe { GetProcessId(supervisor) };
    if supervisor_id == 0 || supervisor_id == unsafe { GetCurrentProcessId() } {
        return Err("Worker authorization has no valid supervisor identity".into());
    }
    require_running_process(supervisor)?;
    for pipe in [read_pipe, write_pipe] {
        let mut pipe_owner = 0;
        unsafe { GetNamedPipeServerProcessId(pipe, &mut pipe_owner) }
            .map_err(|_| "Worker authorization requires private supervisor pipes".to_string())?;
        if pipe_owner != supervisor_id {
            return Err("Worker authorization channels belong to another process".into());
        }
    }
    if current_parent_process_id()? != supervisor_id {
        return Err("Worker was not created by its authorization supervisor".into());
    }
    let supervisor_image = process_image_path(supervisor)?;
    let own_image = std::env::current_exe()
        .map_err(|error| format!("Could not verify worker executable identity: {error}"))?;
    if file_identity(&supervisor_image)? != file_identity(&own_image)? {
        return Err("Worker supervisor does not use the same executable".into());
    }
    Ok(supervisor_id)
}

/// Resolves the current process's recorded parent from one read-only process snapshot.
/// Time: O(p); space: O(1), where p is the number of active processes.
fn current_parent_process_id() -> Result<u32, String> {
    let snapshot = KernelHandle::new(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|error| format!("Could not verify worker parent process: {error}"))?,
    )?;
    let mut process = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    unsafe { Process32FirstW(snapshot.raw(), &mut process) }
        .map_err(|error| format!("Could not inspect worker parent process: {error}"))?;
    let current_id = unsafe { GetCurrentProcessId() };
    loop {
        if process.th32ProcessID == current_id {
            return Ok(process.th32ParentProcessID);
        }
        if unsafe { Process32NextW(snapshot.raw(), &mut process) }.is_err() {
            return Err("Could not locate the worker's parent process".into());
        }
    }
}

fn process_image_path(process: HANDLE) -> Result<PathBuf, String> {
    let mut path = vec![0u16; MAX_PROCESS_IMAGE_CODE_UNITS];
    let mut length = path.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|error| format!("Could not verify supervisor executable identity: {error}"))?;
    path.truncate(length as usize);
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&path)))
}

#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    volume: u32,
    index_high: u32,
    index_low: u32,
}

fn file_identity(path: &Path) -> Result<FileIdentity, String> {
    let file = File::open(path)
        .map_err(|error| format!("Could not open executable for identity verification: {error}"))?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information) }
        .map_err(|error| format!("Could not read executable file identity: {error}"))?;
    Ok(FileIdentity {
        volume: information.dwVolumeSerialNumber,
        index_high: information.nFileIndexHigh,
        index_low: information.nFileIndexLow,
    })
}

fn current_desktop_name() -> Result<String, String> {
    let desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
        .map_err(|error| format!("Could not verify the worker desktop assignment: {error}"))?;
    let mut name = [0u16; MAX_DESKTOP_NAME_BYTES + 1];
    let mut needed_bytes = 0;
    unsafe {
        GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            std::mem::size_of_val(&name) as u32,
            Some(&mut needed_bytes),
        )
    }
    .map_err(|error| format!("Could not read the worker desktop identity: {error}"))?;
    let terminator = name
        .iter()
        .position(|character| *character == 0)
        .ok_or("Worker desktop identity is unterminated")?;
    String::from_utf16(&name[..terminator])
        .map_err(|_| "Worker desktop identity is invalid Unicode".to_string())
}

fn require_running_process(process: HANDLE) -> Result<(), String> {
    match unsafe { WaitForSingleObject(process, 0) } {
        WAIT_TIMEOUT => Ok(()),
        WAIT_OBJECT_0 => Err("Worker authorization peer exited before startup completed".into()),
        _ => Err("Worker authorization peer could not be verified".into()),
    }
}

/// Reads bounded protocol records without allowing a stalled peer to hang startup.
/// Time: O(n + t / poll); space: O(1), for n bytes and at most timeout t.
fn read_exact_before(
    pipe: HANDLE,
    buffer: &mut [u8],
    peer: HANDLE,
    deadline: Instant,
) -> Result<(), String> {
    let mut completed = 0;
    while completed < buffer.len() {
        require_running_process(peer)?;
        if Instant::now() >= deadline {
            return Err("Worker authorization timed out".into());
        }
        let mut available = 0;
        unsafe { PeekNamedPipe(pipe, None, 0, None, Some(&mut available), None) }
            .map_err(|_| "Worker authorization channel closed before completion".to_string())?;
        if available == 0 {
            std::thread::sleep(HANDSHAKE_POLL_INTERVAL);
            continue;
        }
        let amount = (available as usize).min(buffer.len() - completed);
        let mut read_count = 0;
        unsafe {
            ReadFile(
                pipe,
                Some(&mut buffer[completed..completed + amount]),
                Some(&mut read_count),
                None,
            )
        }
        .map_err(|_| "Worker authorization channel could not be read".to_string())?;
        if read_count == 0 {
            return Err("Worker authorization channel ended before completion".into());
        }
        completed += read_count as usize;
    }
    Ok(())
}

fn write_all(pipe: HANDLE, buffer: &[u8]) -> Result<(), String> {
    if buffer.len() > AUTH_PIPE_CAPACITY as usize {
        return Err("Worker authorization record exceeds its channel capacity".into());
    }
    let mut written = 0;
    unsafe { WriteFile(pipe, Some(buffer), Some(&mut written), None) }
        .map_err(|_| "Worker authorization channel could not be written".to_string())?;
    if written as usize != buffer.len() {
        return Err("Worker authorization channel accepted an incomplete record".into());
    }
    Ok(())
}

struct Hello {
    supervisor_id: u32,
    worker_id: u32,
    nonce: [u8; NONCE_LENGTH],
    desktop_name_length: usize,
}

impl Hello {
    fn decode_header(header: &[u8; HELLO_HEADER_LENGTH]) -> Result<Self, String> {
        if &header[..8] != HELLO_MAGIC
            || u16::from_le_bytes(header[8..10].try_into().unwrap()) != AUTH_VERSION
        {
            return Err("Unsupported worker authorization protocol".into());
        }
        let desktop_name_length = u16::from_le_bytes(header[34..36].try_into().unwrap()) as usize;
        if desktop_name_length == 0 || desktop_name_length > MAX_DESKTOP_NAME_BYTES {
            return Err("Worker authorization desktop identity has an invalid length".into());
        }
        Ok(Self {
            supervisor_id: u32::from_le_bytes(header[10..14].try_into().unwrap()),
            worker_id: u32::from_le_bytes(header[14..18].try_into().unwrap()),
            nonce: header[18..34].try_into().unwrap(),
            desktop_name_length,
        })
    }
}

fn hello_message(
    supervisor_id: u32,
    worker_id: u32,
    nonce: &[u8; NONCE_LENGTH],
    desktop_name: &str,
) -> Result<Vec<u8>, String> {
    validate_session_desktop_name(desktop_name)?;
    let mut message = Vec::with_capacity(HELLO_HEADER_LENGTH + desktop_name.len());
    message.extend_from_slice(HELLO_MAGIC);
    message.extend_from_slice(&AUTH_VERSION.to_le_bytes());
    message.extend_from_slice(&supervisor_id.to_le_bytes());
    message.extend_from_slice(&worker_id.to_le_bytes());
    message.extend_from_slice(nonce);
    message.extend_from_slice(&(desktop_name.len() as u16).to_le_bytes());
    message.extend_from_slice(desktop_name.as_bytes());
    Ok(message)
}

fn acknowledgement(worker_id: u32, nonce: &[u8; NONCE_LENGTH]) -> [u8; ACK_LENGTH] {
    let mut message = [0u8; ACK_LENGTH];
    message[..8].copy_from_slice(ACK_MAGIC);
    message[8..12].copy_from_slice(&worker_id.to_le_bytes());
    message[12..].copy_from_slice(nonce);
    message
}

fn commit_message(nonce: &[u8; NONCE_LENGTH]) -> [u8; COMMIT_LENGTH] {
    let mut message = [0u8; COMMIT_LENGTH];
    message[..8].copy_from_slice(COMMIT_MAGIC);
    message[8..].copy_from_slice(nonce);
    message
}

fn parse_handle_address(value: &str) -> Result<usize, String> {
    let address = value
        .parse::<usize>()
        .map_err(|_| "Worker authorization handle address is invalid".to_string())?;
    if address == 0 || address == usize::MAX {
        return Err("Worker authorization handle address is invalid".into());
    }
    Ok(address)
}

fn build_worker_command_line(
    executable: &Path,
    worker_arguments: &[&str],
    authorization: WorkerAuthArguments,
) -> Result<Vec<u16>, String> {
    let mut command_line = Vec::new();
    append_quoted_argument(&mut command_line, executable.as_os_str())?;
    let internal_arguments = [
        AUTH_READ_ARGUMENT.to_owned(),
        authorization.read_handle.to_string(),
        AUTH_WRITE_ARGUMENT.to_owned(),
        authorization.write_handle.to_string(),
        AUTH_PARENT_ARGUMENT.to_owned(),
        authorization.parent_handle.to_string(),
    ];
    for argument in worker_arguments
        .iter()
        .copied()
        .chain(internal_arguments.iter().map(String::as_str))
    {
        command_line.push(b' ' as u16);
        append_quoted_argument(&mut command_line, OsStr::new(argument))?;
    }
    command_line.push(0);
    if command_line.len() > MAX_COMMAND_LINE_CODE_UNITS {
        return Err("Worker command line exceeds the Windows length limit".into());
    }
    Ok(command_line)
}

/// Quotes one Windows CRT argument without allowing a URL to add launch flags.
/// Time: O(n); space: O(n), where n is the UTF-16 argument length.
fn append_quoted_argument(command_line: &mut Vec<u16>, argument: &OsStr) -> Result<(), String> {
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    command_line.push(QUOTE);
    let mut backslashes = 0;
    for character in argument.encode_wide() {
        if character == 0 {
            return Err("Worker arguments cannot contain null characters".into());
        }
        if character == BACKSLASH {
            backslashes += 1;
            continue;
        }
        let escape_count = if character == QUOTE {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        command_line.extend(std::iter::repeat_n(BACKSLASH, escape_count));
        command_line.push(character);
        backslashes = 0;
    }
    command_line.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
    command_line.push(QUOTE);
    Ok(())
}

fn to_wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests;
