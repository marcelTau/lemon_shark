#![cfg_attr(not(test), no_std)]

use core::alloc::Layout;
use core::mem;
use core::ops::ControlFlow;

pub fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct AlignedPtr<T>(*mut T);

impl<T> AlignedPtr<T> {
    /// Minimum alignment for data pointers to make `AllocationMetaData`
    /// automatically aligned right before the pointer.
    const MIN_ALIGN: usize = 8;

    pub fn new(addr: usize) -> Self {
        Self(align_up(addr, mem::align_of::<T>().max(Self::MIN_ALIGN)) as *mut T)
    }

    pub fn new_with(addr: usize, align: usize) -> Self {
        Self(align_up(addr, align.max(Self::MIN_ALIGN)) as *mut T)
    }

    pub fn as_ptr(&self) -> *mut T {
        self.0
    }

    pub fn as_addr(&self) -> usize {
        self.0 as usize
    }

    /// This enforces the correct alignment of `AllocationMetaData` as
    /// `Self::MIN_ALIGN` is 8 and hence writing the metadata right before
    /// an aligned address automatically aligns it as well.
    ///
    /// # Safety
    ///
    /// Caller must ensure there is enough valid, writable memory immediately
    /// before `self.as_addr()` (i.e. at `self.as_addr() - size_of::<AllocationMetaData>()`).
    pub unsafe fn write_metadata(&self, metadata: AllocationMetaData) {
        let md_addr = self.as_addr() - mem::size_of::<AllocationMetaData>();
        unsafe {
            *(md_addr as *mut AllocationMetaData) = metadata;
        }
    }

    /// # Safety
    ///
    /// Caller must ensure that valid `AllocationMetaData` was previously written
    /// immediately before `self.as_addr()` via `write_metadata`.
    pub unsafe fn read_metadata(&self) -> &AllocationMetaData {
        let md_addr = self.as_addr() - mem::size_of::<AllocationMetaData>();
        unsafe { &*(md_addr as *mut AllocationMetaData) }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FreeBlock {
    pub size: usize,
    pub next: Option<AlignedPtr<FreeBlock>>,
}

impl FreeBlock {
    pub fn can_allocate(&self, align: usize, size: usize) -> Option<AlignedPtr<u8>> {
        let self_addr = self as *const Self as usize;
        let self_end_addr = self_addr + self.size;

        // Need space to store the MetaData
        let next_possible_addr = self_addr + mem::size_of::<AllocationMetaData>();

        // Align the address
        let next_possible_addr_aligned = AlignedPtr::new_with(next_possible_addr, align);

        // Check if the aligned address + the requested size fits into this block
        if self_end_addr >= next_possible_addr_aligned.as_addr() + size {
            Some(next_possible_addr_aligned)
        } else {
            None
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct AllocationMetaData {
    /// Start address of this allocation including padding and meta data.
    pub start_addr: usize,

    /// Actual size of the allocation from the Allocator's PoV.
    pub size: usize,
}

#[derive(Default)]
pub struct FreeListAllocator {
    pub head: Option<AlignedPtr<FreeBlock>>,
}

impl FreeListAllocator {
    /// Initialize the allocator with an explicit heap range.
    ///
    /// # Safety
    ///
    /// The memory range `[start, end)` must be valid, writable, and exclusively
    /// owned by this allocator for its entire lifetime.
    pub unsafe fn init(&mut self, start: usize, end: usize) {
        let heap_size = end - start;
        let aligned_ptr: AlignedPtr<FreeBlock> = AlignedPtr::new(start);

        unsafe {
            (*aligned_ptr.as_ptr()).size = heap_size - mem::size_of::<FreeBlock>();
            (*aligned_ptr.as_ptr()).next = None;

            log::info!(
                "[ALLOC] Initialized allocator at {:#x} with size of {:#x} ({}KB)",
                start,
                (*aligned_ptr.as_ptr()).size,
                (*aligned_ptr.as_ptr()).size / 1024
            );
        }

        self.head = Some(aligned_ptr);
    }

    pub fn free(&self) -> usize {
        let mut total = 0;

        let _ = Self::walk_list(self.head, |current, _| {
            total += unsafe { (*current.as_ptr()).size };
            ControlFlow::<(), _>::Continue(())
        });

        total
    }

    pub fn free_blocks(&self) -> usize {
        let mut total = 0;

        let _ = Self::walk_list(self.head, |_, _| {
            total += 1;
            ControlFlow::<(), _>::Continue(())
        });

        total
    }

    fn walk_list<F, R>(start: Option<AlignedPtr<FreeBlock>>, mut f: F) -> Option<R>
    where
        F: FnMut(AlignedPtr<FreeBlock>, Option<AlignedPtr<FreeBlock>>) -> ControlFlow<R>,
    {
        let mut prev: Option<AlignedPtr<FreeBlock>> = None;
        let mut current: Option<AlignedPtr<FreeBlock>> = start;

        while let Some(block) = current {
            if let ControlFlow::Break(res) = f(block, prev) {
                return Some(res);
            };
            prev = current;
            current = unsafe { (*block.as_ptr()).next };
        }
        None
    }

    /// Size is always aligned to 8.
    /// Each pointer is aligned correctly.
    /// The FreeBlock's in the list are all aligned.
    ///
    /// Because the DataPointer & Size are both aligned, the FreeBlock should be automatically aligned.
    pub fn alloc(&mut self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(8);
        let size = align_up(layout.size(), 8);

        log::trace!(
            "[ALLOC] allocating {size} (req: {}) bytes with alignment {align} (req: {})",
            layout.size(),
            layout.align()
        );

        let result = unsafe {
            Self::walk_list(self.head, |block, prev| {
                let Some(aligned_ptr) = (*block.as_ptr()).can_allocate(align, size) else {
                    return ControlFlow::Continue(());
                };

                log::trace!("[ALLOC] Found block to allocate");

                let block_ptr = *block.as_ptr();

                // Required size for a valid new block
                let required_size =
                    mem::size_of::<FreeBlock>().max(mem::size_of::<AllocationMetaData>());

                // Number of bytes left of the allocation
                let bytes_left = (aligned_ptr.as_addr() - mem::size_of::<AllocationMetaData>())
                    - block.as_addr();

                let bytes_right =
                    (block.as_addr() + block_ptr.size) - (aligned_ptr.as_addr() + size);

                if bytes_left >= required_size {
                    log::trace!("[ALLOC] Splitting block to the left");
                }

                if bytes_right >= required_size {
                    log::trace!("[ALLOC] Splitting block to the right");
                }

                let metadata = match (bytes_left >= required_size, bytes_right >= required_size) {
                    (false, false) => {
                        // remove this `FreeBlock` from the list
                        match prev {
                            Some(prev) => (*prev.as_ptr()).next = block_ptr.next,
                            None => self.head = block_ptr.next,
                        }

                        AllocationMetaData {
                            start_addr: block.as_addr(),
                            size: block_ptr.size,
                        }
                    }
                    (true, false) => {
                        let left_block: AlignedPtr<FreeBlock> = AlignedPtr::new(block.as_addr());
                        (*left_block.as_ptr()).size = bytes_left;
                        (*left_block.as_ptr()).next = block_ptr.next;

                        AllocationMetaData {
                            start_addr: block.as_addr() + bytes_left,
                            size: block_ptr.size - bytes_left,
                        }
                    }
                    (false, true) => {
                        let right_block: AlignedPtr<FreeBlock> =
                            AlignedPtr::new(aligned_ptr.as_addr() + size);

                        (*right_block.as_ptr()).size = bytes_right;
                        (*right_block.as_ptr()).next = block_ptr.next;

                        match prev {
                            Some(prev) => (*prev.as_ptr()).next = Some(right_block),
                            None => self.head = Some(right_block),
                        }

                        AllocationMetaData {
                            start_addr: block.as_addr(),
                            size: bytes_left + mem::size_of::<AllocationMetaData>() + size,
                        }
                    }
                    (true, true) => {
                        let left_block: AlignedPtr<FreeBlock> = AlignedPtr::new(block.as_addr());
                        let right_block: AlignedPtr<FreeBlock> =
                            AlignedPtr::new(aligned_ptr.as_addr() + size);

                        (*left_block.as_ptr()).size = bytes_left;
                        (*right_block.as_ptr()).size = bytes_right;

                        (*left_block.as_ptr()).next = Some(right_block);
                        (*right_block.as_ptr()).next = block_ptr.next;

                        AllocationMetaData {
                            start_addr: block.as_addr() + bytes_left,
                            size: block_ptr.size - bytes_left - bytes_right,
                        }
                    }
                };

                log::trace!("[ALLOC] AllocationMetaData: {metadata:#0x?}");

                aligned_ptr.write_metadata(metadata);
                log::trace!("[ALLOC] wrote metadata");
                ControlFlow::Break(aligned_ptr)
            })
        };

        result
            .unwrap_or_else(|| panic!("Out of memory, requested {}kb", size / 1024))
            .as_ptr()
    }

    pub fn dealloc(&mut self, ptr: *mut u8, _layout: Layout) {
        // Don't re-align! The ptr is already the aligned address we returned from alloc()
        let aligned_ptr: AlignedPtr<u8> = AlignedPtr(ptr);
        let (size, start_addr) = {
            // NOTE: limit scope of metadata as we're writing into that memory
            // and thus change the value.
            let metadata = unsafe { aligned_ptr.read_metadata() };
            (metadata.size, metadata.start_addr)
        };

        let new_block: AlignedPtr<FreeBlock> = AlignedPtr::new(start_addr);

        log::trace!("[ALLOC] deallocating {size} bytes at {start_addr:#x}",);

        unsafe {
            (*new_block.as_ptr()).size = size;
            (*new_block.as_ptr()).next = None;
        }

        if self.head.is_none() {
            log::trace!("Head is none!");
            self.head = Some(new_block);
            return;
        }

        unsafe {
            let head = self.head.unwrap();

            if start_addr < head.as_addr() {
                log::trace!(
                    "start_addr={start_addr} size={size} head={:#x?} diff={}",
                    head.as_addr(),
                    head.as_addr() - (start_addr + size),
                );
                if start_addr + size == head.as_addr() {
                    (*new_block.as_ptr()).size += (*head.as_ptr()).size;
                    (*new_block.as_ptr()).next = (*head.as_ptr()).next;
                } else {
                    (*new_block.as_ptr()).next = Some(head);
                }

                self.head = Some(new_block);
                return;
            }

            let mut prev = head;

            while (*prev.as_ptr()).next.is_some()
                && ((*prev.as_ptr())
                    .next
                    .is_some_and(|next| next.as_addr() < start_addr))
            {
                prev = (*prev.as_ptr()).next.unwrap();
            }

            let next = (*prev.as_ptr()).next;

            let merge_left = (prev.as_addr()) + (*prev.as_ptr()).size == start_addr;
            let merge_right = next.is_some_and(|next| start_addr + size == next.as_addr());

            log::trace!(
                "[ALLOC] merge_left: {}, merge_right: {}",
                merge_left,
                merge_right
            );

            match (merge_left, merge_right) {
                (true, true) => {
                    (*prev.as_ptr()).size += size + (*next.unwrap().as_ptr()).size;
                    (*prev.as_ptr()).next = (*next.unwrap().as_ptr()).next;
                }
                (true, false) => {
                    (*prev.as_ptr()).size += size;
                }
                (false, true) => {
                    (*new_block.as_ptr()).size += (*next.unwrap().as_ptr()).size;
                    (*new_block.as_ptr()).next = (*next.unwrap().as_ptr()).next;
                    (*prev.as_ptr()).next = Some(new_block);
                }
                (false, false) => {
                    (*new_block.as_ptr()).next = next;
                    (*prev.as_ptr()).next = Some(new_block);
                }
            }
        }
    }

    pub fn dump_state(&self, out: &mut impl core::fmt::Write) {
        if self.head.is_none() {
            let _ = out.write_str("========== ALLOCATOR DUMP ==========\n");
            let _ = out.write_str("No more free memory :(\n");
            let _ = out.write_str("====================================\n");
            return;
        }

        let mut i = 0;
        let start_addr = self.head.unwrap().as_addr();
        let mut total_size = 0;

        let _ = core::fmt::write(out, format_args!("========== ALLOCATOR DUMP ==========\n"));
        let _ = core::fmt::write(out, format_args!("Allocator starting at {start_addr:#x}\n"));

        unsafe {
            Self::walk_list(self.head, |current, _| {
                let _ = core::fmt::write(
                    out,
                    format_args!(
                        "  Block {i} at={:#0x} size={} next={:?}\n",
                        current.as_addr(),
                        (*current.as_ptr()).size,
                        (*current.as_ptr()).next
                    ),
                );
                total_size += (*current.as_ptr()).size;
                if (*current.as_ptr()).next.is_none() {
                    return ControlFlow::Break(());
                }
                i += 1;
                ControlFlow::Continue(())
            });
        }

        let _ = core::fmt::write(out, format_args!("Total free memory: {total_size} bytes\n"));
        let _ = out.write_str("====================================\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test gets its own heap allocation so tests can safely run in parallel.
    const HEAP_SIZE: usize = 4 * 1024 * 1024;

    struct TestHeap {
        alloc: FreeListAllocator,
        // Keep the backing storage alive for the lifetime of the allocator.
        _storage: Vec<u8>,
    }

    fn make_alloc() -> TestHeap {
        // Vec avoids the stack allocation of 4 MiB that `Box::new([0u8; N])`
        // would cause. Each call gets a fresh, independent region so parallel
        // tests can't interfere with each other.
        let storage = vec![0u8; HEAP_SIZE];
        let start = storage.as_ptr() as usize;
        let end = start + HEAP_SIZE;

        let mut alloc = FreeListAllocator::default();
        unsafe { alloc.init(start, end) };

        TestHeap {
            alloc,
            _storage: storage,
        }
    }

    impl core::ops::Deref for TestHeap {
        type Target = FreeListAllocator;
        fn deref(&self) -> &Self::Target {
            &self.alloc
        }
    }

    impl core::ops::DerefMut for TestHeap {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.alloc
        }
    }

    impl TestHeap {
        fn free(&self) -> usize {
            self.alloc.free()
        }
        fn free_blocks(&self) -> usize {
            self.alloc.free_blocks()
        }
        fn alloc(&mut self, layout: core::alloc::Layout) -> *mut u8 {
            self.alloc.alloc(layout)
        }
        fn dealloc(&mut self, ptr: *mut u8, layout: core::alloc::Layout) {
            self.alloc.dealloc(ptr, layout)
        }
        fn dump_state(&self, out: &mut impl core::fmt::Write) {
            self.alloc.dump_state(out)
        }
    }

    // Sink for dump_state() output — discards everything.
    struct NullWriter;
    impl core::fmt::Write for NullWriter {
        fn write_str(&mut self, _s: &str) -> core::fmt::Result {
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Basic init / alloc / dealloc
    // -----------------------------------------------------------------------

    #[test]
    fn allocator_init() {
        let _heap = make_alloc();
    }

    #[test]
    fn allocator_can_allocate() {
        let mut alloc = make_alloc();

        let initial_free_bytes = alloc.free();

        let layout = core::alloc::Layout::new::<[usize; 100]>();
        let align = layout.align();

        let ptr = alloc.alloc(layout);
        let mem = ptr as *mut usize;

        assert!(!mem.is_null());
        assert_eq!(mem as usize % align, 0);

        unsafe { core::ptr::write_bytes(mem, 1u8, 100) };

        for i in 0..100 {
            unsafe { *mem.add(i) = i };
        }
        for i in 0..100 {
            unsafe { assert_eq!(*mem.add(i), i) };
        }

        assert_eq!(alloc.free_blocks(), 1);

        alloc.dealloc(ptr, layout);
        assert_eq!(alloc.free(), initial_free_bytes);
    }

    #[test]
    fn fragmentation_and_merge() {
        let mut alloc = make_alloc();

        let layout = core::alloc::Layout::new::<[usize; 100]>();
        let initial_free = alloc.free();

        assert_eq!(alloc.free_blocks(), 1);

        let left = alloc.alloc(layout);
        let mid = alloc.alloc(layout);
        let right = alloc.alloc(layout);

        assert_eq!(alloc.free_blocks(), 1);

        alloc.dealloc(mid, layout);
        assert_eq!(alloc.free_blocks(), 2);

        alloc.dealloc(left, layout);
        assert_eq!(alloc.free_blocks(), 2);

        alloc.dealloc(right, layout);
        assert_eq!(alloc.free_blocks(), 1);
        assert_eq!(alloc.free(), initial_free);

        alloc.dump_state(&mut NullWriter);
    }

    #[test]
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

        alloc.dump_state(&mut NullWriter);

        assert_eq!(alloc.free_blocks(), 1);
        assert_eq!(init, alloc.free());
    }

    // -----------------------------------------------------------------------
    // Alignment Tests
    // -----------------------------------------------------------------------

    #[test]
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

    #[test]
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

    #[test]
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

    #[test]
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

    #[test]
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

    // -----------------------------------------------------------------------
    // Mixed Alignment Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_mixed_alignments_sequence() {
        let mut alloc = make_alloc();

        let layout_u8 = core::alloc::Layout::new::<u8>();
        let layout_u64 = core::alloc::Layout::new::<u64>();
        let layout_u128 = core::alloc::Layout::new::<u128>();

        let p1 = alloc.alloc(layout_u8);
        let p2 = alloc.alloc(layout_u64);
        let p3 = alloc.alloc(layout_u128);

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

    #[test]
    fn test_alignment_after_dealloc_and_realloc() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout_small = core::alloc::Layout::new::<[u8; 7]>();
        let p1 = alloc.alloc(layout_small);
        alloc.dealloc(p1, layout_small);

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

    #[test]
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

        assert_eq!(b as usize % 4, 0, "u32 array not 4-byte aligned");
        assert_eq!(d as usize % 8, 0, "u64 array not 8-byte aligned");
        assert_eq!(e as usize % 4, 0, "second u32 array not 4-byte aligned");

        alloc.dealloc(b, layout_u32);
        alloc.dealloc(d, layout_u64);
        alloc.dealloc(a, layout_u8);
        alloc.dealloc(e, layout_u32);
        alloc.dealloc(c, layout_u8);
    }

    // -----------------------------------------------------------------------
    // Array Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_large_u8_array() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::new::<[u8; 1024]>();

        let ptr = alloc.alloc(layout);
        assert!(!ptr.is_null());

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

    #[test]
    fn test_u32_array_alignment() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::new::<[u32; 256]>();

        let ptr = alloc.alloc(layout) as *mut u32;
        assert_eq!(ptr as usize % 4, 0, "u32 array not 4-byte aligned");

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

    #[test]
    fn test_u64_array_alignment() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::new::<[u64; 128]>();

        let ptr = alloc.alloc(layout) as *mut u64;
        assert_eq!(ptr as usize % 8, 0, "u64 array not 8-byte aligned");

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

    // -----------------------------------------------------------------------
    // Fragmentation Stress Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_many_small_allocations() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout = core::alloc::Layout::new::<u64>();
        let mut ptrs = [core::ptr::null_mut(); 50];

        for i in 0..50 {
            ptrs[i] = alloc.alloc(layout);
            assert!(!ptrs[i].is_null());
            assert_eq!(ptrs[i] as usize % 8, 0, "allocation {} not aligned", i);
        }

        for i in 0..50 {
            alloc.dealloc(ptrs[i], layout);
        }

        assert_eq!(alloc.free(), init_free, "memory leak detected");
    }

    #[test]
    fn test_alternating_sizes() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let small = core::alloc::Layout::new::<u32>();
        let large = core::alloc::Layout::new::<[u64; 32]>();

        let mut small_ptrs = [core::ptr::null_mut(); 10];
        let mut large_ptrs = [core::ptr::null_mut(); 10];

        for i in 0..10 {
            small_ptrs[i] = alloc.alloc(small);
            large_ptrs[i] = alloc.alloc(large);
        }

        for i in 0..10 {
            alloc.dealloc(small_ptrs[i], small);
        }
        for i in 0..10 {
            alloc.dealloc(large_ptrs[i], large);
        }

        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_reverse_deallocation_order() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout = core::alloc::Layout::new::<[usize; 16]>();
        let mut ptrs = [core::ptr::null_mut(); 20];

        for i in 0..20 {
            ptrs[i] = alloc.alloc(layout);
        }

        for i in (0..20).rev() {
            alloc.dealloc(ptrs[i], layout);
        }

        assert_eq!(alloc.free(), init_free);
        assert_eq!(alloc.free_blocks(), 1);
    }

    // -----------------------------------------------------------------------
    // Edge Cases and Boundary Tests
    // -----------------------------------------------------------------------

    #[test]
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

    #[test]
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

    #[test]
    fn test_max_alignment_request() {
        let mut alloc = make_alloc();

        let layout = core::alloc::Layout::from_size_align(128, 64).unwrap();
        let ptr = alloc.alloc(layout);

        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 64, 0, "allocation not 64-byte aligned");

        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_stress_fragmentation() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let l1 = core::alloc::Layout::new::<[u8; 13]>();
        let l2 = core::alloc::Layout::new::<[u16; 7]>();
        let l3 = core::alloc::Layout::new::<[u32; 11]>();
        let l4 = core::alloc::Layout::new::<[u64; 5]>();

        let a1 = alloc.alloc(l1);
        let b1 = alloc.alloc(l2);
        let c1 = alloc.alloc(l3);
        let d1 = alloc.alloc(l4);
        let a2 = alloc.alloc(l1);
        let b2 = alloc.alloc(l2);
        let c2 = alloc.alloc(l3);

        alloc.dealloc(b1, l2);
        alloc.dealloc(d1, l4);
        alloc.dealloc(a1, l1);
        alloc.dealloc(c2, l3);
        alloc.dealloc(b2, l2);
        alloc.dealloc(a2, l1);
        alloc.dealloc(c1, l3);

        assert_eq!(alloc.free(), init_free);
    }

    // -----------------------------------------------------------------------
    // Alignment Verification After Complex Operations
    // -----------------------------------------------------------------------

    #[test]
    fn test_alignment_preservation() {
        let mut alloc = make_alloc();

        let l8 = core::alloc::Layout::new::<[u8; 100]>();
        let l64 = core::alloc::Layout::new::<[u64; 50]>();

        let tmp = alloc.alloc(l8);
        alloc.dealloc(tmp, l8);

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

    #[test]
    fn test_multiple_alignment_boundaries() {
        let mut alloc = make_alloc();

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

    #[test]
    fn test_realistic_kernel_allocations() {
        let mut alloc = make_alloc();

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

    // -----------------------------------------------------------------------
    // RISC-V Specific Alignment Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_riscv_atomic_u32_alignment() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::new::<core::sync::atomic::AtomicU32>();

        let ptr = alloc.alloc(layout);
        assert_eq!(
            ptr as usize % 4,
            0,
            "AtomicU32 not 4-byte aligned for RISC-V lr.w/sc.w"
        );

        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_riscv_atomic_u64_alignment() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::new::<core::sync::atomic::AtomicU64>();

        let ptr = alloc.alloc(layout);
        assert_eq!(
            ptr as usize % 8,
            0,
            "AtomicU64 not 8-byte aligned for RISC-V lr.d/sc.d"
        );

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

    #[test]
    fn test_riscv_cache_line_alignment() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(64, 64).unwrap();

        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 64, 0, "Cache line not 64-byte aligned");

        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_riscv_page_alignment_4k() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(4096, 4096).unwrap();

        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 4096, 0, "Page not 4KB aligned");

        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_riscv_double_word_loads() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::new::<[u64; 10]>();

        let ptr = alloc.alloc(layout) as *mut u64;
        assert_eq!(
            ptr as usize % 8,
            0,
            "u64 array not aligned for ld instruction"
        );

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

    #[test]
    fn test_riscv_word_loads() {
        let mut alloc = make_alloc();
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

    #[test]
    fn test_riscv_halfword_loads() {
        let mut alloc = make_alloc();
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

    // -----------------------------------------------------------------------
    // Block Splitting — All Four Scenarios
    // -----------------------------------------------------------------------

    #[test]
    fn test_split_neither_left_nor_right() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout = core::alloc::Layout::from_size_align(32, 8).unwrap();
        let p1 = alloc.alloc(layout);

        assert!(!p1.is_null());
        assert_eq!(p1 as usize % 8, 0);

        alloc.dealloc(p1, layout);
        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_split_left_only() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let l1 = core::alloc::Layout::from_size_align(100, 8).unwrap();
        let l2 = core::alloc::Layout::from_size_align(200, 64).unwrap();

        let p1 = alloc.alloc(l1);
        alloc.dealloc(p1, l1);

        let p2 = alloc.alloc(l2);
        assert_eq!(p2 as usize % 64, 0);

        alloc.dealloc(p2, l2);
        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_split_right_only() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let l1 = core::alloc::Layout::from_size_align(50, 8).unwrap();
        let p1 = alloc.alloc(l1);

        assert!(!p1.is_null());

        alloc.dealloc(p1, l1);
        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_split_both_left_and_right() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let l_small = core::alloc::Layout::from_size_align(32, 8).unwrap();
        let l_aligned = core::alloc::Layout::from_size_align(64, 128).unwrap();

        let p1 = alloc.alloc(l_small);
        alloc.dealloc(p1, l_small);

        let p2 = alloc.alloc(l_aligned);
        assert_eq!(p2 as usize % 128, 0);

        alloc.dealloc(p2, l_aligned);
        assert_eq!(alloc.free(), init_free);
    }

    // -----------------------------------------------------------------------
    // Fragmentation and Coalescing Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_fragmentation_max() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout = core::alloc::Layout::new::<u64>();
        let mut ptrs = [core::ptr::null_mut(); 100];

        (0..100).for_each(|i| {
            ptrs[i] = alloc.alloc(layout);
        });

        // Free every other one to maximize fragmentation
        for i in (0..100).step_by(2) {
            alloc.dealloc(ptrs[i], layout);
        }

        let blocks_fragmented = alloc.free_blocks();
        eprintln!("Fragmented blocks: {}", blocks_fragmented);

        // Free the rest
        for i in (1..100).step_by(2) {
            alloc.dealloc(ptrs[i], layout);
        }

        assert_eq!(alloc.free_blocks(), 1);
        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_coalesce_sequential_forward() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout = core::alloc::Layout::new::<[u64; 10]>();

        let p1 = alloc.alloc(layout);
        let p2 = alloc.alloc(layout);
        let p3 = alloc.alloc(layout);

        alloc.dealloc(p1, layout);
        alloc.dealloc(p2, layout);
        alloc.dealloc(p3, layout);

        assert_eq!(alloc.free_blocks(), 1);
        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_coalesce_sequential_backward() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout = core::alloc::Layout::new::<[u64; 10]>();

        let p1 = alloc.alloc(layout);
        let p2 = alloc.alloc(layout);
        let p3 = alloc.alloc(layout);

        alloc.dealloc(p3, layout);
        alloc.dealloc(p2, layout);
        alloc.dealloc(p1, layout);

        assert_eq!(alloc.free_blocks(), 1);
        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_coalesce_middle_first() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout = core::alloc::Layout::new::<[u64; 10]>();

        let p1 = alloc.alloc(layout);
        let p2 = alloc.alloc(layout);
        let p3 = alloc.alloc(layout);

        alloc.dealloc(p2, layout);
        assert_eq!(alloc.free_blocks(), 2);

        alloc.dealloc(p1, layout);
        assert_eq!(alloc.free_blocks(), 2);

        alloc.dealloc(p3, layout);
        assert_eq!(alloc.free_blocks(), 1);
        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_coalesce_extremes_then_middle() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout = core::alloc::Layout::new::<[u64; 10]>();

        let p1 = alloc.alloc(layout);
        let p2 = alloc.alloc(layout);
        let p3 = alloc.alloc(layout);
        let p4 = alloc.alloc(layout);
        let p5 = alloc.alloc(layout);

        alloc.dealloc(p1, layout);
        alloc.dealloc(p5, layout);

        alloc.dealloc(p3, layout);
        alloc.dealloc(p2, layout);
        alloc.dealloc(p4, layout);

        assert_eq!(alloc.free_blocks(), 1);
        assert_eq!(alloc.free(), init_free);
    }

    // -----------------------------------------------------------------------
    // Stress Tests with Different Patterns
    // -----------------------------------------------------------------------

    #[test]
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

        (0..20).rev().for_each(|i| {
            alloc.dealloc(large_ptrs[i], large);
        });
        (0..20).for_each(|i| {
            alloc.dealloc(small_ptrs[i], small);
        });

        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_stress_pyramid_allocation() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let mut ptrs = [core::ptr::null_mut(); 10];
        let mut layouts = [core::alloc::Layout::new::<u8>(); 10];

        for i in 0..10 {
            let size = 8 * (i + 1);
            layouts[i] = core::alloc::Layout::from_size_align(size, 8).unwrap();
            ptrs[i] = alloc.alloc(layouts[i]);
        }

        for i in (0..10).rev() {
            alloc.dealloc(ptrs[i], layouts[i]);
        }

        assert_eq!(alloc.free(), init_free);
    }

    #[test]
    fn test_stress_repeated_alloc_dealloc_same_size() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let layout = core::alloc::Layout::new::<[u64; 50]>();

        for _ in 0..50 {
            let ptr = alloc.alloc(layout);
            assert!(!ptr.is_null());
            alloc.dealloc(ptr, layout);
        }

        assert_eq!(alloc.free(), init_free);
        assert_eq!(alloc.free_blocks(), 1);
    }

    #[test]
    fn test_stress_random_pattern() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let l1 = core::alloc::Layout::new::<u8>();
        let l2 = core::alloc::Layout::new::<u16>();
        let l3 = core::alloc::Layout::new::<u32>();
        let l4 = core::alloc::Layout::new::<u64>();
        let l5 = core::alloc::Layout::new::<u128>();

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

    // -----------------------------------------------------------------------
    // Edge Cases and Boundary Conditions
    // -----------------------------------------------------------------------

    #[test]
    fn test_minimum_allocation_size() {
        let mut alloc = make_alloc();

        let layout = core::alloc::Layout::new::<u8>();
        let ptr = alloc.alloc(layout);

        assert!(!ptr.is_null());
        unsafe { *ptr = 0x42 }
        assert_eq!(unsafe { *ptr }, 0x42);

        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_maximum_reasonable_allocation() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let large_size = init_free - 1024;
        let layout = core::alloc::Layout::from_size_align(large_size, 8).unwrap();

        let ptr = alloc.alloc(layout);
        assert!(!ptr.is_null());

        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_zero_sized_allocation_via_empty_array() {
        let mut alloc = make_alloc();

        let layout = core::alloc::Layout::new::<[u8; 0]>();
        let ptr = alloc.alloc(layout);

        assert!(!ptr.is_null());

        alloc.dealloc(ptr, layout);
    }

    #[test]
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

    #[test]
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

    // -----------------------------------------------------------------------
    // Alignment Edge Cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_alignment_2_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 2).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 2, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_4_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 4).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 4, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_8_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 8).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 8, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_16_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 16).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 16, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_32_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 32).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 32, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_64_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 64).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 64, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_128_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 128).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 128, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_256_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 256).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 256, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_512_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 512).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 512, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_1024_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 1024).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 1024, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_alignment_2048_bytes() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 2048).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 2048, 0);
        alloc.dealloc(ptr, layout);
    }

    // -----------------------------------------------------------------------
    // Complex Scenarios
    // -----------------------------------------------------------------------

    #[test]
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

    #[test]
    fn test_reuse_freed_blocks_exact_fit() {
        let mut alloc = make_alloc();

        let layout = core::alloc::Layout::new::<[u64; 20]>();

        let p1 = alloc.alloc(layout);
        let addr1 = p1 as usize;
        alloc.dealloc(p1, layout);

        let p2 = alloc.alloc(layout);
        let addr2 = p2 as usize;

        assert_eq!(addr1, addr2, "Should reuse the same memory block");

        alloc.dealloc(p2, layout);
    }

    #[test]
    fn test_allocate_after_complex_fragmentation() {
        let mut alloc = make_alloc();
        let init_free = alloc.free();

        let l_small = core::alloc::Layout::new::<u32>();
        let l_medium = core::alloc::Layout::new::<[u64; 10]>();
        let l_large = core::alloc::Layout::new::<[u64; 100]>();

        let s1 = alloc.alloc(l_small);
        let m1 = alloc.alloc(l_medium);
        let s2 = alloc.alloc(l_small);
        let m2 = alloc.alloc(l_medium);
        let s3 = alloc.alloc(l_small);

        alloc.dealloc(m1, l_medium);
        alloc.dealloc(m2, l_medium);

        let large = alloc.alloc(l_large);
        assert!(!large.is_null());

        alloc.dealloc(large, l_large);
        alloc.dealloc(s1, l_small);
        alloc.dealloc(s2, l_small);
        alloc.dealloc(s3, l_small);

        assert_eq!(alloc.free(), init_free);
    }

    #[test]
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

        let large = core::alloc::Layout::new::<[u64; 500]>();
        let p = alloc.alloc(large);
        assert!(!p.is_null());

        alloc.dealloc(p, large);
        assert_eq!(alloc.free(), init_free);
    }

    // -----------------------------------------------------------------------
    // Memory Safety and Correctness Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_allocated_memory_is_writable() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::new::<[u8; 1000]>();

        let ptr = alloc.alloc(layout);

        unsafe {
            for i in 0..1000 {
                *ptr.add(i) = (i % 256) as u8;
            }
            for i in 0..1000 {
                assert_eq!(*ptr.add(i), (i % 256) as u8);
            }
        }

        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_allocations_dont_overlap() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::new::<[u64; 50]>();

        let p1 = alloc.alloc(layout);
        let p2 = alloc.alloc(layout);
        let p3 = alloc.alloc(layout);

        let addr1 = p1 as usize;
        let addr2 = p2 as usize;
        let addr3 = p3 as usize;

        assert!(addr1 + layout.size() <= addr2 || addr2 + layout.size() <= addr1);
        assert!(addr2 + layout.size() <= addr3 || addr3 + layout.size() <= addr2);
        assert!(addr1 + layout.size() <= addr3 || addr3 + layout.size() <= addr1);

        alloc.dealloc(p1, layout);
        alloc.dealloc(p2, layout);
        alloc.dealloc(p3, layout);
    }

    #[test]
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

    #[test]
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

    // -----------------------------------------------------------------------
    // Specific RISC-V Instruction Alignment Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_riscv_compressed_instruction_alignment() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 2).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 2, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_riscv_standard_instruction_alignment() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(100, 4).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 4, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_riscv_float_double_alignment() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::new::<f64>();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 8, 0);
        alloc.dealloc(ptr, layout);
    }

    #[test]
    fn test_riscv_vector_alignment() {
        let mut alloc = make_alloc();
        let layout = core::alloc::Layout::from_size_align(256, 32).unwrap();
        let ptr = alloc.alloc(layout);
        assert_eq!(ptr as usize % 32, 0);
        alloc.dealloc(ptr, layout);
    }
}
