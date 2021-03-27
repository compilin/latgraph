use std::sync::Arc;
use std::time::Duration;

use clap::{App, Arg};
use conrod_core::utils::clamp;
use log::*;
use rand::{thread_rng, Rng};
use rand_distr::Normal;
use tokio::{
    net::{lookup_host, UdpSocket},
    time,
};

macro_rules! parse_args {
    ($matches:ident, $varname:ident : str = $argname:literal) => {
        let $varname = $matches.value_of($argname).ok_or(concat!("Missing argument ", $argname))?;
    };
    ($matches:ident, $varname:ident : $vartype:ty = $argname:literal) => {
        let $varname = $matches.value_of($argname).ok_or(concat!("Missing argument ", $argname)).map(|v| v.parse::<$vartype>().expect(concat!($argname, " needs to be a valid ", stringify!($argtype))))?;
    };
    ($matches:ident, $varname:ident : $vartype:tt = $argname:literal, $($tail:tt)+) => {
        parse_args!($matches, $varname: $vartype = $argname);
        parse_args!($matches, $($tail)+)
    };
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let matches = App::new("Test UDP echo server")
        .arg(
            Arg::with_name("bind-address")
                .short("b")
                .long("bind-address")
                .default_value("127.0.0.1"),
        )
        .arg(
            Arg::with_name("port")
                .short("p")
                .long("port")
                .default_value("4207"),
        )
        .arg(
            Arg::with_name("avg-lat")
                .short("a")
                .long("avg-lat")
                .default_value("20"),
        )
        .arg(
            Arg::with_name("min-lat")
                .short("m")
                .long("min-lat")
                .default_value("1"),
        )
        .arg(
            Arg::with_name("max-lat")
                .short("M")
                .long("max-lat")
                .default_value("100"),
        )
        .arg(
            Arg::with_name("jitter")
                .short("j")
                .long("jitter")
                .default_value("3"),
        )
        .arg(
            Arg::with_name("loss-chance")
                .short("l")
                .long("loss-chance")
                .default_value(".1"),
        )
        .get_matches();

    parse_args!(
        matches,
        bind_port: u16 = "port",
        avg_lat: f32 = "avg-lat",
        jitter: f32 = "jitter",
        min_lat: u16 = "min-lat",
        max_lat: u16 = "max-lat",
        loss_chance: f32 = "loss-chance",
        bind_addr: str = "bind-address"
    );
    let distr = Normal::new(avg_lat, jitter).unwrap();
    let mut rng = thread_rng();
    let mut next_latency = move || clamp(rng.sample(distr) as u16, min_lat, max_lat);
    let loss_theshold = clamp(loss_chance, 0., 1.);

    let bind_sockaddr = lookup_host((bind_addr, bind_port))
        .await?
        .next()
        .ok_or("Couldn't resolve bind address")?;
    let socket = UdpSocket::bind(bind_sockaddr).await?;
    let socket = Arc::from(socket);

    info!("Starting listen on {}", bind_sockaddr);
    let mut rng = thread_rng();
    let mut buffer = [0u8; 64];
    loop {
        match socket.recv_from(&mut buffer).await {
            Ok((len, addr)) => {
                if loss_theshold > rng.gen() {
                    trace!("Received {} bytes from {}, dropping", len, addr);
                } else {
                    let wait = next_latency() as u64;
                    trace!("Received {} bytes from {}, delaying {}ms", len, addr, wait);
                    let socket = socket.clone();
                    tokio::spawn(async move {
                        time::sleep(Duration::from_millis(wait)).await;
                        socket.send_to(&buffer[..len], addr).await?;
                        Ok(()) as Result<(), std::io::Error>
                    });
                }
            },
            Err(e) => {
                warn!("Got network error {}", e);
            }
        }
    }
}
