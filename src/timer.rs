use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::log;

static TIMER_FREQUENCY: AtomicUsize = AtomicUsize::new(0);

/// https://github.com/devicetree-org/devicetree-specification/releases/download/v0.4/devicetree-specification-v0.4.pdf
/// Section 5.2
#[derive(Debug)]
#[repr(C)]
struct FdtHeader {
    magic: u32,
    totalsize: u32,
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
    unsafe fn from_ptr(ptr: *const u8) -> Self {
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

/// Helper function that creates a timer for 1 second using the frequency read
/// from the device tree.
pub fn new_time() {
    const TIME_FN: usize = 0x54494D45;

    let freq = TIMER_FREQUENCY.load(Ordering::Acquire);

    unsafe {
        asm!(
            "rdtime t0",
            "add a0, t0, t1",
            "li a6, 0x0",
            "ecall",
            in("t1") freq,
            in("a7") TIME_FN,
            out("t0") _,
            out("a0") _,
            out("a6") _,
        )
    }
}

pub fn init(dtb_addr: usize) {
    // manually read the header to get the size of the device table
    let ptr = dtb_addr as *const u8;

    let size = {
        let header = unsafe { FdtHeader::from_ptr(ptr) };
        header.totalsize
    };

    let slice = unsafe { core::slice::from_raw_parts(ptr, size as usize) };
    let device_tree = fdt::Fdt::new(&slice).expect("Could not read device tree");

    let timer_frequency = device_tree
        .cpus()
        .next()
        .expect("No CPU?")
        .timebase_frequency();

    TIMER_FREQUENCY.store(timer_frequency, Ordering::Release);
    log!("Timer initialized @ {}MHz\n", timer_frequency / 1_000_000);
}
