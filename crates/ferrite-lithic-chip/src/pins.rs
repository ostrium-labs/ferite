//! The pin budget, and the map from a design's ports to it.
//!
//! # The budget is not negotiable
//!
//! A Tiny Tapeout design's top module has a **fixed** port list. There are
//! eight dedicated inputs, eight dedicated outputs, eight bidirectional pins, a
//! clock, a reset and a power enable, and that is all there is: the pads are
//! cut in the die before the design is written. The rules say so about the
//! pinout section of `info.yaml` — *"Leave unused pins blank. DO NOT delete or
//! add any pins."*
//!
//! So the budget is not "how many pins may I use" but "which of these twenty-
//! four user pins does each of my signals get", and the check that matters is
//! **the design does not want more than the die has**. A design with nine
//! dedicated inputs cannot be built, and the failure mode of finding that out
//! at the end is a design that does not fit.
//!
//! # Why the count is checked rather than trusted
//!
//! Because the check is cheap and the alternative is a tapeout. ADR-0017 of
//! this repository says it plainly: *"I/O count is a checked constraint with a
//! static error, not a runtime surprise. For the competition chip this is
//! load-bearing: 26 pins means a wide SIMD datapath must serialise or
//! time-multiplex across `uio[7:0]`."*
//!
//! # One bit per pin
//!
//! Every user pin is one bit, because a pad is one bit. A design that wants an
//! eight-bit bus across `ui_in[7:0]` declares eight pins, or declares one
//! eight-bit core port and wires it up itself inside the core. The chip crate
//! does not try to be clever about this: the mapping is one signal, one bit,
//! one pad, and the wrapper it generates says so in the text.

use std::fmt;

use ferrite_lithic_rtl::Module;

/// How many dedicated input pins the die has.
pub const DEDICATED_INPUTS: usize = 8;
/// How many dedicated output pins the die has.
pub const DEDICATED_OUTPUTS: usize = 8;
/// How many bidirectional pins the die has.
pub const BIDIRECTIONAL_PINS: usize = 8;

/// How many user pins the die has in total.
///
/// Eight plus eight plus eight. The clock, the reset and the power enable are
/// the die's, not the design's.
pub const USER_PINS: usize = DEDICATED_INPUTS + DEDICATED_OUTPUTS + BIDIRECTIONAL_PINS;

/// How many pins the die has in total.
///
/// Twenty-six: twenty-four user pins, plus the clock, the active-low reset and
/// the power enable.
pub const TOTAL_PINS: usize = USER_PINS + 3;

/// A dedicated input pin.
///
/// It drives a core module's input port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputPin {
    /// The pin's name, for the `info.yaml` pinout.
    pub name: String,
    /// The core module's input port this pin drives.
    pub port: String,
}

/// A dedicated output pin.
///
/// It is driven by a core module's output port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputPin {
    /// The pin's name, for the `info.yaml` pinout.
    pub name: String,
    /// The core module's output port that drives this pin.
    pub port: String,
}

/// A bidirectional pin.
///
/// A bidirectional pad is three signals, not one: the value the design reads
/// from the pad, the value the design wants to drive onto it, and a bit saying
/// whether to drive it at all. Tiny Tapeout spells those `uio_in`, `uio_out`
/// and `uio_oe`, and the `oe` is active high: one means output.
///
/// **The enable is not optional and it is not decoration.** It is how a pin
/// becomes an input or an output at run time, which is the whole reason a
/// protocol emulator has bidirectional pins at all: an I2C master has to
/// release the clock line and let the slave pull it low, and only a
/// tri-state pad can do that.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BidirectionalPin {
    /// The pin's name, for the `info.yaml` pinout.
    pub name: String,
    /// The core module's input port that reads the pad.
    pub input_port: String,
    /// The core module's output port that holds the value to drive.
    pub output_port: String,
    /// The core module's output port that says whether to drive.
    pub enable_port: String,
}

/// Which of the three user-pin groups a design signal wants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PinKind {
    /// A dedicated input.
    Input,
    /// A dedicated output.
    Output,
    /// A bidirectional pin.
    Bidirectional,
}

/// A map from a design's ports to the die's pads.
///
/// Built with the [`PinMap::builder`], and checked when it is finished: the
/// number of pins of each kind has to fit the die.
#[derive(Clone, Debug, Default)]
pub struct PinMap {
    inputs: Vec<InputPin>,
    outputs: Vec<OutputPin>,
    bidirectional: Vec<BidirectionalPin>,
}

impl PinMap {
    /// An empty map.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inputs: Vec::new(),
            outputs: Vec::new(),
            bidirectional: Vec::new(),
        }
    }

    /// A builder.
    #[must_use]
    pub fn builder() -> PinMapBuilder {
        PinMapBuilder::new()
    }

    /// The dedicated input pins, in order.
    #[must_use]
    pub fn inputs(&self) -> &[InputPin] {
        &self.inputs
    }

    /// The dedicated output pins, in order.
    #[must_use]
    pub fn outputs(&self) -> &[OutputPin] {
        &self.outputs
    }

    /// The bidirectional pins, in order.
    #[must_use]
    pub fn bidirectional(&self) -> &[BidirectionalPin] {
        &self.bidirectional
    }

    /// How many user pins the design wants.
    #[must_use]
    pub fn used(&self) -> usize {
        self.inputs.len() + self.outputs.len() + self.bidirectional.len()
    }

    /// How many user pins are left over.
    #[must_use]
    pub fn spare(&self) -> usize {
        USER_PINS.saturating_sub(self.used())
    }

    /// Check the map against the die.
    ///
    /// # Errors
    ///
    /// [`Error::TooManyInputs`], [`Error::TooManyOutputs`] or
    /// [`Error::TooManyBidirectional`] if the design wants more of a kind than
    /// the die has, and [`Error::DuplicatePinName`] if two pins share a name.
    pub fn check(&self) -> Result<(), Error> {
        if self.inputs.len() > DEDICATED_INPUTS {
            return Err(Error::TooManyInputs {
                wanted: self.inputs.len(),
                have: DEDICATED_INPUTS,
            });
        }
        if self.outputs.len() > DEDICATED_OUTPUTS {
            return Err(Error::TooManyOutputs {
                wanted: self.outputs.len(),
                have: DEDICATED_OUTPUTS,
            });
        }
        if self.bidirectional.len() > BIDIRECTIONAL_PINS {
            return Err(Error::TooManyBidirectional {
                wanted: self.bidirectional.len(),
                have: BIDIRECTIONAL_PINS,
            });
        }
        let mut names: Vec<&str> = Vec::with_capacity(self.used());
        names.extend(self.inputs.iter().map(|p| p.name.as_str()));
        names.extend(self.outputs.iter().map(|p| p.name.as_str()));
        names.extend(self.bidirectional.iter().map(|p| p.name.as_str()));
        names.sort_unstable();
        for pair in names.windows(2) {
            if pair[0] == pair[1] {
                return Err(Error::DuplicatePinName {
                    name: pair[0].to_string(),
                });
            }
        }
        Ok(())
    }
}

/// A builder for a [`PinMap`].
#[derive(Clone, Debug, Default)]
pub struct PinMapBuilder {
    map: PinMap,
}

impl PinMapBuilder {
    /// A builder with no pins in it.
    #[must_use]
    pub const fn new() -> Self {
        Self { map: PinMap::new() }
    }

    /// A dedicated input pin: the pad drives the design.
    #[must_use]
    pub fn input(mut self, name: impl Into<String>, port: impl Into<String>) -> Self {
        self.map.inputs.push(InputPin {
            name: name.into(),
            port: port.into(),
        });
        self
    }

    /// A dedicated output pin: the design drives the pad.
    #[must_use]
    pub fn output(mut self, name: impl Into<String>, port: impl Into<String>) -> Self {
        self.map.outputs.push(OutputPin {
            name: name.into(),
            port: port.into(),
        });
        self
    }

    /// A bidirectional pin.
    #[must_use]
    pub fn bidirectional(
        mut self,
        name: impl Into<String>,
        input_port: impl Into<String>,
        output_port: impl Into<String>,
        enable_port: impl Into<String>,
    ) -> Self {
        self.map.bidirectional.push(BidirectionalPin {
            name: name.into(),
            input_port: input_port.into(),
            output_port: output_port.into(),
            enable_port: enable_port.into(),
        });
        self
    }

    /// Finish, and check the result.
    ///
    /// # Errors
    ///
    /// Whatever [`PinMap::check`] reports.
    pub fn build(self) -> Result<PinMap, Error> {
        self.map.check()?;
        Ok(self.map)
    }
}

/// Why a pin map does not fit.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// More dedicated inputs than the die has.
    TooManyInputs {
        /// How many the design wants.
        wanted: usize,
        /// How many the die has.
        have: usize,
    },
    /// More dedicated outputs than the die has.
    TooManyOutputs {
        /// How many the design wants.
        wanted: usize,
        /// How many the die has.
        have: usize,
    },
    /// More bidirectional pins than the die has.
    TooManyBidirectional {
        /// How many the design wants.
        wanted: usize,
        /// How many the die has.
        have: usize,
    },
    /// Two pins have the same name.
    DuplicatePinName {
        /// The name that appears twice.
        name: String,
    },
    /// A pin names a core port the module does not have.
    UnknownPort {
        /// The port the pin names.
        port: String,
        /// The ports the module does have.
        available: Vec<String>,
    },
    /// A pin names a core port of the wrong width.
    PortWidth {
        /// The port the pin names.
        port: String,
        /// The width the pin needs.
        expected: u32,
        /// The width the port has.
        found: u32,
    },
    /// The module has no clock, and the wrapper has to drive one.
    NoClock,
    /// The core module could not be emitted.
    Core(String),
}

impl From<ferrite_lithic_rtl::Error> for Error {
    fn from(inner: ferrite_lithic_rtl::Error) -> Self {
        Self::Core(inner.to_string())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyInputs { wanted, have } => write!(
                f,
                "the design wants {wanted} dedicated input pins but the die has {have}. \
                 The pads are cut before the design is written, so this cannot be \
                 negotiated: move a signal to a bidirectional pin, or pack several \
                 signals into one pin and unpack them inside the design."
            ),
            Self::TooManyOutputs { wanted, have } => write!(
                f,
                "the design wants {wanted} dedicated output pins but the die has {have}. \
                 The pads are cut before the design is written, so this cannot be \
                 negotiated: move a signal to a bidirectional pin, or pack several \
                 signals into one pin and unpack them inside the design."
            ),
            Self::TooManyBidirectional { wanted, have } => write!(
                f,
                "the design wants {wanted} bidirectional pins but the die has {have}. \
                 The pads are cut before the design is written, so this cannot be \
                 negotiated: a bidirectional pin can carry an input and an output at \
                 once, which is often what a protocol pin needs."
            ),
            Self::DuplicatePinName { name } => write!(
                f,
                "two pins are named `{name}`. The name appears in the `info.yaml` \
                 pinout, which is what the datasheet and the shuttle documentation \
                 are generated from, so two pins with one name would publish one \
                 signal twice."
            ),
            Self::UnknownPort { port, available } => {
                let mut list = String::new();
                for (index, name) in available.iter().enumerate() {
                    if index > 0 {
                        list.push_str(", ");
                    }
                    list.push_str(name);
                }
                write!(
                    f,
                    "the pin map names the port `{port}`, which the core module does \
                     not have. The module's ports are: {list}. A pin is bound to a \
                     port by name, so the name has to be the module's own."
                )
            }
            Self::PortWidth { port, found, .. } => write!(
                f,
                "the port `{port}` is {found} bits wide but a pin carries one bit. \
                 A pad is one bit: either give the port a one-bit name, or unpack \
                 the bus inside the design and bring the pieces out on separate \
                 pins."
            ),
            Self::Core(inner) => write!(f, "the core module could not be read: {inner}"),
            Self::NoClock => write!(
                f,
                "the core module has no clock, but the Tiny Tapeout wrapper has to \
                 drive one: the die's clock pin goes to the module's clock port. \
                 A design with state needs a clock."
            ),
        }
    }
}

impl std::error::Error for Error {}

/// The core module's port names, with their widths.
pub(crate) fn port_table(module: &Module) -> Result<Vec<(String, u32)>, Error> {
    let mut out = Vec::new();
    for id in module.inputs().iter().chain(module.outputs().iter()) {
        let Some(name) = module.circuit().name(*id) else {
            continue;
        };
        out.push((name.to_string(), module.circuit().width_of(*id)));
    }
    Ok(out)
}
