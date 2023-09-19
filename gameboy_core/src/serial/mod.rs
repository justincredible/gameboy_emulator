use crate::mmu::interrupt::Interrupt;
use crate::mmu::Memory;

const PROJ_ID: i32 = 87;
const SHM_SZ: usize = 24;
const LINK_DATA: usize = 4;
const LINK_CTRL: usize = 8;
const FIRST_PID: usize = 12;
const SHM_DATA: usize = 16; // safety: align(size_of(machine word))

fn str2cstr(string: &str) -> Vec<i8> {
    string
        .bytes()
        .filter(|&c| c < 128)
        .map(|u| u as i8)
        .chain(std::iter::once(0))
        .collect()
}

pub struct LinkCable {
    shmid: i32,
    link: *mut libc::c_void,
    first: bool,
    counter: u8,
    waiting: bool,
}

impl Drop for LinkCable {
    fn drop(&mut self) {
        unsafe {
            let offset = if !self.first { LINK_CTRL } else { FIRST_PID };
            let rid = std::ptr::read(self.link.add(offset) as *const u32);

            let offset = if self.first { LINK_CTRL } else { FIRST_PID };
            std::ptr::write(self.link.add(offset) as *mut u32, 0);

            if rid == 0 {
                libc::sem_trywait(self.semaphore());
                libc::sem_post(self.semaphore());
                libc::sem_destroy(self.semaphore());
            }
        }
        unsafe {
            libc::shmdt(self.link);
            libc::shmctl(self.shmid, libc::IPC_RMID, std::ptr::null_mut());
        }
    }
}

impl LinkCable {
    pub fn new(is_cbg: bool) -> Self {
        if is_cbg { panic!("Game Boy Color not supported"); }

        let key_file = std::path::Path::new("link_cable");
        if !key_file.exists() {
            std::fs::File::create(key_file).unwrap();
        }

        let key = unsafe {
            libc::ftok(str2cstr("link_cable").as_ptr(), PROJ_ID)
        };
        if key == -1 {
            unsafe {
                panic!("ftok failed: {:?}", *libc::__errno_location());
            }
        }

        let shmid = unsafe {
            libc::shmget(key, SHM_SZ, 0o664 | libc::IPC_CREAT)
        };
        if shmid == -1 { panic!("shmget failed"); }

        let link = unsafe {
            libc::shmat(shmid, std::ptr::null(), 0)
        };
        unsafe {
            if *(link as *const i8) == -1 {
                panic!("shmat failed");
            }
        }

        let pid = std::process::id();

        unsafe {
            std::ptr::write(link.add(LINK_CTRL) as *mut u32, pid);
        }

        std::thread::sleep(std::time::Duration::from_millis(10));

        let first = unsafe {
            pid == std::ptr::read(link.add(LINK_CTRL) as *mut u32)
        };

        if first {
            // initialize all memory
            unsafe {
                std::ptr::write_bytes(link as *mut u64, 0, SHM_SZ / std::mem::size_of::<u64>());
                std::ptr::write_bytes(link as *mut u8, 0xFF, 4);
                std::ptr::write(link.add(LINK_CTRL) as *mut u32, pid);
                std::ptr::write(link.add(FIRST_PID) as *mut u32, pid);
            }

            let status = unsafe {
                libc::sem_init(link.add(SHM_DATA) as *mut libc::sem_t, PROJ_ID, 1)
            };
            if status != 0 { panic!("sem_init failed"); }
        }

        unsafe {
            if first != (pid == std::ptr::read(link.add(LINK_CTRL) as *mut u32)) {
                panic!("Failed to synchronize");
            }
        }

        println!("{:?} {:?}", pid, first);

        LinkCable {
            shmid,
            link,
            first,
            counter: 0,
            waiting: false,
        }
    }

    unsafe fn semaphore(&mut self) -> *mut libc::sem_t {
        self.link.add(SHM_DATA) as *mut libc::sem_t
    }

    fn disconnected(&self) -> bool {
        // SAFETY
        // this function only ever reads and compares to zero
        unsafe {
            let offset = if self.first { LINK_CTRL } else { FIRST_PID };

            std::ptr::read(self.link.add(offset) as *const u32) == 0
        }
    }

    #[allow(dead_code)]
    fn write(&mut self, offset: usize, value: u8) {
        unsafe {
            let success = libc::sem_wait(self.semaphore());

            if success == 0 {
                std::ptr::write(self.link.add(offset) as *mut u8, value);
                libc::sem_post(self.semaphore());
            }
        }
    }

    pub fn update(&mut self, mmu: &mut Memory) {
        let sdc = (mmu.read_byte(0xFF01), mmu.read_byte(0xFF02));
        let pid = std::process::id();

        let slice = unsafe {
            std::slice::from_raw_parts_mut(self.link as *mut u8, SHM_DATA)
        };

        // link order synchronization
        // SAFETY
        // this area of memory is only ever supposed to contain
        // the process id of the first process in the working directory
        unsafe {
            let rid = std::ptr::read(self.link.add(FIRST_PID) as *mut u32);
            let fid = std::ptr::read(self.link.add(LINK_CTRL) as *mut u32);
            if self.first && fid != pid {
                std::ptr::write(self.link.add(LINK_CTRL) as *mut u32, pid);
            }
            if !self.first && fid != 0 && fid != rid {
                std::ptr::write(self.link.add(LINK_CTRL) as *mut u32, rid);
            }
        }

        let success = unsafe { libc::sem_wait(self.semaphore()) };
        if success == 0 {
            if self.first && slice[4] == 0 {
                slice[0] = sdc.0;
                slice[1] = sdc.1;
            }
            if !self.first && slice[5] == 0 {
                slice[2] = sdc.0;
                slice[3] = sdc.1;
            }

            if slice[4] == 0 && slice[5] == 0 &&
                (self.first && slice[1] == 0x81 && slice[3] == 0x80 ||
                !self.first && slice[3] == 0x81 && slice[1] == 0x80 ||
                slice[1] == 0x81 && slice[3] == 0x81) {
                slice[4] = 1;
                slice[5] = 1;

                self.counter = 0;
            }

            if slice[4] == 1 && slice[5] == 1 &&
                (self.first && slice[1] == 0x81 && slice[3] == 0x80 ||
                !self.first && slice[3] == 0x81 && slice[1] == 0x80 ||
                slice[1] == 0x81 && slice[3] == 0x81) {
                if self.counter < 1 {
                    /*let a = if self.first { slice[0] } else { slice[2] };
                    let b = if self.first {slice[2] } else { slice[0] };
                    slice[0] = (a & 0x7F) << 1 | (b & 0x80) >> 7;
                    slice[2] = (b & 0x7F) << 1 | (a & 0x80) >> 7;*/
                    if slice[1] == 0x81 && slice[3] == 0x80 {
                        slice[2] = slice[0];
                    } else if slice[3] == 0x81 && slice[1] == 0x80 {
                        slice[0] = slice[2];
                    } else if slice[1] == 0x81 && slice[3] == 0x81 {
                        let temp = slice[0];
                        slice[0] = slice[2];
                        slice[2] = temp;
                    }
                    self.counter += 1;
                } else {
                    if slice[1] == 0x81 && slice[3] == 0x80 {
                        slice[5] = 2;
                        slice[3] &= 0x7F;
                    } else if slice[3] == 0x81 && slice[1] == 0x80 {
                        slice[4] = 2;
                        slice[1] &= 0x7F;
                    } else if slice[1] == 0x81 && slice[3] == 0x81 {
                        slice[4] = 2;
                        slice[1] &= 0x7F;
                        slice[5] = 2;
                        slice[3] &= 0x7F;
                    }
                    self.waiting = true;
                    self.counter = 0;
                }
            }

            if self.first && self.waiting && slice[3] & 0x80 == 0x80 {
                slice[0] = slice[2];
                slice[4] = 2;
                slice[1] &= 0x7F;
            }
            if !self.first && self.waiting && slice[1] & 0x80 == 0x80 {
                slice[2] = slice[0];
                slice[5] = 2;
                slice[3] &= 0x7F;
            }

            if self.first && slice[4] > 0 {
                mmu.write_byte(0xFF01, slice[0]);

                if self.disconnected() {
                    mmu.write_byte(0xFF01, 0xFF);
                }

                if slice[4] == 2 {
                    mmu.write_byte(0xFF02, slice[1]);
                    mmu.request_interrupt(Interrupt::Serial);
                    slice[4] = 0;
                    self.waiting = false;
                }
            }
            if !self.first && slice[5] > 0 {
                mmu.write_byte(0xFF01, slice[2]);

                if self.disconnected() {
                    mmu.write_byte(0xFF01, 0xFF);
                }

                if slice[5] == 2 {
                    mmu.write_byte(0xFF02, slice[3]);
                    mmu.request_interrupt(Interrupt::Serial);
                    slice[5] = 0;
                    self.waiting = false;
                }
            }

            unsafe {
                libc::sem_post(self.semaphore());
            }
        }
    }
}
