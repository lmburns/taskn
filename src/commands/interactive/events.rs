use anyhow::{Context, Result};
use signal_hook::{consts::signal::SIGWINCH, iterator::Signals};
use std::{io, sync::mpsc, thread};
use termion::{event::Key, input::TermRead};

pub(crate) enum Event {
    Key(Key),
    Resize,
}

pub(crate) struct Events {
    rx: mpsc::Receiver<Event>,

    _input_thread:  thread::JoinHandle<()>,
    _signal_thread: thread::JoinHandle<()>,
}

impl Events {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            rx,
            _input_thread: make_input_thread(tx.clone()),
            _signal_thread: make_signal_thread(tx),
        }
    }

    pub(crate) fn next(&self) -> Result<Event> {
        self.rx
            .recv()
            .context("error receiving next item in iterator")
    }
}

fn make_input_thread(tx: mpsc::Sender<Event>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let stdin = io::stdin();
        for key in stdin.keys() {
            tx.send(Event::Key(key.unwrap())).unwrap();
        }
    })
}

fn make_signal_thread(tx: mpsc::Sender<Event>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut signals = Signals::new(&[SIGWINCH]).unwrap();
        loop {
            for signal in &mut signals {
                if signal == SIGWINCH {
                    tx.send(Event::Resize).unwrap();
                }
            }
        }
    })
}
