//! This is a simple implementation to schedule multiple tasks in the kernel address space as a
//! start to explore processes and eventually userspace programs.
//!
//! The schedulers responsibility is to decide which [`Task`] is running when on the single CPU
//! that is available in this System. For a first version this should be a simple round-robin
//! scheduler using the hardware timer ticks to switch the context.
//!
//! We treat the shell as the intial process for the kernel as everything builds up on it and when
//! we exit the shell, the kernel exits as well.
//!
//! For that we can define a `new_with_shell()` function that initializes the scheduler and
//! schedules the shell which keeps the kernel running.
//!
//! A couple of important questions:
//!
//! 1. How do we create a [`Task`]?
//!
//!     To create a new [`Task`] we need to do initialize a stack for the [`Task`] to run on and
//!     let the scheduler know that there is a new task.
//!
//!     Since the [`Task`] is also running in the kernel address space and is just another Rust
//!     function, we don't need to change `satp` to point to a different page table nor do we need
//!     to load a binary from disk as the code is already in the kernel binary.
//!
//!     To create a `Stack` for the new [`Task`] we can use the [`page_frame_allocator`] to create
//!     a new mapping in the kernel page table for an empty page. We can then set the `sp` stack
//!     pointer to the top of this newly allocated page.
//!
//!     The new [`Task`] was not interrupted yet and thus does not have a `TrapStack`. We need to
//!     create one with most registers defaulting to 0.
//!
//!     We also need to set the `sstatus` register. The newly created task has not been interrupted
//!     yet but we want to return from it like we'd do after an interrupt. In order to do this, we
//!     need to set the `SPP` bit in `sstatus` to 1 indicating that we came from the supervisor mode
//!     and `sret` will return into supervisor mode. We also need to set `SPIE` to enable interrupts
//!     after the `sret` call and disable `SIE` as the current operation i.e context switching
//!     should not be interrupted again. Returning from the trap handler with `sret` will then set
//!     the priviledge level to supervisor mode (SPP) and enable interrupts (SPIE).
//!
//!     One caveat here is that we need to think about how to manage the stack. In this case, we
//!     only allocate one page for the whole stack. This seems like it's too little. Actually the
//!     stack size for a process on my current machine is just 2 pages so this should be fine for
//!     now. Anyways, we should make sure we handle the stack overflow case.
//!
//! 2. How do we run a [`Task`]?
//!
//!     To run a task, we first need to stop the old task and then hand over execution to the new
//!     task. Stopping the old task is the next point in this list.
//!
//!     Each [`Task`] stores it's own [`TrapFrame`] which contains all the registers and states
//!     that this [`Task`] was in when it was interrupted. The `kernel_sp` field points to the
//!     `TrapStack`, i.e the stack that we use to handle traps.
//!
//!     Since we're still in the kernel address space, we can just use the same `TrapStack` to
//!     handle traps for all the processes, hence we can put the same address in all the
//!     `.kernel_sp` fields for all the [`Task`]s in the current implmentation.
//!
//!     There is an issue in the current implementation which causes nested traps to overwrite
//!     the state of the previous trap handling which should crash the system. This is fine for
//!     now.
//!
//!     Since the context switch happens during an interrupt, we need to look at the trap handler
//!     in order to work out how to hand of execution to another [`Task`].
//!
//!     We know about the [`TrapFrame`] of the newly scheduled [`Task`] which contains the `sepc`
//!     register. At the end of the trap handler we return with `sret` instruction which hands
//!     exectution off to the address in `sepc`.
//!
//!     So in order to fully context switch, we need to point `sscratch` to the [`TrapFrame`] of
//!     the newly scheduled task. The trap handler will then set all the registers in place and
//!     `sret` will hand over execution to it.
//!
//!     We don't need to call `sfence.vma` instruction as we're still in the same address space and
//!     all the [`Task`]s are in the kernel. No need to flush the TLB or change the `satp` register
//!     for now.
//!
//! 3. How do we clean up a task?
//!
//!     I haven't thought about this yet. I have no idea how a [`Task`] would communicate that it is
//!     finished.
//!
//!     I think in general (i.e in Linux) the process or task finishing is calling the `exit`
//!     syscall. Since we don't have syscalls yet, we can create a wrapper which produces the
//!     functions (or [`Task`]s) that we accept in the scheduler to something that calls a function
//!     on the scheduler once it's finished.
//!
//!     Since we can't cleanup the task during the 'exit' of this task as we'd work on free'd stack
//!     memeory, we can only mark a task as 'Exited' and the cleanup happens during execution of
//!     another task. Need to check that `sscratch` doesn't match the `sscratch` of the exited task.
//!
//!     This way the scheduler would be invoked directly and we could handle the exiting task then.
//!
//!     I do know that we need to:
//!         1. free the page that we used for the stack
//!         2. remove the [`Task`] from the scheduler
//!         3. make the `pid` re-usable
//!         4. make sure we don't access any memory which is now free'd
//!         5. hand off execution to something else (for now we can assume that the shell will
//!            always be alive, so there will always be at least one other process)
//!
//! 4. Check all the locks (scheduler, console, page_frame_allocator etc).
//!
//! 5. Check how we can implement 'sleep' like we use in the shell to wait for input.

extern crate alloc;

use alloc::vec::Vec;

use virtual_memory::PAGE_SIZE;

use crate::process::{DeadProcessInfo, ExitableTask, ProcessInfo};
use crate::riscv::{self, sstatus};
use crate::{
    interrupts,
    process::{Pid, Process, State},
    trap_handler::TrapFrame,
};
use crate::{page_frame_allocator, page_table};

const MAX_TASKS: usize = 20;

/// This is supposed to be a simple scheduler running a couple of tasks in the kernel address
/// space. There is no userspace yet and we're not running executables etc, just different rust
/// functions.
///
/// The first stage is to have a simple round-robin scheduler walking through the list of processes
/// and handing execution over. This is done on a timer interrupt currently configured to 10ms
/// which means that every 10ms we're running a different task on the CPU.
///
/// A [`Task`] is currently defined as a function pointer to a Rust function.
struct Scheduler {
    /// A list of all processes handled by the `Scheduler`. The index in the array corresponds with
    /// the `pid` of the process. The fixed array has stable slot addresses while the scheduler
    /// remains in its static location. If we want to change this later
    /// on, we should make this a linked-list or another structure which keeps stable addresses.
    tasks: [Option<Process>; MAX_TASKS],

    /// The [`Pid`] of the currently running process.
    current: Option<Pid>,

    /// The [`Pid`] to be removed in the next `.next()` call.
    to_be_removed: Option<Pid>,

    proc_history: alloc::vec::Vec<DeadProcessInfo>,
}

#[derive(Debug)]
enum SchedulerError {
    /// Max number of tasks is reached.
    TaskLimit,

    /// Scheduler was not able to allocate a page.
    OutOfMemory,

    /// `next()` was called but there is nothing to run. Can't deal with this at the moment.
    NoRunnableTask,

    /// Could not allocate the stack for a new task.
    FailedAllocateStack,
}

/// Number of pages allocated as the stack for each task including a guard page to detect stack
/// overflows.
const TASK_STACK_PAGES: usize = 4;

impl Scheduler {
    /// Initializes the Scheduler with `MAX_TASKS` empty slots.
    const fn new() -> Self {
        Self {
            tasks: [const { None }; MAX_TASKS],
            current: None,
            to_be_removed: None,
            proc_history: Vec::new(),
        }
    }

    fn schedule(&mut self, task: fn(), name: &'static str) -> Result<(), SchedulerError> {
        let pid = self
            .tasks
            .iter()
            .position(Option::is_none)
            .map(Pid::from_usize)
            .ok_or(SchedulerError::TaskLimit)?;

        let exitable_task = ExitableTask::new(task);

        // Allocate the stack for the new task
        let phys_sp = page_frame_allocator::alloc_contiguous(TASK_STACK_PAGES)
            .ok_or(SchedulerError::FailedAllocateStack)?;

        // Physical address of the guard page. The lowest page in the range
        let guard_page = phys_sp;

        // The stack grows downwards so put the stack pointer at the top of the stack. It's a
        // physical address which is fine in this case, as we're in the kernel address space and all
        // available pages are idendity mapped.
        let phys_sp = phys_sp + TASK_STACK_PAGES * PAGE_SIZE;

        if let Err(e) = page_table::unmap_identity_mapped_page(guard_page) {
            log::error!("Could not unmap guard page {e:?}");
        }

        // documented at top of file
        let sstatus = (sstatus::SPIE | sstatus::SPP) & !sstatus::SIE;

        // TODO(mt): rename `kernel_sp` to `trap_stack_top` in `TrapFrame`.

        // The `kernel_sp` is currently the same for all tasks. Just read it from the current
        // TrapFrame.
        let kernel_sp = {
            // Get a pointer to the current `TrapFrame`.
            let sscratch = riscv::asm::sscratch();

            // The first field of `TrapFrame` is the `kernel_sp`. This means we actually don't need
            // to cast to a `TrapFrame` here and access the `.kernel_sp` field as this would also
            // work if we'd just return a usize read at `*sscratch`.
            unsafe { (*(sscratch as *const TrapFrame)).kernel_sp }
        };

        let sepc = ExitableTask::run_with_exit as *const () as usize;

        let mut tf = TrapFrame::zero();

        tf.kernel_sp = kernel_sp;
        tf.sstatus = sstatus;
        tf.sepc = sepc;
        tf.sp = phys_sp;
        tf.guard_page = guard_page;

        let process = Process {
            task: exitable_task,
            frame: tf,
            pid,
            state: State::Runnable,
            name,
        };

        self.tasks[pid.as_usize()] = Some(process);

        // After moving the process into `.tasks` take the address of `process.task` and store it in
        // `a0` as this is the first argument to the `run_with_exit` call.
        let process = self.tasks[pid.as_usize()].as_mut().unwrap();
        process.frame.a0 = (&raw const process.task) as usize;

        if self.current.is_none() {
            self.current = Some(pid);
        }

        Ok(())
    }

    fn maybe_cleanup_exited_task(&mut self) {
        if let Some(to_be_removed) = self.to_be_removed.take() {
            assert_ne!(
                to_be_removed,
                self.current.unwrap(),
                "Can't remove the currently running process"
            );

            // take the task out of the task list in the scheduler
            let task = core::mem::take(&mut self.tasks[to_be_removed.as_usize()]).unwrap();

            // free it's allocated stack
            page_frame_allocator::free_contiguous(task.frame.guard_page, TASK_STACK_PAGES);
        }
    }

    /// This is called from the interrupt handler which is capturing the timer events which drive
    /// this scheduler.
    ///
    /// The goal here is to just move execution to the next available process.
    ///
    /// In order to do that we have to:
    /// 1. Stop execution of the previous task
    /// 2. Store all information so we can resume it
    /// 3. Update old tasks state to [`target_state`] (this makes it possible to exit a task as
    ///    well)
    /// 4. Figure out next task to run
    /// 5. Load all registers from the new task
    /// 6. Change it's state
    /// 7. Resume execution from that point onwards.
    fn next(
        &mut self,
        current_frame: *const TrapFrame,
        target_state: Option<State>,
    ) -> Result<*const TrapFrame, SchedulerError> {
        self.maybe_cleanup_exited_task();

        let current_pid = self
            .current
            .ok_or(SchedulerError::NoRunnableTask)?
            .as_usize();

        let proc = self.tasks[current_pid]
            .as_mut()
            .ok_or(SchedulerError::NoRunnableTask)?;

        proc.frame = unsafe { *current_frame };
        proc.state = target_state.unwrap_or(State::Runnable);

        // Search actual slot indices, wrapping once and considering the current task last.
        let next_runnable_pid = (1..=MAX_TASKS)
            .map(|offset| (current_pid + offset) % MAX_TASKS)
            .find(|&index| {
                self.tasks[index]
                    .as_ref()
                    .is_some_and(|proc| matches!(proc.state, State::Runnable))
            })
            .map(Pid::from_usize)
            .ok_or(SchedulerError::NoRunnableTask)?;

        let next_proc = self.tasks[next_runnable_pid.as_usize()]
            .as_mut()
            .ok_or(SchedulerError::NoRunnableTask)?;

        next_proc.state = State::Running;

        self.current = Some(next_runnable_pid);

        Ok(&next_proc.frame)
    }
}

fn exit_current() {
    // On a normal exit of a `Task` there is no way we can return execution to that process ever
    // again.
    //
    // The process is currently executing and the exit call should come from a trap i.e through a
    // syscall so we should have the TrapFrame accessible and could just call the same
    // `kill_current_and_schedule_next()` function but right now this is not what happens.
    //
    // Could we do it that way and implement our first syscall? We could have a
    // SupervisorSoftwareInterrupt to create an interrupt which is handled as an exit syscall.
    //
    // This is something for another MR so it will not be implemented right away and this todo!()
    // will stay for now.
    //
    // The idea:
    //
    // - trap handler checks for SupervisorSoftwareInterrupt as we currently only have kernel
    // tasks.
    // - when it's detected it checks the registers to understand which syscall was called
    // and we implement exit in a similar way of how we're currently exiting when a stack overflow
    // is found.
    // - the `run_with_exit()` function is then just issuing this syscall at the end after running
    // `task()` and thus transfers execution to the trap handler which then exits this task and
    // hands execution over to the next task.
    todo!()
}

static SCHEDULER: spin::Mutex<Scheduler> = spin::Mutex::new(Scheduler::new());

/// Need to disable interrupts as this function is currently called from normal code which holds a
/// lock and the interrupt triggered `next` would deadlock then.
pub fn schedule(task: fn(), name: &'static str) {
    interrupts::without_interrupts(|| {
        let mut sched = SCHEDULER.lock();
        if let Err(e) = sched.schedule(task, name) {
            log::error!("{e:?}");
        }
    });
}

/// This is only called from an interrupt context. Therefore we don't need to disable interrupts
/// right now.
pub fn next(frame: *const TrapFrame) -> *const TrapFrame {
    let mut sched = SCHEDULER.lock();

    match sched.next(frame, None) {
        Ok(frame) => frame,
        Err(e) => {
            log::error!("{e:?}");
            frame
        }
    }
}

/// This function is **only** called from the trap handler when a task has to be killed
/// and execution moves on to the next task.
///
/// Since this is called from the trap handler we do not need to worry about interrupts.
pub fn kill_current_and_schedule_next(
    frame: *const TrapFrame,
    exit_code: usize,
) -> *const TrapFrame {
    let mut sched = SCHEDULER.lock();
    let current_pid = sched.current.unwrap();
    let new_frame = sched.next(frame, Some(State::Exited)).unwrap();
    sched.to_be_removed = Some(current_pid);
    sched.proc_history.push(DeadProcessInfo {
        pid: current_pid,
        exit_code,
    });

    new_frame

    // When a process is killed, we need to
    // - remove it's state from the scheduler
    // - free the stack
    //   The stack are allocated pages set by `schedule()`. They need to be free'd in the page_frame_allocator.
}

pub fn init() {
    interrupts::without_interrupts(|| {
        let sched = SCHEDULER.lock();
        let first_task = sched.tasks.first().as_ref().unwrap().as_ref().unwrap();
        let tf = first_task.frame_ptr() as usize;
        unsafe { core::arch::asm!("csrw sscratch, {}", in(reg) tf) }
    })
}

pub struct SchedulerState {
    pub live: alloc::vec::Vec<ProcessInfo>,
    pub dead: alloc::vec::Vec<DeadProcessInfo>,
}

/// Returns the state of the Scheduler for use in the shell.
pub fn state() -> SchedulerState {
    interrupts::without_interrupts(|| {
        let sched = SCHEDULER.lock();
        let live = sched
            .tasks
            .iter()
            .flatten()
            .map(|proc| ProcessInfo {
                pid: proc.pid,
                state: proc.state,
                name: proc.name,
            })
            .collect();

        let dead = sched.proc_history.clone();

        SchedulerState { live, dead }
    })
}

/// Hand off execution to the first process by restoring its TrapFrame and doing `sret`.
/// sscratch already points to process[0]'s TrapFrame from `init`.
#[unsafe(naked)]
pub extern "C" fn start() -> ! {
    core::arch::naked_asm!(
        "csrr a0, sscratch",
        "ld t0,   256(a0)",
        "csrw sepc, t0",
        "ld t0, 264(a0)",
        "csrw sstatus, t0",
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
