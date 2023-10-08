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
const BIT_LEN: u8 = 8; // arbitrary, master transfer expected duration

#[repr(u8)]
#[derive(PartialEq)]
enum LinkState {
    Disconnect,
    Ready,
    Transfer,
    Complete,
}

pub struct LinkPort {
    owning: Option<Child>,
    _cable: Shmem,
    mutex: (Box<dyn LockImpl>, usize),
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
            })
        }
    }

    unsafe fn data_pointer(&self, guard: &LockGuard<'_>, index: usize) -> &mut u8 {
        &mut *(*guard).add(self.owning
            .as_ref()
            .map_or(index + HALF_LINK, |_| index))
    }

    unsafe fn data_pointer_alt(&self, guard: &LockGuard<'_>, index: usize) -> &mut u8 {
        &mut *(*guard).add(self.owning
            .as_ref()
            .map_or(index, |_| index + HALF_LINK))
    }
}

impl ByteTransfer for LinkPort {

    fn transfer(&mut self, cycles: i32, data: u8, control: u8) -> (bool, u8, u8) {
        self.mutex.0
            .lock()
            .map_or((false, data, control), |guard| {
                let link_data = unsafe { self.data_pointer(&guard, SERIAL_DATA) };
                let link_control = unsafe { self.data_pointer(&guard, SERIAL_CTRL) };
                let link_status = unsafe { self.data_pointer(&guard, LINK_STATE) };

                if *link_status == LinkState::Ready as u8 {
                    *link_data = data;
                    *link_control = control;
                }

                if self.owning.is_some() {
                    // aliasing here is serial
                    let dp = unsafe { self.data_pointer(&guard, SERIAL_DATA) };
                    let bp = unsafe { self.data_pointer_alt(&guard, SERIAL_DATA) };
                    let cp = unsafe { self.data_pointer(&guard, SERIAL_CTRL) };
                    let ep = unsafe { self.data_pointer_alt(&guard, SERIAL_CTRL) };
                    let sp = unsafe { self.data_pointer(&guard, LINK_STATE) };
                    let zp = unsafe { self.data_pointer_alt(&guard, LINK_STATE) };
                    let wp = unsafe { self.data_pointer(&guard, LINK_COUNT) };
                    let vp = unsafe { self.data_pointer_alt(&guard, LINK_COUNT) };

                    match (*sp, *cp, *zp, *ep) {
                        (ra, 0x81, rb, 1) if ra == rb && ra == LinkState::Ready as u8 => (), // courtesy wait is key
                        (ra, 0x81, rb, _) | (ra, 0x80, rb, 0x81)
                        if ra == rb && ra == LinkState::Ready as u8 => {
                            *sp = LinkState::Transfer as u8;
                            *zp = LinkState::Transfer as u8;
                            *wp = 0;
                            *vp = 0;
                        },
                        (ra, 0x80, rb, 0x80) | (ra, 0x80, rb, 0)
                        if ra == rb && ra == LinkState::Ready as u8 => *cp = 0x81,
                        (ra, 0x81, rb, _)
                        if ra == rb && ra == LinkState::Transfer as u8 => {
                            if *wp < BIT_LEN {
                                let remaining = BIT_LEN - *wp;

                                *wp += cycles as u8;

                                let shift_out = u8::min(cycles as u8, remaining);

                                if shift_out == BIT_LEN {
                                    let tmp = *dp;
                                    *dp = *bp;
                                    *bp = tmp;
                                } else {
                                    let shift_in = BIT_LEN - shift_out;

                                    let a = *dp;
                                    let b = *bp;

                                    *dp = a << shift_out | b >> shift_in;
                                    *bp = b << shift_out | a >> shift_in;
                                }

                                if *wp >= BIT_LEN {
                                    *sp = LinkState::Complete as u8;
                                    *zp = LinkState::Complete as u8;
                                    *wp = 0;
                                    *vp = 0;
                                }
                            } else {
                                *sp = LinkState::Complete as u8;
                                *zp = LinkState::Complete as u8;
                                *wp = 0;
                                *vp = 0;
                            }

                        },
                        (d, _, _, _) | (_, _, d, _) if d == LinkState::Disconnect as u8 => {
                            *dp = 0;
                            *bp = 0;
                            *cp = 0;
                            *ep = 0;
                            *sp = LinkState::Ready as u8;
                            *zp = LinkState::Ready as u8;
                            *wp = 0;
                            *vp = 0;
                        },
                        _ => (),
                    }

                }

                match *link_status {
                    c if c == LinkState::Complete as u8 => {
                        *link_status = LinkState::Ready as u8;
                        (true, *link_data, *link_control)
                    },
                    d if d == LinkState::Disconnect as u8 => Default::default(),
                    _ => (false, *link_data, *link_control),
                }
            })
    }

    fn disconnected(&self) -> bool {
        false
    }
}
