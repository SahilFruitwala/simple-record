//! Manual check of the stderr tee: `REC_LOG_DIR=/tmp/x cargo run --example log_tee`.
fn main() {
    simple_record_portable::logging::start();
    eprintln!("first line");
    eprint!("status 1\rstatus 2\r");
    eprintln!("last line");
    simple_record_portable::logging::finish();
    eprintln!("after finish (terminal only)");
}
