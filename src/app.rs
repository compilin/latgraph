use crate::{ringbuf::RingBuffer, widget::LatencyGraphWidget};
use std::{
    hash::Hash,
    io::Cursor,
    net::UdpSocket,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use conrod_core::{
    color, image::Map, text::Font, widget, widget_ids, Borderable, Colorable, Positionable,
    Sizeable, Ui, UiBuilder, Widget,
};
use conrod_glium::Renderer;
use glium::{
    self,
    glutin::{
        dpi::LogicalSize,
        event::{ElementState, Event, KeyboardInput, StartCause, VirtualKeyCode, WindowEvent},
        event_loop::{ControlFlow, EventLoop, EventLoopProxy},
        window::WindowBuilder,
        ContextBuilder,
    },
    Display, Surface, Texture2d,
};
use log::*;
use thread_priority::ThreadPriority;
use winit::window::Icon;

pub struct LatGraphApp {
    ringbuf: RingBuffer,
    settings: LatGraphSettings,
    settings_tx: mpsc::Sender<LatGraphSettings>,
    config_path: Option<PathBuf>,
    display: Display,
    ui: Ui,
    widget_ids: Ids,
    image_map: Map<Texture2d>,
    renderer: Renderer,
    is_mouse_over_window: bool,
}

#[cfg_attr(
    feature = "config",
    derive(serde_derive::Serialize, serde_derive::Deserialize)
)]
#[derive(Clone, Debug, Hash)]
pub struct LatGraphSettings {
    pub running: bool,
    pub remote_host: String,
    pub zoom: (u16, u16),
    pub delay: Duration,
}

widget_ids! {
    struct Ids {
        canvas,
        grid,
        graph,
        status_bar
    }
}

#[derive(Debug)]
enum AppEvent {
    Ping(Instant),
    Pong(u64, Instant),
    Error(AppError),
}

#[derive(Debug)]
enum AppError {
    HostResolution,
}

impl LatGraphApp {
    pub fn start(settings: LatGraphSettings, config_path: Option<PathBuf>) {
        let (settings_tx, settings_rx) = mpsc::channel();

        let (mut app, event_loop) = LatGraphApp::init_ui(settings_tx);
        app.config_path = config_path;

        LatGraphApp::init_network(settings_rx, event_loop.create_proxy());

        app.settings = settings;

        info!("Starting event loop");
        app.run_loop(event_loop);
    }

    fn init_network(
        settings_rx: mpsc::Receiver<LatGraphSettings>,
        event_tx_rcv: EventLoopProxy<AppEvent>,
    ) {
        debug!("Initializing network threads");
        let socket_tx = UdpSocket::bind("0.0.0.0:0").expect("Couldn't bind network socket");
        let socket_rx = socket_tx.try_clone().unwrap();
        let event_tx_snd = event_tx_rcv.clone();

        // Sender thread
        thread::spawn(move || {
            let event_tx = event_tx_snd;
            let mut settings = LatGraphSettings::default();
            let mut new_settings = false;
            let mut new_remote = false;
            let mut valid_remote = false; // Whether we managed to ever send a ping to the current remote
            let mut next_ping = Instant::now();
            let mut ping_id = 0u64;
            if let Err(e) = ThreadPriority::Max.set_for_current() {
                warn!("Couldn't set thread priority : {:?}", e);
            }

            loop {
                if settings.running {
                    debug!("SND: Sending ping");
                    let now = Instant::now();
                    if let Err(_) = event_tx.send_event(AppEvent::Ping(now)) {
                        break;
                    }
                    if let Err(e) = socket_tx.send(&ping_id.to_ne_bytes()) {
                        warn!("SND: Couldn't send ping ({}), attempting reconnect", e);

                        let mut addr = settings.remote_host.clone();
                        if !addr.contains(":") {
                            addr += ":7";
                        }
                        if let Err(e) = socket_tx
                            .connect(addr)
                            .and_then(|_| socket_tx.send(&ping_id.to_ne_bytes()))
                        {
                            next_ping += Duration::from_secs(3);
                            if valid_remote { // If we could send a ping to the host at least once, keep trying again
                                error!("SND: Reconnect failed ({}), waiting 3s", e);
                            } else { // Otherwise return a host resolution error
                                error!("SND: Reconnect failed ({}), giving up", e);
                                if let Err(_) =
                                    event_tx.send_event(AppEvent::Error(AppError::HostResolution))
                                {
                                    break;
                                }
                                settings.running = false;
                            }
                        }
                    } else {
                        valid_remote = true;
                    }
                    ping_id += 1;
                    next_ping = next_ping + settings.delay;
                    if next_ping < Instant::now() {
                        // If we're already past the next ping (process lagged a lot, computer went to sleep, etc),
                        next_ping = Instant::now() + settings.delay;
                    }
                    thread::sleep(next_ping - Instant::now());

                    // Poll for new settings, using 'while' in case there's multiple values queued
                    for set in settings_rx.try_iter() {
                        settings = set;
                        new_settings = true;
                    }
                } else {
                    match settings_rx.recv() {
                        Ok(set) => {
                            new_remote = set.remote_host != settings.remote_host;
                            settings = set;
                        }
                        Err(_) => break, // Main thread is probably shutting down, just exit
                    }
                    new_settings = true;
                }

                if new_settings {
                    debug!("SND: Received new settings {:#?}", settings);
                    new_settings = false;

                    // If remote host settings have changed
                    if new_remote && !settings.remote_host.is_empty() {
                        valid_remote = false;
                        info!("SND: Connecting to new host");
                        let mut addr = settings.remote_host.clone();
                        if !addr.contains(":") {
                            addr += ":7";
                        }
                        if let Err(e) = socket_tx.connect(addr) {
                            error!("SND: Couldn't connect to host ({})", e);
                            if let Err(_) =
                                event_tx.send_event(AppEvent::Error(AppError::HostResolution))
                            {
                                break;
                            }
                            settings.running = false;
                        }
                        new_remote = false;
                    }

                    settings.running &= !settings.remote_host.is_empty();
                }
            }
            debug!("SND: Stopping send thread");
        });

        // Receiver thread
        thread::spawn(move || {
            let event_tx = event_tx_rcv;
            let mut buf = [0u8; 8];
            if let Err(e) = ThreadPriority::Max.set_for_current() {
                warn!("Couldn't set thread priority : {:?}", e);
            }

            loop {
                match socket_rx.recv(&mut buf) {
                    Ok(_) => {
                        let id = u64::from_ne_bytes(buf);
                        debug!("RCV: Received ping {}", id);
                        if let Err(_) = event_tx.send_event(AppEvent::Pong(id, Instant::now())) {
                            break;
                        }
                    }
                    Err(e) => debug!("RCV: Got err on receiver thread : {}", e),
                }
            }
            debug!("RCV: Stopping receiver thread");
        });
    }

    fn init_ui(settings_tx: mpsc::Sender<LatGraphSettings>) -> (LatGraphApp, EventLoop<AppEvent>) {
        const WIDTH: u32 = 800;
        const HEIGHT: u32 = 400;
        let font_data = include_bytes!("resources/WorkSans-Regular.ttf");
        let app_icon_data = include_bytes!("resources/icon.png");
        let app_icon =
            image::io::Reader::with_format(Cursor::new(app_icon_data), image::ImageFormat::Png)
                .decode()
                .unwrap()
                .to_rgba8();

        let event_loop = EventLoop::with_user_event();
        let window = WindowBuilder::new()
            .with_title("Latency Graph")
            .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
            .with_window_icon(Some(
                Icon::from_rgba(app_icon.to_vec(), app_icon.width(), app_icon.height()).unwrap(),
            ));
        let context = ContextBuilder::new().with_vsync(true)/* .with_multisampling(4) */;
        let display =
            Display::new(window, context, &event_loop).expect("Couldn't instanciate display");

        let mut ui = UiBuilder::new([(WIDTH + 1) as f64, (HEIGHT + 1) as f64]).build();
        let font = Font::from_bytes(font_data).expect("Couldn't load font");
        ui.fonts.insert(font);

        let widget_ids = Ids::new(ui.widget_id_generator());

        let image_map = Map::<Texture2d>::new();
        let renderer = Renderer::new(&display).expect("Couldn't instanciate renderer");

        (
            LatGraphApp {
                ringbuf: RingBuffer::new(1000),
                settings: LatGraphSettings::default(),
                settings_tx,
                config_path: None,
                display,
                ui,
                widget_ids,
                image_map,
                renderer,
                is_mouse_over_window: false,
            },
            event_loop,
        )
    }

    fn set_ui(&mut self, needs_redraw: &mut bool) {
        let ui = &mut self.ui.set_widgets();
        let ids = &self.widget_ids;

        widget::Canvas::new()
            .color(color::DARK_CHARCOAL)
            .border(0.)
            .set(ids.canvas, ui);

        self.settings.zoom =
            LatencyGraphWidget::new(&self.ringbuf, &self.settings, self.is_mouse_over_window)
                .color(color::LIGHT_BLUE)
                .missing_color(color::rgba_bytes(192, 64, 32, 0.3))
                .border_color(color::LIGHT_BLUE)
                .wh_of(ids.canvas)
                .middle_of(ids.canvas)
                .set(ids.graph, ui);

        *needs_redraw = ui.has_changed();
    }

    fn process_event(
        &mut self,
        event: &Event<AppEvent>,
        should_update_ui: &mut bool,
        should_exit: &mut bool,
    ) {
        if let Some(event) = convert_event(event, self.display.gl_window().window()) {
            self.ui.handle_event(event);
            *should_update_ui = true;
        }

        match event {
            Event::UserEvent(event) => {
                debug!("Processing app event {:?}", event);
                match event {
                    AppEvent::Ping(time) => {
                        self.ringbuf.sent(*time);
                    }
                    AppEvent::Pong(id, time) => {
                        self.ringbuf.received(*id, *time);
                    }
                    AppEvent::Error(AppError::HostResolution) => {
                        error!("Received a Host Resolution error, exiting");
                        *should_exit = true;
                    }
                }
                *should_update_ui = true;
            }
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    *should_exit = true;
                }
                WindowEvent::KeyboardInput {
                    input:
                        KeyboardInput {
                            virtual_keycode: Some(VirtualKeyCode::Space),
                            state: ElementState::Pressed,
                            ..
                        },
                    ..
                } => {
                    self.toggle_running();
                }
                WindowEvent::CursorLeft { .. } => {
                    self.is_mouse_over_window = false;
                }
                WindowEvent::CursorEntered { .. } => {
                    self.is_mouse_over_window = true;
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn redraw(&mut self) {
        trace!("Redrawing");
        // Render the `Ui` and then display it on the screen.
        let primitives = self.ui.draw();

        self.renderer
            .fill(&self.display, primitives, &self.image_map);
        let mut target = self.display.draw();
        target.clear_color(0., 0., 0., 1.0);
        self.renderer
            .draw(&self.display, &mut target, &self.image_map)
            .unwrap();
        target.finish().unwrap();
    }

    fn send_settings(&self) {
        self.settings_tx.send(self.settings.clone()).unwrap();
    }

    fn toggle_running(&mut self) {
        self.set_running(!self.settings.running);
    }

    fn set_running(&mut self, running: bool) {
        if running != self.settings.running && (!running || !self.settings.remote_host.is_empty()) {
            info!(
                "Toggling packet sending {}",
                if running { "ON" } else { "OFF" }
            );
            self.settings.running = running;
            self.send_settings();
        }
    }

    /*
        Copyright (c) 2014 PistonDevelopers
        This function is copied from conrod's examples with minor modifications, and distributed under the MIT licence
    */
    fn run_loop(mut self, event_loop: EventLoop<AppEvent>) -> ! {
        let redraw_delay = std::time::Duration::from_millis(16);
        // let redraw_delay = std::time::Duration::from_millis(16);
        let mut next_update = None;
        let mut ui_update_needed = false;
        self.send_settings(); // Send initial settings to start the send thread
        event_loop.run(move |event, _, control_flow| {
            {
                let mut should_update_ui = false;
                let mut should_exit = false;
                self.process_event(&event, &mut should_update_ui, &mut should_exit);
                ui_update_needed |= should_update_ui;
                if should_exit {
                    *control_flow = ControlFlow::Exit;
                    return;
                }
            }
            // We don't want to draw any faster than 60 FPS, so set the UI only on every 16ms, unless:
            // - this is the very first event, or
            // - we didn't request update on the last event and new events have arrived since then.
            let should_set_ui_on_main_events_cleared = next_update.is_none() && ui_update_needed;
            match (&event, should_set_ui_on_main_events_cleared) {
                (Event::NewEvents(StartCause::Init { .. }), _)
                | (Event::NewEvents(StartCause::ResumeTimeReached { .. }), _)
                | (Event::MainEventsCleared, true) => {
                    trace!(
                        "Setting UI. Event: {:?}, should_set_ui_on_main_events_cleared: {} && {}",
                        event,
                        next_update.is_none(),
                        ui_update_needed
                    );
                    next_update = Some(std::time::Instant::now() + redraw_delay);
                    ui_update_needed = false;
                    let mut needs_redraw = false;
                    self.set_ui(&mut needs_redraw);
                    if needs_redraw {
                        self.display.gl_window().window().request_redraw();
                    } else {
                        // We don't need to redraw anymore until more events arrives.
                        next_update = None;
                    }
                }
                _ => {}
            }
            if let Some(next_update) = next_update {
                *control_flow = ControlFlow::WaitUntil(next_update);
            } else {
                *control_flow = ControlFlow::Wait;
            }
            // Request redraw if needed.
            match &event {
                Event::RedrawRequested(_) => {
                    self.redraw();
                }
                _ => {}
            }
        })
    }
}

impl LatGraphSettings {
    #[cfg(not(feature = "config"))]
    pub fn save(&self, _: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    #[cfg(feature = "config")]
    pub fn save(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        use std::fs::OpenOptions;
        use std::io::Write;

        debug!("Saving config to file {:?}", path);
        let ser = toml::to_string_pretty(self)?;
        let parent = path.parent().unwrap();
        if !parent.is_dir() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        file.write(ser.as_bytes())?;
        file.flush()?;

        Ok(())
    }
}

impl Default for LatGraphSettings {
    fn default() -> Self {
        LatGraphSettings {
            remote_host: String::new(),
            delay: Duration::from_millis(100),
            running: false,
            zoom: (crate::widget::ZOOM_DEFAULT, crate::widget::ZOOM_DEFAULT),
        }
    }
}

conrod_winit::v023_conversion_fns!();
