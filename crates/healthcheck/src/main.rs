// SPDX-License-Identifier: AGPL-3.0-only
//! Tiny static health probe bind-mounted into user containers (stub until phase 5)

fn main() {
    println!("bn-healthcheck {}", env!("CARGO_PKG_VERSION"));
}
