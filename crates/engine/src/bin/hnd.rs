//! Headless HumanNeeded app (same engine as the tray app, no UI). For CI, integration tests and
//! servers. Environment: `HN_PUCK` (auto | serial:COM7 | tcp:host:port | sim | none), `HN_IPC`,
//! `HN_LATEST_URL`, `HN_UF2_DRIVE`, `HN_DATA_DIR`, `CLAUDE_CONFIG_DIR`.

use hn_engine::{log, paths, Config, Engine};

#[tokio::main]
async fn main() {
    log::init(paths::data_dir().join("logs"));
    let engine = match Engine::start(Config::from_env()).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("hnd: {e} (already running?)");
            std::process::exit(1);
        }
    };
    let _ = tokio::signal::ctrl_c().await;
    engine.shutdown().await;
}
