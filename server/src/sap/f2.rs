// SAP F2 derivation (port of AssppWeb f2.ts, from ApplePackage).
use super::crypto::read32;
use super::tables::Tables;

fn read32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

pub fn derive_f2(key: &[u8], input: &[u8], tables: &Tables) -> Result<[u8; 16], String> {
    if key.len() != 16 || input.len() != 16 {
        return Err("Invalid SAP F2 inputs".to_string());
    }
    let bytes = &tables.f2;
    let encoded: Vec<u8> = key
        .iter()
        .enumerate()
        .map(|(index, &value)| bytes[256 * index + value as usize])
        .collect();
    let mut words: Vec<u32> = (0..4)
        .map(|index| read32(&encoded, 4 * index))
        .collect::<Result<_, _>>()?;
    for step in &tables.expansion {
        let mut word = step
            .selection
            .iter()
            .fold(0, |value, &index| value ^ words[index]);
        word = word.rotate_left(8) ^ step.whitening;
        let mut substituted = 0u32;
        for position in 0..4 {
            let shift = 24 - 8 * position;
            substituted |=
                (bytes[4096 + 256 * position + ((word >> shift) & 255) as usize] as u32) << shift;
        }
        words.push(substituted);
    }
    let additions: Vec<[u8; 16]> = tables
        .key_selections
        .iter()
        .map(|row| {
            let mut result = [0u8; 16];
            for (slot, selection) in row.iter().enumerate() {
                let word = selection
                    .words
                    .iter()
                    .fold(0, |value, &index| value ^ words[index]);
                let mut byte = ((word >> selection.shift) & 255) as u8;
                if selection.linear >= 0 {
                    byte = bytes[17408 + 256 * selection.linear as usize + byte as usize];
                }
                result[slot] = byte ^ selection.whitening;
            }
            result
        })
        .collect();
    let mut state = additions[0];
    for position in 0..16 {
        state[tables.entry_positions[position]] ^=
            bytes[5120 + 256 * position + input[position] as usize];
    }
    for (round, row) in tables.main_routing.iter().enumerate() {
        let mut output = additions[round + 1];
        for (position, routing) in row.iter().enumerate() {
            let word = read32_le(
                bytes,
                13312 + 4 * (256 * routing.table + state[position] as usize),
            );
            for (index, &destination) in routing.destinations.iter().enumerate() {
                output[destination] ^= ((word >> (24 - 8 * index)) & 255) as u8;
            }
        }
        state = output;
    }
    let mut output = additions[10];
    for position in 0..16 {
        output[tables.final_positions[position]] ^=
            bytes[9216 + 256 * position + state[position] as usize];
    }
    Ok(output.map(|value| value ^ 0x2d))
}
