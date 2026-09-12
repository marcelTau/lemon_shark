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
//!

#![allow(unused)]
extern crate alloc;
use alloc::boxed::Box;

use crate::interrupts;
use crate::page_frame_allocator;
use crate::riscv;
use crate::riscv::sstatus;
use crate::trap_handler::TrapFrame;
use virtual_memory::PAGE_SIZE;

/// The Process ID
#[derive(Copy, Clone, Default, Debug)]
struct Pid(i32);

impl Pid {
    fn as_usize(self) -> usize {
        self.0 as usize
    }

    fn from_usize(pid: usize) -> Self {
        Self(pid as i32)
    }
}

#[derive(Copy, Clone, Debug)]
enum State {
    /// This process is available to run
    Runnable,

    /// This process is currently running
    Running,

    /// This process is sleeping / waiting for an event
    Sleep,

    /// The process has exited and is waiting to be removed from the scheduler.
    Exited,
}

/// A `Process` stores all the information needed to resume it's execution after a context switch.
/// In order to do that, we store the full set of registers.
struct Process {
    pid: Pid,
    state: State,
    frame: TrapFrame,
    task: ExitableTask,
    name: &'static str,
}

impl core::fmt::Debug for Process {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(
            f,
            "Task '{}' ({}) is {:?}",
            self.name,
            self.pid.as_usize(),
            self.state
        )
    }
}

/// A `ExitableTask` is an abstraction around a Rust function we run as a separate task in the
/// kernel. We define this abstraction to enforce that we call the [`process::exit_current()`]
/// function after the task itself finished. This is because we don't have syscalls yet which would
/// inform the scheduler about the exiting state of the task.
struct ExitableTask {
    task: fn(),
}

impl ExitableTask {
    fn new(task: fn()) -> Self {
        Self { task }
    }

    unsafe extern "C" fn run_with_exit(ptr: *const Self) {
        let task = unsafe { (*ptr).task };
        task();
        exit_current();
    }
}

fn exit_current() {
    todo!()
}

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
}

#[derive(Debug)]
enum SchedulerError {
    /// Max number of tasks is reached.
    TaskLimit,

    /// Scheduler was not able to allocate a page.
    OutOfMemory,

    /// `next()` was called but there is nothing to run. Can't deal with this at the moment.
    NoRunnableTask,
}

impl Scheduler {
    /// Initializes the Scheduler with `MAX_TASKS` empty slots.
    const fn new() -> Self {
        Self {
            tasks: [const { None }; MAX_TASKS],
            current: None,
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
        let phys_sp = page_frame_allocator::alloc_frame().ok_or(SchedulerError::OutOfMemory)?;

        // The stack grows downwards so put the stack pointer at the top of the stack. It's a
        // physical address which is fine in this case, as we're in the kernel address space and all
        // available pages are idendity mapped.
        let phys_sp = phys_sp + PAGE_SIZE;

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

    /// This is called from the interrupt handler which is capturing the timer events which drive
    /// this scheduler.
    ///
    /// The goal here is to just move execution to the next available process.
    ///
    /// In order to do that we have to:
    /// 1. Stop execution of the previous task
    /// 2. Store all information so we can resume it
    /// 3. Update old tasks state to `Sleep`
    /// 4. Figure out next task to run
    /// 5. Load all registers from the new task
    /// 6. Change it's state
    /// 7. Resume execution from that point onwards.
    fn next(
        &mut self,
        current_frame: *const TrapFrame,
    ) -> Result<*const TrapFrame, SchedulerError> {
        let current_pid = self
            .current
            .ok_or(SchedulerError::NoRunnableTask)?
            .as_usize();

        let proc = self.tasks[current_pid]
            .as_mut()
            .ok_or(SchedulerError::NoRunnableTask)?;

        proc.frame = unsafe { *current_frame };
        proc.state = State::Runnable;

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

    match sched.next(frame) {
        Ok(frame) => frame,
        Err(e) => {
            log::error!("{e:?}");
            frame
        }
    }
}

pub fn print_state() {
    let mut tasks = alloc::vec::Vec::new();
    let mut current = None;
    interrupts::without_interrupts(|| {
        let sched = SCHEDULER.lock();
        for task in &sched.tasks {
            match task {
                Some(task) => tasks.push(task.name),
                None => tasks.push("<empty>"),
            }
        }
        current = sched.current;
    });

    log::debug!("SchedulerState: cur={current:?} | {tasks:?}");
}

pub fn init() {
    interrupts::without_interrupts(|| {
        let sched = SCHEDULER.lock();
        let first_task = sched.tasks.first().as_ref().unwrap().as_ref().unwrap();
        let tf = &first_task.frame as *const TrapFrame as usize;
        unsafe { core::arch::asm!("csrw sscratch, {}", in(reg) tf) }
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
