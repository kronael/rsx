/// Install panic handler that crashes process on any
/// thread panic. Exit code 1, print panic info to stderr.
pub fn install_panic_handler() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("fatal: {}", info);
        std::process::exit(1);
    }));
}
