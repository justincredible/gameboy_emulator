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
        owning: Option<Child>,
        receiving: Option<u8>,
        status: LinkState,
        shmem: Shmem,
        mutex: Box<dyn LockImpl>,
    },
}

impl ByteTransfer for LinkCable {

    fn send(&mut self, byte: u8) {
        match self {
            LinkCable::Unlinked => (),
            LinkCable::Linked { mutex, .. } => {
                let mut guard = mutex.lock().unwrap();
                let val: &mut u8 = unsafe { &mut **guard };
                *val = byte;
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
            LinkCable::Linked { receiving, mutex, .. } => {
                mutex.try_lock(Timeout::Val(std::time::Duration::from_secs(0)))
                    .and_then(|mut guard| {
                        let val: &mut u8 = unsafe { &mut **guard };

                        *receiving = Some(*val);

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
            LinkCable::Linked { .. } => {
            },
        }
    }
}

impl From<(bool, Option<Child>)> for LinkCable {

    fn from(value: (bool, Option<Child>)) -> Self {
        match value {
            (false, None) => LinkCable::Unlinked,
            (_, value) => {
                let shmem = match ShmemConf::new().size(16).flink("link_cable").create() {
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
                    let (lock, _) = unsafe {
                        Mutex::new(
                            raw_ptr,
                            raw_ptr.add(Mutex::size_of(Some(raw_ptr))),
                        )
                        .unwrap()
                    };
                    is_init.store(1, Ordering::Relaxed);
                    lock
                } else {
                    while is_init.load(Ordering::Relaxed) != 1 {}
                    let (lock, _) = unsafe {
                        Mutex::from_existing(
                            raw_ptr,
                            raw_ptr.add(Mutex::size_of(Some(raw_ptr))),
                        )
                        .unwrap()
                    };
                    lock
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
