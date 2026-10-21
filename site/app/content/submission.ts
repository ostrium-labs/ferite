/**
 * Documentation content for the two submission-boundary crates.
 *
 * These two are the answer to a question the DSL cannot answer: how does a
 * design become a chip? The first holds what the *process* is; the second holds
 * what the *die* is. Neither is part of the DSL, and both are documented here
 * because the reader who has just finished the emitter walkthrough is exactly
 * the reader who needs to know where the submission boundary is.
 *
 * Every number below is measured: the macro areas are out of the IHP open PDK's
 * own LEF files, and the tile figures are the competition's rules.
 */

import type { Page } from "./types";

export const SUBMISSION: Page[] = [
  {
    slug: "pdk",
    title: "What a process is",
    crate: "ferrite-lithic-pdk",
    tier: "—",
    summary:
      "Tile geometry, flip-flops per tile, and the memory macros with their names, port lists and measured areas.",
    lede: [
      "The DSL crates are a faithful reimplementation of Hardcaml's model, and Hardcaml knows nothing about foundries. What *is* process knowledge — how big a tile is, how many flip-flops fit in one, whether the process has a hard SRAM and what it is called — changes with every shuttle and is not the DSL's business.",
      "So it lives here, on its own, with the facts written down and dated. A design that targets IHP asks this crate what IHP has; a design that targets sky130 asks the same question and gets a different answer. The DSL does not change, and neither does the other process's data.",
    ],
    steps: [
      {
        title: "Name the process",
        detail:
          "Three are known. `IhpCmos5l` is the submission target; `Sky130` is kept because it is the most mature open PDK with the best-documented tooling, so it is the cheapest place to iterate on the DSL; `Gf180mcu` is the third live open node.",
        code: `use ferrite_lithic_pdk::Pdk;

assert_eq!(Pdk::IhpCmos5l.open_pdk_name(), "ihp-sg13g2");
assert_eq!(Pdk::IhpCmos5l.feature_size_nm(), 130);
assert_eq!(Pdk::IhpCmos5l.display_name(), "IHP 130nm CMOS5L");`,
        figure: { value: "130", unit: "nm", caption: "the submission process's feature size" },
      },
      {
        title: "Measure the tile, and the flip-flops in it",
        detail:
          "The tile is 200 × 150 µm — the figure the competition rules state, recorded as *that* rather than as a fact about the process, because only one of the two can be checked against something we can point at. The flip-flop count is what rules out flip-flop memory: 320 per tile over 24 tiles is 7 680 flip-flops, which is 960 bytes, and that is the *entire* design's register budget spent on nothing else.",
        code: `use ferrite_lithic_pdk::Pdk;

let ihp = Pdk::IhpCmos5l;
assert!(ihp.tile_area_um2() > 25_000.0);
assert_eq!(ihp.dff_per_tile(), 320);`,
      },
      {
        title: "Look up the macro, and what it costs",
        detail:
          "The macro's name is the cell name in the LEF, its area is the `SIZE` in the same LEF in micrometres, and its port list is the port list in the foundry's Verilog simulation model. Nothing here is paraphrased from a datasheet summary.",
        code: `use ferrite_lithic_pdk::Pdk;

let ihp = Pdk::IhpCmos5l;
let macro_form = ihp.sram(1024, 8).expect("IHP has a 1024x8 macro");
assert_eq!(macro_form.module, "RM_IHPSG13_1P_1024x8_c2_bm_bist");
assert_eq!(macro_form.bytes(), 1024);
assert_eq!(macro_form.read_latency, 1);`,
      },
      {
        title: "Count the tiles, not the area",
        detail:
          "A macro is placed on a tile grid, so what it costs is its **bounding box**, not its area. The 1024x8 macro is 147 µm wide and 336 µm tall, which on a 200 × 150 µm tile is one tile wide and three tiles tall — whatever its 49 419 µm² area says. This is the calculation that decides the program-memory budget.",
        code: `use ferrite_lithic_pdk::Pdk;

let ihp = Pdk::IhpCmos5l;
let macro_form = ihp.sram(1024, 8).unwrap();
// One tile wide, three tall.
assert_eq!(ihp.tiles_for_sram(macro_form), 3.0);

// And the biggest macro in the family does not fit at all.
let biggest = ihp.sram(8192, 32).unwrap();
assert!(ihp.tiles_for_sram(biggest) > 24.0);`,
        figure: { value: "40", unit: "tiles", caption: "the 8192x32's bounding box, against an allocation of 24" },
      },
    ],
    gotchas: [
      {
        title: "The area is not the cost that matters",
        body: "A macro's area and its tile cost differ by a factor of two or three, because the grid is not the area. Reading a macro's area and dividing it by a tile's area gives the number a datasheet would quote, and it is wrong in the direction that matters: it says a 4096x8 macro costs 4.9 tiles when it costs ten.",
      },
      {
        title: "The tile figure is a rule, not a measurement",
        body: "200 × 150 µm is what the competition rules state. It is recorded as the competition's number and not as a fact about the process, because the two are not the same thing and only one of them can be checked against a source we can point at.",
      },
      {
        title: "Sky130 and GF180MCU have no macro table here",
        body: "The IHP table is measured; the other two are empty rather than filled with plausible numbers. A design that asks for a macro the process does not have should be told so, not handed an estimate that turns into a floorplan that does not close.",
      },
    ],
    notes: [
      {
        kind: "measured",
        title: "The macro areas come out of the PDK's LEF files",
        body: "The 1024x8 is 146.88 × 336.46 µm, the 4096x8 is 236.8 × 618.3 µm, and the 8192x32 is 1520.16 × 618.3 µm. Those are `SIZE` records in `ihp-sg13g2/libs.ref/sg13g2_sram/lef/`, not estimates.",
      },
      {
        kind: "decision",
        title: "The read latency is one, and it is the macro's",
        body: "Every macro in the table samples the read enable and the address at the clock edge and produces the data in the next cycle. That is why a memory's read latency is a property of the memory rather than of the backend.",
      },
      {
        kind: "trap",
        title: "The `c2`, `c3` and `c4` suffixes are not performance grades",
        body: "They are the foundry's column counts. A `c3` macro is not a faster `c2` macro, and the suffix is part of the name and has to be copied exactly.",
      },
    ],
  },
  {
    slug: "chip",
    title: "The submission boundary",
    crate: "ferrite-lithic-chip",
    tier: "—",
    summary:
      "The Tiny Tapeout wrapper, the pin budget, the `info.yaml`, and a memory on a foundry macro.",
    lede: [
      "A Tiny Tapeout submission is not a Verilog file. It is a directory: a top module with a **fixed** port list, an `info.yaml` describing the pinout, and a documentation page. The port list is fixed because the pads are cut before the design is written, and the module name is fixed by convention because the shuttle documentation indexes the die by it.",
      "Getting any of that wrong is the most common submission failure, and it is a *mechanical* failure: not a design bug, not a timing bug, a shape mismatch. This crate is the generator that makes the shape impossible to get wrong by hand.",
    ],
    steps: [
      {
        title: "Map the design's ports onto the die's pads",
        detail:
          "A pin map names the core module's ports, and the pin budget is checked when the map is built. The die has eight dedicated inputs, eight dedicated outputs and eight bidirectional pins; the clock, the reset and the power enable are the die's, not the design's. A design that wants twenty-five user pins does not fit, and the error says so by name rather than at the end.",
        code: `use ferrite_lithic_chip::PinMap;

let pins = PinMap::builder()
    .input("rx", "rx")
    .output("tx", "tx")
    .bidirectional("scl", "scl_in", "scl_out", "scl_oe")
    .build()?;`,
      },
      {
        title: "Generate the wrapper",
        detail:
          "`tt_um_<user>_<project>`, with the die's exact port list. The name is legalised, because a GitHub user name may contain a hyphen and a Verilog identifier may not — and telling the submitter to pick a different account name would be worse than rewriting one character.",
        code: `use ferrite_lithic_chip::{top_module_name, wrapper};

let text = wrapper(&chip, &core, &pins)?;
assert!(text.contains("module tt_um_ostrium_labs_ferrite ("));
assert!(text.contains("input  wire [7:0] uio_oe,"));`,
      },
      {
        title: "Let the wrapper do the three things the design must not",
        detail:
          "It inverts the die's active-low reset into the DSL's active-high synchronous one, so the design is written in one polarity. It ties off every pad the design does not use, because an undriven output pad is an `x` in simulation and a warning in every tool downstream. And it turns a bidirectional pad into the three signals a pad really is.",
        code: `// The reset inversion, at the boundary.
assert!(text.contains("wire reset = ~rst_n;"));

// The unused pads, tied off.
assert!(text.contains("assign uo_out[7] = 1'b0;"));

// A bidirectional pad: read, drive, and whether to drive.
assert!(text.contains(".scl_in (uio_in[0])"));
assert!(text.contains(".scl_out (uio_out[0])"));
assert!(text.contains(".scl_oe (uio_oe[0])"));`,
      },
      {
        title: "Build a memory on the macro, with the details in one place",
        detail:
          "`A_DLY` must be tied high — the foundry's own model stops the simulation otherwise, with an error rather than a warning. The eight `A_BIST_*` pins have to be tied off. A single-port macro has one address and a design wants two, so the two are muxed. And the read is synchronous, so the memory carries a read latency that both backends honour.",
        code: `use ferrite_lithic_chip::sram::{MacroPorts, macro_memory};
use ferrite_lithic_pdk::Pdk;

let memory = macro_memory(
    &design,
    Pdk::IhpCmos5l,
    1024,
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
)?;`,
      },
      {
        title: "Write the whole submission",
        detail:
          "`src/project.v`, `src/core.v`, `info.yaml` and `docs/info.md`, in the layout the Tiny Tapeout template uses. The `info.yaml` carries the tile count, the top module name, the source files and the pinout — and the pinout names are the design's, not the port names, because the pinout is what the datasheet is generated from.",
        code: `use ferrite_lithic_chip::{project, write_project};

let files = project(&chip, &core, &pins, &["project.v", "core.v"])?;
write_project(std::path::Path::new("./tt-submission"), &files)?;`,
      },
    ],
    gotchas: [
      {
        title: "A pad is one bit",
        body: "A pin names a one-bit core port, and a wide port is refused rather than truncated. A design that wants an eight-bit bus across `ui_in[7:0]` declares eight pins, or declares one eight-bit core port and unpacks it itself.",
      },
      {
        title: "The output enable is not decoration",
        body: "It is how a pin becomes an input or an output at run time, which is the whole reason a protocol emulator has bidirectional pins at all: an I2C master has to release the clock line and let the slave pull it low, and only a tri-state pad can do that.",
      },
      {
        title: "The port names in the map are the core's, and they are legalised",
        body: "The wrapper connects the identifiers the emitter actually wrote, so a core port called `signed` is emitted as `signed_` and connected as `signed_` rather than as a name the emitter never wrote.",
      },
    ],
    notes: [
      {
        kind: "decision",
        title: "The pin budget is twenty-four user pins, not twenty-six",
        body: "Twenty-six is the die's total: twenty-four user pins plus the clock, the reset and the power enable. A design that wants twenty-five user pins does not fit.",
      },
      {
        kind: "trap",
        title: "A synchronous memory read holds; an array read drives zero",
        body: "A disabled synchronous read keeps the last value it captured, because a macro's output register holds. A disabled array read drives zero, because an array read is combinational and has no previous value. `Op::ReadPort` grew a `hold` flag for exactly this, and it is observable.",
      },
      {
        kind: "gap",
        title: "The tri-state lives in the wrapper, not in the IR",
        body: "The IR has no `inout` and no open-drain. That is fine for a pad and not fine for I2C's open-drain clock line *inside* the design, which is a real gap for the protocol engine and a different gap from the pad.",
      },
    ],
  },
];
