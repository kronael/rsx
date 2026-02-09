/// Install panic handler that crashes process on any
/// thread panic. Exit code 1, print panic info to stderr.
pub fn install_panic_handler() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("fatal: {}", info);
        std::process::exit(1);
    }));
}

/// RAII cleanup guard. Runs callback on drop.
pub struct DeferCall<F: FnOnce()> {
    callback: Option<F>,
}

impl<F: FnOnce()> DeferCall<F> {
    pub fn new(callback: F) -> Self {
        Self {
            callback: Some(callback),
        }
    }
}

impl<F: FnOnce()> Drop for DeferCall<F> {
    fn drop(&mut self) {
        if let Some(f) = self.callback.take() {
            f();
        }
    }
}

#[macro_export]
macro_rules! defer {
    ($($block:tt)*) => {
        let _guard = $crate::macros::DeferCall::new(
            || { $($block)* }
        );
    };
}

#[macro_export]
macro_rules! on_error_continue {
    ($($block:tt)*) => {
        match $($block)* {
            Ok(value) => value,
            Err(_) => { continue; }
        }
    };
}

#[macro_export]
macro_rules! on_none_continue {
    ($($block:tt)*) => {
        match $($block)* {
            Some(value) => value,
            None => { continue; }
        }
    };
}

#[macro_export]
macro_rules! on_error_return_ok {
    ($($block:tt)*) => {
        match $($block)* {
            Ok(value) => value,
            Err(_) => {
                return Ok(Default::default());
            }
        }
    };
}

#[macro_export]
macro_rules! on_none_return_ok {
    ($($block:tt)*) => {
        match $($block)* {
            Some(value) => value,
            None => {
                return Ok(Default::default());
            }
        }
    };
}
