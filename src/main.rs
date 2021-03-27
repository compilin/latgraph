use std::time::Duration;

mod app;
mod ringbuf;
mod widget;

fn main() {
    env_logger::init();
    app::LatGraphApp::start(app::LatGraphSettings {
        remote_host: String::from("127.0.0.1"),
        remote_port: 4207,
        delay: Duration::from_millis(500),
        running: true,
    });
}
