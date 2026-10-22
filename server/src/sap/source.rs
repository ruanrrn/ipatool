// Address-independent SAP source expansion (port of AssppWeb source.ts, from ApplePackage).
use super::tables::Tables;

pub const INITIAL_ORDER: [usize; 16] = [13, 0, 11, 10, 6, 12, 15, 1, 5, 14, 2, 4, 3, 8, 9, 7];
pub const TERMINAL_ORDER: [usize; 16] = [13, 4, 8, 9, 14, 1, 7, 15, 12, 2, 6, 0, 11, 3, 10, 5];

fn linear(value: usize, basis: [u32; 8], mut constant: u32) -> u32 {
    for (bit, item) in basis.iter().enumerate() {
        if value & (1 << bit) != 0 {
            constant ^= item;
        }
    }
    constant
}

pub struct PreparedSource {
    pub material: [u8; 576],
    pub auxiliary0: [u32; 256],
    pub auxiliary1: [u32; 256],
    pub cache2: Vec<u8>,
    pub cache3: Vec<u8>,
    pub projection_mask: u32,
    pub selectors: [usize; 16],
    pub initial_cache: [u32; 16],
    pub terminal_cache: [u32; 16],
}

pub fn prepare_source(source: &[u8], tables: &Tables) -> Result<PreparedSource, String> {
    if source.len() != 576 {
        return Err("Invalid SAP session source".to_string());
    }
    let mut lengths = [[0usize; 11]; 16];
    for column in 0..11 {
        let bank = [0xe0, 0xf0, 0xc0][column / 4];
        let moduli = [[7, 27, 47, 77], [9, 29, 49, 79], [5, 25, 45, 75]][column / 4];
        for group in 0..4 {
            let at = 4 * (column % 4) + group;
            let value = (source[0x30 + at] ^ source[bank + at]) as usize;
            for lane in 0..4 {
                lengths[4 * group + lane][column] = [3, 3, 5, 1][(value % moduli[lane]) & 3];
            }
        }
        let mut cost: usize = lengths.iter().map(|row| row[column]).sum();
        let mut row = ((source[0x31] ^ source[0xc1]) & 15) as usize;
        while cost > 32 {
            cost -= lengths[row][column] - 1;
            lengths[row][column] = 1;
            row = (row + 5) & 15;
        }
    }

    let mut selectors = [0usize; 16];
    for index in 0..8 {
        let value = (source[0xe0 + index] ^ source[0xf0 + index]) as usize;
        selectors[2 * index] = value % 7;
        selectors[2 * index + 1] = (value / 7) % 7;
    }

    let size =
        (((source[0xfa] ^ source[0xca]) as usize) << 8) | (source[0xf3] ^ source[0xc3]) as usize;
    if size == 0 {
        return Err("Invalid SAP inverse cache".to_string());
    }
    let mut seed = 0u32;
    for (a, b) in [(0xea, 0xfa), (0xe4, 0xf4), (0xed, 0xfd), (0xe6, 0xf6)] {
        seed = (seed << 8) | (source[a] ^ source[b]) as u32;
    }
    let mut state = seed;
    let mut states = [0u32; 13];
    for value in states.iter_mut() {
        state = ((state as u128 * state as u128 + 1) % 0xffff_fffb) as u32;
        *value = state;
    }
    let mut mask = 1u32 << (seed & 7);
    for lane in 1..4 {
        mask |= 1u32 << (8 * lane as u32 + (states[lane - 1] & 7));
    }
    let basis: [u32; 8] = states[3..11].try_into().unwrap();
    let header = states[11];
    state = states[12];

    let mut cache: Vec<u8> = (0..size)
        .map(|index| source[(index + 3 * source[index % 576] as usize + 2) % 576] ^ 0xd6)
        .collect();
    let mut words = [0u32; 256];
    for (index, slot) in words.iter_mut().enumerate() {
        let mut word = header ^ (state & mask);
        for (bit, item) in basis.iter().enumerate() {
            if index & (1 << bit) != 0 {
                word ^= item;
            }
        }
        *slot = word;
        cache[((word & !mask) as usize) % size] = (index ^ 0xd6) as u8;
        let value = (state ^ 0x63) as u64;
        let correction = if index != 0 && state & 0x8000_0000 != 0 {
            512
        } else {
            0
        };
        let position = ((17 * (value >> 4) + 3 * (value & 15) + 176 + correction) % 576) as usize;
        state = 0;
        for byte in 0..4 {
            state |= (source[(position + byte) % 576] as u32) << (8 * byte);
        }
    }

    let mut auxiliary0 = [0u32; 256];
    let mut auxiliary1 = [0u32; 256];
    for (index, word) in words.iter().enumerate() {
        auxiliary0[linear(index, [0x3c, 0x44, 0x73, 0xcc, 0x50, 0xba, 0x9e, 0xee], 0) as usize] =
            word ^ 0xd90b_5d96;
        auxiliary1[linear(index, [0x9f, 0x72, 0xe1, 0x3b, 0xdd, 0x31, 0x24, 0x8f], 0) as usize] =
            word ^ 0xaa80_95ee;
    }
    let cache2: Vec<u8> = cache
        .iter()
        .map(|&value| {
            linear(
                value as usize,
                [0xe3, 0x34, 0xda, 0x73, 0xb4, 0xa0, 0x4f, 0x2b],
                0xc0,
            ) as u8
        })
        .collect();
    let cache3: Vec<u8> = cache
        .iter()
        .enumerate()
        .map(|(index, &value)| {
            (linear(
                value as usize,
                [0x1f, 0x35, 0x3b, 0xf7, 0xd9, 0xad, 0xd4, 0x93],
                0x47,
            ) ^ ((index >> 2) & 255) as u32) as u8
        })
        .collect();

    let operand = |row: usize, position: usize, length: usize, base: usize, column: usize| -> u8 {
        let mut key = 0u8;
        for byte in 0..length {
            key ^= source.get(base + position + byte).copied().unwrap_or(0)
                ^ tables.pad_columns[column][(position + byte) % 16]
                ^ tables.pad_rows[column][row];
        }
        key
    };

    let build_cache = |column: usize, base: usize, order: &[usize; 16], bias: u32| -> [u32; 16] {
        let mut result = [0u32; 16];
        let mut position = 0;
        for (row, &slot) in order.iter().enumerate() {
            let length = lengths[row][column];
            if length != 1 {
                result[slot] = words[operand(row, position, length, base, column) as usize] ^ bias;
            }
            position += length;
        }
        result
    };
    let initial_cache = build_cache(10, 0x1a0, &INITIAL_ORDER, states[11] ^ 0xe628_18fc);
    let terminal_cache = build_cache(0, 0x40, &TERMINAL_ORDER, 0xaf61_5514);

    let mut material = [0u8; 576];
    for (destination, origin) in [
        (0x20, 0xf0),
        (0x70, 0xe0),
        (0x120, 0xd0),
        (0x130, 0x220),
        (0x140, 0x110),
        (0x150, 0x100),
        (0x160, 0x80),
        (0x170, 0x120),
        (0x180, 0x20),
        (0x190, 0x90),
        (0x1a0, 0x190),
        (0x1b0, 0x230),
        (0x1e0, 0xc0),
        (0x1f0, 0x30),
    ] {
        material[destination..destination + 16].copy_from_slice(&source[origin..origin + 16]);
    }
    let offsets = [2, 12, 6, 16, 0, 8, 22, 28, 4, 26, 18, 10, 20, 30, 24, 14];
    let mut walk = 0usize;
    for (column, base, destination) in [
        (1, 0xa0, 0x200),
        (2, 0x200, 0x1c0),
        (3, 0x60, 0xc0),
        (4, 0, 0),
        (5, 0x1c0, 0x50),
        (6, 0x130, 0x80),
        (7, 0x1e0, 0xa0),
        (8, 0x170, 0x30),
        (9, 0x150, 0x100),
    ] {
        let mut position = 0;
        for (row, offset) in offsets.iter().enumerate() {
            let length = lengths[row][column];
            let key = operand(row, position, length, base, column);
            walk = (walk + base + position) % 576;
            let mut first = source[walk];
            walk = (walk + base + position + 1) % 576;
            first = first.wrapping_add(source[walk]);
            let at = destination + offset;
            material[at] = first;
            material[at + 1] = key.wrapping_sub(first);
            position += length;
        }
    }

    Ok(PreparedSource {
        material,
        auxiliary0,
        auxiliary1,
        cache2,
        cache3,
        projection_mask: !mask,
        selectors,
        initial_cache,
        terminal_cache,
    })
}
