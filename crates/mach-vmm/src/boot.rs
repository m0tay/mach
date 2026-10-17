//! Linux ARM64 kernel boot layout and image loading helpers.

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use tracing::{debug, info};

/// Guest physical RAM base address (1 GiB boundary).
pub const RAM_BASE: u64 = 0x4000_0000;

/// Default guest physical address where the Flattened Device Tree is placed.
pub const FDT_LOAD_ADDR: u64 = 0x4000_0000;

/// Maximum size allowed for the device tree blob (2 MiB).
pub const FDT_MAX_SIZE: u64 = 2 * 1024 * 1024;

/// Default guest physical address where the Linux kernel Image is placed (2 MiB after RAM start).
pub const KERNEL_LOAD_ADDR: u64 = 0x4020_0000;

/// Default guest physical address where the initrd is placed (128 MiB after RAM start).
pub const INITRD_LOAD_ADDR: u64 = 0x4800_0000;

/// Base physical address for PL011 UART early serial console.
pub const UART_BASE: u64 = 0x0900_0000;
/// Size of the PL011 UART MMIO region.
pub const UART_SIZE: u64 = 0x1000;

/// Base physical address for Virtio MMIO devices.
pub const MMIO_BASE: u64 = 0x0a00_0000;
/// Size allocated for each Virtio MMIO device slot.
pub const MMIO_SLOT_SIZE: u64 = 0x200;

/// Load a raw ARM64 Linux kernel `Image` from disk into the guest RAM buffer.
///
/// Returns the actual entry point physical address.
pub fn load_kernel(guest_mem: &mut [u8], ram_base: u64, kernel_path: &Path) -> Result<u64> {
    let mut file = File::open(kernel_path)
        .with_context(|| format!("failed to open kernel file: {}", kernel_path.display()))?;

    let mut kernel_bytes = Vec::new();
    file.read_to_end(&mut kernel_bytes)
        .with_context(|| "failed to read kernel file")?;

    if kernel_bytes.len() < 64 {
        bail!("kernel file too small to be a valid ARM64 Image");
    }

    // Inspect ARM64 Image header (Linux Documentation/arm64/booting.rst)
    // Offset 0x38: Magic "ARM\x64" (0x644d5241)
    let magic = u32::from_le_bytes(kernel_bytes[0x38..0x3c].try_into().unwrap());
    let mut load_addr = KERNEL_LOAD_ADDR;

    if magic == 0x644d_5241 {
        // Valid ARM64 Image
        let text_offset = u64::from_le_bytes(kernel_bytes[0x08..0x10].try_into().unwrap());
        let image_size = u64::from_le_bytes(kernel_bytes[0x10..0x18].try_into().unwrap());
        debug!(
            magic = format!("{magic:#x}"),
            text_offset = format!("{text_offset:#x}"),
            image_size = format!("{image_size:#x}"),
            "detected ARM64 Linux Image header"
        );
        load_addr = ram_base + text_offset;
    } else {
        debug!("raw kernel image without ARM64 header; using default load address");
    }

    let offset = (load_addr - ram_base) as usize;
    if offset + kernel_bytes.len() > guest_mem.len() {
        bail!("kernel image exceeds guest RAM allocation");
    }

    guest_mem[offset..offset + kernel_bytes.len()].copy_from_slice(&kernel_bytes);
    info!(
        load_addr = format!("{load_addr:#x}"),
        size = kernel_bytes.len(),
        "kernel loaded into guest memory"
    );

    Ok(load_addr)
}

/// Load an optional initial RAM disk (initrd) into guest RAM.
///
/// Returns `(guest_phys_start, guest_phys_end)`.
pub fn load_initrd(
    guest_mem: &mut [u8],
    ram_base: u64,
    ram_size: u64,
    initrd_path: &Path,
) -> Result<(u64, u64)> {
    let mut file = File::open(initrd_path)
        .with_context(|| format!("failed to open initrd file: {}", initrd_path.display()))?;

    let mut initrd_bytes = Vec::new();
    file.read_to_end(&mut initrd_bytes)
        .with_context(|| "failed to read initrd file")?;

    let initrd_size = initrd_bytes.len() as u64;

    // Place initrd at INITRD_LOAD_ADDR, or dynamically below top of RAM
    let mut load_addr = INITRD_LOAD_ADDR;
    if load_addr + initrd_size > ram_base + ram_size {
        // Place 16MB below top of RAM
        load_addr = (ram_base + ram_size).saturating_sub(initrd_size + 16 * 1024 * 1024);
    }

    let offset = (load_addr - ram_base) as usize;
    if offset + initrd_bytes.len() > guest_mem.len() {
        bail!("initrd image exceeds guest RAM allocation");
    }

    guest_mem[offset..offset + initrd_bytes.len()].copy_from_slice(&initrd_bytes);
    let end_addr = load_addr + initrd_size;

    info!(
        start = format!("{load_addr:#x}"),
        end = format!("{end_addr:#x}"),
        size = initrd_size,
        "initrd loaded into guest memory"
    );

    Ok((load_addr, end_addr))
}
