//! Process-token integrity checks for the browser host.
//!
//! A browser process must never inherit administrator privileges. Web content is
//! intentionally untrusted, so starting SafeBrowse with a high-integrity token
//! would unnecessarily enlarge the impact of a browser or host compromise.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, IsValidSid, TokenElevation,
    TokenIntegrityLevel, TOKEN_ELEVATION, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::SystemServices::{
    SECURITY_MANDATORY_HIGH_RID, SECURITY_MANDATORY_LOW_RID, SECURITY_MANDATORY_MEDIUM_PLUS_RID,
    SECURITY_MANDATORY_MEDIUM_RID, SECURITY_MANDATORY_PROTECTED_PROCESS_RID,
    SECURITY_MANDATORY_SYSTEM_RID,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

const SID_HEADER_BYTES: usize = 8;
const SID_SUB_AUTHORITY_BYTES: usize = size_of::<u32>();

/// The integrity band represented by the mandatory-label RID in a process token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegrityLevel {
    Untrusted,
    Low,
    Medium,
    MediumPlus,
    High,
    System,
    ProtectedProcess,
}

impl IntegrityLevel {
    /// Returns whether this level grants at least high-integrity access.
    #[must_use]
    pub const fn is_high_or_above(self) -> bool {
        matches!(self, Self::High | Self::System | Self::ProtectedProcess)
    }
}

/// Security properties read from the current process token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessIntegrity {
    pub integrity_rid: u32,
    pub level: IntegrityLevel,
    pub token_is_elevated: bool,
}

impl ProcessIntegrity {
    /// Returns whether SafeBrowse must reject this process token.
    #[must_use]
    pub const fn requires_browser_host_refusal(self) -> bool {
        self.token_is_elevated || self.level.is_high_or_above()
    }
}

/// Classifies a mandatory integrity RID into its documented Windows band.
#[must_use]
pub const fn classify_integrity_rid(integrity_rid: u32) -> IntegrityLevel {
    if integrity_rid >= SECURITY_MANDATORY_PROTECTED_PROCESS_RID as u32 {
        return IntegrityLevel::ProtectedProcess;
    }
    if integrity_rid >= SECURITY_MANDATORY_SYSTEM_RID as u32 {
        return IntegrityLevel::System;
    }
    if integrity_rid >= SECURITY_MANDATORY_HIGH_RID as u32 {
        return IntegrityLevel::High;
    }
    if integrity_rid >= SECURITY_MANDATORY_MEDIUM_PLUS_RID {
        return IntegrityLevel::MediumPlus;
    }
    if integrity_rid >= SECURITY_MANDATORY_MEDIUM_RID as u32 {
        return IntegrityLevel::Medium;
    }
    if integrity_rid >= SECURITY_MANDATORY_LOW_RID as u32 {
        return IntegrityLevel::Low;
    }
    IntegrityLevel::Untrusted
}

/// Reads elevation and mandatory-integrity information from the current process token.
pub fn current_process_integrity() -> Result<ProcessIntegrity, String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|error| format!("Could not open the current process token: {error}"))?;
    let token = OwnedTokenHandle(token);

    let token_is_elevated = query_token_elevation(token.get())?;
    let integrity_rid = query_token_integrity_rid(token.get())?;
    Ok(ProcessIntegrity {
        integrity_rid,
        level: classify_integrity_rid(integrity_rid),
        token_is_elevated,
    })
}

/// Rejects administrator and high-integrity launches before the browser host starts.
pub fn refuse_elevated_browser_host() -> Result<(), String> {
    let integrity = current_process_integrity().map_err(|error| {
        format!(
            "SafeBrowse could not verify that it is running without administrator privileges: \
             {error}. SafeBrowse will not start."
        )
    })?;

    if !integrity.requires_browser_host_refusal() {
        return Ok(());
    }

    Err(
        "SafeBrowse cannot start with administrator or high-integrity privileges.\n\n\
         Open SafeBrowse without \"Run as administrator\". If Windows also opens ordinary apps \
         with administrator privileges, use a standard Windows account or enable User Account \
         Control (UAC), restart Windows, and try again.\n\n\
         See README.md under Startup troubleshooting for the UAC repair command and restart steps.\n\n\
         No browsing session was started."
            .to_string(),
    )
}

/// Owns the token handle returned by `OpenProcessToken`.
struct OwnedTokenHandle(HANDLE);

impl OwnedTokenHandle {
    fn get(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedTokenHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

fn query_token_elevation(token: HANDLE) -> Result<bool, String> {
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned_bytes = 0;
    unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast::<c_void>()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_bytes,
        )
    }
    .map_err(|error| format!("Could not read token elevation: {error}"))?;

    if returned_bytes < size_of::<TOKEN_ELEVATION>() as u32 {
        return Err("Windows returned incomplete token elevation data".to_string());
    }
    Ok(elevation.TokenIsElevated != 0)
}

fn query_token_integrity_rid(token: HANDLE) -> Result<u32, String> {
    let mut required_bytes = 0;
    let probe =
        unsafe { GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut required_bytes) };
    if required_bytes < size_of::<TOKEN_MANDATORY_LABEL>() as u32 {
        return Err(format!(
            "Could not determine token integrity buffer size: {}",
            probe.err().map_or_else(
                || "Windows returned no data".to_string(),
                |error| error.to_string()
            )
        ));
    }

    // A word-backed buffer provides sufficient alignment for TOKEN_MANDATORY_LABEL.
    let word_bytes = size_of::<usize>();
    let word_count = (required_bytes as usize).div_ceil(word_bytes);
    let mut buffer = vec![0usize; word_count];
    let mut returned_bytes = 0;
    unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            required_bytes,
            &mut returned_bytes,
        )
    }
    .map_err(|error| format!("Could not read token integrity level: {error}"))?;

    if returned_bytes < size_of::<TOKEN_MANDATORY_LABEL>() as u32
        || returned_bytes as usize > buffer.len() * word_bytes
    {
        return Err("Windows returned invalid token integrity data length".to_string());
    }

    let mandatory_label = unsafe { &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    let sid = mandatory_label.Label.Sid;
    let buffer_start = buffer.as_ptr() as usize;
    let buffer_end = buffer_start + returned_bytes as usize;
    let sid_start = sid.0 as usize;
    if sid_start < buffer_start || sid_start.saturating_add(SID_HEADER_BYTES) > buffer_end {
        return Err("Windows returned an out-of-range integrity SID".to_string());
    }

    let sub_authority_count = unsafe { *GetSidSubAuthorityCount(sid) } as usize;
    let sid_bytes = SID_HEADER_BYTES
        .checked_add(
            sub_authority_count
                .checked_mul(SID_SUB_AUTHORITY_BYTES)
                .ok_or_else(|| "Token integrity SID length overflowed".to_string())?,
        )
        .ok_or_else(|| "Token integrity SID length overflowed".to_string())?;
    if sub_authority_count == 0 || sid_start.saturating_add(sid_bytes) > buffer_end {
        return Err("Windows returned an invalid integrity SID length".to_string());
    }
    if !unsafe { IsValidSid(sid) }.as_bool() {
        return Err("Windows returned an invalid integrity SID".to_string());
    }

    let rid_pointer = unsafe { GetSidSubAuthority(sid, sub_authority_count as u32 - 1) };
    if rid_pointer.is_null()
        || (rid_pointer as usize) < sid_start
        || (rid_pointer as usize).saturating_add(SID_SUB_AUTHORITY_BYTES) > buffer_end
    {
        return Err("Windows returned an out-of-range integrity RID".to_string());
    }
    Ok(unsafe { *rid_pointer })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_integrity_boundaries() {
        let cases = [
            (0, IntegrityLevel::Untrusted),
            (SECURITY_MANDATORY_LOW_RID as u32, IntegrityLevel::Low),
            (SECURITY_MANDATORY_MEDIUM_RID as u32, IntegrityLevel::Medium),
            (
                SECURITY_MANDATORY_MEDIUM_PLUS_RID,
                IntegrityLevel::MediumPlus,
            ),
            (SECURITY_MANDATORY_HIGH_RID as u32, IntegrityLevel::High),
            (SECURITY_MANDATORY_SYSTEM_RID as u32, IntegrityLevel::System),
            (
                SECURITY_MANDATORY_PROTECTED_PROCESS_RID as u32,
                IntegrityLevel::ProtectedProcess,
            ),
        ];

        for (rid, expected) in cases {
            assert_eq!(classify_integrity_rid(rid), expected, "RID {rid:#x}");
        }
    }

    #[test]
    fn refuses_elevated_flag_or_high_integrity() {
        let standard = ProcessIntegrity {
            integrity_rid: SECURITY_MANDATORY_MEDIUM_RID as u32,
            level: IntegrityLevel::Medium,
            token_is_elevated: false,
        };
        let elevated_flag = ProcessIntegrity {
            token_is_elevated: true,
            ..standard
        };
        let high_integrity = ProcessIntegrity {
            integrity_rid: SECURITY_MANDATORY_HIGH_RID as u32,
            level: IntegrityLevel::High,
            token_is_elevated: false,
        };

        assert!(!standard.requires_browser_host_refusal());
        assert!(elevated_flag.requires_browser_host_refusal());
        assert!(high_integrity.requires_browser_host_refusal());
    }

    #[test]
    fn live_token_matches_the_pure_classification() {
        let integrity =
            current_process_integrity().expect("current process token should be readable");

        assert_eq!(
            integrity.level,
            classify_integrity_rid(integrity.integrity_rid)
        );
        assert_eq!(
            integrity.requires_browser_host_refusal(),
            integrity.token_is_elevated
                || integrity.integrity_rid >= SECURITY_MANDATORY_HIGH_RID as u32
        );
    }
}
