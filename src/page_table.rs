use crate::riscv::{self, Asid, Satp, SatpMode};
use core::arch::asm;

use virtual_memory::{PAGE_SIZE, PageTable, PhysAddr, PhysRange, VirtAddr, pte_flags};

use crate::{device_tree, kernel_layout::KernelLayout, page_frame_allocator};

static mut KERNEL_PAGE_TABLE: PageTable = PageTable::new();

/// Identity-maps a physical page into the kernel page table (virtual == physical).
pub(crate) fn new_identity_map(phys: PhysAddr) {
    let flags = pte_flags::READ | pte_flags::WRITE;
    let alloc = || page_frame_allocator::alloc_frame().unwrap();
    unsafe {
        (*&raw mut KERNEL_PAGE_TABLE).map(VirtAddr(phys), phys, flags, alloc);
        asm!("sfence.vma");
    }
}

/// This initializes the kernel page table, identity mapping all kernel pages and pages used for
/// MMIO. We also identity-map all allocator-managed RAM pages so that the kernel can reach them.
///
/// NOTE: For now it's just mapping all kernel pages as READ | WRITE | EXECUTE.
///
/// docs: https://www.scs.stanford.edu/~zyedidia/docs/riscv/riscv-privileged.pdf Section 4.1.11
pub fn init(kernel_layout: KernelLayout) {
    let kernel_start = kernel_layout.kernel_start;
    let kernel_end = kernel_layout.kernel_end;

    let upper_half_offset = 0xFFFF_FFFF_0000_0000_usize;

    let alloc = || page_frame_allocator::alloc_frame().unwrap();

    for page in (kernel_start..kernel_end).step_by(PAGE_SIZE) {
        let flags = pte_flags::READ | pte_flags::WRITE | pte_flags::EXECUTE;
        unsafe {
            // NOTE: Funky syntax here because rust doesn't allow taking a mutable reference to a
            // static. This is a workaround like `addr_of_mut!()` which is getting deprecated.
            (*&raw mut KERNEL_PAGE_TABLE).map(VirtAddr(page), page, flags, alloc);
            (*&raw mut KERNEL_PAGE_TABLE).map(
                VirtAddr(upper_half_offset + page),
                page,
                flags,
                alloc,
            );
        }
    }

    let id_map_region = |range: PhysRange, flags| {
        for page in range.range().step_by(PAGE_SIZE) {
            unsafe {
                (*&raw mut KERNEL_PAGE_TABLE).map(VirtAddr(page), page, flags, alloc);
            }
        }
    };

    // Identity-map every frame which the page frame allocator may return.
    // Deliberately do not map holes or firmware `no-map` reservations.
    for range in page_frame_allocator::managed_ranges() {
        id_map_region(range, pte_flags::READ | pte_flags::WRITE);
    }

    // Keep the live FDT readable after paging is enabled. The allocator has
    // excluded every page touched by this range, so these mappings cannot
    // alias an allocated frame.
    let fdt_range = device_tree::fdt_range()
        .covering_pages()
        .expect("FDT range overflow");
    id_map_region(fdt_range, pte_flags::READ);

    for range in device_tree::system_mmio_regions() {
        id_map_region(*range, pte_flags::READ | pte_flags::WRITE);
    }

    let kernel_page_table_addr = &raw const KERNEL_PAGE_TABLE as usize;
    let satp = Satp::new(SatpMode::Sv39, Asid::KERNEL, kernel_page_table_addr);

    riscv::write_satp_and_flush_tlb(satp);

    // We added mappings of the kernel pages to the upper half of the address
    // space (0xFFFF_FFFF_0000_0000). This is an optimization for the time when
    // we implement processes. The idea here is that we frequently need to
    // switch to the kernel code for example during interrupt handling. This
    // requires us to have access to the memory containing the kernel code. To
    // avoid switching the page tables and flushing the TLB on each interrupt,
    // we map the kernel code into every user-process page table. This way the
    // kernel code is already accessible without needing to change the page
    // table. The user-process can't access the kernel code as the
    // `PageTableEntry`s don't have the `USER` flag set. During an interrupt,
    // the CPU switches into Supervisor mode which means we can access the
    // pages.

    // Jump to the higher address space
    unsafe {
        asm!(
            "la t0, 1f", // take address of label `1:` at the end of this block
            "li t1, 0xFFFFFFFF00000000",
            "add t0, t0, t1",   // add the offset to it
            "jalr zero, t0, 0", // jump there
            "1:"
        )
    }

    log::info!("Kernel page table initialized");
}
