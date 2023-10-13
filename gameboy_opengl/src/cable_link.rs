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
const TIMEOUT: u8 = 8;

#[repr(u8)]
#[derive(PartialEq)]
enum LinkState {
    Disconnect,
    Ready,
    Complete,
}

pub struct LinkPort {
    owning: Option<Child>,
    _cable: Shmem,
    mutex: (Box<dyn LockImpl>, usize),
    connect: bool,
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
                connect: false,
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

    fn disconnected(&self) -> bool {
        !self.connect
    }

    fn transfer(&mut self, data: u8, control: u8) -> (bool, u8, u8) {
        let mut connected = true;

        let status = self.mutex.0
            .lock()
            .map_or((false, data, control), |guard| {
                let link_data = unsafe { self.data_pointer(false, &guard, SERIAL_DATA) };
                let link_control = unsafe { self.data_pointer(false, &guard, SERIAL_CTRL) };
                let link_status = unsafe { self.data_pointer(false, &guard, LINK_STATE) };

                if *link_status == LinkState::Ready as u8 {
                    *link_data = data;
                    *link_control = control;
                }

                // aliasing here is serial
                let dp = unsafe { self.data_pointer(false, &guard, SERIAL_DATA) };
                let bp = unsafe { self.data_pointer(true, &guard, SERIAL_DATA) };
                let cp = unsafe { self.data_pointer(false, &guard, SERIAL_CTRL) };
                let ep = unsafe { self.data_pointer(true, &guard, SERIAL_CTRL) };
                let sp = unsafe { self.data_pointer(false, &guard, LINK_STATE) };
                let zp = unsafe { self.data_pointer(true, &guard, LINK_STATE) };
                let wp = unsafe { self.data_pointer(false, &guard, LINK_COUNT) };
                let vp = unsafe { self.data_pointer(true, &guard, LINK_COUNT) };

                match (*sp, *cp, *zp, *ep) {
                    // transfer delay states
                    (ra, _, rb, 1) | (ra, 1, rb, _) | (ra, 0, rb, 0x80)
                    if ra == rb && ra == LinkState::Ready as u8 => (),
                    // timeout disconnect
                    (ra, 0x80, rb, 0)
                    if ra == rb && ra == LinkState::Ready as u8 => {
                        if *wp < TIMEOUT {
                            *wp += 1;
                        } else {
                            *sp = LinkState::Disconnect as u8;
                        }
                    },
                    // otherwise transfer
                    (ra, 0x81, rb, _) | (ra, _, rb, 0x81) | (ra, 0x80, rb, _) | (ra, _, rb, 0x80)
                    if ra == rb && ra == LinkState::Ready as u8 => {
                        let tmp = *dp;
                        *dp = *bp;
                        *bp = tmp;

                        *sp = LinkState::Complete as u8;
                        *zp = LinkState::Complete as u8;
                        *cp &= 0x7F;
                        *ep &= 0x7F;
                        *wp = 0;
                        *vp = 0;
                    },
                    // post init state
                    (da, db, dc, dd) if da == LinkState::Disconnect as u8
                        && da == db && db == dc && dc == dd => {
                        *dp = 0;
                        *bp = 0;
                        *cp = 0;
                        *ep = 0;
                        *sp = LinkState::Ready as u8;
                        *zp = LinkState::Ready as u8;
                        *wp = 0;
                        *vp = 0;
                    },
                    // check reconnect
                    (d, _, _, _) | (_, _, d, _) if d == LinkState::Disconnect as u8 => {
                        if *cp & 0x80 == 0x80 && *ep & 0x80 == 0x80 {
                            *sp = LinkState::Ready as u8;
                            *zp = LinkState::Ready as u8;
                            *wp = 0;
                            *vp = 0;
                        }
                    },
                    _ => (),
                }

                match *link_status {
                    c if c == LinkState::Complete as u8 => {
                        *link_status = LinkState::Ready as u8;
                        (true, *link_data, *link_control)
                    },
                    d if d == LinkState::Disconnect as u8 => {
                        connected = false;
                        Default::default()
                    },
                    _ => (false, *link_data, *link_control),
                }
            });

        self.connect = connected;

        status
    }
}
