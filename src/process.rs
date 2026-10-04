extern crate alloc;
use crate::trap_handler::TrapFrame;

/// The Process ID
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct Pid(i32);

impl Pid {
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }

    pub fn from_usize(pid: usize) -> Self {
        Self(pid as i32)
    }
}

#[derive(Copy, Clone, Debug)]
pub enum State {
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
pub(crate) struct Process {
    pub pid: Pid,
    pub state: State,
    pub frame: TrapFrame,
    pub task: ExitableTask,
    pub name: &'static str,
}

impl Process {
    pub fn frame_ptr(&self) -> *const TrapFrame {
        &self.frame as *const TrapFrame
    }
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
pub struct ExitableTask {
    task: fn(),
}

impl ExitableTask {
    pub fn new(task: fn()) -> Self {
        Self { task }
    }

    /// Note: [`extern "C"`] to fix the calling convention and be able to take a
    /// pointer to `Self` as the first argument
    pub unsafe extern "C" fn run_with_exit(ptr: *const Self) {
        let task = unsafe { (*ptr).task };
        task();
        unsafe { core::arch::asm!("li a0, 0", "li a7, 93", "ecall") }
    }
}

#[derive(Clone)]
pub struct DeadProcessInfo {
    pub pid: Pid,
    pub exit_code: usize,
}

impl core::fmt::Display for DeadProcessInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Task {} exited with code {}",
            self.pid.as_usize(),
            self.exit_code
        )
    }
}

pub struct ProcessInfo {
    pub pid: Pid,
    pub state: State,
    pub name: &'static str,
}

impl core::fmt::Display for ProcessInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Task '{}' ({}) is {:?}",
            self.name,
            self.pid.as_usize(),
            self.state
        )
    }
}
