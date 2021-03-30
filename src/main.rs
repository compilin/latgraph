use std::time::Duration;

use clap::{App,Arg};
use log::info;

mod app;
mod ringbuf;
mod widget;

fn main() {
    let matches = App::new("LatGraph - Real-time network latency graph")
        .arg(Arg::with_name("remote")
            .short("r")
            .long("remote")
            .help("Remote host for the UDP Echo server. Port will be assumed to be 7 if not included (e.g example.org:1234)")
            .default_value(""))
        .arg(Arg::with_name("rate")
            .short("t")
            .long("rate")
            .help("Polling rate, as the delay in milliseconds between polls")
            .default_value("100"))
        .arg(Arg::with_name("paused")
            .short("p")
            .long("paused")
            .help("Don't immediately start polling the server")
            .takes_value(false))
        .get_matches();

    let remote = matches.value_of("remote").unwrap();
    let rate = matches.value_of("rate").unwrap().parse().expect("Invalid number for rate argument");
    let running = matches.occurrences_of("paused") == 0;

    let settings = app::LatGraphSettings {
        remote_host: String::from(remote),
        delay: Duration::from_millis(rate),
        running: running,
        zoom: Default::default(),
    };
    info!("Starting app with settings {:?}", settings);

    env_logger::init();
    app::LatGraphApp::start(settings);
}
