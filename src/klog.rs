use crate::println::_print;

struct KernelLogger;

impl log::Log for KernelLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        use core::fmt::Write;
        let mut buf = StackString::<512>::new();
        let _ = write!(buf, "[{}] {}\n", record.level(), record.args());
        let written = crate::virtio2::console_write(buf.as_bytes());
        if !written {
            // Console not yet initialised (early boot) — fall back to UART.
            _print(format_args!("[{}] {}\n", record.level(), record.args()));
        }
    }

    fn flush(&self) {}
}

/// Simple fixed-capacity stack-allocated string for formatting log lines
/// without heap allocation.
struct StackString<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StackString<N> {
    fn new() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl<const N: usize> core::fmt::Write for StackString<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let remaining = N - self.len;
        let to_copy = s.len().min(remaining);
        self.buf[self.len..self.len + to_copy].copy_from_slice(&s.as_bytes()[..to_copy]);
        self.len += to_copy;
        Ok(())
    }
}

static KERNEL_LOGGER: KernelLogger = KernelLogger;

pub fn init() {
    log::set_logger(&KERNEL_LOGGER).ok();
    log::set_max_level(log::LevelFilter::Debug);

    log::debug!("[Logger] initialized");
}
