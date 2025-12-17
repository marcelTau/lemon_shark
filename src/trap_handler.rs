use core::arch::asm;

use crate::log;

#[derive(Debug)]
enum ScauseReason {
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

    Reserved,
}

/// https://people.eecs.berkeley.edu/~krste/papers/riscv-privileged-v1.9.1.pdf
/// Section 4.1.8 (Supervisor Cause Register)
#[repr(transparent)]
struct Scause(usize);

impl Scause {
    fn is_interrupt(&self) -> bool {
        (self.0 & (1 << usize::BITS - 1)) != 0
    }

    fn reason(&self) -> ScauseReason {
        if self.is_interrupt() {
            match self.0 >> 1_usize {
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

#[inline(never)]
#[unsafe(no_mangle)]
extern "C" fn trap_handler() -> ! {
    let sepc: usize;
    let scause: usize;

    unsafe {
        asm!("csrr {0}, scause", out(reg) scause);
        asm!("csrr {0}, sepc", out(reg) sepc);
    };

    let scause = Scause(scause);
    log!("Trap Handler at {:#0x}\n", sepc);
    log!("\tScause = {scause:?}\n");

    loop {}
}

/// Initializes the trap handler by writing the address of the `trap_handler` to the `stvec`
/// register.
pub(crate) fn init() {
    let write_addr = (trap_handler as usize) & !0b11;
    unsafe {
        asm!("csrw stvec, {}", in(reg) write_addr);
    };

    let read_addr: usize;

    unsafe {
        asm!("csrr {}, stvec", out(reg) read_addr);
    };

    // sanity check
    assert_eq!(write_addr, read_addr);

    log!("Trap Handler initialized at {:#0x}\n", write_addr);
}
