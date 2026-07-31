use std::{env, error::Error, path::PathBuf, process::ExitCode};

use pumpkin::{data::VanillaData, init_logger, PumpkinServer};
use pumpkin_config::{LoadConfiguration, PumpkinConfig};
use pumpkin_world::world_info::{anvil::AnvilLevelInfo, WorldInfoReader};

const DEFAULT_ROOT: &str = ".";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Mythicraft server failed to start: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let root = server_root(env::args().skip(1))?;
    env::set_current_dir(&root)?;

    let config = PumpkinConfig::load(std::path::Path::new(DEFAULT_ROOT));
    let vanilla_data = VanillaData::load();
    init_logger(&config.advanced);
    validate_existing_world(&config)?;

    tokio::spawn(async {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("Interrupt received; stopping Mythicraft gracefully");
            pumpkin::stop_or_exit_server();
        }
    });

    tracing::info!(root = %root.display(), "Starting Mythicraft on the Pumpkin runtime");
    tracing::info!(
        "Mythicraft RPG/compatibility crates remain host-side integration modules; Pumpkin owns the Minecraft session, world, tick, and plugin lifecycle"
    );

    let server = PumpkinServer::new(config.basic, config.advanced, vanilla_data).await;
    let plugin_wait = server.init_plugins().await;
    tracing::info!(
        plugin_wait_ms = plugin_wait.as_millis(),
        "Mythicraft plugins initialized"
    );
    server.start().await;

    Ok(())
}

fn validate_existing_world(config: &PumpkinConfig) -> Result<(), Box<dyn Error>> {
    let world_path = config.basic.get_world_path();
    let level_dat = world_path.join("level.dat");
    if !level_dat.exists() {
        tracing::info!(world = %world_path.display(), "No existing level.dat; Pumpkin will initialize the world");
        return Ok(());
    }

    let level = AnvilLevelInfo
        .read_world_info(&world_path)
        .map_err(|error| {
            format!(
                "world preflight failed for {}: {error}",
                world_path.display()
            )
        })?;
    tracing::info!(
        world = %world_path.display(),
        data_version = level.data_version,
        spawn_x = level.spawn_x,
        spawn_y = level.spawn_y,
        spawn_z = level.spawn_z,
        "Existing Anvil world passed Pumpkin world-info preflight"
    );
    Ok(())
}

fn server_root<I>(mut args: I) -> Result<PathBuf, Box<dyn Error>>
where
    I: Iterator<Item = String>,
{
    let mut root = env::var_os("MYTHICRAFT_SERVER_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT));

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--root" => {
                let value = args.next().ok_or("--root requires a directory path")?;
                root = PathBuf::from(value);
            }
            "--help" | "-h" => {
                println!("Usage: mythicraft-server [--root <server-root>]");
                println!("Environment fallback: MYTHICRAFT_SERVER_ROOT");
                std::process::exit(0);
            }
            unknown => return Err(format!("unknown argument: {unknown}").into()),
        }
    }

    if !root.exists() {
        return Err(format!("server root does not exist: {}", root.display()).into());
    }
    if !root.is_dir() {
        return Err(format!("server root is not a directory: {}", root.display()).into());
    }

    Ok(root.canonicalize()?)
}

#[cfg(test)]
mod tests {
    use super::server_root;

    #[test]
    fn parses_root_argument() {
        let root = server_root(["--root".to_owned(), ".".to_owned()].into_iter())
            .expect("current directory is valid");
        assert!(root.is_dir());
    }

    #[test]
    fn rejects_missing_root_value() {
        let error =
            server_root(["--root".to_owned()].into_iter()).expect_err("missing root should fail");
        assert!(error.to_string().contains("requires"));
    }
}
