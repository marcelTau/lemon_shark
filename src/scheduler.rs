extern crate alloc;
use core::arch::{asm, naked_asm};

use alloc::vec::Vec;
use virtual_memory::{PhysAddr, PAGE_SIZE};

use crate::{
    page_frame_allocator,
    trap_handler::{self, TrapFrame},
};

static SCHEDULER: spin::Mutex<Option<Scheduler>> = spin::Mutex::new(None);

struct Scheduler {
    processes: Vec<Process>,
    currently_running: usize,
}

struct Process {
    id: usize,
    trap_frame: TrapFrame,
    running: bool,
}

impl Scheduler {
    /// This switches the context to the next waiting process.
    ///
    /// For that we need to store the current `TrapFrame` for the currently running process and set
    /// `sscratch` to the address of the new processes `TrapFrame`.
    fn next(&mut self, current_trap_frame: TrapFrame) -> *mut TrapFrame {
        self.processes[self.currently_running].running = false;
        self.processes[self.currently_running].trap_frame = current_trap_frame;

        let prev = self.currently_running;

        self.currently_running = (self.currently_running + 1) % self.processes.len();

        // log::debug!("Context switch {prev} -> {}", self.currently_running);

        self.processes[self.currently_running].running = true;
        &raw mut self.processes[self.currently_running].trap_frame
    }

    /// Spawns a new process.
    ///
    /// For that it needs to allocate a frame to load the binary into.
    fn spawn(&mut self, entry: fn()) {
        if self.processes.len() >= MAX_PROCESSES {
            panic!("Too many processes");
        }

        let addr = page_frame_allocator::alloc_frame().unwrap();

        let mut trap_frame = TrapFrame::zero();

        // set up stack at the top of the allocated frame as the stack grows downwards
        trap_frame.sp = addr + PAGE_SIZE;

        // set up the entry point to the function
        trap_frame.sepc = entry as usize;

        // since every process is a kernel process, they use the same trap stack
        trap_frame.kernel_sp = trap_handler::kernel_sp();

        let process = Process {
            id: self.processes.len(),
            trap_frame,
            running: false,
        };

        self.processes.push(process);
    }
}

pub struct ProcessInfo {
    pub id: usize,
    pub running: bool,
}

pub fn list() -> alloc::vec::Vec<ProcessInfo> {
    let guard = SCHEDULER.lock();
    let Some(sched) = guard.as_ref() else {
        return alloc::vec::Vec::new();
    };
    sched
        .processes
        .iter()
        .map(|p| ProcessInfo {
            id: p.id,
            running: p.running,
        })
        .collect()
}

pub fn spawn(entry: fn()) {
    if let Some(s) = SCHEDULER.lock().as_mut() {
        s.spawn(entry);
    }
}

pub fn next(tf: *mut TrapFrame) -> *mut TrapFrame {
    if let Some(s) = SCHEDULER.lock().as_mut() {
        unsafe { s.next(*tf) }
    } else {
        tf
    }
}

pub fn current_pid() -> usize {
    SCHEDULER
        .lock()
        .as_ref()
        .map(|s| s.currently_running)
        .unwrap_or_default()
}

const MAX_PROCESSES: usize = 10;

/// Hand off execution to the first process by restoring its TrapFrame and doing `sret`.
///
/// sscratch already points to process[0]'s TrapFrame from `init_with_shell`.
/// We set sstatus.SPP=1 (stay in supervisor mode) and sstatus.SPIE=1 (enable
/// interrupts after sret) since this isn't coming from a real trap.
#[unsafe(naked)]
pub extern "C" fn start() -> ! {
    naked_asm!(
        "csrr a0, sscratch",
        "ld t0,   256(a0)",
        "csrw sepc, t0",
        // SPP=1 (bit 8), SPIE=1 (bit 5) — supervisor mode, interrupts enabled after sret
        "li t0, 0x120",
        "csrs sstatus, t0",
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
        "ld sp,   16(a0)",
        "ld a0,   80(a0)",
        "sret",
    );
}

pub fn init_with_shell(entry: fn()) {
    let mut sched = Scheduler {
        processes: Vec::with_capacity(MAX_PROCESSES),
        currently_running: 0,
    };

    sched.spawn(entry);

    let tf_addr = &sched.processes[0].trap_frame as *const TrapFrame as usize;

    SCHEDULER.lock().replace(sched);

    unsafe { asm!("csrw sscratch, {}", "sfence.vma", in(reg) tf_addr) }
}
