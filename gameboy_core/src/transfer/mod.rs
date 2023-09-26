use crate::mmu::Memory;
use crate::mmu::interrupt::Interrupt;

pub trait ByteTransfer {

    fn step(&mut self);

    fn send(&mut self, d: u8, c: u8);

    fn receive(&self) -> Option<(u8, u8)>;

    fn ready(&self) -> bool;

    fn disconnected(&self) -> bool;

    fn update(&mut self, mmu: &mut Memory) {
        if self.ready() {
            self.send(mmu.read_byte(0xFF01), mmu.read_byte(0xFF02));
        }

        self.receive()
            .map_or((), |(data, control)| {
                mmu.write_byte(0xFF01, data);
                if self.disconnected() {
                    mmu.write_byte(0xFF01, 0xFF);
                }
                mmu.write_byte(0xFF02, control & 0x7F);
                mmu.request_interrupt(Interrupt::Serial);
            });

        self.step();
    }
}
