extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
pub use virtual_memory::PhysRange;
use virtual_memory::{PhysRangeError, normalize_ranges};

static SYSINFO: spin::Once<SystemInfo> = spin::Once::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UartInfo {
    pub mmio_region: PhysRange,
    pub interrupt_id: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlicContext {
    pub hart_id: usize,
    pub context_index: usize,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PlicInfo {
    pub mmio_region: PhysRange,
    pub num_sources: usize,
    /// PLIC contexts which target a hart's supervisor external interrupt.
    ///
    /// Context register blocks use `context_index` when calculating their
    /// enable, threshold, and claim/complete addresses. On a single-hart QEMU
    /// `virt` machine hart 0 uses context 1; context 0 targets machine mode.
    pub supervisor_contexts: Vec<PlicContext>,
}

#[derive(Debug)]
struct SystemInfo {
    timer_frequency: usize,
    cpus: usize,
    cpu_isa: String,
    memory_regions: Vec<PhysRange>,
    reserved_memory_regions: Vec<PhysRange>,
    mmio_regions: Vec<PhysRange>,
    virtio_mmio_regions: Vec<PhysRange>,
    fdt_range: PhysRange,
    total_memory: usize,
    block_device_region: Option<PhysRange>,
    console_uart: UartInfo,
    plic: PlicInfo,
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
    MissingConsole,
    UnsupportedConsole,
    MissingConsoleInterrupt,
    MissingInterruptParent,
    UnsupportedInterruptParent,
    MissingPlicSourceCount,
    MissingPlicContexts,
    MalformedPlicContexts,
    InvalidConsoleInterrupt,
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
    node_has_compatible(node, "ns16550a")
        || node_has_compatible(node, "virtio,mmio")
        || node_is_plic(node)
}

fn node_is_plic(node: &fdt::node::FdtNode<'_, '_>) -> bool {
    node_has_compatible(node, "sifive,plic-1.0.0") || node_has_compatible(node, "riscv,plic0")
}

fn first_reg_region(node: &fdt::node::FdtNode<'_, '_>) -> Result<PhysRange, DeviceTreeError> {
    let region = node
        .reg()
        .ok_or(DeviceTreeError::MissingMmioReg)?
        .next()
        .ok_or(DeviceTreeError::MissingMmioReg)?;
    let size = region.size.ok_or(DeviceTreeError::MissingRegionSize)?;

    PhysRange::from_start_size(region.starting_address as usize, size).map_err(Into::into)
}

fn read_be_u32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?))
}

fn cpu_hart_id_for_interrupt_controller(fdt: &fdt::Fdt<'_>, phandle: u32) -> Option<usize> {
    let cpus = fdt.find_node("/cpus")?;

    for cpu in cpus.children().filter(node_is_enabled) {
        let owns_interrupt_controller = cpu.children().any(|child| {
            child.property("phandle").and_then(|value| value.as_usize()) == Some(phandle as usize)
        });
        if !owns_interrupt_controller {
            continue;
        }

        // A CPU node's `reg` address is its hart ID. CPU nodes conventionally
        // have zero size cells, so the returned region intentionally has no size.
        return cpu
            .reg()?
            .next()
            .map(|region| region.starting_address as usize);
    }

    None
}

fn collect_supervisor_contexts(
    fdt: &fdt::Fdt<'_>,
    plic: &fdt::node::FdtNode<'_, '_>,
) -> Result<Vec<PlicContext>, DeviceTreeError> {
    const SUPERVISOR_EXTERNAL_INTERRUPT: usize = 9;

    let mut remaining = plic
        .property("interrupts-extended")
        .ok_or(DeviceTreeError::MissingPlicContexts)?
        .value;
    let mut context_index = 0;
    let mut supervisor_contexts = Vec::new();

    while !remaining.is_empty() {
        let phandle = read_be_u32(remaining).ok_or(DeviceTreeError::MalformedPlicContexts)?;
        remaining = remaining
            .get(4..)
            .ok_or(DeviceTreeError::MalformedPlicContexts)?;

        let interrupt_controller = fdt
            .find_phandle(phandle)
            .ok_or(DeviceTreeError::MalformedPlicContexts)?;
        let interrupt_cells = interrupt_controller
            .interrupt_cells()
            .ok_or(DeviceTreeError::MalformedPlicContexts)?;
        let specifier_size = interrupt_cells
            .checked_mul(core::mem::size_of::<u32>())
            .ok_or(DeviceTreeError::MalformedPlicContexts)?;
        let specifier = remaining
            .get(..specifier_size)
            .ok_or(DeviceTreeError::MalformedPlicContexts)?;

        // RISC-V CPU interrupt controllers use one interrupt cell. The value 9
        // is the supervisor external interrupt input (`SEIP`). The tuple's
        // position is also the PLIC context number used by its MMIO layout.
        if interrupt_cells == 1
            && node_has_compatible(&interrupt_controller, "riscv,cpu-intc")
            && read_be_u32(specifier) == Some(SUPERVISOR_EXTERNAL_INTERRUPT as u32)
        {
            let hart_id = cpu_hart_id_for_interrupt_controller(fdt, phandle)
                .ok_or(DeviceTreeError::MalformedPlicContexts)?;
            supervisor_contexts.push(PlicContext {
                hart_id,
                context_index,
            });
        }

        remaining = &remaining[specifier_size..];
        context_index += 1;
    }

    if supervisor_contexts.is_empty() {
        return Err(DeviceTreeError::MissingPlicContexts);
    }

    Ok(supervisor_contexts)
}

fn collect_interrupt_devices(fdt: &fdt::Fdt<'_>) -> Result<(UartInfo, PlicInfo), DeviceTreeError> {
    let uart = fdt
        .chosen()
        .stdin()
        .ok_or(DeviceTreeError::MissingConsole)?;
    if !node_is_enabled(&uart) || !node_has_compatible(&uart, "ns16550a") {
        return Err(DeviceTreeError::UnsupportedConsole);
    }

    let plic = uart
        .interrupt_parent()
        .ok_or(DeviceTreeError::MissingInterruptParent)?;
    if !node_is_enabled(&plic) || !node_is_plic(&plic) {
        return Err(DeviceTreeError::UnsupportedInterruptParent);
    }

    let interrupt_id = uart
        .interrupts()
        .ok_or(DeviceTreeError::MissingConsoleInterrupt)?
        .next()
        .ok_or(DeviceTreeError::MissingConsoleInterrupt)?;
    let num_sources = plic
        .property("riscv,ndev")
        .and_then(|property| property.as_usize())
        .ok_or(DeviceTreeError::MissingPlicSourceCount)?;

    if interrupt_id == 0 || interrupt_id > num_sources {
        return Err(DeviceTreeError::InvalidConsoleInterrupt);
    }

    let uart_info = UartInfo {
        mmio_region: first_reg_region(&uart)?,
        interrupt_id,
    };
    let plic_info = PlicInfo {
        mmio_region: first_reg_region(&plic)?,
        num_sources,
        supervisor_contexts: collect_supervisor_contexts(fdt, &plic)?,
    };

    Ok((uart_info, plic_info))
}

/// Collect the exact register windows of enabled VirtIO MMIO transports.
///
/// Unlike [`collect_mmio_regions`], these ranges retain the device-tree resource boundaries. They
/// are therefore suitable for constructing a typed MMIO transport. Page-table construction uses
/// the page-covered, normalized ranges returned by `collect_mmio_regions` instead.
fn collect_virtio_mmio_regions(fdt: &fdt::Fdt<'_>) -> Result<Vec<PhysRange>, DeviceTreeError> {
    let mut ranges = Vec::new();

    for node in fdt
        .all_nodes()
        .filter(node_is_enabled)
        .filter(|node| node_has_compatible(node, "virtio,mmio"))
    {
        let regions = node.reg().ok_or(DeviceTreeError::MissingMmioReg)?;

        for region in regions {
            let size = region.size.ok_or(DeviceTreeError::MissingRegionSize)?;
            ranges.push(PhysRange::from_start_size(
                region.starting_address as usize,
                size,
            )?);
        }
    }

    Ok(ranges)
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
        let virtio_mmio_regions = collect_virtio_mmio_regions(&fdt)?;
        let fdt_range = PhysRange::from_start_size(fdt_addr, fdt.total_size())?;
        let total_memory = memory_regions.iter().try_fold(0usize, |total, range| {
            total
                .checked_add(range.size())
                .ok_or(DeviceTreeError::AddressOverflow)
        })?;
        let (console_uart, plic) = collect_interrupt_devices(&fdt)?;

        const VIRTIO_DEVICE_ID_OFFSET: usize = 0x008;
        const VIRTIO_BLOCK_DEVICE_ID: u32 = 2;

        let mut block_device_region = None;

        for region in virtio_mmio_regions.iter() {
            let device_id_addr = region.start() + VIRTIO_DEVICE_ID_OFFSET;
            if region.size() < VIRTIO_DEVICE_ID_OFFSET + core::mem::size_of::<u32>()
                || device_id_addr.is_multiple_of(32)
            {
                log::warn!(
                    "VirtIO MMIO region cannot safely expose its device ID register: {region:?}"
                );
                continue;
            }

            // SAFETY: `region` is the exact `reg` resource of an enabled node whose compatible
            // list contains `virtio,mmio`. The checks above establish that its device-ID register
            // is contained and aligned, the early boot address space makes the physical MMIO
            // address directly accessible, and no VirtIO transport has been constructed before
            // device-tree initialization.
            let device_id = unsafe { core::ptr::read_volatile(device_id_addr as *const u32) };
            if device_id == VIRTIO_BLOCK_DEVICE_ID {
                block_device_region = Some(*region);
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
            virtio_mmio_regions,
            fdt_range,
            total_memory,
            block_device_region,
            console_uart,
            plic,
        };

        log::info!("memory regions: {:?}", system_info.memory_regions);
        log::info!(
            "reserved memory regions: {:?}",
            system_info.reserved_memory_regions
        );
        log::info!("MMIO regions: {:?}", system_info.mmio_regions);
        log::info!("console UART: {:?}", system_info.console_uart);
        log::info!("PLIC: {:?}", system_info.plic);
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

/// The timer frequency is describing the number of ticks that the hardware-timer advances per second.
pub fn timer_frequency() -> usize {
    system_info().timer_frequency
}

pub(crate) fn cpus() -> usize {
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

pub fn console_uart() -> &'static UartInfo {
    &system_info().console_uart
}

pub fn plic() -> &'static PlicInfo {
    &system_info().plic
}

/// Exact `reg` resources of enabled `virtio,mmio` device-tree nodes.
pub(crate) fn system_virtio_mmio_regions() -> &'static [PhysRange] {
    &system_info().virtio_mmio_regions
}

pub(crate) fn fdt_range() -> PhysRange {
    system_info().fdt_range
}

pub(crate) fn total_memory() -> usize {
    system_info().total_memory
}

pub(crate) fn cpu_isa() -> String {
    system_info().cpu_isa.clone()
}

pub(crate) fn block_device_region() -> PhysRange {
    system_info()
        .block_device_region
        .expect("no enabled VirtIO block device was found in the device tree")
}

/// Return the page-aligned MMIO windows used by the kernel from an early-boot FDT.
pub fn mmio_regions(fdt_addr: usize) -> Result<Vec<PhysRange>, DeviceTreeError> {
    let fdt = unsafe { fdt::Fdt::from_ptr(fdt_addr as *const u8) }
        .map_err(|_| DeviceTreeError::InvalidFdt)?;

    collect_mmio_regions(&fdt)
}

/// Return the exact register windows of all enabled VirtIO MMIO nodes from an early-boot FDT.
pub fn virtio_mmio_regions(fdt_addr: usize) -> Result<Vec<PhysRange>, DeviceTreeError> {
    let fdt = unsafe { fdt::Fdt::from_ptr(fdt_addr as *const u8) }
        .map_err(|_| DeviceTreeError::InvalidFdt)?;

    collect_virtio_mmio_regions(&fdt)
}

/// Return the chosen NS16550A console and its PLIC interrupt parent from an
/// early-boot FDT.
pub fn interrupt_devices(fdt_addr: usize) -> Result<(UartInfo, PlicInfo), DeviceTreeError> {
    let fdt = unsafe { fdt::Fdt::from_ptr(fdt_addr as *const u8) }
        .map_err(|_| DeviceTreeError::InvalidFdt)?;

    collect_interrupt_devices(&fdt)
}

/// Return all VirtIO MMIO devices from an FDT supplied during early boot.
pub fn virtio_mmio_devices(fdt_addr: usize) -> Vec<usize> {
    virtio_mmio_regions(fdt_addr)
        .expect("Could not read VirtIO MMIO resources from device tree")
        .into_iter()
        .map(PhysRange::start)
        .collect()
}
