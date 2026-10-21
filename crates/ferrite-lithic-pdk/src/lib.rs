//! What a process is: its tiles, its flip-flops, and its memory macros.
//!
//! # Why this is a crate and not a table in the DSL
//!
//! The DSL crates are a faithful reimplementation of Hardcaml's model, and
//! Hardcaml knows nothing about foundries. What *is* process knowledge — how
//! big a tile is, how many flip-flops fit in one, whether the process has a
//! hard SRAM and what it is called — changes with every shuttle and is not the
//! DSL's business.
//!
//! So it lives here, on its own, with the facts written down and dated. A
//! design that targets IHP asks this crate what IHP has; a design that targets
//! sky130 asks the same question and gets a different answer. The DSL does not
//! change, and neither does the other process's data.
//!
//! # Every number here has a source
//!
//! The macro names, port lists and areas are read out of the IHP open PDK's own
//! Liberty and LEF files, not out of a datasheet summary or a blog post. The
//! tile geometry is the figure the competition rules state, and it is recorded
//! as *that* — the competition's own number — rather than as a fact about the
//! process, because those are not the same thing and only one of them can be
//! checked against a source we can point at.
//!
//! # What this crate is for
//!
//! Two questions, both of which have to be answered before a design is
//! submitted and neither of which the DSL can answer:
//!
//! 1. **Will it fit?** A memory of a given size costs a given area, and a
//!    design has a given number of tiles. [`Pdk::sram`] and [`SramMacro::area_um2`]
//!    turn "I need 4 KB of program memory" into "that is 4.9 of my 24 tiles".
//! 2. **How do I build it?** The macro's name and port list, so the emitter can
//!    instantiate it and the simulator can be told what it is.
//!
//! ```
//! use ferrite_lithic_pdk::Pdk;
//!
//! // One 4096x8 macro holds 4 KB and costs 4.9 of a 6x4 IHP allocation.
//! let ihp = Pdk::IhpCmos5l;
//! let macro_form = ihp.sram(4096, 8).expect("IHP has a 4096x8 macro");
//! assert_eq!(macro_form.depth, 4096);
//! assert_eq!(macro_form.read_latency, 1);
//! assert!(macro_form.area_um2() > 140_000.0);
//!
//! // And the whole allocation is 24 tiles of about 30 000 um².
//! assert!(ihp.tile_area_um2() > 25_000.0);
//! ```

#![doc(html_root_url = "https://docs.rs/ferrite-lithic-pdk/0.0.1")]
#![warn(missing_docs)]

mod pdk;
mod sram;

pub use pdk::Pdk;
pub use sram::{SramMacro, SramPort};
