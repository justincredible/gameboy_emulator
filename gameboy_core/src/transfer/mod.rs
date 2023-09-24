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
                }
                if sdc.1 & 0x81 == 0x81 {
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
