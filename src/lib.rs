//! DCF77 receiver for embedded platforms using e.g. a Canaduino V3 receiver.

#![no_std]

use radio_datetime_utils::{
    get_bcd_value, get_parity, time_diff, RadioDateTimeUtils, LEAP_PROCESSED,
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

    /// Return if this is the first minute that is decoded.
    pub fn get_first_minute(&self) -> bool {
        self.first_minute
    }

    /// Return if a new minute has arrived.
    pub fn get_new_minute(&self) -> bool {
        self.new_minute
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

    /// Determine the length of this minute in bits, tolerate None as leap second state.
    pub fn get_minute_length(&self) -> u8 {
        let leap_second = self.radio_datetime.get_leap_second();
        if let Some(s_leap_second) = leap_second {
            59 + ((s_leap_second & LEAP_PROCESSED) != 0) as u8
        } else {
            59
        }
    }

    /// Increase or reset `second` and clear `first_minute` when appropriate.
    pub fn increase_second(&mut self) {
        let minute_length = self.get_minute_length();
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
    pub fn decode_time(&mut self) {
        let mut added_minute = false;
        if !self.first_minute {
            added_minute = self.radio_datetime.add_minute();
        }
        let minute_length = self.get_minute_length();
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
