//! This module provides types for specific registers and concepts that require
//! some documentation and are easy to misuse if not handled carefully.

/// A `Asid` or Address Space IDentifier is used in the TLB to identify the address space of a
/// process. It is a part of the `satp` register. On risc-v, the ASID is limited to 16 bits.
///
/// The `Asid` is used to avoid flushing the TLB on context switches.
///
/// TODO(mt): Currently it's not used as we only have the kernel itself.
pub struct Asid(u16);

impl Asid {
    pub const KERNEL: Self = Self(0);

    pub fn new(val: u16) -> Self {
        Self(val)
    }

    pub fn as_u16(&self) -> u16 {
        self.0
    }
}

impl From<Asid> for usize {
    fn from(asid: Asid) -> Self {
        asid.0 as usize
    }
}

// https://www.scs.stanford.edu/~zyedidia/docs/riscv/riscv-privileged.pdf Section: 4.1.1
pub mod sstatus {
    pub const SPP: usize = 1 << 8;
    pub const SPIE: usize = 1 << 5;
    pub const SIE: usize = 1 << 1;
}

// TODO(mt): also read about other CSR's (here)[https://people.eecs.berkeley.edu/~krste/papers/riscv-privileged-v1.9.1.pdf] Section 2.2

/// Supervisor Interrupt Enabled
pub struct Sie(usize);

impl Sie {}

pub enum SatpMode {
    Bare = 0,
    Sv39 = 8,
    Sv48 = 9,
    Sv57 = 10,
}

/// The `satp` (Supervisor Address Translation and Protection) register controls
/// the address translation for the current process.
///
/// It consists of the mode, ASID, and PPN fields.
///
/// 63   60 59          44 43                               0
/// +------+--------------+---------------------------------+
/// | mode | ASID         | PPN of page table               |
/// +------+--------------+---------------------------------+
///
/// https://www.scs.stanford.edu/~zyedidia/docs/riscv/riscv-privileged.pdf Section 4.1.11
pub struct Satp(usize);

impl Satp {
    pub fn new(mode: SatpMode, asid: Asid, ppn: usize) -> Self {
        Self(((mode as usize) << 60) | (usize::from(asid) << 44) | (ppn >> 12))
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScauseReason {
    // interrupts
    UserSoftwareInterrupt,
    SupervisorSoftwareInterrupt,
    UserTimerInterrupt,
    SupervisorTimerInterrupt,
    UserExternalInterrupt,
    SupervisorExternalInterrupt,

    // exceptions
    InstructionAddressMisaligned,
    InstructionAccessFault,
    IllegalInstruction,
    Breakpoint,
    LoadAddressMisaligned,
    LoadAccessFault,
    StoreAmoAddressMisaligned,
    StoreAmoAccessFault,
    EnvironmentCallFromUserMode,
    EnvironmentCallFromSupervisorMode,
    InstructionPageFault,
    LoadPageFault,
    StoreAmoPageFault,

    // TODO(mt): when looking into semihosting again, 0x3f is the code for a
    // semihost operation in qemu: https://github.com/qemu/qemu/blob/master/target/riscv/cpu_bits.h#L785
    //
    // Don't know if this is useful as with the latest try, we could not manage
    // to make qemu read the ebreak call as openSBI only reads things in
    // m-mode and we capture the breakpoint exception in s-mode.
    Reserved(usize),
}

/// https://people.eecs.berkeley.edu/~krste/papers/riscv-privileged-v1.9.1.pdf
/// Section 4.1.8 (Supervisor Cause Register)
#[repr(transparent)]
pub struct Scause(usize);

impl Scause {
    pub const fn raw(&self) -> usize {
        self.0
    }

    pub const fn is_interrupt(&self) -> bool {
        (self.0 & (1 << (usize::BITS - 1))) != 0
    }

    pub const fn code(&self) -> usize {
        self.0 & (usize::MAX >> 1)
    }

    pub fn reason(&self) -> ScauseReason {
        if self.is_interrupt() {
            match self.code() {
                0 => ScauseReason::UserSoftwareInterrupt,
                1 => ScauseReason::SupervisorSoftwareInterrupt,
                4 => ScauseReason::UserTimerInterrupt,
                5 => ScauseReason::SupervisorTimerInterrupt,
                8 => ScauseReason::UserExternalInterrupt,
                9 => ScauseReason::SupervisorExternalInterrupt,
                code => ScauseReason::Reserved(code),
            }
        } else {
            match self.code() {
                0 => ScauseReason::InstructionAddressMisaligned,
                1 => ScauseReason::InstructionAccessFault,
                2 => ScauseReason::IllegalInstruction,
                3 => ScauseReason::Breakpoint,
                4 => ScauseReason::LoadAddressMisaligned,
                5 => ScauseReason::LoadAccessFault,
                6 => ScauseReason::StoreAmoAddressMisaligned,
                7 => ScauseReason::StoreAmoAccessFault,
                8 => ScauseReason::EnvironmentCallFromUserMode,
                9 => ScauseReason::EnvironmentCallFromSupervisorMode,
                12 => ScauseReason::InstructionPageFault,
                13 => ScauseReason::LoadPageFault,
                15 => ScauseReason::StoreAmoPageFault,
                code => ScauseReason::Reserved(code),
            }
        }
    }
}

impl core::fmt::Debug for Scause {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.reason())
    }
}

/// The `stvec` register holds the address of the trap handler.
///
/// The address has to be aligned to 4 bytes, so the lower 2 bits are always
/// zero.
pub struct Stvec(usize);

impl Stvec {
    pub fn new(addr: usize) -> Self {
        Self(addr & !0b11)
    }
}

/// Contains common assembly-level operations for risc-v to avoid inline assembly.
pub mod asm {
    use super::{Satp, Scause, Stvec};

    /// Reads the current scause value from the CSR.
    #[inline(always)]
    pub fn scause() -> Scause {
        let value: usize;

        unsafe {
            core::arch::asm!("csrr {}, scause", out(reg) value);
        }

        Scause(value)
    }

    /// Reads supervisor trap value (`stval`).
    ///
    /// Its meaning depends on the trap cause. For address and page faults it
    /// normally contains the virtual address involved in the fault; for traps
    /// without additional information it is zero.
    #[inline(always)]
    pub fn stval() -> usize {
        let value: usize;

        unsafe {
            core::arch::asm!("csrr {}, stval", out(reg) value, options(nomem, nostack));
        }

        value
    }

    #[inline(always)]
    pub fn sscratch() -> usize {
        let value: usize;

        unsafe {
            core::arch::asm!("csrr {}, sscratch", out(reg) value, options(nomem, nostack));
        }

        value
    }

    /// Disable supervisor interrupts and put this hart into an idle loop.
    pub fn halt() -> ! {
        unsafe {
            core::arch::asm!("csrci sstatus, 0x2", options(nomem, nostack));
        }

        loop {
            unsafe {
                core::arch::asm!("wfi", options(nomem, nostack));
            }
        }
    }

    /// Synchronizes page-table writes and invalidates all cached translations on this hart.
    ///
    /// Call after each operation that modifies the active page table (adding, removing,
    /// or changing mappings or permissions), before relying on the updated mappings.
    /// An operation may update several entries before calling this once. Even adding
    /// a previously absent mapping requires synchronization: invalid entries can be cached.
    ///
    /// This executes `sfence.vma` for all virtual addresses and address spaces on the
    /// current hart only; other harts using the page table need their own fence.
    #[inline(always)]
    pub fn flush_tlb() {
        unsafe {
            // Keep the default memory effects so page-table writes cannot move past the fence.
            core::arch::asm!("sfence.vma", options(nostack));
        }
    }

    /// Writes the `satp` register with the given value, followed by a `sfence.vma` to flush the TLB.
    #[inline(always)]
    pub fn write_satp_and_flush_tlb(satp: Satp) {
        unsafe {
            core::arch::asm!(
                "csrw satp, {satp}",
                satp = in(reg) satp.as_usize()
            );
        }
        flush_tlb();
    }

    #[inline(always)]
    pub fn write_stvec(val: Stvec) {
        unsafe {
            core::arch::asm!("csrw stvec, {}", in(reg) val.0);
        }
    }

    #[inline(always)]
    pub fn rdtime() -> usize {
        let time: usize;
        unsafe { core::arch::asm!("rdtime {}", out(reg) time) }
        time
    }

    /// https://www.scs.stanford.edu/~zyedidia/docs/riscv/riscv-privileged.pdf Section 4.1.3
    pub mod sie {
        #[inline(always)]
        pub fn enable_timer_interrupt() {
            unsafe { core::arch::asm!("csrs sie, {}", in(reg) 1 << 5) };
        }

        #[inline(always)]
        pub fn enable_external_interrupt() {
            unsafe { core::arch::asm!("csrs sie, {}", in(reg) 1 << 9) };
        }

        /// Checks whether the supervisor software, timer, and external interrupt
        /// enable bits are implemented without changing the interrupt state seen
        /// by the caller.
        ///
        /// Global supervisor interrupts are disabled while probing so a pending
        /// interrupt cannot be delivered while the temporary `sie` bits are set.
        /// Both `sie` and `sstatus` are restored before this function returns.
        ///
        /// # Reference
        ///
        /// https://www.scs.stanford.edu/~zyedidia/docs/riscv/riscv-privileged.pdf Section 4.1.3
        pub fn probe() {
            const SSTATUS_SIE: usize = 1 << 1;
            const SSIE: usize = 1 << 1;
            const STIE: usize = 1 << 5;
            const SEIE: usize = 1 << 9;
            const PROBED_INTERRUPTS: usize = SSIE | STIE | SEIE;

            let _previous_sstatus: usize;
            let _previous_sie: usize;
            let implemented: usize;

            unsafe {
                core::arch::asm!(
                    "csrrc {previous_sstatus}, sstatus, {sstatus_sie}",
                    "csrrs {previous_sie}, sie, {probed_interrupts}",
                    "csrr {implemented}, sie",
                    "csrw sie, {previous_sie}",
                    "csrw sstatus, {previous_sstatus}",
                    previous_sstatus = out(reg) _previous_sstatus,
                    previous_sie = out(reg) _previous_sie,
                    implemented = out(reg) implemented,
                    sstatus_sie = in(reg) SSTATUS_SIE,
                    probed_interrupts = in(reg) PROBED_INTERRUPTS,
                    options(nomem, nostack),
                );
            }

            let missing = PROBED_INTERRUPTS & !implemented;

            if missing & SSIE != 0 {
                log::error!("Unimplemented Interrupt: SSIE (Supervisor Software Interrupt)");
            }

            if missing & STIE != 0 {
                log::error!("Unimplemented Interrupt: STIE (Supervisor Timer Interrupts)");
            }

            if missing & SEIE != 0 {
                log::error!("Unimplemented Interrupt: SEIE (Supervisor External Interrupts)");
            }
        }
    }
}
