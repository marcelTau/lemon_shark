extern crate alloc;
use alloc::vec::Vec;

static EARLY_BUF: spin::Mutex<EarlyBuffer> = spin::Mutex::new(EarlyBuffer::new());

struct EarlyBuffer {
    buf: [u8; 2048],
    len: usize,
}

impl EarlyBuffer {
    const fn new() -> Self {
        Self {
            buf: [0u8; 2048],
            len: 0,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let len = bytes.len();
        self.buf[self.len..self.len + len].copy_from_slice(bytes);
        self.len += len;
    }

    fn inner(&self) -> Vec<u8> {
        Vec::from(&self.buf[..])
    }
}

pub fn flush_early_buffer() {
    let vec = EARLY_BUF.lock().inner();

    crate::virtio2::console_write(&vec);
}

fn module_prefix(target: &str) -> &str {
    // Map Rust module paths to short, fixed-width labels.
    // The fallback strips the "lemon_shark::" crate prefix and uses whatever remains.
    if target.starts_with("lemon_shark::filesystem") || target.starts_with("filesystem") {
        "fs"
    } else if target.starts_with("lemon_shark::allocator") || target.starts_with("allocator") {
        "alloc"
    } else if target.starts_with("lemon_shark::virtio") || target.starts_with("virtio_drivers") {
        "virtio"
    } else if target.starts_with("lemon_shark::trap_handler") {
        "trap"
    } else if target.starts_with("lemon_shark::interrupts") {
        "irq"
    } else if target.starts_with("lemon_shark::device_tree") {
        "dtb"
    } else if target.starts_with("lemon_shark::shell") {
        "shell"
    } else if target.starts_with("lemon_shark::timer") {
        "timer"
    } else if target.starts_with("lemon_shark::klog") {
        "klog"
    } else {
        target.trim_start_matches("lemon_shark::")
    }
}

struct KernelLogger;

impl log::Log for KernelLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        use core::fmt::Write;
        let mut buf = StackString::<512>::new();
        let label = module_prefix(record.target());
        // ANSI color codes: yellow for WARN, red for ERROR, reset after level.
        let (pre, post) = match record.level() {
            log::Level::Error => ("\x1b[31m", "\x1b[0m"),
            log::Level::Warn => ("\x1b[33m", "\x1b[0m"),
            _ => ("", ""),
        };
        // Level is at most 5 chars (DEBUG). Label padded to 8 chars for alignment.
        let _ = write!(
            buf,
            "{}[{:<5} {:<8}] {}{}\n",
            pre,
            record.level(),
            label,
            record.args(),
            post,
        );
        let written = crate::virtio2::console_write(buf.as_bytes());
        if !written {
            EARLY_BUF.lock().append(buf.as_bytes());
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

    log::debug!("initialized");
}
