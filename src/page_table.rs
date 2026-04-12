use core::arch::asm;

use virtual_memory::{pte_flags, PageTable, VirtAddr, PAGE_SIZE};

use crate::page_frame_allocator;

static mut KERNEL_PAGE_TABLE: PageTable = PageTable::new();

/// This initializes the kernel page table, identity mapping all kernel pages and pages used for
/// MMIO.
///
/// NOTE: For now it's just mapping all kernel pages as READ | WRITE | EXECTUE.
///
/// docs: https://www.scs.stanford.edu/~zyedidia/docs/riscv/riscv-privileged.pdf Section 4.1.11
pub fn init() {
    unsafe extern "C" {
        static _kernel_end: u8;
    }

    // defined in `linker.ld`
    let kernel_start = 0x80200000;
    let kernel_end = unsafe { &_kernel_end as *const u8 as usize };

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

    // Those include the UART & virtio MMIO ranges
    let mmio_start = 0x10000000;
    let mmio_end = 0x10008000;

    for page in (mmio_start..=mmio_end).step_by(PAGE_SIZE) {
        let flags = pte_flags::READ | pte_flags::WRITE;
        unsafe {
            (*&raw mut KERNEL_PAGE_TABLE).map(VirtAddr(page), page, flags, alloc);
        }
    }

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

    // map the kernel code to the higher half of the virtual address space

    log::info!("Kernel page table initialized");
}
