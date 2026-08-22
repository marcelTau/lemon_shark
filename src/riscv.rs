//! This module provides types for specific registers and concepts that require
//! some documentation and are easy to misuse if not handled carefully.

use core::arch::asm;

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

/// Writes the `satp` register with the given value, followed by a `sfence.vma` to flush the TLB.
pub fn write_satp_and_flush_tlb(satp: Satp) {
    unsafe {
        asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp.as_usize()
        );
    }
}
