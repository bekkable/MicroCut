//! Last-resort crash guard. Not a licence to panic: code must return `Result` instead.

use std::panic;

/// Reports panics with location before the default hook runs, so a crash leaves a trace even
/// in release builds without a console.
pub fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        eprintln!("microcut: internal error: {info}");
        default_hook(info);
    }));
}
