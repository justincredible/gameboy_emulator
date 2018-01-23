mod mode;
mod flag;

use self::mode::Mode;
use self::flag::ControlFlag;
use mmu::Memory;
use mmu::interrupt::Interrupt;
use emulator::traits::Io;

const CONTROL_REGISTER_INDEX: u16 = 0xFF40;
const SCROLL_Y_INDEX: u16 = 0xFF42;
const SCROLL_X_INDEX: u16 = 0xFF43;
const SCAN_LINE_INDEX: u16 = 0xFF44;
const BACKGROUND_PALETTE_INDEX: u16 = 0xFF47;

const WHITE: u8 = 0b11111111;
const LIGHT_GRAY: u8 = 0b01001010;
const DARK_GRAY: u8 = 0b00100101;
const BLACK: u8 = 0b00000000;


pub struct GPU {
    pub pixels: [u8; 144 * 160],
    cycles: u64,
    mode: Mode,
}

impl GPU {
    pub fn new() -> GPU {
        GPU {
            pixels: [0; 144 * 160],
            cycles: 0,
            mode: Mode::HBlank,
        }
    }

    pub fn step<T: Io>(&mut self, steps: u8, memory: &mut Memory, drawer: &T) {
        self.cycles += steps as u64;
        match self.mode {
            Mode::HBlank => self.h_blank(memory, drawer),
            Mode::VBlank => self.v_blank(),
            Mode::OAM => self.oam(memory),
            Mode::VRAM => self.vram(memory)
        }
    }

    fn h_blank<T: Io>(&mut self, memory: &mut Memory, drawer: &T) {
        if self.cycles >= 204 {

            self.cycles = 0;
            self.increment_scanline(memory);

            if memory.read_byte(SCAN_LINE_INDEX) == 143 {
                self.mode = Mode::OAM;
                drawer.draw(&self.pixels);
                memory.request_interrupt(Interrupt::Vblank);
            } else {
                self.mode = Mode::VBlank;
            }
        }
    }

    fn v_blank(&mut self) {
        if self.cycles >= 80 {
            self.mode = Mode::VRAM;
            self.cycles = 0;
        }
    }

    fn oam(&mut self, memory: &mut Memory) {
        if self.cycles >= 456 {

            self.cycles = 0;
            self.increment_scanline(memory);

            if memory.read_byte(SCAN_LINE_INDEX) > 153 {
                self.mode = Mode::HBlank;
                memory.write_byte(SCAN_LINE_INDEX, 0);
            }
        }
    }

    fn vram(&mut self, memory: &Memory) {
        if self.cycles >= 172 {
            self.cycles = 0;
            self.mode = Mode::HBlank;
            self.render_scan(memory);
        }
    }

    fn increment_scanline(&self, memory: &mut Memory) {
        let mut scanline = memory.read_byte(SCAN_LINE_INDEX);
        scanline += 1;
        memory.write_byte(SCAN_LINE_INDEX, scanline);
    }

    fn get_palette(&self, memory: &Memory) -> [u8; 4] {
        let order = memory.read_byte(BACKGROUND_PALETTE_INDEX);
        let mut palette: [u8; 4] = [0; 4];

        // iterate through each pair of two bits in the byte
        for i in 0..4 {
            match (order >> (i * 2)) & 0b11 {
                0b00 => palette[i] = WHITE,
                0b01 => palette[i] = LIGHT_GRAY,
                0b10 => palette[i] = DARK_GRAY,
                0b11 => palette[i] = BLACK,
                _ => {}
            }
        }

        palette
    }

    fn render_scan(&mut self, memory: &Memory) {
        let flag = memory.read_byte(CONTROL_REGISTER_INDEX);
        let scan_line = memory.read_byte(SCAN_LINE_INDEX);
        let scroll_y = memory.read_byte(SCROLL_Y_INDEX);
        let scroll_x = memory.read_byte(SCROLL_X_INDEX);
        let palette = self.get_palette(memory);

        let flag = ControlFlag::from_bits(flag).unwrap();

        let line_offset = (scan_line + scroll_y) as usize;

        let map_y = line_offset / 8;

        for x in 0..160 {
            let x_offset = (x + scroll_x) as usize;
            let map_x = x_offset / 8;

            let tile_id = if flag.contains(ControlFlag::BACKGROUND_TILE_MAP) {
                memory.read_byte((0x9C00 + (32 * map_y + map_x)) as u16)
            } else {
                memory.read_byte((0x9800 + (32 * map_y + map_x)) as u16)
            };

            let tile = if flag.contains(ControlFlag::BACKGROUND_TILE_SET) {
                memory.get_tile_from_set1(tile_id)
            } else {
                memory.get_tile_from_set0(tile_id as i8)
            };

            let row_num = (line_offset % 8) * 2;
            let column_num = 7 - x_offset % 8;

            let high = tile[row_num];
            let low = tile[(row_num + 1)];

            let high_color = ((high & (1 << column_num) != 0) as u8) << 1;
            let low_color = (low & (1 << column_num) != 0) as u8;
            let color = palette[(high_color + low_color) as usize];

            self.pixels[(160 * (143 - scan_line as usize) + x as usize)] = color;
        }
    }
}

#[cfg(test)]
mod tests {
    use gpu::GPU;
    use mmu::Memory;

    #[test]
    fn test_render_scan() {
        let mut memory = Memory::new();
        let mut gpu = GPU::new();

        assert_eq!(1, 1);
    }
}