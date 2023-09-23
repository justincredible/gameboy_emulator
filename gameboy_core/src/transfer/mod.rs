use crate::mmu::Memory;
use crate::mmu::interrupt::Interrupt;

pub trait ByteTransfer {

    fn send(&mut self, byte: u8);

    fn receive(&mut self) -> u8;

    fn step(&mut self);

    fn reset(&mut self);

    fn ready(&self) -> bool;

    fn waiting(&self) -> bool;

    fn received(&mut self) -> bool;

    fn disconnected(&self) -> bool;

    fn update(&mut self, mmu: &mut Memory) {
        let sdc = (mmu.read_byte(0xFF01), mmu.read_byte(0xFF02));

        if self.ready() {
            if sdc.1 & 0x80 == 0x80 {
                self.send(sdc.0);
                self.step();
            }
        }

        if self.received() {
            mmu.write_byte(0xFF01, self.receive());
            mmu.write_byte(0xFF02, sdc.1 & 0x7F);
            mmu.request_interrupt(Interrupt::Serial);
            self.reset();
        } else if self.waiting() {
            self.step();
        }
    }
}

use std::io::{Read, stdin, stdout, Write};
use std::process::Child;
use std::thread::JoinHandle;

pub enum LinkState {
    Ready,
    Waiting(u8),
    TimedOut,
    Disconnected,
}

pub enum LinkCable {
    Unlinked,
    Linked {
        owning: Option<Child>,
        receiving: Option<JoinHandle<u8>>,
        status: LinkState,
    },
}

impl ByteTransfer for LinkCable {

    fn send(&mut self, byte: u8) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { owning: None, .. } => {
                // windows may have an issue with stdout
                stdout()
                    .write_all(&[byte])
                    .expect("error other than interrupted");
                stdout().flush().expect("I/O error or EOF");
            },
            LinkCable::Linked { owning: Some(linked_gameboy), .. } => {},
        }
    }

    fn receive(&mut self) -> u8 {
        match self {
            LinkCable::Unlinked => unreachable!(),
            LinkCable::Linked { owning: None, receiving, .. } => {
                receiving
                    .take()
                    .expect("a completed thread")
                    .join()
                    .expect("a byte")
            },
            LinkCable::Linked{ owning: Some(linked_gameboy), .. } => 0,
        }
    }

    fn step(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { status, .. } => *status = match status {
                LinkState::Ready => LinkState::Waiting(0),
                LinkState::Waiting(c) if *c < 8 => LinkState::Waiting(*c + 1),
                LinkState::Waiting(_) => LinkState::TimedOut,
                _ => LinkState::Disconnected,
            },
        }
    }

    fn reset(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { status, .. } => *status = LinkState::Ready,
        }
    }

    fn ready(&self) -> bool {
        match self {
            LinkCable::Unlinked => false,
            LinkCable::Linked { status, .. } => match status {
                LinkState::Ready => true,
                _ => false,
            },
        }
    }

    fn waiting(&self) -> bool {
        match self {
            LinkCable::Unlinked => true,
            LinkCable::Linked { status, .. } => match status {
                LinkState::Waiting(_) => true,
                _ => false,
            },
        }
    }

    fn received(&mut self) -> bool {
        match self {
            LinkCable::Unlinked => false,
            LinkCable::Linked { owning: None, receiving, .. } => {
                if receiving.is_none() {
                    *receiving = std::thread::Builder::new()
                        .spawn(|| {
                            let mut buffer = [0];

                            let mut result = stdin().read(&mut buffer);
                            while let Err(error) = result {
                                eprintln!("{:?}", error);
                                result = stdin().read(&mut buffer);
                            }
                            let count = unsafe {
                                result.unwrap_unchecked()
                            };
                            if count > 0 {
                                buffer[0]
                            } else {
                                panic!("unexpected count: {:?}", count);
                            }
                        })
                        .ok();
                }

                receiving
                    .as_ref()
                    .map(|thread| thread.is_finished())
                    .unwrap_or_default()
            },
            LinkCable::Linked{ owning: Some(linked_gameboy), .. } => false,
        }
    }

    fn disconnected(&self) -> bool {
        match self {
            LinkCable::Unlinked => true,
            LinkCable::Linked { .. } => false,
        }
    }
}

impl From<(bool, Option<Child>)> for LinkCable {

    fn from(value: (bool, Option<Child>)) -> Self {
        match value {
            (false, None) => LinkCable::Unlinked,
            (_, value) => LinkCable::Linked {
                owning: value,
                receiving: None,
                status: LinkState::Ready,
            }
        }
    }
}
