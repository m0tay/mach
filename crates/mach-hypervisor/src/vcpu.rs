//! Safe wrapper for a hypervisor virtual CPU (`VCpu`).

use tracing::{debug, error, instrument, trace};

use crate::ffi::{
    hv_vcpu_create, hv_vcpu_destroy, hv_vcpu_get_reg, hv_vcpu_get_sys_reg, hv_vcpu_run,
    hv_vcpu_set_reg, hv_vcpu_set_sys_reg, HvExitReason, HvReg, HvReturn, HvSysReg, HvVcpuExit,
    HV_SUCCESS,
};
use crate::memory::HvError;

/// High-level exit event produced when a vCPU stops execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VCpuExit {
    /// The vCPU execution was canceled.
    Canceled,
    /// The guest performed an MMIO read from `addr` of `size` bytes.
    /// `reg` indicates the target GPR register (0..30) or 31 (XZR).
    MmioRead {
        /// Guest physical address (IPA)
        addr: u64,
        /// Size in bytes (1, 2, 4, 8)
        size: u8,
        /// Destination register index
        reg: u8,
    },
    /// The guest performed an MMIO write of `data` to `addr` of `size` bytes.
    MmioWrite {
        /// Guest physical address (IPA)
        addr: u64,
        /// Data being written
        data: u64,
        /// Size in bytes (1, 2, 4, 8)
        size: u8,
    },
    /// The virtual timer fired.
    VTimerActivated,
    /// An unknown or unhandled trap occurred.
    Unknown {
        /// Exception syndrome register value
        syndrome: u64,
    },
}

/// A virtual CPU running within a hypervisor VM.
pub struct VCpu {
    id: u64,
    exit_info: *mut HvVcpuExit,
}

// SAFETY: VCpu can be moved between threads, though hv_vcpu_run must be called
// by the thread that represents the vCPU.
unsafe impl Send for VCpu {}

impl VCpu {
    /// Create a new vCPU for the currently active VM.
    #[instrument(name = "VCpu::new")]
    pub fn new() -> Result<Self, HvError> {
        #[cfg(target_os = "macos")]
        {
            let mut vcpu_id: u64 = 0;
            let mut exit_ptr: *mut HvVcpuExit = std::ptr::null_mut();

            let ret: HvReturn = unsafe {
                hv_vcpu_create(
                    &mut vcpu_id,
                    &mut exit_ptr,
                    std::ptr::null_mut(),
                )
            };

            if ret != HV_SUCCESS {
                error!(ret, "hv_vcpu_create failed");
                return Err(HvError::Framework(ret));
            }

            debug!(vcpu_id, "vCPU created successfully");
            Ok(Self {
                id: vcpu_id,
                exit_info: exit_ptr,
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(HvError::NotSupported)
        }
    }

    /// Returns the internal vCPU handle ID.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Read a general-purpose or special register.
    pub fn get_reg(&self, reg: HvReg) -> Result<u64, HvError> {
        #[cfg(target_os = "macos")]
        {
            let mut val: u64 = 0;
            let ret = unsafe { hv_vcpu_get_reg(self.id, reg, &mut val) };
            if ret != HV_SUCCESS {
                return Err(HvError::Framework(ret));
            }
            Ok(val)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = reg;
            Err(HvError::NotSupported)
        }
    }

    /// Read a general-purpose register by index (0..=30). 31 returns 0 (XZR).
    pub fn get_gpr(&self, index: u8) -> Result<u64, HvError> {
        if index == 31 {
            return Ok(0);
        }
        if index > 31 {
            return Err(HvError::Register(format!("invalid register index: {index}")));
        }
        let reg = match index {
            0 => HvReg::X0,
            1 => HvReg::X1,
            2 => HvReg::X2,
            3 => HvReg::X3,
            4 => HvReg::X4,
            5 => HvReg::X5,
            6 => HvReg::X6,
            7 => HvReg::X7,
            8 => HvReg::X8,
            9 => HvReg::X9,
            10 => HvReg::X10,
            11 => HvReg::X11,
            12 => HvReg::X12,
            13 => HvReg::X13,
            14 => HvReg::X14,
            15 => HvReg::X15,
            16 => HvReg::X16,
            17 => HvReg::X17,
            18 => HvReg::X18,
            19 => HvReg::X19,
            20 => HvReg::X20,
            21 => HvReg::X21,
            22 => HvReg::X22,
            23 => HvReg::X23,
            24 => HvReg::X24,
            25 => HvReg::X25,
            26 => HvReg::X26,
            27 => HvReg::X27,
            28 => HvReg::X28,
            29 => HvReg::X29,
            30 => HvReg::X30,
            _ => unreachable!(),
        };
        self.get_reg(reg)
    }

    /// Write a general-purpose register by index (0..=30). Writing to 31 is a no-op (XZR).
    pub fn set_gpr(&self, index: u8, value: u64) -> Result<(), HvError> {
        if index == 31 {
            return Ok(());
        }
        if index > 31 {
            return Err(HvError::Register(format!("invalid register index: {index}")));
        }
        let reg = match index {
            0 => HvReg::X0,
            1 => HvReg::X1,
            2 => HvReg::X2,
            3 => HvReg::X3,
            4 => HvReg::X4,
            5 => HvReg::X5,
            6 => HvReg::X6,
            7 => HvReg::X7,
            8 => HvReg::X8,
            9 => HvReg::X9,
            10 => HvReg::X10,
            11 => HvReg::X11,
            12 => HvReg::X12,
            13 => HvReg::X13,
            14 => HvReg::X14,
            15 => HvReg::X15,
            16 => HvReg::X16,
            17 => HvReg::X17,
            18 => HvReg::X18,
            19 => HvReg::X19,
            20 => HvReg::X20,
            21 => HvReg::X21,
            22 => HvReg::X22,
            23 => HvReg::X23,
            24 => HvReg::X24,
            25 => HvReg::X25,
            26 => HvReg::X26,
            27 => HvReg::X27,
            28 => HvReg::X28,
            29 => HvReg::X29,
            30 => HvReg::X30,
            _ => unreachable!(),
        };
        self.set_reg(reg, value)
    }

    /// Write a general-purpose or special register.
    pub fn set_reg(&self, reg: HvReg, value: u64) -> Result<(), HvError> {
        #[cfg(target_os = "macos")]
        {
            let ret = unsafe { hv_vcpu_set_reg(self.id, reg, value) };
            if ret != HV_SUCCESS {
                return Err(HvError::Framework(ret));
            }
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (reg, value);
            Err(HvError::NotSupported)
        }
    }

    /// Read a system register.
    pub fn get_sys_reg(&self, reg: HvSysReg) -> Result<u64, HvError> {
        #[cfg(target_os = "macos")]
        {
            let mut val: u64 = 0;
            let ret = unsafe { hv_vcpu_get_sys_reg(self.id, reg, &mut val) };
            if ret != HV_SUCCESS {
                return Err(HvError::Framework(ret));
            }
            Ok(val)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = reg;
            Err(HvError::NotSupported)
        }
    }

    /// Write a system register.
    pub fn set_sys_reg(&self, reg: HvSysReg, value: u64) -> Result<(), HvError> {
        #[cfg(target_os = "macos")]
        {
            let ret = unsafe { hv_vcpu_set_sys_reg(self.id, reg, value) };
            if ret != HV_SUCCESS {
                return Err(HvError::Framework(ret));
            }
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (reg, value);
            Err(HvError::NotSupported)
        }
    }

    /// Get the Program Counter.
    pub fn get_pc(&self) -> Result<u64, HvError> {
        self.get_reg(HvReg::Pc)
    }

    /// Set the Program Counter.
    pub fn set_pc(&self, addr: u64) -> Result<(), HvError> {
        self.set_reg(HvReg::Pc, addr)
    }

    /// Advance the Program Counter by 4 bytes (one ARM64 instruction).
    pub fn advance_pc(&self) -> Result<(), HvError> {
        let pc = self.get_pc()?;
        self.set_pc(pc + 4)
    }

    /// Run the vCPU until an exit condition occurs.
    #[instrument(skip(self), fields(vcpu_id = self.id))]
    pub fn run(&self) -> Result<VCpuExit, HvError> {
        #[cfg(target_os = "macos")]
        {
            let ret = unsafe { hv_vcpu_run(self.id) };
            if ret != HV_SUCCESS {
                error!(ret, "hv_vcpu_run failed");
                return Err(HvError::Framework(ret));
            }

            let exit = unsafe { &*self.exit_info };
            trace!(reason = ?exit.reason, "vCPU exited");

            match exit.reason {
                HvExitReason::Canceled => Ok(VCpuExit::Canceled),
                HvExitReason::VTimerActivated => Ok(VCpuExit::VTimerActivated),
                HvExitReason::Exception => {
                    let syndrome = exit.exception.syndrome;
                    let ec = (syndrome >> 26) & 0x3f;

                    // EC 0x24: Data Abort from lower EL (MMIO access)
                    if ec == 0x24 {
                        let isv = (syndrome >> 24) & 1;
                        if isv == 1 {
                            let sas = (syndrome >> 22) & 3;
                            let size: u8 = 1 << sas;
                            let wnr = (syndrome >> 6) & 1;
                            let srt = ((syndrome >> 16) & 0x1f) as u8;
                            let addr = exit.exception.physical_address;

                            if wnr == 1 {
                                // Write from guest to MMIO
                                let raw_data = self.get_gpr(srt)?;
                                let data = match size {
                                    1 => raw_data & 0xff,
                                    2 => raw_data & 0xffff,
                                    4 => raw_data & 0xffff_ffff,
                                    _ => raw_data,
                                };
                                return Ok(VCpuExit::MmioWrite { addr, data, size });
                            } else {
                                // Read from MMIO to guest
                                return Ok(VCpuExit::MmioRead { addr, size, reg: srt });
                            }
                        }
                    }

                    Ok(VCpuExit::Unknown { syndrome })
                }
                _ => Ok(VCpuExit::Unknown {
                    syndrome: exit.exception.syndrome,
                }),
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(HvError::NotSupported)
        }
    }
}

impl Drop for VCpu {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            let ret = unsafe { hv_vcpu_destroy(self.id) };
            if ret != HV_SUCCESS {
                error!(ret, vcpu_id = self.id, "hv_vcpu_destroy failed");
            } else {
                debug!(vcpu_id = self.id, "vCPU destroyed");
            }
        }
    }
}
