use crate::mmu::interrupt::Interrupt;
use crate::mmu::Memory;

const SHM_SZ: usize = 16;

fn slc2u32(slice: &[u8]) -> u32 {
    slice.iter().fold(0, |sum, &val| sum * 256 + val as u32)
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
}

impl Drop for LinkCable {
    fn drop(&mut self) {
        {
            let slice = unsafe {
                std::slice::from_raw_parts_mut(self.link as *mut u8, SHM_SZ)
            };
            let mut index = if self.first { 4 } else { 12 };
            for _ in 0..4 {
                slice[index] = 0;
                index += 1;
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
            libc::ftok(str2cstr("link_cable").as_ptr(), 87)
        };
        if key == -1 { panic!("ftok failed"); }

        let shmid = unsafe {
            libc::shmget(key, SHM_SZ, 0o666 | libc::IPC_CREAT)
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
            std::ptr::write_bytes(link as *mut u8, 0, SHM_SZ);
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

        std::thread::sleep(std::time::Duration::from_secs(1));

        let first = pid == slc2u32(&slice[4..8]);

        println!("{:?} {:?}", pid, first);

        LinkCable {
            shmid,
            link,
            first,
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

        if slice[2] > 0 || slice[10] > 0 {
            if self.first {
                mmu.write_byte(0xFF01, slice[0]);
                mmu.write_byte(0xFF02, slice[1]);
            } else {
                mmu.write_byte(0xFF01, slice[8]);
                mmu.write_byte(0xFF02, slice[9]);
            }
        } else {
            if self.first {
                slice[0] = sdc.0;
                slice[1] = sdc.1;
            } else {
                slice[8] = sdc.0;
                slice[9] = sdc.1;
            }
        }

        if sdc.1 == 0x81 && slice[2] == 0 && slice[10] == 0 {
            slice[2] = 1;
            slice[10] = 1;
            if self.first {
                slice[9] = 0x80;
            } else {
                slice[1] = 0x80;
            }
            let temp = slice[0];
            slice[0] = slice[8];
            slice[8] = temp;
            slice[2] = 2;
            slice[10] = 2;
        }

        let complete = if self.first { slice[2] } else { slice[10] } == 2;
        if complete {
            if self.first {
                mmu.write_byte(0xFF01, slice[0]);
                mmu.write_byte(0xFF02, slice[1] & 0x7F);
                mmu.request_interrupt(Interrupt::Serial);
                slice[2] = 0;
            } else {
                mmu.write_byte(0xFF01, slice[8]);
                mmu.write_byte(0xFF02, slice[9] & 0x7F);
                mmu.request_interrupt(Interrupt::Serial);
                slice[10] = 0;
            }
        }
    }
}
