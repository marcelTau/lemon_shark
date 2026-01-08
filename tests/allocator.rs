#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(lemon_shark::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::arch::global_asm;

use lemon_shark::allocator2::FreeListAllocator;
use lemon_shark::{interrupts, logln, trap_handler};

use core::arch::asm;

global_asm!(
    ".section .text.boot",
    ".global _boot",
    "_boot:",
    "   la sp, _stack_top",
    "   call _start",
);

/// Custom `_start` function allows each test suite to explicitly initialize the things
/// it needs.
#[unsafe(no_mangle)]
pub extern "C" fn _start(_: usize, _: usize) -> ! {
    unsafe extern "C" {
        static _trap_stack_top: u8;
        static _heap_top: u8;
    }

    // Set the `sscratch` register to a 'known good' stack that the `trap_handler` can use.
    unsafe {
        let trap_stack = &_trap_stack_top as *const u8 as usize;
        asm!("csrw sscratch, {}", in(reg) trap_stack);
    }

    trap_handler::init();
    interrupts::init();

    test_main();
    loop {}
}

#[test_case]
fn allocator_init() {
    let mut alloc = FreeListAllocator { head: None };
    unsafe { alloc.init() };
}

fn make_alloc() -> FreeListAllocator {
    let mut alloc = FreeListAllocator { head: None };
    unsafe { alloc.init() };
    alloc
}

#[test_case]
fn allocator_can_allocate() {
    let mut alloc = make_alloc();

    let initial_free_bytes = alloc.free();

    let layout = core::alloc::Layout::new::<[usize; 100]>();

    let align = layout.align();

    let ptr = alloc.alloc(layout);
    let mem = ptr as *mut usize;

    assert!(!mem.is_null());
    assert!((mem as usize).is_multiple_of(align));

    unsafe { core::ptr::write_bytes(mem, 1u8, 100) };

    // memory is writable
    for i in 0..100 {
        unsafe { *mem.add(i) = i };
    }

    // memory is readable
    for i in 0..100 {
        unsafe {
            assert_eq!(*mem.add(i), i);
        }
    }

    assert_eq!(alloc.free_blocks(), 1);

    alloc.dealloc(ptr, layout);

    assert_eq!(alloc.free(), initial_free_bytes);
}

#[test_case]
fn fragmentation_and_merge() {
    let mut alloc = make_alloc();

    // layout to allocate 800 bytes
    let layout = core::alloc::Layout::new::<[usize; 100]>();
    let initial_free = alloc.free();

    assert_eq!(alloc.free_blocks(), 1);

    let left = alloc.alloc(layout);
    let mid = alloc.alloc(layout);
    let right = alloc.alloc(layout);

    assert_eq!(alloc.free_blocks(), 1);

    // At this point, the allocator should have created 3 blocks of useable memory
    // all before the main block of free memory.

    // This should free the mid block and thus create 2 separate free blocks that
    // can't yet be merged because of `right`.
    alloc.dealloc(mid, layout);

    assert_eq!(alloc.free_blocks(), 2);

    // Deallocating `left` should cause `left` and `mid` to be merged into a single
    // `FreeBlock` and thus the number of free blocks should not increase.
    alloc.dealloc(left, layout);
    assert_eq!(alloc.free_blocks(), 2);
    // assert_eq!(alloc.free(), initial_free - layout.size());

    // Deallocating the `right` now should consolidate all the blocks into a single
    // block again and restore the initial state of the allocator.
    alloc.dealloc(right, layout);
    assert_eq!(alloc.free_blocks(), 1);
    assert_eq!(alloc.free(), initial_free);

    alloc.dump_state();
}

#[test_case]
fn fragmentation_and_merging2() {
    let mut alloc = make_alloc();

    let init = alloc.free();

    let layout_16 = core::alloc::Layout::new::<[usize; 2]>();
    let layout_64 = core::alloc::Layout::new::<[usize; 8]>();

    let a = alloc.alloc(layout_16);
    let b = alloc.alloc(layout_64);

    alloc.dealloc(b, layout_64);
    alloc.dealloc(a, layout_16);

    let c = alloc.alloc(layout_16);
    let d = alloc.alloc(layout_16);

    alloc.dealloc(c, layout_16);

    let e = alloc.alloc(layout_64);
    alloc.dealloc(e, layout_64);

    alloc.dealloc(d, layout_16);

    alloc.dump_state();

    assert_eq!(alloc.free_blocks(), 1);
    assert_eq!(init, alloc.free());
}

// ============================================================================
// Alignment Tests
// ============================================================================

#[test_case]
fn test_u8_alignment() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<u8>();

    let ptr = alloc.alloc(layout);
    assert!(!ptr.is_null());
    assert_eq!(
        ptr as usize % layout.align(),
        0,
        "u8 allocation not aligned"
    );

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_u16_alignment() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<u16>();

    let ptr = alloc.alloc(layout);
    assert!(!ptr.is_null());
    assert_eq!(
        ptr as usize % layout.align(),
        0,
        "u16 allocation not aligned to 2 bytes"
    );

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_u32_alignment() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<u32>();

    let ptr = alloc.alloc(layout);
    assert!(!ptr.is_null());
    assert_eq!(
        ptr as usize % layout.align(),
        0,
        "u32 allocation not aligned to 4 bytes"
    );

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_u64_alignment() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<u64>();

    let ptr = alloc.alloc(layout);
    assert!(!ptr.is_null());
    assert_eq!(
        ptr as usize % layout.align(),
        0,
        "u64 allocation not aligned to 8 bytes"
    );

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_u128_alignment() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<u128>();

    let ptr = alloc.alloc(layout);
    assert!(!ptr.is_null());
    assert_eq!(
        ptr as usize % layout.align(),
        0,
        "u128 allocation not aligned to 16 bytes"
    );

    alloc.dealloc(ptr, layout);
}

// ============================================================================
// Mixed Alignment Tests - Critical for finding alignment bugs
// ============================================================================

#[test_case]
fn test_mixed_alignments_sequence() {
    let mut alloc = make_alloc();

    // Allocate with different alignments in sequence
    let layout_u8 = core::alloc::Layout::new::<u8>();
    let layout_u64 = core::alloc::Layout::new::<u64>();
    let layout_u128 = core::alloc::Layout::new::<u128>();

    let p1 = alloc.alloc(layout_u8);
    let p2 = alloc.alloc(layout_u64);
    let p3 = alloc.alloc(layout_u128);

    // Verify all alignments
    assert_eq!(p1 as usize % layout_u8.align(), 0);
    assert_eq!(
        p2 as usize % layout_u64.align(),
        0,
        "u64 not aligned after u8"
    );
    assert_eq!(
        p3 as usize % layout_u128.align(),
        0,
        "u128 not aligned after u64"
    );

    alloc.dealloc(p1, layout_u8);
    alloc.dealloc(p2, layout_u64);
    alloc.dealloc(p3, layout_u128);
}

#[test_case]
fn test_alignment_after_dealloc_and_realloc() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    // Allocate a small aligned block
    let layout_small = core::alloc::Layout::new::<[u8; 7]>();
    let p1 = alloc.alloc(layout_small);
    alloc.dealloc(p1, layout_small);

    // Now allocate a block with strict alignment
    let layout_aligned = core::alloc::Layout::new::<u128>();
    let p2 = alloc.alloc(layout_aligned);

    assert_eq!(
        p2 as usize % 16,
        0,
        "u128 not 16-byte aligned after small dealloc"
    );

    alloc.dealloc(p2, layout_aligned);
    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_interleaved_alloc_different_alignments() {
    let mut alloc = make_alloc();

    let layout_u8 = core::alloc::Layout::new::<[u8; 3]>();
    let layout_u32 = core::alloc::Layout::new::<[u32; 5]>();
    let layout_u64 = core::alloc::Layout::new::<[u64; 7]>();

    let a = alloc.alloc(layout_u8);
    let b = alloc.alloc(layout_u32);
    let c = alloc.alloc(layout_u8);
    let d = alloc.alloc(layout_u64);
    let e = alloc.alloc(layout_u32);

    // Verify alignments
    assert_eq!(b as usize % 4, 0, "u32 array not 4-byte aligned");
    assert_eq!(d as usize % 8, 0, "u64 array not 8-byte aligned");
    assert_eq!(e as usize % 4, 0, "second u32 array not 4-byte aligned");

    // Deallocate in different order
    alloc.dealloc(b, layout_u32);
    alloc.dealloc(d, layout_u64);
    alloc.dealloc(a, layout_u8);
    alloc.dealloc(e, layout_u32);
    alloc.dealloc(c, layout_u8);
}

// ============================================================================
// Array Tests with Different Sizes and Alignments
// ============================================================================

#[test_case]
fn test_large_u8_array() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<[u8; 1024]>();

    let ptr = alloc.alloc(layout);
    assert!(!ptr.is_null());

    // Write and verify
    unsafe {
        for i in 0..1024 {
            *ptr.add(i) = (i & 0xFF) as u8;
        }
        for i in 0..1024 {
            assert_eq!(*ptr.add(i), (i & 0xFF) as u8);
        }
    }

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_u32_array_alignment() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<[u32; 256]>();

    let ptr = alloc.alloc(layout) as *mut u32;
    assert_eq!(ptr as usize % 4, 0, "u32 array not 4-byte aligned");

    // Write and verify
    unsafe {
        for i in 0..256 {
            *ptr.add(i) = i as u32 * 123456;
        }
        for i in 0..256 {
            assert_eq!(*ptr.add(i), i as u32 * 123456);
        }
    }

    alloc.dealloc(ptr as *mut u8, layout);
}

#[test_case]
fn test_u64_array_alignment() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<[u64; 128]>();

    let ptr = alloc.alloc(layout) as *mut u64;
    assert_eq!(ptr as usize % 8, 0, "u64 array not 8-byte aligned");

    // Write and verify
    unsafe {
        for i in 0..128 {
            *ptr.add(i) = i as u64 * 0xDEADBEEF;
        }
        for i in 0..128 {
            assert_eq!(*ptr.add(i), i as u64 * 0xDEADBEEF);
        }
    }

    alloc.dealloc(ptr as *mut u8, layout);
}

// ============================================================================
// Fragmentation Stress Tests
// ============================================================================

#[test_case]
fn test_many_small_allocations() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let layout = core::alloc::Layout::new::<u64>();
    let mut ptrs = [core::ptr::null_mut(); 50];

    // Allocate 50 u64s
    for i in 0..50 {
        ptrs[i] = alloc.alloc(layout);
        assert!(!ptrs[i].is_null());
        assert_eq!(ptrs[i] as usize % 8, 0, "allocation {} not aligned", i);
    }

    // Deallocate all
    for i in 0..50 {
        alloc.dealloc(ptrs[i], layout);
    }

    assert_eq!(alloc.free(), init_free, "memory leak detected");
}

#[test_case]
fn test_alternating_sizes() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let small = core::alloc::Layout::new::<u32>();
    let large = core::alloc::Layout::new::<[u64; 32]>();

    let mut small_ptrs = [core::ptr::null_mut(); 10];
    let mut large_ptrs = [core::ptr::null_mut(); 10];

    // Alternating allocation
    for i in 0..10 {
        small_ptrs[i] = alloc.alloc(small);
        large_ptrs[i] = alloc.alloc(large);
    }

    // Deallocate small first
    for i in 0..10 {
        alloc.dealloc(small_ptrs[i], small);
    }

    // Deallocate large
    for i in 0..10 {
        alloc.dealloc(large_ptrs[i], large);
    }

    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_reverse_deallocation_order() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let layout = core::alloc::Layout::new::<[usize; 16]>();
    let mut ptrs = [core::ptr::null_mut(); 20];

    for i in 0..20 {
        ptrs[i] = alloc.alloc(layout);
    }

    // Deallocate in reverse order
    for i in (0..20).rev() {
        alloc.dealloc(ptrs[i], layout);
    }

    assert_eq!(alloc.free(), init_free);
    assert_eq!(alloc.free_blocks(), 1);
}

// ============================================================================
// Edge Cases and Boundary Tests
// ============================================================================

#[test_case]
fn test_single_byte_allocations() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<u8>();

    let p1 = alloc.alloc(layout);
    let p2 = alloc.alloc(layout);
    let p3 = alloc.alloc(layout);

    unsafe {
        *p1 = 42;
        *p2 = 123;
        *p3 = 255;

        assert_eq!(*p1, 42);
        assert_eq!(*p2, 123);
        assert_eq!(*p3, 255);
    }

    alloc.dealloc(p2, layout);
    alloc.dealloc(p1, layout);
    alloc.dealloc(p3, layout);
}

#[test_case]
fn test_odd_sized_allocations() {
    let mut alloc = make_alloc();

    let layout_17 = core::alloc::Layout::from_size_align(17, 1).unwrap();
    let layout_33 = core::alloc::Layout::from_size_align(33, 1).unwrap();
    let layout_65 = core::alloc::Layout::from_size_align(65, 1).unwrap();

    let p1 = alloc.alloc(layout_17);
    let p2 = alloc.alloc(layout_33);
    let p3 = alloc.alloc(layout_65);

    assert!(!p1.is_null());
    assert!(!p2.is_null());
    assert!(!p3.is_null());

    alloc.dealloc(p1, layout_17);
    alloc.dealloc(p2, layout_33);
    alloc.dealloc(p3, layout_65);
}

#[test_case]
fn test_max_alignment_request() {
    let mut alloc = make_alloc();

    // Request large alignment (64 bytes)
    let layout = core::alloc::Layout::from_size_align(128, 64).unwrap();
    let ptr = alloc.alloc(layout);

    assert!(!ptr.is_null());
    assert_eq!(ptr as usize % 64, 0, "allocation not 64-byte aligned");

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_stress_fragmentation() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let l1 = core::alloc::Layout::new::<[u8; 13]>();
    let l2 = core::alloc::Layout::new::<[u16; 7]>();
    let l3 = core::alloc::Layout::new::<[u32; 11]>();
    let l4 = core::alloc::Layout::new::<[u64; 5]>();

    // Create fragmentation
    let a1 = alloc.alloc(l1);
    let b1 = alloc.alloc(l2);
    let c1 = alloc.alloc(l3);
    let d1 = alloc.alloc(l4);
    let a2 = alloc.alloc(l1);
    let b2 = alloc.alloc(l2);
    let c2 = alloc.alloc(l3);

    // Free in scattered order
    alloc.dealloc(b1, l2);
    alloc.dealloc(d1, l4);
    alloc.dealloc(a1, l1);
    alloc.dealloc(c2, l3);
    alloc.dealloc(b2, l2);
    alloc.dealloc(a2, l1);
    alloc.dealloc(c1, l3);

    assert_eq!(alloc.free(), init_free);
}

// ============================================================================
// Alignment Verification After Complex Operations
// ============================================================================

#[test_case]
fn test_alignment_preservation() {
    let mut alloc = make_alloc();

    // Do some allocations and deallocations
    let l8 = core::alloc::Layout::new::<[u8; 100]>();
    let l64 = core::alloc::Layout::new::<[u64; 50]>();

    let tmp = alloc.alloc(l8);
    alloc.dealloc(tmp, l8);

    // Now allocate with strict alignment requirement
    let p = alloc.alloc(l64);
    assert_eq!(
        p as usize % 8,
        0,
        "u64 array lost alignment after operations"
    );

    unsafe {
        let arr = p as *mut u64;
        for i in 0..50 {
            *arr.add(i) = i as u64;
        }
        for i in 0..50 {
            assert_eq!(*arr.add(i), i as u64);
        }
    }

    alloc.dealloc(p, l64);
}

#[test_case]
fn test_multiple_alignment_boundaries() {
    let mut alloc = make_alloc();

    // Test alignments: 1, 2, 4, 8, 16
    let layouts = [
        core::alloc::Layout::from_size_align(100, 1).unwrap(),
        core::alloc::Layout::from_size_align(100, 2).unwrap(),
        core::alloc::Layout::from_size_align(100, 4).unwrap(),
        core::alloc::Layout::from_size_align(100, 8).unwrap(),
        core::alloc::Layout::from_size_align(100, 16).unwrap(),
    ];

    for layout in &layouts {
        let ptr = alloc.alloc(*layout);
        assert_eq!(
            ptr as usize % layout.align(),
            0,
            "allocation not aligned to {} bytes",
            layout.align()
        );
        alloc.dealloc(ptr, *layout);
    }
}

#[test_case]
fn test_realistic_kernel_allocations() {
    let mut alloc = make_alloc();

    // Simulate typical kernel allocations
    let page_table_layout = core::alloc::Layout::from_size_align(4096, 4096).unwrap();
    let task_struct_layout = core::alloc::Layout::from_size_align(256, 8).unwrap();
    let buffer_layout = core::alloc::Layout::from_size_align(1024, 1).unwrap();

    let page_table = alloc.alloc(page_table_layout);

    assert_eq!(page_table as usize % 4096, 0, "page table not page-aligned");

    let task1 = alloc.alloc(task_struct_layout);
    let task2 = alloc.alloc(task_struct_layout);
    assert_eq!(task1 as usize % 8, 0);
    assert_eq!(task2 as usize % 8, 0);

    let buf = alloc.alloc(buffer_layout);
    assert!(!buf.is_null());

    alloc.dealloc(task1, task_struct_layout);
    alloc.dealloc(page_table, page_table_layout);
    alloc.dealloc(buf, buffer_layout);
    alloc.dealloc(task2, task_struct_layout);
}

// ============================================================================
// RISC-V Specific Alignment Tests
// ============================================================================

#[test_case]
fn test_riscv_atomic_u32_alignment() {
    let mut alloc = make_alloc();
    // AtomicU32 requires 4-byte alignment for lr.w/sc.w instructions
    let layout = core::alloc::Layout::new::<core::sync::atomic::AtomicU32>();

    let ptr = alloc.alloc(layout);
    assert_eq!(
        ptr as usize % 4,
        0,
        "AtomicU32 not 4-byte aligned for RISC-V lr.w/sc.w"
    );

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_riscv_atomic_u64_alignment() {
    let mut alloc = make_alloc();
    // AtomicU64 requires 8-byte alignment for lr.d/sc.d instructions
    let layout = core::alloc::Layout::new::<core::sync::atomic::AtomicU64>();

    let ptr = alloc.alloc(layout);
    assert_eq!(
        ptr as usize % 8,
        0,
        "AtomicU64 not 8-byte aligned for RISC-V lr.d/sc.d"
    );

    // Verify we can actually use it
    unsafe {
        let atomic = &*(ptr as *const core::sync::atomic::AtomicU64);
        atomic.store(0xDEADBEEFCAFEBABE, core::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            atomic.load(core::sync::atomic::Ordering::SeqCst),
            0xDEADBEEFCAFEBABE
        );
    }

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_riscv_cache_line_alignment() {
    let mut alloc = make_alloc();
    // Cache lines are typically 64 bytes
    let layout = core::alloc::Layout::from_size_align(64, 64).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 64, 0, "Cache line not 64-byte aligned");

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_riscv_page_alignment_4k() {
    let mut alloc = make_alloc();
    // Standard 4KB page alignment for RISC-V Sv39/Sv48
    let layout = core::alloc::Layout::from_size_align(4096, 4096).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 4096, 0, "Page not 4KB aligned");

    alloc.dealloc(ptr, layout);
}

// TODO(mt): we don't have that much memory right now.
// #[test_case]
// fn test_riscv_superpage_alignment_2m() {
//     let mut alloc = make_alloc();
//     // 2MB superpage alignment (if heap is large enough)
//     let layout = core::alloc::Layout::from_size_align(128, 2 * 1024 * 1024).unwrap();
//
//     let ptr = alloc.alloc(layout);
//     assert_eq!(ptr as usize % (2 * 1024 * 1024), 0, "Superpage not 2MB aligned");
//
//     alloc.dealloc(ptr, layout);
// }

#[test_case]
fn test_riscv_double_word_loads() {
    let mut alloc = make_alloc();
    // ld instruction requires 8-byte alignment
    let layout = core::alloc::Layout::new::<[u64; 10]>();

    let ptr = alloc.alloc(layout) as *mut u64;
    assert_eq!(
        ptr as usize % 8,
        0,
        "u64 array not aligned for ld instruction"
    );

    // Write and read using potential ld/sd instructions
    unsafe {
        for i in 0..10 {
            *ptr.add(i) = 0xFEDCBA9876543210u64.wrapping_add(i as u64);
        }
        for i in 0..10 {
            assert_eq!(*ptr.add(i), 0xFEDCBA9876543210u64.wrapping_add(i as u64));
        }
    }

    alloc.dealloc(ptr as *mut u8, layout);
}

#[test_case]
fn test_riscv_word_loads() {
    let mut alloc = make_alloc();
    // lw instruction requires 4-byte alignment
    let layout = core::alloc::Layout::new::<[u32; 20]>();

    let ptr = alloc.alloc(layout) as *mut u32;
    assert_eq!(
        ptr as usize % 4,
        0,
        "u32 array not aligned for lw instruction"
    );

    unsafe {
        for i in 0..20 {
            *ptr.add(i) = 0xABCD1234u32.wrapping_add(i as u32);
        }
        for i in 0..20 {
            assert_eq!(*ptr.add(i), 0xABCD1234u32.wrapping_add(i as u32));
        }
    }

    alloc.dealloc(ptr as *mut u8, layout);
}

#[test_case]
fn test_riscv_halfword_loads() {
    let mut alloc = make_alloc();
    // lh instruction requires 2-byte alignment
    let layout = core::alloc::Layout::new::<[u16; 30]>();

    let ptr = alloc.alloc(layout) as *mut u16;
    assert_eq!(
        ptr as usize % 2,
        0,
        "u16 array not aligned for lh instruction"
    );

    unsafe {
        for i in 0..30 {
            *ptr.add(i) = 0xABCDu16.wrapping_add(i as u16);
        }
        for i in 0..30 {
            assert_eq!(*ptr.add(i), 0xABCDu16.wrapping_add(i as u16));
        }
    }

    alloc.dealloc(ptr as *mut u8, layout);
}

// ============================================================================
// Block Splitting Test Cases - All Four Scenarios
// ============================================================================

#[test_case]
fn test_split_neither_left_nor_right() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    // Allocate something that uses the entire first block (or close to it)
    // This triggers (false, false) - no splitting
    let layout = core::alloc::Layout::from_size_align(32, 8).unwrap();
    let p1 = alloc.alloc(layout);

    assert!(!p1.is_null());
    assert_eq!(p1 as usize % 8, 0);

    alloc.dealloc(p1, layout);
    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_split_left_only() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    // First, fragment the heap
    let l1 = core::alloc::Layout::from_size_align(100, 8).unwrap();
    let l2 = core::alloc::Layout::from_size_align(200, 64).unwrap();

    let p1 = alloc.alloc(l1);
    alloc.dealloc(p1, l1);

    // Now allocate with high alignment - should split left only
    let p2 = alloc.alloc(l2);
    assert_eq!(p2 as usize % 64, 0);

    alloc.dealloc(p2, l2);
    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_split_right_only() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    // Allocate small size with low alignment, then free
    // This creates a scenario where we split right but not left
    let l1 = core::alloc::Layout::from_size_align(50, 8).unwrap();
    let p1 = alloc.alloc(l1);

    // Should leave a right block
    assert!(!p1.is_null());

    alloc.dealloc(p1, l1);
    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_split_both_left_and_right() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    // Create fragmentation first
    let l_small = core::alloc::Layout::from_size_align(32, 8).unwrap();
    let l_aligned = core::alloc::Layout::from_size_align(64, 128).unwrap();

    let p1 = alloc.alloc(l_small);
    alloc.dealloc(p1, l_small);

    // This should create both left and right splits due to high alignment
    let p2 = alloc.alloc(l_aligned);
    assert_eq!(p2 as usize % 128, 0);

    alloc.dealloc(p2, l_aligned);
    assert_eq!(alloc.free(), init_free);
}

// ============================================================================
// Fragmentation and Coalescing Tests
// ============================================================================

#[test_case]
fn test_fragmentation_max() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let layout = core::alloc::Layout::new::<u64>();
    let mut ptrs = [core::ptr::null_mut(); 100];

    // Allocate 100 u64s
    (0..100).for_each(|i| {
        ptrs[i] = alloc.alloc(layout);
    });

    // Free every other one to maximize fragmentation
    for i in (0..100).step_by(2) {
        alloc.dealloc(ptrs[i], layout);
    }

    // Should have many free blocks now
    let blocks_fragmented = alloc.free_blocks();
    logln!("Fragmented blocks: {}", blocks_fragmented);

    // Free the rest
    for i in (1..100).step_by(2) {
        alloc.dealloc(ptrs[i], layout);
    }

    // Should coalesce back to 1 block
    assert_eq!(alloc.free_blocks(), 1);
    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_coalesce_sequential_forward() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let layout = core::alloc::Layout::new::<[u64; 10]>();

    let p1 = alloc.alloc(layout);
    let p2 = alloc.alloc(layout);
    let p3 = alloc.alloc(layout);

    // Deallocate in forward order - should coalesce
    alloc.dealloc(p1, layout);
    alloc.dealloc(p2, layout);
    alloc.dealloc(p3, layout);

    assert_eq!(alloc.free_blocks(), 1);
    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_coalesce_sequential_backward() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let layout = core::alloc::Layout::new::<[u64; 10]>();

    let p1 = alloc.alloc(layout);
    let p2 = alloc.alloc(layout);
    let p3 = alloc.alloc(layout);

    // Deallocate in reverse order
    alloc.dealloc(p3, layout);
    alloc.dealloc(p2, layout);
    alloc.dealloc(p1, layout);

    assert_eq!(alloc.free_blocks(), 1);
    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_coalesce_middle_first() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let layout = core::alloc::Layout::new::<[u64; 10]>();

    let p1 = alloc.alloc(layout);
    let p2 = alloc.alloc(layout);
    let p3 = alloc.alloc(layout);

    // Deallocate middle first
    alloc.dealloc(p2, layout);
    assert_eq!(alloc.free_blocks(), 2); // p2 + remaining heap

    alloc.dealloc(p1, layout);
    assert_eq!(alloc.free_blocks(), 2); // p1+p2 merged + remaining heap

    alloc.dealloc(p3, layout);
    assert_eq!(alloc.free_blocks(), 1); // All merged
    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_coalesce_extremes_then_middle() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let layout = core::alloc::Layout::new::<[u64; 10]>();

    let p1 = alloc.alloc(layout);
    let p2 = alloc.alloc(layout);
    let p3 = alloc.alloc(layout);
    let p4 = alloc.alloc(layout);
    let p5 = alloc.alloc(layout);

    // Free first and last
    alloc.dealloc(p1, layout);
    alloc.dealloc(p5, layout);

    // Free middle ones
    alloc.dealloc(p3, layout);
    alloc.dealloc(p2, layout);
    alloc.dealloc(p4, layout);

    assert_eq!(alloc.free_blocks(), 1);
    assert_eq!(alloc.free(), init_free);
}

// ============================================================================
// Stress Tests with Different Patterns
// ============================================================================

#[test_case]
fn test_stress_alternating_sizes_small_large() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let small = core::alloc::Layout::new::<u8>();
    let large = core::alloc::Layout::new::<[u64; 100]>();

    let mut small_ptrs = [core::ptr::null_mut(); 20];
    let mut large_ptrs = [core::ptr::null_mut(); 20];

    for i in 0..20 {
        small_ptrs[i] = alloc.alloc(small);
        large_ptrs[i] = alloc.alloc(large);
    }

    // Deallocate in mixed order
    (0..20).rev().for_each(|i| {
        alloc.dealloc(large_ptrs[i], large);
    });
    (0..20).for_each(|i| {
        alloc.dealloc(small_ptrs[i], small);
    });

    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_stress_pyramid_allocation() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    // Allocate in increasing sizes
    let mut ptrs = [core::ptr::null_mut(); 10];
    let mut layouts = [core::alloc::Layout::new::<u8>(); 10];

    for i in 0..10 {
        let size = 8 * (i + 1);
        layouts[i] = core::alloc::Layout::from_size_align(size, 8).unwrap();
        ptrs[i] = alloc.alloc(layouts[i]);
    }

    // Deallocate from largest to smallest
    for i in (0..10).rev() {
        alloc.dealloc(ptrs[i], layouts[i]);
    }

    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_stress_repeated_alloc_dealloc_same_size() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let layout = core::alloc::Layout::new::<[u64; 50]>();

    // Repeatedly allocate and deallocate the same size
    for _ in 0..50 {
        let ptr = alloc.alloc(layout);
        assert!(!ptr.is_null());
        alloc.dealloc(ptr, layout);
    }

    assert_eq!(alloc.free(), init_free);
    assert_eq!(alloc.free_blocks(), 1);
}

#[test_case]
fn test_stress_random_pattern() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let l1 = core::alloc::Layout::new::<u8>();
    let l2 = core::alloc::Layout::new::<u16>();
    let l3 = core::alloc::Layout::new::<u32>();
    let l4 = core::alloc::Layout::new::<u64>();
    let l5 = core::alloc::Layout::new::<u128>();

    // Pseudo-random pattern
    let p1 = alloc.alloc(l3);
    let p2 = alloc.alloc(l1);
    let p3 = alloc.alloc(l5);
    let p4 = alloc.alloc(l2);
    let p5 = alloc.alloc(l4);
    let p6 = alloc.alloc(l1);
    let p7 = alloc.alloc(l3);

    alloc.dealloc(p3, l5);
    alloc.dealloc(p1, l3);
    alloc.dealloc(p5, l4);
    alloc.dealloc(p7, l3);
    alloc.dealloc(p2, l1);
    alloc.dealloc(p6, l1);
    alloc.dealloc(p4, l2);

    assert_eq!(alloc.free(), init_free);
}

// ============================================================================
// Edge Cases and Boundary Conditions
// ============================================================================

#[test_case]
fn test_minimum_allocation_size() {
    let mut alloc = make_alloc();

    // Single byte allocation
    let layout = core::alloc::Layout::new::<u8>();
    let ptr = alloc.alloc(layout);

    assert!(!ptr.is_null());
    unsafe {
        *ptr = 0x42;
    }
    assert_eq!(unsafe { *ptr }, 0x42);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_maximum_reasonable_allocation() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    // Try to allocate most of the heap
    let large_size = init_free - 1024; // Leave some room for metadata
    let layout = core::alloc::Layout::from_size_align(large_size, 8).unwrap();

    let ptr = alloc.alloc(layout);
    assert!(!ptr.is_null());

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_zero_sized_allocation_via_empty_array() {
    let mut alloc = make_alloc();

    // Empty array - still gets minimum allocation
    let layout = core::alloc::Layout::new::<[u8; 0]>();
    let ptr = alloc.alloc(layout);

    assert!(!ptr.is_null()); // Should still return a valid pointer

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_power_of_two_sizes() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let sizes = [8, 16, 32, 64, 128, 256, 512, 1024];
    let mut ptrs = [core::ptr::null_mut(); 8];

    for (i, &size) in sizes.iter().enumerate() {
        let layout = core::alloc::Layout::from_size_align(size, 8).unwrap();
        ptrs[i] = alloc.alloc(layout);
        assert!(!ptrs[i].is_null());
    }

    for (i, &size) in sizes.iter().enumerate() {
        let layout = core::alloc::Layout::from_size_align(size, 8).unwrap();
        alloc.dealloc(ptrs[i], layout);
    }

    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_non_power_of_two_sizes() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let sizes = [7, 13, 23, 37, 67, 123, 237, 511];
    let mut ptrs = [core::ptr::null_mut(); 8];

    for (i, &size) in sizes.iter().enumerate() {
        let layout = core::alloc::Layout::from_size_align(size, 8).unwrap();
        ptrs[i] = alloc.alloc(layout);
        assert!(!ptrs[i].is_null());
    }

    for (i, &size) in sizes.iter().enumerate() {
        let layout = core::alloc::Layout::from_size_align(size, 8).unwrap();
        alloc.dealloc(ptrs[i], layout);
    }

    assert_eq!(alloc.free(), init_free);
}

// ============================================================================
// Alignment Edge Cases
// ============================================================================

#[test_case]
fn test_alignment_2_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 2).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 2, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_4_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 4).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 4, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_8_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 8).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 8, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_16_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 16).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 16, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_32_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 32).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 32, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_64_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 64).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 64, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_128_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 128).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 128, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_256_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 256).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 256, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_512_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 512).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 512, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_1024_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 1024).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 1024, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_alignment_2048_bytes() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::from_size_align(100, 2048).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 2048, 0);

    alloc.dealloc(ptr, layout);
}

// ============================================================================
// Complex Scenarios
// ============================================================================

#[test_case]
fn test_interleaved_different_alignments() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let l1 = core::alloc::Layout::from_size_align(50, 8).unwrap();
    let l2 = core::alloc::Layout::from_size_align(50, 64).unwrap();
    let l3 = core::alloc::Layout::from_size_align(50, 16).unwrap();
    let l4 = core::alloc::Layout::from_size_align(50, 128).unwrap();

    let p1 = alloc.alloc(l1);
    let p2 = alloc.alloc(l2);
    let p3 = alloc.alloc(l3);
    let p4 = alloc.alloc(l4);

    assert_eq!(p1 as usize % 8, 0);
    assert_eq!(p2 as usize % 64, 0);
    assert_eq!(p3 as usize % 16, 0);
    assert_eq!(p4 as usize % 128, 0);

    alloc.dealloc(p2, l2);
    alloc.dealloc(p4, l4);
    alloc.dealloc(p1, l1);
    alloc.dealloc(p3, l3);

    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_reuse_freed_blocks_exact_fit() {
    let mut alloc = make_alloc();

    let layout = core::alloc::Layout::new::<[u64; 20]>();

    // Allocate and free to create a known free block
    let p1 = alloc.alloc(layout);
    let addr1 = p1 as usize;
    alloc.dealloc(p1, layout);

    // Allocate same size again - should reuse same block
    let p2 = alloc.alloc(layout);
    let addr2 = p2 as usize;

    assert_eq!(addr1, addr2, "Should reuse the same memory block");

    alloc.dealloc(p2, layout);
}

#[test_case]
fn test_allocate_after_complex_fragmentation() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let l_small = core::alloc::Layout::new::<u32>();
    let l_medium = core::alloc::Layout::new::<[u64; 10]>();
    let l_large = core::alloc::Layout::new::<[u64; 100]>();

    // Create complex fragmentation
    let s1 = alloc.alloc(l_small);
    let m1 = alloc.alloc(l_medium);
    let s2 = alloc.alloc(l_small);
    let m2 = alloc.alloc(l_medium);
    let s3 = alloc.alloc(l_small);

    alloc.dealloc(m1, l_medium);
    alloc.dealloc(m2, l_medium);

    // Try to allocate large - should find space
    let large = alloc.alloc(l_large);
    assert!(!large.is_null());

    alloc.dealloc(large, l_large);
    alloc.dealloc(s1, l_small);
    alloc.dealloc(s2, l_small);
    alloc.dealloc(s3, l_small);

    assert_eq!(alloc.free(), init_free);
}

#[test_case]
fn test_many_small_then_one_large() {
    let mut alloc = make_alloc();
    let init_free = alloc.free();

    let small = core::alloc::Layout::new::<u64>();
    let mut ptrs = [core::ptr::null_mut(); 30];

    (0..30).for_each(|i| {
        ptrs[i] = alloc.alloc(small);
    });

    (0..30).for_each(|i| {
        alloc.dealloc(ptrs[i], small);
    });

    // Should coalesce and allow large allocation
    let large = core::alloc::Layout::new::<[u64; 500]>();
    let p = alloc.alloc(large);
    assert!(!p.is_null());

    alloc.dealloc(p, large);
    assert_eq!(alloc.free(), init_free);
}

// ============================================================================
// Memory Safety and Correctness Tests
// ============================================================================

#[test_case]
fn test_allocated_memory_is_writable() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<[u8; 1000]>();

    let ptr = alloc.alloc(layout);

    // Write pattern
    unsafe {
        for i in 0..1000 {
            *ptr.add(i) = (i % 256) as u8;
        }
    }

    // Verify pattern
    unsafe {
        for i in 0..1000 {
            assert_eq!(*ptr.add(i), (i % 256) as u8);
        }
    }

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_allocations_dont_overlap() {
    let mut alloc = make_alloc();
    let layout = core::alloc::Layout::new::<[u64; 50]>();

    let p1 = alloc.alloc(layout);
    let p2 = alloc.alloc(layout);
    let p3 = alloc.alloc(layout);

    let addr1 = p1 as usize;
    let addr2 = p2 as usize;
    let addr3 = p3 as usize;

    // Check they don't overlap
    assert!(addr1 + layout.size() <= addr2 || addr2 + layout.size() <= addr1);
    assert!(addr2 + layout.size() <= addr3 || addr3 + layout.size() <= addr2);
    assert!(addr1 + layout.size() <= addr3 || addr3 + layout.size() <= addr1);

    alloc.dealloc(p1, layout);
    alloc.dealloc(p2, layout);
    alloc.dealloc(p3, layout);
}

#[test_case]
fn test_different_types_aligned_correctly() {
    let mut alloc = make_alloc();

    let p_u8 = alloc.alloc(core::alloc::Layout::new::<u8>());
    let p_u16 = alloc.alloc(core::alloc::Layout::new::<u16>());
    let p_u32 = alloc.alloc(core::alloc::Layout::new::<u32>());
    let p_u64 = alloc.alloc(core::alloc::Layout::new::<u64>());
    let p_u128 = alloc.alloc(core::alloc::Layout::new::<u128>());

    assert_eq!(p_u16 as usize % 2, 0);
    assert_eq!(p_u32 as usize % 4, 0);
    assert_eq!(p_u64 as usize % 8, 0);
    assert_eq!(p_u128 as usize % 16, 0);

    alloc.dealloc(p_u8, core::alloc::Layout::new::<u8>());
    alloc.dealloc(p_u16, core::alloc::Layout::new::<u16>());
    alloc.dealloc(p_u32, core::alloc::Layout::new::<u32>());
    alloc.dealloc(p_u64, core::alloc::Layout::new::<u64>());
    alloc.dealloc(p_u128, core::alloc::Layout::new::<u128>());
}

#[test_case]
fn test_struct_alignment() {
    let mut alloc = make_alloc();

    #[repr(C)]
    struct TestStruct {
        a: u8,
        b: u64,
        c: u32,
    }

    let layout = core::alloc::Layout::new::<TestStruct>();
    let ptr = alloc.alloc(layout) as *mut TestStruct;

    assert_eq!(ptr as usize % core::mem::align_of::<TestStruct>(), 0);

    unsafe {
        (*ptr).a = 42;
        (*ptr).b = 0xDEADBEEFCAFEBABE;
        (*ptr).c = 0x12345678;

        assert_eq!((*ptr).a, 42);
        assert_eq!((*ptr).b, 0xDEADBEEFCAFEBABE);
        assert_eq!((*ptr).c, 0x12345678);
    }

    alloc.dealloc(ptr as *mut u8, layout);
}

// ============================================================================
// Specific RISC-V Instruction Alignment Tests
// ============================================================================

#[test_case]
fn test_riscv_compressed_instruction_alignment() {
    let mut alloc = make_alloc();
    // Compressed instructions (C extension) require 2-byte alignment
    let layout = core::alloc::Layout::from_size_align(100, 2).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 2, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_riscv_standard_instruction_alignment() {
    let mut alloc = make_alloc();
    // Standard instructions require 4-byte alignment
    let layout = core::alloc::Layout::from_size_align(100, 4).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 4, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_riscv_float_double_alignment() {
    let mut alloc = make_alloc();
    // f64 requires 8-byte alignment for fld/fsd instructions
    let layout = core::alloc::Layout::new::<f64>();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 8, 0);

    alloc.dealloc(ptr, layout);
}

#[test_case]
fn test_riscv_vector_alignment() {
    let mut alloc = make_alloc();
    // Vector operations often require 16 or 32 byte alignment
    let layout = core::alloc::Layout::from_size_align(256, 32).unwrap();

    let ptr = alloc.alloc(layout);
    assert_eq!(ptr as usize % 32, 0);

    alloc.dealloc(ptr, layout);
}
