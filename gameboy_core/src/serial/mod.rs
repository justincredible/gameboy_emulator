use crate::mmu::interrupt::Interrupt;
use crate::mmu::Memory;

const PROJ_ID: i32 = 87;
const SHM_SZ: usize = 24;
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
            let slice = std::slice::from_raw_parts_mut(self.link as *mut u8, SHM_SZ);

            let index = if !self.first { 4 } else { 12 };
            let other_pid = slc2u32(&slice[index..index + 4]);

            let index = if self.first { 4 } else { 12 };
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

        unsafe {
            std::ptr::write_bytes(link as *mut u8, 0, SHM_DATA);
        }

        let pid = std::process::id();

        let slice = unsafe {
            std::slice::from_raw_parts_mut(link as *mut u8, SHM_SZ)
        };
        let mut index = 4;
        for byte in u322vec(pid) {
            slice[index] = byte;
            index += 1;
        }

        std::thread::sleep(std::time::Duration::from_millis(10));

        let first = pid == slc2u32(&slice[4..8]);

        if first {
            let status = unsafe {
                libc::sem_init(link.add(SHM_DATA) as *mut libc::sem_t, PROJ_ID, 1)
            };
            if status != 0 { panic!("sem_init failed"); }
        }

        if first != (pid == slc2u32(&slice[4..8])) {
            panic!("Failed to synchronize");
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
            std::slice::from_raw_parts_mut(self.link as *mut u8, SHM_SZ)
        };

        // link order synchronization
        if self.first && slc2u32(&slice[4..8]) != pid {
            let mut index = 4;
            for byte in u322vec(pid) {
                slice[index] = byte;
                index += 1;
            }
        }

        let success = unsafe { libc::sem_wait(self.semaphore()) };
        if success == 0 {
            if self.first && slice[2] == 0 {
                slice[0] = sdc.0;
                slice[1] = sdc.1;
            }
            if !self.first && slice[10] == 0 {
                slice[8] = sdc.0;
                slice[9] = sdc.1;
            }

            if slice[2] == 0 && slice[10] == 0 &&
                (slice[1] == 0x81 && slice[9] == 0x80 ||
                slice[9] == 0x81 && slice[1] == 0x80) {
                slice[2] = 1;
                slice[10] = 1;

                self.counter = 0;
            }

            if slice[2] == 1 && slice[10] == 1 &&
                (slice[1] == 0x81 && slice[9] == 0x80 ||
                slice[9] == 0x81 && slice[1] == 0x80) {
                if self.counter < 8 {
                    let a = if self.first { slice[0] } else { slice[8] };
                    let b = if self.first {slice[8] } else { slice[0] };
                    slice[0] = (a & 0x7F) << 1 | (b & 0x80) >> 7;
                    slice[8] = (b & 0x7F) << 1 | (a & 0x80) >> 7;
                    self.counter += 1;
                } else {
                    slice[2] = 2;
                    slice[10] = 2;
                    self.counter = 0;
                }
            }

            if self.first && slice[2] > 0 {
                mmu.write_byte(0xFF01, slice[0]);
                mmu.write_byte(0xFF02, slice[1]);

                if slice[2] == 2 {
                    mmu.write_byte(0xFF02, slice[1] & 0x7F);
                    mmu.request_interrupt(Interrupt::Serial);
                    slice[2] = 0;
                }
            }
            if !self.first && slice[10] > 0 {
                mmu.write_byte(0xFF01, slice[8]);
                mmu.write_byte(0xFF02, slice[9]);

                if slice[10] == 2 {
                    mmu.write_byte(0xFF02, slice[9] & 0x7F);
                    mmu.request_interrupt(Interrupt::Serial);
                    slice[10] = 0;
                }
            }

            unsafe {
                libc::sem_post(self.semaphore());
            }
        }
    }
}
