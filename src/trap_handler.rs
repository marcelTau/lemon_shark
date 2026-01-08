use crate::{logln, println};
use core::arch::asm;
use core::arch::naked_asm;
use core::sync::atomic::{AtomicBool, Ordering};

/// Atomic that indicates that there was a `TRAP`.
pub static TRAP: AtomicBool = AtomicBool::new(false);

#[derive(Debug, PartialEq)]
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
struct Scause(usize);

impl Scause {
    fn is_interrupt(&self) -> bool {
        (self.0 & (1 << (usize::BITS - 1))) != 0
    }

    fn reason(&self) -> ScauseReason {
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

/// Naked wrapper around the `trap handler` to swap out the stack with a known good stack
/// that has been set in the `sscratch` register.
///
/// For that to work, we need to store all registers that might be clobbered on the stack
/// before calling the rust trap handler and restore them afterwards.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn trap_handler() -> ! {
    naked_asm!(
           // Swap `sp` and `sscratch` atomically
           "csrrw sp, sscratch, sp",

           // Allocate stack frame for all registers we need to save
           // We need to save: ra, a0-a7, t0-t6 = 1 + 8 + 7 = 16 registers = 128 bytes
           "addi sp, sp, -128",

           // Save all caller-saved registers that might be clobbered
           "sd ra, 0(sp)",
           "sd t0, 8(sp)",
           "sd t1, 16(sp)",
           "sd t2, 24(sp)",
           "sd t3, 32(sp)",
           "sd t4, 40(sp)",
           "sd t5, 48(sp)",
           "sd t6, 56(sp)",
           "sd a0, 64(sp)",
           "sd a1, 72(sp)",
           "sd a2, 80(sp)",
           "sd a3, 88(sp)",
           "sd a4, 96(sp)",
           "sd a5, 104(sp)",
           "sd a6, 112(sp)",
           "sd a7, 120(sp)",

           // Call the rust code
           "call {trap_handler_rust}",

           "ld ra, 0(sp)",
           "ld t0, 8(sp)",
           "ld t1, 16(sp)",
           "ld t2, 24(sp)",
           "ld t3, 32(sp)",
           "ld t4, 40(sp)",
           "ld t5, 48(sp)",
           "ld t6, 56(sp)",
           "ld a1, 72(sp)",
           "ld a2, 80(sp)",
           "ld a3, 88(sp)",
           "ld a4, 96(sp)",
           "ld a5, 104(sp)",
           "ld a6, 112(sp)",
           "ld a7, 120(sp)",

           // Deallocate stack frame
           "addi sp, sp, 128",

           // Swap back the stacks
           "csrrw sp, sscratch, sp",

           // Return from trap handler
           "sret",

           trap_handler_rust = sym trap_handler_rust,
    );
}

#[unsafe(no_mangle)]
extern "C" fn trap_handler_rust() {
    let sepc: usize;
    let scause: usize;

    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, sepc", out(reg) sepc);
    };

    let scause = Scause(scause);

    println!("TRAP at {sepc:#0x?} ({scause:?})");

    panic!();

    match scause.reason() {
        ScauseReason::SupervisorTimerInterrupt => {
            crate::timer::new_time(1000000000);

            // Setting the TRAP to true if it's currently false, not changing it if it's true.
            // Don't care about the result here.
            let _ = TRAP.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
        }
        ScauseReason::Breakpoint => {
            // Determine ebreak size and skip it
            let ebreak_size = unsafe {
                let instr_ptr = sepc as *const u16;
                let first_halfword = instr_ptr.read();
                if (first_halfword & 0b11) != 0b11 {
                    2
                } else {
                    4
                }
            };

            unsafe {
                asm!("csrw sepc, {}", in(reg) sepc + ebreak_size);
            }
        }
        _ => {}
    }
}

/// Initializes the trap handler by writing the address of the `trap_handler` to the `stvec`
/// register.
pub fn init() {
    let trap_handler_addr = (trap_handler as *const () as usize) & !0b11;
    unsafe {
        asm!("csrw stvec, {}", in(reg) trap_handler_addr);
    };

    logln!("Trap Handler initialized at {trap_handler_addr:#0x}");
}
