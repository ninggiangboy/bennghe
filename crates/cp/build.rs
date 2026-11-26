// SPDX-License-Identifier: AGPL-3.0-only
//! The web UI is embedded from `web/dist`. Make sure the folder exists so a backend-only build
//! (no Node toolchain) still compiles and serves a clear "UI not built" message.
fn main() {
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    std::fs::create_dir_all(&dist).expect("create the web UI output folder");
    println!("cargo:rerun-if-changed=build.rs");
}
