//! The memory macros a process provides.

/// One port of a memory macro.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SramPort {
    /// A one-bit input.
    Flag(&'static str),
    /// A bus input.
    Bus(&'static str, u32),
    /// A one-bit output.
    Out(&'static str),
    /// A bus output.
    OutBus(&'static str, u32),
}

impl SramPort {
    /// The port's name, as the foundry spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Flag(name) | Self::Bus(name, _) | Self::Out(name) | Self::OutBus(name, _) => name,
        }
    }

    /// The port's width in bits.
    #[must_use]
    pub const fn width(self) -> u32 {
        match self {
            Self::Flag(_) | Self::Out(_) => 1,
            Self::Bus(_, width) | Self::OutBus(_, width) => width,
        }
    }

    /// Whether the macro drives this port.
    #[must_use]
    pub const fn is_output(self) -> bool {
        matches!(self, Self::Out(_) | Self::OutBus(_, _))
    }
}

/// A hard memory macro a process provides.
///
/// # Everything here was read out of the foundry's own files
///
/// The name is the cell name in the LEF. The area is the `SIZE` in the same
/// LEF, in micrometres. The port list is the port list in the Verilog
/// simulation model, including the `A_BIST_*` pins the built-in self test
/// needs and the `A_DLY` pin the model refuses to simulate unless it is tied
/// high.
#[derive(Clone, Copy, Debug)]
pub struct SramMacro {
    /// The macro's cell name, as the foundry spells it.
    pub module: &'static str,
    /// How many words the macro holds.
    pub depth: u32,
    /// How many bits each word has.
    pub width: u32,
    /// The macro's width in micrometres.
    pub width_um: f64,
    /// The macro's height in micrometres.
    pub height_um: f64,
    /// The macro's ports, in the order the simulation model declares them.
    pub ports: &'static [SramPort],
    /// How many clock cycles a read takes.
    ///
    /// One for every macro here. The read enable and the address are sampled
    /// at the clock edge and the data appears in the next cycle, which is what
    /// "synchronous" means for a memory and is why a design that reads an SRAM
    /// and a design that reads an array of flip-flops are not the same design
    /// unless the difference is written down.
    pub read_latency: u32,
    /// How many ports the macro has.
    pub ports_count: u32,
}

impl SramMacro {
    /// The macro's area in square micrometres.
    #[must_use]
    pub fn area_um2(&self) -> f64 {
        self.width_um * self.height_um
    }

    /// How many bits the macro holds.
    #[must_use]
    pub fn bits(&self) -> u64 {
        u64::from(self.depth) * u64::from(self.width)
    }

    /// How many bytes the macro holds.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        u64::from(self.depth) * u64::from(self.width) / 8
    }

    /// The address width the macro needs.
    #[must_use]
    pub const fn address_width(&self) -> u32 {
        // The IHP macros use a fixed address width per family, and for every
        // depth in the family the width is the ceiling of the base-2 log. A
        // depth of one still needs one bit, because a memory with a single word
        // still has an address input the graph has to carry.
        32 - self.depth.leading_zeros()
    }
}

/// The port list of a plain (no built-in self test) 16-bit macro.
pub(crate) const PLAIN_PORTS_16: &[SramPort] = &[
    SramPort::Flag("A_CLK"),
    SramPort::Flag("A_MEN"),
    SramPort::Flag("A_WEN"),
    SramPort::Flag("A_REN"),
    SramPort::Bus("A_ADDR", 6),
    SramPort::Bus("A_DIN", 16),
    SramPort::Flag("A_DLY"),
    SramPort::OutBus("A_DOUT", 16),
];

/// The port list of a plain (no built-in self test) 32-bit macro.
pub(crate) const PLAIN_PORTS_32: &[SramPort] = &[
    SramPort::Flag("A_CLK"),
    SramPort::Flag("A_MEN"),
    SramPort::Flag("A_WEN"),
    SramPort::Flag("A_REN"),
    SramPort::Bus("A_ADDR", 13),
    SramPort::Bus("A_DIN", 32),
    SramPort::Flag("A_DLY"),
    SramPort::OutBus("A_DOUT", 32),
];

/// The port list of a built-in-self-test 8-bit macro.
pub(crate) const BIST_PORTS_8: &[SramPort] = &[
    SramPort::Flag("A_CLK"),
    SramPort::Flag("A_MEN"),
    SramPort::Flag("A_WEN"),
    SramPort::Flag("A_REN"),
    SramPort::Bus("A_ADDR", 8),
    SramPort::Bus("A_DIN", 8),
    SramPort::Flag("A_DLY"),
    SramPort::OutBus("A_DOUT", 8),
    SramPort::Bus("A_BM", 8),
    SramPort::Flag("A_BIST_CLK"),
    SramPort::Flag("A_BIST_EN"),
    SramPort::Flag("A_BIST_MEN"),
    SramPort::Flag("A_BIST_WEN"),
    SramPort::Flag("A_BIST_REN"),
    SramPort::Bus("A_BIST_ADDR", 8),
    SramPort::Bus("A_BIST_DIN", 8),
    SramPort::Bus("A_BIST_BM", 8),
];

/// The port list of a built-in-self-test 16-bit macro.
pub(crate) const BIST_PORTS_16: &[SramPort] = &[
    SramPort::Flag("A_CLK"),
    SramPort::Flag("A_MEN"),
    SramPort::Flag("A_WEN"),
    SramPort::Flag("A_REN"),
    SramPort::Bus("A_ADDR", 10),
    SramPort::Bus("A_DIN", 16),
    SramPort::Flag("A_DLY"),
    SramPort::OutBus("A_DOUT", 16),
    SramPort::Bus("A_BM", 16),
    SramPort::Flag("A_BIST_CLK"),
    SramPort::Flag("A_BIST_EN"),
    SramPort::Flag("A_BIST_MEN"),
    SramPort::Flag("A_BIST_WEN"),
    SramPort::Flag("A_BIST_REN"),
    SramPort::Bus("A_BIST_ADDR", 10),
    SramPort::Bus("A_BIST_DIN", 16),
    SramPort::Bus("A_BIST_BM", 16),
];

/// The port list of a built-in-self-test 32-bit macro.
pub(crate) const BIST_PORTS_32: &[SramPort] = &[
    SramPort::Flag("A_CLK"),
    SramPort::Flag("A_MEN"),
    SramPort::Flag("A_WEN"),
    SramPort::Flag("A_REN"),
    SramPort::Bus("A_ADDR", 10),
    SramPort::Bus("A_DIN", 32),
    SramPort::Flag("A_DLY"),
    SramPort::OutBus("A_DOUT", 32),
    SramPort::Bus("A_BM", 32),
    SramPort::Flag("A_BIST_CLK"),
    SramPort::Flag("A_BIST_EN"),
    SramPort::Flag("A_BIST_MEN"),
    SramPort::Flag("A_BIST_WEN"),
    SramPort::Flag("A_BIST_REN"),
    SramPort::Bus("A_BIST_ADDR", 10),
    SramPort::Bus("A_BIST_DIN", 32),
    SramPort::Bus("A_BIST_BM", 32),
];

/// The port list of a built-in-self-test 64-bit macro.
pub(crate) const BIST_PORTS_64: &[SramPort] = &[
    SramPort::Flag("A_CLK"),
    SramPort::Flag("A_MEN"),
    SramPort::Flag("A_WEN"),
    SramPort::Flag("A_REN"),
    SramPort::Bus("A_ADDR", 10),
    SramPort::Bus("A_DIN", 64),
    SramPort::Flag("A_DLY"),
    SramPort::OutBus("A_DOUT", 64),
    SramPort::Bus("A_BM", 64),
    SramPort::Flag("A_BIST_CLK"),
    SramPort::Flag("A_BIST_EN"),
    SramPort::Flag("A_BIST_MEN"),
    SramPort::Flag("A_BIST_WEN"),
    SramPort::Flag("A_BIST_REN"),
    SramPort::Bus("A_BIST_ADDR", 10),
    SramPort::Bus("A_BIST_DIN", 64),
    SramPort::Bus("A_BIST_BM", 64),
];
