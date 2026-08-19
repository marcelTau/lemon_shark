/// This abstraction reads the addresses of all used linker-inserted labels that the kernel needs
/// for orientation once in the beginning instead of having unsafe functions that assume that those
/// labels exist down the line. This also makes testing a lot easier.
#[derive(Copy, Clone, Debug)]
pub struct KernelLayout {
    pub(crate) kernel_start: usize,
    pub(crate) kernel_end: usize,
    pub(crate) heap_start: usize,
    pub(crate) heap_end: usize,
    pub(crate) trap_stack_top: usize,
}

impl KernelLayout {
    /// Creates a `KernelLayout` by reading labels defined in the linker script.
    ///
    /// # Safety
    ///
    /// Requires all used labels to be defined and properly aligned. See `linker.ld`.
    pub unsafe fn from_labels() -> Self {
        unsafe extern "C" {
            static _kernel_start: u8;
            static _kernel_end: u8;
            static _heap_start: u8;
            static _heap_end: u8;
            static _trap_stack_top: u8;
        }

        Self {
            kernel_start: core::ptr::addr_of!(_kernel_start) as usize,
            kernel_end: core::ptr::addr_of!(_kernel_end) as usize,
            heap_start: core::ptr::addr_of!(_heap_start) as usize,
            heap_end: core::ptr::addr_of!(_heap_end) as usize,
            trap_stack_top: core::ptr::addr_of!(_trap_stack_top) as usize,
        }
    }
}
