//! MMIO bus and device dispatching (including PL011 UART emulation).

use mach_virtio::VirtioMmio;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, trace};

use crate::boot::{UART_BASE, UART_SIZE};
use crate::fdt::VirtioMmioDesc;

/// PL011 UART early console emulator.
pub struct Pl011Uart {
    console_out: Option<UnboundedSender<Vec<u8>>>,
}

impl Pl011Uart {
    /// Create a new PL011 UART emulator connected to a console output channel.
    pub fn new(console_out: Option<UnboundedSender<Vec<u8>>>) -> Self {
        Self { console_out }
    }

    /// Read a PL011 register.
    pub fn read(&self, offset: u64) -> u32 {
        match offset {
            // UARTFR (Flag Register):
            // Bit 7: TXFE (Transmit FIFO empty)
            // Bit 4: RXFE (Receive FIFO empty)
            0x18 => 0x90,
            // UARTCR (Control Register): bit 0: UARTEN
            0x30 => 0x01,
            // UARTIMSC (Interrupt Mask Set/Clear)
            0x38 => 0x00,
            _ => 0,
        }
    }

    /// Write a PL011 register.
    pub fn write(&mut self, offset: u64, value: u32) {
        match offset {
            // UARTDR (Data Register): write character to serial console
            0x00 => {
                let byte = (value & 0xff) as u8;
                if let Some(ref tx) = self.console_out {
                    let _ = tx.send(vec![byte]);
                }
            }
            _ => {}
        }
    }
}

/// MMIO bus managing all mapped device regions.
pub struct MmioBus {
    uart: Pl011Uart,
    virtio_devices: Vec<VirtioMmio>,
}

impl MmioBus {
    /// Create an empty `MmioBus`.
    pub fn new(console_out: Option<UnboundedSender<Vec<u8>>>) -> Self {
        Self {
            uart: Pl011Uart::new(console_out),
            virtio_devices: Vec::new(),
        }
    }

    /// Register a Virtio MMIO device onto the bus.
    pub fn register_virtio(&mut self, dev: VirtioMmio) {
        self.virtio_devices.push(dev);
    }

    /// Return descriptions of all Virtio MMIO devices on the bus for FDT creation.
    pub fn virtio_descriptors(&self) -> Vec<VirtioMmioDesc> {
        self.virtio_devices
            .iter()
            .map(|d| VirtioMmioDesc {
                base_addr: d.base_addr(),
                size: VirtioMmio::size(),
            })
            .collect()
    }

    /// Dispatch an MMIO read from the guest.
    pub fn read(&self, addr: u64, size: u8) -> u32 {
        trace!(addr = format!("{addr:#x}"), size, "mmio read dispatch");

        // Check PL011 UART
        if addr >= UART_BASE && addr < UART_BASE + UART_SIZE {
            return self.uart.read(addr - UART_BASE);
        }

        // Check Virtio devices
        for dev in &self.virtio_devices {
            let base = dev.base_addr();
            let len = VirtioMmio::size();
            if addr >= base && addr < base + len {
                return dev.mmio_read(addr - base, size);
            }
        }

        debug!(addr = format!("{addr:#x}"), size, "unmapped MMIO read");
        0
    }

    /// Dispatch an MMIO write from the guest.
    pub fn write(&mut self, addr: u64, value: u32, size: u8, mem: &mut [u8]) {
        trace!(
            addr = format!("{addr:#x}"),
            value = format!("{value:#x}"),
            size,
            "mmio write dispatch"
        );

        // Check PL011 UART
        if addr >= UART_BASE && addr < UART_BASE + UART_SIZE {
            self.uart.write(addr - UART_BASE, value);
            return;
        }

        // Check Virtio devices
        for dev in &mut self.virtio_devices {
            let base = dev.base_addr();
            let len = VirtioMmio::size();
            if addr >= base && addr < base + len {
                dev.mmio_write(addr - base, value, size, mem);
                return;
            }
        }

        debug!(
            addr = format!("{addr:#x}"),
            value = format!("{value:#x}"),
            size,
            "unmapped MMIO write"
        );
    }
}
