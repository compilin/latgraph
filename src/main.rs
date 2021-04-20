#![cfg_attr(
    not(any(test, debug_assertions, feature = "console")),
    windows_subsystem = "windows"
)]
use std::fs::read;
use std::path::PathBuf;
use std::time::Duration;

use clap::{crate_version, App, Arg, ArgMatches};
use log::*;

mod app;
mod ringbuf;
mod widget;

fn main() {
    #[cfg(all(feature = "config", not(test), not(debug_assertions)))]
    // If we have the `dirs` dependency
    {
        use std::panic;
        let def_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = print_panic(info);
            def_hook(info);
        }));
    }
    run();
}

fn run() {
    env_logger::init();
    let mut app = App::new("LatGraph")
        .about("Real-time network latency graph")
        .author("Compilin, <lin@compilin.dev>")
        .version(crate_version!())
        .arg(Arg::with_name("remote")
            .short("r")
            .long("remote")
            .help("Remote host for the UDP Echo server. Port will be assumed to be 7 if not included (e.g example.org == example.org:7)")
            .takes_value(true))
        .arg(Arg::with_name("rate")
            .short("t")
            .long("rate")
            .help("Polling rate, as the delay in milliseconds between polls")
            .default_value("100"))
        .arg(Arg::with_name("paused")
            .short("p")
            .long("paused")
            .help("Don't immediately start polling the server"))
        .arg(Arg::with_name("running")
            .short("P")
            .long("running")
            .conflicts_with("paused")
            .help("Don't immediately start polling the server"));
    if cfg!(feature = "config") {
        app = app
            .arg(Arg::with_name("config")
                .short("c")
                .long("config")
                .help("Location of the config file to load/save settings from/to. Set to an empty string to not use a config file at all.")
                .takes_value(true))
            .arg(Arg::with_name("no-config-save")
                .short("-C")
                .long("no-config-save")
                .help("Disable the saving of settings to the config file, the file will only be read on startup."));
    }
    let matches = app.get_matches();

    let (config_location, mut settings) = parse_config(&matches);
    if let Some(remote) = matches.value_of("remote") {
        settings.remote_host = String::from(remote);
    }
    if let Some(rate) = matches.value_of("rate") {
        settings.delay =
            Duration::from_millis(rate.parse().expect("Invalid number for rate argument"));
    }
    if matches.is_present("paused") || matches.is_present("running") {
        settings.running = matches.is_present("running");
    }
    settings.running &= !settings.remote_host.is_empty();

    if let Some(path) = &config_location {
        if let Err(err) = settings.save(path) {
            error!("Couldn't save settings: {}", err);
        }
    }

    info!("Starting app with settings {:?}", settings);

    app::LatGraphApp::start(settings, config_location);
}

#[cfg(not(feature = "config"))]
fn parse_config(_: &ArgMatches) -> (Option<PathBuf>, app::LatGraphSettings) {
    (None, app::LatGraphSettings::default())
}

#[cfg(feature = "config")]
fn parse_config(matches: &ArgMatches) -> (Option<PathBuf>, app::LatGraphSettings) {
    let config_path = if let Some(path) = matches.value_of("config") {
        if path.is_empty() {
            info!("Config file path is empty : disabling config feature");
            None
        } else {
            let path = PathBuf::from(path);
            info!("Using config file path: {:?}", path);
            Some(path)
        }
    } else {
        if let Some(mut path) = dirs::config_dir() {
            path.push("latgraph");
            path.push("config.toml");
            info!("Using config file path: {:?}", path);
            Some(path)
        } else {
            error!(
                "Could not determine default config path! Configuration feature will be disabled"
            );
            None
        }
    };

    let mut settings = app::LatGraphSettings::default();
    if let Some(path) = &config_path {
        if path.exists() {
            let config_data = read(path).expect("Couldn't open config file");
            match toml::from_str(&String::from_utf8_lossy(&config_data)) {
                Ok(s) => settings = s,
                Err(e) => error!("Couldn't load settings from file: {}", e),
            }
        }
    }

    (config_path, settings)
}

#[cfg(all(feature = "config", not(test), not(debug_assertions)))]
fn print_panic(info: &std::panic::PanicInfo) -> std::io::Result<()> {
    if let Some(mut path) = dirs::config_dir() {
        path.push("latgraph");
        path.push("error.log");
        let parent = path.parent().unwrap();
        if !parent.is_dir() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;

        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            *s
        } else {
            "No printable error information"
        };
        let location = if let Some(l) = info.location() {
            format!("{}:{}", l.file(), l.line())
        } else {
            String::from("<unknown location>")
        };

        use std::io::Write;
        write!(file, "Application error at {} : {}", location, message)?;
        error!("Panic information written to {:?}", path);
    }
    Ok(())
}
