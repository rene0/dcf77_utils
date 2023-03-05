/// Returns the binary-encoded value of the given buffer over the given range, or None if the input is invalid.
///
/// # Arguments
/// * `bit_buffer` - buffer containing the bits
/// * `start` - start bit position (least significant)
/// * `stop` - stop bit position (most significant)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_binary_value_all_0() {
        const BINARY_BUFFER: [Option<bool>; 7] = [
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
        ];
        assert_eq!(get_binary_value(&BINARY_BUFFER, 0, 6), Some(0));
    }

    #[test]
    fn test_get_binary_value_all_1() {
        const BINARY_BUFFER: [Option<bool>; 7] = [
            Some(true),
            Some(true),
            Some(true),
            Some(true),
            Some(true),
            Some(true),
            Some(true),
        ];
        assert_eq!(get_binary_value(&BINARY_BUFFER, 0, 6), Some(0x7f));
    }

    #[test]
    fn test_get_binary_value_middle() {
        const BINARY_BUFFER: [Option<bool>; 7] = [
            Some(true),
            Some(true),
            Some(false),
            Some(false),
            Some(true),
            Some(false),
            Some(true),
        ];
        assert_eq!(get_binary_value(&BINARY_BUFFER, 0, 6), Some(0x53));
    }

    #[test]
    fn test_get_unary_value_invalid_none() {
        const BINARY_BUFFER: [Option<bool>; 4] = [Some(true), Some(true), None, Some(false)];
        assert_eq!(get_binary_value(&BINARY_BUFFER, 0, 3), None);
    }
}
