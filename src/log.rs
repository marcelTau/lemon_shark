#![allow(unused)]
fn write_char_to_uart(c: char) {
    /// The UART is a hardware device which QEMU reads from and displays in
    /// the terminal.
    const UART_ADDRESS: usize = 0x10_000_000;

    unsafe {
        let uart = UART_ADDRESS as *mut u8;
        uart.write_volatile(c as u8);
    }
}

pub struct UartWriter;

impl core::fmt::Write for UartWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.bytes() {
            write_char_to_uart(c as char);
        }

        Ok(())
    }
}

pub fn _print(args: ::core::fmt::Arguments) {
    #[cfg(not(feature = "logging"))]
    return;
    use core::fmt::Write;

    if UartWriter.write_fmt(args).is_err() {
        // Fallback: write error message directly
        unsafe {
            let uart = 0x10_000_000 as *mut u8;
            uart.write_volatile(b'[');
            uart.write_volatile(b'E');
            uart.write_volatile(b']');
        }
    }
}

pub fn _println(args: ::core::fmt::Arguments) {
    #[cfg(not(feature = "logging"))]
    return;
    use core::fmt::Write;

    _print(args);
    let _ = UartWriter.write_str("\n");
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::interrupts::without_interrupts(|| {
            $crate::log::_print(format_args!($($arg)*))
        });
    };
}

#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {
        $crate::interrupts::without_interrupts(|| {
            $crate::log::_println(format_args!($($arg)*))
        });
    };
}

// TODO(mt): try of semihosted direct writing to file on host os
// extern crate spin;
// static DEBUG_LOGGER: spin::Mutex<DebugLogger> = spin::Mutex::new(DebugLogger::new());
//
// use crate::println;
//
// pub struct DebugLogger {
//     fd: Option<isize>,
// }
//
// impl DebugLogger {
//     pub const fn new() -> Self {
//         // let fd = semihosting::open("kernel.log\0");
//         Self { fd: None }
//     }
//
//     pub fn init(&mut self) {
//         println!("DebugLogger init");
//         let fd = semihosting::open("kernel.log\0");
//         // panic!();
//         println!("Got fd = {fd}");
//         self.fd = (fd > 0).then_some(fd);
//         // self.fd.unwrap();
//     }
//
//     fn write_str_inner(&self, s: &str) {
//         if let Some(fd) = self.fd {
//             semihosting::write(fd, s.as_bytes());
//         }
//     }
// }
//
// pub fn init() {
//     println!("In init");
//     if DEBUG_LOGGER.is_locked() {
//         unsafe {
//             let uart0 = 0x10000000 as *mut u8;
//             uart0.write_volatile(b'1');
//             uart0.write_volatile(b'\n');
//         }
//     } else {
//         println!("Not locked");
//         unsafe {
//             let uart0 = 0x10000000 as *mut u8;
//             uart0.write_volatile(b'p');
//             uart0.write_volatile(b'\n');
//         }
//     }
//
//     DEBUG_LOGGER.lock().init();
// }
//
// struct DebugLogWriter<'a> {
//     logger: &'a DebugLogger,
// }
//
// impl<'a> core::fmt::Write for DebugLogWriter<'a> {
//     fn write_str(&mut self, s: &str) -> core::fmt::Result {
//         self.logger.write_str_inner(s);
//         Ok(())
//     }
// }
//
// pub fn _log(args: ::core::fmt::Arguments) {
//     use core::fmt::Write;
//
//     let logger = DEBUG_LOGGER.lock();
//     let mut writer = DebugLogWriter { logger: &logger };
//     writer.write_fmt(args).unwrap();
// }
//
// pub fn _logln(args: ::core::fmt::Arguments) {
//     use core::fmt::Write;
//     _log(args);
//     let logger = DEBUG_LOGGER.lock();
//     let mut writer = DebugLogWriter { logger: &logger };
//     writer.write_fmt(args).unwrap();
//     writer.write_str("\n").unwrap();
// }
//
// #[macro_export]
// macro_rules! log {
//     ($($arg:tt)*) => {
//         $crate::interrupts::without_interrupts(|| {
//             $crate::log::_log(format_args!($($arg)*))
//         });
//     };
// }
//
// #[macro_export]
// macro_rules! logln {
//     ($($arg:tt)*) => {
//         $crate::interrupts::without_interrupts(|| {
//             $crate::log::_logln(format_args!($($arg)*))
//         });
//     };
// }
//
// // https://github.com/ARM-software/abi-aa/blob/main/semihosting/semihosting.rst#semihosting-operations
// // https://docs.riscv.org/reference/platform-software/semihosting/_attachments/riscv-semihosting.pdf
// pub mod semihosting {
//     use core::arch::asm;
//
//     use crate::println;
//     const SYS_OPEN: usize = 0x1;
//     const SYS_CLOSE: usize = 0x2;
//     const SYS_WRITE: usize = 0x5;
//
//     pub fn system(cmd: &str) {
//         // Length must exclude the null terminator
//         let len = cmd.len().saturating_sub(1);
//         let args = [cmd.as_ptr() as usize, len];
//
//         unsafe {
//             let mut result: isize;
//             asm!(
//                 "slli x0, x0, 0x1f",
//                 "ebreak",
//                 "srai x0, x0, 0x7",
//                 inout("a0") 0x12isize => result,
//                 in("a1") &args as *const _ as usize,
//             );
//
//             println!("SYSTEM RESULT: {result}");
//
//             result;
//         }
//     }
//
//     pub fn open(fname: &str) -> isize {
//         // Mode flags for ARM/RISC-V semihosting open:
//         // 0-3 = read, 4-7 = write, 8-11 = append
//         // Even = text mode, Odd = binary mode
//         let mode = 4; // write, text mode
//
//         // Length must exclude the null terminator
//         let len = fname.len().saturating_sub(1);
//         let args = [fname.as_ptr() as usize, mode, len];
//
//         unsafe {
//             let mut result: isize;
//             asm!(
//                 "slli x0, x0, 0x1f",
//                 "ebreak",
//                 "srai x0, x0, 0x7",
//                 inout("a0") SYS_OPEN => result,
//                 in("a1") &args as *const _ as usize,
//             );
//
//             println!("OPEN FD ({fname}): {result}");
//
//             result
//         }
//     }
//
//     pub fn close(fd: isize) {
//         let args = [fd as usize];
//         unsafe {
//             asm!(
//                 "slli x0, x0, 0x1f",
//                 "ebreak",
//                 "srai x0, x0, 0x7",
//                 in("a0") SYS_CLOSE,
//                 in("a1") &args as *const _ as usize,
//             );
//         }
//     }
//
//     pub fn write(fd: isize, data: &[u8]) -> isize {
//         println!("Write to {data:?} to {fd}");
//         let args = [fd as usize, data.as_ptr() as usize, data.len()];
//         unsafe {
//             let mut result: isize;
//             asm!(
//                 "slli x0, x0, 0x1f",
//                 "ebreak",
//                 "srai x0, x0, 0x7",
//                 inout("a0") SYS_WRITE => result,
//                 in("a1") &args as *const _ as usize,
//             );
//             println!("Write return code ={result}");
//             result
//         }
//
//     }
// }
