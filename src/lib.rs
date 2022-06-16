//! DCF77 receiver for embedded platforms using e.g. a Canaduino V3 receiver.

#![no_std]

use radio_datetime_utils::RadioDateTimeUtils;

/// Time in microseconds for a bit to be considered 1
const ACTIVE_LIMIT: u32 = 150_000;
/// Minimum amount of time in microseconds between two bits, mostly to deal with noise
const SECOND_LIMIT: u32 = 950_000;
/// Time in microseconds for the minute marker to be detected
const MINUTE_LIMIT: u32 = 1_500_000;
/// Signal is considered lost after this many microseconds
const PASSIVE_LIMIT: u32 = 2_500_000;

/// DCF77 decoder class
pub struct DCF77Utils {
    before_first_edge: bool,
    first_minute: bool,
    new_minute: bool,
    act_len: u32,
    sec_len: u32,
    split_second: bool,
    second: u8,
    bit_buffer: [Option<bool>; 60],
    radio_datetime: RadioDateTimeUtils,
    parity_1: Option<bool>,
    parity_2: Option<bool>,
    parity_3: Option<bool>,
    frame_counter: u8,
    ticks_per_second: u8,
    ind_time: bool,
    ind_bit: bool,
    ind_error: bool,
}

impl DCF77Utils {
    pub fn new(tps: u8) -> Self {
        Self {
            before_first_edge: true,
            first_minute: true,
            new_minute: false,
            act_len: 0,
            sec_len: 0,
            second: 0,
            split_second: false,
            bit_buffer: [None; 60],
            radio_datetime: RadioDateTimeUtils::new(7),
            parity_1: None,
            parity_2: None,
            parity_3: None,
            frame_counter: 0,
            ticks_per_second: tps,
            ind_time: true,
            ind_bit: false,
            ind_error: true,
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

    /// Get the second counter.
    pub fn get_second(&self) -> u8 {
        self.second
    }

    /// Get a copy of the date/time structure.
    pub fn get_radio_datetime(&self) -> RadioDateTimeUtils {
        self.radio_datetime
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

    /// Get the value of bit 0 (must always be 0).
    pub fn get_bit_0(&self) -> Option<bool> {
        self.bit_buffer[0]
    }

    /// Get the value of the transmitter call bit.
    pub fn get_call_bit(&self) -> Option<bool> {
        self.bit_buffer[15]
    }

    /// Get the value of bit 20 (must always be 1).
    pub fn get_bit_20(&self) -> Option<bool> {
        self.bit_buffer[20]
    }

    /// Get the frame-in-second counter.
    pub fn get_frame_counter(&self) -> u8 {
        self.frame_counter
    }

    /// Return if the time (i.e. new second or minute) indicator is active.
    pub fn get_ind_time(&self) -> bool {
        self.ind_time
    }

    /// Return if the currently received bit is a 1.
    pub fn get_ind_bit(&self) -> bool {
        self.ind_bit
    }

    /// Return if there was an error receiving this bit.
    pub fn get_ind_error(&self) -> bool {
        self.ind_error
    }

    /**
     * Determine the bit value if a new edge is received. indicates reception errors,
     * and checks if a new minute has started.
     *
     * # Arguments
     * * `is_low_edge` - indicates that the edge has gone from high to low (as opposed to
     *                   low-to-high).
     * * `t0` - time stamp of the previously received edge, in microseconds
     * * `t1` - time stamp of the currently received edge, in microseconds
     */
    pub fn handle_new_edge(&mut self, is_low_edge: bool, t0: u32, t1: u32) {
        if self.before_first_edge {
            self.before_first_edge = false;
            return;
        }
        let t_diff = radio_datetime_utils::time_diff(t0, t1);
        self.sec_len += t_diff;
        if is_low_edge {
            self.bit_buffer[self.second as usize] = Some(false);
            if self.frame_counter < 4 * self.ticks_per_second / 10 {
                // suppress noise in case a bit got split
                self.act_len += t_diff;
            }
            if self.act_len > ACTIVE_LIMIT {
                self.ind_bit = true;
                self.bit_buffer[self.second as usize] = Some(true);
                if self.act_len > 2 * ACTIVE_LIMIT {
                    self.ind_error = true;
                    self.bit_buffer[self.second as usize] = None;
                }
            }
        } else if self.sec_len > PASSIVE_LIMIT {
            self.ind_error = true;
            self.act_len = 0;
            self.sec_len = 0;
        } else if self.sec_len > SECOND_LIMIT {
            self.ind_time = true;
            self.new_minute = self.sec_len > MINUTE_LIMIT;
            self.act_len = 0;
            self.sec_len = 0;
            if !self.split_second {
                self.frame_counter = 0;
            }
            self.split_second = false;
        } else {
            self.split_second = true;
            // self.bit_buffer[self.second as usize] = None; // perhaps?
            self.ind_error = true;
        }
    }

    /// Determine the length of this minute in bits, tolerate None as leap second state.
    fn get_minute_length(&self) -> u8 {
        if self.radio_datetime.get_leap_second().is_none() {
            return 59;
        }
        59 + if (self.radio_datetime.get_leap_second().unwrap()
            & radio_datetime_utils::LEAP_PROCESSED)
            != 0
        {
            1
        } else {
            0
        }
    }

    /// Increase or reset `second` and clear `first_minute` when appropriate.
    pub fn increase_second(&mut self) {
        if self.new_minute {
            if self.first_minute
                && self.second == self.get_minute_length()
                && self.bit_buffer[0] == Some(false)
                && self.bit_buffer[20] == Some(true)
                && self.bit_buffer[17].is_some()
                && self.bit_buffer[18].is_some()
                && self.bit_buffer[17] != self.bit_buffer[18]
                && self.radio_datetime.get_year().is_none()
                && self.radio_datetime.get_month().is_none()
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
            // wrap in case we missed the minute marker to prevent index-out-of-range
            self.second += 1;
            if self.second == self.get_minute_length() + 1 {
                self.second = 0;
            }
        }
    }

    /// Update the frame counter and the status of the time, bit, and error indicators when a
    /// new timer tick arrives. Calculate the current date ane time upon a new minute.
    pub fn handle_new_timer_tick(&mut self) {
        if self.frame_counter == 0 {
            self.ind_time = true;
            self.ind_bit = false;
            self.ind_error = false;
            if self.new_minute {
                self.decode_time();
            }
        } else if (self.frame_counter == self.ticks_per_second / 10 && !self.new_minute)
            || (self.frame_counter == 7 * self.ticks_per_second / 10 && self.new_minute)
        {
            self.ind_time = false;
        }
        if self.frame_counter == self.ticks_per_second {
            self.frame_counter = 0;
        } else {
            self.frame_counter += 1;
        }
    }

    /// Decode the time broadcast during the last minute, tolerate bad DST status.
    fn decode_time(&mut self) {
        if !self.first_minute {
            self.radio_datetime.add_minute();
        }
        if self.second == self.get_minute_length() {
            let tmp0 = radio_datetime_utils::get_bcd_value(&self.bit_buffer, 21, 27);
            self.parity_1 = radio_datetime_utils::get_parity(&self.bit_buffer, 21, 27, 28);
            self.radio_datetime
                .set_minute(tmp0, self.parity_1 == Some(false), !self.first_minute);

            let tmp0 = radio_datetime_utils::get_bcd_value(&self.bit_buffer, 29, 34);
            self.parity_2 = radio_datetime_utils::get_parity(&self.bit_buffer, 29, 34, 35);
            self.radio_datetime
                .set_hour(tmp0, self.parity_2 == Some(false), !self.first_minute);

            self.parity_3 = radio_datetime_utils::get_parity(&self.bit_buffer, 36, 57, 58);

            let tmp0 = radio_datetime_utils::get_bcd_value(&self.bit_buffer, 42, 44);
            self.radio_datetime
                .set_weekday(tmp0, self.parity_3 == Some(false), !self.first_minute);

            let tmp0 = radio_datetime_utils::get_bcd_value(&self.bit_buffer, 45, 49);
            self.radio_datetime
                .set_month(tmp0, self.parity_3 == Some(false), !self.first_minute);

            let tmp0 = radio_datetime_utils::get_bcd_value(&self.bit_buffer, 50, 57);
            self.radio_datetime
                .set_year(tmp0, self.parity_3 == Some(false), !self.first_minute);

            let tmp0 = radio_datetime_utils::get_bcd_value(&self.bit_buffer, 36, 41);
            self.radio_datetime
                .set_day(tmp0, self.parity_3 == Some(false), !self.first_minute);
        }
    }
}
