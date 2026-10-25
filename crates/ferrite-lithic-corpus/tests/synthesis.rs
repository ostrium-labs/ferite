#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Gate counts for every corpus design, synthesised rather than counted.
//!
//! # What this file exists for
//!
//! `graph_metrics.rs` measures the *graph*: nodes, flops, operator depth. It is
//! explicit about what it is not:
//!
//! > These are *structural* figures, not silicon figures. Node count is not gate
//! > count — an `add` is a whole adder, not one gate. […] what a synthesiser
//! > does with the graph is a different question and needs a different tool.
//!
//! This is that tool. It emits each design's Verilog, runs `synth` over it, and
//! reads the cell counts back out of yosys's own `stat` output. The number it
//! produces is the one a chip submission is actually judged on: a design that
//! costs 30 000 gates does not fit in the 24 tiles the competition allows, and
//! no node count says so.
//!
//! # What is measured, and what is not
//!
//! Yosys is run without a liberty file, because the IHP PDK's libraries are not
//! in this repository and are not installable from it. So the cells it reports
//! are *generic* — `$and`, `$or`, `$mux`, `$add`, `$dff` — and **a generic cell
//! is not an IHP standard cell.** The counts are therefore:
//!
//! - **comparable between designs**, because they come from the same synthesis
//!   of the same tool at the same settings, and
//! - **not convertible to an area** without the PDK, which this file does not
//!   attempt.
//!
//! The one number that *is* convertible is the flip-flop count. A flip-flop is a
//! flip-flop in every library, and the competition's rules give the budget
//! directly: about 320 DFF per tile, 24 tiles, so 7 680 flip-flops for the
//! whole die. A design's flop count divided by 320 is how many tiles of
//! *register* budget it takes, and that division is made here.
//!
//! # Running it
//!
//! Yosys is looked for rather than required, and the skip is printed, so a green
//! run that checked nothing is visible rather than hidden. With yosys present:
//!
//! ```text
//! cargo test -p ferrite-lithic-corpus --test synthesis -- --ignored --nocapture
//! ```
//!
//! The CI job `synth` installs yosys and fails if the table is empty.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use ferrite_lithic::Design;
use ferrite_lithic_rtl::Module;

use ferrite_lithic_corpus as corpus;

/// What yosys reported about one design.
#[derive(Clone, Debug, Default)]
struct Cells {
    /// Combinational gates, after `techmap`.
    gates: usize,
    /// Flip-flops.
    flops: usize,
    /// Memory bits yosys inferred, if any.
    memory_bits: usize,
}

impl Cells {
    /// The register budget this design takes, in tiles.
    ///
    /// The competition's rules give about 320 flip-flops per tile, so this is
    /// the one number in this file that can be compared against a budget
    /// without a liberty file.
    fn register_tiles(&self) -> f64 {
        self.flops as f64 / 320.0
    }
}

/// Runs yosys over one design's Verilog and reads the cell counts back.
///
/// The script is deliberately small: `synth` does the work, and `stat` prints
/// the counts. Anything more elaborate — a target, a liberty file, an ABC
/// script — would need the PDK and would make the numbers describe this
/// repository's guesses rather than the design.
fn synthesise(name: &str, verilog: &str) -> Cells {
    let dir = std::env::temp_dir().join("ferrite-lithic-synth");
    std::fs::create_dir_all(&dir).expect("a temp directory");
    let path: PathBuf = dir.join(format!("{name}.v"));
    std::fs::write(&path, verilog).expect("writing the design");

    let output = run_yosys(name, &path, &dir);
    assert!(
        output.status.success(),
        "yosys rejected {name}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_stat(&String::from_utf8_lossy(&output.stdout))
}

/// The synthesis script every design runs.
///
/// `synth -noabc` rather than `synth`, and the reason is worth stating: ABC is
/// a separate binary that yosys spawns, and it is the technology-mapping step
/// that turns a gate netlist into a library's cells. Without a liberty file
/// there is no library to map to, so ABC has nothing to do but consume time
/// and, in a sandbox, fail.
///
/// `techmap` then expands the word-level cells — an `add` is one `$add` cell
/// until it is forty gates — so the counts below are gates rather than
/// operators. An eight-bit adder comes out at forty cells, which is the right
/// answer, and a count of one would have been the wrong one.
fn run_yosys(name: &str, path: &Path, dir: &Path) -> std::process::Output {
    // The file is named relatively and yosys is run *in* its directory, rather
    // than being handed an absolute path. A normal yosys handles either, but a
    // sandboxed one only sees the directory it was started in — and a harness
    // that only works with an absolute path is a harness that cannot be run
    // inside a container.
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{name}.v"));
    let script = format!(
        "read_verilog {file}\nsynth -noabc -top {name}\ntechmap\nopt -fast\nstat -top {name}\n"
    );
    // No `-q`: it suppresses the log, and the log is where `stat` prints.
    Command::new("yosys")
        .arg("-p")
        .arg(&script)
        .current_dir(dir)
        .output()
        .expect("yosys is on the PATH")
}

/// Reads the cell counts out of yosys's `stat` output.
///
/// The output has one indented line per cell kind — `"    17   $_AND_"` — and
/// the kinds are the *internal* gate primitives, because `techmap` has already
/// expanded the word-level cells.
///
/// The sequential kinds are the ones that count against a register budget. A
/// flip-flop is a flip-flop in every library, so its count converts to tiles;
/// the memory cells are counted separately, because a memory is not a
/// flip-flop and does not come out of the register budget.
fn parse_stat(text: &str) -> Cells {
    let mut cells = Cells::default();
    for (kind, count) in stat_rows(text) {
        if kind.contains("DFF") {
            cells.flops += count;
        } else if kind.contains("mem") || kind.contains("MEM") {
            cells.memory_bits += count;
        } else {
            cells.gates += count;
        }
    }
    cells
}

fn stat_rows(text: &str) -> Vec<(&str, usize)> {
    let last = text.rsplit("=== ").next().unwrap_or(text);
    last.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let first = fields.next()?;
            let second = fields.next()?;
            if fields.next().is_some() {
                return None;
            }
            let (kind, count) = if first.starts_with('$') {
                (first, second)
            } else {
                (second, first)
            };
            if !kind.starts_with('$') {
                return None;
            }
            Some((kind, count.parse().ok()?))
        })
        .collect()
}

#[test]
fn stat_parser_accepts_both_column_orders_and_only_the_final_report() {
    for report in [
        "17 $_AND_\n8 $_DFF_P_\n3 $mem_v2",
        "$_AND_ 17\n$_DFF_P_ 8\n$mem_v2 3",
    ] {
        let cells = parse_stat(&format!(
            "=== old ===\n999 $_AND_\n=== top ===\n{report}\nEnd of script."
        ));
        assert_eq!(cells.gates, 17);
        assert_eq!(cells.flops, 8);
        assert_eq!(cells.memory_bits, 3);
    }
    assert_eq!(parse_stat("unrecognized output").gates, 0);
}

/// A design to synthesise: its name, its variant, and how to build it.
type Subject = (&'static str, &'static str, fn(&Design));

/// The designs to synthesise, with the module name each one is emitted under.
fn designs() -> Vec<Subject> {
    vec![
        ("crc3", "3-bit CRC", |d| {
            corpus::crc3::build(d).unwrap();
        }),
        ("crc32", "CRC-32", |d| {
            corpus::crc32::build(d).unwrap();
        }),
        ("ghash", "128-bit field", |d| {
            corpus::ghash::build(d).unwrap();
        }),
        ("sha256", "64 unrolled rounds", |d| {
            corpus::sha256::build(d).unwrap();
        }),
        ("chacha20", "one block/cycle", |d| {
            corpus::chacha20::build(d).unwrap();
        }),
        ("aes", "200-arm S-box", |d| {
            corpus::aes::build(d).unwrap();
        }),
        ("hex", "4 bits in, 8 out", |d| {
            corpus::hex::build(d).unwrap();
        }),
        ("base64", "6 bits in, 8 out", |d| {
            corpus::base64::build(d).unwrap();
        }),
        ("memchr", "comparator", |d| {
            corpus::memchr::build(d).unwrap();
        }),
        ("memchr_rom", "256-entry ROM", |d| {
            corpus::memchr::build_from_rom(d).unwrap();
        }),
        ("aho_corasick", "19 states", |d| {
            corpus::aho_corasick::build(d).unwrap();
        }),
        ("dfa", "5 states, 64 symbols", |d| {
            corpus::dfa::build(d).unwrap();
        }),
        ("bitpack", "expander, w=8", |d| {
            corpus::bitpack::build(d).unwrap();
        }),
        ("rle", "decoder", |d| {
            corpus::rle::build(d).unwrap();
        }),
        ("roaring", "bitmap", |d| {
            corpus::roaring::build(d).unwrap();
        }),
        ("hamming", "popcount64", |d| {
            corpus::hamming::build(d).unwrap();
        }),
        ("sorting", "bitonic, n=8", |d| {
            corpus::sorting::build(d).unwrap();
        }),
        ("sketch", "64-bit state", |d| {
            corpus::sketch::build(d).unwrap();
        }),
        ("packet", "frame in/frame out", |d| {
            corpus::packet::build(d).unwrap();
        }),
    ]
}

/// Builds a design and emits its Verilog under a known module name.
fn emit(name: &str, build: fn(&Design)) -> String {
    let design = Design::new();
    build(&design);
    let clock = design
        .input_ports()
        .iter()
        .find(|signal| signal.name().as_deref() == Some("clk"))
        .cloned();
    let module = match clock {
        Some(clock) => Module::new(
            name,
            design.build().unwrap(),
            clock.id(),
            design.input_ports().iter().map(|s| s.id()).collect(),
            design.output_ports().iter().map(|s| s.id()).collect(),
        ),
        None => Module::combinational(
            name,
            design.build().unwrap(),
            design.input_ports().iter().map(|s| s.id()).collect(),
            design.output_ports().iter().map(|s| s.id()).collect(),
        ),
    };
    module.emit().unwrap()
}

/// Whether yosys is on the PATH.
fn yosys_available() -> bool {
    Command::new("sh")
        .args(["-c", "command -v yosys"])
        .output()
        .ok()
        .is_some_and(|output| output.status.success())
}

/// Every corpus design synthesises, and the counts are what the file claims.
///
/// The assertion is not about a number: a design that fails to synthesise is a
/// design whose emitted Verilog is not valid, which is the thing worth catching.
/// The counts themselves are printed, because they are a measurement rather than
/// a bound.
#[test]
#[ignore = "needs yosys; run with --ignored --nocapture"]
fn every_corpus_design_synthesises() {
    if !yosys_available() {
        println!(
            "SKIPPED: yosys is not on the PATH, so no design was synthesised. The gate \
             counts are a measurement, and a measurement that did not run is not a \
             measurement."
        );
        return;
    }

    let mut rows: Vec<(&str, &str, Cells)> = Vec::new();
    for (name, variant, build) in designs() {
        let verilog = emit(name, build);
        let cells = synthesise(name, &verilog);
        rows.push((name, variant, cells));
    }

    println!(
        "\n{:<14} {:<22} {:>9} {:>7} {:>9} {:>10}",
        "design", "variant", "gates", "flops", "reg tiles", "mem bits"
    );
    println!("{}", "-".repeat(80));

    for (name, variant, cells) in &rows {
        println!(
            "{:<14} {:<22} {:>9} {:>7} {:>9.2} {:>10}",
            name,
            variant,
            cells.gates,
            cells.flops,
            cells.register_tiles(),
            cells.memory_bits
        );
    }

    let total_gates: usize = rows.iter().map(|(_, _, cells)| cells.gates).sum();
    let total_flops: usize = rows.iter().map(|(_, _, cells)| cells.flops).sum();
    println!(
        "\n{} designs: {total_gates} generic gates, {total_flops} flip-flops \
         ({} tiles of register budget at 320 per tile)\n",
        rows.len(),
        total_flops as f64 / 320.0
    );

    // A design that synthesised to nothing is a design that was optimised away
    // or failed silently, and either way the row is a lie.
    for (name, _, cells) in &rows {
        assert!(
            cells.gates + cells.flops > 0,
            "{name} synthesised to nothing: the emitted Verilog was empty or was \
             optimised away, and the row would be a lie"
        );
    }

    // And the register budget is the one number that can be compared with a
    // budget, so it gets a bound rather than a print.
    //
    // The competition allows 24 tiles of about 320 flip-flops each. A single
    // corpus design is not a chip — these are algorithm benchmarks, not the
    // submission — so the bound is per design, and it is loose enough to catch
    // a design that has become a register farm without failing a benchmark that
    // is not about area.
    const MAX_REGISTER_TILES: f64 = 24.0;
    for (name, _, cells) in &rows {
        assert!(
            cells.register_tiles() <= MAX_REGISTER_TILES,
            "{name} needs {} tiles of flip-flops, past the {MAX_REGISTER_TILES} the \
             whole die has. A design that cannot fit the die's register budget \
             cannot be submitted, whatever its benchmark says.",
            cells.register_tiles()
        );
    }
}

/// The spread of gate counts, which is the interesting comparison.
///
/// Two designs can have the same node count and very different gate counts — a
/// `case` with eight arms is a mux tree and an `add` is a whole adder — and the
/// ratio is what a floorplan cares about.
#[test]
#[ignore = "needs yosys; run with --ignored --nocapture"]
fn the_gate_count_spread_is_reported() {
    if !yosys_available() {
        println!("SKIPPED: yosys is not on the PATH, so no design was synthesised.");
        return;
    }

    let mut rows: Vec<(&str, &str, Cells)> = Vec::new();
    for (name, variant, build) in designs() {
        let verilog = emit(name, build);
        let cells = synthesise(name, &verilog);
        rows.push((name, variant, cells));
    }

    let mut ranked: Vec<(&str, &str, Cells)> = rows.clone();
    ranked.sort_by_key(|(_, _, cells)| std::cmp::Reverse(cells.gates));

    println!(
        "\nlargest designs by generic gate count\n{}",
        "-".repeat(52)
    );
    for (name, variant, cells) in ranked.iter().take(8) {
        println!(
            "{:<14} {:<22} gates {:>7}   flops {:>5}",
            name, variant, cells.gates, cells.flops
        );
    }

    let worst = ranked.first().expect("there is at least one design");
    let cheapest = ranked.last().expect("there is at least one design");
    println!(
        "\nlargest: {} {} at {} gates\nsmallest: {} {} at {} gates\n",
        worst.0, worst.1, worst.2.gates, cheapest.0, cheapest.1, cheapest.2.gates
    );

    // The spread is the claim: the biggest design costs several times the
    // smallest, which is what makes the choice of algorithm an area decision
    // rather than a detail.
    assert!(
        worst.2.gates > cheapest.2.gates * 4,
        "the largest design ({}, {} gates) is not several times the smallest \
         ({}), so the spread this file reports is not a real spread",
        worst.0,
        worst.2.gates,
        cheapest.0
    );
}

/// The per-kind breakdown, for one design.
///
/// The total says how much; the breakdown says why, and the "why" is where a
/// ROM-heavy design differs from a register-heavy one.
#[test]
#[ignore = "needs yosys; run with --ignored --nocapture"]
fn a_designs_cell_breakdown_is_reported() {
    if !yosys_available() {
        println!("SKIPPED: yosys is not on the PATH, so no design was synthesised.");
        return;
    }

    for (name, variant, build) in designs() {
        if name != "aes" && name != "crc3" {
            continue;
        }
        let verilog = emit(name, build);
        let dir = std::env::temp_dir().join("ferrite-lithic-synth");
        std::fs::create_dir_all(&dir).unwrap();
        let path: PathBuf = dir.join(format!("{name}.v"));
        std::fs::write(&path, &verilog).unwrap();
        let output = run_yosys(name, &path, &dir);
        let text = String::from_utf8_lossy(&output.stdout);
        println!("\n{variant} ({name}) cell kinds\n{}", "-".repeat(52));
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('$') {
                println!("  {trimmed}");
            }
        }
    }
}

/// The kinds of cell a design used, as a map.
///
/// Kept for a caller that wants the breakdown rather than the total.
#[must_use]
pub fn cell_kinds(verilog: &str, name: &str) -> BTreeMap<String, usize> {
    let dir = std::env::temp_dir().join("ferrite-lithic-synth");
    std::fs::create_dir_all(&dir).expect("a temp directory");
    let path: PathBuf = dir.join(format!("{name}.v"));
    std::fs::write(&path, verilog).expect("writing the design");
    let output = run_yosys(name, &path, &dir);
    stat_rows(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .map(|(kind, count)| (kind.to_string(), count))
        .collect()
}

/// The path a design's Verilog was written to, for a caller that wants to keep it.
#[must_use]
pub fn verilog_path(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join("ferrite-lithic-synth")
        .join(format!("{name}.v"))
}

/// Whether a path exists, for a caller checking a file was written.
#[must_use]
pub fn exists(path: &Path) -> bool {
    path.exists()
}
