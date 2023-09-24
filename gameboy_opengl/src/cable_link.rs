use gameboy_core::ByteTransfer;

use std::process::Child;
use std::sync::atomic::{AtomicU8, Ordering};

use raw_sync::locks::{LockImpl, LockInit, Mutex};
use raw_sync::Timeout;
use shared_memory::{Shmem, ShmemConf, ShmemError};

pub enum LinkState {
    Disconnected,
    Ready,
    Waiting(u8),
}

pub enum LinkCable {
    Unlinked,
    Linked {
        status: LinkState,
        owning: Option<Child>,
        receiving: Option<u8>,
        shmem: Shmem,
        mutex: (Box<dyn LockImpl>, usize),
    },
}

impl ByteTransfer for LinkCable {

    fn send(&mut self, byte: u8) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { mutex, owning, .. } => {
                //mutex.0.lock()
                mutex.0.try_lock(Timeout::Val(std::time::Duration::from_nanos(1)))
                    .and_then(|guard| {
                        let data = unsafe {
                            &mut *(*guard).add(owning
                                .as_ref()
                                .map_or(0, |_| 1))
                        };
                        let alert = unsafe {
                            &mut *(*guard).add(owning
                                .as_ref()
                                .map_or(2, |_| 3))
                        };

                        if *alert != 1 {
                            *data = byte;
                            *alert = 1;
                        }

                        Ok(())
                    })
                    .unwrap_or_default();
            },
        }
    }

    fn receive(&mut self) -> u8 {
        match self {
            LinkCable::Unlinked => unreachable!(),
            LinkCable::Linked { receiving, .. } => {
                receiving
                    .take()
                    .expect("a byte")
            },
        }
    }

    fn step(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { status, .. } => *status = match status {
                LinkState::Ready => LinkState::Waiting(0),
                LinkState::Waiting(c) if *c < 1 => LinkState::Waiting(*c + 1),
                LinkState::Waiting(_) => LinkState::Ready,
                _ => LinkState::Disconnected,
            },
        }
    }

    fn reset(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { status, .. } => *status = LinkState::Ready,
        }
    }

    fn ready(&self) -> bool {
        match self {
            LinkCable::Unlinked => false,
            LinkCable::Linked { status, .. } => match status {
                LinkState::Ready => true,
                _ => false,
            },
        }
    }

    fn waiting(&self) -> bool {
        match self {
            LinkCable::Unlinked => true,
            LinkCable::Linked { status, .. } => match status {
                LinkState::Waiting(_) => true,
                _ => false,
            },
        }
    }

    fn received(&mut self) -> bool {
        match self {
            LinkCable::Unlinked => false,
            LinkCable::Linked { mutex, owning, receiving, .. } => {
                //mutex.0.lock()
                mutex.0.try_lock(Timeout::Val(std::time::Duration::from_nanos(1)))
                    .and_then(|guard| {
                        let data = unsafe {
                            &mut *(*guard).add(owning
                                .as_ref()
                                .map_or(1, |_| 0))
                        };
                        let alert = unsafe {
                            &mut *(*guard).add(owning
                                 .as_ref()
                                 .map_or(3, |_| 2))
                        };

                        if *alert == 1 {
                            *receiving = Some(*data);
                            *alert = 0;
                        }

                        Ok(())
                    })
                    .unwrap_or_default();
                receiving.is_some()
            },
        }
    }

    fn disconnected(&self) -> bool {
        match self {
            LinkCable::Unlinked => true,
            LinkCable::Linked { status, .. } => match status {
                LinkState::Disconnected => true,
                _ => false,
            }
        }
    }

    fn connect(&mut self) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { status, .. } => *status = LinkState::Ready,
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
                        for i in 0..4 {
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
                    receiving: None,
                    status: LinkState::Ready,
                    shmem,
                    mutex,
                }
            },
        }
    }
}
