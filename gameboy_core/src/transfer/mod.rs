use crate::mmu::Memory;
use crate::mmu::interrupt::Interrupt;

pub trait ByteTransfer {

    fn send(&mut self, d: u8, c: u8);

    fn receive(&self) -> (bool, u8, u8);

    fn disconnected(&self) -> bool;

    fn update(&mut self, mmu: &mut Memory) {
        self.send(mmu.read_byte(0xFF01), mmu.read_byte(0xFF02));

        let (interrupt, data, control) = self.receive();

        mmu.write_byte(0xFF01, data);
        mmu.write_byte(0xFF02, control);

        if interrupt {
            mmu.request_interrupt(Interrupt::Serial);
        }

        if self.disconnected() {
            mmu.write_byte(0xFF01, 0xFF);
        }
    }
}
