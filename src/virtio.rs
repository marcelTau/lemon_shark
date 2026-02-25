//! Virtio 1.1 spec: https://docs.oasis-open.org/virtio/virtio/v1.1/cs01/virtio-v1.1-cs01.html#x1-1460002
//!
//! Terminology:
//! Device - The virtual hardware (i.e the virtio block device)
//! Driver - The kernel code that controls the Device
//! Guest  - This kernel running in QEMU
//! Host   - The device inside of QEMU 

#![allow(unused)]

use crate::{logln, println};

// virtio_mmio@10008000 {
//     interrupts = <0x8>
//     interrupt-parent = <0x3>
//     reg = <0x10008000 0x1000>
//     compatible = "virtio,mmio"
// };


/// Choosen Queue size has to be power of 2 and <= QUEUE_MAX_SIZE;
const QUEUE_SIZE: usize = 64;

/// Each virtqueue must be aligned on a 4096 boundary.
const QUEUE_ALIGN_BYTES: usize = 4096;

/// Offsets defined by section 4.2.4
mod mmiodevice_legacy_register_layout {
    // Currently hard-coded from the device-tree information.
    pub(crate) const BASE: usize = 0x10008000;
    pub(crate) const EXPECTED_MAGIC: u32 = 0x74726976;

    pub(crate) const MAGIC: usize = 0x000;
    pub(crate) const VERSION: usize = 0x004;
    pub(crate) const DEVICE_ID: usize = 0x008;
    pub(crate) const VENDOR_ID: usize = 0x00c;
    pub(crate) const HOST_FEATURES: usize = 0x010;
    pub(crate) const HOST_FEATURES_SEL: usize = 0x014;
    pub(crate) const GUEST_FEATURES: usize = 0x020;
    pub(crate) const GUEST_FEATURES_SEL: usize = 0x024;
    pub(crate) const GUEST_PAGE_SIZE: usize = 0x028;
    pub(crate) const QUEUE_SEL: usize = 0x030;
    pub(crate) const QUEUE_NUM_MAX: usize = 0x034;
    pub(crate) const QUEUE_NUM: usize = 0x038;
    pub(crate) const QUEUE_ALIGN: usize = 0x03c;
    pub(crate) const QUEUE_PFN: usize = 0x040;
    pub(crate) const QUEUE_NOTIFY: usize = 0x050;
    pub(crate) const INTERRUPT_STATUS: usize = 0x060;
    pub(crate) const INTERRUPT_ACK: usize = 0x064;
    pub(crate) const STATUS: usize = 0x070;
    pub(crate) const CONFIG: usize = 0x100;  // Device-specific config starts here
}

use mmiodevice_legacy_register_layout::*;

fn read_volatile(offset: usize) -> u32 {
    unsafe {
        core::ptr::read_volatile((BASE + offset) as *const u32)
    }
}

fn write_volatile(offset: usize, value: u32) {
    unsafe {
        core::ptr::write_volatile((BASE + offset) as *mut u32, value);
    }
}

fn read_config() {
    unsafe {
        let size_max = core::ptr::read_volatile((BASE + 0x8) as *const u32);
        println!("size_max={}", size_max);
    }
}

const fn align_up(x: usize, a: usize) -> usize { (x + a - 1) & !(a - 1) }

/// The total number of bytes used for the `VirtQ`. Following section 2.6.2.
///
/// +------------------+---------------------------+----------+
/// | descriptor table | Avaiable Ring (+ padding) | Used Ring|
/// +------------------+---------------------------+----------+
///
/// Available Ring: write-only for driver
/// Used Ring: read-only for driver
///
/// VirtQueueSize = 16 byte parameter
///
/// This has to be allocated in contiguous memory.
const QUEUE_BYTES: usize = {
    let descriptor = 16 * QUEUE_SIZE;
    let available = 6  + 2 * QUEUE_SIZE;
    let used = 6 + 8 * QUEUE_SIZE;

    let used_offset = align_up(descriptor + available, QUEUE_ALIGN_BYTES);
    used_offset + used
};

#[repr(align(4096))]
struct QBuf { data: [u8; QUEUE_BYTES] }

static mut VIRTQ: QBuf = QBuf { data: [0u8; QUEUE_BYTES] };

/// Device initialization sequence: Section 3.1
/// Device status fields: Section 2.1
fn initialize_driver() -> bool {
    // 1. Reset the device
    write_volatile(STATUS, 0x0);

    // 2/3. Set the ACK & DRIVER status bits
    write_volatile(STATUS, 0x1 | 0x2);

    // 4. Read device feature bits

    // What the driver offers
    let host_features = read_volatile(HOST_FEATURES);
    println!("Found host features: {host_features:0b} {host_features:#0x}");

    // Don't accept any extra features for now.
    let guest_features = 0;
    write_volatile(GUEST_FEATURES, 0);

    // Setup the page size.
    write_volatile(GUEST_PAGE_SIZE, 4096);

    true
}

/// The virtual queue is configured as follows:
///
/// 1. Select the queue writing its index (first queue is 0) to QueueSel.
///
/// 2. Check if the queue is not already in use: read QueuePFN, expecting a
/// returned value of zero (0x0).
///
/// 3. Read maximum queue size (number of elements) from QueueNumMax. If the
/// returned value is zero (0x0) the queue is not available.
///
/// 4. Allocate and zero the queue pages in contiguous virtual memory, aligning
/// the Used Ring to an optimal boundary (usually page size). The driver
/// should choose a queue size smaller than or equal to QueueNumMax.
///
/// 5. Notify the device about the queue size by writing the size to QueueNum.
///
/// 6. Notify the device about the used alignment by writing its value in bytes
/// to QueueAlign.
///
/// 7. Write the physical number of the first page of the queue to the QueuePFN
/// register.
fn setup_virtqueue() {

    // 1. Select queue 0
    write_volatile(QUEUE_SEL, 0);

    // 2. Check that the queue is ready.
    let ready = read_volatile(QUEUE_PFN);

    if ready != 0 {
        println!("Queue is not ready");
        return;
    }

    // 3. Read `QueueNumMax` = 1024
    let queue_num_max = read_volatile(QUEUE_NUM_MAX);

    if queue_num_max == 0 {
        println!("Queue is not available");
        return;
    }

    if QUEUE_SIZE as u32 > queue_num_max {
        panic!("Wrong queue_size expected {QUEUE_SIZE} <= {queue_num_max}");
    }

    // 4. Zero out the static memory.

    let base = unsafe { core::ptr::addr_of_mut!(VIRTQ.data) as usize};

    // 5. Write `QUEUE_SIZE` to `QUEUE_NUM`
    write_volatile(QUEUE_NUM, QUEUE_SIZE as u32);

    // 6. Write `QUEUE_ALIGN`
    write_volatile(QUEUE_ALIGN, QUEUE_ALIGN_BYTES as u32);

    // 7. Write physical page number to `QUEUE_PFN`
    write_volatile(QUEUE_PFN, (base / QUEUE_ALIGN_BYTES) as u32);
       
}

// We need to use this: 2.6 Split Virtqueues (legacy interface)

/// The spec says that in the legacy format there can be a race condition around
/// the `capacity`. In order to avoid that we should read multiple times until
/// we get consisten results.
///
/// Section 2.4.4
///
/// The `capacity` is returned as the number of 512-byte sectors.
fn read_capacity() -> u64 {
    let mut prev = 0;
    for _ in 0..10 {
        let new = unsafe { core::ptr::read_volatile((BASE + CONFIG) as *const u64) };

        if new == prev {
            return new;
        } else {
            prev = new;
        }
    }

    panic!("Could not read capacity");
}

#[repr(u32)]
enum DeviceOperation {
    In = 0,
    Out = 1,
    Flush = 4,
    Discard = 11,
    WriteZeros = 13,
}

struct Request {
    op: DeviceOperation,
    reserved: u32,
    /// Offset multiplied by 512 where read or write occur. Set to 0 for any
    /// operation other than `Read` or `Write`.
    sector: u64,
    data: [u8; 512],
    status: u8, // 0=Ok, 1=Error, 2=Unsupported
}

pub fn init() {
    // Validate magic & version
    assert_eq!(read_volatile(MAGIC), EXPECTED_MAGIC);
    assert_eq!(read_volatile(VERSION), 0x1);

    // Section 4.2.3.1.1 - if device_id == 0, we're not allowed to read any
    // other registers and need to abort the initilization.
    if read_volatile(DEVICE_ID) == 0 {
        println!("[VIRTIO] DeviceID zero. Aborting.");
        return;
    }

    let capacity = read_capacity();

    println!("[VIRTIO] Found disk with Capacity: {}MB", capacity * 512 / 1024 / 1024);

    let max_queue_size = read_volatile(QUEUE_NUM_MAX);
    println!("[VIRTIO] Max queue size: {}", max_queue_size);

    read_config();

    if !initialize_driver() {
        return;
    }

    setup_virtqueue();


    let status = read_volatile(STATUS);
    
    write_volatile(STATUS, status | 0x4);


    let interrupt_status = read_volatile(INTERRUPT_STATUS);

    assert!(interrupt_status == 0);

    println!("[VIRTIO] initialized");
}

