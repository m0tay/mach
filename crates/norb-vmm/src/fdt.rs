//! Flattened Device Tree (DTB/FDT) generation for ARM64 Linux boot.
//!
//! Complies with the Devicetree Specification v0.4 and Linux ARM64 boot expectations.

use anyhow::Result;
use std::collections::HashMap;

const FDT_MAGIC: u32 = 0xd00d_feed;
const FDT_VERSION: u32 = 17;
const FDT_LAST_COMP_VERSION: u32 = 16;

const FDT_BEGIN_NODE: u32 = 0x1;
const FDT_END_NODE: u32 = 0x2;
const FDT_PROP: u32 = 0x3;
#[allow(dead_code)]
const FDT_NOP: u32 = 0x4;
const FDT_END: u32 = 0x9;

/// Low-level builder for generating DTB binary blobs.
pub struct FdtBuilder {
    struct_data: Vec<u8>,
    strings_data: Vec<u8>,
    string_offsets: HashMap<String, u32>,
}

impl Default for FdtBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl FdtBuilder {
    /// Create a new, empty `FdtBuilder`.
    pub fn new() -> Self {
        Self {
            struct_data: Vec::new(),
            strings_data: Vec::new(),
            string_offsets: HashMap::new(),
        }
    }

    /// Begin a new node with `name`. Pass `""` for the root node.
    pub fn begin_node(&mut self, name: &str) {
        self.struct_data.extend_from_slice(&FDT_BEGIN_NODE.to_be_bytes());
        self.struct_data.extend_from_slice(name.as_bytes());
        self.struct_data.push(0); // null terminator

        // Pad to 4-byte boundary
        while self.struct_data.len() % 4 != 0 {
            self.struct_data.push(0);
        }
    }

    /// End the current node.
    pub fn end_node(&mut self) {
        self.struct_data.extend_from_slice(&FDT_END_NODE.to_be_bytes());
    }

    /// Intern a property name in the strings block and return its offset.
    fn intern_string(&mut self, name: &str) -> u32 {
        if let Some(&off) = self.string_offsets.get(name) {
            return off;
        }

        let off = self.strings_data.len() as u32;
        self.strings_data.extend_from_slice(name.as_bytes());
        self.strings_data.push(0);
        self.string_offsets.insert(name.to_string(), off);
        off
    }

    /// Add a raw byte property to the current node.
    pub fn add_prop_bytes(&mut self, name: &str, data: &[u8]) {
        let nameoff = self.intern_string(name);
        let len = data.len() as u32;

        self.struct_data.extend_from_slice(&FDT_PROP.to_be_bytes());
        self.struct_data.extend_from_slice(&len.to_be_bytes());
        self.struct_data.extend_from_slice(&nameoff.to_be_bytes());
        self.struct_data.extend_from_slice(data);

        // Pad to 4-byte boundary
        while self.struct_data.len() % 4 != 0 {
            self.struct_data.push(0);
        }
    }

    /// Add a 32-bit big-endian integer property.
    pub fn add_prop_u32(&mut self, name: &str, val: u32) {
        self.add_prop_bytes(name, &val.to_be_bytes());
    }

    /// Add a 64-bit big-endian integer property (two 32-bit cells).
    pub fn add_prop_u64(&mut self, name: &str, val: u64) {
        self.add_prop_bytes(name, &val.to_be_bytes());
    }

    /// Add a list of 32-bit integers property.
    pub fn add_prop_u32_array(&mut self, name: &str, vals: &[u32]) {
        let mut buf = Vec::with_capacity(vals.len() * 4);
        for &v in vals {
            buf.extend_from_slice(&v.to_be_bytes());
        }
        self.add_prop_bytes(name, &buf);
    }

    /// Add a null-terminated UTF-8 string property.
    pub fn add_prop_str(&mut self, name: &str, val: &str) {
        let mut bytes = val.as_bytes().to_vec();
        bytes.push(0);
        self.add_prop_bytes(name, &bytes);
    }

    /// Add a string list property (multiple null-terminated strings concatenated).
    pub fn add_prop_str_list(&mut self, name: &str, list: &[&str]) {
        let mut bytes = Vec::new();
        for s in list {
            bytes.extend_from_slice(s.as_bytes());
            bytes.push(0);
        }
        self.add_prop_bytes(name, &bytes);
    }

    /// Add an empty property (boolean flag).
    pub fn add_prop_empty(&mut self, name: &str) {
        self.add_prop_bytes(name, &[]);
    }

    /// Finalize the device tree into a binary DTB blob.
    pub fn finish(mut self) -> Vec<u8> {
        self.struct_data.extend_from_slice(&FDT_END.to_be_bytes());

        // Memory reservation block: empty list terminated by (0, 0) = 16 bytes
        let mem_rsv_size = 16usize;

        let header_size = 40usize;
        let off_mem_rsvmap = header_size as u32;
        let off_dt_struct = (header_size + mem_rsv_size) as u32;
        let size_dt_struct = self.struct_data.len() as u32;
        let off_dt_strings = off_dt_struct + size_dt_struct;
        let size_dt_strings = self.strings_data.len() as u32;
        let totalsize = off_dt_strings + size_dt_strings;

        let mut dtb = Vec::with_capacity(totalsize as usize);

        // Header (40 bytes, all 32-bit BE)
        dtb.extend_from_slice(&FDT_MAGIC.to_be_bytes());
        dtb.extend_from_slice(&totalsize.to_be_bytes());
        dtb.extend_from_slice(&off_dt_struct.to_be_bytes());
        dtb.extend_from_slice(&off_dt_strings.to_be_bytes());
        dtb.extend_from_slice(&off_mem_rsvmap.to_be_bytes());
        dtb.extend_from_slice(&FDT_VERSION.to_be_bytes());
        dtb.extend_from_slice(&FDT_LAST_COMP_VERSION.to_be_bytes());
        dtb.extend_from_slice(&0u32.to_be_bytes()); // boot_cpuid_phys
        dtb.extend_from_slice(&size_dt_strings.to_be_bytes());
        dtb.extend_from_slice(&size_dt_struct.to_be_bytes());

        // Memory reservation block: 64-bit addr (0), 64-bit size (0)
        dtb.extend_from_slice(&0u64.to_be_bytes());
        dtb.extend_from_slice(&0u64.to_be_bytes());

        // Structure block
        dtb.extend_from_slice(&self.struct_data);

        // Strings block
        dtb.extend_from_slice(&self.strings_data);

        dtb
    }
}

/// Description of a Virtio MMIO device slot for FDT creation.
pub struct VirtioMmioDesc {
    /// Guest physical base address
    pub base_addr: u64,
    /// Size of the MMIO region (usually 0x200)
    pub size: u64,
}

/// Build a complete Flattened Device Tree for booting ARM64 Linux on Norb.
pub fn build_fdt(
    ram_base: u64,
    ram_size: u64,
    cpus: u32,
    cmdline: &str,
    initrd_range: Option<(u64, u64)>,
    virtio_devices: &[VirtioMmioDesc],
) -> Result<Vec<u8>> {
    let mut builder = FdtBuilder::new();

    // Root node
    builder.begin_node("");
    builder.add_prop_u32("#address-cells", 2);
    builder.add_prop_u32("#size-cells", 2);
    builder.add_prop_str("compatible", "linux,dummy-virt");
    builder.add_prop_str("model", "norb-arm64-vmm");

    // /chosen node
    builder.begin_node("chosen");
    builder.add_prop_str("bootargs", cmdline);
    builder.add_prop_str("stdout-path", "/pl011@9000000");

    if let Some((start, end)) = initrd_range {
        builder.add_prop_u64("linux,initrd-start", start);
        builder.add_prop_u64("linux,initrd-end", end);
    }
    builder.end_node(); // chosen

    // /cpus node
    builder.begin_node("cpus");
    builder.add_prop_u32("#address-cells", 1);
    builder.add_prop_u32("#size-cells", 0);

    for i in 0..cpus.max(1) {
        let cpu_name = format!("cpu@{i}");
        builder.begin_node(&cpu_name);
        builder.add_prop_str("device_type", "cpu");
        builder.add_prop_str("compatible", "arm,arm-v8");
        builder.add_prop_u32("reg", i);
        builder.add_prop_str("enable-method", "spin-table");
        builder.end_node();
    }
    builder.end_node(); // cpus

    // /memory node
    let mem_name = format!("memory@{ram_base:x}");
    builder.begin_node(&mem_name);
    builder.add_prop_str("device_type", "memory");
    // reg is <addr_hi addr_lo size_hi size_lo>
    let reg_cells = [
        (ram_base >> 32) as u32,
        (ram_base & 0xffff_ffff) as u32,
        (ram_size >> 32) as u32,
        (ram_size & 0xffff_ffff) as u32,
    ];
    builder.add_prop_u32_array("reg", &reg_cells);
    builder.end_node(); // memory

    // PL011 UART early serial console at 0x09000000
    builder.begin_node("pl011@9000000");
    builder.add_prop_str_list("compatible", &["arm,pl011", "arm,primecell"]);
    let uart_base: u64 = 0x0900_0000;
    let uart_size: u64 = 0x1000;
    let uart_reg = [
        (uart_base >> 32) as u32,
        (uart_base & 0xffff_ffff) as u32,
        (uart_size >> 32) as u32,
        (uart_size & 0xffff_ffff) as u32,
    ];
    builder.add_prop_u32_array("reg", &uart_reg);
    builder.add_prop_u32("clock-frequency", 24_000_000);
    builder.end_node(); // pl011

    // Virtio MMIO devices
    for dev in virtio_devices {
        let node_name = format!("virtio_mmio@{:x}", dev.base_addr);
        builder.begin_node(&node_name);
        builder.add_prop_str("compatible", "virtio,mmio");
        let dev_reg = [
            (dev.base_addr >> 32) as u32,
            (dev.base_addr & 0xffff_ffff) as u32,
            (dev.size >> 32) as u32,
            (dev.size & 0xffff_ffff) as u32,
        ];
        builder.add_prop_u32_array("reg", &dev_reg);
        builder.end_node();
    }

    builder.end_node(); // root

    Ok(builder.finish())
}
