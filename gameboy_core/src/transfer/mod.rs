use crate::mmu::Memory;
use crate::mmu::interrupt::Interrupt;

pub trait ByteTransfer {

    fn disconnected(&self) -> bool;

    fn transfer(&mut self, sd: u8, sc: u8) -> (bool, u8, u8);

    fn update(&mut self, mmu: &mut Memory) {
        let (is_complete, data, control) = self.transfer(
            mmu.read_byte(0xFF01),
            mmu.read_byte(0xFF02)
        );

        mmu.write_byte(0xFF01, data);
        mmu.write_byte(0xFF02, control);

        if self.disconnected() {
            mmu.write_byte(0xFF01, 0xFF);
        }

        if is_complete {
            mmu.request_interrupt(Interrupt::Serial);
        }
    }
}

pub struct Unlinked;

impl ByteTransfer for Unlinked {

    fn disconnected(&self) -> bool {
        true
    }

    fn transfer(&mut self, _: u8, _: u8) -> (bool, u8, u8) {
        Default::default()
    }
}
