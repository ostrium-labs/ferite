//! The pin and timing engine: the hardware the competition is actually about.
//!
//! # What this is
//!
//! The competition brief asks for *"a tiny CPU with an instruction set designed
//! for reading pins, writing pins, counting cycles, and hitting timing precisely
//! enough that you can implement a real protocol in firmware rather than in
//! fixed logic."*
//!
//! Four capabilities. Three are the CPU's problem and one is hardware's:
//! **reading a pin reliably** is the one that cannot be done in software,
//! because the CPU cannot sample a line faster than it can execute
//! instructions, and a line that changes in the middle of a read is a line
//! that is read wrong.
//!
//! So the engine is a **sampler with a debounce**, and everything else in the
//! brief is a register the firmware writes:
//!
//! - **writing pins** — a drive value and an output enable per pin,
//! - **counting cycles** — a free-running counter,
//! - **hitting timing precisely** — a programmable sample period, so a line is
//!   sampled every N cycles rather than every cycle, and a firmware loop can
//!   then count bit times in whole samples.
//!
//! The debounce is the part worth having. A UART RX line is sampled eight or
//! sixteen times per bit and the majority wins; a pin that is *only* sampled at
//! the bit rate cannot be read at all, because the sample lands at an arbitrary
//! phase. So the engine samples at a programmable rate and requires a
//! programmable number of disagreeing samples before it believes the line
//! changed.
//!
//! # The interface is PicoRV32's memory bus
//!
//! The registers are memory-mapped, and the bus is the one PicoRV32 exposes:
//! `mem_valid`, `mem_addr`, `mem_wdata`, `mem_wstrb`, `mem_ready`, `mem_rdata`.
//! A peripheral that always asserts `mem_ready` is a zero-wait-state
//! peripheral, which is the simplest thing that works and the one the core is
//! designed around.
//!
//! The engine is therefore usable **without forking the core**: the custom
//! instructions the plan describes become loads and stores to these registers,
//! and the only thing lost is the atomicity of a read-after-delay, which the
//! engine provides by keeping the sampled value in a register the CPU polls.
//!
//! # Why this is not a corpus design
//!
//! The corpus exists to find defects in the *tools*, with algorithm designs
//! whose behaviour is defined by somebody else's specification. This is the
//! chip's own logic, and it is checked against a reference model written
//! separately — the same discipline, a different reason.

pub mod model;

use ferrite_lithic::{Design, Signal};

/// How many pins the engine has.
///
/// Eight is the number of bidirectional pads the Tiny Tapeout die has, so the
/// engine's pin vector is exactly as wide as the pad budget.
pub const PINS: u32 = 8;

/// The register map, in bytes.
///
/// The offsets are PicoRV32's byte addresses. Only the low bits are decoded,
/// so the peripheral sits at the bottom of the address space and the core can
/// be told to place it there.
pub mod reg {
    /// The value driven on each pin. One bit per pin. Write.
    pub const DRIVE: u32 = 0x00;
    /// One bit per pin: 1 drives the pin, 0 releases it. Write.
    ///
    /// Releasing is not the same as driving zero, and the difference is the
    /// whole of I2C. A released pad goes back to being an input, so an
    /// external pull-up takes the line high and a slave can pull it low.
    pub const ENABLE: u32 = 0x04;
    /// The number of cycles between samples. Write.
    pub const PERIOD: u32 = 0x08;
    /// How many disagreeing samples are needed before a change is believed.
    /// Write.
    pub const THRESH: u32 = 0x0C;
    /// The pin's level as of the last clock edge. Read.
    pub const LEVEL: u32 = 0x10;
    /// The debounced pin level. Read.
    pub const SAMPLED: u32 = 0x14;
    /// One bit per pin: the debounced level changed. Write one to clear.
    pub const EDGE: u32 = 0x18;
    /// The free-running cycle counter. Read.
    pub const CYCLE: u32 = 0x1C;
}

/// How wide the sample period is, in bits.
///
/// Sixteen, so a sample can be taken every 65 536 cycles, which is longer than
/// any protocol's bit time by a wide margin.
pub const PERIOD_BITS: u32 = 16;

/// How wide the debounce threshold is, in bits.
///
/// Eight, so a line has to disagree for up to 255 samples before it is
/// believed to have changed.
pub const THRESH_BITS: u32 = 8;

/// The engine's ports.
#[derive(Clone, Debug)]
pub struct Ports {
    /// The bus is ready. Always asserted: the engine answers in one cycle.
    pub mem_ready: Signal,
    /// The read data.
    pub mem_rdata: Signal,
    /// The value to drive on each pin.
    pub drive: Signal,
    /// One bit per pin: 1 drives the pin, 0 releases it.
    pub enable: Signal,
    /// The free-running cycle counter.
    pub cycle: Signal,
    /// The debounced level, one bit per pin.
    ///
    /// Exposed on a port rather than only through the bus, for two reasons. A
    /// read through the bus costs a clock edge, and the engine samples the pin
    /// on every edge — so a test that read the debounced level through the bus
    /// would perturb the thing it was reading. And on a chip, the sampled level
    /// is what you want to see on a bench scope while a protocol runs.
    pub sampled: Signal,
    /// The edge flags, one bit per pin.
    pub edge: Signal,
}

/// The engine's design, built on `design`.
///
/// The inputs it expects are named, and the names are PicoRV32's:
///
/// | Name | Width | Direction |
/// |---|---:|---|
/// | `clk` | 1 | in |
/// | `rst` | 1 | in |
/// | `mem_valid` | 1 | in |
/// | `mem_addr` | 32 | in |
/// | `mem_wdata` | 32 | in |
/// | `mem_wstrb` | 4 | in |
/// | `pin_level` | 8 | in |
///
/// and the outputs are `mem_ready`, `mem_rdata`, `drive`, `enable` and `cycle`.
///
/// # Errors
///
/// Whatever [`Design`] returns: a width mismatch, a duplicate name, or a
/// signal from another design.
pub fn build(design: &Design) -> Result<Ports, ferrite_lithic::Error> {
    let clk = design.input("clk", 1)?;
    let rst = design.input("rst", 1)?;
    let mem_valid = design.input("mem_valid", 1)?;
    let mem_addr = design.input("mem_addr", 32)?;
    let mem_wdata = design.input("mem_wdata", 32)?;
    let mem_wstrb = design.input("mem_wstrb", 4)?;
    let pin_level = design.input("pin_level", PINS)?;

    // The register index is the address itself, decoded over a sixteen-bit
    // window. The window matters: a narrower one would alias, and an address
    // the peripheral does not own would land on a register it does. PicoRV32
    // passes the address through and lets the peripheral decode it, so the
    // peripheral has to be told how much of the address is *its*.
    let word = design.slice(&mem_addr, 0, 16)?;

    // A write happens when the bus is valid, some byte strobe is set, and the
    // address names a register this one owns.
    let strobe = |index: u32| -> Result<Signal, ferrite_lithic::Error> {
        design.slice(&mem_wstrb, index, 1)
    };
    let any_strobe = design.or(&strobe(0)?, &strobe(1)?)?;
    let any_strobe = design.or(&any_strobe, &strobe(2)?)?;
    let any_strobe = design.or(&any_strobe, &strobe(3)?)?;
    let writing = design.and(&mem_valid, &any_strobe)?;

    // ---- The writable registers.
    let drive = write_reg(
        design,
        &word,
        &writing,
        &mem_wdata,
        reg::DRIVE,
        PINS,
        &clk,
        &rst,
    )?;
    let enable = write_reg(
        design,
        &word,
        &writing,
        &mem_wdata,
        reg::ENABLE,
        PINS,
        &clk,
        &rst,
    )?;
    let period = write_reg(
        design,
        &word,
        &writing,
        &mem_wdata,
        reg::PERIOD,
        PERIOD_BITS,
        &clk,
        &rst,
    )?;
    let thresh = write_reg(
        design,
        &word,
        &writing,
        &mem_wdata,
        reg::THRESH,
        THRESH_BITS,
        &clk,
        &rst,
    )?;

    // ---- The free-running cycle counter.
    //
    // Thirty-two bits, and it is the clock the whole engine is timed against.
    // It is not resettable: a counter that resets is a counter whose zero is
    // arbitrary, and firmware that wants a time base wants one that only moves
    // forward.
    let cycle_wire = design.wire(32)?;
    let one = design.lit(1, 32)?;
    let cycle_next = design.add(&cycle_wire, &one)?;
    let cycle = design.reg(&cycle_next, &clk, &rst, &design.constant(false))?;
    design.drive(&cycle_wire, &cycle)?;

    // ---- The sample countdown.
    //
    // A sample is due when the counter is zero, and the counter reloads with
    // `PERIOD - 1`. A period of one therefore samples every cycle, which is
    // the case a firmware wants when it is counting bit times itself.
    //
    // The period is a *count* rather than a log2 divider on purpose: a
    // programmable divider would need a variable shift, and the DSL has no
    // variable shift — a five-bit log2 divider is a thirty-one-mux barrel
    // shifter, which is a real cost for a feature a counter gives for free.
    let zero16 = design.lit(0, PERIOD_BITS)?;
    let reload = design.sub(&period, &design.lit(1, PERIOD_BITS)?)?;
    let reload = design.ite(&design.eq(&period, &zero16)?, &zero16, &reload)?;
    let cnt = design.wire(PERIOD_BITS)?;
    let due = design.eq(&cnt, &zero16)?;
    let cnt_next = design.ite(
        &due,
        &reload,
        &design.sub(&cnt, &design.lit(1, PERIOD_BITS)?)?,
    )?;
    let cnt_reg = design.reg(&cnt_next, &clk, &rst, &design.constant(false))?;
    design.drive(&cnt, &cnt_reg)?;

    // ---- The pin level as of the last edge.
    let level = design.reg(&pin_level, &clk, &rst, &design.constant(false))?;

    // ---- The debouncer.
    let (sampled, edge) = debounce(
        design, &level, &due, &thresh, &word, &writing, &mem_wdata, &clk, &rst,
    )?;

    // ---- The read path.
    let rdata = read_mux(
        design, &word, &drive, &enable, &period, &thresh, &level, &sampled, &edge, &cycle,
    )?;

    let mem_ready = design.constant(true);
    design.output("mem_ready", 1, &mem_ready)?;
    design.output("mem_rdata", 32, &rdata)?;
    design.output("drive", PINS, &drive)?;
    design.output("enable", PINS, &enable)?;
    design.output("cycle", 32, &cycle)?;
    design.output("sampled", PINS, &sampled)?;
    design.output("edge", PINS, &edge)?;

    Ok(Ports {
        mem_ready,
        mem_rdata: rdata,
        drive,
        enable,
        cycle,
        sampled,
        edge,
    })
}

/// A register written when the bus names it.
///
/// The register is held when the bus names a *different* register, which is the
/// `clear` input of a `Reg`: the register's own "hold" is the clock domain's
/// business and the bus's business is a different signal, and a `Reg` can only
/// take one of them.
#[allow(clippy::too_many_arguments)]
fn write_reg(
    design: &Design,
    word: &Signal,
    writing: &Signal,
    wdata: &Signal,
    offset: u32,
    width: u32,
    clk: &Signal,
    rst: &Signal,
) -> Result<Signal, ferrite_lithic::Error> {
    let hit = design.eq(word, &design.lit(u64::from(offset), 16)?)?;
    let enable = design.and(writing, &hit)?;
    let data = design.slice(wdata, 0, width)?;
    // The register's *clear* input holds rather than clears, so the hold is the
    // **negation** of the write enable: the register takes the written value
    // when the bus named it, and keeps what it had when the bus did not.
    //
    // Getting this backwards is the kind of mistake the reference model exists
    // to catch, and it caught it: the register was taking the written value
    // exactly when the bus did *not* name it.
    design.reg(&data, clk, rst, &design.not(&enable))
}

/// The debouncer: a sampled level and the edge flags, per pin.
///
/// The rule is the one a debounce is supposed to have: **a line is believed to
/// have changed after N consecutive samples that disagree with it.** A sample
/// that agrees resets the count, so a line that glitches for one sample out of
/// a hundred does not move the debounced level at all.
///
/// The edge flag is **sticky** and cleared by writing a one to it, rather than
/// being a one-cycle pulse. A pulse can be missed by firmware that was busy
/// when it happened, and a flag that is missed is a flag that does not exist.
/// Write-one-to-clear rather than write-zero-to-clear, because a firmware that
/// writes the whole register for another reason must not lose an edge it had
/// not seen.
#[allow(clippy::too_many_arguments)]
fn debounce(
    design: &Design,
    level: &Signal,
    due: &Signal,
    thresh: &Signal,
    word: &Signal,
    writing: &Signal,
    wdata: &Signal,
    clk: &Signal,
    rst: &Signal,
) -> Result<(Signal, Signal), ferrite_lithic::Error> {
    let zero8 = design.lit(0, THRESH_BITS)?;
    let mut sampled_bits = Vec::with_capacity(PINS as usize);
    let mut edge_bits = Vec::with_capacity(PINS as usize);

    for pin in 0..PINS {
        let level_bit = design.slice(level, pin, 1)?;

        // The debounced level, as a wire, so that the next value can read the
        // current one. A register cannot be its own input, and the debounce
        // compares the level against the *debounced* level rather than against
        // the raw one — so the two are different registers and only one of them
        // is read back here.
        let stable = design.wire(1)?;
        let disagree = design.not(&design.eq(&level_bit, &stable)?);

        // How many consecutive samples have disagreed with the debounced level.
        let agree = design.wire(THRESH_BITS)?;
        let count_up = design.add(&agree, &design.lit(1, THRESH_BITS)?)?;
        let agree_next = design.ite(&disagree, &count_up, &zero8)?;
        // The count only moves on a sample: between samples the line is not
        // being looked at, and counting then would defeat the divider.
        let agree_next = design.ite(due, &agree_next, &agree)?;
        let agree_reg = design.reg(&agree_next, clk, rst, &design.constant(false))?;
        design.drive(&agree, &agree_reg)?;

        // The line is believed to have changed when enough samples disagreed.
        // The threshold is unsigned and the count is too, so
        // `count >= threshold` is `not(count < threshold)` — and a threshold of
        // zero then means "one disagreeing sample", which is what a firmware
        // that leaves the register at zero expects.
        let enough = design.not(&design.ult(&agree_next, thresh)?);
        let flip = design.and(due, &disagree)?;
        let flip = design.and(&flip, &enough)?;

        let sampled = design.ite(&flip, &level_bit, &stable)?;
        let stable_reg = design.reg(&sampled, clk, rst, &design.constant(false))?;
        design.drive(&stable, &stable_reg)?;

        // The sticky edge flag, also as a wire, because the flag's next value
        // depends on the flag's current value.
        let edge = design.wire(1)?;
        let clear = clear_bit(design, word, writing, wdata, reg::EDGE, pin)?;
        let held = design.and(&edge, &design.not(&clear))?;
        let edge_next = design.or(&flip, &held)?;
        let edge_reg = design.reg(&edge_next, clk, rst, &design.constant(false))?;
        design.drive(&edge, &edge_reg)?;

        sampled_bits.push(stable_reg);
        edge_bits.push(edge_reg);
    }

    // `concat` puts the first part in the *most significant* bits, so the
    // vectors are reversed before they are joined: pin 0 has to end up in the
    // least significant bit, or the whole pin map is upside down.
    sampled_bits.reverse();
    edge_bits.reverse();
    Ok((design.concat(&sampled_bits)?, design.concat(&edge_bits)?))
}

/// Whether a write clears one bit of the edge register.
fn clear_bit(
    design: &Design,
    word: &Signal,
    writing: &Signal,
    wdata: &Signal,
    offset: u32,
    pin: u32,
) -> Result<Signal, ferrite_lithic::Error> {
    let hit = design.eq(word, &design.lit(u64::from(offset), 16)?)?;
    let writing = design.and(writing, &hit)?;
    let bit = design.slice(wdata, pin, 1)?;
    design.and(&writing, &bit)
}

/// The read multiplexer.
///
/// Every register is a word, and the eight word offsets are decoded. The
/// default is zero, so an address the peripheral does not answer reads as zero
/// rather than as whatever the last register held.
#[allow(clippy::too_many_arguments)]
fn read_mux(
    design: &Design,
    word: &Signal,
    drive: &Signal,
    enable: &Signal,
    period: &Signal,
    thresh: &Signal,
    level: &Signal,
    sampled: &Signal,
    edge: &Signal,
    cycle: &Signal,
) -> Result<Signal, ferrite_lithic::Error> {
    let widen = |value: &Signal, _width: u32| -> Result<Signal, ferrite_lithic::Error> {
        if value.width() == 32 {
            Ok(value.clone())
        } else {
            design.zero_extend(value, 32)
        }
    };
    let arms = vec![
        (u64::from(reg::DRIVE), widen(drive, PINS)?),
        (u64::from(reg::ENABLE), widen(enable, PINS)?),
        (u64::from(reg::PERIOD), widen(period, PERIOD_BITS)?),
        (u64::from(reg::THRESH), widen(thresh, THRESH_BITS)?),
        (u64::from(reg::LEVEL), widen(level, PINS)?),
        (u64::from(reg::SAMPLED), widen(sampled, PINS)?),
        (u64::from(reg::EDGE), widen(edge, PINS)?),
        (u64::from(reg::CYCLE), cycle.clone()),
    ];
    design.case_(word, &arms, &design.zeros(32)?)
}
