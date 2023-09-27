use gameboy_core::ByteTransfer;

use std::process::Child;
use std::sync::atomic::{AtomicU8, Ordering};

use raw_sync::locks::{LockImpl, LockInit, Mutex};
use raw_sync::Timeout;
use shared_memory::{Shmem, ShmemConf, ShmemError};

pub enum LinkCable {
    Unlinked,
    Linked {
        owning: Option<Child>,
        shmem: Shmem,
        mutex: (Box<dyn LockImpl>, usize),
        status: LinkState
    },
}

#[derive(PartialEq)]
pub enum LinkState {
    Ready,
    Receiving,
    Transferring,
    Disconnected,
}

impl ByteTransfer for LinkCable {

    fn send(&mut self, data: u8, control: u8) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { mutex, owning, status, .. } => mutex.0
                .lock()
                .map_or((), |guard| {
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

                    if control == 0x81 {
                        *status = LinkState::Transferring;
                    }
                }),
        }
    }

    fn receive(&mut self) -> Option<(u8, u8)> {
        match self {
            LinkCable::Unlinked => Some((0xFF, 0xFF)),
            LinkCable::Linked { mutex, owning, status, .. } => if *status == LinkState::Receiving {
                mutex.0
                    .lock()
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
                        let status_self = unsafe {
                            &mut *(*guard).add(owning
                                .as_ref()
                                .map_or(6, |_| 2))
                        };

                        *status_self = 0;
                        *status = LinkState::Ready;

                        Some((*data, *control))
                    })
            } else {
                None
            },
        }
    }

    fn step(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { mutex, owning, status, .. } => mutex.0
                .try_lock(Timeout::Val(std::time::Duration::from_secs(0)))
                .map_or((), |guard| {
                    let data_self = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(4, |_| 0))
                    };
                    let data_other = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(0, |_| 4))
                    };
                    let ctrl_self = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(5, |_| 1))
                    };
                    let ctrl_other = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(1, |_| 5))
                    };
                    let status_self = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(6, |_| 2))
                    };
                    let status_other = unsafe {
                        &mut *(*guard).add(owning
                            .as_ref()
                            .map_or(2, |_| 6))
                    };

                    let state = (*ctrl_self, *ctrl_other, *status_self, *status_other);
                    match state {
                        (0x80, 0x80, 0, 0)
                        | (0x80, 0, 0, 0) | (0, 0x80, 0, 0)
                        | (0x80, 1, 0, 0) | (1, 0x80, 0, 0) => (),
                        (0x81, 0x81, 0, 0)
                        | (0x81, 0x80, 0, 0) | (0x80, 0x81, 0, 0)
                        | (0x81, 0, 0, 0) | (0, 0x81, 0, 0)
                        | (0x81, 1, 0, 0) | (1, 0x81, 0, 0) => {
                            *status_self = 1;
                            *status_other = 1;
                        },
                        (0x81, 0x81, 1, 1)
                        | (0x81, 0x80, 1, 1) | (0x80, 0x81, 1, 1)
                        | (0x81, 0, 1, 1) | (0, 0x81, 1, 1)
                        | (0x81, 1, 1, 1) | (1, 0x81, 1, 1) => {
                            let temp = *data_self;
                            *data_self = *data_other;
                            *data_other = temp;

                            if *ctrl_self == 0x81 {
                                *status_other = 2;
                            }
                            if *ctrl_other == 0x81 {
                                *status_self = 2;
                            }

                        },
                        (_, _, 1, 0) => {
                            *status_self = 0;
                            *status = LinkState::Ready;
                        },
                        (_, _, 0, 1) => (),
                        (_, _, 2, _) => *status = LinkState::Receiving,
                        (_, _, _, 2) => (),
                        (_, _, 0xFF, 0xFF) => {
                            *data_self = 0;
                            *data_other = 0;
                            *ctrl_self = 0;
                            *ctrl_other = 0;
                            *status_self = 0;
                            *status_other = 0;
                        },
                        (0, 0, 0, 0)
                        | (1, 0, 0, 0) | (0, 1, 0, 0)
                        | (0x7E, 0, 0, 0) | (0, 0x7E, 0, 0) => (),
                        _ => {
                            println!("Unexpected state: {:?}", state);
                        },
                    }

                }),
        }
    }

    fn ready(&self) -> bool {
        match self {
            LinkCable::Unlinked => false,
            LinkCable::Linked { status, .. } => *status == LinkState::Ready,
        }
    }

    fn disconnected(&self) -> bool {
        match self {
            LinkCable::Unlinked => true,
            LinkCable::Linked { status, .. } => *status == LinkState::Disconnected,
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
                    status: LinkState::Ready,
                }
            },
        }
    }
}
