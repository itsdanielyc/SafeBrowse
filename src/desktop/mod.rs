//! Desktop module root

pub mod manager;
pub mod recovery;

pub use manager::DesktopManager;
pub use recovery::{DesktopRecoveryGuard, DesktopWatchdog};
