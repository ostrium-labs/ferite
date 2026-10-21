//! The `tt_um_*` wrapper: the module the die actually sees.
//!
//! # The wrapper is not a convenience
//!
//! A Tiny Tapeout submission is a single module with a **fixed** port list:
//! eight dedicated inputs, eight dedicated outputs, eight bidirectional pins, a
//! clock, an active-low reset and a power enable. The pads are cut before the
//! design is written, so the wrapper is not something a designer gets to shape
//! — it is the shape of the die.
//!
//! ADR-0017 of this repository calls it *"a mechanical, unforgiving
//! requirement and the most common submission failure"*, and says the wrapper
//! module name and its exact port list *"are generated automatically by the
//! emitter"*. This is that generator.
//!
//! # Three things the wrapper does that the design must not
//!
//! 1. **It inverts the reset.** The die's `rst_n` is active low; the DSL's
//!    register reset is active high and synchronous. The inversion belongs at
//!    the boundary, so the design is written in one polarity and cannot get it
//!    wrong in three places.
//! 2. **It ties off the unused pads.** An undriven output pad is an `x` in
//!    simulation and a warning in every tool downstream. Every output bit the
//!    design does not use is driven to zero.
//! 3. **It turns a bidirectional pin into three signals.** `uio_in` is what the
//!    pad reads, `uio_out` is what the design wants to drive, and `uio_oe` says
//!    whether to drive it. The enable is active high.

use ferrite_lithic_rtl::Module;

use crate::chip::Chip;
use crate::pins::{
    BIDIRECTIONAL_PINS, DEDICATED_INPUTS, DEDICATED_OUTPUTS, Error, PinMap, port_table,
};

/// The wrapper module's name.
///
/// `tt_um_<user>_<project>`, which is the form the rules require: *"Your top
/// module name must start with `tt_um_`. Make it unique by including your
/// github username."*
///
/// # The name is legalised, and that is not a detail
///
/// A GitHub user name may contain a hyphen, and a Verilog identifier may not.
/// `ostrium-labs` is a perfectly good user name and `tt_um_ostrium-labs_ferrite`
/// is not a legal module name, so the hyphen becomes an underscore.
///
/// The alternative — telling the submitter to pick a different user name —
/// is worse: the name is supposed to be the person who built the thing, and
/// the person did not choose their account name for this. The legalised name
/// still starts with `tt_um_` and still carries the user name, which is what
/// the rule is for.
#[must_use]
pub fn top_module_name(chip: &Chip) -> String {
    legalise(&format!("tt_um_{}_{}", chip.user, chip.project))
}

/// A legal Verilog identifier, derived from a name that may not be one.
///
/// Anything that is not a letter, a digit or an underscore becomes an
/// underscore, and a name that starts with a digit gets an underscore in
/// front. Reserved words are not handled here: the name always starts with
/// `tt_um_`, so it cannot be one.
fn legalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 1);
    for (index, ch) in name.chars().enumerate() {
        let ok = ch.is_ascii_alphanumeric() || ch == '_';
        if index == 0 && ch.is_ascii_digit() {
            out.push('_');
        }
        out.push(if ok { ch } else { '_' });
    }
    if out.is_empty() { "_".to_string() } else { out }
}

/// Generate the wrapper module.
///
/// # Errors
///
/// - [`Error::NoClock`] if the core module has no clock, because the die's
///   clock pin has to go somewhere.
/// - [`Error::UnknownPort`] if a pin names a port the module does not have.
/// - [`Error::PortWidth`] if a pin names a port that is not one bit wide.
/// - [`Error::TooManyInputs`] and friends, from [`PinMap::check`].
pub fn wrapper(chip: &Chip, core: &Module, pins: &PinMap) -> Result<String, Error> {
    pins.check()?;

    let identifiers = core.port_identifiers_by_name()?;
    let widths = port_table(core)?;
    let clock = core.clock().ok_or(Error::NoClock)?;
    let clock_name = core
        .circuit()
        .name(clock)
        .ok_or(Error::NoClock)?
        .to_string();

    let resolve = |name: &str| -> Result<String, Error> {
        identifiers
            .get(name)
            .cloned()
            .ok_or_else(|| Error::UnknownPort {
                port: name.to_string(),
                available: identifiers.keys().cloned().collect(),
            })
    };
    let width_of = |name: &str| -> Result<u32, Error> {
        widths
            .iter()
            .find(|(port, _)| port == name)
            .map(|(_, width)| *width)
            .ok_or_else(|| Error::UnknownPort {
                port: name.to_string(),
                available: widths.iter().map(|(p, _)| p.clone()).collect(),
            })
    };
    // A pad is one bit. A core port that is wider cannot be bound to one.
    let one_bit = |name: &str| -> Result<String, Error> {
        let width = width_of(name)?;
        if width != 1 {
            return Err(Error::PortWidth {
                port: name.to_string(),
                expected: 1,
                found: width,
            });
        }
        resolve(name)
    };

    let mut out = String::new();
    out.push_str("// Generated by ferrite-lithic-chip. Do not edit; regenerate from the design.\n");
    out.push_str("// The Tiny Tapeout top module: a fixed port list, because the pads are cut\n");
    out.push_str("// before the design is written.\n");
    out.push_str("`default_nettype none\n\n");
    out.push_str(&format!("module {} (\n", top_module_name(chip)));
    out.push_str("    input  wire [7:0] ui_in,    // Dedicated inputs\n");
    out.push_str("    output wire [7:0] uo_out,   // Dedicated outputs\n");
    out.push_str("    input  wire [7:0] uio_in,   // IOs: Input path\n");
    out.push_str("    output wire [7:0] uio_out,  // IOs: Output path\n");
    out.push_str(
        "    output wire [7:0] uio_oe,   // IOs: Enable path (active high: 0=input, 1=output)\n",
    );
    out.push_str("    input  wire       ena,      // always 1 when the design is powered, so you can ignore it\n");
    out.push_str("    input  wire       clk,      // clock\n");
    out.push_str("    input  wire       rst_n     // reset_n - low to reset\n");
    out.push_str(");\n\n");

    out.push_str("  // The die's reset is active low; the DSL's register reset is active high\n");
    out.push_str("  // and synchronous. The inversion happens here, at the boundary, so the\n");
    out.push_str("  // design is written in one polarity and cannot get it wrong.\n");
    out.push_str("  wire reset = ~rst_n;\n\n");

    out.push_str(&format!("  {} u_core (\n", core.name()));
    let mut connections: Vec<(String, String)> = Vec::new();
    connections.push((resolve(&clock_name)?, "clk".to_string()));
    connections.push((resolve(&chip.reset_port)?, "reset".to_string()));
    for (index, pin) in pins.inputs().iter().enumerate() {
        connections.push((one_bit(&pin.port)?, format!("ui_in[{index}]")));
    }
    for (index, pin) in pins.outputs().iter().enumerate() {
        connections.push((one_bit(&pin.port)?, format!("uo_out[{index}]")));
    }
    for (index, pin) in pins.bidirectional().iter().enumerate() {
        connections.push((one_bit(&pin.input_port)?, format!("uio_in[{index}]")));
        connections.push((one_bit(&pin.output_port)?, format!("uio_out[{index}]")));
        connections.push((one_bit(&pin.enable_port)?, format!("uio_oe[{index}]")));
    }
    for (index, (port, signal)) in connections.iter().enumerate() {
        let comma = if index + 1 == connections.len() {
            ""
        } else {
            ","
        };
        out.push_str(&format!("    .{port} ({signal}){comma}\n"));
    }
    out.push_str("  );\n\n");

    // Every output bit the design does not use is tied off.
    let mut tied: Vec<String> = Vec::new();
    for index in pins.outputs().len()..DEDICATED_OUTPUTS {
        tied.push(format!("  assign uo_out[{index}] = 1'b0;"));
    }
    for index in pins.bidirectional().len()..BIDIRECTIONAL_PINS {
        tied.push(format!("  assign uio_out[{index}] = 1'b0;"));
        tied.push(format!("  assign uio_oe[{index}] = 1'b0;"));
    }
    if !tied.is_empty() {
        out.push_str("  // The pads the design does not use are tied off: an undriven output\n");
        out.push_str("  // pad is an `x` in simulation and a warning in every tool downstream.\n");
        for line in &tied {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str("  // `ena` is always one when the design is powered.\n");
    out.push_str("  wire _unused = &{ena, 1'b0};\n");
    out.push_str("endmodule\n");
    Ok(out)
}

/// The pinout of a submission: three groups of pad names.
///
/// The order is the template's — dedicated inputs, dedicated outputs,
/// bidirectional pins — so a generated `info.yaml` reads the same as a
/// hand-written one. YAML does not care about the order of a mapping, but a
/// reader diffing a generated file against the template would.
///
/// The names are what the datasheet and the shuttle documentation are
/// generated from, so a name that appears twice would publish one signal
/// twice. The builder refuses that; this function trusts the map.
#[must_use]
pub fn pinout(pins: &PinMap) -> Vec<(&'static str, Vec<String>)> {
    vec![
        (
            "ui",
            (0..DEDICATED_INPUTS)
                .map(|index| {
                    pins.inputs()
                        .get(index)
                        .map_or(String::new(), |p| p.name.clone())
                })
                .collect(),
        ),
        (
            "uo",
            (0..DEDICATED_OUTPUTS)
                .map(|index| {
                    pins.outputs()
                        .get(index)
                        .map_or(String::new(), |p| p.name.clone())
                })
                .collect(),
        ),
        (
            "uio",
            (0..BIDIRECTIONAL_PINS)
                .map(|index| {
                    pins.bidirectional()
                        .get(index)
                        .map_or(String::new(), |p| p.name.clone())
                })
                .collect(),
        ),
    ]
}
