use virtio_drivers::device::blk::VirtIOBlk;
use virtio_drivers::transport::mmio::{MmioTransport, VirtIOHeader};
use virtio_drivers::transport::Transport;
use virtio_drivers::{BufferDirection, Hal, PhysAddr, PAGE_SIZE};

extern crate alloc;
use alloc::alloc::{alloc_zeroed, dealloc, Layout};

use crate::filesystem::BlockIndex;
use crate::println;
use core::ptr::NonNull;

pub struct LockedBlockDevice<'a> {
    disk: VirtIOBlk<DeviceAllocator, MmioTransport<'a>>,
}

impl LockedBlockDevice<'_> {
    fn new() -> Self {
        let header = NonNull::new(0x10008000 as *mut VirtIOHeader).unwrap();
        // let transport = unsafe { MmioTransport::new(header, 0x1000) }.unwrap();

        let transport = match unsafe { MmioTransport::new(header, 0x1000) } {
            Err(e) => panic!("Error creating VirtIO MMIO transport: {}", e),
            Ok(transport) => {
                println!(
                    "Detected virtio MMIO device with vendor id {:#X}, device type {:?}, version {:?}",
                    transport.vendor_id(),
                    transport.device_type(),
                    transport.version(),
                );
                transport
            }
        };

        println!("Found header: {transport:?}");

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
