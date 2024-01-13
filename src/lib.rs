//! Collection of utilities for DCF77 receivers.

//! Build with no_std for embedded platforms.
#![cfg_attr(not(test), no_std)]

use radio_datetime_utils::{radio_datetime_helpers, RadioDateTimeUtils};

pub mod dcf77_helpers;

/// Default upper limit for spike detection in microseconds
const SPIKE_LIMIT: u32 = 30_000;
/// Maximum time in microseconds for a bit to be considered 0
const ACTIVE_LIMIT: u32 = 150_000;
/// Maximum time in microseconds for a bit to be considered 1
const ACTIVE_RUNAWAY: u32 = 250_000;
/// Minimum time in microseconds for a new minute to be detected
const MINUTE_LIMIT: u32 = 1_500_000;
/// Signal is considered lost after this many microseconds
const PASSIVE_RUNAWAY: u32 = 2_500_000;

pub enum DecodeType {
    Live,
    LogFile,
}

/// DCF77 decoder class
pub struct DCF77Utils {
    decode_type: DecodeType,
    first_minute: bool,
    new_minute: bool,
    new_second: bool,
    second: u8,
    old_second: u8, // to see how long the minute was
    bit_buffer: [Option<bool>; radio_datetime_utils::BIT_BUFFER_SIZE],
    radio_datetime: RadioDateTimeUtils,
    leap_second_is_one: Option<bool>,
    parity_1: Option<bool>,
    parity_2: Option<bool>,
    parity_3: Option<bool>,
    bit_0: Option<bool>,
    third_party: Option<u16>,
    call_bit: Option<bool>,
    bit_20: Option<bool>,
    // below for handle_new_edge()
    before_first_edge: bool,
    t0: u32,
    spike_limit: u32,
}

/// Abstract generic version of get_*_minute_length()
///
/// # Arguments
/// * `$self` - instance of DCF77Utils
/// * `$condition` - an optional condition that must hold.
/// * `$flags` - flags that must apply.
macro_rules! get_minute_length {
    ($self: expr, $condition: expr, $flags: expr) => {
        if let Some(s_leap_second) = $self.radio_datetime.get_leap_second() {
            if $condition && ((s_leap_second & $flags) != 0) {
                61
            } else {
                60
            }
        } else {
            60
        }
    };
}

impl DCF77Utils {
    /// Initialize a new DCF77Utils instance.
    pub fn new(dt: DecodeType) -> Self {
        Self {
            decode_type: dt,
            first_minute: true,
            new_minute: false,
            new_second: false,
            second: 0,
            old_second: 0,
            bit_buffer: [None; radio_datetime_utils::BIT_BUFFER_SIZE],
            radio_datetime: RadioDateTimeUtils::new(7),
            leap_second_is_one: None,
            parity_1: None,
            parity_2: None,
            parity_3: None,
            bit_0: None,
            third_party: None,
            call_bit: None,
            bit_20: None,
            before_first_edge: true,
            t0: 0,
            spike_limit: SPIKE_LIMIT,
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
    ///
    /// This method must be called _before_ `increase_second()`
    pub fn force_new_minute(&mut self) {
        self.new_minute = true;
    }

    /// Return if a new second has arrived.
    pub fn get_new_second(&self) -> bool {
        self.new_second
    }

    /// Get the old second counter.
    pub fn get_old_second(&self) -> u8 {
        self.old_second
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
    /// This method must be called _before_ `increase_second()`
    ///
    /// # Arguments
    /// * `value` - the value to set the current bit to
    pub fn set_current_bit(&mut self, value: Option<bool>) {
        self.bit_buffer[self.second as usize] = value;
        self.new_minute = false;
    }

    /// Get the value of bit 0 (must always be 0).
    pub fn get_bit_0(&self) -> Option<bool> {
        self.bit_0
    }

    /// Get the value of the third-party buffer, a 14-bit number with the least significant bit first.
    pub fn get_third_party_buffer(&self) -> Option<u16> {
        self.third_party
    }

    /// Get the value of the transmitter call bit.
    pub fn get_call_bit(&self) -> Option<bool> {
        self.call_bit
    }

    /// Get the value of bit 20 (must always be 1).
    pub fn get_bit_20(&self) -> Option<bool> {
        self.bit_20
    }

    /// Return the current spike limit in microseconds.
    pub fn get_spike_limit(&self) -> u32 {
        self.spike_limit
    }

    /// Set the new spike limit in microseconds, [0(off)..ACTIVE_LIMIT)
    ///
    /// # Arguments
    /// * `value` - the value to set the spike limit to.
    pub fn set_spike_limit(&mut self, value: u32) {
        if value < ACTIVE_LIMIT {
            self.spike_limit = value;
        }
    }

    /// Determine the bit value if a new edge is received. indicates reception errors,
    /// and checks if a new minute has started.
    ///
    /// This function can deal with spikes, which are arbitrarily set to `spike_limit` microseconds.
    ///
    /// This method must be called _after_ `increase_second()`
    ///
    /// # Arguments
    /// * `is_low_edge` - indicates that the edge has gone from high to low (as opposed to
    ///                   low-to-high).
    /// * `t` - time stamp of the received edge, in microseconds
    pub fn handle_new_edge(&mut self, is_low_edge: bool, t: u32) {
        if self.before_first_edge {
            self.before_first_edge = false;
            self.t0 = t;
            return;
        }
        let t_diff = radio_datetime_helpers::time_diff(self.t0, t);
        if t_diff < self.spike_limit {
            // Shift t0 to deal with a train of spikes adding up to more than `spike_limit` microseconds.
            self.t0 += t_diff;
            return; // random positive or negative spike, ignore
        }
        self.t0 = t;
        if is_low_edge {
            // leave self.new_minute unaltered
            self.new_second = false;
            self.bit_buffer[self.second as usize] = if t_diff < ACTIVE_LIMIT {
                Some(false)
            } else if t_diff < ACTIVE_RUNAWAY {
                Some(true)
            } else {
                None // broken bit, active runaway
            };
        } else if t_diff < PASSIVE_RUNAWAY {
            self.new_minute = t_diff > MINUTE_LIMIT;
            self.new_second = t_diff > 1_000_000 - ACTIVE_RUNAWAY;
        } else {
            self.bit_buffer[self.second as usize] = None; // broken bit, passive runaway
        }
    }

    /// Determine the length of _this_ minute in seconds, tolerate None as leap second state.
    pub fn get_this_minute_length(&self) -> u8 {
        get_minute_length!(self, true, radio_datetime_utils::LEAP_PROCESSED)
    }

    /// Determine the length of _the next_ minute in seconds, tolerate None as a leap second state.
    pub fn get_next_minute_length(&self) -> u8 {
        get_minute_length!(
            self,
            self.radio_datetime.get_minute() == Some(59),
            radio_datetime_utils::LEAP_ANNOUNCED
        )
    }

    /// Increase or reset `second`.
    ///
    /// Returns if the second counter was increased/wrapped normally (true)
    /// or due to an overflow (false).
    ///
    /// This method must be called _after_ `decode_time()`, `handle_new_edge()`,
    /// `set_current_bit()`, and `force_new_minute()`.
    pub fn increase_second(&mut self) -> bool {
        self.old_second = self.second;
        let minute_length = self.get_next_minute_length();
        RadioDateTimeUtils::increase_second(&mut self.second, self.new_minute, minute_length)
    }

    /// Call add_minute() on `self.radio_datetime` and passes on that result.
    ///
    /// This could be useful for consumers just wanting to advance their current date/time.
    pub fn add_minute(&mut self) -> bool {
        self.radio_datetime.clear_jumps();
        self.radio_datetime.add_minute()
    }

    /// Decode the time broadcast during the last minute and clear `first_minute` when appropriate.
    ///
    /// This method must be called _before_ `increase_second()` in LogFile mode
    /// and _after_ `increase_second()` in Live mode.
    ///
    /// # Arguments
    /// * `strict_checks` - checks all parities, DST validity, bit 0, and bit 20 when setting
    ///                     date/time and clearing self.first_minute
    pub fn decode_time(&mut self, strict_checks: bool) {
        self.radio_datetime.clear_jumps();
        let mut added_minute = false;
        let minute_length = self.get_next_minute_length();
        if !self.first_minute {
            added_minute = self.radio_datetime.add_minute();
        }
        if 1 + match self.decode_type {
            DecodeType::Live => self.old_second,
            DecodeType::LogFile => self.second,
        } == minute_length
        {
            self.bit_0 = self.bit_buffer[0];
            self.third_party = dcf77_helpers::get_binary_value(&self.bit_buffer, 1, 14);
            self.call_bit = self.bit_buffer[15];
            self.bit_20 = self.bit_buffer[20];

            self.parity_1 =
                radio_datetime_helpers::get_parity(&self.bit_buffer, 21, 27, self.bit_buffer[28]);
            self.parity_2 =
                radio_datetime_helpers::get_parity(&self.bit_buffer, 29, 34, self.bit_buffer[35]);
            self.parity_3 =
                radio_datetime_helpers::get_parity(&self.bit_buffer, 36, 57, self.bit_buffer[58]);

            let dst = if self.bit_buffer[17].is_some()
                && self.bit_buffer[18].is_some()
                && self.bit_buffer[17] != self.bit_buffer[18]
            {
                self.bit_buffer[17]
            } else {
                None
            };

            let strict_ok = self.parity_1 == Some(false)
                && self.parity_2 == Some(false)
                && self.parity_3 == Some(false)
                && self.bit_0 == Some(false)
                && self.bit_20 == Some(true)
                && dst.is_some();

            self.radio_datetime.set_minute(
                radio_datetime_helpers::get_bcd_value(&self.bit_buffer, 21, 27),
                if strict_checks {
                    strict_ok
                } else {
                    self.parity_1 == Some(false)
                },
                added_minute && !self.first_minute,
            );

            self.radio_datetime.set_hour(
                radio_datetime_helpers::get_bcd_value(&self.bit_buffer, 29, 34),
                if strict_checks {
                    strict_ok
                } else {
                    self.parity_2 == Some(false)
                },
                added_minute && !self.first_minute,
            );

            self.radio_datetime.set_weekday(
                radio_datetime_helpers::get_bcd_value(&self.bit_buffer, 42, 44),
                if strict_checks {
                    strict_ok
                } else {
                    self.parity_3 == Some(false)
                },
                added_minute && !self.first_minute,
            );

            self.radio_datetime.set_month(
                radio_datetime_helpers::get_bcd_value(&self.bit_buffer, 45, 49),
                if strict_checks {
                    strict_ok
                } else {
                    self.parity_3 == Some(false)
                },
                added_minute && !self.first_minute,
            );

            self.radio_datetime.set_year(
                radio_datetime_helpers::get_bcd_value(&self.bit_buffer, 50, 57),
                if strict_checks {
                    strict_ok
                } else {
                    self.parity_3 == Some(false)
                },
                added_minute && !self.first_minute,
            );

            self.radio_datetime.set_day(
                radio_datetime_helpers::get_bcd_value(&self.bit_buffer, 36, 41),
                if strict_checks {
                    strict_ok
                } else {
                    self.parity_3 == Some(false)
                },
                added_minute && !self.first_minute,
            );

            self.radio_datetime.set_dst(
                dst,
                self.bit_buffer[16],
                added_minute && !self.first_minute,
            );

            self.radio_datetime
                .set_leap_second(self.bit_buffer[19], minute_length);
            self.leap_second_is_one = None;
            let leap_second = self.radio_datetime.get_leap_second();
            if leap_second.is_some()
                && (leap_second.unwrap() & radio_datetime_utils::LEAP_PROCESSED) != 0
            {
                self.leap_second_is_one = Some(self.bit_buffer[59] == Some(true));
            }

            if if strict_checks {
                strict_ok
            } else {
                self.bit_0 == Some(false) && self.bit_20 == Some(true)
            } && self.radio_datetime.is_valid()
            {
                // allow displaying of information after the first properly decoded minute
                self.first_minute = false;
            }

            self.radio_datetime.bump_minutes_running();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIT_BUFFER: [bool; 59 /* EOM not included */] = [
        false, // 0
        false, true, false, false, true, true, true, true, false, false, false, true, true, false, // 0x18f2
        true, // call bit set!
        false, true, false, // regular DST
        false, // no leap second announcement
        true, // 1
        false, false, false, true, true, false, true, true, // minute 58 + parity
        false, true, true, false, true, false, true, // hour 16 + parity
        false, true, false, false, false, true, // day 22
        false, true, true, // Saturday
        false, false, false, false, true, // October
        false, true, false, false, false, true, false, false, // year 22
        true, // date parity
        // None, // end-of-minute
    ];

    #[test]
    fn test_new_edge_bit_0() {
        const EDGE_BUFFER: [(bool, u32); 4] = [
            // Some(false) bit value
            (!false, 366_097_734),
            (!true, 366_879_141),
            (!false, 366_993_436), // 114_295 us
            (!true, 367_879_221),
        ];
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        assert_eq!(dcf77.before_first_edge, true);
        dcf77.handle_new_edge(EDGE_BUFFER[0].0, EDGE_BUFFER[0].1);
        assert_eq!(dcf77.before_first_edge, false);
        assert_eq!(dcf77.t0, EDGE_BUFFER[0].1); // very first edge

        dcf77.handle_new_edge(EDGE_BUFFER[1].0, EDGE_BUFFER[1].1); // first significant edge
        assert_eq!(dcf77.t0, EDGE_BUFFER[1].1); // longer than a spike
        assert_eq!(dcf77.new_second, true);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), None); // not yet determined, passive part

        dcf77.handle_new_edge(EDGE_BUFFER[2].0, EDGE_BUFFER[2].1);
        assert_eq!(dcf77.t0, EDGE_BUFFER[2].1); // longer than a spike
        assert_eq!(dcf77.new_second, false);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), Some(false)); // 114_295 microseconds

        // passive part of second must keep the bit value
        dcf77.handle_new_edge(EDGE_BUFFER[3].0, EDGE_BUFFER[3].1);
        assert_eq!(dcf77.t0, EDGE_BUFFER[3].1); // longer than a spike
        assert_eq!(dcf77.new_second, true);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), Some(false)); // keep bit value
    }
    #[test]
    fn test_new_edge_bit_1() {
        const EDGE_BUFFER: [(bool, u32); 4] = [
            // Some(true) bit value
            (!false, 361_997_291),
            (!true, 362_879_580),
            (!false, 363_096_452), // 216_872 us
            (!true, 363_879_672),
        ];
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        assert_eq!(dcf77.before_first_edge, true);
        dcf77.handle_new_edge(EDGE_BUFFER[0].0, EDGE_BUFFER[0].1);
        assert_eq!(dcf77.before_first_edge, false);
        assert_eq!(dcf77.t0, EDGE_BUFFER[0].1); // very first edge

        dcf77.handle_new_edge(EDGE_BUFFER[1].0, EDGE_BUFFER[1].1); // first significant edge
        assert_eq!(dcf77.t0, EDGE_BUFFER[1].1); // longer than a spike
        assert_eq!(dcf77.new_second, true);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), None); // not yet determined, passive part

        dcf77.handle_new_edge(EDGE_BUFFER[2].0, EDGE_BUFFER[2].1);
        assert_eq!(dcf77.t0, EDGE_BUFFER[2].1); // longer than a spike
        assert_eq!(dcf77.new_second, false);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), Some(true)); // 216_872 microseconds

        // passive part of second must keep the bit value
        dcf77.handle_new_edge(EDGE_BUFFER[3].0, EDGE_BUFFER[3].1);
        assert_eq!(dcf77.t0, EDGE_BUFFER[3].1); // longer than a spike
        assert_eq!(dcf77.new_second, true);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), Some(true)); // keep bit value
    }
    #[test]
    fn test_new_edge_minute() {
        const EDGE_BUFFER: [(bool, u32); 3] = [
            // new minute, Some(false) bit value
            (!true, 419_878_222),
            (!false, 419_994_127),
            (!true, 421_879_420), // 1_885_293 us
        ];
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        assert_eq!(dcf77.before_first_edge, true);
        dcf77.handle_new_edge(EDGE_BUFFER[0].0, EDGE_BUFFER[0].1);
        assert_eq!(dcf77.before_first_edge, false);
        assert_eq!(dcf77.t0, EDGE_BUFFER[0].1); // very first edge

        dcf77.handle_new_edge(EDGE_BUFFER[1].0, EDGE_BUFFER[1].1); // first significant edge
        assert_eq!(dcf77.t0, EDGE_BUFFER[1].1); // longer than a spike
        assert_eq!(dcf77.new_second, false);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), Some(false));

        assert_eq!(dcf77.increase_second(), true);

        dcf77.handle_new_edge(EDGE_BUFFER[2].0, EDGE_BUFFER[2].1);
        assert_eq!(dcf77.t0, EDGE_BUFFER[2].1); // longer than a spike
        assert_eq!(dcf77.new_second, true);
        assert_eq!(dcf77.new_minute, true);
        assert_eq!(dcf77.get_current_bit(), None); // 1_885_293 microseconds, end-of-minute marker
    }
    #[test]
    fn test_new_edge_active_runaway() {
        const EDGE_BUFFER: [(bool, u32); 3] = [
            // active runaway (broken bit)
            (!false, 3_303_417_788),
            (!true, 3_304_200_237),
            (!false, 3_304_674_788), // 474_551 us
        ];
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        assert_eq!(dcf77.before_first_edge, true);
        dcf77.handle_new_edge(EDGE_BUFFER[0].0, EDGE_BUFFER[0].1);
        assert_eq!(dcf77.before_first_edge, false);
        assert_eq!(dcf77.t0, EDGE_BUFFER[0].1); // very first edge

        dcf77.handle_new_edge(EDGE_BUFFER[1].0, EDGE_BUFFER[1].1); // first significant edge
        assert_eq!(dcf77.t0, EDGE_BUFFER[1].1); // longer than a spike
        assert_eq!(dcf77.new_second, true);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), None); // not yet determined, passive part

        dcf77.handle_new_edge(EDGE_BUFFER[2].0, EDGE_BUFFER[2].1);
        assert_eq!(dcf77.t0, EDGE_BUFFER[2].1); // longer than a spike
        assert_eq!(dcf77.new_second, false);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), None); // 474_551 microseconds
    }
    #[test]
    fn test_new_edge_passive_runaway() {
        const EDGE_BUFFER: [(bool, u32); 3] = [
            // passive runaway (transmitter outage)
            (!true, 2_917_778_338),
            (!false, 2_917_791_465),
            (!true, 2_920_614_145),
        ];
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        assert_eq!(dcf77.before_first_edge, true);
        dcf77.handle_new_edge(EDGE_BUFFER[0].0, EDGE_BUFFER[0].1);
        assert_eq!(dcf77.before_first_edge, false);
        assert_eq!(dcf77.t0, EDGE_BUFFER[0].1); // very first edge

        dcf77.handle_new_edge(EDGE_BUFFER[1].0, EDGE_BUFFER[1].1); // first significant edge
        assert_eq!(dcf77.t0, EDGE_BUFFER[1].1); // actually a spike
        assert_eq!(dcf77.new_second, false);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), None); // not yet determined, passive part

        dcf77.handle_new_edge(EDGE_BUFFER[2].0, EDGE_BUFFER[2].1);
        assert_eq!(dcf77.t0, EDGE_BUFFER[2].1); // longer than a spike
        assert_eq!(dcf77.new_second, false);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), None); // 2_822_680 microseconds
    }
    #[test]
    fn test_new_edge_spikes() {
        const EDGE_BUFFER: [(bool, u32); 12] = [
            // spikes (also lot of same-edge transitions)
            (!true, 111_141_523),
            (!false, 111_256_572), // 115_049 us
            (!true, 111_286_015),  // 29_443 us
            (!false, 111_286_025), // 10 us
            (!true, 111_286_651),  // 626 us
            (!false, 111_286_683), // 32 us
            (!true, 111_286_815),  // 132 us
            (!false, 111_286_873), // 58 us
            (!true, 111_286_977),  // 104 us
            (!false, 111_287_012), // 35 us
            (!true, 112_141_743),  // 854_731 us
            (!false, 112_359_105), // 217_362 us
        ];
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        assert_eq!(dcf77.before_first_edge, true);
        dcf77.handle_new_edge(EDGE_BUFFER[0].0, EDGE_BUFFER[0].1);
        assert_eq!(dcf77.before_first_edge, false);
        assert_eq!(dcf77.t0, EDGE_BUFFER[0].1); // very first edge

        dcf77.handle_new_edge(EDGE_BUFFER[1].0, EDGE_BUFFER[1].1); // first significant edge
        assert_eq!(dcf77.t0, EDGE_BUFFER[1].1); // longer than a spike
        assert_eq!(dcf77.new_second, false);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), Some(false)); // 115_049

        // Feed a bunch of spikes of less than spike_limit us, nothing should happen
        let mut spike = dcf77.t0;
        for i in 2..=9 {
            spike += radio_datetime_helpers::time_diff(EDGE_BUFFER[i - 1].1, EDGE_BUFFER[i].1);
            dcf77.handle_new_edge(EDGE_BUFFER[i].0, EDGE_BUFFER[i].1);
            assert_eq!(dcf77.t0, spike);
            assert_eq!(dcf77.new_second, false);
            assert_eq!(dcf77.new_minute, false);
            assert_eq!(dcf77.get_current_bit(), Some(false));
        }
        dcf77.handle_new_edge(EDGE_BUFFER[10].0, EDGE_BUFFER[10].1);
        assert_eq!(dcf77.t0, EDGE_BUFFER[10].1); // longer than a spike
        assert_eq!(dcf77.new_second, true);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), Some(false)); // 854_731 microseconds, keep bit value

        // A 1-bit should arrive next:
        dcf77.handle_new_edge(EDGE_BUFFER[11].0, EDGE_BUFFER[11].1);
        assert_eq!(dcf77.t0, EDGE_BUFFER[11].1); // longer than a spike
        assert_eq!(dcf77.new_second, false);
        assert_eq!(dcf77.new_minute, false);
        assert_eq!(dcf77.get_current_bit(), Some(true)); // 217_362 microseconds
    }

    // relaxed checks
    #[test]
    fn test_decode_time_incomplete_minute() {
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        assert_eq!(dcf77.first_minute, true);
        dcf77.old_second = 41;
        dcf77.second = 42;
        // note that dcf77.bit_buffer is still empty
        assert_ne!(dcf77.get_this_minute_length(), dcf77.old_second);
        assert_ne!(dcf77.get_next_minute_length(), dcf77.old_second);
        assert_ne!(dcf77.get_this_minute_length(), dcf77.second);
        assert_ne!(dcf77.get_next_minute_length(), dcf77.second);
        assert_eq!(dcf77.parity_1, None);
        dcf77.decode_time(false);
        // not enough seconds in this minute, so nothing should happen:
        assert_eq!(dcf77.parity_1, None);
        assert_eq!(dcf77.get_bit_0(), None);
        assert_eq!(dcf77.get_third_party_buffer(), None);
        assert_eq!(dcf77.get_call_bit(), None);
        assert_eq!(dcf77.get_bit_20(), None);
    }
    #[test]
    fn test_decode_time_complete_minute_ok() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second + 1);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second + 1);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.decode_time(false);
        // we should have a valid decoding:
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(58));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), Some(6));
        assert_eq!(dcf77.radio_datetime.get_day(), Some(22));
        assert_eq!(dcf77.radio_datetime.get_month(), Some(10));
        assert_eq!(dcf77.radio_datetime.get_year(), Some(22));
        assert_eq!(dcf77.parity_1, Some(false));
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, Some(false));
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_SUMMER)
        );
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.get_bit_0(), Some(false));
        assert_eq!(dcf77.get_third_party_buffer(), Some(0x18f2)); // random value
        assert_eq!(dcf77.get_call_bit(), Some(true)); // because why not?
        assert_eq!(dcf77.get_bit_20(), Some(true));
    }
    #[test]
    fn test_decode_time_complete_minute_bad_bits() {
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        dcf77.old_second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.old_second + 1);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.old_second + 1);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        // introduce some distortions:
        dcf77.bit_buffer[26] = Some(!dcf77.bit_buffer[26].unwrap());
        dcf77.bit_buffer[39] = None;
        dcf77.decode_time(false);
        assert_eq!(dcf77.radio_datetime.get_minute(), None); // bad parity and first decoding
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), None); // broken parity and first decoding
        assert_eq!(dcf77.radio_datetime.get_day(), None); // broken bit
        assert_eq!(dcf77.radio_datetime.get_month(), None); // broken parity and first decoding
        assert_eq!(dcf77.radio_datetime.get_year(), None); // broken parity and first decoding
        assert_eq!(dcf77.parity_1, Some(true)); // bad parity
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, None); // broken bit
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_SUMMER)
        );
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.get_bit_0(), Some(false));
        assert_eq!(dcf77.get_third_party_buffer(), Some(0x18f2)); // random value
        assert_eq!(dcf77.get_call_bit(), Some(true)); // because why not?
        assert_eq!(dcf77.get_bit_20(), Some(true));
    }
    #[test]
    fn continue_decode_time_complete_minute_jumped_values() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second + 1);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second + 1);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.decode_time(false);
        assert_eq!(dcf77.first_minute, false);
        // minute 58 is really cool, so do not update bit 21 (and 28)
        dcf77.decode_time(false);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(58));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), Some(6));
        assert_eq!(dcf77.radio_datetime.get_day(), Some(22));
        assert_eq!(dcf77.radio_datetime.get_month(), Some(10));
        assert_eq!(dcf77.radio_datetime.get_year(), Some(22));
        assert_eq!(dcf77.parity_1, Some(false));
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, Some(false));
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_SUMMER)
        );
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.get_bit_0(), Some(false));
        assert_eq!(dcf77.get_third_party_buffer(), Some(0x18f2)); // random value
        assert_eq!(dcf77.get_call_bit(), Some(true)); // because why not?
        assert_eq!(dcf77.get_bit_20(), Some(true));
        assert_eq!(dcf77.radio_datetime.get_jump_minute(), true);
        assert_eq!(dcf77.radio_datetime.get_jump_hour(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_weekday(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_day(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_month(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_year(), false);
    }
    #[test]
    fn continue_decode_time_complete_minute_bad_bits() {
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        dcf77.old_second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.old_second + 1);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.old_second + 1);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.decode_time(false);
        assert_eq!(dcf77.first_minute, false);
        // update bit 21 and 28 for the next minute:
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(false);
        // introduce some distortions:
        dcf77.bit_buffer[26] = Some(!dcf77.bit_buffer[26].unwrap());
        dcf77.bit_buffer[39] = None;
        dcf77.decode_time(false);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(59)); // bad parity
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), Some(6)); // broken parity
        assert_eq!(dcf77.radio_datetime.get_day(), Some(22)); // broken bit
        assert_eq!(dcf77.radio_datetime.get_month(), Some(10)); // broken parity
        assert_eq!(dcf77.radio_datetime.get_year(), Some(22)); // broken parity
        assert_eq!(dcf77.parity_1, Some(true)); // bad parity
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, None); // broken bit
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_SUMMER)
        );
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.get_bit_0(), Some(false));
        assert_eq!(dcf77.get_third_party_buffer(), Some(0x18f2)); // random value
        assert_eq!(dcf77.get_call_bit(), Some(true)); // because why not?
        assert_eq!(dcf77.get_bit_20(), Some(true));
        assert_eq!(dcf77.radio_datetime.get_jump_minute(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_hour(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_weekday(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_day(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_month(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_year(), false);
    }
    #[test]
    fn continue2_decode_time_complete_minute_leap_second_is_one() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second + 1); // sanity check
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second + 1); // sanity check
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        // leap second must be at top of hour and
        // announcements only count before the hour, so set minute to 59:
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(false);
        // announce a leap second:
        dcf77.bit_buffer[19] = Some(true);
        dcf77.decode_time(false);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(59)); // sanity check
        assert_eq!(
            dcf77.radio_datetime.get_leap_second(),
            Some(radio_datetime_utils::LEAP_ANNOUNCED)
        );
        assert_eq!(dcf77.second, 59);
        assert_eq!(dcf77.get_this_minute_length(), 60);
        assert_eq!(dcf77.get_next_minute_length(), 61);

        // next minute and hour:
        dcf77.bit_buffer[21] = Some(false);
        dcf77.bit_buffer[24] = Some(false);
        dcf77.bit_buffer[25] = Some(false);
        dcf77.bit_buffer[27] = Some(false);
        dcf77.bit_buffer[29] = Some(true);
        dcf77.bit_buffer[35] = Some(false);
        // which will have a leap second:
        dcf77.bit_buffer[59] = Some(true); // which has value 1 instead of 0
        dcf77.second = 60; // 60 bits (61 seconds here, before decoding)

        dcf77.decode_time(false);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(0));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(17));
        assert_eq!(
            dcf77.radio_datetime.get_leap_second(),
            Some(radio_datetime_utils::LEAP_PROCESSED)
        );
        assert_eq!(dcf77.second, 60);
        assert_eq!(dcf77.get_this_minute_length(), 61);
        assert_eq!(dcf77.get_next_minute_length(), 60);
        assert_eq!(dcf77.get_leap_second_is_one(), Some(true));

        // next regular minute:
        dcf77.bit_buffer[19] = Some(false);
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(true);
        // dcf77.bit_buffer[59] remains Some() but is never touched again
        dcf77.second = 59;
        dcf77.decode_time(false);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(1));
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.second, 59); // sanity check
        assert_eq!(dcf77.get_this_minute_length(), 60);
        assert_eq!(dcf77.get_next_minute_length(), 60);
    }
    #[test]
    fn continue_decode_time_complete_minute_dst_change_to_winter() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 59;
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        // DST change must be at top of hour and
        // announcements only count before the hour, so set minute to 59:
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(false);
        // announce a DST change:
        dcf77.bit_buffer[16] = Some(true);
        dcf77.decode_time(false);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(59));
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_ANNOUNCED | radio_datetime_utils::DST_SUMMER)
        );
        // next minute and hour:
        dcf77.bit_buffer[21] = Some(false);
        dcf77.bit_buffer[24] = Some(false);
        dcf77.bit_buffer[25] = Some(false);
        dcf77.bit_buffer[27] = Some(false);
        dcf77.bit_buffer[29] = Some(true);
        dcf77.bit_buffer[35] = Some(false);
        // which will have a DST change:
        dcf77.bit_buffer[17] = Some(false);
        dcf77.bit_buffer[18] = Some(true);
        dcf77.decode_time(false);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(0));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(17));
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_PROCESSED)
        ); // DST flipped off
    }

    // strict checks
    #[test]
    fn test_decode_time_incomplete_minute_strict() {
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        assert_eq!(dcf77.first_minute, true);
        dcf77.old_second = 41;
        dcf77.second = 42;
        // note that dcf77.bit_buffer is still empty
        assert_ne!(dcf77.get_this_minute_length(), dcf77.old_second);
        assert_ne!(dcf77.get_next_minute_length(), dcf77.old_second);
        assert_ne!(dcf77.get_this_minute_length(), dcf77.second);
        assert_ne!(dcf77.get_next_minute_length(), dcf77.second);
        assert_eq!(dcf77.parity_1, None);
        dcf77.decode_time(true);
        // not enough seconds in this minute, so nothing should happen:
        assert_eq!(dcf77.parity_1, None);
        assert_eq!(dcf77.get_bit_0(), None);
        assert_eq!(dcf77.get_third_party_buffer(), None);
        assert_eq!(dcf77.get_call_bit(), None);
        assert_eq!(dcf77.get_bit_20(), None);
    }
    #[test]
    fn test_decode_time_complete_minute_ok_strict() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second + 1);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second + 1);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.decode_time(true);
        // we should have a valid decoding:
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(58));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), Some(6));
        assert_eq!(dcf77.radio_datetime.get_day(), Some(22));
        assert_eq!(dcf77.radio_datetime.get_month(), Some(10));
        assert_eq!(dcf77.radio_datetime.get_year(), Some(22));
        assert_eq!(dcf77.parity_1, Some(false));
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, Some(false));
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_SUMMER)
        );
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.get_bit_0(), Some(false));
        assert_eq!(dcf77.get_third_party_buffer(), Some(0x18f2)); // random value
        assert_eq!(dcf77.get_call_bit(), Some(true)); // because why not?
        assert_eq!(dcf77.get_bit_20(), Some(true));
    }
    #[test]
    fn test_decode_time_complete_minute_bad_bits_strict() {
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        dcf77.old_second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.old_second + 1);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.old_second + 1);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        // introduce some distortions:
        dcf77.bit_buffer[26] = Some(!dcf77.bit_buffer[26].unwrap());
        dcf77.bit_buffer[39] = None;
        dcf77.decode_time(true);
        assert_eq!(dcf77.radio_datetime.get_minute(), None); // bad parity and first decoding
        assert_eq!(dcf77.radio_datetime.get_hour(), None); // strict checks failed
        assert_eq!(dcf77.radio_datetime.get_weekday(), None); // broken parity and first decoding
        assert_eq!(dcf77.radio_datetime.get_day(), None); // broken bit
        assert_eq!(dcf77.radio_datetime.get_month(), None); // broken parity and first decoding
        assert_eq!(dcf77.radio_datetime.get_year(), None); // broken parity and first decoding
        assert_eq!(dcf77.parity_1, Some(true)); // bad parity
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, None); // broken bit
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_SUMMER)
        ); // not affected by strict checks
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.get_bit_0(), Some(false));
        assert_eq!(dcf77.get_third_party_buffer(), Some(0x18f2)); // random value
        assert_eq!(dcf77.get_call_bit(), Some(true)); // because why not?
        assert_eq!(dcf77.get_bit_20(), Some(true));
    }
    #[test]
    fn continue_decode_time_complete_minute_jumped_values_strict() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second + 1);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second + 1);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.decode_time(true);
        assert_eq!(dcf77.first_minute, false);
        // minute 58 is really cool, so do not update bit 21 (and 28)
        dcf77.decode_time(true);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(58));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), Some(6));
        assert_eq!(dcf77.radio_datetime.get_day(), Some(22));
        assert_eq!(dcf77.radio_datetime.get_month(), Some(10));
        assert_eq!(dcf77.radio_datetime.get_year(), Some(22));
        assert_eq!(dcf77.parity_1, Some(false));
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, Some(false));
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_SUMMER)
        );
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.get_bit_0(), Some(false));
        assert_eq!(dcf77.get_third_party_buffer(), Some(0x18f2)); // random value
        assert_eq!(dcf77.get_call_bit(), Some(true)); // because why not?
        assert_eq!(dcf77.get_bit_20(), Some(true));
        assert_eq!(dcf77.radio_datetime.get_jump_minute(), true);
        assert_eq!(dcf77.radio_datetime.get_jump_hour(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_weekday(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_day(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_month(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_year(), false);
    }
    #[test]
    fn continue_decode_time_complete_minute_bad_bits_strict() {
        let mut dcf77 = DCF77Utils::new(DecodeType::Live);
        dcf77.old_second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.old_second + 1);
        assert_eq!(dcf77.get_next_minute_length(), dcf77.old_second + 1);
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        dcf77.decode_time(true);
        assert_eq!(dcf77.first_minute, false);
        // update bit 21 and 28 for the next minute:
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(false);
        // introduce some distortions:
        dcf77.bit_buffer[26] = Some(!dcf77.bit_buffer[26].unwrap());
        dcf77.bit_buffer[39] = None;
        dcf77.decode_time(true);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(59)); // bad parity
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(16));
        assert_eq!(dcf77.radio_datetime.get_weekday(), Some(6)); // broken parity
        assert_eq!(dcf77.radio_datetime.get_day(), Some(22)); // broken bit
        assert_eq!(dcf77.radio_datetime.get_month(), Some(10)); // broken parity
        assert_eq!(dcf77.radio_datetime.get_year(), Some(22)); // broken parity
        assert_eq!(dcf77.parity_1, Some(true)); // bad parity
        assert_eq!(dcf77.parity_2, Some(false));
        assert_eq!(dcf77.parity_3, None); // broken bit
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_SUMMER)
        );
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.leap_second_is_one, None);
        assert_eq!(dcf77.get_bit_0(), Some(false));
        assert_eq!(dcf77.get_third_party_buffer(), Some(0x18f2)); // random value
        assert_eq!(dcf77.get_call_bit(), Some(true)); // because why not?
        assert_eq!(dcf77.get_bit_20(), Some(true));
        assert_eq!(dcf77.radio_datetime.get_jump_minute(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_hour(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_weekday(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_day(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_month(), false);
        assert_eq!(dcf77.radio_datetime.get_jump_year(), false);
    }
    #[test]
    fn continue2_decode_time_complete_minute_leap_second_is_one_strict() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 59;
        assert_eq!(dcf77.get_this_minute_length(), dcf77.second + 1); // sanity check
        assert_eq!(dcf77.get_next_minute_length(), dcf77.second + 1); // sanity check
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        // leap second must be at top of hour and
        // announcements only count before the hour, so set minute to 59:
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(false);
        // announce a leap second:
        dcf77.bit_buffer[19] = Some(true);
        dcf77.decode_time(true);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(59)); // sanity check
        assert_eq!(
            dcf77.radio_datetime.get_leap_second(),
            Some(radio_datetime_utils::LEAP_ANNOUNCED)
        );
        assert_eq!(dcf77.second, 59);
        assert_eq!(dcf77.get_this_minute_length(), 60);
        assert_eq!(dcf77.get_next_minute_length(), 61);

        // next minute and hour:
        dcf77.bit_buffer[21] = Some(false);
        dcf77.bit_buffer[24] = Some(false);
        dcf77.bit_buffer[25] = Some(false);
        dcf77.bit_buffer[27] = Some(false);
        dcf77.bit_buffer[29] = Some(true);
        dcf77.bit_buffer[35] = Some(false);
        // which will have a leap second:
        dcf77.bit_buffer[59] = Some(true); // which has value 1 instead of 0
        dcf77.second = 60; // 60 bits (61 seconds here, before decoding)

        dcf77.decode_time(true);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(0));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(17));
        assert_eq!(
            dcf77.radio_datetime.get_leap_second(),
            Some(radio_datetime_utils::LEAP_PROCESSED)
        );
        assert_eq!(dcf77.second, 60);
        assert_eq!(dcf77.get_this_minute_length(), 61);
        assert_eq!(dcf77.get_next_minute_length(), 60);
        assert_eq!(dcf77.get_leap_second_is_one(), Some(true));

        // next regular minute:
        dcf77.bit_buffer[19] = Some(false);
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(true);
        // dcf77.bit_buffer[59] remains Some() but is never touched again
        dcf77.second = 59;
        dcf77.decode_time(true);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(1));
        assert_eq!(dcf77.radio_datetime.get_leap_second(), Some(0));
        assert_eq!(dcf77.second, 59); // sanity check
        assert_eq!(dcf77.get_this_minute_length(), 60);
        assert_eq!(dcf77.get_next_minute_length(), 60);
    }
    #[test]
    fn continue_decode_time_complete_minute_dst_change_to_summer_strict() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 59;
        for b in 0..=58 {
            dcf77.bit_buffer[b] = Some(BIT_BUFFER[b]);
        }
        // flip to winter
        dcf77.bit_buffer[17] = Some(false);
        dcf77.bit_buffer[18] = Some(true);
        // DST change must be at top of hour and
        // announcements only count before the hour, so set minute to 59:
        dcf77.bit_buffer[21] = Some(true);
        dcf77.bit_buffer[28] = Some(false);
        // announce a DST change:
        dcf77.bit_buffer[16] = Some(true);
        dcf77.decode_time(true);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(59));
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_ANNOUNCED)
        );
        // next minute and hour:
        dcf77.bit_buffer[21] = Some(false);
        dcf77.bit_buffer[24] = Some(false);
        dcf77.bit_buffer[25] = Some(false);
        dcf77.bit_buffer[27] = Some(false);
        dcf77.bit_buffer[29] = Some(true);
        dcf77.bit_buffer[35] = Some(false);
        // which will have a DST change:
        dcf77.bit_buffer[17] = Some(true);
        dcf77.bit_buffer[18] = Some(false);
        dcf77.decode_time(true);
        assert_eq!(dcf77.radio_datetime.get_minute(), Some(0));
        assert_eq!(dcf77.radio_datetime.get_hour(), Some(17));
        assert_eq!(
            dcf77.radio_datetime.get_dst(),
            Some(radio_datetime_utils::DST_PROCESSED | radio_datetime_utils::DST_SUMMER)
        ); // DST flipped on
    }

    #[test]
    fn test_increase_second_same_minute_ok() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 37;
        // all date/time values are None
        assert_eq!(dcf77.increase_second(), true);
        assert_eq!(dcf77.first_minute, true);
        assert_eq!(dcf77.second, 38);
    }
    #[test]
    fn test_increase_second_partial_new_minute_ok() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.new_minute = true;
        dcf77.second = 37;
        // all date/time values are None
        assert_eq!(dcf77.increase_second(), true);
        assert_eq!(dcf77.first_minute, true);
        assert_eq!(dcf77.second, 0);
    }
    #[test]
    fn test_increase_second_same_minute_overflow() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.second = 59;
        // leap second value is None
        assert_eq!(dcf77.increase_second(), false);
        assert_eq!(dcf77.first_minute, true);
        assert_eq!(dcf77.second, 0);
    }
    #[test]
    fn test_increase_second_new_minute_ok() {
        let mut dcf77 = DCF77Utils::new(DecodeType::LogFile);
        dcf77.new_minute = true;
        dcf77.second = 59;
        // leap second value is None
        assert_eq!(dcf77.increase_second(), true);
        assert_eq!(dcf77.first_minute, true);
        assert_eq!(dcf77.second, 0);
    }
}
