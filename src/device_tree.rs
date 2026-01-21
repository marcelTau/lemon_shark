use crate::logln;
use core::{cell::UnsafeCell, str::FromStr};

extern crate alloc;
use alloc::string::String;

/// The Device Tree Header sturcture is used to get the size of the actual
/// device tree to be able to pass it to the `fdt` crate which parses it.
///
/// Find more information about the structure of the device tree
/// [here](https://github.com/devicetree-org/devicetree-specification/releases/download/v0.4/devicetree-specification-v0.4.pdf)
/// in Section 5.2
#[derive(Debug)]
#[repr(C)]
pub struct FdtHeader {
    magic: u32,
    pub totalsize: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    off_mem_rsvmap: u32,
    version: u32,
    last_comp_version: u32,
    boot_cpuid_phys: u32,
    size_dt_strings: u32,
    size_dt_struct: u32,
}

impl FdtHeader {
    /// SAFETY: `ptr` must be valid and pointing to start of the device tree.
    pub unsafe fn from_ptr(ptr: *const u8) -> Self {
        let read_be_u32 = |offset: usize| -> u32 {
            let bytes = unsafe { core::slice::from_raw_parts(ptr.add(offset), 4) };
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        };

        let magic = read_be_u32(0x0);

        assert_eq!(magic, 0xd00dfeed);

        Self {
            magic,
            totalsize: read_be_u32(0x04),
            off_dt_struct: read_be_u32(0x08),
            off_dt_strings: read_be_u32(0x0c),
            off_mem_rsvmap: read_be_u32(0x10),
            version: read_be_u32(0x14),
            last_comp_version: read_be_u32(0x18),
            boot_cpuid_phys: read_be_u32(0x1c),
            size_dt_strings: read_be_u32(0x20),
            size_dt_struct: read_be_u32(0x24),
        }
    }
}

// TODO(mt): make it impossible to re-call init and break things.
static SYSINFO: spin::Mutex<LockedSystemInfo> = spin::Mutex::new(LockedSystemInfo::new());

struct LockedSystemInfo {
    inner: UnsafeCell<Option<SystemInfo>>,
}

impl LockedSystemInfo {
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(None),
        }
    }

    pub fn init(&self, fdt_addr: usize) {
        unsafe { (*self.inner.get()).replace(SystemInfo::new(fdt_addr)) };
    }

    pub fn inner(&self) -> &SystemInfo {
        unsafe { (*self.inner.get()).as_ref().unwrap() }
    }
}

#[derive(Debug)]
struct SystemInfo {
    pub timer_frequency: usize,
    pub cpus: usize,
    pub cpu_isa: String,
    pub total_memory: usize,
}

impl SystemInfo {
    pub fn new(fdt_addr: usize) -> Self {
        let ptr = fdt_addr as *const u8;

        let size = {
            let header = unsafe { FdtHeader::from_ptr(ptr) };
            header.totalsize
        };

        let slice = unsafe { core::slice::from_raw_parts(ptr, size as usize) };
        let fdt = fdt::Fdt::new(slice).expect("Could not read device tree");

        let cpu = fdt.cpus().next().unwrap();

        let mut total_memory = 0;
        for region in fdt.memory().regions() {
            if let Some(size) = region.size {
                total_memory += size;
            }
        }

        let isa = cpu.properties().find(|p| p.name.starts_with("riscv,isa"));
        let value = isa.unwrap().value;
        let str_value = alloc::string::String::from_utf8(value.to_vec()).unwrap();
        let (base_isa, _) = str_value.split_once('_').unwrap();

        SystemInfo {
            cpus: fdt.cpus().count(),
            cpu_isa: String::from_str(base_isa).unwrap(),
            timer_frequency: fdt.cpus().next().expect("No cpu?").timebase_frequency(),
            total_memory,
        }
    }
}

pub fn init(fdt_addr: usize) {
    (*SYSINFO.lock()).init(fdt_addr);
    logln!("[DEVICE TREE] initialized");
}

pub fn timer_frequency() -> usize {
    SYSINFO.lock().inner().timer_frequency
}

pub fn cpus() -> usize {
    SYSINFO.lock().inner().cpus
}

pub fn total_memory() -> usize {
    SYSINFO.lock().inner().total_memory
}

pub fn cpu_isa() -> String {
    SYSINFO.lock().inner().cpu_isa.clone()
}
