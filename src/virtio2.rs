use virtio_drivers::device::blk::VirtIOBlk;
use virtio_drivers::device::console::VirtIOConsole;
use virtio_drivers::transport::Transport;
use virtio_drivers::transport::mmio::{MmioError, MmioTransport, VirtIOHeader};
use virtio_drivers::{BufferDirection, Hal, PAGE_SIZE, PhysAddr};

extern crate alloc;
use alloc::alloc::{Layout, alloc_zeroed, dealloc};

use crate::device_tree;

use crate::filesystem::BlockIndex;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, Ordering};
use spin::{Mutex, Once};

static CONSOLE: Mutex<Option<VirtIOConsole<DeviceAllocator, MmioTransport<'static>>>> =
    Mutex::new(None);
static CONSOLE_INIT: Once<()> = Once::new();
static BLOCK_DEVICE_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Initialise the VirtIO console device exposed by the device tree.
/// Must be called after the allocator is initialised.
pub fn init_console() {
    CONSOLE_INIT.call_once(init_console_once);
}

fn init_console_once() {
    use virtio_drivers::transport::DeviceType;

    // The exact DT resources preserve the fact that every candidate implements the VirtIO MMIO
    // register interface. Generic page-table MMIO ranges cannot provide that guarantee because
    // they are page-covered and may be merged with adjacent devices such as the UART.
    for region in device_tree::system_virtio_mmio_regions() {
        if region.start() % core::mem::align_of::<VirtIOHeader>() != 0 {
            log::warn!("Unaligned DT-declared VirtIO MMIO transport: {region:?}");
            continue;
        }

        let header = NonNull::new(region.start() as *mut VirtIOHeader)
            .expect("a VirtIO MMIO resource cannot start at the null address");

        // SAFETY: `region` is the exact `reg` resource of an enabled `virtio,mmio` DT node. It is
        // directly accessible during early boot and remains identity-mapped after paging is
        // enabled. This initialization path has exclusive access to each candidate, and a retained
        // transport is never probed again here.
        let transport = match unsafe { MmioTransport::new(header, region.size()) } {
            Ok(transport) => transport,
            // Device ID zero denotes an unused VirtIO MMIO transport slot. The driver crate also
            // uses this error for device IDs it does not yet recognize; neither is a console.
            Err(MmioError::InvalidDeviceID(_)) => continue,
            Err(error) => {
                log::warn!("Invalid DT-declared VirtIO MMIO transport at {region:?}: {error}");
                continue;
            }
        };

        if transport.device_type() == DeviceType::Console
            && let Ok(console) = VirtIOConsole::<DeviceAllocator, MmioTransport>::new(transport)
        {
            *CONSOLE.lock() = Some(console);
            crate::klog::flush_early_buffer();
            return;
        }
    }

    panic!("Could not find console");
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
        let region = device_tree::block_device_region();
        assert_eq!(
            region.start() % core::mem::align_of::<VirtIOHeader>(),
            0,
            "unaligned VirtIO block-device MMIO resource: {region:?}",
        );
        let header = NonNull::new(region.start() as *mut VirtIOHeader)
            .expect("a VirtIO MMIO resource cannot start at the null address");

        // SAFETY: `region` is the exact resource of the block transport discovered from an enabled
        // `virtio,mmio` DT node. It remains identity-mapped for the lifetime of the device, and no
        // other live transport or raw MMIO access aliases this register window.
        let transport = unsafe { MmioTransport::new(header, region.size()) }
            .unwrap_or_else(|e| panic!("Error creating VirtIO MMIO transport: {e}"));

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
    assert!(
        !BLOCK_DEVICE_CLAIMED.swap(true, Ordering::AcqRel),
        "the VirtIO block device has already been claimed",
    );
    LockedBlockDevice::new()
}

// TODO(mt): we have virtual memory now. What do we need to change here.
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
