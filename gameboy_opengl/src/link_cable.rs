use gameboy_core::{ByteTransfer, Unlinked};

use std::process::Child;
use std::sync::atomic::{AtomicU8, Ordering};

use raw_sync::locks::{LockGuard, LockImpl, LockInit, Mutex};
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
    _cable: Shmem,
    mutex: (Box<dyn LockImpl>, usize),
    counter: u16,
}

impl LinkPort {

    pub fn from_linkage((linked, link): (bool, Option<Child>)) -> Box<dyn ByteTransfer> {
        if !linked && link.is_none() {
            Box::new(Unlinked)
        } else {
            // 8 aligned init byte and mutex plus 8 for the link
            const SHM_SZ: usize = 48;

            let shmem = match ShmemConf::new().size(SHM_SZ).flink("link_cable").create() {
                Ok(m) => m,
                Err(ShmemError::LinkExists) => ShmemConf::new().flink("link_cable").open().unwrap(),
                Err(e) => {
                    panic!("Unable to create or open shmem flink link_cable : {:?}", e);
                },
            };

            let mut raw_ptr = shmem.as_ptr();
            let is_init: &mut AtomicU8;

            unsafe {
                is_init = &mut *(raw_ptr as *mut u8 as *mut AtomicU8);
                raw_ptr = raw_ptr.add(8); // align mutex
            }

            let mutex = if shmem.is_owner() {
                is_init.store(0, Ordering::Relaxed);
                let mutex = unsafe {
                    Mutex::new(
                        raw_ptr,
                        raw_ptr.add(Mutex::size_of(Some(raw_ptr))),
                    )
                    .unwrap()
                };
                {
                    let guard = mutex.0.lock().unwrap();
                    for i in 0..8 {
                        unsafe { *(*guard).add(i) = LinkState::Disconnect as u8; }
                    }
                }
                is_init.store(1, Ordering::Relaxed);
                mutex
            } else {
                while is_init.load(Ordering::Relaxed) != 1 {}

                unsafe {
                    Mutex::from_existing(
                        raw_ptr,
                        raw_ptr.add(Mutex::size_of(Some(raw_ptr))),
                    )
                    .unwrap()
                }
            };

            Box::new(LinkPort {
                owning: link,
                _cable: shmem,
                mutex,
                counter: 0,
            })
        }
    }

    unsafe fn data_pointer(&self, remote: bool, guard: &LockGuard<'_>, index: usize) -> &mut u8 {
        let (a,b) = if remote {
            (index, index + HALF_LINK)
        } else {
            (index + HALF_LINK, index)
        };

        &mut *(*guard).add(self.owning.as_ref().map_or(a, |_| b))
    }
}

impl ByteTransfer for LinkPort {

    fn transfer(&mut self, _cycles: i32, data: u8, control: u8) -> Option<(u8, u8)> {
        self.counter += 1;
        // Dance of the borrow checker
        let mut reset = false;
        let mut result = None;

        if let Ok(guard) = self.mutex.0.lock() {
            let dp = unsafe { self.data_pointer(false, &guard, SERIAL_DATA) };
            let bp = unsafe { self.data_pointer(true, &guard, SERIAL_DATA) };
            let cp = unsafe { self.data_pointer(false, &guard, SERIAL_CTRL) };
            let ep = unsafe { self.data_pointer(true, &guard, SERIAL_CTRL) };
            let sp = unsafe { self.data_pointer(false, &guard, LINK_STATE) };
            let zp = unsafe { self.data_pointer(true, &guard, LINK_STATE) };
            let wp = unsafe { self.data_pointer(false, &guard, LINK_COUNT) };
            let vp = unsafe { self.data_pointer(true, &guard, LINK_COUNT) };

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
            // First time called and first process to call
            if *sp == *zp && *sp == LinkState::Disconnect as u8 {
                *dp = 0xFF;
                *bp = 0xFF;
                *cp = 0;
                *ep = 0;
                *sp = LinkState::Ready as u8;
                *zp = LinkState::Ready as u8;
                *wp = 0;
                *vp = 0;
                reset = true;
            }
            // Signal and update emulator
            if *sp == LinkState::Complete as u8 {
                *sp = LinkState::Ready as u8;
                reset = true;
                result = Some((*dp, *cp));
            }
        }

        // And thus the borrow checker abides
        if reset {
            self.counter = 0;
        }

        result
    }

    // Not implemented, could implement `Drop` for `LinkPort` to signal the other process
    // But that would add additional blocking in `drop` and `disconnected`
    fn disconnected(&self) -> bool {
        false
    }
}

