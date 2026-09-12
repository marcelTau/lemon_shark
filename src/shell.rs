extern crate alloc;
use core::str::FromStr;
use core::time::Duration;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{riscv, uart};

use crate::{print, println};

const HISTORY_CAPACITY: usize = 100;

struct CommandHistory {
    entries: Vec<String>,
}

impl CommandHistory {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn push(&mut self, line: String) {
        if line.trim().is_empty() {
            return;
        }

        if self.entries.last() == Some(&line) {
            return;
        }

        if self.entries.len() == HISTORY_CAPACITY {
            self.entries.remove(0);
        }

        self.entries.push(line);
    }

    fn print(&self) {
        for (index, command) in self.entries.iter().enumerate() {
            println!("{:>4}  {command}", index + 1);
        }
    }
}

const ASCII_BACKSPACE: u8 = 8;
const ASCII_DELETE: u8 = 127;

/// To read from the UART, we need to check wether there is some data available
/// by reading the Line status register and check for the set bit.
fn read_line_and_display() -> String {
    let mut cmd = String::new();

    print!("hai> ");
    loop {
        let byte = uart::read_byte_blocking();

        if byte == b'\r' {
            print!("\n");
            break;
        }

        // Terminals commonly send either DEL or BS for the backspace key.
        if byte == ASCII_DELETE || byte == ASCII_BACKSPACE {
            if cmd.pop().is_some() {
                print!("\x08 \x08");
            }
            continue;
        }

        print!("{}", byte as char);
        cmd.push(byte as char);
    }

    cmd
}

fn hello() {
    println!("Hello there :)");
}

fn percentage_tenths(part: usize, total: usize) -> usize {
    if total == 0 {
        return 0;
    }

    part.saturating_mul(1000).saturating_add(total / 2) / total
}

fn memory() {
    // Take a snapshot so the allocator lock is released before writing to the UART.
    let stats = crate::ALLOCATOR.stats();
    let used_percent = percentage_tenths(stats.used, stats.total);
    let free_percent = percentage_tenths(stats.free, stats.total);

    println!("Heap memory:");
    println!(
        "  Used:  {} bytes ({} KiB, {}.{}%)",
        stats.used,
        stats.used / 1024,
        used_percent / 10,
        used_percent % 10,
    );
    println!(
        "  Free:  {} bytes ({} KiB, {}.{}%)",
        stats.free,
        stats.free / 1024,
        free_percent / 10,
        free_percent % 10,
    );
    println!(
        "  Total: {} bytes ({} KiB)",
        stats.total,
        stats.total / 1024,
    );
    println!("  Free blocks: {}", stats.free_blocks);
    println!(
        "  Largest free block: {} bytes ({} KiB)",
        stats.largest_free_block,
        stats.largest_free_block / 1024,
    );
}

fn exit() {
    crate::filesystem::api::flush();
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
    println!("  help                -- show this help menu");
    println!("  exit                -- shutdown the OS");
    println!("  sysinfo             -- print system information");
    println!("  memory              -- show the current state of the global kernel allocator");
    println!("  timer <n>           -- set a timer for N seconds which will cause an interrupt");
    println!("  ls                  -- show directories");
    println!("  mkdir <name>        -- creates a new directory");
    println!("  touch <name>        -- creates a new file");
    println!("  rm <path>           -- removes a file");
    println!("  dumpfs              -- dump of the filesystem");
    println!("  cat <file>          -- print content of file to the console");
    println!("  uptime              -- show for how long the system is running");
    println!("  write <file> <text> -- write text to the file");
    println!("  tree                -- show a tree view of the filesystem");
    println!("  flush               -- flush filesystem metadata to disk");
    println!("  history             -- show recently entered commands");
    println!("  allocate <n>        -- allocate memory of size n to test the kernel allocator");
}

fn normalize_root_path(path: &str) -> String {
    let mut path = String::from_str(path).unwrap();

    if !path.starts_with('/') {
        path.insert(0, '/');
    }

    path
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
    let start = riscv::asm::rdtime() / freq;

    // Current memory size is 1024Kb
    for _ in 0..n {
        allocations.push(vec![0; size]);
    }

    for _ in 0..n {
        let alloc = allocations.pop().unwrap();
        drop(alloc);
    }

    let end = riscv::asm::rdtime() / freq;

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
    Ls { path: String }, // INodeIndex for now
    Mkdir { name: String },
    DumpFs,
    Write { path: String, text: String },
    Cat { path: String },
    Touch { path: String },
    Rm { path: String },
    Tree,
    Flush,
    History,
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
            "tree" => ShellCommand::Tree,
            "history" => ShellCommand::History,
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
                let name = normalize_root_path(parts.get(1)?);
                ShellCommand::Mkdir { name }
            }
            "touch" => {
                let name = normalize_root_path(parts.get(1)?);
                ShellCommand::Touch { path: name }
            }
            "rm" => {
                let path = normalize_root_path(parts.get(1)?);
                ShellCommand::Rm { path }
            }
            "ls" => {
                let dir = normalize_root_path(parts.get(1).unwrap_or(&"."));
                ShellCommand::Ls { path: dir }
            }
            "dumpfs" => ShellCommand::DumpFs,
            "flush" => ShellCommand::Flush,
            "write" => {
                let (head, rest) = parts.split_at(2);
                ShellCommand::Write {
                    path: normalize_root_path(head.get(1)?),
                    text: rest.join(" "),
                }
            }
            "cat" => {
                let path = normalize_root_path(parts.get(1)?);
                ShellCommand::Cat { path }
            }
            _ => return None,
        };

        Some(command)
    }

    fn call(&self, history: &CommandHistory) {
        match self {
            ShellCommand::Help => help(),
            ShellCommand::Hello => hello(),
            ShellCommand::Exit => exit(),
            ShellCommand::SysInfo => sysinfo(),
            ShellCommand::MemoryDump => memory(),
            ShellCommand::Bench { n, size } => benchmark_allocator(*n, *size),
            ShellCommand::Allocate { size } => shell_allocate(*size),
            ShellCommand::Timer { secs } => {
                crate::timer::new_time(Duration::from_secs(*secs as u64))
            }
            ShellCommand::Ls { path: dir } => {
                if let Err(e) = crate::filesystem::api::dump_dir(dir) {
                    println!("ls failed: {e:?}");
                }
            }
            ShellCommand::Mkdir { name } => {
                if let Err(e) = crate::filesystem::api::mkdir(name) {
                    println!("mkdir failed: {e:?}");
                }
            }
            ShellCommand::Touch { path: name } => {
                if let Err(e) = crate::filesystem::api::create_file(name) {
                    println!("touch failed: {e:?}");
                }
            }
            ShellCommand::Rm { path } => {
                if let Err(e) = crate::filesystem::api::remove_dir_entry(path) {
                    println!("rm failed: {e:?}");
                }
            }
            ShellCommand::DumpFs => {
                crate::filesystem::api::dump();
            }
            ShellCommand::Cat { path } => match crate::filesystem::api::read_file(path) {
                Ok(output) => println!("{output}"),
                Err(e) => println!("cat failed: {e:?}"),
            },
            ShellCommand::Uptime => {
                let uptime_ms = crate::timer::uptime_ms();

                // Use integer formatting: floating-point initialization and context
                // saving/restoring are not set up in the kernel yet.
                let secs = uptime_ms / 1000;
                let millis = uptime_ms % 1000;

                let irqs = crate::timer::interrupt_count();
                println!("Currently running for {secs}.{millis:03}s with {irqs} interrupts");
            }
            ShellCommand::Write { path, text } => {
                if let Err(e) = crate::filesystem::api::write_to_file(path, text.clone()) {
                    println!("Writing to file failed: {e}");
                }
            }
            ShellCommand::Tree => {
                crate::filesystem::api::tree();
            }
            ShellCommand::Flush => {
                crate::filesystem::api::flush();
            }
            ShellCommand::History => history.print(),
        }
    }
}

/// This spawns a simple shell which let's the user input some commands
/// and reads from the UART and outputs something based on the command.
pub fn shell() {
    let mut history = CommandHistory::new();

    loop {
        let line = read_line_and_display();
        history.push(line.clone());

        match ShellCommand::from_line(&line) {
            Some(command) => command.call(&history),
            None => println!("ShellCommand not found: '{line}'"),
        }
    }
}
