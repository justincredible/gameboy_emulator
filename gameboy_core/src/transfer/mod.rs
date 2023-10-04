use crate::mmu::Memory;
use crate::mmu::interrupt::Interrupt;

pub trait ByteTransfer {

    fn transfer(&mut self, sd: u8, sc: u8) -> Option<(bool, u8, u8)>;

    fn disconnected(&self) -> bool;

    fn update(&mut self, mmu: &mut Memory) {
        self.transfer(mmu.read_byte(0xFF01), mmu.read_byte(0xFF02))
            .map(|(complete, data, control)| {
                mmu.write_byte(0xFF01, data);

                if self.disconnected() {
                    mmu.write_byte(0xFF01, 0xFF);
                }

                if complete {
                    mmu.write_byte(0xFF02, control);
                    mmu.request_interrupt(Interrupt::Serial);
                }
            })
            .unwrap_or_default()
    }
}

pub struct Unlinked;

impl ByteTransfer for Unlinked {
    fn transfer(&mut self, _: u8, _: u8) -> Option<(bool, u8, u8)> {
        None
    }

    fn disconnected(&self) -> bool {
        true
    }
}
