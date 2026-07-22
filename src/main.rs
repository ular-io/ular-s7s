//! s7s binary entry point.
//!
//! All application logic lives in the `s7s` library crate so it can be tested
//! without launching the binary; this shim only forwards to the runtime.

fn main() -> anyhow::Result<()> {
    s7s::run()
}
