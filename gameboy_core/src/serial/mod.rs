use crate::mmu::Memory;
use crate::mmu::interrupt::Interrupt;

pub trait ByteTransfer {
    type Wait;
    type Receive;
    type Counter;

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
            if sc & 0x81 == 0x81 {
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

use std::process::Child;

pub enum LinkCable {
    Unlinked,
    Linked(Option<Child>),
}

