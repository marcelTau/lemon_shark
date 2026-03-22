use virtio_drivers::device::blk::VirtIOBlk;
use virtio_drivers::device::console::VirtIOConsole;
use virtio_drivers::transport::Transport;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::{BufferDirection, Hal, PAGE_SIZE, PhysAddr};

extern crate alloc;
use alloc::alloc::{Layout, alloc_zeroed, dealloc};

use crate::filesystem::BlockIndex;
use core::ptr::NonNull;
use spin::Mutex;

static CONSOLE: Mutex<Option<VirtIOConsole<DeviceAllocator, MmioTransport<'static>>>> =
    Mutex::new(None);

/// Initialise the VirtIO console device at MMIO slot 0x10007000.
/// Must be called after the allocator is initialised.
pub fn init_console() {
    // Scan all 8 virtio MMIO slots to find the console device.
    // Slots are at 0x10001000..=0x10008000 in 0x1000 increments.
    use virtio_drivers::transport::DeviceType;
    for slot in (0x10001000usize..=0x10008000).step_by(0x1000) {
        let header = NonNull::new(slot as *mut VirtIOHeader).unwrap();
        let transport = match unsafe { MmioTransport::new(header, 0x1000) } {
            Ok(t) => t,
            Err(_) => continue,
        };
        if transport.device_type() == DeviceType::Console {
            if let Ok(console) = VirtIOConsole::<DeviceAllocator, MmioTransport>::new(transport) {
                *CONSOLE.lock() = Some(console);
                crate::klog::flush_early_buffer();
                return;
            }
        }
    }
}

/// Write bytes to the VirtIO console log channel.
/// Returns false if the console is not yet initialised.
pub fn console_write(bytes: &[u8]) -> bool {
    let mut guard = CONSOLE.lock();
    match guard.as_mut() {
        Some(console) => {
            let _ = console.send_bytes(bytes);
            true
        }
        None => false,
    }
}

pub struct LockedBlockDevice<'a> {
    disk: VirtIOBlk<DeviceAllocator, MmioTransport<'a>>,
}

impl LockedBlockDevice<'_> {
    fn new() -> Self {
        let header = NonNull::new(0x10008000 as *mut VirtIOHeader).unwrap();

        let transport = unsafe { MmioTransport::new(header, 0x1000) }
            .unwrap_or_else(|e| panic!("Error creating VirtIO MMIO transport: {}", e));

        Self {
            disk: VirtIOBlk::<DeviceAllocator, MmioTransport>::new(transport).unwrap(),
        }
    }

    pub(crate) fn read_block(&mut self, block_idx: BlockIndex, buf: &mut [u8]) {
        self.disk
            .read_blocks(block_idx.inner() as usize, buf)
            .unwrap();
    }

    pub(crate) fn write_block(&mut self, block_idx: BlockIndex, data: &[u8]) {
        self.disk
            .write_blocks(block_idx.inner() as usize, data)
            .unwrap();
    }

    pub(crate) fn total_blocks(&mut self) -> usize {
        self.disk.capacity() as usize
    }
}

pub fn make_device() -> LockedBlockDevice<'static> {
    LockedBlockDevice::new()
}

/// Simple allocator for the device. This is very simple as we're not having
/// virtual memory yet so most functions don't do much.
pub struct DeviceAllocator;

unsafe impl Hal for DeviceAllocator {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let layout = Layout::from_size_align(PAGE_SIZE * pages, PAGE_SIZE).unwrap();
        let ptr = unsafe { alloc_zeroed(layout) };

        let addr = ptr as u64;
        let start = NonNull::new(addr as _).unwrap();

        (addr, start)
    }

    unsafe fn dma_dealloc(paddr: PhysAddr, _vaddr: NonNull<u8>, pages: usize) -> i32 {
        let layout = Layout::from_size_align(PAGE_SIZE * pages, PAGE_SIZE).unwrap();
        unsafe { dealloc(paddr as *mut u8, layout) };
        0
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        NonNull::new(paddr as _).unwrap()
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        buffer.as_ptr() as *mut u8 as u64
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {}
}
