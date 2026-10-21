//! A reference model of the pin and timing engine.
//!
//! # Why there are two implementations
//!
//! The engine's behaviour is specified here in prose, and the hardware in
//! [`super::build`]. They are separate because a design checked against a
//! description of itself proves very little: a mistake in the understanding
//! appears in both, and the two agree.
//!
//! This model is written from the specification and *not* from the design, and
//! the two are compared over random stimuli. That is the discipline the corpus
//! uses and the discipline the Jane Street puzzle results called out: *"models
//! that reproduced our sample waveform perfectly still contained mistakes.
//! Comparing separate implementations and constructing inputs that should fail
//! exposed errors that replaying the examples hadn't caught."*
//!
//! So the differential test here does two things: it compares the two
//! implementations over random inputs, and it includes **inputs that should
//! fail** — a threshold of zero, a period of zero, a write of zero to the
//! edge register, a write to an address the peripheral does not own.
//!
//! # The ordering
//!
//! Every register in the engine updates **at the same edge**. So the model
//! computes every next value from the current state and the current inputs,
//! and then commits them together. An implementation that updated the cycle
//! counter before computing the debounce would agree with the hardware on every
//! input and be wrong about the cycle the read data came from.

use crate::engine::{PERIOD_BITS, PINS, THRESH_BITS};

/// The bus, as the engine sees it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Bus {
    /// The reset is asserted.
    ///
    /// The engine's reset is synchronous and active high, and it wins over
    /// everything else: a write on a reset cycle does not land.
    pub rst: bool,
    /// The bus is valid.
    pub mem_valid: bool,
    /// The byte address.
    pub mem_addr: u32,
    /// The write data.
    pub mem_wdata: u32,
    /// The byte strobes.
    pub mem_wstrb: u8,
    /// The pin levels.
    pub pin_level: u8,
}

/// The register map, mirroring [`crate::engine::reg`].
pub mod reg {
    /// The value driven on each pin.
    pub const DRIVE: u32 = 0x00;
    /// The output enable.
    pub const ENABLE: u32 = 0x04;
    /// The sample period.
    pub const PERIOD: u32 = 0x08;
    /// The debounce threshold.
    pub const THRESH: u32 = 0x0C;
    /// The pin level.
    pub const LEVEL: u32 = 0x10;
    /// The debounced level.
    pub const SAMPLED: u32 = 0x14;
    /// The edge flags.
    pub const EDGE: u32 = 0x18;
    /// The cycle counter.
    pub const CYCLE: u32 = 0x1C;
}

/// The model's state.
///
/// Every field is a register in the hardware, plus the countdown and the
/// per-pin disagreement counts, which are internal.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Model {
    /// The value driven on each pin.
    pub drive: u8,
    /// The output enable.
    pub enable: u8,
    /// The sample period.
    pub period: u16,
    /// The debounce threshold.
    pub thresh: u8,
    /// The pin level as of the last edge.
    pub level: u8,
    /// The debounced level.
    pub sampled: u8,
    /// The sticky edge flags.
    pub edge: u8,
    /// The cycle counter.
    pub cycle: u32,
    /// The sample countdown.
    cnt: u16,
    /// How many consecutive samples have disagreed, per pin.
    agree: [u8; PINS as usize],
}

impl Model {
    /// A model in its reset state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The register a write lands in, if any.
    fn written_register(&self, bus: &Bus) -> Option<u32> {
        if !bus.mem_valid || bus.mem_wstrb == 0 {
            return None;
        }
        match bus.mem_addr {
            reg::DRIVE => Some(reg::DRIVE),
            reg::ENABLE => Some(reg::ENABLE),
            reg::PERIOD => Some(reg::PERIOD),
            reg::THRESH => Some(reg::THRESH),
            reg::EDGE => Some(reg::EDGE),
            _ => None,
        }
    }

    /// One clock edge.
    ///
    /// Every next value is computed from the current state and the current
    /// inputs, and then committed together — which is what the hardware does,
    /// because every one of these registers updates at the same edge.
    pub fn step(&mut self, bus: &Bus) {
        // A reset wins over everything, and every register in the engine resets
        // to zero — including the cycle counter, which is resettable so that
        // firmware can time from a known point.
        if bus.rst {
            *self = Model::new();
            return;
        }

        // ---- The writes.
        let written = self.written_register(bus);
        let clearing = written == Some(reg::EDGE);
        let drive_next = if written == Some(reg::DRIVE) {
            (bus.mem_wdata as u8) & pin_mask()
        } else {
            self.drive
        };
        let enable_next = if written == Some(reg::ENABLE) {
            (bus.mem_wdata as u8) & pin_mask()
        } else {
            self.enable
        };
        let period_next = if written == Some(reg::PERIOD) {
            (bus.mem_wdata as u16) & period_mask()
        } else {
            self.period
        };
        let thresh_next = if written == Some(reg::THRESH) {
            (bus.mem_wdata as u8) & thresh_mask()
        } else {
            self.thresh
        };

        // ---- The sample countdown.
        // The divider and the threshold are the values the registers *hold*,
        // not the values being written this cycle: the hardware's write lands
        // at the edge, after the sampling decision has already been made with
        // the old value.
        let due = self.cnt == 0;
        let reload = if self.period == 0 { 0 } else { self.period - 1 };
        let cnt_next = if due { reload } else { self.cnt - 1 };

        // ---- The debouncer.
        //
        // The comparison is against the level *as of the last edge*, not the
        // level being presented now: the hardware samples the pin into a
        // register and the debounce works on the registered value, so the
        // debounced level lags the pin by two cycles — one for the sample and
        // one for the debounce. The model has to lag the same way, or the two
        // implementations agree on the level and disagree on the cycle.
        let level_next = bus.pin_level & pin_mask();
        let level_now = self.level;
        let mut agree_next = self.agree;
        let mut sampled_next = self.sampled;
        let mut edge_next = self.edge;

        for (pin, agree) in self.agree.iter().enumerate() {
            let bit = 1u8 << pin;
            let level_bit = level_now & bit != 0;
            let stable_bit = self.sampled & bit != 0;
            let disagree = level_bit != stable_bit;

            let count = if due {
                if disagree { agree.saturating_add(1) } else { 0 }
            } else {
                *agree
            };
            agree_next[pin] = count;

            // "Enough" is `count >= threshold`, and a threshold of zero means
            // "one disagreeing sample", because a count of zero is reached on
            // the first disagreement.
            let enough = count >= self.thresh;
            let flip = due && disagree && enough;

            if flip {
                if level_bit {
                    sampled_next |= bit;
                } else {
                    sampled_next &= !bit;
                }
            }

            // The edge flag is sticky, and a write of a one clears it. A write
            // of a zero leaves it alone, so a firmware that writes the whole
            // register for another reason does not lose an edge.
            //
            // A flip wins over a clear in the same cycle: the edge is newer
            // than the acknowledgement, so clearing it would lose an edge that
            // happened at the same moment firmware was acknowledging the last
            // one. The hardware computes `flip | (edge & !clear)`, which is the
            // same rule.
            let cleared = clearing && (bus.mem_wdata & (bit as u32)) != 0;
            if flip {
                edge_next |= bit;
            } else if cleared {
                edge_next &= !bit;
            }
        }

        // ---- Commit.
        self.drive = drive_next;
        self.enable = enable_next;
        self.period = period_next;
        self.thresh = thresh_next;
        self.level = level_next;
        self.sampled = sampled_next;
        self.edge = edge_next;
        self.cycle = self.cycle.wrapping_add(1);
        self.cnt = cnt_next;
        self.agree = agree_next;
    }

    /// The read data for an address, from the current state.
    #[must_use]
    pub fn read(&self, addr: u32) -> u32 {
        match addr {
            reg::DRIVE => u32::from(self.drive),
            reg::ENABLE => u32::from(self.enable),
            reg::PERIOD => u32::from(self.period),
            reg::THRESH => u32::from(self.thresh),
            reg::LEVEL => u32::from(self.level),
            reg::SAMPLED => u32::from(self.sampled),
            reg::EDGE => u32::from(self.edge),
            reg::CYCLE => self.cycle,
            _ => 0,
        }
    }
}

/// The mask of the pin bits.
///
/// The shifts are written as `u128` and narrowed, because a shift by the width
/// of the type overflows and a mask of every bit is the right answer here.
fn pin_mask() -> u8 {
    ((1u128 << PINS) - 1) as u8
}

/// The mask of the period bits.
fn period_mask() -> u16 {
    ((1u128 << PERIOD_BITS) - 1) as u16
}

/// The mask of the threshold bits.
fn thresh_mask() -> u8 {
    ((1u128 << THRESH_BITS) - 1) as u8
}
