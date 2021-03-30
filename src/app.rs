use crate::{ringbuf::RingBuffer, widget::LatencyGraphWidget};
use conrod_glium::Renderer;
use std::{
    net::UdpSocket,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use conrod_core::{image::Map, widget_ids, Ui, UiBuilder};
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

pub struct LatGraphApp {
    ringbuf: RingBuffer,
    settings: LatGraphSettings,
    settings_tx: mpsc::Sender<LatGraphSettings>,
    display: Display,
    ui: Ui,
    widget_ids: Ids,
    image_map: Map<Texture2d>,
    renderer: Renderer,
}

#[derive(Clone, Debug)]
pub struct LatGraphSettings {
    pub running: bool,
    pub remote_host: String,
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
    pub fn start(settings: LatGraphSettings) {
        let (settings_tx, settings_rx) = mpsc::channel();

        let (mut app, event_loop) = LatGraphApp::init_ui(settings_tx);

        LatGraphApp::init_network(settings_rx, event_loop.create_proxy());

        app.settings = settings;

        info!("Starting event loop");
        app.run_loop(event_loop);
    }

    // fn load_settings() -> LatGraphSettings {
    //     LatGraphSettings::default()
    // }

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
            let mut prev_remote = String::new();
            let mut new_settings = false;
            let mut next_ping = Instant::now();
            let mut ping_id = 0u64;

            loop {
                if settings.running {
                    debug!("SND: Sending ping");
                    let now = Instant::now();
                    if let Err(e) = socket_tx.send(&ping_id.to_ne_bytes()) {
                        error!("SND: Couldn't send ping ({})", e);
                        std::process::exit(1); // Kill the process instead of panicking only this thread
                    }
                    if let Err(_) = event_tx.send_event(AppEvent::Ping(now)) {
                        break;
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
                        Ok(set) => settings = set,
                        Err(_) => break, // Main thread is probably shutting down, just exit
                    }
                    new_settings = true;
                }

                if new_settings {
                    debug!("SND: Received new settings {:#?}", settings);
                    new_settings = false;

                    // If remote host settings have changed
                    if prev_remote != settings.remote_host && !settings.remote_host.is_empty() {
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
                        prev_remote = settings.remote_host.clone();
                    }
                }
            }
            debug!("SND: Stopping send thread");
        });

        // Receiver thread
        thread::spawn(move || {
            let event_tx = event_tx_rcv;
            let mut buf = [0u8; 8];

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

        let event_loop = EventLoop::with_user_event();
        let window = WindowBuilder::new()
            .with_title("Latency Graph")
            .with_inner_size(LogicalSize::new(WIDTH, HEIGHT));
        let context = ContextBuilder::new().with_vsync(true).with_multisampling(4);
        let display =
            Display::new(window, context, &event_loop).expect("Couldn't instanciate display");

        let mut ui = UiBuilder::new([WIDTH as f64, HEIGHT as f64]).build();
        ui.fonts
            .insert_from_file("C:\\Windows\\Fonts\\LatoWeb-Regular.ttf")
            .expect("Couldn't load font");
        let widget_ids = Ids::new(ui.widget_id_generator());

        let image_map = Map::<Texture2d>::new();
        let renderer = Renderer::new(&display).expect("Couldn't instanciate renderer");

        (
            LatGraphApp {
                ringbuf: RingBuffer::new(10000),
                settings: LatGraphSettings::default(),
                settings_tx,
                display,
                ui,
                widget_ids,
                image_map,
                renderer,
            },
            event_loop,
        )
    }

    fn set_ui(&mut self, needs_redraw: &mut bool) {
        use conrod_core::{color, widget, Colorable, Positionable, Sizeable, Widget};

        let ui = &mut self.ui.set_widgets();
        let ids = &self.widget_ids;

        widget::Canvas::new()
            .color(color::DARK_CHARCOAL)
            .set(ids.canvas, ui);

        LatencyGraphWidget::new(&self.ringbuf, self.settings.delay, 8)
            .color(color::LIGHT_BLUE)
            .line_thickness(1.5)
            .point_thickness(3.)
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
                        todo!();
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
        target.clear_color(0.0, 0.0, 0.0, 1.0);
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

impl Default for LatGraphSettings {
    fn default() -> Self {
        LatGraphSettings {
            remote_host: String::new(),
            delay: Duration::from_millis(100),
            running: false,
        }
    }
}

conrod_winit::v023_conversion_fns!();
