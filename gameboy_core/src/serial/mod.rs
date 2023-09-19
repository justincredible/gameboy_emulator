use crate::mmu::interrupt::Interrupt;
use crate::mmu::Memory;

const PROJ_ID: i32 = 87;
const SHM_SZ: usize = 24;
const LINK_DATA: usize = 4;
const LINK_CTRL: usize = 8;
const FIRST_PID: usize = 12;
const SHM_DATA: usize = 16; // safety: align(size_of(machine word))

fn slc2u32(slice: &[u8]) -> u32 {
    slice
        .iter()
        .take(4)
        .fold(0, |sum, &val| sum * 256 + val as u32)
}

fn u322vec(value: u32) -> Vec<u8> {
    let d = (value % 256) as u8;
    let c = ((value / 256) % 256) as u8;
    let b = ((value / 65536) % 256) as u8;
    let a = (value / 16777216) as u8;

    vec![a, b, c, d]
}

fn str2cstr(string: &str) -> Vec<i8> {
    string
        .bytes()
        .filter(|&c| c < 128)
        .map(|u| u as i8)
        .collect()
}

pub struct LinkCable {
    shmid: i32,
    link: *mut libc::c_void,
    first: bool,
    counter: u8,
}

impl Drop for LinkCable {
    fn drop(&mut self) {
        unsafe {
            let slice = std::slice::from_raw_parts_mut(self.link as *mut u8, SHM_DATA);

            let index = if !self.first { LINK_CTRL } else { FIRST_PID };
            let other_pid = slc2u32(&slice[index..index + 4]);

            let index = if self.first { LINK_CTRL } else { FIRST_PID };
            for offset in 0..std::mem::size_of::<u32>() {
                slice[index + offset] = 0;
            }

            if other_pid == 0 {
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
        if key == -1 { panic!("ftok failed"); }

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
                std::ptr::write_bytes(link as *mut u8, 0, SHM_SZ);
                std::ptr::write_bytes(link as *mut u8, 0xFF, 4);
                std::ptr::write(link.add(LINK_CTRL) as *mut u32, pid);
            }

            let status = unsafe {
                libc::sem_init(link.add(SHM_DATA) as *mut libc::sem_t, PROJ_ID, 1)
            };
            if status != 0 { panic!("sem_init failed"); }
        } else {
            unsafe { // is this aligned on 64-bit machines?
                std::ptr::write(link.add(FIRST_PID) as *mut u32, pid);
            }
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
        }
    }

    unsafe fn semaphore(&mut self) -> *mut libc::sem_t {
        self.link.add(SHM_DATA) as *mut libc::sem_t
    }

    #[allow(dead_code)]
    fn write(&mut self, index: usize, value: u8) {
        unsafe {
            let success = libc::sem_wait(self.semaphore());

            if success == 0 {
                std::ptr::write(self.link.add(index) as *mut u8, value);
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
        if self.first && slc2u32(&slice[LINK_CTRL..FIRST_PID]) != pid {
            let mut index = LINK_CTRL;
            for byte in u322vec(pid) {
                slice[index] = byte;
                index += 1;
            }
        }
        if !self.first && slc2u32(&slice[FIRST_PID..SHM_DATA]) != pid {
            let mut index = FIRST_PID;
            for byte in u322vec(pid) {
                slice[index] = byte;
                index += 1;
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

            unsafe {
                libc::sem_post(self.semaphore());
            }
        }

        let success = unsafe { libc::sem_wait(self.semaphore()) };
        if success == 0 {
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
                if self.counter < 8 {
                    let a = if self.first { slice[0] } else { slice[2] };
                    let b = if self.first {slice[2] } else { slice[0] };
                    slice[0] = (a & 0x7F) << 1 | (b & 0x80) >> 7;
                    slice[2] = (b & 0x7F) << 1 | (a & 0x80) >> 7;
                    /*let temp = slice[0];
                    slice[0] = slice[2];
                    slice[2] = temp;*/
                    self.counter += 1;
                } else {
                    slice[4] = 2;
                    slice[5] = 2;
                    self.counter = 0;
                }
            }

            unsafe {
                libc::sem_post(self.semaphore());
            }
        }

        let success = unsafe { libc::sem_wait(self.semaphore()) };
        if success == 0 {
            if self.first && slice[4] > 0 {
                mmu.write_byte(0xFF01, slice[0]);
                mmu.write_byte(0xFF02, slice[1]);

                let shmpid = slc2u32(&slice[FIRST_PID..SHM_DATA]);
                if shmpid == 0 {
                    mmu.write_byte(0xFF01, 0xFF);
                }

                if slice[4] == 2 {
                    mmu.write_byte(0xFF02, slice[1] & 0x7F);
                    mmu.request_interrupt(Interrupt::Serial);
                    slice[4] = 0;
                }
            }
            if !self.first && slice[5] > 0 {
                mmu.write_byte(0xFF01, slice[2]);
                mmu.write_byte(0xFF02, slice[3]);

                let shmpid = slc2u32(&slice[LINK_CTRL..FIRST_PID]);
                if shmpid == 0 {
                    mmu.write_byte(0xFF01, 0xFF);
                }

                if slice[5] == 2 {
                    mmu.write_byte(0xFF02, slice[3] & 0x7F);
                    mmu.request_interrupt(Interrupt::Serial);
                    slice[5] = 0;
                }
            }

            unsafe {
                libc::sem_post(self.semaphore());
            }
        }
    }
}
