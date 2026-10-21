#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The pin and timing engine, checked against a reference model.
//!
//! # The test that matters
//!
//! The hardware and the model are separate implementations of the same
//! specification, and they are compared over random stimuli. The interesting
//! part is the last third of the stimulus: **inputs that should fail**.
//!
//! The Jane Street puzzle results said it plainly about their own chip: *"models
//! that reproduced our sample waveform perfectly still contained mistakes.
//! Comparing separate implementations and constructing inputs that should fail
//! exposed errors that replaying the examples hadn't caught."*
//!
//! So this file does not only replay the happy path. It drives:
//!
//! - a **period of zero**, which the model and the hardware both have to treat
//!   as one sample per cycle rather than as a divide-by-zero,
//! - a **threshold of zero**, which has to mean "one disagreeing sample",
//! - a **write of zero to the edge register**, which must *not* clear the
//!   flags — only a write of a one does,
//! - **writes to addresses the peripheral does not own**, which must leave
//!   every register alone,
//! - a **byte strobe of zero**, which is not a write at all,
//! - and **a line that disagrees for exactly one sample and then agrees
//!   again**, which must not move the debounced level.
//!
//! # One edge per cycle
//!
//! The testbench's `drive` takes a clock edge, so driving five inputs with five
//! `drive` calls is five cycles. Every input here is therefore *set* and the
//! edge is taken once, by `step`. Getting this wrong is not a small mistake: it
//! makes the hardware and the model disagree about which cycle they are on, and
//! the disagreement looks like a design bug.

use ferrite_lithic::Design;
use ferrite_lithic_chip::engine::{
    self,
    model::{self, Bus, Model},
    reg,
};
use ferrite_lithic_tb::{Handle, Testbench};
use proptest::prelude::*;

/// A design with the engine on it.
fn design() -> Design {
    let design = Design::new();
    engine::build(&design).unwrap();
    design
}

/// One clock edge, with the bus driven.
///
/// The inputs are *set* and the edge is taken once, so one call is one cycle.
async fn bus_cycle(tb: &Handle, valid: bool, addr: u32, wdata: u32, wstrb: u8, level: u8) {
    tb.set("rst", 0).await;
    tb.set("mem_valid", u64::from(valid)).await;
    tb.set("mem_addr", u64::from(addr)).await;
    tb.set("mem_wdata", u64::from(wdata)).await;
    tb.set("mem_wstrb", u64::from(wstrb)).await;
    tb.set("pin_level", u64::from(level)).await;
    tb.step().await;
}

/// The reset edge, with every input quiet.
async fn reset(tb: &Handle) {
    tb.set("rst", 1).await;
    tb.set("mem_valid", 0).await;
    tb.set("mem_addr", 0).await;
    tb.set("mem_wdata", 0).await;
    tb.set("mem_wstrb", 0).await;
    tb.set("pin_level", 0).await;
    tb.step().await;
}

/// The state the test compares: the read data, the drive and enable registers,
/// the cycle counter, and the bus's ready signal.
type State = (u64, u64, u64, u64, u64);

/// Reads a register through the bus.
///
/// A read costs an edge, because a read is a bus transaction and a bus
/// transaction happens on a clock edge. The engine's own state does not depend
/// on the address, so the read is transparent apart from the cycle counter
/// advancing — which the model does too.
async fn read_reg(tb: &Handle, offset: u32) -> u64 {
    tb.set("rst", 0).await;
    tb.set("mem_valid", 1).await;
    tb.set("mem_addr", u64::from(offset)).await;
    tb.set("mem_wstrb", 0).await;
    tb.step().await;
    tb.value("mem_rdata")
}

/// Runs the hardware over a sequence of bus transactions.
fn hardware(cycles: &[(bool, u32, u32, u8, u8)]) -> Vec<State> {
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    let mut out = Vec::with_capacity(cycles.len());
    tb.run(|tb| async move {
        reset(&tb).await;
        for (valid, addr, wdata, wstrb, level) in cycles {
            bus_cycle(&tb, *valid, *addr, *wdata, *wstrb, *level).await;
            out.push((
                tb.value("mem_rdata"),
                tb.value("drive"),
                tb.value("enable"),
                tb.value("cycle"),
                tb.value("mem_ready"),
            ));
        }
        out
    })
    .expect("the engine drives only the ports it declares")
}

/// Runs the model over the same sequence.
fn reference(cycles: &[(bool, u32, u32, u8, u8)]) -> Vec<State> {
    let mut model = Model::new();
    // The reset edge, which the hardware takes before the sequence starts.
    model.step(&Bus {
        rst: true,
        ..Bus::default()
    });
    let mut out = Vec::with_capacity(cycles.len());
    for (valid, addr, wdata, wstrb, level) in cycles {
        let bus = model::Bus {
            rst: false,
            mem_valid: *valid,
            mem_addr: *addr,
            mem_wdata: *wdata,
            mem_wstrb: *wstrb,
            pin_level: *level,
        };
        model.step(&bus);
        // The read data is combinational, so it reflects the state *after* the
        // edge the hardware's outputs are sampled at.
        out.push((
            u64::from(model.read(*addr)),
            u64::from(model.drive),
            u64::from(model.enable),
            u64::from(model.cycle),
            1,
        ));
    }
    out
}

/// Compares the hardware and the model over a sequence.
fn compare(cycles: &[(bool, u32, u32, u8, u8)]) {
    let hardware = hardware(cycles);
    let reference = reference(cycles);
    assert_eq!(
        hardware.len(),
        reference.len(),
        "the two implementations ran a different number of cycles"
    );
    for (index, (got, want)) in hardware.iter().zip(&reference).enumerate() {
        assert_eq!(
            got, want,
            "cycle {index}: the hardware and the reference model disagree\n  \
             cycle: {:?}\n  hardware: {got:?}\n  model:    {want:?}",
            cycles[index]
        );
    }
}

/// The registers, read back one at a time.
///
/// The engine's read path is a multiplexer, and a multiplexer with a wrong
/// address decode is a design that answers the wrong question — so every
/// register is asked for by name.
#[test]
fn every_register_answers() {
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        reset(&tb).await;

        for (offset, expected) in [
            (reg::DRIVE, 0u64),
            (reg::ENABLE, 0),
            (reg::PERIOD, 0),
            (reg::THRESH, 0),
            (reg::LEVEL, 0),
            (reg::SAMPLED, 0),
            (reg::EDGE, 0),
            // The cycle counter reads one more than the number of edges before
            // it, because the read itself is an edge: the reset edge, then one
            // edge per register read.
            (reg::CYCLE, 8),
        ] {
            assert_eq!(
                read_reg(&tb, offset).await,
                expected,
                "register at {offset:#x} answered wrong"
            );
        }

        // An address the peripheral does not own reads as zero.
        assert_eq!(
            read_reg(&tb, 0x40).await,
            0,
            "an unknown address must read as zero"
        );
    })
    .expect("the engine drives only the ports it declares");
}

/// A write to a register is read back.
#[test]
fn a_write_is_read_back() {
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        reset(&tb).await;
        bus_cycle(&tb, true, reg::DRIVE, 0b1010_0101, 0b1111, 0).await;
        assert_eq!(
            read_reg(&tb, reg::DRIVE).await,
            0b1010_0101,
            "the drive register did not take the written value"
        );
    })
    .expect("the engine drives only the ports it declares");
}

/// A line that disagrees for one sample and then agrees again does not move
/// the debounced level.
///
/// This is the case a debounce exists for, and it is the one a "replay the
/// example" test would miss: the line is high for one sample in the middle of
/// a long low stretch, and a design that looked at the raw level would report
/// an edge that is not there.
#[test]
fn a_single_disagreement_is_not_an_edge() {
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        // Sample every cycle, and believe a change after two disagreements.
        bus_cycle(&tb, true, reg::PERIOD, 1, 0b1111, 0).await;
        bus_cycle(&tb, true, reg::THRESH, 2, 0b1111, 0).await;

        // A long low stretch.
        for _ in 0..8 {
            bus_cycle(&tb, false, 0, 0, 0, 0).await;
        }
        assert_eq!(tb.value("sampled"), 0, "the line is low");
        assert_eq!(tb.value("edge"), 0, "and there was no edge");

        // One sample high.
        bus_cycle(&tb, false, 0, 0, 0, 1).await;
        assert_eq!(
            tb.value("sampled"),
            0,
            "one disagreeing sample must not move the debounced level"
        );
        assert_eq!(tb.value("edge"), 0, "and must not raise an edge");

        // And back to low.
        bus_cycle(&tb, false, 0, 0, 0, 0).await;
        assert_eq!(tb.value("sampled"), 0, "the line is still low");
        assert_eq!(tb.value("edge"), 0, "and there is still no edge");
    })
    .expect("the engine drives only the ports it declares");
}

/// The edge flag is sticky, and a write of a one clears it while a write of a
/// zero does not.
#[test]
fn the_edge_flag_is_sticky_and_clears_on_a_one() {
    let design = design();
    let tb = Testbench::new(&design).unwrap();
    tb.run(|tb| async move {
        // Believe a change after one disagreement.
        bus_cycle(&tb, true, reg::THRESH, 1, 0b1111, 0).await;

        // The line goes high and stays there, so the edge is raised.
        bus_cycle(&tb, false, 0, 0, 0, 1).await;
        bus_cycle(&tb, false, 0, 0, 0, 1).await;
        assert_eq!(tb.value("sampled"), 1, "the line is high");
        assert_eq!(tb.value("edge"), 1, "and the edge was raised");

        // A write of zero does not clear it.
        bus_cycle(&tb, true, reg::EDGE, 0, 0b1111, 1).await;
        assert_eq!(
            tb.value("edge"),
            1,
            "a write of zero must not clear an edge that firmware has not seen"
        );

        // A write of a one does.
        bus_cycle(&tb, true, reg::EDGE, 1, 0b1111, 1).await;
        assert_eq!(tb.value("edge"), 0, "a write of one clears the edge");
    })
    .expect("the engine drives only the ports it declares");
}

// The hardware and the model agree over random stimuli.
proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Random bus traffic, compared register by register.
    #[test]
    fn the_engine_agrees_with_its_reference_model(
        cycles in prop::collection::vec(
            (
                any::<bool>(),
                0u32..64,
                any::<u32>(),
                0u8..16,
                any::<u8>(),
            ),
            1..40,
        )
    ) {
        compare(&cycles);
    }
}

/// The inputs that should fail.
#[test]
fn the_awkward_inputs_are_handled() {
    // A period of zero, a threshold of zero, a write of zero to the edge
    // register, a write to an address the peripheral does not own, and a byte
    // strobe of zero.
    let cases: Vec<(bool, u32, u32, u8, u8)> = vec![
        // A period of zero: one sample per cycle, not a divide by zero.
        (true, reg::PERIOD, 0, 0b1111, 0),
        (true, reg::PERIOD, 0, 0b1111, 1),
        (false, 0, 0, 0, 1),
        (true, reg::THRESH, 0, 0b1111, 0),
        (true, reg::THRESH, 0, 0b1111, 1),
        (false, 0, 0, 0, 1),
        // A write of zero to the edge register: not a clear.
        (true, reg::EDGE, 0, 0b1111, 0),
        (false, 0, 0, 0, 0),
        // An address the peripheral does not own: no register moves.
        (true, 0x40, 0xdead_beef, 0b1111, 0),
        (true, 0x1000, 0xdead_beef, 0b1111, 0),
        // A byte strobe of zero is not a write at all.
        (true, reg::DRIVE, 0xff, 0, 0),
        (true, reg::DRIVE, 0xff, 0b0001, 0),
        (false, 0, 0, 0, 0),
    ];
    compare(&cases);
}
