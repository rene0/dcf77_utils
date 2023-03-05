pub fn get_binary_value(bit_buffer: &[Option<bool>], start: usize, stop: usize) -> Option<u16> {
    let mut val = 0;
    let mut mult = 1;
    for b in &bit_buffer[start..=stop] {
        (*b)?;
        val += mult * b.unwrap() as u16;
        mult *= 2;
    }
    Some(val)
}
