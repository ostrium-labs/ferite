# The submission boundary is a crate, and the process is not the DSL's business

## Status

Accepted. Implements ADR-0017's second half, which asked for a checked pin count
and an automatically generated Tiny Tapeout wrapper.

## Context

A Tiny Tapeout submission is not a Verilog file. It is a directory: a top module
with a **fixed** port list, an `info.yaml` describing the pinout, and a
documentation page.

The port list is fixed because the pads are cut before the design is written. The
IHP template's port list is:

```verilog
module tt_um_example (
    input  wire [7:0] ui_in,    // Dedicated inputs
    output wire [7:0] uo_out,   // Dedicated outputs
    input  wire [7:0] uio_in,   // IOs: Input path
    output wire [7:0] uio_out,  // IOs: Output path
    output wire [7:0] uio_oe,   // IOs: Enable path (active high: 0=input, 1=output)
    input  wire       ena,      // always 1 when the design is powered, so you can ignore it
    input  wire       clk,      // clock
    input  wire       rst_n     // reset_n - low to reset
);
```

Twenty-six pins: eight dedicated inputs, eight dedicated outputs, eight
bidirectional, and the clock, the reset and the power enable.

Three things about this list are not obvious, and each of them is a way to lose a
submission:

1. **The reset is active low.** The DSL's register reset is active high and
   synchronous. Something has to invert it, and it has to be in one place.
2. **A bidirectional pad is three signals, not one.** `uio_in` is what the pad
   reads, `uio_out` is what the design wants to drive, and `uio_oe` says whether
   to drive it. The enable is active high. This is not decoration: an I2C master
   has to release the clock line and let the slave pull it low, and only a
   tri-state pad can do that.
3. **Every output pad must be driven.** The template says so — *"All output pins
   must be assigned. If not used, assign to 0."* An undriven output pad is an
   `x` in simulation and a warning in every tool downstream.

And the process itself: how big a tile is, how many flip-flops fit in one,
whether there is a hard SRAM and what it is called. That knowledge changes with
every shuttle and is not the DSL's business — the DSL crates are a faithful
reimplementation of Hardcaml's model, and Hardcaml knows nothing about foundries.

ADR-0017 already said what the answer should look like: *"the Tiny Tapeout
wrapper module name `tt_um_<user>_<project>` and its exact port list are
generated automatically by the emitter"* and *"I/O count is a checked constraint
with a static error, not a runtime surprise"*. It did not say *where* the
generator lives.

## Decision

**Two new crates, and neither of them is the DSL.**

- **`ferrite-lithic-pdk`** holds the process knowledge: the tile geometry, the
  flip-flops per tile, and the memory macros with their names, port lists and
  **areas measured out of the foundry's own LEF files**. It depends on nothing.
- **`ferrite-lithic-chip`** holds the submission boundary: the pin map and its
  budget check, the `tt_um_*` wrapper, the `info.yaml`, the documentation page,
  and the helper that builds a memory on a foundry macro.

The chip crate depends on the DSL crates; the DSL crates do not depend on it. A
design that targets sky130 and a design that targets IHP ask the same questions
and get different answers, and neither the questions nor the answers are the
DSL's.

**The pin budget is checked, and the check is a static error.** `PinMap` refuses
more than eight dedicated inputs, eight dedicated outputs or eight bidirectional
pins, and refuses two pins with the same name.

**The wrapper inverts the reset, ties off the unused pads, and turns a
bidirectional pad into three signals.** All three at the boundary, so the design
is written in one polarity and cannot get it wrong in three places.

**The module name is legalised.** A GitHub user name may contain a hyphen and a
Verilog identifier may not, so `ostrium-labs` becomes `ostrium_labs`. The
alternative — telling the submitter to pick a different user name — is worse: the
name is supposed to be the person who built the thing, and the person did not
choose their account name for this.

## Consequences

**The pin budget is twenty-four user pins, not twenty-six.** The clock, the
reset and the power enable are the die's, not the design's. A design that wants
twenty-five user pins does not fit, and the check says so by name.

**One bit per pad.** A pad is one bit, so a pin names a one-bit core port. A
design that wants an eight-bit bus across `ui_in[7:0]` declares eight pins, or
declares one eight-bit core port and unpacks it itself. Binding a wide port to
one pad is refused rather than truncated.

**The design's ports are named in the pin map, not indexed.** The pin map names
the core module's ports; the wrapper looks them up by name and binds the emitted
identifiers, so a port called `signed` is emitted as `signed_` and connected as
`signed_` rather than as a name the emitter never wrote.

**The macro's details are in one place.** `A_DLY` tied high, the eight `A_BIST_*`
pins tied off, the address mux, the master enable: all of them in
`ferrite-lithic-chip`'s `sram` module, with a test that checks the emitted text
for each one.

**The process data is dated and sourced.** The macro names and areas are read
out of the IHP open PDK's LEF and Verilog files, and the tile geometry is
recorded as *the competition's figure* rather than as a fact about the process,
because only one of those can be checked against something we can point at.
