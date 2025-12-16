#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    loop {}
}

#[panic_handler]
fn panic_handler(_info: &PanicInfo) -> ! {
    loop {}
}
