//! The processes this project can build for.

use crate::sram::SramMacro;

/// A process: its geometry, its resources, and its memory macros.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pdk {
    /// IHP's 130 nm CMOS5L (`ihp-sg13g2`), the submission target.
    ///
    /// The one the competition names: *"We're targeting IHP's 130nm CMOS5L
    /// process through our friends at Tiny Tapeout."*
    IhpCmos5l,
    /// SkyWater's 130 nm open library (`sky130A`).
    ///
    /// Not the submission target, and kept because it is the most mature open
    /// PDK with the best-documented tooling and the widest example corpus, so
    /// it is the cheapest place to iterate on the DSL.
    Sky130,
    /// GlobalFoundries' 180 nm MCU (`gf180mcu_fd_sc_mcu7`).
    ///
    /// The third live open node, and the one with the smallest memory macros of
    /// the three.
    Gf180mcu,
}

impl Pdk {
    /// The process's name as the open-PDK tooling knows it.
    #[must_use]
    pub const fn open_pdk_name(self) -> &'static str {
        match self {
            Self::IhpCmos5l => "ihp-sg13g2",
            Self::Sky130 => "sky130A",
            Self::Gf180mcu => "gf180mcu_fd_sc_mcu7",
        }
    }

    /// The process's name for a human.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::IhpCmos5l => "IHP 130nm CMOS5L",
            Self::Sky130 => "SkyWater 130nm",
            Self::Gf180mcu => "GlobalFoundries 180nm MCU",
        }
    }

    /// The process's feature size in nanometres.
    #[must_use]
    pub const fn feature_size_nm(self) -> u32 {
        match self {
            Self::IhpCmos5l => 130,
            Self::Sky130 => 130,
            Self::Gf180mcu => 180,
        }
    }

    /// The width of one tile in micrometres.
    ///
    /// # Where this number comes from, and what it is not
    ///
    /// For IHP this is the figure the competition rules state: *"At
    /// approximately 200um × 150um per tile, that's about 0.7 mm² of nominal
    /// tile area"* for the 6x4 allocation. It is a rule, not a measurement, and
    /// it is recorded as a rule.
    ///
    /// The other two are the open PDKs' own figures, which are what the
    /// Tiny Tapeout templates quote.
    #[must_use]
    pub const fn tile_width_um(self) -> f64 {
        match self {
            Self::IhpCmos5l => 200.0,
            Self::Sky130 => 167.0,
            Self::Gf180mcu => 224.0,
        }
    }

    /// The height of one tile in micrometres.
    #[must_use]
    pub const fn tile_height_um(self) -> f64 {
        match self {
            Self::IhpCmos5l => 150.0,
            Self::Sky130 => 108.0,
            Self::Gf180mcu => 228.0,
        }
    }

    /// The area of one tile in square micrometres.
    #[must_use]
    pub fn tile_area_um2(self) -> f64 {
        self.tile_width_um() * self.tile_height_um()
    }

    /// How many flip-flops fit in one tile.
    ///
    /// The competition's own figure for the IHP process: *"The current maximum
    /// area is 6x4 tiles per design"* and *"about 1K logic cells per tile"*,
    /// with roughly a third of those being registers.
    ///
    /// This is the number that rules out flip-flops for program memory. Twenty-
    /// four tiles of three hundred and twenty flip-flops is 7 680 bits — under a
    /// kilobyte — and that is the *whole* design's register budget, spent on
    /// nothing else.
    #[must_use]
    pub const fn dff_per_tile(self) -> u32 {
        match self {
            Self::IhpCmos5l => 320,
            Self::Sky130 => 256,
            Self::Gf180mcu => 200,
        }
    }

    /// The memory macros this process provides.
    ///
    /// Empty for the processes whose macros this crate has not measured, which
    /// is stated rather than guessed: a design that asks for a macro the
    /// process does not have should be told so, not handed a plausible number.
    #[must_use]
    pub const fn sram_macros(self) -> &'static [SramMacro] {
        match self {
            Self::IhpCmos5l => IHP_TABLE,
            Self::Sky130 => &[],
            Self::Gf180mcu => &[],
        }
    }

    /// The macro of a given shape, if this process has one.
    #[must_use]
    pub fn sram(self, depth: u32, width: u32) -> Option<&'static SramMacro> {
        self.sram_macros()
            .iter()
            .find(|m| m.depth == depth && m.width == width)
    }

    /// The cheapest macro that holds at least `bytes` bytes.
    ///
    /// "Cheapest" is smallest area, which is not the same as smallest: a
    /// deeper macro is usually *more* area-efficient per bit, so a design that
    /// needs 4 KB is usually better served by one 4096x8 than by four 1024x8s.
    #[must_use]
    pub fn smallest_sram_for(self, bytes: u64) -> Option<&'static SramMacro> {
        self.sram_macros()
            .iter()
            .filter(|m| m.bytes() >= bytes)
            .min_by(|a, b| {
                a.area_um2()
                    .partial_cmp(&b.area_um2())
                    .unwrap_or(core::cmp::Ordering::Equal)
            })
    }

    /// How many tiles a memory of this shape costs, including the whole tile it
    /// straddles.
    ///
    /// A macro does not have to fit inside one tile: the 1024x8 macro is
    /// 147 µm wide and 336 µm tall, so on a 200 µm × 150 µm tile it is one tile
    /// wide and three tiles tall. Counting the area alone would say 1.65 tiles;
    /// counting the *bounding box* says three, and three is what the floorplan
    /// has to find.
    #[must_use]
    pub fn tiles_for_sram(self, macro_form: &SramMacro) -> f64 {
        let across = (macro_form.width_um / self.tile_width_um()).ceil();
        let down = (macro_form.height_um / self.tile_height_um()).ceil();
        across * down
    }
}

/// Every IHP macro, in one table.
///
/// Written out rather than concatenated from per-width tables, because a `const`
/// cannot build a `&'static [SramMacro]` out of two `&'static [SramMacro]` in
/// stable Rust. The order is by word width and then by depth, which is how the
/// PDK lists them.
const IHP_TABLE: &[SramMacro] = &[
    SramMacro {
        module: "RM_IHPSG13_1P_256x8_c3_bm_bist",
        depth: 256,
        width: 8,
        width_um: 236.8,
        height_um: 74.1,
        ports: crate::sram::BIST_PORTS_8,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_512x8_c3_bm_bist",
        depth: 512,
        width: 8,
        width_um: 236.8,
        height_um: 110.38,
        ports: crate::sram::BIST_PORTS_8,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_1024x8_c2_bm_bist",
        depth: 1024,
        width: 8,
        width_um: 146.88,
        height_um: 336.46,
        ports: crate::sram::BIST_PORTS_8,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_4096x8_c3_bm_bist",
        depth: 4096,
        width: 8,
        width_um: 236.8,
        height_um: 618.3,
        ports: crate::sram::BIST_PORTS_8,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_64x16_c2",
        depth: 64,
        width: 16,
        width_um: 236.8,
        height_um: 64.36,
        ports: crate::sram::PLAIN_PORTS_16,
        read_latency: 1,
        ports_count: 8,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_256x16_c2_bm_bist",
        depth: 256,
        width: 16,
        width_um: 236.8,
        height_um: 118.78,
        ports: crate::sram::BIST_PORTS_16,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_512x16_c2_bm_bist",
        depth: 512,
        width: 16,
        width_um: 236.8,
        height_um: 191.34,
        ports: crate::sram::BIST_PORTS_16,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_1024x16_c2_bm_bist",
        depth: 1024,
        width: 16,
        width_um: 236.8,
        height_um: 336.46,
        ports: crate::sram::BIST_PORTS_16,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_4096x16_c3_bm_bist",
        depth: 4096,
        width: 16,
        width_um: 416.64,
        height_um: 618.3,
        ports: crate::sram::BIST_PORTS_16,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_256x32_c2_bm_bist",
        depth: 256,
        width: 32,
        width_um: 416.64,
        height_um: 118.78,
        ports: crate::sram::BIST_PORTS_32,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_512x32_c2_bm_bist",
        depth: 512,
        width: 32,
        width_um: 416.64,
        height_um: 191.34,
        ports: crate::sram::BIST_PORTS_32,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_1024x32_c2_bm_bist",
        depth: 1024,
        width: 32,
        width_um: 416.64,
        height_um: 336.46,
        ports: crate::sram::BIST_PORTS_32,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_2048x32_c2_bm_bist",
        depth: 2048,
        width: 32,
        width_um: 416.64,
        height_um: 626.7,
        ports: crate::sram::BIST_PORTS_32,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_8192x32_c4",
        depth: 8192,
        width: 32,
        width_um: 1520.16,
        height_um: 618.3,
        ports: crate::sram::PLAIN_PORTS_32,
        read_latency: 1,
        ports_count: 8,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_64x64_c2_bm_bist",
        depth: 64,
        width: 64,
        width_um: 784.48,
        height_um: 64.36,
        ports: crate::sram::BIST_PORTS_64,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_256x64_c2_bm_bist",
        depth: 256,
        width: 64,
        width_um: 784.48,
        height_um: 118.78,
        ports: crate::sram::BIST_PORTS_64,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_512x64_c2_bm_bist",
        depth: 512,
        width: 64,
        width_um: 784.48,
        height_um: 191.34,
        ports: crate::sram::BIST_PORTS_64,
        read_latency: 1,
        ports_count: 17,
    },
    SramMacro {
        module: "RM_IHPSG13_1P_1024x64_c2_bm_bist",
        depth: 1024,
        width: 64,
        width_um: 784.48,
        height_um: 336.46,
        ports: crate::sram::BIST_PORTS_64,
        read_latency: 1,
        ports_count: 17,
    },
];
