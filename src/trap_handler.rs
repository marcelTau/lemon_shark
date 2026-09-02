use core::arch::asm;
use core::arch::naked_asm;

use crate::kernel_layout::KernelLayout;
use crate::riscv;
use crate::riscv::Scause;
use crate::riscv::ScauseReason;
use crate::riscv::Stvec;
use crate::timer;

/// Saved state of a process at the point it was interrupted.
///
/// Layout is fixed (`#[repr(C)]`) because the trap entry assembly addresses
/// fields by hardcoded byte offsets. `kernel_sp` must remain at offset 0 —
/// it is loaded first to switch to the kernel trap stack before any Rust code runs.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct TrapFrame {
    pub kernel_sp: usize, // offset 0   — loaded first to switch stacks
    pub ra: usize,        // offset 8
    pub sp: usize,        // offset 16
    pub gp: usize,        // offset 24
    pub tp: usize,        // offset 32
    pub t0: usize,        // offset 40
    pub t1: usize,        // offset 48
    pub t2: usize,        // offset 56
    pub s0: usize,        // offset 64
    pub s1: usize,        // offset 72
    pub a0: usize,        // offset 80
    pub a1: usize,        // offset 88
    pub a2: usize,        // offset 96
    pub a3: usize,        // offset 104
    pub a4: usize,        // offset 112
    pub a5: usize,        // offset 120
    pub a6: usize,        // offset 128
    pub a7: usize,        // offset 136
    pub s2: usize,        // offset 144
    pub s3: usize,        // offset 152
    pub s4: usize,        // offset 160
    pub s5: usize,        // offset 168
    pub s6: usize,        // offset 176
    pub s7: usize,        // offset 184
    pub s8: usize,        // offset 192
    pub s9: usize,        // offset 200
    pub s10: usize,       // offset 208
    pub s11: usize,       // offset 216
    pub t3: usize,        // offset 224
    pub t4: usize,        // offset 232
    pub t5: usize,        // offset 240
    pub t6: usize,        // offset 248
    pub sepc: usize,      // offset 256
    pub sstatus: usize,   // offset 264
}

// Keep the Rust structure layout synchronized with the hardcoded offsets in
// `trap_handler`. A mismatch here would make the assembly silently save or
// restore a register through the wrong field.
const _: () = {
    assert!(core::mem::align_of::<TrapFrame>() == 8);
    assert!(core::mem::size_of::<TrapFrame>() == 272);

    assert!(core::mem::offset_of!(TrapFrame, kernel_sp) == 0);
    assert!(core::mem::offset_of!(TrapFrame, ra) == 8);
    assert!(core::mem::offset_of!(TrapFrame, sp) == 16);
    assert!(core::mem::offset_of!(TrapFrame, gp) == 24);
    assert!(core::mem::offset_of!(TrapFrame, tp) == 32);
    assert!(core::mem::offset_of!(TrapFrame, t0) == 40);
    assert!(core::mem::offset_of!(TrapFrame, t1) == 48);
    assert!(core::mem::offset_of!(TrapFrame, t2) == 56);
    assert!(core::mem::offset_of!(TrapFrame, s0) == 64);
    assert!(core::mem::offset_of!(TrapFrame, s1) == 72);
    assert!(core::mem::offset_of!(TrapFrame, a0) == 80);
    assert!(core::mem::offset_of!(TrapFrame, a1) == 88);
    assert!(core::mem::offset_of!(TrapFrame, a2) == 96);
    assert!(core::mem::offset_of!(TrapFrame, a3) == 104);
    assert!(core::mem::offset_of!(TrapFrame, a4) == 112);
    assert!(core::mem::offset_of!(TrapFrame, a5) == 120);
    assert!(core::mem::offset_of!(TrapFrame, a6) == 128);
    assert!(core::mem::offset_of!(TrapFrame, a7) == 136);
    assert!(core::mem::offset_of!(TrapFrame, s2) == 144);
    assert!(core::mem::offset_of!(TrapFrame, s3) == 152);
    assert!(core::mem::offset_of!(TrapFrame, s4) == 160);
    assert!(core::mem::offset_of!(TrapFrame, s5) == 168);
    assert!(core::mem::offset_of!(TrapFrame, s6) == 176);
    assert!(core::mem::offset_of!(TrapFrame, s7) == 184);
    assert!(core::mem::offset_of!(TrapFrame, s8) == 192);
    assert!(core::mem::offset_of!(TrapFrame, s9) == 200);
    assert!(core::mem::offset_of!(TrapFrame, s10) == 208);
    assert!(core::mem::offset_of!(TrapFrame, s11) == 216);
    assert!(core::mem::offset_of!(TrapFrame, t3) == 224);
    assert!(core::mem::offset_of!(TrapFrame, t4) == 232);
    assert!(core::mem::offset_of!(TrapFrame, t5) == 240);
    assert!(core::mem::offset_of!(TrapFrame, t6) == 248);
    assert!(core::mem::offset_of!(TrapFrame, sepc) == 256);
    assert!(core::mem::offset_of!(TrapFrame, sstatus) == 264);
};

impl TrapFrame {
    pub const fn zero() -> Self {
        Self {
            kernel_sp: 0,
            ra: 0,
            sp: 0,
            gp: 0,
            tp: 0,
            t0: 0,
            t1: 0,
            t2: 0,
            s0: 0,
            s1: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            a3: 0,
            a4: 0,
            a5: 0,
            a6: 0,
            a7: 0,
            s2: 0,
            s3: 0,
            s4: 0,
            s5: 0,
            s6: 0,
            s7: 0,
            s8: 0,
            s9: 0,
            s10: 0,
            s11: 0,
            t3: 0,
            t4: 0,
            t5: 0,
            t6: 0,
            sepc: 0,
            sstatus: 0,
        }
    }
}

/// `TrapFrame` for the initial kernel context (before any processes exist).
/// `kernel_sp` is filled in by `init()`.
static mut INITIAL_TRAP_FRAME: TrapFrame = TrapFrame::zero();

/// Naked wrapper around the `trap handler` to save all registers into the
/// current process's `TrapFrame` and switch to the kernel trap stack.
///
/// `sscratch` points to the current process's `TrapFrame`. We swap it with
/// `a0` to get the frame pointer, save all 31 GP registers plus `sepc` and
/// `sstatus` into it, then load `kernel_sp` from the frame (offset 0) to switch
/// stacks before calling the Rust handler.
///
/// On exit, `sscratch` is read again — the scheduler may have updated it to
/// a different process's frame during a context switch — and all registers
/// are restored from that frame before `sret`.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn trap_handler() -> ! {
    naked_asm!(
        // a0 = &TrapFrame, sscratch = original a0
        "csrrw a0, sscratch, a0",

        // save all GP registers except a0 (handled below after recovering from sscratch)
        "sd ra,   8(a0)",
        "sd sp,   16(a0)",
        "sd gp,   24(a0)",
        "sd tp,   32(a0)",
        "sd t0,   40(a0)",
        "sd t1,   48(a0)",
        "sd t2,   56(a0)",
        "sd s0,   64(a0)",
        "sd s1,   72(a0)",
        // a0 at offset 80 — saved after recovering original value from sscratch
        "sd a1,   88(a0)",
        "sd a2,   96(a0)",
        "sd a3,   104(a0)",
        "sd a4,   112(a0)",
        "sd a5,   120(a0)",
        "sd a6,   128(a0)",
        "sd a7,   136(a0)",
        "sd s2,   144(a0)",
        "sd s3,   152(a0)",
        "sd s4,   160(a0)",
        "sd s5,   168(a0)",
        "sd s6,   176(a0)",
        "sd s7,   184(a0)",
        "sd s8,   192(a0)",
        "sd s9,   200(a0)",
        "sd s10,  208(a0)",
        "sd s11,  216(a0)",
        "sd t3,   224(a0)",
        "sd t4,   232(a0)",
        "sd t5,   240(a0)",
        "sd t6,   248(a0)",

        // save sepc (t0 is already saved above, safe to clobber)
        "csrr t0, sepc",
        "sd t0,   256(a0)",

        // Save the trap-time privilege and interrupt state. In particular,
        // sret uses SPP to select the return privilege and SPIE to restore SIE.
        "csrr t0, sstatus",
        "sd t0,   264(a0)",

        // recover original a0 (stashed in sscratch) and save it
        "csrr t0, sscratch",
        "sd t0,   80(a0)",

        // restore sscratch = &TrapFrame so the Rust handler and scheduler can find it
        "csrw sscratch, a0",

        // switch to the kernel trap stack
        "ld sp, 0(a0)",

        // call Rust handler — a0 = &TrapFrame is already the first argument
        "call {trap_handler_rust}",

        // reload TrapFrame pointer
        "csrr a0, sscratch",

        // Restore the selected context's control state before t0 is overwritten.
        // The scheduler may have changed sscratch to point at another frame.
        "ld t0,   256(a0)",
        "csrw sepc, t0",
        "ld t0,   264(a0)",
        "csrw sstatus, t0",

        // restore all GP registers except sp and a0
        "ld ra,   8(a0)",
        "ld gp,   24(a0)",
        "ld tp,   32(a0)",
        "ld t0,   40(a0)",
        "ld t1,   48(a0)",
        "ld t2,   56(a0)",
        "ld s0,   64(a0)",
        "ld s1,   72(a0)",
        "ld a1,   88(a0)",
        "ld a2,   96(a0)",
        "ld a3,   104(a0)",
        "ld a4,   112(a0)",
        "ld a5,   120(a0)",
        "ld a6,   128(a0)",
        "ld a7,   136(a0)",
        "ld s2,   144(a0)",
        "ld s3,   152(a0)",
        "ld s4,   160(a0)",
        "ld s5,   168(a0)",
        "ld s6,   176(a0)",
        "ld s7,   184(a0)",
        "ld s8,   192(a0)",
        "ld s9,   200(a0)",
        "ld s10,  208(a0)",
        "ld s11,  216(a0)",
        "ld t3,   224(a0)",
        "ld t4,   232(a0)",
        "ld t5,   240(a0)",
        "ld t6,   248(a0)",

        // restore sp - kernel stack unreachable after this
        "ld sp,   16(a0)",
        // restore a0 - TrapFrame unreachable after this
        "ld a0,   80(a0)",

        "sret", // TODO(mt): `sret` also does some modifications on `sstatus` to enable interrupts
                // again.

        trap_handler_rust = sym trap_handler_rust,
    );
}

#[unsafe(no_mangle)]
extern "C" fn trap_handler_rust(frame: *mut TrapFrame) {
    let scause = riscv::asm::scause();
    let stval = riscv::asm::stval();
    let frame = unsafe { &mut *frame };

    match scause.reason() {
        ScauseReason::SupervisorTimerInterrupt => {
            timer::handle_interrupt();
        }
        ScauseReason::Breakpoint => {
            let sepc = frame.sepc;
            // Determine ebreak size and skip it
            let ebreak_size = {
                let first_halfword = unsafe { (sepc as *const u16).read() };
                // RISC-V instructions longer than 16 bits have their lowest two bits set
                // to 0b11; 16-bit compressed instructions do not. Breakpoints therefore
                // use 2 << 0 = 2 bytes for `c.ebreak`, or 2 << 1 = 4 bytes for `ebreak`.
                2 << ((first_halfword & 0b11 == 0b11) as usize)
            };
            frame.sepc = sepc + ebreak_size;
        }
        _ => fatal_unhandled_trap(&scause, frame, stval),
    }
}

/// Report an unhandled trap without returning to the faulting instruction.
///
/// At present every runnable context belongs to the kernel, so an unhandled
/// trap is fatal. Once U-mode tasks exist, traps whose saved `SPP` bit is clear
/// should terminate only the current task instead of halting the whole system.
fn fatal_unhandled_trap(scause: &Scause, frame: &TrapFrame, stval: usize) -> ! {
    const SSTATUS_SPP: usize = 1 << 8;

    let origin = if frame.sstatus & SSTATUS_SPP != 0 {
        "supervisor"
    } else {
        "user"
    };

    // Use the direct UART console rather than the kernel logger here. A fault
    // may have interrupted code while it held the VirtIO logging lock.
    crate::println!("\n[FATAL] Unhandled trap from {origin} mode");
    crate::println!("  scause = {:#018x} ({:?})", scause.raw(), scause.reason());
    crate::println!("  sepc   = {:#018x}", frame.sepc);
    crate::println!("  stval  = {stval:#018x}");

    riscv::asm::halt()
}

/// Sets up the initial `TrapFrame` for the kernel context and points
/// `sscratch` at it. `kernel_sp` is set to `_trap_stack_top` — the same
/// known-good stack used before, now stored in the frame instead of directly
/// in `sscratch`.
fn setup_trap_frame(layout: KernelLayout) {
    unsafe {
        INITIAL_TRAP_FRAME.kernel_sp = layout.trap_stack_top;
        let frame_addr = &raw const INITIAL_TRAP_FRAME as usize;
        asm!("csrw sscratch, {}", in(reg) frame_addr);
    }
}

/// Set the trap handler by writing the address of `trap_handler` to the
/// `stvec` register with the lower 2 bits masked off.
fn setup_trap_handler() {
    let trap_handler_addr = trap_handler as *const () as usize;
    let stvec = Stvec::new(trap_handler_addr);

    riscv::asm::write_stvec(stvec);
}

/// Initializes the trap handler by writing the address of the `trap_handler` to the `stvec`
/// register.
pub fn init(layout: KernelLayout) {
    setup_trap_frame(layout);
    setup_trap_handler();

    log::info!("initialized");
}
