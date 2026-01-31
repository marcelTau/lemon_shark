extern crate alloc;
use core::str::FromStr;

use crate::dump_memory;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::timer;

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
                    if s.pop().is_some() {
                        print!("\x08 \x08");
                    }
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
    println!("CPUs: {cpus} ({cpu_isa})");
    println!("Timer frequency: {}MHz", timer_frequency / 1000 / 1000);
    println!("Total memory: {}MB", total_memory / 1024 / 1024);
}

fn help() {
    println!("Available commands:");
    println!("  help            -- show this help menu");
    println!("  exit            -- shutdown the OS");
    println!("  memory          -- show the current state of the global kernel allocator");
    println!("  timer <n>       -- set a timer for N seconds which will cause an interrupt");
    println!("  sysinfo         -- print system information");
    println!("  uptime          -- show for how long the system is running");
    println!("  allocate <n>    -- allocate memory of size n to test the kernel allocator");
    println!("  mkdir <name>    -- create a new directory in root");
    println!("  ls              -- show directories");
}

fn shell_allocate(size: usize) {
    let vec: Vec<u8> = alloc::vec![0; size];
    let b = Box::new(vec);
    Box::leak(b);
}

fn benchmark_allocator(n: usize, size: usize) {
    extern crate alloc;
    use alloc::vec;
    use alloc::vec::Vec;

    let mut allocations: Vec<Vec<u8>> = Vec::new();

    let freq = crate::device_tree::timer_frequency() / 1000;
    let start = timer::rdtime() / freq;

    // Current memory size is 1024Kb
    for _ in 0..n {
        allocations.push(vec![0; size]);
    }

    for _ in 0..n {
        let alloc = allocations.pop().unwrap();
        drop(alloc);
    }

    let end = timer::rdtime() / freq;

    println!("Took: {}ms", end - start);
}

enum ShellCommand {
    Hello,
    Exit,
    MemoryDump,
    Timer { secs: usize },
    SysInfo,
    Uptime,
    Help,
    Allocate { size: usize },
    Bench { n: usize, size: usize },
    Ls { dir: u32 }, // INodeIndex for now
    Mkdir { name: String },
    DumpFs,
    Write { inode_index: usize, text: String },
    Cat { inode_index: usize },
}

impl ShellCommand {
    /// A very naive way of reading user-input but for this shell it's fine :)
    fn from_line(line: &str) -> Option<ShellCommand> {
        let parts: Vec<&str> = line.trim().split(' ').collect();

        if parts.is_empty() {
            return None;
        }

        let command = parts[0];

        let command = match command {
            "hello" => ShellCommand::Hello,
            "exit" => ShellCommand::Exit,
            "memory" => ShellCommand::MemoryDump,
            "uptime" => ShellCommand::Uptime,
            "sysinfo" => ShellCommand::SysInfo,
            "bench" => {
                let n = parts.get(1).and_then(|n| n.parse().ok())?;
                let size = parts.get(2).and_then(|n| n.parse().ok())?;
                ShellCommand::Bench { n, size }
            }
            "allocate" => {
                let n = parts.get(1).and_then(|n| n.parse().ok())?;
                ShellCommand::Allocate { size: n }
            }
            "timer" => {
                let secs = parts.get(1).and_then(|secs| secs.parse().ok())?;
                ShellCommand::Timer { secs }
            }
            "help" => ShellCommand::Help,
            "mkdir" => {
                let mut name = String::from_str(parts.get(1).unwrap()).unwrap();

                // TODO(mt): quick hack, if the path doesn't start with a '/'
                // the prepend it so that it works with the current implementation.

                if !name.starts_with("/") {
                    name.insert(0, '/');
                }

                ShellCommand::Mkdir { name }
            }
            "ls" => {
                let dir = parts
                    .get(1)
                    .and_then(|secs| secs.parse().ok())
                    .unwrap_or_default();
                ShellCommand::Ls { dir }
            }
            "dumpfs" => ShellCommand::DumpFs,
            "write" => {
                let (inode_index, rest) = parts.split_at(2);

                ShellCommand::Write {
                    inode_index: inode_index[1].parse().unwrap(),
                    text: rest.join(" "),
                }
            }
            "cat" => {
                let inode_index = parts.get(1).and_then(|n| n.parse().ok())?;
                ShellCommand::Cat { inode_index }
            }
            _ => return None,
        };

        Some(command)
    }

    fn call(&self) {
        match self {
            ShellCommand::Help => help(),
            ShellCommand::Hello => hello(),
            ShellCommand::Exit => exit(),
            ShellCommand::SysInfo => sysinfo(),
            ShellCommand::MemoryDump => dump_memory(),
            ShellCommand::Bench { n, size } => benchmark_allocator(*n, *size),
            Self::Allocate { size } => shell_allocate(*size),
            ShellCommand::Timer { secs } => crate::timer::new_time(*secs),
            ShellCommand::Ls { dir } => crate::filesystem::api::dump_dir(*dir),
            ShellCommand::Mkdir { name } => { 
                if let Err(e) = crate::filesystem::api::mkdir(name) {
                    println!("mkdir failed: {e:?}");
                }
            }
            ShellCommand::DumpFs => crate::filesystem::dump(),
            ShellCommand::Cat { inode_index } => {
                let output = crate::filesystem::api::read_file(*inode_index);
                println!("{output}");
            }
            ShellCommand::Uptime => {
                let time = crate::timer::uptime();
                println!("Currently running for {time}s");
            }
            ShellCommand::Write { inode_index, text } => {
                crate::filesystem::api::write_to_file(*inode_index, text.clone()).unwrap();
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
