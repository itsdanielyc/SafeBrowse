//! Desktop module root

pub mod dock;
pub mod manager;
pub mod recovery;

pub use dock::run_default_desktop_dock;
pub use manager::DesktopManager;
pub use recovery::{DesktopRecoveryGuard, DesktopWatchdog};
