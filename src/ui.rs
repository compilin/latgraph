use crate::ringbuf::{Ping, RingBuffer};
use std::convert::TryFrom;
use std::net::UdpSocket;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::task::Poll;
use std::thread;
use std::time::{Duration, Instant};

use iced::{
    executor, Application, Column, Command, Container, Element, Length, Subscription,
    Text,
};
use iced_futures::futures::stream;
use iced_native::subscription::Recipe;
use log::{debug, error, warn};

#[derive(Debug)]
pub enum Event {
    Ping(),
    Pong(u64),
}

pub struct LatViewApp {
    ringbuf: RingBuffer,
    running: AtomicBool,
    show_settings: bool,
    settings: LatViewFlags,
    socket: UdpSocket,
    event_rx: Arc<Mutex<mpsc::Receiver<Event>>>,
    event_tx: mpsc::Sender<Event>,
    tick_thead_stop: Option<Arc<AtomicBool>>,
    error: Option<String>
}

pub struct LatViewFlags {
    pub remote_host: String,
    pub remote_port: u16,
    pub delay: Duration,
}

const MIN_DELAY: Duration = Duration::from_millis(50);
const MAX_DELAY: Duration = Duration::from_secs(5);

impl Application for LatViewApp {
    type Executor = executor::Default;
    type Message = Event;
    type Flags = LatViewFlags;

    fn new(flags: Self::Flags) -> (Self, Command<Self::Message>) {
        let (tx, rx) = mpsc::channel();
        let rx = Arc::new(Mutex::new(rx));

        let mut app = LatViewApp {
            ringbuf: RingBuffer::new(),
            running: AtomicBool::from(!flags.remote_host.is_empty()),
            show_settings: flags.remote_host.is_empty(),
            settings: flags,
            socket: UdpSocket::bind("0.0.0.0:0").expect("Couldn't bind network socket"),
            event_rx: rx,
            event_tx: tx,
            tick_thead_stop: None,
            error: None,
        };

        let socket_clone = app
            .socket
            .try_clone()
            .expect("Couldn't clone network socket handle");
        let net_tx = app.event_tx.clone();
        thread::spawn(move || {
            let mut buf = [0; 8];
            let mut last_error = String::new();
            let mut error_count = 0;
            loop {
                match socket_clone.recv(&mut buf) {
                    Ok(_) => {
                        let id = u64::from_ne_bytes(buf);
                        net_tx.send(Event::Pong(id)).unwrap();
                        error_count = 0;
                    }
                    Err(e) => {
                        if e.to_string() != last_error {
                            last_error = e.to_string();
                            warn!("Got an error on the network listen thread: {}", last_error);
                        } else {
                            error_count += 1;
                            if error_count >= 4 {
                                panic!("Network listen thread got 5 error in a row!");
                            }
                        }
                    }
                }
            }
        });

        if *app.running.get_mut() {
            if let Err(e) = app.start_sending() {
                app.catch_error(e);
            }
        }

        (app, Command::none())
    }

    fn title(&self) -> String {
        String::from("UDP Latency Viewer")
    }

    fn update(&mut self, event: Self::Message) -> Command<Self::Message> {
        debug!("Event: {:?}", event);
        match event {
            Event::Ping() => {
                if *self.running.get_mut() {
                    let i = self.ringbuf.get_next_index();
                    self.ringbuf.sent(Instant::now());
                    if let Err(e) = self.socket.send(&i.to_ne_bytes()) {
                        self.catch_error(e);
                        self.stop_sending();
                    }
                } else {
                    self.stop_sending();
                }
            }
            Event::Pong(id) => {
                self.ringbuf.received(id);
            }
        }
        Command::none()
    }

    fn view(&mut self) -> Element<Self::Message> {
        let mut text = String::with_capacity(self.ringbuf.len() * 9);
        let mut i = self.ringbuf.get_start_index();
        for ping in self.ringbuf.iter() {
            if !text.is_empty() {
                text += " | ";
            }
            text = format!("{}{:>3}: ", text, i);
            match ping {
                Ping::None => panic!(),
                Ping::Sent(_) => text += "…",
                Ping::Received(lat) => {
                    text = format!("{}{:>3}ms", text, lat.to_string());
                }
            }

            i += 1;
        }

        let col = Column::new().push(Text::new(text));
        Container::new(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(20)
            .center_x()
            .center_y()
            .into()
    }

    fn subscription(&self) -> Subscription<Event> {
        iced::Subscription::from_recipe(EventStreamRecipe {
            rx: Arc::clone(&self.event_rx),
        })
    }
}

impl LatViewApp {
    fn start_sending(&mut self) -> std::io::Result<()> {
        self.stop_sending();
        if !self.settings.remote_host.is_empty() {
            self.socket
                .connect((&self.settings.remote_host[..], self.settings.remote_port))?;
            *self.running.get_mut() = true;
        }

        let stop_flag = Arc::from(AtomicBool::from(false));
        self.tick_thead_stop = Some(stop_flag.clone());
        let delay = self.settings.delay;
        let tx = self.event_tx.clone();

        thread::spawn(move || {
            let start = Instant::now();
            loop {
                tx.send(Event::Ping()).unwrap();
                let elapsed = start.elapsed().as_millis();
                let next_sleep = (elapsed / delay.as_millis() + 1) * delay.as_millis() - elapsed;
                let next_sleep = Duration::from_millis(u64::try_from(next_sleep).unwrap());
                thread::sleep(next_sleep);

                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
            }
        });
        Ok(())
    }

    fn stop_sending(&mut self) {
        *self.running.get_mut() = false;
        if let Some(stop_flag) = &mut self.tick_thead_stop {
            stop_flag.store(true, Ordering::Relaxed);
            self.tick_thead_stop = None;
        }
    }

    fn catch_error(&mut self, e: std::io::Error) {
        error!("Got error: {}", e);
        self.error = Some(String::from(e.to_string()));
    }
}

struct EventStreamRecipe {
    rx: Arc<Mutex<mpsc::Receiver<Event>>>,
}

impl<H, I> Recipe<H, I> for EventStreamRecipe
where
    H: std::hash::Hasher,
{
    type Output = Event;

    fn hash(&self, state: &mut H) {
        use std::hash::Hash;

        std::any::TypeId::of::<Self>().hash(state);
    }

    fn stream(
        self: Box<Self>,
        _input: stream::BoxStream<'static, I>,
    ) -> stream::BoxStream<'static, Self::Output> {
        Box::pin(stream::poll_fn(move |_| -> Poll<Option<Event>> {
            let rx = self.rx.lock().unwrap();
            match rx.recv() {
                Ok(v) => Poll::Ready(Some(v)),
                Err(_) => Poll::Ready(None),
            }
        }))
    }
}