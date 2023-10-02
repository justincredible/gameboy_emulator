use gameboy_core::ByteTransfer;

use std::process::Child;
use std::sync::atomic::{AtomicU8, Ordering};

use raw_sync::locks::{LockImpl, LockInit, Mutex};
use shared_memory::{Shmem, ShmemConf, ShmemError};

pub enum LinkCable {
    Unlinked,
    Linked {
        owning: Option<Child>,
        shmem: Shmem,
        mutex: (Box<dyn LockImpl>, usize),
    },
}

impl LinkCable {
    fn transfer(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { mutex, owning, .. } => mutex.0
                .lock()
                //.try_lock(Timeout::Val(std::time::Duration::from_secs(0)))
                .map_or((), |guard| {
                    let dp = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(4, |_| 0))
                    };
                    let bp = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(0, |_| 4))
                    };
                    let cp = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(5, |_| 1))
                    };
                    let ep = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(1, |_| 5))
                    };
                    let sp = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(6, |_| 2))
                    };
                    let zp = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(2, |_| 6))
                    };
                    let tp = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(7, |_| 3))
                    };
                    let ip = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(3, |_| 7))
                    };

                    match (*sp, *cp, *zp, *ep) {
                        //(0, 0, 0, 0) | (0, 0x7E, 0, 0) | (0, 0, 0, 0x7E) => (),
                        (0, 0x81, 0, 0x80) | (0, 0x81, 0, 0x81) => {
                            *sp = 1;
                            *zp = 1;

                            *tp = 0;
                            *ip = 0;
                        },
                        (1, 0x81, 1, 0x80) | (1, 0x81, 1, 0x81) => {
                            if *tp == 0 && *ip == 0 {
                                let a = *dp;
                                let b = *bp;

                                if *ep == 0x81 {
                                    *dp = b;
                                }
                                *bp = a;
                            } else {
                                if *ep == 0x81 {
                                    *sp = 2;
                                    *cp &= 0x7F;
                                }
                                *zp = 2;
                                *ep &= 0x7F;
                            }

                            *tp += 1;
                            if *ep == 1 {
                                *ip += 1;
                            }
                        },
                        (255, _, _, _) | (_, _, 255, _) => {
                            *dp = 0;
                            *cp = 0;
                            *sp = 0;
                            *tp = 0;
                            *bp = 0;
                            *ep = 0;
                            *zp = 0;
                            *ip = 0;
                        },
                        _ => (),
                    }

                    if *tp == 2 && *ep & 0x80 == 0x80 {
                        *dp = *bp;
                        *sp = 2;
                        *cp &= 0x7F;
                    }
                }),
        }
    }
}

impl ByteTransfer for LinkCable {

    fn send(&mut self, data: u8, control: u8) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { mutex, owning, .. } => {
                mutex.0
                    .lock()
                    //.try_lock(Timeout::Val(std::time::Duration::from_secs(0)))
                    .map_or((), |guard| {
                        let sp = unsafe {
                            &mut *(*guard).add(owning
                                .as_ref()
                                .map_or(6, |_| 2))
                        };

                        if *sp == 0 {
                            let dp = unsafe {
                                &mut *(*guard).add(owning
                                    .as_ref()
                                    .map_or(4, |_| 0))
                            };
                            let cp = unsafe {
                                &mut *(*guard).add(owning
                                    .as_ref()
                                    .map_or(5, |_| 1))
                            };

                            *dp = data;
                            *cp = control;
                        }
                    });

                self.transfer();
            }
        }
    }

    fn receive(&self) -> Option<(bool, u8, u8)> {
        match self {
            LinkCable::Unlinked => None,
            LinkCable::Linked { mutex, owning, .. } => mutex.0
                .lock()
                //.try_lock(Timeout::Val(std::time::Duration::from_secs(0)))
                .map_or(None, |guard| {
                    let data = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(4, |_| 0))
                    };
                    let control = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(5, |_| 1))
                    };
                    let status = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(6, |_| 2))
                    };
                    let counter = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(7, |_| 3))
                    };

                    if *status > 0 {
                        if *status == 2 {
                            *status = 0;
                            *counter = 0;

                            Some((true, *data, *control))
                        } else {
                            Some((false, *data, *control))
                        }
                    } else {
                        None
                    }
                })
        }
    }

    fn disconnected(&self) -> bool {
        match self {
            LinkCable::Unlinked => true,
            LinkCable::Linked { .. } => false,
        }
    }
}

const SHM_SZ: usize = 48;

impl From<(bool, Option<Child>)> for LinkCable {

    fn from(value: (bool, Option<Child>)) -> Self {
        match value {
            (false, None) => LinkCable::Unlinked,
            (_, value) => {
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
                    raw_ptr = raw_ptr.add(8);
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
                            unsafe { *(*guard).add(i) = 0xFF; }
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

                LinkCable::Linked {
                    owning: value,
                    shmem,
                    mutex,
                }
            },
        }
    }
}
