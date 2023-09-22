use crate::mmu::Memory;
use crate::mmu::interrupt::Interrupt;

pub trait ByteTransfer {

    fn send(&mut self, byte: u8);

    fn receive(&mut self) -> u8;

    fn wait(&mut self);

    fn idle(&mut self);

    fn count(&mut self);

    fn waiting(&self) -> bool;

    fn timeout(&self) -> bool;

    fn received(&mut self) -> bool;

    fn disconnected(&self) -> bool;

    fn step(&mut self, mmu: &mut Memory) {
        let sdc = (mmu.read_byte(0xFF01), mmu.read_byte(0xFF02));

        if !self.waiting() {
            if sdc.1 & 0x80 == 0x80 {
                self.send(sdc.0);
            }
            if sdc.1 & 0x81 == 0x81 {
                self.wait();
            }
        }

        if self.received() {
            mmu.write_byte(0xFF01, self.receive());
            mmu.write_byte(0xFF02, sdc.1 & 0x7F);
            mmu.request_interrupt(Interrupt::Serial);
            self.idle();
        } else if self.waiting() && self.timeout() {
            self.idle();
        } else if self.waiting() {
            self.count();
        }
    }
}

use std::io::{Read, stdin, stdout, Write};
use std::process::Child;
use std::thread::JoinHandle;

pub enum LinkCable {
    Unlinked,
    Linked {
        owning: Option<Child>,
        waiting: bool,
        counting: u8,
        receiving: Option<JoinHandle<u8>>,
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
            LinkCable::Unlinked => panic!(""),
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

    fn wait(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { waiting, .. } => *waiting = true,
        }
    }

    fn idle(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { waiting, counting, .. } => {
                *waiting = false;
                *counting = 0;
            },
        }
    }

    fn count(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { counting, .. } => *counting += 1,
        }
    }

    fn waiting(&self) -> bool {
        match self {
            LinkCable::Unlinked => true,
            LinkCable::Linked { waiting, .. } => *waiting,
        }
    }

    fn timeout(&self) -> bool {
        match self {
            LinkCable::Unlinked => false,
            LinkCable::Linked { counting, .. } => *counting >= 8,
        }
    }

    fn received(&mut self) -> bool {
        match self {
            LinkCable::Unlinked => false,
            LinkCable::Linked { owning: None, receiving, .. } => {
                receiving
                    .get_or_insert(std::thread::spawn(|| {
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
                    }))
                    .is_finished()
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
                waiting: false,
                counting: 0,
                receiving: None,
            }
        }
    }
}
