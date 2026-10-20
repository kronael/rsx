use rsx_types::install_panic_handler;

mod app;
mod cancel;
mod cast_loop;
mod order;
#[cfg(test)]
mod tile_test_utils;

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

    app::run();
}
