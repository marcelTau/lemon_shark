//! This is supposed to be used as output for the user, i.e in the shell
//! implementation. Logging can be done via the `log!` & `logln!` macros.
//! Currently both do the same thing as I haven't figured out how to separate
//! the streams in a nice way.

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
    use core::fmt::Write;

    _print(args);

    let _ = UartWriter.write_str("\n");
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::interrupts::without_interrupts(|| {
            $crate::println::_print(format_args!($($arg)*))
        })
    };
}

#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {
        $crate::interrupts::without_interrupts(|| {
            $crate::println::_println(format_args!($($arg)*))
        })
    };
}
