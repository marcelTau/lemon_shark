//! PLIC - Platform Level Interrupt Controller
//! docs: (https://github.com/riscv/riscv-plic-spec/blob/master/riscv-plic.adoc)
//!
//! It's task is to notify a hart (CPU) if there is an interrupt available.
//!
//! Terminology:
//! Source: The source of an external interrupt, i.e a UART driver.
//! Context: A combination of (hart_id, mode) i.e (hart_id=0, mode=S).
//! Interrupt Identifier: An small integer assigned to each interrupt source. Also defined in the
//! device tree, i.e the `interrupts` field of the serial node. It's `interrupt-parent` links to
//! the PLIC node. 0 means no-interrupt.
//!
//! The `PLIC` works by having a static memory map layout at a dynamic offset. The offset is read
//! via the device tree.
//!
//! 1. Get hart id from device tree and pass it, also we're using S-mode as this kernel runs in
//!    S-mode. I think this is context=1
//!
//! 2. Make sure we enable interrupts globally in the `sstatus` and `sie` register.
//!
//! 3. Enable the UART source interupts by setting the `enabled` bit for this source.
//!
//! 4. Give the UART interrupts a priority and make sure it works with the assigned thresshold for
//!    this context.
use crate::device_tree;
use crate::device_tree::{PlicContext, PlicInfo};
use crate::uart;
use virtual_memory::PhysRange;

#[derive(Debug)]
pub enum Error {
    InvlidSource,
    InvalidAddress,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SourceId(usize);

impl SourceId {
    pub fn new(source: usize) -> Self {
        Self(source)
    }

    pub fn inner(self) -> usize {
        self.0
    }
}

static PLIC: spin::Once<Plic> = spin::Once::new();

struct Plic {
    region: PhysRange,
    num_sources: usize,
    context: PlicContext,
}

impl Plic {
    fn new(info: PlicInfo, context: PlicContext) -> Self {
        Self {
            region: info.mmio_region,
            num_sources: info.num_sources,
            context,
        }
    }

    /// Validates a `SourceId` for the current configuration.
    /// Source ID 0 is reserved and not valid in this case.
    fn validate_source_id(&self, source: usize) -> Result<SourceId, Error> {
        if source == 0 || source > self.num_sources {
            Err(Error::InvlidSource)
        } else {
            Ok(SourceId(source))
        }
    }

    /// Validates an address for the current configuration.
    ///
    /// It checks that the address is inside the MMIO region and aligned to 4 byte as all PLIC
    /// accesses are u32. It also checks, that the whole range of bytes for this u32 are valid.
    fn validate_address(&self, addr: usize) -> Result<*mut u32, Error> {
        if !self.region.contains(addr) || !self.region.contains(addr + 3) || !addr.is_multiple_of(4)
        {
            return Err(Error::InvalidAddress);
        }

        Ok(addr as *mut u32)
    }

    /// Enables interupts for `source` for the current context.
    ///
    /// offset = base + 0x2000 + 0x80 * context + 4 * (source / 32)
    fn enable(&self, source: SourceId, context: PlicContext) -> Result<(), Error> {
        let addr =
            self.region.start() + 0x2000 + 0x80 * context.context_index + 4 * (source.inner() / 32);
        let addr = self.validate_address(addr)?;

        let bit = source.inner() % 32;

        unsafe {
            let val = core::ptr::read_volatile(addr);
            core::ptr::write_volatile(addr, val | (1 << bit));
        }

        Ok(())
    }

    /// Sets the priority for the `source` to `prio`.
    ///
    /// offset = base + 0 + source_id * 4
    fn set_priority(&self, source: SourceId, prio: u32) -> Result<(), Error> {
        let addr = self.region.start() + source.inner() * 4;
        let addr = self.validate_address(addr)?;

        unsafe {
            core::ptr::write_volatile(addr, prio);
        }

        Ok(())
    }

    /// base + 0x200000 + 0x1000 * context
    fn set_threshold(&self, context: PlicContext, threshold: u32) -> Result<(), Error> {
        let addr = self.region.start() + 0x200000 + 0x1000 * context.context_index;
        let addr = self.validate_address(addr)?;

        unsafe {
            core::ptr::write_volatile(addr, threshold);
        }

        Ok(())
    }

    /// base + 0x200004 + 0x1000 * context
    fn claim(&self, context: PlicContext) -> Result<Option<SourceId>, Error> {
        let addr = self.region.start() + 0x200004 + 0x1000 * context.context_index;
        let addr = self.validate_address(addr)?;

        let value: u32 = unsafe { core::ptr::read_volatile(addr) };

        if value == 0 {
            Ok(None)
        } else {
            self.validate_source_id(value as usize).map(Some)
        }
    }

    fn claim_current(&self) -> Result<Option<SourceId>, Error> {
        self.claim(self.context)
    }

    /// base + 0x200004 + 0x1000 * context
    fn complete(&self, context: PlicContext, source: SourceId) -> Result<(), Error> {
        let addr = self.region.start() + 0x200004 + 0x1000 * context.context_index;
        let addr = self.validate_address(addr)?;

        unsafe { core::ptr::write_volatile(addr, source.inner() as u32) };

        Ok(())
    }

    fn complete_current(&self, source: SourceId) -> Result<(), Error> {
        self.complete(self.context, source)
    }
}

fn plic() -> &'static Plic {
    PLIC.get().expect("PLIC accessed before initialized")
}

fn claim_current() -> Option<SourceId> {
    plic().claim_current().unwrap()
}

fn complete_current(source: SourceId) {
    plic().complete_current(source).unwrap()
}

/// The handler for all external interrupts at the moment.
///
/// TODO(mt): Might be worth storing the `SourceId` locally here instead of querying them from all
/// the modules later on as they will all be separate atmoic accesses.
pub fn handle_external_interrupt() {
    let Some(source) = claim_current() else {
        return;
    };

    // The interrupt is for the UART device.
    if source == uart::interrupt_source() {
        uart::handle_rx_interrupt();
    }

    complete_current(source);
}

/// Initializes the PLIC and configures it to enable interrupts for the UART.
pub fn init() -> Result<(), Error> {
    let info = device_tree::plic().clone();
    let uart = device_tree::console_uart();

    // TODO(mt): will need to change when we have multiple CPUs. This assumes, that the first one is
    // the one we're currently booting on.
    let context = info.supervisor_contexts[0];

    PLIC.call_once(|| Plic::new(info, context));

    let source = plic().validate_source_id(uart.interrupt_id)?;

    plic().enable(source, context)?;
    plic().set_priority(source, 5)?;
    plic().set_threshold(context, 1)?;

    log::info!("initialized");

    Ok(())
}
