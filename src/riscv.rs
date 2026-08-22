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

#[derive(Debug, PartialEq)]
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
    LoadAccessFault,
    AmoAddressMisaligned,
    StoreAmoAccessFault,
    EnvironmentCall,

    // TODO(mt): when looking into semihosting again, 0x3f is the code for a
    // semihost operation in qemu: https://github.com/qemu/qemu/blob/master/target/riscv/cpu_bits.h#L785
    //
    // Don't know if this is useful as with the latest try, we could not manage
    // to make qemu read the ebreak call as openSBI only reads things in
    // m-mode and we capture the breakpoint exception in s-mode.
    Reserved,
}

/// https://people.eecs.berkeley.edu/~krste/papers/riscv-privileged-v1.9.1.pdf
/// Section 4.1.8 (Supervisor Cause Register)
#[repr(transparent)]
pub struct Scause(usize);

impl Scause {
    pub fn is_interrupt(&self) -> bool {
        (self.0 & (1 << (usize::BITS - 1))) != 0
    }

    pub fn reason(&self) -> ScauseReason {
        if self.is_interrupt() {
            // unset the interrupt bit
            match self.0 & 0x7FFFFFFFFFFFFFFF {
                0 => ScauseReason::UserSoftwareInterrupt,
                1 => ScauseReason::SupervisorSoftwareInterrupt,
                4 => ScauseReason::UserTimerInterrupt,
                5 => ScauseReason::SupervisorTimerInterrupt,
                8 => ScauseReason::UserExternalInterrupt,
                9 => ScauseReason::SupervisorExternalInterrupt,
                2 | 3 | 6 | 7 | 10.. => ScauseReason::Reserved,
            }
        } else {
            match self.0 {
                0 => ScauseReason::InstructionAddressMisaligned,
                1 => ScauseReason::InstructionAccessFault,
                2 => ScauseReason::IllegalInstruction,
                3 => ScauseReason::Breakpoint,
                5 => ScauseReason::LoadAccessFault,
                6 => ScauseReason::AmoAddressMisaligned,
                7 => ScauseReason::StoreAmoAccessFault,
                8 => ScauseReason::EnvironmentCall,
                4 | 9.. => ScauseReason::Reserved,
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
    pub fn scause() -> Scause {
        let value: usize;

        unsafe {
            core::arch::asm!("csrr {}, scause", out(reg) value);
        }

        Scause(value)
    }

    /// Writes the `satp` register with the given value, followed by a `sfence.vma` to flush the TLB.
    pub fn write_satp_and_flush_tlb(satp: Satp) {
        unsafe {
            core::arch::asm!(
                "csrw satp, {satp}",
                "sfence.vma",
                satp = in(reg) satp.as_usize()
            );
        }
    }

    pub fn write_stvec(val: Stvec) {
        unsafe {
            core::arch::asm!("csrw stvec, {}", in(reg) val.0);
        }
    }

    pub fn rdtime() -> usize {
        let time: usize;
        unsafe { core::arch::asm!("rdtime {}", out(reg) time) }
        time
    }
}
