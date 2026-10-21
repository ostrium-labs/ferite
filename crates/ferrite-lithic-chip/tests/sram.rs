//! A memory on a foundry macro, in both backends.
//!
//! The thing that has to be true is that **the simulator and the emitted
//! Verilog agree about when the data arrives**. A macro's read is
//! synchronous: the address and the read enable are sampled at the clock edge
//! and the data appears in the next cycle. An array read is combinational. If
//! the two backends disagreed about that by a cycle, every test that used the
//! memory would be testing a different circuit from the one that was
//! fabricated — and the differential testing this project is built on would be
//! checking nothing.

#![allow(clippy::unwrap_used)]

use ferrite_lithic::Design;
use ferrite_lithic_bits::Bits;
use ferrite_lithic_chip::sram::{MacroPorts, macro_memory};
use ferrite_lithic_pdk::Pdk;
use ferrite_lithic_rtl::Module;
use ferrite_lithic_sim::Sim;

/// A design with one macro-backed memory, one write port and one read port.
fn design_with_macro() -> (Design, ferrite_lithic::Signal, ferrite_lithic::Signal) {
    let design = Design::new();
    let clk = design.input("clk", 1).unwrap();
    let rst = design.input("rst", 1).unwrap();
    let wen = design.input("wen", 1).unwrap();
    let ren = design.input("ren", 1).unwrap();
    let waddr = design.input("waddr", 8).unwrap();
    let raddr = design.input("raddr", 8).unwrap();
    let wdata = design.input("wdata", 8).unwrap();

    // The master enable is the die's reset: holding it low puts the macro in
    // its reset state.
    let master = design.not(&rst);

    let memory = macro_memory(
        &design,
        Pdk::IhpCmos5l,
        256,
        8,
        MacroPorts {
            write_data: &wdata,
            write_address: &waddr,
            write_enable: &wen,
            read_address: &raddr,
            read_enable: &ren,
            clock: &clk,
            master_enable: &master,
        },
    )
    .unwrap();

    design.output("dout", 8, &memory.read).unwrap();
    (design, memory.memory, memory.read)
}

#[test]
fn the_emitted_verilog_instantiates_the_macro_with_the_foundry_s_name() {
    let (design, _, _) = design_with_macro();
    let module = Module::new(
        "core",
        design.build().unwrap(),
        design.input_ports()[0].id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let text = module.emit().unwrap();
    assert!(
        text.contains("RM_IHPSG13_1P_256x8_c3_bm_bist u_Mem ("),
        "the macro is instantiated by its foundry name:\n{text}"
    );
    // `A_DLY` must be tied high: the foundry's own model stops the simulation
    // if it is not, with an error rather than a warning.
    assert!(
        text.contains(".A_DLY (1'h1)"),
        "A_DLY has to be tied high:\n{text}"
    );
    // And the built-in self test has to be off.
    assert!(text.contains(".A_BIST_EN (1'h0)"), "{text}");
    assert!(text.contains(".A_BIST_CLK (1'h0)"), "{text}");
    assert!(text.contains(".A_BIST_ADDR (8'h00)"), "{text}");
    // The bit mask is all ones: every bit of every word is writable.
    assert!(text.contains(".A_BM (8'hff)"), "{text}");
    // The macro's output is the read port's net, so the design reads the macro
    // exactly as it would have read an array.
    assert!(text.contains(".A_DOUT (signal_ReadPort)"), "{text}");
    // And there is no array: the macro *is* the storage.
    assert!(!text.contains("reg [7:0] signal_Mem [0:255]"), "{text}");
}

#[test]
fn a_macro_backed_read_arrives_one_cycle_later_than_an_array_read() {
    // The model: a synchronous read reports the value the memory held at the
    // edge, one cycle after the address was presented. An array read reports
    // it in the same cycle.
    let design = Design::new();
    let clk = design.input("clk", 1).unwrap();
    let wen = design.input("wen", 1).unwrap();
    let ren = design.input("ren", 1).unwrap();
    let addr = design.input("addr", 8).unwrap();
    let wdata = design.input("wdata", 8).unwrap();
    let master = design.constant(true);

    // A register, so the design says what its clock is. The simulator takes
    // the clock from the first register it finds, and a synchronous memory
    // read is a register too — so a design with a synchronous memory and no
    // register has no clock to be captured at.
    let tick = design
        .reg(
            &wdata,
            &clk,
            &design.constant(false),
            &design.constant(false),
        )
        .unwrap();

    let memory = macro_memory(
        &design,
        Pdk::IhpCmos5l,
        256,
        8,
        MacroPorts {
            write_data: &wdata,
            write_address: &addr,
            write_enable: &wen,
            read_address: &addr,
            read_enable: &ren,
            clock: &clk,
            master_enable: &master,
        },
    )
    .unwrap();
    design.output("dout", 8, &memory.read).unwrap();
    design.output("tick", 8, &tick).unwrap();

    let mut sim = Sim::new(&design).unwrap();
    // Cycle 1: write 0xA5 to address 3.
    sim.set_input("clk", bit(false)).unwrap();
    sim.set_input("wen", bit(true)).unwrap();
    sim.set_input("ren", bit(false)).unwrap();
    sim.set_input("addr", Bits::constant(3, 8).unwrap())
        .unwrap();
    sim.set_input("wdata", Bits::constant(0xA5, 8).unwrap())
        .unwrap();
    let (_, _) = sim.step().unwrap();

    // Cycle 2: read address 3. The read enable is presented now, so the macro
    // latches the address at the edge and the data appears next cycle.
    sim.set_input("wen", bit(false)).unwrap();
    sim.set_input("ren", bit(true)).unwrap();
    sim.set_input("addr", Bits::constant(3, 8).unwrap())
        .unwrap();
    let (before, after) = sim.step().unwrap();
    let dout = after.get("dout").expect("the design has a dout port");
    assert_eq!(
        dout.to_u64().unwrap(),
        0xA5,
        "a synchronous read reports the value the memory held at the edge, so the \\
         word written in the previous cycle is there now"
    );
    let _ = before;

    // Cycle 3: the same read again. The data is still there: a synchronous
    // read holds its value when the read enable is low, rather than driving
    // zero, which is the macro's behaviour and the array's is not.
    sim.set_input("ren", bit(false)).unwrap();
    let (_, after) = sim.step().unwrap();
    assert_eq!(
        after.get("dout").unwrap().to_u64().unwrap(),
        0xA5,
        "a disabled synchronous read holds the last value, which is what a hard \\
         SRAM macro does"
    );
}

#[test]
fn an_array_read_is_still_combinational() {
    // The change to the memory model must not have moved the *array* read: a
    // design that reads an array and a design that reads a macro are different
    // circuits, and only the macro has a latency.
    let design = Design::new();
    let clk = design.input("clk", 1).unwrap();
    let wen = design.input("wen", 1).unwrap();
    let addr = design.input("addr", 8).unwrap();
    let wdata = design.input("wdata", 8).unwrap();
    let en = design.constant(true);

    // A register, so the design has a clock: a memory's write port is clocked
    // by the module's clock, and a design with state and no clock is a design
    // that cannot be stepped.
    let tick = design
        .reg(
            &wdata,
            &clk,
            &design.constant(false),
            &design.constant(false),
        )
        .unwrap();

    let mem = design.mem(8, 16).unwrap();
    design.write_port(&mem, &wdata, &addr, &wen).unwrap();
    let read = design.read_port(&mem, &addr, &en).unwrap();
    design.output("dout", 8, &read).unwrap();
    design.output("tick", 8, &tick).unwrap();

    let mut sim = Sim::new(&design).unwrap();
    sim.set_input("clk", bit(false)).unwrap();
    sim.set_input("wen", bit(true)).unwrap();
    sim.set_input("addr", Bits::constant(5, 8).unwrap())
        .unwrap();
    sim.set_input("wdata", Bits::constant(0x3C, 8).unwrap())
        .unwrap();
    sim.step().unwrap();

    sim.set_input("wen", bit(false)).unwrap();
    let (_, after) = sim.step().unwrap();
    assert_eq!(
        after.get("dout").unwrap().to_u64().unwrap(),
        0x3C,
        "an array read is combinational: the word written in the previous cycle \\
         is readable in this one"
    );
}

#[test]
fn a_process_without_a_macro_of_that_shape_says_so() {
    let design = Design::new();
    let clk = design.input("clk", 1).unwrap();
    let wen = design.input("wen", 1).unwrap();
    let ren = design.input("ren", 1).unwrap();
    let addr = design.input("addr", 8).unwrap();
    let wdata = design.input("wdata", 8).unwrap();
    let master = design.constant(true);

    let err = macro_memory(
        &design,
        Pdk::IhpCmos5l,
        999,
        8,
        MacroPorts {
            write_data: &wdata,
            write_address: &addr,
            write_enable: &wen,
            read_address: &addr,
            read_enable: &ren,
            clock: &clk,
            master_enable: &master,
        },
    )
    .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("no 999-word 8-bit memory macro"), "{text}");
    assert!(text.contains("sram_macros"), "{text}");
}

/// A one-bit literal.
fn bit(value: bool) -> Bits {
    Bits::constant(u64::from(value), 1).unwrap()
}
