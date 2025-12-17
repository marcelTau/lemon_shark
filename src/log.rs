fn write_char_to_uart(c: char) {
    /// The UART is a hardware device which QEMU reads from and displays in the terminal.
    const UART_ADDRESS: usize = 0x10000000;

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
    UartWriter.write_fmt(args).unwrap()
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::_print(format_args!($($arg)*));
    };
}
