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

    fn connect(&mut self);

    fn update(&mut self, mmu: &mut Memory) {
        if self.disconnected() {
            self.connect()
        } else {
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
}

use std::io::{Read, stdin, stdout, Write};
use std::process::Child;

pub enum LinkState {
    Ready,
    Waiting(u8),
    Disconnected,
}

pub enum LinkCable {
    Unlinked,
    Linked {
        owning: Option<Child>,
        receiving: Option<u8>,
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
                    .write(&[byte])
                    .expect("error other than interrupted");
                stdout().flush().expect("I/O error or EOF");
            },
            LinkCable::Linked { owning: Some(gameboy), .. } => {
                let pipe = gameboy.stdin
                    .as_mut()
                    .expect("pipe to child stdin");
                pipe
                    .write(&[byte])
                    .expect("error other than interrupted");
                pipe.flush().expect("I/O error or EOF");
            },
        }
    }

    fn receive(&mut self) -> u8 {
        match self {
            LinkCable::Unlinked => unreachable!(),
            LinkCable::Linked { receiving, .. } => {
                receiving
                    .take()
                    .expect("a byte")
            },
        }
    }

    fn step(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { status, .. } => *status = match status {
                LinkState::Ready => LinkState::Waiting(0),
                LinkState::Waiting(c) if *c < 1 => LinkState::Waiting(*c + 1),
                LinkState::Waiting(_) => LinkState::Ready,
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
                    let mut buffer = [0];

                    let result = stdin().read(&mut buffer);
                    let count = result.expect("a byte");
                    if count > 0 {
                        *receiving = Some(buffer[0]);
                    }
                }

                receiving.is_some()
            },
            LinkCable::Linked { owning: Some(gameboy), receiving, .. } => {
                if receiving.is_none() {
                    let mut buffer = [0];

                    let result = gameboy.stdout
                        .as_mut()
                        .expect("pipe to child stdout")
                        .read(&mut buffer);
                    let count = result.expect("a byte");
                    if count > 0 {
                        *receiving = Some(buffer[0]);
                    }
                }

                receiving.is_some()
            },
        }
    }

    fn disconnected(&self) -> bool {
        match self {
            LinkCable::Unlinked => true,
            LinkCable::Linked { status, .. } => match status {
                LinkState::Disconnected => true,
                _ => false,
            }
        }
    }

    fn connect(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { owning: None, status, .. } => {
                stdout()
                    .write(&[0xFF])
                    .expect("stdout piped");
                stdout().flush().expect("IO error");
                let mut buffer = [0];
                let response = stdin()
                    .read(&mut buffer)
                    .expect("the same byte value");
                if response == 0xFF {
                    *status = LinkState::Ready;
                }
            },
            LinkCable::Linked { owning: Some(gameboy), status, .. } => {
                let pipe = gameboy.stdin
                    .as_mut()
                    .expect("pipe to child stdin");
                pipe
                    .write(&[0xFF])
                    .expect("stdout piped");
                pipe.flush().expect("IO error");
                let mut buffer = [0];
                let response = gameboy.stdout
                    .as_mut()
                    .expect("pipe to child stdout")
                    .read(&mut buffer)
                    .expect("the same byte value");
                if response == 0xFF {
                    *status = LinkState::Ready;
                }
            },
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
                status: LinkState::Disconnected,
            }
        }
    }
}
