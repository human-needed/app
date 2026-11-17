// SPDX-License-Identifier: GPL-3.0-or-later
fn main() {
    // Stamp the target triple into `hn version` (per-OS builds for the plugins, D-046).
    println!(
        "cargo:rustc-env=HN_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
