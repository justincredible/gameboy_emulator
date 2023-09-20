use crate::mmu::interrupt::Interrupt;
use crate::mmu::Memory;

const PROJ_ID: i32 = 65;
const SHM_SZ: usize = 24;
const LINK_DATA: usize = 4;
const LINK_CTRL: usize = 8;
const FIRST_PID: usize = 12;
const SHM_DATA: usize = 16; // safety: align(size_of(machine word))

const STATUS_INT: u8 = 0xFF;
const REQ_SEND: u8 = 0x81;
const REQ_RECV: u8 = 0x80;
const CLEAR_7: u8 = 0x7F;

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
            libc::shmget(key, SHM_SZ, 0o666 | libc::IPC_CREAT)
        };
        if shmid == -1 { panic!("shmget failed"); }

        let link = unsafe {
            libc::shmat(shmid, std::ptr::null(), 0)
        };
        unsafe {
            if *(link as *const i8) == -1 {
                panic!("shmat failed: {:?}", *libc::__errno_location());
            }
        }

        let pid = std::process::id();

        unsafe {
            std::ptr::write(link.add(LINK_CTRL) as *mut u32, pid);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));

        let first = unsafe {
            pid == std::ptr::read(link.add(LINK_CTRL) as *mut u32)
        };

        if first {
            // initialize all memory
            unsafe {
                std::ptr::write_bytes(link as *mut u64, 0, SHM_SZ / std::mem::size_of::<u64>());
                std::ptr::write_bytes(link as *mut u8, 0xFF, 4);
                std::ptr::write(link.add(LINK_CTRL) as *mut u32, pid);
            }

            let status = unsafe {
                libc::sem_init(link.add(SHM_DATA) as *mut libc::sem_t, PROJ_ID, 1)
            };
            if status != 0 { panic!("sem_init failed"); }
        } else {
            unsafe {
                std::ptr::write(link.add(FIRST_PID) as *mut u32, pid);
            }
        }

        println!("{:?} {:?}", pid, first);

        LinkCable {
            shmid,
            link,
            first,
            waiting: false,
        }
    }

    unsafe fn semaphore(&mut self) -> *mut libc::sem_t {
        self.link.add(SHM_DATA) as *mut libc::sem_t
    }

    fn disconnected(&self) -> bool {
        // SAFETY
        // this function only ever reads and compares to zero
        // false positives result in emulating reality ;-)
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

        if self.first {
            unsafe {
                if pid != std::ptr::read(self.link.add(LINK_CTRL) as *mut u32) {
                    std::ptr::write(self.link.add(LINK_CTRL) as *mut u32, pid);
                }
            }
        }

        if self.disconnected() {
            mmu.write_byte(0xFF01, 0xFF);
        } else {
            let slice = unsafe {
                std::slice::from_raw_parts_mut(self.link as *mut u8, SHM_DATA)
            };

            let success = unsafe {
                libc::sem_wait(self.semaphore())
            };
            if success == 0 {
                let (check_int_slot, raise_int_slot, read_slot, write_slot) =
                    if self.first {
                        (4, 5, 0, 2)
                    } else {
                        (5, 4, 2, 0)
                    };

                if !self.waiting {
                    if slice[check_int_slot] == STATUS_INT {
                        self.waiting = true;
                    } else {
                        if sdc.1 == 0x81 {
                            slice[write_slot] = sdc.0;
                            slice[raise_int_slot] = STATUS_INT;
                            self.waiting = true;
                        }
                    }
                }
                else {
                    if slice[check_int_slot] == 0 {
                        slice[write_slot] = sdc.0;
                    }
                    if slice[check_int_slot] == STATUS_INT {
                        mmu.write_byte(0xFF01, slice[read_slot] & 0x7F);
                        mmu.request_interrupt(Interrupt::Serial);
                        slice[check_int_slot] = 0;
                    }
                }

                unsafe {
                    libc::sem_post(self.semaphore());
                }
            }
        }
    }
}
