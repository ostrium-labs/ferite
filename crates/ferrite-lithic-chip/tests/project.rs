//! A whole submission, written to disk.
//!
//! This is the end-to-end check: a design is built, the pin map is checked
//! against the die, the wrapper and the `info.yaml` are generated, the files
//! are written, and then the two Verilog files are elaborated together by
//! Icarus. If that elaborates, the submission would build.

#![allow(clippy::unwrap_used)]

use ferrite_lithic::Design;
use ferrite_lithic_chip::{Chip, PinMap, project, write_project};
use ferrite_lithic_rtl::Module;

/// A chip's metadata.
fn chip() -> Chip {
    Chip {
        user: "ostrium-labs".to_string(),
        project: "ferrite".to_string(),
        title: "Ferrite protocol emulator".to_string(),
        author: "Ostrium Labs".to_string(),
        description: "A programmable protocol emulator: protocols live in firmware".to_string(),
        clock_hz: 10_000_000,
        tiles: (6, 4),
        reset_port: "rst".to_string(),
    }
}

/// A design with a clock, a reset, a UART transmitter and a bidirectional pin.
fn design() -> (Module, Design) {
    let design = Design::new();
    let clk = design.input("clk", 1).unwrap();
    let rst = design.input("rst", 1).unwrap();
    let tx = design.input("tx", 1).unwrap();
    let led = design.output("led", 1, &tx).unwrap();
    let _scl_in = design.input("scl_in", 1).unwrap();
    let _scl_out = design.output("scl_out", 1, &tx).unwrap();
    let _scl_oe = design.output("scl_oe", 1, &rst).unwrap();
    let _ = led;

    let module = Module::new(
        "core",
        design.build().unwrap(),
        clk.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    );
    (module, design)
}

#[test]
fn a_whole_submission_is_written_and_elaborates() {
    let (module, _) = design();
    let pins = PinMap::builder()
        .input("tx", "tx")
        .output("led", "led")
        .bidirectional("scl", "scl_in", "scl_out", "scl_oe")
        .build()
        .unwrap();

    let files = project(&chip(), &module, &pins, &["project.v", "core.v"]).unwrap();

    let dir = std::env::temp_dir().join("ferrite-lithic-chip-project");
    let _ = std::fs::remove_dir_all(&dir);
    write_project(&dir, &files).unwrap();

    // Every file the template's build reads is there.
    for path in ["info.yaml", "src/project.v", "src/core.v", "docs/info.md"] {
        assert!(dir.join(path).exists(), "the submission is missing {path}");
    }

    // And the two Verilog files elaborate together.
    let Some(tool) = iverilog() else {
        println!(
            "SKIPPED: iverilog is not installed, so the submission was not elaborated. \
             The file assertions above still check the generated files."
        );
        return;
    };
    let output = std::process::Command::new(&tool)
        .args(["-o", "out", "src/project.v", "src/core.v"])
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
