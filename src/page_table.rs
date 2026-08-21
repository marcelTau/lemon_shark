use core::arch::asm;

use virtual_memory::{PAGE_SIZE, PageTable, PhysAddr, VirtAddr, pte_flags};

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
/// NOTE: For now it's just mapping all kernel pages as READ | WRITE | EXECTUE.
///
/// docs: https://www.scs.stanford.edu/~zyedidia/docs/riscv/riscv-privileged.pdf Section 4.1.11
pub fn init(kernel_layout: KernelLayout) {
    // defined in `linker.ld`
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

    let id_map_region = |range: core::ops::Range<usize>, flags| {
        for page in range.step_by(PAGE_SIZE) {
            unsafe {
                (*&raw mut KERNEL_PAGE_TABLE).map(VirtAddr(page), page, flags, alloc);
            }
        }
    };

    // Identity-map every frame which the page frame allocator may return.
    // Deliberately do not map holes or firmware `no-map` reservations.
    for range in page_frame_allocator::managed_ranges() {
        id_map_region(
            range.start()..range.end(),
            pte_flags::READ | pte_flags::WRITE,
        );
    }

    // Keep the live FDT readable after paging is enabled. The allocator has
    // excluded every page touched by this range, so these mappings cannot
    // alias an allocated frame.
    let fdt_range = device_tree::fdt_range();
    let fdt_start = fdt_range.start() & !(PAGE_SIZE - 1);
    let fdt_end = fdt_range
        .end()
        .checked_add(PAGE_SIZE - 1)
        .expect("FDT range overflow")
        & !(PAGE_SIZE - 1);

    id_map_region(fdt_start..fdt_end, pte_flags::READ);

    // Those include the UART & virtio MMIO ranges
    let mmio_start = 0x10000000;
    let mmio_end = 0x10009000;

    id_map_region(mmio_start..mmio_end, pte_flags::READ | pte_flags::WRITE);

    // TODO(mt): This becomes important when implementing processes. The ASID is used in the TLB to
    // avoid flushing the TLB on context switches. Each process has it's own ASID (limited to
    // 16 bytes) on risc-v. The TLB then ignores translations for other ASID's and by doing that
    // avoids flushing it on every context switch.
    let asid = 0;

    let kernel_page_table_addr = &raw const KERNEL_PAGE_TABLE as usize;

    // mode=0x8=Sv39
    let satp = (0x8_usize << 60) | (asid << 44) | (kernel_page_table_addr >> 12);

    unsafe {
        asm!(
           "csrw satp, {satp}",
           "sfence.vma",
           satp = in(reg) satp
        );
    }

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
