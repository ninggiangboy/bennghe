// SPDX-License-Identifier: AGPL-3.0-only
//! Generates the Rust types from `proto/` with a pure-Rust parser, so no `protoc` is needed.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = protox::compile(["bn/v1/version.proto"], ["../../proto"])?;
    prost_build::compile_fds(files)?;
    println!("cargo:rerun-if-changed=../../proto");
    Ok(())
}
