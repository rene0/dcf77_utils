//! Collection of utilities for DCF77 receivers.

//! Build with no_std for embedded platforms.
#![cfg_attr(not(test), no_std)]

use radio_datetime_utils::{
    get_bcd_value, get_parity, time_diff, RadioDateTimeUtils, LEAP_ANNOUNCED, LEAP_PROCESSED,
};

/// Upper limit for spike detection in microseconds, fine tune
const SPIKE_LIMIT: u32 = 30_000;
/// Maximum time in microseconds for a bit to be considered 0
const ACTIVE_LIMIT: u32 = 150_000;
/// Maximum time in microseconds for a bit to be considered 1
const ACTIVE_RUNAWAY: u32 = 250_000;
/// Minimum time in microseconds for a new minute to be detected
const MINUTE_LIMIT: u32 = 1_500_000;
/// Signal is considered lost after this many microseconds
const PASSIVE_RUNAWAY: u32 = 2_500_000;

/// DCF77 decoder class
pub struct DCF77Utils {
    first_minute: bool,
    new_minute: bool,
    new_second: bool,
    second: u8,
    bit_buffer: [Option<bool>; 60],
    radio_datetime: RadioDateTimeUtils,
    leap_second_is_one: Option<bool>,
    parity_1: Option<bool>,
    parity_2: Option<bool>,
    parity_3: Option<bool>,
    // below for handle_new_edge()
    before_first_edge: bool,
    t0: u32,
}

impl DCF77Utils {
    /// Initialize a new DCF77Utils instance.
    pub fn new() -> Self {
        Self {
            first_minute: true,
            new_minute: false,
            new_second: false,
            second: 0,
            bit_buffer: [None; 60],
            radio_datetime: RadioDateTimeUtils::new(7),
            leap_second_is_one: None,
            parity_1: None,
            parity_2: None,
            parity_3: None,
            before_first_edge: true,
            t0: 0,
        }
    }

    /// Return if this is before the first minute that has been successfully decoded.
    pub fn get_first_minute(&self) -> bool {
        self.first_minute
    }

    /// Return if a new minute has arrived.
    pub fn get_new_minute(&self) -> bool {
        self.new_minute
    }

    /// Force the arrival of a new minute.
    ///
    /// This could be useful when reading from a log file.
    pub fn force_new_minute(&mut self) {
        self.new_minute = true;
    }

    /// Return if a new second has arrived.
    pub fn get_new_second(&self) -> bool {
        self.new_second
    }

    /// Get the second counter.
    pub fn get_second(&self) -> u8 {
        self.second
    }

    /// Get a copy of the date/time structure.
    pub fn get_radio_datetime(&self) -> RadioDateTimeUtils {
        self.radio_datetime
    }

    /// Get the leap-second-is-one anomaly.
    pub fn get_leap_second_is_one(&self) -> Option<bool> {
        self.leap_second_is_one
    }

    /// Get the minute parity bit, Some(false) means OK.
    pub fn get_parity_1(&self) -> Option<bool> {
        self.parity_1
    }

    /// Get the hour parity bit, Some(false) means OK.
    pub fn get_parity_2(&self) -> Option<bool> {
        self.parity_2
    }

    /// Get the date parity bit, Some(false) means OK.
    pub fn get_parity_3(&self) -> Option<bool> {
        self.parity_3
    }

    /// Get the value of the current bit.
    pub fn get_current_bit(&self) -> Option<bool> {
        self.bit_buffer[self.second as usize]
    }

    /// Set the value of the current bit and clear the flag indicating arrival of a new minute.
    ///
    /// This could be useful when reading from a log file.
    ///
    /// # Arguments
    /// * `value` - the value to set the current bit to
    pub fn set_current_bit(&mut self, value: Option<bool>) {
        self.bit_buffer[self.second as usize] = value;
        self.new_minute = false;
    }

    /// Get the value of bit 0 (must always be 0).
    pub fn get_bit_0(&self) -> Option<bool> {
        self.bit_buffer[0]
    }

    /// Get the value of the third-party buffer, a 14-bit number with the least significant bit first.
    pub fn get_third_party_buffer(&self) -> Option<u16> {
        let mut val = 0;
        let mut mult = 1;
        for b in &self.bit_buffer[1..=14] {
            (*b)?;
            val += mult * b.unwrap() as u16;
            mult *= 2;
        }
        Some(val)
    }

    /// Get the value of the transmitter call bit.
    pub fn get_call_bit(&self) -> Option<bool> {
        self.bit_buffer[15]
    }

    /// Get the value of bit 20 (must always be 1).
    pub fn get_bit_20(&self) -> Option<bool> {
        self.bit_buffer[20]
    }

    /**
     * Determine the bit value if a new edge is received. indicates reception errors,
     * and checks if a new minute has started.
     *
     * This function can deal with spikes, which are arbitrarily set to `SPIKE_LIMIT` microseconds.
     *
     * This method must be called _before_ `increase_second()`
     *
     * # Arguments
     * * `is_low_edge` - indicates that the edge has gone from high to low (as opposed to
     *                   low-to-high).
     * * `t` - time stamp of the received edge, in microseconds
     */
    pub fn handle_new_edge(&mut self, is_low_edge: bool, t: u32) {
        if self.before_first_edge {
            self.before_first_edge = false;
            self.t0 = t;
            return;
        }
        let t_diff = time_diff(self.t0, t);
        if t_diff < SPIKE_LIMIT {
            return; // random positive or negative spike, ignore
        }
        self.t0 = t;
        if is_low_edge {
            self.new_second = false;
            self.bit_buffer[self.second as usize] = if t_diff < ACTIVE_LIMIT {
                Some(false)
            } else if t_diff < ACTIVE_RUNAWAY {
                Some(true)
            } else {
                None
            };
        } else if t_diff < PASSIVE_RUNAWAY {
            self.new_minute = t_diff > MINUTE_LIMIT;
            self.new_second = true;
        } else {
            self.bit_buffer[self.second as usize] = None;
        }
    }

    /// Determine the length of _this_ minute in bits, tolerate None as leap second state.
    pub fn get_this_minute_length(&self) -> u8 {
        if let Some(s_leap_second) = self.radio_datetime.get_leap_second() {
            if (s_leap_second & LEAP_PROCESSED) != 0 {
                60
            } else {
                59
            }
        } else {
            59
        }
    }

    /// Determine the length of _the next_ minute in bits, tolerate None as a leap second state.
    pub fn get_next_minute_length(&self) -> u8 {
        if let Some(s_leap_second) = self.radio_datetime.get_leap_second() {
            if (self.radio_datetime.get_minute() == Some(59))
                && ((s_leap_second & LEAP_ANNOUNCED) != 0)
            {
                60
            } else {
                59
            }
        } else {
            59
        }
    }

    /// Increase or reset `second` and clear `first_minute` when appropriate.
    ///
    /// This method must be called _after_ `decode_time()` and `handle_new_edge()`
    pub fn increase_second(&mut self) {
        let minute_length = self.get_next_minute_length();
        if self.new_minute {
            if self.first_minute
                && self.second == minute_length
                && self.bit_buffer[0] == Some(false)
                && self.bit_buffer[20] == Some(true)
                && self.radio_datetime.get_dst().is_some()
                && self.radio_datetime.get_year().is_some()
                && self.radio_datetime.get_month().is_some()
                && self.radio_datetime.get_day().is_some()
                && self.radio_datetime.get_weekday().is_some()
                && self.radio_datetime.get_hour().is_some()
                && self.radio_datetime.get_minute().is_some()
            {
                // allow displaying of information after the first properly decoded minute
                self.first_minute = false;
            }
            self.second = 0;
        } else {
            self.second += 1;
            // wrap in case we missed the minute marker to prevent index-out-of-range
            if self.second == minute_length + 1 {
                self.second = 0;
            }
        }
    }

    /// Decode the time broadcast during the last minute.
    ///
    /// This method must be called _before_ `increase_second()`
    pub fn decode_time(&mut self) {
        let mut added_minute = false;
        let minute_length = self.get_next_minute_length();
        if !self.first_minute {
            added_minute = self.radio_datetime.add_minute();
        }
        if self.second == minute_length {
            self.parity_1 = get_parity(&self.bit_buffer, 21, 27, self.bit_buffer[28]);
            self.radio_datetime.set_minute(
                get_bcd_value(&self.bit_buffer, 21, 27),
                self.parity_1 == Some(false),
                added_minute && !self.first_minute,
            );

            self.parity_2 = get_parity(&self.bit_buffer, 29, 34, self.bit_buffer[35]);
            self.radio_datetime.set_hour(
                get_bcd_value(&self.bit_buffer, 29, 34),
                self.parity_2 == Some(false),
                added_minute && !self.first_minute,
            );

            self.parity_3 = get_parity(&self.bit_buffer, 36, 57, self.bit_buffer[58]);

            self.radio_datetime.set_weekday(
                get_bcd_value(&self.bit_buffer, 42, 44),
                self.parity_3 == Some(false),
                added_minute && !self.first_minute,
            );

            self.radio_datetime.set_month(
                get_bcd_value(&self.bit_buffer, 45, 49),
                self.parity_3 == Some(false),
                added_minute && !self.first_minute,
            );

            self.radio_datetime.set_year(
                get_bcd_value(&self.bit_buffer, 50, 57),
                self.parity_3 == Some(false),
                added_minute && !self.first_minute,
            );

            self.radio_datetime.set_day(
                get_bcd_value(&self.bit_buffer, 36, 41),
                self.parity_3 == Some(false),
                added_minute && !self.first_minute,
            );

            let dst = if self.bit_buffer[17].is_some()
                && self.bit_buffer[18].is_some()
                && self.bit_buffer[17] != self.bit_buffer[18]
            {
                self.bit_buffer[17]
            } else {
                None
            };
            self.radio_datetime.set_dst(
                dst,
                self.bit_buffer[16],
                added_minute && !self.first_minute,
            );

            // set_leap_second() wants minute length in seconds
            self.radio_datetime
                .set_leap_second(self.bit_buffer[19], minute_length + 1);
            self.leap_second_is_one = None;
            let leap_second = self.radio_datetime.get_leap_second();
            if leap_second.is_some() && (leap_second.unwrap() & LEAP_PROCESSED) != 0 {
                self.leap_second_is_one = Some(self.bit_buffer[59] == Some(true));
            }

            self.radio_datetime.bump_minutes_running();
        }
    }
}

impl Default for DCF77Utils {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::DCF77Utils;
    use radio_datetime_utils::{
        DST_ANNOUNCED, DST_PROCESSED, DST_SUMMER, LEAP_ANNOUNCED, LEAP_PROCESSED,
    };

    const BIT_BUFFER: [bool; 59 /* EOM not included */] = [
        false, // 0
        false, true, false, false, true, true, true, true, false, false, false, true, true, false, // 0x18f2
        true, // call bit set!
        false, true, false, // regular DST
        false, // no leap second announcement
        true, // 1
        false, true, true, false, false, false, true, true, // minute 46 + parity
        false, true, true, false, true, false, true, // hour 16 + parity
        false, true, false, false, false, true, // day 22
        false, true, true, // Saturday
        false, false, false, false, true, // October
        false, true, false, false, false, true, false, false, // year 22
        true, // date parity
        // None, // end-of-minute
    ];

    #[test]
    fn test_get_third_party_buffer_ok() {
        let mut dcf77 = DCF77Utils::default();
        for b in 1..=14 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        assert_eq!(dcf77.get_third_party_buffer(), Some(0x18f2)); // random value
    }
    #[test]
    fn test_get_third_party_buffer_none() {
        let mut dcf77 = DCF77Utils::default();
        for b in 1..=14 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.bit_buffer[4] = None;
        assert_eq!(dcf77.get_third_party_buffer(), None); // contains a None value
    }

    #[test]
    fn test_new_edge() {
        // TODO implement
    }

    #[test]
    fn test_decode_time_incomplete_minute() {
        let mut dcf77 = DCF77Utils::default();
        assert_eq!(dcf77.first_minute, true);
        dcf77.second = 42;
        // note that dcf77.bit_buffer is still empty
        assert_ne!(dcf77.get_this_minute_length(), dcf77.second);
        assert_ne!(dcf77.get_next_minute_length(), dcf77.second);
        assert_eq!(dcf77.parity_1, None);
        dcf77.decode_time();
        // not enough seconds in this minute, so nothing should happen:
        assert_eq!(dcf77.parity_1, None);
    }
    #[test]
    fn test_decode_time_complete_minute_ok() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.decode_time();
        // we should have a valid decoding:
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(46));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), Some(6));
        assert_eq!(dcf77.radio_datetime.get_day(), Some(22));
        assert_eq!(dcf77.radio_datetime.get_month(), Some(10));
        assert_eq!(dcf77.radio_datetime.get_year(), Some(22));
        assert_eq!(dcf77.parity_1, Some(false));
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, Some(false));
        assert_eq!(dcf77.radio_datetime.get_dst(), Some(DST_SUMMER));
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
    }
    #[test]
    fn test_decode_time_complete_minute_bad_bits() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        // introduce some distortions:
        dcf77.bit_buffer[26] = Some(!dcf77.bit_buffer[26].unwrap());
        dcf77.bit_buffer[39] = None;
        dcf77.decode_time();
        assert_eq!(dcf77.radio_datetime.get_minute(), None); // bad parity and first decoding
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), None); // broken parity and first decoding
        assert_eq!(dcf77.radio_datetime.get_day(), None); // broken bit
        assert_eq!(dcf77.radio_datetime.get_month(), None); // broken parity and first decoding
        assert_eq!(dcf77.radio_datetime.get_year(), None); // broken parity and first decoding
        assert_eq!(dcf77.parity_1, Some(true)); // bad parity
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, None); // broken bit
        assert_eq!(dcf77.radio_datetime.get_dst(), Some(DST_SUMMER));
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
    }
    #[test]
    fn continue_decode_time_complete_minute_jumped_values() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.decode_time();
        dcf77.first_minute = false;
        // minute 46 is really cool, so do not update bit 21 (and 28)
        dcf77.decode_time();
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(46));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), Some(6));
        assert_eq!(dcf77.radio_datetime.get_day(), Some(22));
        assert_eq!(dcf77.radio_datetime.get_month(), Some(10));
        assert_eq!(dcf77.radio_datetime.get_year(), Some(22));
        assert_eq!(dcf77.parity_1, Some(false));
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, Some(false));
        assert_eq!(dcf77.radio_datetime.get_dst(), Some(DST_SUMMER));
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.radio_datetime.get_jump_minute(), true);
        assert_eq!(dcf77.radio_datetime.get_jump_hour(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_weekday(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_day(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_month(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_year(), false);
    }
    #[test]
    fn continue_decode_time_complete_minute_bad_bits() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.decode_time();
        dcf77.first_minute = false;
        // update bit 21 and 28 for the next minute:
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(false);
        // introduce some distortions:
        dcf77.bit_buffer[26] = Some(!dcf77.bit_buffer[26].unwrap());
        dcf77.bit_buffer[39] = None;
        dcf77.decode_time();
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(47)); // bad parity
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), Some(6)); // broken parity
        assert_eq!(dcf77.radio_datetime.get_day(), Some(22)); // broken bit
        assert_eq!(dcf77.radio_datetime.get_month(), Some(10)); // broken parity
        assert_eq!(dcf77.radio_datetime.get_year(), Some(22)); // broken parity
        assert_eq!(dcf77.parity_1, Some(true)); // bad parity
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, None); // broken bit
        assert_eq!(dcf77.radio_datetime.get_dst(), Some(DST_SUMMER));
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.radio_datetime.get_jump_minute(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_hour(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_weekday(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_day(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_month(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_year(), false);
    }
    #[test]
    fn continue2_decode_time_complete_minute_leap_second_is_one() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second); // sanity check
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second); // sanity check
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        // leap second must be at top of hour and
        // announcements only count before the hour, so set minute to 59:
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[22] = Some(false);
        dcf77.bit_buffer[23] = Some(false);
        dcf77.bit_buffer[24] = Some(true);
        dcf77.bit_buffer[25] = Some(true);
        dcf77.bit_buffer[26] = Some(false);
        dcf77.bit_buffer[27] = Some(true);
        dcf77.bit_buffer[28] = Some(false);
        // announce a leap second:
        dcf77.bit_buffer[19] = Some(true);
        dcf77.decode_time();
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(59));
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(LEAP_ANNOUNCED));
        assert_eq!(dcf77.second, 59);
        assert_eq!(dcf77.get_this_minute_length(), 59);
        assert_eq!(dcf77.get_next_minute_length(), 60);

        // next minute and hour:
        dcf77.bit_buffer[21] = Some(false);
        dcf77.bit_buffer[24] = Some(false);
        dcf77.bit_buffer[25] = Some(false);
        dcf77.bit_buffer[27] = Some(false);
        dcf77.bit_buffer[29] = Some(true);
        dcf77.bit_buffer[35] = Some(false);
        // which will have a leap second:
        dcf77.bit_buffer[19] = Some(true); // not sure but should not matter
                                           // which has value 1 instead of 0:
        dcf77.bit_buffer[59] = Some(true);
        dcf77.second = 60; // 60 bits (61 seconds here, before decoding)

        // leave dcf77.fist_minute true on purpose to catch minute-length bugs
        dcf77.decode_time();
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(0));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(17));
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(LEAP_PROCESSED));
        assert_eq!(dcf77.second, 60);
        assert_eq!(dcf77.get_this_minute_length(), 60);
        assert_eq!(dcf77.get_next_minute_length(), 59);
        assert_eq!(dcf77.get_leap_second_is_one(), Some(true));

        // next regular minute:
        dcf77.bit_buffer[19] = Some(false);
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(true);
        // dcf77.bit_buffer[59] remains Some() but is never touched again
        dcf77.second = 59;
        dcf77.decode_time();
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(1));
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.second, 59); // sanity check
        assert_eq!(dcf77.get_this_minute_length(), 59);
        assert_eq!(dcf77.get_next_minute_length(), 59);
    }
    #[test]
    fn continue_decode_time_complete_minute_dst_change() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.second = 59;
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        // DST change must be at top of hour and
        // announcements only count before the hour, so set minute to 59:
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[22] = Some(false);
        dcf77.bit_buffer[23] = Some(false);
        dcf77.bit_buffer[24] = Some(true);
        dcf77.bit_buffer[25] = Some(true);
        dcf77.bit_buffer[26] = Some(false);
        dcf77.bit_buffer[27] = Some(true);
        dcf77.bit_buffer[28] = Some(false);
        // announce a DST change:
        dcf77.bit_buffer[16] = Some(true);
        dcf77.decode_time();
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(59));
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(DST_ANNOUNCED | DST_SUMMER)
        );
        // next minute and hour:
        dcf77.bit_buffer[21] = Some(false);
        dcf77.bit_buffer[24] = Some(false);
        dcf77.bit_buffer[25] = Some(false);
        dcf77.bit_buffer[27] = Some(false);
        dcf77.bit_buffer[29] = Some(true);
        dcf77.bit_buffer[35] = Some(false);
        // which will have a DST change:
        dcf77.bit_buffer[16] = Some(true); // not sure but should not matter
        dcf77.bit_buffer[17] = Some(false);
        dcf77.bit_buffer[18] = Some(true);
        // leave dcf77.fist_minute true on purpose to catch minute-length bugs
        dcf77.decode_time();
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(0));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(17));
        assert_eq!(dcf77.radio_datetime.get_dst(), Some(DST_PROCESSED)); // DST flipped off
    }

    #[test]
    fn test_increase_second_same_minute_ok() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.second = 37;
        // all date/time values are None
        dcf77.increase_second();
        assert_eq!(dcf77.first_minute, true);
        assert_eq!(dcf77.second, 38);
    }
    #[test]
    fn test_increase_second_same_minute_overflow() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.second = 59;
        // leap second value is None
        dcf77.increase_second();
        assert_eq!(dcf77.first_minute, true);
        assert_eq!(dcf77.second, 0);
    }
    #[test]
    fn test_increase_second_new_minute_ok() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.new_minute = true;
        dcf77.second = 59;
        dcf77.bit_buffer[0] = Some(false);
        dcf77.bit_buffer[20] = Some(true);
        dcf77.radio_datetime.set_year(Some(22), true, false);
        dcf77.radio_datetime.set_month(Some(10), true, false);
        dcf77.radio_datetime.set_weekday(Some(6), true, false);
        dcf77.radio_datetime.set_day(Some(22), true, false);
        dcf77.radio_datetime.set_hour(Some(12), true, false);
        dcf77.radio_datetime.set_minute(Some(59), true, false);
        dcf77.radio_datetime.set_dst(Some(true), Some(false), false);
        // leap second value is None
        dcf77.increase_second();
        assert_eq!(dcf77.first_minute, false);
        assert_eq!(dcf77.second, 0);
    }
    #[test]
    fn test_increase_second_new_minute_none_values() {
        let mut dcf77 = DCF77Utils::default();
        dcf77.new_minute = true;
        dcf77.second = 59;
        // all date/time values left None
        dcf77.increase_second();
        assert_eq!(dcf77.first_minute, true);
        assert_eq!(dcf77.second, 0);
    }
}
