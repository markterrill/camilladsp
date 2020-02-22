extern crate alsa;
extern crate libpulse_binding as pulse;
extern crate libpulse_simple_binding as psimple;
extern crate rustfft;
extern crate serde;

use std::env;
use std::error;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::{thread, time};

// Sample format
pub type PrcFmt = f64;
pub type Res<T> = Result<T, Box<dyn error::Error>>;

mod alsadevice;
mod audiodevice;
mod basicfilters;
mod biquad;
mod fftconv;
mod filedevice;
mod filters;
mod pulsedevice;
use audiodevice::*;
mod config;
mod fifoqueue;
mod mixer;

use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
//use std::path::PathBuf;

pub enum StatusMessage {
    PlaybackReady,
    CaptureReady,
    PlaybackError { message: String },
    CaptureError { message: String },
    PlaybackDone,
    CaptureDone,
}

fn run(conf: config::Configuration) -> Res<()> {
    let (tx_pb, rx_pb) = mpsc::channel();
    let (tx_cap, rx_cap) = mpsc::channel();

    let (tx_status, rx_status) = mpsc::channel();
    let tx_status_pb = tx_status.clone();
    let tx_status_cap = tx_status.clone();

    let barrier = Arc::new(Barrier::new(4));
    let barrier_pb = barrier.clone();
    let barrier_cap = barrier.clone();
    let barrier_proc = barrier.clone();

    let conf_pb = conf.clone();
    let conf_cap = conf.clone();
    let conf_proc = conf.clone();

    // Processing thread
    thread::spawn(move || {
        let mut pipeline = filters::Pipeline::from_config(conf_proc);
        eprintln!("build filters, waiting to start processing loop");
        barrier_proc.wait();
        loop {
            match rx_cap.recv() {
                Ok(AudioMessage::Audio(mut chunk)) => {
                    chunk = pipeline.process_chunk(chunk);
                    let msg = AudioMessage::Audio(chunk);
                    tx_pb.send(msg).unwrap();
                }
                Ok(AudioMessage::EndOfStream) => {
                    let msg = AudioMessage::EndOfStream;
                    tx_pb.send(msg).unwrap();
                }
                _ => {}
            }
        }
    });

    // Playback thread
    let mut playback_dev = audiodevice::get_playback_device(conf_pb.devices);
    let _pb_handle = playback_dev.start(rx_pb, barrier_pb, tx_status_pb);

    // Capture thread
    let mut capture_dev = audiodevice::get_capture_device(conf_cap.devices);
    let _cap_handle = capture_dev.start(tx_cap, barrier_cap, tx_status_cap);

    let delay = time::Duration::from_millis(1000);

    let mut pb_ready = false;
    let mut cap_ready = false;
    loop {
        match rx_status.recv_timeout(delay) {
            Ok(msg) => match msg {
                StatusMessage::PlaybackReady => {
                    pb_ready = true;
                    if cap_ready {
                        barrier.wait();
                    }
                }
                StatusMessage::CaptureReady => {
                    cap_ready = true;
                    if pb_ready {
                        barrier.wait();
                    }
                }
                StatusMessage::PlaybackError { message } => {
                    eprintln!("Playback error: {}", message);
                    return Ok(());
                }
                StatusMessage::CaptureError { message } => {
                    eprintln!("Capture error: {}", message);
                    return Ok(());
                }
                StatusMessage::PlaybackDone => {
                    eprintln!("Playback finished");
                    return Ok(());
                }
                StatusMessage::CaptureDone => {
                    eprintln!("Capture finished");
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("No config file given!");
        return;
    }
    let configname = &args[1];
    let file = match File::open(configname) {
        Ok(f) => f,
        Err(_) => {
            eprintln!("Could not open config file!");
            return;
        }
    };
    let mut buffered_reader = BufReader::new(file);
    let mut contents = String::new();
    let _number_of_bytes: usize = match buffered_reader.read_to_string(&mut contents) {
        Ok(number_of_bytes) => number_of_bytes,
        Err(_err) => {
            eprintln!("Could not read config file!");
            return;
        }
    };
    let configuration: config::Configuration = match serde_yaml::from_str(&contents) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Invalid config file!");
            eprintln!("{}", err);
            return;
        }
    };

    match config::validate_config(configuration.clone()) {
        Ok(()) => {}
        Err(err) => {
            eprintln!("Invalid config file!");
            eprintln!("{}", err);
            return;
        }
    }
    if let Err(e) = run(configuration) {
        eprintln!("Error ({}) {}", e.description(), e);
    }
}
