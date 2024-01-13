use gameboy_core::{ByteTransfer, Unlinked};

use std::process::Child;

use shared_memory::{Shmem, ShmemConf, ShmemError};

const SERIAL_DATA: usize = 0;
const SERIAL_CTRL: usize = 1;
const LINK_STATE: usize = 2;
const LINK_COUNT: usize = 3;
const HALF_LINK: usize = 4;
// Arbitrary threshold in the range 4K - 16K
// Noticeable delays during heavy transfers but a higher value is generally more reliable
const TRANSFER_DELAY: u16 = 8192;

#[repr(u8)]
#[derive(
    Clone, Copy,
    Debug,
    PartialEq,
)]
enum LinkState {
    Complete,
    Disconnect,
    Ready,
    Receive,
    Transfer,
}

pub struct LinkPort {
    owning: Option<Child>,
    cable: Shmem,
    counter: u16,
}

impl LinkPort {

    pub fn from_linkage((linked, link): (bool, Option<Child>)) -> Box<dyn ByteTransfer> {
        if !linked && link.is_none() {
            Box::new(Unlinked)
        } else {
            // only 6 bytes needed, but rounding up to power of 2
            const SHM_SZ: usize = 8;

            let shmem = match ShmemConf::new().size(SHM_SZ).flink("link_cable").create() {
                Ok(m) => m,
                Err(ShmemError::LinkExists) => ShmemConf::new().flink("link_cable").open().unwrap(),
                Err(e) => {
                    panic!("Unable to create or open shmem flink link_cable : {:?}", e);
                },
            };

            if shmem.is_owner() {
                let mut raw_ptr = shmem.as_ptr();

                unsafe {
                    *raw_ptr = 0xFF;
                    raw_ptr = raw_ptr.add(1);
                    *raw_ptr = 0;
                    raw_ptr = raw_ptr.add(1);
                    *raw_ptr = LinkState::Ready as u8;
                    raw_ptr = raw_ptr.add(1);
                    *raw_ptr = 0;
                    raw_ptr = raw_ptr.add(1);
                    *raw_ptr = 0xFF;
                    raw_ptr = raw_ptr.add(1);
                    *raw_ptr = 0;
                    raw_ptr = raw_ptr.add(1);
                    *raw_ptr = LinkState::Ready as u8;
                    raw_ptr = raw_ptr.add(1);
                    *raw_ptr = 0;
                }
            }

            Box::new(LinkPort {
                owning: link,
                cable: shmem,
                counter: 0,
            })
        }
    }
}

impl ByteTransfer for LinkPort {

    fn transfer(&mut self, _cycles: i32, data: u8, control: u8) -> Option<(u8, u8)> {
        self.counter += 1;

        unsafe {
            let (dp, bp, cp, ep, sp, zp, _wp, _vp) = if self.owning.is_some() {
                (
                    self.cable.as_ptr().add(SERIAL_DATA),
                    self.cable.as_ptr().add(SERIAL_DATA + HALF_LINK),
                    self.cable.as_ptr().add(SERIAL_CTRL),
                    self.cable.as_ptr().add(SERIAL_CTRL + HALF_LINK),
                    self.cable.as_ptr().add(LINK_STATE),
                    self.cable.as_ptr().add(LINK_STATE + HALF_LINK),
                    self.cable.as_ptr().add(LINK_COUNT),
                    self.cable.as_ptr().add(LINK_COUNT + HALF_LINK),
                )
            } else {
                (
                    self.cable.as_ptr().add(SERIAL_DATA + HALF_LINK),
                    self.cable.as_ptr().add(SERIAL_DATA),
                    self.cable.as_ptr().add(SERIAL_CTRL + HALF_LINK),
                    self.cable.as_ptr().add(SERIAL_CTRL),
                    self.cable.as_ptr().add(LINK_STATE + HALF_LINK),
                    self.cable.as_ptr().add(LINK_STATE),
                    self.cable.as_ptr().add(LINK_COUNT + HALF_LINK),
                    self.cable.as_ptr().add(LINK_COUNT),
                )
            };

            // Begin transfer
            if *cp & 0x81 == 0x81 && *zp == LinkState::Receive as u8 {
                *sp = LinkState::Transfer as u8;
                *zp = LinkState::Transfer as u8;
            }
            // Complete transfer
            if *cp & 0x81 == 0x81 && *zp == LinkState::Transfer as u8 && self.counter >= TRANSFER_DELAY {
                let tmp = *dp;
                *dp = *bp;
                *bp = tmp;

                *sp = LinkState::Complete as u8;
                *cp &= 0x7F;
                *zp = LinkState::Complete as u8;
                *ep &= 0x7F;
            }
            // Update linked memory with serial bytes
            if *sp == LinkState::Ready as u8 || *sp == LinkState::Receive as u8 {
                *dp = data;
                *cp = control;

                if *cp & 0x80 == 0x80 {
                    *sp = LinkState::Receive as u8;
                }
            }
            // Signal and update emulator
            if *sp == LinkState::Complete as u8 {
                *sp = LinkState::Ready as u8;
                self.counter = 0;
                return Some((*dp, *cp));
            }
        }

        None
    }

    // Not implemented, could implement `Drop` for `LinkPort` to signal the other process
    // But that would add additional blocking in `drop` and `disconnected`
    fn disconnected(&self) -> bool {
        false
    }
}

