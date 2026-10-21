#![allow(clippy::unwrap_used, clippy::expect_used)]

//! How big the pin and timing engine is.
//!
//! The competition allows 24 tiles of about 1 000 logic cells each — roughly
//! 24 000 cells for the whole die — and the engine is the part of the chip
//! that is ours. So its size is a number worth having rather than a number
//! worth assuming.
//!
//! Yosys is looked for rather than required, and the skip is printed, so a
//! green run that measured nothing is visible rather than hidden.

use std::path::{Path, PathBuf};
use std::process::Command;

use ferrite_lithic::Design;
use ferrite_lithic_chip::engine;

/// The engine's design.
fn design() -> Design {
    let design = Design::new();
    engine::build(&design).unwrap();
    design
}

/// The engine's Verilog, under a known module name.
fn emit() -> String {
    let design = design();
    let clock = design
        .input_ports()
        .iter()
        .find(|signal| signal.name().as_deref() == Some("clk"))
        .cloned()
        .unwrap();
    ferrite_lithic_rtl::Module::new(
        "engine",
        design.build().unwrap(),
        clock.id(),
        design.input_ports().iter().map(|s| s.id()).collect(),
        design.output_ports().iter().map(|s| s.id()).collect(),
    )
    .emit()
    .unwrap()
}

/// Runs yosys over the design and returns (gates, flip-flops).
fn synthesise(name: &str, verilog: &str) -> (usize, usize) {
    let dir = std::env::temp_dir().join("ferrite-lithic-chip-synth");
    std::fs::create_dir_all(&dir).unwrap();
    let path: PathBuf = dir.join(format!("{name}.v"));
    std::fs::write(&path, verilog).unwrap();
    let output = run_yosys(name, &path, &dir);
    assert!(
        output.status.success(),
        "yosys rejected {name}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut gates = 0;
    let mut flops = 0;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        let Some((count, kind)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        let kind = kind.trim();
        if !kind.starts_with('$') {
            continue;
        }
        let Ok(count) = count.trim().parse::<usize>() else {
            continue;
        };
        if kind.contains("DFF") {
            flops += count;
        } else {
            gates += count;
        }
    }
    (gates, flops)
}

/// The synthesis script.
///
/// `synth -noabc` because ABC is a separate binary that needs a liberty file to
/// map to, and there is none here; `techmap` then expands the word-level cells
/// so the counts are gates rather than operators.
fn run_yosys(name: &str, path: &Path, dir: &Path) -> std::process::Output {
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{name}.v"));
    let script = format!(
        "read_verilog {file}\nsynth -noabc -top {name}\ntechmap\nopt -fast\nstat -top {name}\n"
    );
    Command::new("yosys")
        .arg("-p")
        .arg(&script)
        .current_dir(dir)
        .output()
        .expect("yosys is on the PATH")
}

/// Whether yosys is on the PATH.
fn yosys_available() -> bool {
    Command::new("sh")
        .args(["-c", "command -v yosys"])
        .output()
        .ok()
        .is_some_and(|output| output.status.success())
}

/// The engine synthesises, and it is small enough to be worth having.
#[test]
#[ignore = "needs yosys; run with --ignored --nocapture"]
fn the_engine_synthesises_and_fits() {
    if !yosys_available() {
        println!(
            "SKIPPED: yosys is not on the PATH, so the engine was not synthesised. \
             A size that was not measured is not a size."
        );
        return;
    }

    let (gates, flops) = synthesise("engine", &emit());
    println!("\npin and timing engine: {gates} gates, {flops} flip-flops");
    println!(
        "  register budget: {:.2} of a tile (at 320 flip-flops per tile)\n",
        flops as f64 / 320.0
    );

    assert!(
        gates > 0 && flops > 0,
        "the engine synthesised to nothing, so the numbers are not a measurement"
    );

    // The engine has to leave room for the CPU, the protocol engine and the
    // SRAM, so it gets a bound rather than a print. The die's logic budget is
    // about 24 000 cells, and the engine is a fraction of it.
    const MAX_GATES: usize = 4_000;
    assert!(
        gates <= MAX_GATES,
        "the engine is {gates} gates, past the {MAX_GATES} allowed. The die has \
         about 24 000 cells in total and the CPU and the protocol engine need \
         most of them, so the pin and timing engine has to stay small."
    );
}
