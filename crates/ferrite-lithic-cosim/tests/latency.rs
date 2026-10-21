//! A memory with a read latency, in both backends, compared.
//!
//! This is the test that matters for the whole latency mechanism. The
//! simulator models a synchronous read as a register fed by the array read,
//! and the emitter builds `always @(posedge clk) q <= en ? mem[addr] : q;`.
//! If those two disagree by a cycle — or disagree about what a *disabled*
//! read does — then the circuit that was simulated is not the circuit that
//! was fabricated, and every differential test that used the memory was
//! checking something else.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrite_lithic::Design;
use ferrite_lithic_cosim::{Harness, Plan, Port, Stimulus, run_with, verilator};
use ferrite_lithic_rtl::Module;
use ferrite_lithic_sim::Sim;

/// A design with a latency-bearing memory, a read port and a write port.
fn design(latency: u32) -> (Design, Module) {
    let design = Design::new();
    let clk = design.input("clk", 1).unwrap();
    let we = design.input("we", 1).unwrap();
    let re = design.input("re", 1).unwrap();
    let addr = design.input("addr", 4).unwrap();
    let wdata = design.input("wdata", 8).unwrap();

    let mem = design.mem_with(8, 16, None, latency).unwrap();
    design.write_port(&mem, &wdata, &addr, &we).unwrap();
    let read = design.read_port(&mem, &addr, &re).unwrap();
    design.output("q", 8, &read).unwrap();

    let module = Module::new(
        "dut",
        design.build().unwrap(),
        clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    (design, module)
}

#[test]
fn a_synchronous_read_agrees_between_the_two_backends() {
    let Some(verilator) = verilator() else {
        println!(
            "SKIPPED: verilator is not found, so the synchronous read was not compared \
             against the emitted Verilog. The simulator-only test below still runs."
        );
        return;
    };
    let _ = verilator;

    let (design, module) = design(1);
    let plan = plan_for(&design, &module);

    // Write 0xA5 to address 3, then read it back, then stop reading.
    //
    // The rows are (we, re, addr, wdata). A read of an address that was
    // written in the previous row must report the *old* value in the row the
    // read is issued, and the value in the row after it, because the capture
    // latched at the edge.
    let mut stimulus = Stimulus::new();
    // 0: idle
    stimulus.push(row(0, 0, 0, 0)).unwrap();
    // 1: write 0xA5 to address 3
    stimulus.push(row(1, 0, 3, 0xA5)).unwrap();
    // 2: read address 3
    stimulus.push(row(0, 1, 3, 0)).unwrap();
    // 3: read address 3 again
    stimulus.push(row(0, 1, 3, 0)).unwrap();
    // 4: no read; the value must hold rather than go to zero
    stimulus.push(row(0, 0, 0, 0)).unwrap();
    // 5: still no read
    stimulus.push(row(0, 0, 0, 0)).unwrap();

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree about a synchronous read:\n{report}\nfirst: {:?}",
        report.first()
    );
}

#[test]
fn an_array_read_agrees_between_the_two_backends() {
    if verilator().is_none() {
        println!(
            "SKIPPED: verilator is not found, so the array read was not compared against \
             the emitted Verilog."
        );
        return;
    }
    let (design, module) = design(0);
    let plan = plan_for(&design, &module);

    let mut stimulus = Stimulus::new();
    stimulus.push(row(0, 0, 0, 0)).unwrap();
    stimulus.push(row(1, 0, 3, 0xA5)).unwrap();
    stimulus.push(row(0, 1, 3, 0)).unwrap();
    stimulus.push(row(0, 1, 3, 0)).unwrap();
    stimulus.push(row(0, 0, 0, 0)).unwrap();

    let verilog = ferrite_lithic_cosim::emit(&design, &plan).unwrap();
    let harness = Harness::build(&plan, &verilog, &plan.work_dir()).unwrap();
    let report = run_with(&design, &plan, &stimulus, &harness).unwrap();
    assert!(
        report.is_equivalent(),
        "the two backends disagree about an array read:\n{report}\nfirst: {:?}",
        report.first()
    );
}

#[test]
fn a_synchronous_read_holds_its_value_when_the_read_is_disabled() {
    // The behaviour an array read does *not* have, and the reason the flag on
    // the simulator's read port exists: a disabled synchronous read keeps the
    // last value it captured, because a macro's output register holds.
    let (design, _) = design(1);
    // The clock is stated, because a design whose only state is a memory has
    // no register for the simulator to infer one from.
    let ports = design.input_ports();
    let clock = ports
        .iter()
        .find(|signal| signal.name().as_deref() == Some("clk"))
        .unwrap();
    let mut sim = Sim::new_with_clock(&design, Some(clock)).unwrap();

    sim.set_input("we", bit(true)).unwrap();
    sim.set_input("re", bit(false)).unwrap();
    sim.set_input("addr", bits(3, 4)).unwrap();
    sim.set_input("wdata", bits(0xA5, 8)).unwrap();
    sim.step().unwrap();

    sim.set_input("we", bit(false)).unwrap();
    sim.set_input("re", bit(true)).unwrap();
    sim.set_input("addr", bits(3, 4)).unwrap();
    let (_, after) = sim.step().unwrap();
    assert_eq!(
        after.get("q").unwrap().to_u64().unwrap(),
        0xA5,
        "the capture latched the word at the edge"
    );

    // Now stop reading. The value must hold.
    sim.set_input("re", bit(false)).unwrap();
    sim.set_input("addr", bits(0, 4)).unwrap();
    let (_, after) = sim.step().unwrap();
    assert_eq!(
        after.get("q").unwrap().to_u64().unwrap(),
        0xA5,
        "a disabled synchronous read holds the last value, which is what a hard \\
         SRAM macro's output register does"
    );
}

/// The stimulus plan: the clock, the four inputs and the one output.
fn plan_for(design: &Design, module: &Module) -> Plan {
    let _ = (design, module);
    Plan::new(
        "dut",
        Some("clk".to_string()),
        ["we", "re", "addr", "wdata"]
            .into_iter()
            .zip([1u32, 1, 4, 8])
            .map(|(name, width)| Port::new(name, width))
            .collect(),
        vec![Port::new("q", 8)],
    )
    .unwrap()
}

/// One stimulus row: (we, re, addr, wdata).
fn row(we: u8, re: u8, addr: u8, wdata: u8) -> Vec<ferrite_lithic_bits::Bits> {
    vec![bit(we != 0), bit(re != 0), bits(addr, 4), bits(wdata, 8)]
}

/// A one-bit literal.
fn bit(value: bool) -> ferrite_lithic_bits::Bits {
    ferrite_lithic_bits::Bits::constant(u64::from(value), 1).unwrap()
}

/// A literal of a given width.
fn bits(value: u8, width: u32) -> ferrite_lithic_bits::Bits {
    ferrite_lithic_bits::Bits::constant(u64::from(value), width).unwrap()
}
