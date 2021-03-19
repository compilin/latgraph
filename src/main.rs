use std::time::Duration;
use iced::{Application, Settings};

mod ui;
mod ringbuf;

fn main() {
    env_logger::init();
    
    ui::LatViewApp::run(Settings::with_flags(ui::LatViewFlags {
        remote_host: String::from("127.0.0.1"),
        remote_port: 4207,
        delay: Duration::from_millis(1000)
    })).unwrap();
}