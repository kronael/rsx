use rsx_types::install_panic_handler;

mod app;
mod cast_loop;
mod metrics;

fn main() {
    install_panic_handler();

    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

    app::run();
}
