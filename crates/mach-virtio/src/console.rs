//! Virtio console device implementation (Virtio 1.2 §5.3, Device ID 3).
//!
//! A virtio console device provides a virtual serial console for the guest OS.
//!
//! ## Virtqueues
//! - Queue 0: `receiveq` (host → guest / guest stdin)
//! - Queue 1: `transmitq` (guest → host / guest stdout)

use std::collections::VecDeque;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::{debug, trace};

use crate::mmio::MmioDevice;
use crate::queue::VirtQueue;

/// Virtio Console Device ID per spec.
pub const VIRTIO_ID_CONSOLE: u32 = 3;

/// Virtio console device.
pub struct VirtioConsole {
    /// Host-to-guest input receiver (drain incoming stdin from host).
    host_tx_rx: UnboundedReceiver<Vec<u8>>,
    /// Internal buffer for staged host input data not yet written into guest buffers.
    tx_buffer: VecDeque<u8>,
    /// Guest-to-host output sender (forward guest stdout to host).
    host_rx_tx: UnboundedSender<Vec<u8>>,
}

impl VirtioConsole {
    /// Create a new `VirtioConsole` device.
    ///
    /// Returns:
    /// - The device instance (to be registered in MMIO bus)
    /// - An `UnboundedSender<Vec<u8>>` for host to send stdin into guest
    /// - An `UnboundedReceiver<Vec<u8>>` for host to receive stdout from guest
    pub fn new() -> (
        Self,
        UnboundedSender<Vec<u8>>,
        UnboundedReceiver<Vec<u8>>,
    ) {
        let (host_tx_sender, host_tx_rx) = unbounded_channel();
        let (host_rx_tx, host_rx_receiver) = unbounded_channel();

        let device = Self {
            host_tx_rx,
            tx_buffer: VecDeque::new(),
            host_rx_tx,
        };

        (device, host_tx_sender, host_rx_receiver)
    }

    /// Pull any pending input chunks from the host channel into our staging buffer.
    fn drain_host_input(&mut self) {
        while let Ok(chunk) = self.host_tx_rx.try_recv() {
            self.tx_buffer.extend(chunk);
        }
    }
}

impl MmioDevice for VirtioConsole {
    fn device_id(&self) -> u32 {
        VIRTIO_ID_CONSOLE
    }

    fn device_features(&self, _page: u32) -> u32 {
        // Basic console without multiport or size negotiation
        0
    }

    fn config_read(&self, _offset: u64, _size: u8) -> u32 {
        0
    }

    fn config_write(&mut self, _offset: u64, _value: u32, _size: u8) {}

    fn queue_notify(&mut self, queue_idx: u32, mem: &mut [u8], queue: Option<&mut VirtQueue>) {
        let Some(q) = queue else {
            return;
        };

        match queue_idx {
            0 => {
                // Queue 0: receiveq (host -> guest)
                self.drain_host_input();

                while let Some(head) = q.pop_avail(mem) {
                    let chain = q.read_chain(head, mem);
                    let mut bytes_written = 0u32;

                    for (addr, len, is_write) in chain {
                        if is_write && !self.tx_buffer.is_empty() {
                            let to_write = (len as usize).min(self.tx_buffer.len());
                            let data: Vec<u8> = self.tx_buffer.drain(..to_write).collect();
                            VirtQueue::write_bytes(mem, addr, &data);
                            bytes_written += to_write as u32;
                        }
                    }

                    q.push_used(mem, head as u32, bytes_written);
                }
            }
            1 => {
                // Queue 1: transmitq (guest -> host)
                while let Some(head) = q.pop_avail(mem) {
                    let chain = q.read_chain(head, mem);

                    for (addr, len, is_write) in chain {
                        if !is_write && len > 0 {
                            let bytes = VirtQueue::read_bytes(mem, addr, len as usize);
                            if !bytes.is_empty() {
                                trace!(len = bytes.len(), "console output from guest");
                                let _ = self.host_rx_tx.send(bytes);
                            }
                        }
                    }

                    // For transmitq, 0 bytes were written by the device into the buffer
                    q.push_used(mem, head as u32, 0);
                }
            }
            other => {
                debug!(queue_idx = other, "virtio-console unknown queue notify");
            }
        }
    }

    fn device_name(&self) -> &str {
        "virtio-console"
    }
}
