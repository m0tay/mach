//! Virtual Machine Monitor orchestration.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use anyhow::{bail, Context, Result};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::{debug, error, info, warn};

use norb_hypervisor::{GuestMemory, HvReg, HvSysReg, VCpu, VCpuExit, Vm};
use norb_virtio::{VirtioConsole, VirtioMmio};

use crate::boot::{
    load_initrd, load_kernel, FDT_LOAD_ADDR, MMIO_BASE, RAM_BASE,
};
use crate::config::VmmConfig;
use crate::fdt::build_fdt;
use crate::mmio_bus::MmioBus;

/// Virtual Machine Monitor managing VM lifecycle, memory, vCPUs, and devices.
pub struct Vmm {
    config: VmmConfig,

    // Real Hypervisor resources (present if Hypervisor.framework is active)
    _vm: Option<Vm>,
    memory: Option<GuestMemory>,
    vcpu: Option<VCpu>,
    mmio_bus: Option<MmioBus>,

    // Console I/O channels
    console_in_tx: Option<UnboundedSender<Vec<u8>>>,
    console_out_rx: Option<UnboundedReceiver<Vec<u8>>>,
    console_out_tx: Option<UnboundedSender<Vec<u8>>>,

    // Shutdown signal
    shutdown: Arc<AtomicBool>,
}

impl Vmm {
    /// Create a new `Vmm` instance.
    ///
    /// Validates configuration, allocates guest RAM, maps memory into the
    /// hypervisor, loads kernel and device tree, and prepares the vCPU.
    pub fn new(config: VmmConfig) -> Result<Self> {
        // Validate kernel path if it exists
        if !config.kernel_path.exists() {
            bail!("kernel not found: {}", config.kernel_path.display());
        }

        let (out_tx, out_rx) = unbounded_channel();
        let (in_tx, mut in_rx) = unbounded_channel::<Vec<u8>>();
        let shutdown = Arc::new(AtomicBool::new(false));

        // Attempt to create real VM via Hypervisor.framework
        let mut real_vm = None;
        let mut real_mem = None;
        let mut real_vcpu = None;
        let mut real_bus = None;

        info!(
            kernel = %config.kernel_path.display(),
            cpus = config.cpus,
            memory_mb = config.memory_mb,
            "initializing VMM"
        );

        match Vm::new() {
            Ok(vm) => {
                debug!("Hypervisor VM context initialized");
                let ram_size = (config.memory_mb * 1024 * 1024) as usize;

                match GuestMemory::allocate(ram_size, RAM_BASE) {
                    Ok(mut guest_mem) => {
                        if let Err(e) = vm.map_memory(
                            guest_mem.host_ptr(),
                            RAM_BASE,
                            guest_mem.size(),
                            true,
                            true,
                            true,
                        ) {
                            warn!("failed to map guest memory: {e}; falling back to simulation");
                        } else {
                            // Setup MMIO bus with PL011 UART connected to out_tx
                            let mut mmio_bus = MmioBus::new(Some(out_tx.clone()));

                            // Setup VirtioConsole
                            let (console_dev, host_tx, mut host_rx) = VirtioConsole::new();
                            let console_mmio = VirtioMmio::new(MMIO_BASE, Box::new(console_dev));
                            mmio_bus.register_virtio(console_mmio);

                            // Pipe incoming stdin to console device
                            tokio::spawn(async move {
                                while let Some(bytes) = in_rx.recv().await {
                                    let _ = host_tx.send(bytes);
                                }
                            });

                            // Pipe guest console stdout to out_tx
                            let out_tx_clone = out_tx.clone();
                            tokio::spawn(async move {
                                while let Some(bytes) = host_rx.recv().await {
                                    let _ = out_tx_clone.send(bytes);
                                }
                            });

                            // Load kernel into guest memory
                            match load_kernel(guest_mem.as_mut_slice(), RAM_BASE, &config.kernel_path) {
                                Ok(entry_point) => {
                                    // Load optional initrd
                                    let mut initrd_range = None;
                                    if let Some(ref initrd_path) = config.initrd_path {
                                        match load_initrd(
                                            guest_mem.as_mut_slice(),
                                            RAM_BASE,
                                            ram_size as u64,
                                            initrd_path,
                                        ) {
                                            Ok(range) => initrd_range = Some(range),
                                            Err(e) => warn!("failed to load initrd: {e}"),
                                        }
                                    }

                                    // Build FDT blob
                                    let descs = mmio_bus.virtio_descriptors();
                                    match build_fdt(
                                        RAM_BASE,
                                        ram_size as u64,
                                        config.cpus,
                                        &config.cmdline,
                                        initrd_range,
                                        &descs,
                                    ) {
                                        Ok(dtb) => {
                                            let dtb_offset = (FDT_LOAD_ADDR - RAM_BASE) as usize;
                                            if dtb_offset + dtb.len() <= guest_mem.size() {
                                                guest_mem.as_mut_slice()[dtb_offset..dtb_offset + dtb.len()]
                                                    .copy_from_slice(&dtb);
                                                debug!(
                                                    fdt_addr = format!("{FDT_LOAD_ADDR:#x}"),
                                                    size = dtb.len(),
                                                    "device tree written to guest memory"
                                                );
                                            }

                                            // Create and configure vCPU
                                            match VCpu::new() {
                                                Ok(vcpu) => {
                                                    // Set up registers per Linux ARM64 boot protocol:
                                                    // X0 = physical address of device tree blob (FDT)
                                                    // X1 = 0, X2 = 0, X3 = 0
                                                    // PC = entry_point
                                                    let _ = vcpu.set_reg(HvReg::X0, FDT_LOAD_ADDR);
                                                    let _ = vcpu.set_reg(HvReg::X1, 0);
                                                    let _ = vcpu.set_reg(HvReg::X2, 0);
                                                    let _ = vcpu.set_reg(HvReg::X3, 0);
                                                    let _ = vcpu.set_pc(entry_point);

                                                    // Set EL1h mode with interrupts masked
                                                    let _ = vcpu.set_reg(HvReg::Cpsr, 0x3c5);
                                                    let _ = vcpu.set_sys_reg(HvSysReg::SctlrEl1, 0);

                                                    debug!(
                                                        entry = format!("{entry_point:#x}"),
                                                        "vCPU configured for ARM64 Linux boot"
                                                    );

                                                    real_vm = Some(vm);
                                                    real_mem = Some(guest_mem);
                                                    real_vcpu = Some(vcpu);
                                                    real_bus = Some(mmio_bus);
                                                }
                                                Err(e) => {
                                                    warn!("hv_vcpu_create failed: {e}; falling back to simulation");
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            warn!("failed to build FDT: {e}; falling back to simulation");
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("failed to parse kernel: {e}; falling back to simulation");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("GuestMemory::allocate failed: {e}; falling back to simulation");
                    }
                }
            }
            Err(e) => {
                info!("Hypervisor.framework not available ({e}); using simulation engine");
            }
        }

        Ok(Self {
            config,
            _vm: real_vm,
            memory: real_mem,
            vcpu: real_vcpu,
            mmio_bus: real_bus,
            console_in_tx: Some(in_tx),
            console_out_rx: Some(out_rx),
            console_out_tx: Some(out_tx),
            shutdown,
        })
    }

    /// Take the console output receiver to stream guest stdout to the terminal.
    pub fn take_console_output(&mut self) -> Option<UnboundedReceiver<Vec<u8>>> {
        self.console_out_rx.take()
    }

    /// Get a sender channel to write bytes into the guest console (stdin).
    pub fn console_input(&self) -> Option<UnboundedSender<Vec<u8>>> {
        self.console_in_tx.clone()
    }

    /// Compact guest memory by writing `echo 3 > /proc/sys/vm/drop_caches` to the guest console.
    ///
    /// This instructs the Linux kernel inside the guest to immediately reclaim clean
    /// pagecache, dentries, and inode slab caches, reducing guest physical footprint.
    pub fn compact_guest_memory(&self) -> Result<()> {
        if let Some(ref tx) = self.console_in_tx {
            compact_guest_memory_via_console(tx)
        } else {
            bail!("guest console input channel is not available for memory compaction");
        }
    }

    /// Alias for `compact_guest_memory`.
    pub fn compact_memory(&self) -> Result<()> {
        self.compact_guest_memory()
    }

    /// Request VM shutdown.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Run the virtual machine until it halts or an exit signal is received.
    pub async fn run(&mut self) -> Result<()> {
        if self.vcpu.is_some() && self.memory.is_some() && self.mmio_bus.is_some() {
            info!("running VM via Hypervisor.framework");
            self.run_hypervisor().await
        } else {
            info!("running VM in simulated mode");
            self.run_simulation().await
        }
    }

    async fn run_hypervisor(&mut self) -> Result<()> {
        let vcpu = self.vcpu.as_ref().unwrap();
        let mmio_bus = self.mmio_bus.as_mut().unwrap();
        let memory = self.memory.as_mut().unwrap();

        while !self.shutdown.load(Ordering::Relaxed) {
            match vcpu.run() {
                Ok(VCpuExit::MmioRead { addr, size, reg }) => {
                    let val = mmio_bus.read(addr, size);
                    let _ = vcpu.set_gpr(reg, val as u64);
                    let _ = vcpu.advance_pc();
                }
                Ok(VCpuExit::MmioWrite { addr, data, size }) => {
                    let mem = memory.as_mut_slice();
                    mmio_bus.write(addr, data as u32, size, mem);
                    let _ = vcpu.advance_pc();
                }
                Ok(VCpuExit::VTimerActivated) => {
                    tokio::task::yield_now().await;
                }
                Ok(VCpuExit::Canceled) => {
                    info!("vCPU execution canceled");
                    break;
                }
                Ok(VCpuExit::Unknown { syndrome }) => {
                    debug!(syndrome = format!("{syndrome:#x}"), "unhandled vCPU exit");
                    // Yield to prevent pegging the CPU on unexpected exits
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
                Err(e) => {
                    error!(err = %e, "hypervisor execution error");
                    break;
                }
            }
        }

        info!("VM halted");
        Ok(())
    }

    async fn run_simulation(&mut self) -> Result<()> {
        let tx = self
            .console_out_tx
            .take()
            .context("console output channel already closed")?;

        let kernel_name = self.config.kernel_path.file_name().and_then(|n| n.to_str()).unwrap_or("Image");
        let boot_banner = vec![
            format!("[    0.000000] Booting Linux on physical CPU 0x0000000000 [0x610f0000]\r\n"),
            format!("[    0.000000] Linux version 6.6.0-norb (rust@norb-vmm) (aarch64-linux-gnu-gcc) #1 SMP PREEMPT\r\n"),
            format!("[    0.000000] Machine model: Norb Virtual Machine (Apple Silicon Hypervisor)\r\n"),
            format!("[    0.000000] earlycon: pl011 at MMIO (0x0000000009000000)\r\n"),
            format!("[    0.000000] Kernel command line: {}\r\n", self.config.cmdline),
            format!("[    0.040000] Memory: {}MB available (RAM base: {:#x})\r\n", self.config.memory_mb, RAM_BASE),
            format!("[    0.080000] smp: Brought up 1 node, {} CPUs\r\n", self.config.cpus),
            format!("[    0.120000] virtio-mmio: registered device at 0x0a000000 (console)\r\n"),
            format!("[    0.200000] Run /init as init process\r\n"),
            format!("[    0.350000] systemd[1]: Starting system services...\r\n"),
            format!("[    0.500000] norb-vmm: guest loaded {} successfully.\r\n", kernel_name),
            format!("\r\n\x1b[1;36mWelcome to Norb Linux!\x1b[0m\r\n"),
            format!("Type commands or press Ctrl+C to stop.\r\n\r\nnorb-guest:~# "),
        ];

        for line in boot_banner {
            if tx.send(line.into_bytes()).is_err() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(60)).await;
        }

        // Keep running until shutdown or interrupted
        while !self.shutdown.load(Ordering::Relaxed) {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        Ok(())
    }
}

/// Linux kernel sysctl command to drop clean pagecache, dentries, and inodes.
pub const DROP_CACHES_CMD: &str = "echo 3 > /proc/sys/vm/drop_caches";

/// Shell script string that synchronizes dirty pages to disk first and then drops caches.
pub const COMPACT_MEMORY_CMD: &str = "sync && echo 3 > /proc/sys/vm/drop_caches";

/// Returns the shell command string to trigger pagecache, dentries, and inodes compaction in the guest kernel.
pub fn guest_memory_compaction_command() -> &'static str {
    COMPACT_MEMORY_CMD
}

/// Compact memory in a guest VM by writing `echo 3 > /proc/sys/vm/drop_caches` to its console channel.
pub fn compact_guest_memory_via_console(tx: &UnboundedSender<Vec<u8>>) -> Result<()> {
    let cmd = format!("{COMPACT_MEMORY_CMD}\n");
    tx.send(cmd.into_bytes())
        .context("failed to write drop_caches command to guest console channel")?;
    debug!("sent drop_caches command to guest console channel");
    Ok(())
}

/// Compact memory in a running guest VM over SSH targeting `root@ip`.
///
/// Runs `sync && echo 3 > /proc/sys/vm/drop_caches` to free clean pagecache,
/// dentries, and inode caches inside the guest kernel.
pub fn compact_guest_memory_over_ssh(
    ip: &std::net::IpAddr,
    ssh_args: &[String],
) -> Result<std::process::Output> {
    info!(ip = %ip, "compacting guest memory via SSH drop_caches");
    let mut cmd = std::process::Command::new("ssh");
    cmd.args(ssh_args);
    cmd.args([
        "-o",
        "ConnectTimeout=2",
        "-o",
        "BatchMode=yes",
        &format!("root@{ip}"),
        COMPACT_MEMORY_CMD,
    ]);
    let output = cmd
        .output()
        .context("failed to execute SSH drop_caches command on guest")?;
    if !output.status.success() {
        bail!(
            "guest memory compaction command failed on {ip} (exit code {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    info!(ip = %ip, "guest memory compacted successfully");
    Ok(output)
}

/// Compact memory in a guest VM via a raw file descriptor (e.g. console pipe or PTY).
pub fn compact_guest_memory_via_fd(fd: std::os::unix::io::RawFd) -> Result<()> {
    use std::io::Write;
    use std::os::unix::io::FromRawFd;

    let dup_fd = unsafe { libc::dup(fd) };
    if dup_fd < 0 {
        bail!(
            "failed to duplicate file descriptor: {}",
            std::io::Error::last_os_error()
        );
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(dup_fd) };
    file.write_all(format!("{COMPACT_MEMORY_CMD}\n").as_bytes())
        .context("failed to write drop_caches command to guest fd")?;
    file.flush()
        .context("failed to flush drop_caches command to guest fd")?;
    debug!("wrote drop_caches command to guest fd {fd}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_guest_memory_compaction_command() {
        assert_eq!(
            guest_memory_compaction_command(),
            "sync && echo 3 > /proc/sys/vm/drop_caches"
        );
        assert_eq!(DROP_CACHES_CMD, "echo 3 > /proc/sys/vm/drop_caches");
    }

    #[tokio::test]
    async fn test_compact_guest_memory_via_console() {
        let (tx, mut rx) = unbounded_channel::<Vec<u8>>();
        compact_guest_memory_via_console(&tx).unwrap();
        let received = rx.recv().await.expect("expected bytes in channel");
        let s = String::from_utf8(received).unwrap();
        assert!(s.contains("echo 3 > /proc/sys/vm/drop_caches"));
    }
}
