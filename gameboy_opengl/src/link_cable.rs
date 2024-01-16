use gameboy_core::{ByteTransfer, Unlinked};

use std::process::Child;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};

use raw_sync::locks::{LockGuard, LockImpl, LockInit, Mutex};
use shared_memory::{Shmem, ShmemConf, ShmemError};

const SERIAL_DATA: usize = 0;
const SERIAL_CTRL: usize = 1;
const LINK_STATE: usize = 2;
const LINK_COUNT: usize = 3;
const HALF_LINK: usize = 4;

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
}

struct LinkCable {
    cable: Shmem,
    mutex: (Box<dyn LockImpl>, usize),
    sender: Sender<(u8, u8)>,
    receiver: Receiver<(i32, (u8, u8))>,
}

impl LinkCable {

    pub fn new(sender: Sender<(u8, u8)>, receiver: Receiver<(i32, (u8, u8))>) -> Self {
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

        LinkCable {
            cable: shmem,
            mutex,
            sender,
            receiver,
        }
    }

    pub fn run(self) {
        // loop indefinitely, but the Shmem and Mutex need to be dropped correctly on program exit
        'thread: loop {
            // Grab the lastest message
            let mut odc = None;
            while let Ok((cycles, dc)) = self.receiver.try_recv() {
                if cycles < 0 {
                    break 'thread;
                } else {
                    odc = Some(dc);
                }
            }

            if let Ok(guard) = self.mutex.0.lock() {
                let dp = unsafe { self.data_pointer(false, &guard, SERIAL_DATA) };
                let bp = unsafe { self.data_pointer(true, &guard, SERIAL_DATA) };
                let cp = unsafe { self.data_pointer(false, &guard, SERIAL_CTRL) };
                let ep = unsafe { self.data_pointer(true, &guard, SERIAL_CTRL) };
                let sp = unsafe { self.data_pointer(false, &guard, LINK_STATE) };
                let zp = unsafe { self.data_pointer(true, &guard, LINK_STATE) };
                let wp = unsafe { self.data_pointer(false, &guard, LINK_COUNT) };
                let vp = unsafe { self.data_pointer(true, &guard, LINK_COUNT) };

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
                }
                // Do transfer
                if *cp & 0x81 == 0x81 && *zp == LinkState::Receive as u8 {
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
                    if let Some((data, control)) = odc {
                        *dp = data;
                        *cp = control;

                        if *cp & 0x80 == 0x80 {
                            *sp = LinkState::Receive as u8;
                        }
                    }
                }
                // Signal and update emulator
                if *sp == LinkState::Complete as u8 {
                    *sp = LinkState::Ready as u8;
                    if let Err(e) = self.sender.send((*dp, *cp)) {
                        eprintln!("{:?}", e);
                    }
                }
            }
        }
    }

    unsafe fn data_pointer(&self, remote: bool, guard: &LockGuard<'_>, index: usize) -> &mut u8 {
        let offset = if self.cable.is_owner() && !remote || !self.cable.is_owner() && remote {
            index
        } else {
            index + HALF_LINK
        };

        &mut *(*guard).add(offset)
    }
}

pub struct LinkPort {
    _owning: Option<Child>,
    sender: Sender<(i32, (u8, u8))>,
    receiver: Receiver<(u8, u8)>,
    last_sent: (u8, u8),
}

impl LinkPort {

    pub fn from_linkage((linked, gameboy): (bool, Option<Child>)) -> Box<dyn ByteTransfer> {
        if !linked && gameboy.is_none() {
            Box::new(Unlinked)
        } else {
            let (ps, pr) = channel();
            let (cs, cr) = channel();

            std::thread::spawn(move || { LinkCable::new(cs, pr).run() });

            Box::new(LinkPort {
                _owning: gameboy,
                sender: ps,
                receiver: cr,
                last_sent: (0xFF, 0),
            })
        }
    }
}

impl ByteTransfer for LinkPort {

    fn transfer(&mut self, cycles: i32, data: u8, control: u8) -> Option<(u8, u8)> {
        // This side of the channel runs frequently, communicate changes only
        if self.last_sent != (data, control) {
            if let Err(e) = self.sender.send((cycles, (data, control))) {
                eprintln!("{:?}", e);
            } else {
                self.last_sent = (data, control);
            }
        }

        self.receiver.try_recv().ok()
    }

    // Not implemented
    fn disconnected(&self) -> bool {
        false
    }
}

impl Drop for LinkPort {

    fn drop(&mut self) {
        // Signal the thread to terminate
        if let Err(e) = self.sender.send((-1,(0xFF,0))) {
            eprintln!("{:?}", e);
        }
    }
}
