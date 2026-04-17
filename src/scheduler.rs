#![allow(unused)]
extern crate alloc;
use alloc::vec::Vec;
use core::arch::asm;
use virtual_memory::PAGE_SIZE;

use crate::{
    page_frame_allocator, page_table,
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
    fn next(&mut self, current_trap_frame: TrapFrame) -> *mut TrapFrame {
        self.processes[self.currently_running].running = false;
        self.processes[self.currently_running].trap_frame = current_trap_frame;

        self.currently_running = (self.currently_running + 1) % self.processes.len();

        self.processes[self.currently_running].running = true;
        &raw mut self.processes[self.currently_running].trap_frame
    }

    fn new_process(&mut self, entry: fn()) {
        let addr = page_frame_allocator::alloc_frame().unwrap();
        page_table::new_identity_map(addr);

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
