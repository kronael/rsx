use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// Install panic handler that crashes process on any
/// thread panic. Exit code 1, print panic info to stderr.
pub fn install_panic_handler() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("fatal: {}", info);
        std::process::exit(1);
    }));
}

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn on_shutdown_signal(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

/// Install SIGINT/SIGTERM handlers that flip a process-global shutdown
/// flag. Returns the flag; a daemon's loop polls it and drains before
/// exiting so the active WAL is left persisted (spec invariant 7). The
/// panic handler is left untouched, so real crashes still surface.
pub fn install_shutdown_handler() -> &'static AtomicBool {
    // SAFETY: the handler only does an atomic store, which is
    // async-signal-safe.
    unsafe {
        libc::signal(
            libc::SIGINT,
            on_shutdown_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            on_shutdown_signal as *const () as libc::sighandler_t,
        );
    }
    &SHUTDOWN
}
