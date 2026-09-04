//! https://opensocdebug.readthedocs.io/en/latest/02_spec/07_modules/dem_uart/uartspec.html
//! https://osblog.stephenmarz.com/ch2.html - # Register Chart

use crate::{device_tree, interrupts, plic::SourceId, ring_buffer::RingBuffer};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Bit 7 Controls DLAB. Other bits have no meaning.
const LCR: usize = 3;

/// Bitmask to check the DLAB bit. DLAB is a control flag that changes the meaning of registers for
/// the UART device.
const DLAB: u8 = 1 << 7;

const INTERRUPT_ENABLE: usize = 1;

/// The `SourceId` of the UART device so it can be queried from the external interrupt handler later
/// on.
static UART_INTERRUPT_SOURCE: AtomicUsize = AtomicUsize::new(0);

/// Initializes the UART device and enables its interrupts.
pub fn init() {
    let uart = device_tree::console_uart();
    let base = uart.mmio_region.start();

    // Relaxed is fine here. This is not guarding anything and interrupts are not enabled yet.
    UART_INTERRUPT_SOURCE.store(uart.interrupt_id, Ordering::Relaxed);

    // Calculate MMIO addresses.
    let interrupt_enable_addr = base + INTERRUPT_ENABLE;
    let line_control_addr = base + LCR;

    // Ensure they're valid.
    assert!(uart.mmio_region.contains(interrupt_enable_addr));
    assert!(uart.mmio_region.contains(line_control_addr));

    unsafe {
        let line_control = line_control_addr as *mut u8;
        let current_line_control = core::ptr::read_volatile(line_control);

        // Ensure the DLAB bit is turned off as it controls the meaning of registers.
        core::ptr::write_volatile(line_control, current_line_control & !DLAB);

        // Enable interrupts for this device.
        let interrupt_enable = interrupt_enable_addr as *mut u8;
        let current_interrupt_enable = interrupt_enable.read_volatile();

        // Enable interrupts by setting the first bit.
        interrupt_enable.write_volatile(current_interrupt_enable | 1);
    }
}

/// Returns the `SourceId` of the configured UART device which is read from the device tree. We
/// store it in an Atomic as this is the fasted / easiest way of storing a global without a lock
/// as this function is called in the interrupt handler.
pub fn interrupt_source() -> SourceId {
    SourceId::new(UART_INTERRUPT_SOURCE.load(Ordering::Relaxed))
}

// TODO(mt): lock-free?

static RX_RING: spin::Mutex<RingBuffer<256>> = spin::Mutex::new(RingBuffer::new());
static DROPPED: AtomicUsize = AtomicUsize::new(0);

fn drain_into_ring_buffer(bytes: &[u8]) {
    let mut ring = RX_RING.lock();

    for b in bytes {
        if ring.push(*b).is_err() {
            DROPPED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
    }
}

const MAX_BYTES_PER_INTERRUPT: usize = 64;

/// This function is called from the interrupt handler to deal with an interrupt for the UART
/// device. It reads up to `MAX_BYTES_PER_INTERRUPT` bytes from the device and pushes them
/// into the ring buffer. Right now the shell reads from that RingBuffer.
pub fn handle_rx_interrupt() {
    let base = device_tree::console_uart().mmio_region.start();

    const LINE_STATUS: usize = 0x5;
    const DATA_READY: u8 = 0x1;

    let mut bytes = [0u8; MAX_BYTES_PER_INTERRUPT];
    let mut next_write = 0;

    let mut push = |byte: u8| {
        bytes[next_write] = byte;
        next_write += 1;
    };

    for _ in 0..MAX_BYTES_PER_INTERRUPT {
        let status_reg = (base + LINE_STATUS) as *mut u8;
        let status: u8 = unsafe { core::ptr::read_volatile(status_reg) };

        if status & DATA_READY == 0 {
            break;
        }

        let byte = unsafe { core::ptr::read_volatile((base) as *mut u8) };
        push(byte);
    }

    drain_into_ring_buffer(&bytes[0..next_write])
}

/// Only to be called from the shell at the moment.
///
/// # Panics
///
/// Panics if supervisor interrupts are not enabled. In particular, do not call this from an
/// interrupt handler or inside a `without_interrupts` block.
pub fn read_byte_blocking() -> u8 {
    interrupts::wait_until(|| RX_RING.lock().pop())
}
