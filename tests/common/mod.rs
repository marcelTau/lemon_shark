use lemon_shark::allocator::HeapBounds;
use lemon_shark::kernel_layout::KernelLayout;
use spin::Once;

static KERNEL_LAYOUT: Once<KernelLayout> = Once::new();

pub fn init_kernel_layout() -> KernelLayout {
    *KERNEL_LAYOUT.call_once(|| unsafe { KernelLayout::from_labels() })
}

pub fn kernel_layout() -> KernelLayout {
    *KERNEL_LAYOUT
        .get()
        .expect("kernel layout must be initialized in _start before running tests")
}

pub fn heap_bounds() -> HeapBounds {
    HeapBounds::new(kernel_layout())
}
