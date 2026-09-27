#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(lemon_shark::test_runner)]
#![reexport_test_harness_main = "test_main"]

mod common;

use core::arch::{asm, global_asm};
use core::time::Duration;

use lemon_shark::trap_handler::{self, TrapFrame};
use lemon_shark::{ALLOCATOR, device_tree, riscv, timer};

global_asm!(
    ".section .text.boot",
    ".global _boot",
    "_boot:",
    "   la sp, _stack_top",
    "   call _start",
);

const SSTATUS_SIE: usize = 1 << 1;
const SSTATUS_SPIE: usize = 1 << 5;
const SSTATUS_SPP: usize = 1 << 8;
const PROBE_COMPLETED: usize = 0x600d;

const RA_SENTINEL: usize = 0x101;
const T0_SENTINEL: usize = 0x500;
const T1_SENTINEL: usize = 0x501;
const T2_SENTINEL: usize = 0x502;
const S0_SENTINEL: usize = 0x800;
const S1_SENTINEL: usize = 0x801;
const A0_SENTINEL: usize = 0xa00;
const A1_SENTINEL: usize = 0xa01;
const A2_SENTINEL: usize = 0xa02;
const A3_SENTINEL: usize = 0xa03;
const A4_SENTINEL: usize = 0xa04;
const A5_SENTINEL: usize = 0xa05;
const A6_SENTINEL: usize = 0xa06;
const A7_SENTINEL: usize = 0xa07;
const S2_SENTINEL: usize = 0x802;
const S3_SENTINEL: usize = 0x803;
const S4_SENTINEL: usize = 0x804;
const S5_SENTINEL: usize = 0x805;
const S6_SENTINEL: usize = 0x806;
const S7_SENTINEL: usize = 0x807;
const S8_SENTINEL: usize = 0x808;
const S9_SENTINEL: usize = 0x809;
const S10_SENTINEL: usize = 0x80a;
const S11_SENTINEL: usize = 0x80b;
const T3_SENTINEL: usize = 0x503;
const T4_SENTINEL: usize = 0x504;
const T5_SENTINEL: usize = 0x505;
const T6_SENTINEL: usize = 0x506;

/// Register state captured by the assembly probe immediately after `sret`.
///
/// The probe temporarily uses this object as its interrupted stack. Trap entry
/// must therefore switch to the dedicated trap stack before calling Rust.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct ProbeState {
    ra: usize,
    sp: usize,
    gp: usize,
    tp: usize,
    t0: usize,
    t1: usize,
    t2: usize,
    s0: usize,
    s1: usize,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
    a7: usize,
    s2: usize,
    s3: usize,
    s4: usize,
    s5: usize,
    s6: usize,
    s7: usize,
    s8: usize,
    s9: usize,
    s10: usize,
    s11: usize,
    t3: usize,
    t4: usize,
    t5: usize,
    t6: usize,
    sstatus: usize,
    completed: usize,
}

impl ProbeState {
    const fn zero() -> Self {
        Self {
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
            sstatus: 0,
            completed: 0,
        }
    }

    fn registers(&self) -> [usize; 31] {
        [
            self.ra, self.sp, self.gp, self.tp, self.t0, self.t1, self.t2, self.s0, self.s1,
            self.a0, self.a1, self.a2, self.a3, self.a4, self.a5, self.a6, self.a7, self.s2,
            self.s3, self.s4, self.s5, self.s6, self.s7, self.s8, self.s9, self.s10, self.s11,
            self.t3, self.t4, self.t5, self.t6,
        ]
    }
}

const _: () = {
    assert!(core::mem::align_of::<ProbeState>() == 16);
    assert!(core::mem::size_of::<ProbeState>() == 272);
    assert!(core::mem::offset_of!(ProbeState, ra) == 0);
    assert!(core::mem::offset_of!(ProbeState, sp) == 8);
    assert!(core::mem::offset_of!(ProbeState, gp) == 16);
    assert!(core::mem::offset_of!(ProbeState, tp) == 24);
    assert!(core::mem::offset_of!(ProbeState, t0) == 32);
    assert!(core::mem::offset_of!(ProbeState, t1) == 40);
    assert!(core::mem::offset_of!(ProbeState, t2) == 48);
    assert!(core::mem::offset_of!(ProbeState, s0) == 56);
    assert!(core::mem::offset_of!(ProbeState, s1) == 64);
    assert!(core::mem::offset_of!(ProbeState, a0) == 72);
    assert!(core::mem::offset_of!(ProbeState, a1) == 80);
    assert!(core::mem::offset_of!(ProbeState, a2) == 88);
    assert!(core::mem::offset_of!(ProbeState, a3) == 96);
    assert!(core::mem::offset_of!(ProbeState, a4) == 104);
    assert!(core::mem::offset_of!(ProbeState, a5) == 112);
    assert!(core::mem::offset_of!(ProbeState, a6) == 120);
    assert!(core::mem::offset_of!(ProbeState, a7) == 128);
    assert!(core::mem::offset_of!(ProbeState, s2) == 136);
    assert!(core::mem::offset_of!(ProbeState, s3) == 144);
    assert!(core::mem::offset_of!(ProbeState, s4) == 152);
    assert!(core::mem::offset_of!(ProbeState, s5) == 160);
    assert!(core::mem::offset_of!(ProbeState, s6) == 168);
    assert!(core::mem::offset_of!(ProbeState, s7) == 176);
    assert!(core::mem::offset_of!(ProbeState, s8) == 184);
    assert!(core::mem::offset_of!(ProbeState, s9) == 192);
    assert!(core::mem::offset_of!(ProbeState, s10) == 200);
    assert!(core::mem::offset_of!(ProbeState, s11) == 208);
    assert!(core::mem::offset_of!(ProbeState, t3) == 216);
    assert!(core::mem::offset_of!(ProbeState, t4) == 224);
    assert!(core::mem::offset_of!(ProbeState, t5) == 232);
    assert!(core::mem::offset_of!(ProbeState, t6) == 240);
    assert!(core::mem::offset_of!(ProbeState, sstatus) == 248);
    assert!(core::mem::offset_of!(ProbeState, completed) == 256);
};

// These mutable statics are safe under this suite's explicit single-hart,
// non-nested test contract. The assembly probe addresses them by symbol name.
#[unsafe(no_mangle)]
static mut TRAP_PROBE_ORIGINALS: ProbeState = ProbeState::zero();

#[unsafe(no_mangle)]
static mut TRAP_PROBE_OBSERVED: ProbeState = ProbeState::zero();

global_asm!(
    r#"
    .section .text
    .align 2

    .macro save_originals
        la t0, TRAP_PROBE_ORIGINALS
        sd ra,   0(t0)
        sd sp,   8(t0)
        sd gp,   16(t0)
        sd tp,   24(t0)
        sd s0,   56(t0)
        sd s1,   64(t0)
        sd s2,   136(t0)
        sd s3,   144(t0)
        sd s4,   152(t0)
        sd s5,   160(t0)
        sd s6,   168(t0)
        sd s7,   176(t0)
        sd s8,   184(t0)
        sd s9,   192(t0)
        sd s10,  200(t0)
        sd s11,  208(t0)
        csrr t1, sstatus
        sd t1,   248(t0)
    .endm

    .macro load_sentinels
        li ra,   0x101
        li t0,   0x500
        li t1,   0x501
        li t2,   0x502
        li s0,   0x800
        li s1,   0x801
        li a0,   0xa00
        li a1,   0xa01
        li a2,   0xa02
        li a3,   0xa03
        li a4,   0xa04
        li a5,   0xa05
        li a6,   0xa06
        li a7,   0xa07
        li s2,   0x802
        li s3,   0x803
        li s4,   0x804
        li s5,   0x805
        li s6,   0x806
        li s7,   0x807
        li s8,   0x808
        li s9,   0x809
        li s10,  0x80a
        li s11,  0x80b
        li t3,   0x503
        li t4,   0x504
        li t5,   0x505
        li t6,   0x506
        la sp, TRAP_PROBE_OBSERVED
    .endm

    .macro capture_observed
        sd ra,   0(sp)
        sd sp,   8(sp)
        sd gp,   16(sp)
        sd tp,   24(sp)
        sd t0,   32(sp)
        sd t1,   40(sp)
        sd t2,   48(sp)
        sd s0,   56(sp)
        sd s1,   64(sp)
        sd a0,   72(sp)
        sd a1,   80(sp)
        sd a2,   88(sp)
        sd a3,   96(sp)
        sd a4,   104(sp)
        sd a5,   112(sp)
        sd a6,   120(sp)
        sd a7,   128(sp)
        sd s2,   136(sp)
        sd s3,   144(sp)
        sd s4,   152(sp)
        sd s5,   160(sp)
        sd s6,   168(sp)
        sd s7,   176(sp)
        sd s8,   184(sp)
        sd s9,   192(sp)
        sd s10,  200(sp)
        sd s11,  208(sp)
        sd t3,   216(sp)
        sd t4,   224(sp)
        sd t5,   232(sp)
        sd t6,   240(sp)
        csrr t0, sstatus
        sd t0,   248(sp)
        li t0,   0x600d
        sd t0,   256(sp)
    .endm

    .macro restore_originals
        la t0, TRAP_PROBE_ORIGINALS
        ld t1,   248(t0)
        csrw sstatus, t1
        ld gp,   16(t0)
        ld tp,   24(t0)
        ld s0,   56(t0)
        ld s1,   64(t0)
        ld s2,   136(t0)
        ld s3,   144(t0)
        ld s4,   152(t0)
        ld s5,   160(t0)
        ld s6,   168(t0)
        ld s7,   176(t0)
        ld s8,   184(t0)
        ld s9,   192(t0)
        ld s10,  200(t0)
        ld s11,  208(t0)
        ld sp,   8(t0)
        ld ra,   0(t0)
    .endm

    .global trap_probe_ebreak
    .type trap_probe_ebreak, @function
trap_probe_ebreak:
    save_originals
    csrci sstatus, 0x2
    load_sentinels
    .option push
    .option norvc
    ebreak
    .option pop
    capture_observed
    restore_originals
    ret
    .size trap_probe_ebreak, . - trap_probe_ebreak

    .global trap_probe_compressed_ebreak
    .type trap_probe_compressed_ebreak, @function
trap_probe_compressed_ebreak:
    save_originals
    csrsi sstatus, 0x2
    load_sentinels
    .option push
    .option rvc
    c.ebreak
    .option pop
    capture_observed
    restore_originals
    ret
    .size trap_probe_compressed_ebreak, . - trap_probe_compressed_ebreak
"#,
);

unsafe extern "C" {
    fn trap_probe_ebreak();
    fn trap_probe_compressed_ebreak();
}

#[unsafe(no_mangle)]
pub extern "C" fn _start(_: usize, fdt_addr: usize) -> ! {
    let layout = common::init_kernel_layout();
    trap_handler::init(layout);
    unsafe { ALLOCATOR.init(layout) };
    device_tree::init(fdt_addr).expect("failed to initialize device tree for trap tests");
    timer::init(device_tree::timer_frequency());

    test_main();
    riscv_halt()
}

fn riscv_halt() -> ! {
    loop {
        unsafe { asm!("wfi", options(nomem, nostack)) };
    }
}

fn current_trap_frame_ptr() -> *const TrapFrame {
    let frame: usize;
    unsafe { asm!("csrr {}, sscratch", out(reg) frame, options(nomem, nostack)) };
    frame as *const TrapFrame
}

fn reset_probe_state() {
    // The suite is single-hart and interrupts remain disabled outside the
    // compressed-breakpoint probe, so no concurrent observer can access these.
    unsafe {
        (&raw mut TRAP_PROBE_ORIGINALS).write_volatile(ProbeState::zero());
        (&raw mut TRAP_PROBE_OBSERVED).write_volatile(ProbeState::zero());
    }
}

fn read_probe_state() -> (ProbeState, ProbeState) {
    unsafe {
        (
            (&raw const TRAP_PROBE_ORIGINALS).read_volatile(),
            (&raw const TRAP_PROBE_OBSERVED).read_volatile(),
        )
    }
}

fn trap_frame_registers(frame: &TrapFrame) -> [usize; 31] {
    [
        frame.ra, frame.sp, frame.gp, frame.tp, frame.t0, frame.t1, frame.t2, frame.s0, frame.s1,
        frame.a0, frame.a1, frame.a2, frame.a3, frame.a4, frame.a5, frame.a6, frame.a7, frame.s2,
        frame.s3, frame.s4, frame.s5, frame.s6, frame.s7, frame.s8, frame.s9, frame.s10, frame.s11,
        frame.t3, frame.t4, frame.t5, frame.t6,
    ]
}

fn expected_registers(originals: &ProbeState) -> [usize; 31] {
    let probe_sp = (&raw const TRAP_PROBE_OBSERVED) as usize;

    [
        RA_SENTINEL,
        probe_sp,
        originals.gp,
        originals.tp,
        T0_SENTINEL,
        T1_SENTINEL,
        T2_SENTINEL,
        S0_SENTINEL,
        S1_SENTINEL,
        A0_SENTINEL,
        A1_SENTINEL,
        A2_SENTINEL,
        A3_SENTINEL,
        A4_SENTINEL,
        A5_SENTINEL,
        A6_SENTINEL,
        A7_SENTINEL,
        S2_SENTINEL,
        S3_SENTINEL,
        S4_SENTINEL,
        S5_SENTINEL,
        S6_SENTINEL,
        S7_SENTINEL,
        S8_SENTINEL,
        S9_SENTINEL,
        S10_SENTINEL,
        S11_SENTINEL,
        T3_SENTINEL,
        T4_SENTINEL,
        T5_SENTINEL,
        T6_SENTINEL,
    ]
}

fn run_probe(probe: unsafe extern "C" fn(), entry_sie_enabled: bool) {
    reset_probe_state();
    let frame_before = current_trap_frame_ptr();

    unsafe { probe() };

    let frame_after = current_trap_frame_ptr();
    assert_eq!(frame_after, frame_before, "trap return changed sscratch");

    let (originals, observed) = read_probe_state();
    let frame = unsafe { frame_after.read_volatile() };
    let expected = expected_registers(&originals);

    assert_eq!(observed.completed, PROBE_COMPLETED, "probe did not resume");
    assert_eq!(
        trap_frame_registers(&frame),
        expected,
        "trap entry saved incorrect register state"
    );
    assert_eq!(
        observed.registers(),
        expected,
        "trap return restored incorrect register state"
    );

    assert_eq!(
        frame.sstatus & SSTATUS_SIE,
        0,
        "hardware must disable SIE during trap handling"
    );
    assert_eq!(
        frame.sstatus & SSTATUS_SPIE != 0,
        entry_sie_enabled,
        "trap entry did not preserve the previous SIE state in SPIE"
    );
    assert_ne!(
        frame.sstatus & SSTATUS_SPP,
        0,
        "the probe should have trapped from supervisor mode"
    );
    assert_eq!(
        observed.sstatus & SSTATUS_SIE != 0,
        entry_sie_enabled,
        "sret did not restore the pre-trap SIE state"
    );
}

#[test_case]
fn four_byte_breakpoint_round_trip_preserves_context() {
    run_probe(trap_probe_ebreak, false);
}

#[test_case]
fn compressed_breakpoint_round_trip_preserves_context() {
    run_probe(trap_probe_compressed_ebreak, true);
}

#[test_case]
fn repeated_breakpoints_leave_trap_state_reusable() {
    run_probe(trap_probe_ebreak, false);
    run_probe(trap_probe_compressed_ebreak, true);
}

#[test_case]
fn recurring_timer_interrupts_return_and_preserve_interrupt_state() {
    const STIE: usize = 1 << 5;
    const REQUIRED_INTERRUPTS: usize = 3;

    let original_sstatus: usize;
    let original_sie: usize;
    unsafe {
        asm!(
            "csrr {sstatus}, sstatus",
            "csrr {sie}, sie",
            "csrci sstatus, 0x2",
            sstatus = out(reg) original_sstatus,
            sie = out(reg) original_sie,
            options(nostack),
        );
    }

    let frame_before = current_trap_frame_ptr();
    let initial_count = timer::interrupt_count();
    let timeout_ticks = device_tree::timer_frequency();

    unsafe {
        asm!("csrs sie, {stie}", stie = in(reg) STIE, options(nostack));
    }

    // The first deadline is immediately due. Interrupts two and three must
    // come from the one-second deadline programmed by the handler itself.
    timer::new_time(Duration::ZERO);

    for interrupt_index in 1..=REQUIRED_INTERRUPTS {
        let expected_count = initial_count + interrupt_index;

        unsafe {
            asm!("csrsi sstatus, 0x2", options(nostack));
        }

        let timeout = riscv::asm::rdtime().saturating_add(timeout_ticks.saturating_mul(2));
        while timer::interrupt_count() < expected_count && riscv::asm::rdtime() < timeout {
            core::hint::spin_loop();
        }

        let returned_sstatus: usize;
        unsafe {
            asm!(
                "csrr {returned_sstatus}, sstatus",
                "csrci sstatus, 0x2",
                returned_sstatus = out(reg) returned_sstatus,
                options(nostack),
            );
        }

        assert_eq!(
            timer::interrupt_count(),
            expected_count,
            "timer interrupt {interrupt_index} was not delivered before the test timeout"
        );
        assert_ne!(
            returned_sstatus & SSTATUS_SIE,
            0,
            "sret did not restore global supervisor interrupts"
        );

        let frame_after = current_trap_frame_ptr();
        assert_eq!(frame_after, frame_before, "timer trap changed sscratch");

        let frame = unsafe { frame_after.read_volatile() };
        assert_eq!(
            frame.sstatus & SSTATUS_SIE,
            0,
            "hardware must clear SIE on timer-trap entry"
        );
        assert_ne!(
            frame.sstatus & SSTATUS_SPIE,
            0,
            "timer trap did not preserve the pre-trap SIE state in SPIE"
        );
        assert_ne!(
            frame.sstatus & SSTATUS_SPP,
            0,
            "timer interrupt should have trapped from supervisor mode"
        );
    }

    // The handler leaves a one-second deadline armed. Restoring the original
    // `sie` value prevents it from interfering with later trap probes.
    unsafe {
        asm!(
            "csrw sie, {sie}",
            "csrw sstatus, {sstatus}",
            sie = in(reg) original_sie,
            sstatus = in(reg) original_sstatus,
            options(nostack),
        );
    }
}
