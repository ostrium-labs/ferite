//! The Tiny Tapeout submission boundary.
//!
//! # What this crate is for
//!
//! A Tiny Tapeout submission is not a Verilog file. It is a **directory**: a
//! top module with a fixed port list, an `info.yaml` describing the pinout,
//! and a documentation page. The port list is fixed because the pads are cut
//! before the design is written, and the module name is fixed by convention
//! because the shuttle documentation indexes the die by it.
//!
//! Getting any of that wrong is the most common submission failure, and it is
//! a *mechanical* failure: not a design bug, not a timing bug, a shape
//! mismatch. ADR-0017 of this repository says the wrapper *"module name and
//! its exact port list are generated automatically by the emitter"*, and this
//! is that generator, on the other side of the emitter.
//!
//! # The three things it does
//!
//! 1. **The wrapper.** [`wrapper`] emits `tt_um_<user>_<project>` with the
//!    exact port list the die has, inverts the die's active-low reset into the
//!    DSL's active-high synchronous one, turns a bidirectional pad into the
//!    three signals it really is, and ties off every pad the design does not
//!    use.
//! 2. **The pin budget.** [`PinMap`] checks the design against the die's fixed
//!    twenty-four user pins. A design that wants twenty-five does not fit, and
//!    the check is a static error rather than a surprise at the end.
//! 3. **The project.** [`project`] and [`write_project`] produce every file a
//!    submission is made of.
//!
//! # The SRAM, which is the actual hard part
//!
//! [`sram::macro_memory`] builds a memory on a foundry macro, with the
//! details that are easy to get wrong written down in one place: the read is
//! synchronous, `A_DLY` must be tied high, the built-in self test has eight
//! pins to tie off, and a single-port macro has one address for a design that
//! wants two.
//!
//! # An example
//!
//! ```
//! use ferrite_lithic::Design;
//! use ferrite_lithic_chip::{Chip, PinMap, project};
//! use ferrite_lithic_rtl::Module;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let design = Design::new();
//! let clk = design.input("clk", 1)?;
//! let rst = design.input("rst", 1)?;
//! let rx = design.input("rx", 1)?;
//! let tx = design.output("tx", 1, &rx)?;
//!
//! let core = Module::new(
//!     "core",
//!     design.build()?,
//!     clk.id(),
//!     design.input_ports().iter().map(|s| s.id()).collect(),
//!     design.output_ports().iter().map(|s| s.id()).collect(),
//! );
//!
//! let pins = PinMap::builder()
//!     .input("rx", "rx")
//!     .output("tx", "tx")
//!     .build()?;
//!
//! let chip = Chip {
//!     user: "ostrium-labs".to_string(),
//!     project: "ferrite".to_string(),
//!     title: "Ferrite protocol emulator".to_string(),
//!     author: "Ostrium Labs".to_string(),
//!     description: "A programmable protocol emulator".to_string(),
//!     clock_hz: 10_000_000,
//!     tiles: (6, 4),
//!     reset_port: "rst".to_string(),
//! };
//!
//! let files = project(&chip, &core, &pins, &["project.v", "core.v"])?;
//! assert!(files.top.contains("module tt_um_ostrium_labs_ferrite ("));
//! assert!(files.top.contains("wire reset = ~rst_n;"));
//! # Ok(())
//! # }
//! ```

#![doc(html_root_url = "https://docs.rs/ferrite-lithic-chip/0.0.1")]
#![warn(missing_docs)]

mod chip;
mod pins;
pub mod sram;
mod wrapper;

pub use chip::{BuildError, Chip, Project, docs_info, info_yaml, project, write_project};
pub use pins::{
    BIDIRECTIONAL_PINS, BidirectionalPin, DEDICATED_INPUTS, DEDICATED_OUTPUTS, Error, InputPin,
    OutputPin, PinKind, PinMap, PinMapBuilder, TOTAL_PINS, USER_PINS,
};
pub use sram::{MacroMemory, MacroPorts, macro_memory};
pub use wrapper::{pinout, top_module_name, wrapper};
