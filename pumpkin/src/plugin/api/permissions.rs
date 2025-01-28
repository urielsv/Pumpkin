use uuid::Uuid;
use std::sync::{Arc, OnceLock};

/// A trait for implementing permission checking logic
/// 
/// This trait is used by plugins to provide custom permission checking functionality.
/// Implementations should be thread-safe and efficient as they may be called frequently.
pub trait PermissionChecker: Send + Sync {
    /// Check if a player has a specific permission
    /// 
    /// # Arguments
    /// * `uuid` - The UUID of the player to check
    /// * `permission` - The permission node to check
    fn check_permission(&self, uuid: &Uuid, permission: &str) -> bool;
}

static PERMISSION_CHECKER: OnceLock<Arc<dyn PermissionChecker>> = OnceLock::new();

pub fn register_permission_checker(checker: Arc<dyn PermissionChecker>) {
    let _ = PERMISSION_CHECKER.set(checker);
}

pub fn get_permission_checker() -> Option<&'static Arc<dyn PermissionChecker>> {
    PERMISSION_CHECKER.get()
} 