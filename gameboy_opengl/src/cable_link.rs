use gameboy_core::{ByteTransfer, Unlinked};

use std::process::Child;
use std::sync::atomic::{AtomicU8, Ordering};

use raw_sync::locks::{LockGuard, LockImpl, LockInit, Mutex};
use shared_memory::{Shmem, ShmemConf, ShmemError};

pub struct LinkCable {
    owning: Option<Child>,
    _shmem: Shmem,
    mutex: (Box<dyn LockImpl>, usize),
}

impl LinkCable {

    pub fn from_init(linked: bool, link: Option<Child>) -> Box<dyn ByteTransfer> {
        if !linked && link.is_none() {
            Box::new(Unlinked)
        } else {
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

            Box::new(LinkCable {
                owning: link,
                _shmem: shmem,
                mutex,
            })
        }
    }

    fn data_pointer(&self, guard: &LockGuard<'_>, index: usize) -> &mut u8 {
        unsafe {
            &mut *(*guard).add(self.owning
                .as_ref()
                .map_or(index + 4, |_| index))
        }
    }

    fn data_pointer_alt(&self, guard: &LockGuard<'_>, index: usize) -> &mut u8 {
        unsafe {
            &mut *(*guard).add(self.owning
                .as_ref()
                .map_or(index, |_| index + 4))
        }
    }
}

impl ByteTransfer for LinkCable {

    fn sync(&mut self, data: u8, control: u8) -> Option<(bool, u8, u8)> {
        self.mutex.0
            .lock()
            //.try_lock(Timeout::Val(std::time::Duration::from_secs(0)))
            .map(|guard| {
                let link_data = self.data_pointer(&guard, 0);
                let link_control = self.data_pointer(&guard, 1);
                let link_status = self.data_pointer(&guard, 2);

                if *link_status == 0 {
                    *link_data = data;
                    *link_control = control;
                }

                let dp = self.data_pointer(&guard, 0);
                let bp = self.data_pointer_alt(&guard, 0);
                let cp = self.data_pointer(&guard, 1);
                let ep = self.data_pointer_alt(&guard, 1);
                let sp = self.data_pointer(&guard, 2);
                let zp = self.data_pointer_alt(&guard, 2);
                let wp = self.data_pointer(&guard, 3);
                let vp = self.data_pointer_alt(&guard, 3);

                match (*sp, *cp, *zp, *ep) {
                    (0, 0x81, 0, 0x80) | (0, 0x81, 0, 0x81) => {
                        let a = *dp;
                        let b = *bp;

                        *dp = b;
                        *bp = a;
                        *sp = 2;
                        *zp = 2;
                    },
                    (0, 0x80, 0, _) => {
                        if *wp < 8 {
                            *wp += 1;
                        } else {
                            *sp = 3;
                            *zp = 3;
                        }
                    },
                    (255, _, _, _) | (_, _, 255, _) => {
                        *dp = 0;
                        *cp = 0;
                        *sp = 0;
                        *wp = 0;
                        *bp = 0;
                        *ep = 0;
                        *zp = 0;
                        *vp = 0;
                    },
                    _ => (),
                }

                let link_data = self.data_pointer(&guard, 0);
                let link_control = self.data_pointer(&guard, 1);
                let link_status = self.data_pointer(&guard, 2);

                if *link_status > 0 {
                    if *link_status == 3 {
                        *link_status = 0;

                        Some((true, *link_data, *link_control))
                    } else {
                        *link_status += 1;

                        Some((false, *link_data, *link_control))
                    }
                } else {
                    None
                }
            })
            .unwrap_or_default()
    }

    fn disconnected(&self) -> bool {
        false
    }
}
