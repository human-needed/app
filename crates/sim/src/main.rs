// SPDX-License-Identifier: FSL-1.1-ALv2
//! `hn-sim --listen 127.0.0.1:47800 [--fw 2.0.0] [--uf2-dir <folder>]`
//! Prints one line per change of what the ring shows.

use hn_sim::Sim;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let mut listen = "127.0.0.1:47800".to_string();
    let mut fw = "2.0.0".to_string();
    let mut uf2_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().expect("--listen <addr>"),
            "--fw" => fw = args.next().expect("--fw <version>"),
            "--uf2-dir" => uf2_dir = Some(args.next().expect("--uf2-dir <folder>").into()),
            _ => {
                eprintln!("usage: hn-sim [--listen addr] [--fw version] [--uf2-dir folder]");
                std::process::exit(2);
            }
        }
    }
    let sim = Sim::new(&fw, uf2_dir);
    let addr = sim.listen_tcp(&listen).await.expect("listen");
    println!("hn-sim fw {fw} listening on tcp:{addr}");
    let mut rx = sim.subscribe();
    loop {
        if rx.changed().await.is_err() {
            break;
        }
        let shown = rx.borrow_and_update().clone();
        let sounds = std::mem::take(&mut sim.puck.lock().unwrap().sounds);
        let time = chrono_like();
        if sounds.is_empty() {
            println!("{time} shown: {shown}");
        } else {
            println!("{time} shown: {shown}  sound: {}", sounds.join(", "));
        }
    }
}

fn chrono_like() -> String {
    let ms = hn_sim::now_ms();
    let s = (ms / 1000) % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:03}Z",
        s / 3600,
        (s / 60) % 60,
        s % 60,
        ms % 1000
    )
}
