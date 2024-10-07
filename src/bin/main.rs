use anyhow::bail;
use tracing::Level;
use tracing_subscriber::{filter, prelude::*};
use trinity::BotConfig;
use std::path::{Path, PathBuf};

// If a path is given, return it if it exists else error out. If a path is not
// given, look in $XDG_CONFIG_DIR/tritongue and return that if it exists else
// return None.
//
// Yes, this suffers from TOCTOU. (But it'll error out later.)
fn config_dir_filename(path: Option<String>, default: &str) -> Result<Option<PathBuf>, anyhow::Error> {
    if let Some(path) = path {
        if Path::new(&path).is_file() {
            Ok(Some(PathBuf::from(&path)))
        } else {
            bail!("config file {} not found", path)
        }
    } else {
        let Some(config_root) = dirs::config_dir() else { bail!("no config_dir directory found") };
        let config_dir = config_root.join("tritongue");
        let rel = config_dir.join(default);
        if rel.is_file() {
            Ok(rel.to_str().map(PathBuf::from))
        } else {
            Ok(None)
        }
    }
}

async fn real_main() -> anyhow::Result<()> {
    let filter = filter::Targets::new()
    .with_target("trinity", Level::DEBUG)
    .with_target("sled", Level::INFO)
    .with_target("hyper::proto", Level::INFO)
    .with_default(Level::INFO);

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .init();

    // This really shouldn't be checked if path is given.
    let config_param = std::env::args().nth(1);
    let Ok(filename) = config_dir_filename(config_param, "config.toml")
        else { anyhow::bail!("error looking for config file") }; // FIXME: Propagate actual error.
    // Check for a config file, then fallback to env if none found.
    let config = if let Some(config_path) = filename {
        tracing::debug!("parsing config {:?}...", config_path.to_string_lossy());
        BotConfig::from_config(Some(String::from(config_path.to_string_lossy())))?
    } else {
        BotConfig::from_env()?
    };

    tracing::debug!("creating client...");
    trinity::run(config).await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // just one trick to get rust-analyzer working in main :-)
    real_main().await
}
