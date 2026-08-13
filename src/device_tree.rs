use core::str::FromStr;

extern crate alloc;
use alloc::string::String;

/*
 memory@80000000 {
        device_type = "memory"
        reg = <0x80000000 0x8000000>
    };

 reserved-memory {
        #address-cells = <0x2>
        #size-cells = <0x2>
        ranges = []

        mmode_resv1@80000000 {
            reg = <0x80000000 0x40000>
            no-map = []
        };

        mmode_resv0@80040000 {
            reg = <0x80040000 0x20000>
            no-map = []
        };
    };
*/

static SYSINFO: spin::Once<SystemInfo> = spin::Once::new();

#[derive(Debug)]
struct SystemInfo {
    pub timer_frequency: usize,
    pub cpus: usize,
    pub cpu_isa: String,
    pub ram_base: usize,
    pub total_memory: usize,
    pub block_device_addr: usize,
}

impl SystemInfo {
    pub fn new(fdt_addr: usize) -> Self {
        let fdt = unsafe { fdt::Fdt::from_ptr(fdt_addr as *const u8) }
            .expect("Could not read device tree");

        // TODO(mt): Typically QEMU only has a single memory region. Also this region is placed
        // after the reserved-memory which means we can use all of it as RAM and divide it up into
        // pages. Technically again we should get the values from the device_tree and use them in
        // the page frame allocator. But testing locally has confirmed, that the `_kernel_end`
        // symbol comes after the reserved-memory and the .bss section as defined in the linker
        // script, hence this should be safe to use right now.
        let mut ram_base = 0;
        let mut total_memory = 0;
        for region in fdt.memory().regions() {
            if ram_base == 0 {
                ram_base = region.starting_address as usize;
            }
            if let Some(size) = region.size {
                total_memory += size;
            }
        }

        let mut block_device_addr = None;

        for node in fdt.all_nodes() {
            // Check if it's a VirtIO MMIO device
            if let Some(compatible) = node.compatible() {
                if compatible.all().any(|s| s == "virtio,mmio") {
                    // Get its MMIO address
                    if let Some(mut reg) = node.reg() {
                        if let Some(region) = reg.next() {
                            let addr = region.starting_address as usize;
                            // Probe to see if it's a block device
                            unsafe {
                                let device_id =
                                    core::ptr::read_volatile((addr + 0x008) as *const u32);
                                if device_id == 2 {
                                    block_device_addr = Some(addr);
                                }
                            }
                        }
                    }
                }
            }
        }

        let cpu = fdt.cpus().next().expect("No CPU?");

        let isa = cpu.properties().find(|p| p.name == "riscv,isa");
        let value = isa.expect("No CPU ISA found").value;
        let str_value = alloc::string::String::from_utf8(value.to_vec()).expect("Invalid CPU ISA");
        let (base_isa, _) = str_value.split_once('_').expect("Invalid CPU ISA");

        SystemInfo {
            cpus: fdt.cpus().count(),
            cpu_isa: String::from_str(base_isa).expect("Invalid CPU ISA"),
            timer_frequency: fdt.cpus().next().expect("No cpu?").timebase_frequency(),
            ram_base,
            total_memory,
            block_device_addr: block_device_addr.expect("No block device found"),
        }
    }
}

pub fn init(fdt_addr: usize) {
    if SYSINFO.get().is_some() {
        log::error!("Tried to re-initialize the system info struct");
        return;
    }

    SYSINFO.call_once(|| SystemInfo::new(fdt_addr));
    log::info!("initialized");
}

fn system_info() -> &'static SystemInfo {
    SYSINFO
        .get()
        .expect("device tree accessed before device_tree::init()")
}

pub fn timer_frequency() -> usize {
    system_info().timer_frequency
}

pub fn cpus() -> usize {
    system_info().cpus
}

pub fn ram_base() -> usize {
    system_info().ram_base
}

pub fn total_memory() -> usize {
    system_info().total_memory
}

pub fn cpu_isa() -> String {
    system_info().cpu_isa.clone()
}

pub fn block_device_addr() -> usize {
    system_info().block_device_addr
}

pub fn virtio_mmio_devices(fdt_addr: usize) -> alloc::vec::Vec<usize> {
    let fdt =
        unsafe { fdt::Fdt::from_ptr(fdt_addr as *const u8) }.expect("Could not read device tree");

    let mut devices = alloc::vec::Vec::new();

    for node in fdt.all_nodes() {
        if let Some(compatible) = node.compatible()
            && compatible.all().any(|s| s == "virtio,mmio")
            && let Some(mut reg) = node.reg()
            && let Some(region) = reg.next()
        {
            devices.push(region.starting_address as usize);
        }
    }
    devices
}
