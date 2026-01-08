extern crate alloc;
use crate::dump_memory;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{print, println};

/// To read from the UART, we need to check wether there is some data available
/// by reading the Line status register and check for the set bit.
fn read_line_and_display() -> String {
    const UART: usize = 0x10_000_000;
    const RECEIVE_BUFFER_REGISTER_OFFSET: usize = 0;
    const LINE_STATUS_REGISTER_OFFSET: usize = 5;

    let uart = UART as *const u8;

    let mut s = String::new();

    print!("> ");

    unsafe {
        loop {
            if uart.add(LINE_STATUS_REGISTER_OFFSET).read_volatile() & 0x1 != 0 {
                let c = uart.add(RECEIVE_BUFFER_REGISTER_OFFSET).read_volatile();

                // TODO(mt): I don't know if this is just QEMU but when pressing
                // enter, it first does a '\r' so we can use this to end the
                // line.
                if c == 13 {
                    print!("\n");
                    break;
                }

                if c == 127 {
                    s.pop();
                    print!("\x08 \x08");
                    continue;
                }

                print!("{}", c as char);

                s.push(c as char);
            }
        }
    }

    s
}

fn hello() {
    println!("Hello there :)");
}

fn exit() {
    crate::exit_qemu(0);
}

fn sysinfo() {
    let cpus = crate::device_tree::cpus();
    let cpu_isa = crate::device_tree::cpu_isa();
    let total_memory = crate::device_tree::total_memory();
    let timer_frequency = crate::device_tree::timer_frequency();

    println!("Kernel: LemonShark v0.0.1");
    println!("CPUs: {cpus} {:?}", cpu_isa);
    println!("Timer frequency: {}MHz", timer_frequency / 1000 / 1000);
    println!("Total memory: {}MB", total_memory / 1024 / 1024);
}

enum ShellCommand {
    Hello,
    Exit,
    MemoryDump,
    Timer { secs: usize },
    SysInfo,
    Uptime,
}

impl ShellCommand {
    fn from_line(line: &str) -> Option<ShellCommand> {
        let parts: Vec<&str> = line.split(' ').collect();

        if parts.is_empty() {
            return None;
        }

        let command = parts[0];

        let command = match command {
            "hello" => ShellCommand::Hello,
            "exit" => ShellCommand::Exit,
            "memory_dump" => ShellCommand::MemoryDump,
            "uptime" => ShellCommand::Uptime,
            "sysinfo" => ShellCommand::SysInfo,
            "timer" => {
                let Some(secs) = parts.get(1).and_then(|secs| secs.parse().ok()) else {
                    return None;
                };

                ShellCommand::Timer { secs }
            }

            _ => return None,
        };

        Some(command)
    }

    fn call(&self) {
        match self {
            ShellCommand::Hello => hello(),
            ShellCommand::Exit => exit(),
            ShellCommand::SysInfo => sysinfo(),
            ShellCommand::MemoryDump => dump_memory(),
            ShellCommand::Timer { secs } => crate::timer::new_time(*secs),
            ShellCommand::Uptime => {
                let time = crate::timer::uptime();
                println!("Currently running for {time}s");
            }
        }
    }
}

/// This spawns a simple shell which let's the user input some commands
/// and reads from the UART and outputs something based on the command.
pub fn shell() -> ! {
    loop {
        let line = read_line_and_display();

        match ShellCommand::from_line(&line) {
            Some(command) => command.call(),
            None => println!("ShellCommand not found: '{line}'"),
        }
    }
}
