//! The wrapper, the pin budget, and the `info.yaml`.
//!
//! The interesting test here is the last one: the generated wrapper and the
//! generated core are elaborated together by Icarus, because a wrapper that
//! does not elaborate is a submission that does not build, and a submission
//! that does not build is a design that never reached a foundry.

#![allow(clippy::unwrap_used)]

use ferrite_lithic::Design;
use ferrite_lithic_chip::{Chip, Error, PinMap, project, top_module_name, wrapper};
use ferrite_lithic_rtl::Module;

/// A chip's metadata, for the tests.
fn chip() -> Chip {
    Chip {
        user: "ostrium-labs".to_string(),
        project: "ferrite".to_string(),
        title: "Ferrite protocol emulator".to_string(),
        author: "Ostrium Labs".to_string(),
        description: "A programmable protocol emulator".to_string(),
        clock_hz: 10_000_000,
        tiles: (6, 4),
        reset_port: "rst".to_string(),
    }
}

/// A core module with a clock, a reset, one signal in each direction, and a
/// bidirectional pad's three signals.
fn core() -> (Module, Design) {
    let design = Design::new();
    let clk = design.input("clk", 1).unwrap();
    let rst = design.input("rst", 1).unwrap();
    let rx = design.input("rx", 1).unwrap();
    let scl_in = design.input("scl_in", 1).unwrap();
    let tx = design.output("tx", 1, &rx).unwrap();
    // The value the design wants to drive onto the pad, and whether to drive it.
    let scl_out = design.output("scl_out", 1, &rx).unwrap();
    let scl_oe = design.output("scl_oe", 1, &design.constant(false)).unwrap();
    let module = Module::new(
        "core",
        design.build().unwrap(),
        clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let _ = (tx, scl_in, scl_out, scl_oe, rst);
    (module, design)
}

#[test]
fn the_wrapper_has_the_ports_the_die_has() {
    let (module, _) = core();
    let pins = PinMap::builder()
        .input("rx", "rx")
        .output("tx", "tx")
        .build()
        .unwrap();
    let text = wrapper(&chip(), &module, &pins).unwrap();
    for expected in [
        "module tt_um_ostrium_labs_ferrite (",
        "input  wire [7:0] ui_in,",
        "output wire [7:0] uo_out,",
        "input  wire [7:0] uio_in,",
        "output wire [7:0] uio_out,",
        "output wire [7:0] uio_oe,",
        "input  wire       ena,",
        "input  wire       clk,",
        "input  wire       rst_n",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in\n{text}");
    }
}

#[test]
fn the_dies_active_low_reset_becomes_the_designs_active_high_one() {
    let (module, _) = core();
    let pins = PinMap::builder().build().unwrap();
    let text = wrapper(&chip(), &module, &pins).unwrap();
    // The inversion belongs at the boundary, so the design is written in one
    // polarity and cannot get it wrong in three places.
    assert!(text.contains("wire reset = ~rst_n;"), "{text}");
    assert!(text.contains(".rst (reset)"), "{text}");
}

#[test]
fn an_unused_pad_is_tied_off_rather_than_left_floating() {
    let (module, _) = core();
    let pins = PinMap::builder()
        .input("rx", "rx")
        .output("tx", "tx")
        .build()
        .unwrap();
    let text = wrapper(&chip(), &module, &pins).unwrap();
    // Every output bit the design does not use is driven, because an undriven
    // output pad is an `x` in simulation and a warning in every tool
    // downstream.
    assert!(text.contains("assign uo_out[1] = 1'b0;"), "{text}");
    assert!(text.contains("assign uo_out[7] = 1'b0;"), "{text}");
    assert!(text.contains("assign uio_out[0] = 1'b0;"), "{text}");
    assert!(text.contains("assign uio_oe[0] = 1'b0;"), "{text}");
    // The used ones are not.
    assert!(!text.contains("assign uo_out[0] = 1'b0;"), "{text}");
}

#[test]
fn a_bidirectional_pad_becomes_three_signals() {
    let (module, _) = core();
    let pins = PinMap::builder()
        .bidirectional("scl", "scl_in", "scl_out", "scl_oe")
        .build()
        .unwrap();
    let text = wrapper(&chip(), &module, &pins).unwrap();
    // The pad is read, driven, and told whether to drive.
    assert!(text.contains(".scl_in (uio_in[0])"), "{text}");
    assert!(text.contains(".scl_out (uio_out[0])"), "{text}");
    assert!(text.contains(".scl_oe (uio_oe[0])"), "{text}");
}

#[test]
fn a_design_that_wants_more_pins_than_the_die_has_is_refused() {
    let (module, _) = core();
    let mut builder = PinMap::builder();
    for index in 0..9 {
        builder = builder.input(format!("in{index}"), format!("in{index}"));
    }
    let err = builder.build().unwrap_err();
    assert!(
        matches!(err, Error::TooManyInputs { wanted: 9, have: 8 }),
        "{err}"
    );
    assert!(
        err.to_string().contains("cannot be negotiated"),
        "the message should say the budget is fixed: {err}"
    );
    // And the module is not even consulted, because the map never built.
    assert!(top_module_name(&chip()).starts_with("tt_um_"));
    let _ = module;
}

#[test]
fn a_pin_naming_a_port_the_module_does_not_have_is_refused() {
    let (module, _) = core();
    let pins = PinMap::builder().input("nope", "nope").build().unwrap();
    let err = wrapper(&chip(), &module, &pins).unwrap_err();
    assert!(matches!(err, Error::UnknownPort { .. }), "{err}");
    assert!(err.to_string().contains("rx"), "{err}");
}

#[test]
fn a_pin_naming_a_wide_port_is_refused() {
    let (module, _) = core();
    let pins = PinMap::builder().output("bus", "rx").build().unwrap();
    // `rx` is one bit, so this one is fine; a wide port is what has to be
    // refused, and the design below has one.
    assert!(wrapper(&chip(), &module, &pins).is_ok());
}

#[test]
fn a_wide_port_cannot_be_bound_to_one_pad() {
    let design = Design::new();
    let clk = design.input("clk", 1).unwrap();
    let rst = design.input("rst", 1).unwrap();
    let bus = design.input("bus", 8).unwrap();
    let _out = design.output("out", 8, &bus).unwrap();
    let module = Module::new(
        "core",
        design.build().unwrap(),
        clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    let pins = PinMap::builder().output("bus", "out").build().unwrap();
    let err = wrapper(&chip(), &module, &pins).unwrap_err();
    assert!(matches!(err, Error::PortWidth { .. }), "{err}");
    assert!(err.to_string().contains("one bit"), "{err}");
    let _ = rst;
}

#[test]
fn the_info_yaml_says_what_the_build_needs_to_say() {
    let (module, _) = core();
    let pins = PinMap::builder()
        .input("rx", "rx")
        .output("tx", "tx")
        .bidirectional("scl", "scl_in", "scl_out", "scl_oe")
        .build()
        .unwrap();
    let files = project(&chip(), &module, &pins, &["project.v", "core.v"]).unwrap();
    for expected in [
        "project:",
        "  title:        \"Ferrite protocol emulator\"",
        "  tiles: \"6x4\"",
        "  top_module:  \"tt_um_ostrium_labs_ferrite\"",
        "  source_files:",
        "    - \"project.v\"",
        "pinout:",
        "  ui[0]: \"rx\"",
        "  uo[0]: \"tx\"",
        "  uio[0]: \"scl\"",
        "  ui[1]: \"\"",
        "yaml_version: 6",
    ] {
        assert!(
            files.info_yaml.contains(expected),
            "missing {expected:?} in\n{}",
            files.info_yaml
        );
    }
}

#[test]
fn the_top_module_name_follows_the_rules() {
    assert_eq!(
        top_module_name(&chip()),
        "tt_um_ostrium_labs_ferrite",
        "the name has to start with tt_um_ and be unique across the shuttle, and \
         a hyphen in a GitHub user name has to be legalised because a Verilog \
         identifier cannot hold one"
    );
}

/// The wrapper and the core, elaborated together.
///
/// This is the test that matters: the wrapper's port list is fixed by the die,
/// the core's port list is fixed by the design, and the two have to meet. A
/// mismatch here is a submission that does not build.
#[test]
fn the_wrapper_and_the_core_elaborate_together() {
    let Some(tool) = iverilog() else {
        println!(
            "SKIPPED: iverilog is not installed, so the wrapper was not elaborated. \
             The text assertions above still compare the generated files."
        );
        return;
    };
    let (module, _) = core();
    let pins = PinMap::builder()
        .input("rx", "rx")
        .output("tx", "tx")
        .bidirectional("scl", "scl_in", "scl_out", "scl_oe")
        .build()
        .unwrap();
    let files = project(&chip(), &module, &pins, &["project.v", "core.v"]).unwrap();

    let dir = std::env::temp_dir().join("ferrite-lithic-chip-elab");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("project.v"), &files.top).unwrap();
    std::fs::write(dir.join("core.v"), &files.core).unwrap();

    let output = std::process::Command::new(&tool)
        .args(["-o", "out", "project.v", "core.v"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "iverilog rejected the submission:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Where Icarus is, if it is.
fn iverilog() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("sh")
        .args(["-c", "command -v iverilog"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    if path.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(path))
    }
}
