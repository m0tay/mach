//! [`VmmConfig`] — static configuration for a single VMM instance.

use std::path::PathBuf;

/// All parameters required to launch a virtual machine.
///
/// Passed by value to [`crate::Vmm::new`].
#[derive(Debug, Clone)]
pub struct VmmConfig {
    /// Path to the Linux kernel `Image` (or `vmlinuz`) file.
    pub kernel_path: PathBuf,

    /// Optional initial RAM disk to pass to the guest.
    pub initrd_path: Option<PathBuf>,

    /// Kernel command-line string (e.g. `"console=hvc0 root=/dev/vda"`).
    pub cmdline: String,

    /// Number of virtual CPUs to expose to the guest.
    pub cpus: u32,

    /// Guest RAM in mebibytes.
    pub memory_mb: u64,
}
