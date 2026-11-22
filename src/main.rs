mod config;
mod input;
mod lua;
#[cfg(target_os = "macos")]
mod macos;
mod output;
mod runtime;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oh-my-gamepad — {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "macos")]
fn run() -> config::Result<()> {
    let home = std::env::var_os("HOME").ok_or("переменная HOME не задана")?;
    let directory = PathBuf::from(home).join(".config/oh-my-gamepad");
    let config_path = directory.join("config.yaml");
    let config = config::Config::read(&config_path)?;
    let sources = lua::PluginSources::discover(&directory.join("plugins"))?;
    config
        .validate_plugins(&sources.names())
        .map_err(|err| format!("{} — {err}", config_path.display()))?;

    let output = macos::MacOutput::new()?;
    output::init_logger();
    let stop = macos::shutdown_signal()?;
    let plugins = sources.instantiate()?;
    let mut runtime = runtime::Runtime::new(config, plugins, output);
    runtime
        .start()
        .map_err(|err| format!("{} — {err}", config_path.display()))?;
    tracing::info!("программа запущена");
    let result = macos::run(
        |event| match event {
            Some((measurement, time)) => runtime.measure(measurement, time),
            None => runtime.tick(),
        },
        stop,
    );
    runtime.shutdown();
    result
}

#[cfg(not(target_os = "macos"))]
fn run() -> config::Result<()> {
    Err("для запуска требуется macOS".into())
}
