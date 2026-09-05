//! Desktop module root

pub mod dock;
pub mod launch_auth;
pub mod manager;
pub mod recovery;

pub use dock::{run_default_desktop_dock, trigger_safe_desktop_switch};
pub use launch_auth::{
    authenticate_worker_launch, extract_worker_auth_arguments, AuthenticatedWorkerSession,
    SupervisedWorkerProcess, WorkerAuthArguments,
};
pub use manager::DesktopManager;
pub use recovery::{DesktopRecoveryGuard, DesktopWatchdog};
