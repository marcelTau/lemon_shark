extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
pub use virtual_memory::PhysRange;
use virtual_memory::{PhysRangeError, normalize_ranges};

static SYSINFO: spin::Once<SystemInfo> = spin::Once::new();

#[derive(Debug)]
struct SystemInfo {
    timer_frequency: usize,
    cpus: usize,
    cpu_isa: String,
    memory_regions: Vec<PhysRange>,
    reserved_memory_regions: Vec<PhysRange>,
    mmio_regions: Vec<PhysRange>,
    fdt_range: PhysRange,
    total_memory: usize,
    block_device_addr: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeviceTreeError {
    InvalidFdt,
    AddressOverflow,
    EmptyRange,
    MissingMemoryReg,
    MissingRegionSize,
    NoMemoryRegions,
    MalformedReservedMemory,
    DynamicReservedMemoryUnsupported,
    MissingMmioReg,
    NoMmioRegions,
    MissingCpu,
    MissingCpuIsa,
    InvalidCpuIsa,
    MissingBlockDevice,
    AlreadyInitialized,
}

impl From<PhysRangeError> for DeviceTreeError {
    fn from(error: PhysRangeError) -> Self {
        match error {
            PhysRangeError::AddressOverflow => Self::AddressOverflow,
            PhysRangeError::EmptyRange => Self::EmptyRange,
        }
    }
}

fn node_is_enabled(node: &fdt::node::FdtNode<'_, '_>) -> bool {
    matches!(
        node.property("status")
            .and_then(|property| property.as_str()),
        None | Some("okay") | Some("ok")
    )
}

fn node_has_compatible(node: &fdt::node::FdtNode<'_, '_>, expected: &str) -> bool {
    node.compatible()
        .is_some_and(|compatible| compatible.all().any(|value| value == expected))
}

fn node_is_kernel_mmio_device(node: &fdt::node::FdtNode<'_, '_>) -> bool {
    node_has_compatible(node, "ns16550a") || node_has_compatible(node, "virtio,mmio")
}

/// Collect all enabled nodes whose `device_type` is `memory`.
fn collect_memory_regions(fdt: &fdt::Fdt<'_>) -> Result<Vec<PhysRange>, DeviceTreeError> {
    let mut ranges = Vec::new();

    for node in fdt.all_nodes().filter(node_is_enabled) {
        let is_memory = node
            .property("device_type")
            .and_then(|property| property.as_str())
            == Some("memory");

        if !is_memory {
            continue;
        }

        let regions = node.reg().ok_or(DeviceTreeError::MissingMemoryReg)?;

        for region in regions {
            let size = region.size.ok_or(DeviceTreeError::MissingRegionSize)?;

            ranges.push(PhysRange::from_start_size(
                region.starting_address as usize,
                size,
            )?);
        }
    }

    if ranges.is_empty() {
        return Err(DeviceTreeError::NoMemoryRegions);
    }

    Ok(normalize_ranges(ranges))
}

/// Collect both kinds of FDT reservations: entries in the binary memory
/// reservation block and children of the `/reserved-memory` node.
fn collect_reserved_memory_regions(fdt: &fdt::Fdt<'_>) -> Result<Vec<PhysRange>, DeviceTreeError> {
    let mut ranges = Vec::new();

    for reservation in fdt.memory_reservations() {
        ranges.push(PhysRange::from_start_size(
            reservation.address() as usize,
            reservation.size(),
        )?);
    }

    if let Some(parent) = fdt.find_node("/reserved-memory")
        && node_is_enabled(&parent)
    {
        for child in parent.children().filter(node_is_enabled) {
            if let Some(regions) = child.reg() {
                for region in regions {
                    let size = region.size.ok_or(DeviceTreeError::MissingRegionSize)?;

                    ranges.push(PhysRange::from_start_size(
                        region.starting_address as usize,
                        size,
                    )?);
                }
            } else if child.property("size").is_some() {
                // Dynamic reservations must be placed before the general page
                // frame allocator is initialized. We do not support that yet.
                return Err(DeviceTreeError::DynamicReservedMemoryUnsupported);
            } else {
                return Err(DeviceTreeError::MalformedReservedMemory);
            }
        }
    }

    Ok(normalize_ranges(ranges))
}

/// Collect and page-align the MMIO windows used by the kernel.
fn collect_mmio_regions(fdt: &fdt::Fdt<'_>) -> Result<Vec<PhysRange>, DeviceTreeError> {
    let mut ranges = Vec::new();

    for node in fdt
        .all_nodes()
        .filter(node_is_enabled)
        .filter(node_is_kernel_mmio_device)
    {
        let regions = node.reg().ok_or(DeviceTreeError::MissingMmioReg)?;

        for region in regions {
            let size = region.size.ok_or(DeviceTreeError::MissingRegionSize)?;
            let range = PhysRange::from_start_size(region.starting_address as usize, size)?;
            ranges.push(range.covering_pages()?);
        }
    }

    if ranges.is_empty() {
        return Err(DeviceTreeError::NoMmioRegions);
    }

    Ok(normalize_ranges(ranges))
}

impl SystemInfo {
    fn new(fdt_addr: usize) -> Result<Self, DeviceTreeError> {
        let fdt = unsafe { fdt::Fdt::from_ptr(fdt_addr as *const u8) }
            .map_err(|_| DeviceTreeError::InvalidFdt)?;

        let memory_regions = collect_memory_regions(&fdt)?;
        let reserved_memory_regions = collect_reserved_memory_regions(&fdt)?;
        let mmio_regions = collect_mmio_regions(&fdt)?;
        let fdt_range = PhysRange::from_start_size(fdt_addr, fdt.total_size())?;
        let total_memory = memory_regions.iter().try_fold(0usize, |total, range| {
            total
                .checked_add(range.size())
                .ok_or(DeviceTreeError::AddressOverflow)
        })?;

        let mut block_device_addr = None;

        for node in fdt.all_nodes().filter(node_is_enabled) {
            if node_has_compatible(&node, "virtio,mmio")
                && let Some(mut reg) = node.reg()
                && let Some(region) = reg.next()
            {
                let addr = region.starting_address as usize;

                // Probe the VirtIO device ID to distinguish the block device
                // from the other VirtIO MMIO transports.
                //
                // TODO(mt): refer to docs here to avoid magic 0x008 value.
                let device_id = unsafe { core::ptr::read_volatile((addr + 0x008) as *const u32) };
                if device_id == 2 {
                    block_device_addr = Some(addr);
                }
            }
        }

        let cpu = fdt.cpus().next().ok_or(DeviceTreeError::MissingCpu)?;
        let isa = cpu
            .properties()
            .find(|property| property.name == "riscv,isa")
            .ok_or(DeviceTreeError::MissingCpuIsa)?;
        let isa =
            String::from_utf8(isa.value.to_vec()).map_err(|_| DeviceTreeError::InvalidCpuIsa)?;
        let (base_isa, _) = isa.split_once('_').ok_or(DeviceTreeError::InvalidCpuIsa)?;

        let system_info = Self {
            timer_frequency: cpu.timebase_frequency(),
            cpus: fdt.cpus().count(),
            cpu_isa: String::from(base_isa),
            memory_regions,
            reserved_memory_regions,
            mmio_regions,
            fdt_range,
            total_memory,
            block_device_addr: block_device_addr.ok_or(DeviceTreeError::MissingBlockDevice)?,
        };

        log::info!("memory regions: {:?}", system_info.memory_regions);
        log::info!(
            "reserved memory regions: {:?}",
            system_info.reserved_memory_regions
        );
        log::info!("MMIO regions: {:?}", system_info.mmio_regions);
        log::info!("FDT range: {:x?}", system_info.fdt_range);

        Ok(system_info)
    }
}

pub fn init(fdt_addr: usize) -> Result<(), DeviceTreeError> {
    if SYSINFO.get().is_some() {
        return Err(DeviceTreeError::AlreadyInitialized);
    }

    SYSINFO
        .try_call_once(|| SystemInfo::new(fdt_addr))
        .map(|_| ())?;

    log::info!("initialized");
    Ok(())
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

pub(crate) fn memory_regions() -> &'static [PhysRange] {
    &system_info().memory_regions
}

pub(crate) fn reserved_memory_regions() -> &'static [PhysRange] {
    &system_info().reserved_memory_regions
}

pub(crate) fn system_mmio_regions() -> &'static [PhysRange] {
    &system_info().mmio_regions
}

pub(crate) fn fdt_range() -> PhysRange {
    system_info().fdt_range
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

/// Return the page-aligned MMIO windows used by the kernel from an early-boot FDT.
pub fn mmio_regions(fdt_addr: usize) -> Result<Vec<PhysRange>, DeviceTreeError> {
    let fdt = unsafe { fdt::Fdt::from_ptr(fdt_addr as *const u8) }
        .map_err(|_| DeviceTreeError::InvalidFdt)?;

    collect_mmio_regions(&fdt)
}

/// Return all VirtIO MMIO devices from an FDT supplied during early boot.
pub fn virtio_mmio_devices(fdt_addr: usize) -> Vec<usize> {
    let fdt =
        unsafe { fdt::Fdt::from_ptr(fdt_addr as *const u8) }.expect("Could not read device tree");

    let mut devices = Vec::new();

    for node in fdt.all_nodes().filter(node_is_enabled) {
        if node_has_compatible(&node, "virtio,mmio")
            && let Some(mut reg) = node.reg()
            && let Some(region) = reg.next()
        {
            devices.push(region.starting_address as usize);
        }
    }

    devices
}
