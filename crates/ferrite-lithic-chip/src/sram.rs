//! Building a memory out of a foundry macro.
//!
//! # The hard part is not the instantiation
//!
//! Instantiating an IHP SRAM macro is three lines of Verilog. What is not
//! three lines is everything around it:
//!
//! - **The read is synchronous.** The macro samples the read enable and the
//!   address at the clock edge and the data appears in the *next* cycle. An
//!   array read is combinational. A design that reads an SRAM and a design
//!   that reads an array are different circuits unless the difference is
//!   written down, so the memory carries its read latency and both backends
//!   honour it.
//! - **`A_DLY` must be tied high.** The foundry's own simulation model
//!   refuses to run otherwise: it prints an error and stops. Not a warning —
//!   a `$stop`.
//! - **`A_MEN` is a master enable.** Holding it low puts the macro in its
//!   reset state.
//! - **The built-in self test has eight pins** that have to be tied to
//!   something: `A_BIST_CLK`, `A_BIST_EN`, `A_BIST_MEN`, `A_BIST_WEN`,
//!   `A_BIST_REN`, `A_BIST_ADDR`, `A_BIST_DIN` and `A_BIST_BM`.
//! - **There is one address port, and the design wants two.** A single-port
//!   macro cannot read and write at the same time, so the two addresses are
//!   muxed: the write address wins when the write is enabled.
//!
//! All of that is here so that a design does not have to remember it, and so
//! that the details are in one place instead of in a Verilog file nobody reads.

use ferrite_lithic::{Design, Signal};
use ferrite_lithic_ir::{InstancePort, MemMacro};
use ferrite_lithic_pdk::{Pdk, SramMacro, SramPort};

use crate::chip::BuildError;

/// A memory built out of a hard macro.
///
/// The result is a design-level memory: the same memory the simulator models,
/// with a macro form attached, so the emitter builds the macro and the
/// simulator still models the array.
#[derive(Clone, Debug)]
pub struct MacroMemory {
    /// The memory, in the design.
    pub memory: Signal,
    /// The data read from the macro, in the design.
    pub read: Signal,
}

/// The signals a macro-backed memory needs.
///
/// A single-port macro has one write port and one read port, and the two share
/// one address, so the design's two addresses are muxed. The rest is what the
/// macro's own pins need.
#[derive(Clone, Copy, Debug)]
pub struct MacroPorts<'a> {
    /// The data to write.
    pub write_data: &'a Signal,
    /// The address to write to.
    pub write_address: &'a Signal,
    /// The write enable.
    pub write_enable: &'a Signal,
    /// The address to read from.
    pub read_address: &'a Signal,
    /// The read enable.
    pub read_enable: &'a Signal,
    /// The clock.
    pub clock: &'a Signal,
    /// The macro's master enable.
    ///
    /// Holding it low puts the macro in its reset state. On a Tiny Tapeout die
    /// the natural choice is the inverted reset, so the macro is quiet while
    /// the die is in reset.
    pub master_enable: &'a Signal,
}

/// Build a memory on a foundry macro.
///
/// The memory is `depth` words of `width` bits, with one write port and one
/// read port, which is what a single-port macro provides.
///
/// # The read latency
///
/// The read data is available **one cycle after** the address is presented,
/// because that is what the macro does. The memory records that latency, and
/// the simulator honours it: the value a read port reports in cycle *n* is the
/// value the macro captured at the edge into cycle *n - 1*.
///
/// # Errors
///
/// [`BuildError::NoSuchMacro`] if the process has no macro of that shape, and
/// [`BuildError::Ir`] if the design rejects the memory.
pub fn macro_memory(
    design: &Design,
    pdk: Pdk,
    depth: u32,
    width: u32,
    ports: MacroPorts<'_>,
) -> Result<MacroMemory, BuildError> {
    let MacroPorts {
        write_data,
        write_address,
        write_enable,
        read_address,
        read_enable,
        clock,
        master_enable,
    } = ports;
    let macro_form = pdk.sram(depth, width).ok_or(BuildError::NoSuchMacro {
        pdk: pdk.open_pdk_name().to_string(),
        depth,
        width,
    })?;

    let memory = design.mem_with(width, depth, None, macro_form.read_latency)?;
    design.write_port(&memory, write_data, write_address, write_enable)?;
    let read = design.read_port(&memory, read_address, read_enable)?;

    let ports = macro_ports(
        design,
        macro_form,
        clock,
        master_enable,
        write_data,
        write_address,
        write_enable,
        read_address,
        read_enable,
        &read,
    )?;

    design.mem_as_macro(
        &memory,
        MemMacro {
            module: macro_form.module.to_string(),
            params: Vec::new(),
            ports,
            read_latency: macro_form.read_latency,
        },
    )?;

    Ok(MacroMemory { memory, read })
}

/// The macro's port connections.
///
/// Every port of the macro is connected, including the ones that are tied
/// off. A macro port left unconnected is a floating input, and the foundry's
/// own model checks one of them — `A_DLY` — and stops the simulation if it is
/// not one.
///
/// The muxed address is built in the *design* rather than in the emitter, so
/// the two backends see the same circuit.
#[allow(clippy::too_many_arguments)]
fn macro_ports(
    design: &Design,
    macro_form: &SramMacro,
    clock: &Signal,
    master_enable: &Signal,
    write_data: &Signal,
    write_address: &Signal,
    write_enable: &Signal,
    read_address: &Signal,
    read_enable: &Signal,
    read: &Signal,
) -> Result<Vec<InstancePort>, BuildError> {
    let address_width = macro_form.address_width();

    // One address for both ports of a single-port macro: the write address
    // wins when the write is enabled, and the read address otherwise.
    let wide_write = resize(design, write_address, address_width)?;
    let wide_read = resize(design, read_address, address_width)?;
    let address = design
        .ite(write_enable, &wide_write, &wide_read)
        .map_err(BuildError::Ir)?;

    let mut ports = Vec::new();
    for port in macro_form.ports {
        let entry = match *port {
            SramPort::Flag(name) => match name {
                "A_CLK" => InstancePort::input(name, clock.id(), 1),
                "A_MEN" => InstancePort::input(name, master_enable.id(), 1),
                "A_WEN" => InstancePort::input(name, write_enable.id(), 1),
                "A_REN" => InstancePort::input(name, read_enable.id(), 1),
                // The foundry's model stops the simulation if this is not one.
                "A_DLY" => InstancePort::input(name, design.constant(true).id(), 1),
                // The built-in self test is off: its clock is low, its enable is
                // low, and its address and data are zero.
                "A_BIST_CLK" | "A_BIST_EN" | "A_BIST_MEN" | "A_BIST_WEN" | "A_BIST_REN" => {
                    InstancePort::input(name, design.constant(false).id(), 1)
                }
                other => {
                    return Err(BuildError::UnknownMacroPort {
                        module: macro_form.module.to_string(),
                        port: other.to_string(),
                    });
                }
            },
            SramPort::Bus(name, width) => match name {
                "A_ADDR" => InstancePort::input(name, address.id(), width),
                "A_DIN" => InstancePort::input(name, write_data.id(), width),
                "A_BM" => InstancePort::input(name, design.ones(width)?.id(), width),
                "A_BIST_ADDR" | "A_BIST_DIN" | "A_BIST_BM" => {
                    InstancePort::input(name, design.zeros(width)?.id(), width)
                }
                other => {
                    return Err(BuildError::UnknownMacroPort {
                        module: macro_form.module.to_string(),
                        port: other.to_string(),
                    });
                }
            },
            SramPort::Out(name) => InstancePort::output(name, read.id(), 1),
            SramPort::OutBus(name, width) => InstancePort::output(name, read.id(), width),
        };
        ports.push(entry);
    }
    Ok(ports)
}

/// Widen or narrow a signal to a given width.
///
/// A zero-extension, because an address is unsigned and a macro that wants
/// more address bits than the design supplies needs zeros, not garbage.
fn resize(design: &Design, signal: &Signal, width: u32) -> Result<Signal, BuildError> {
    if signal.width() == width {
        return Ok(signal.clone());
    }
    if signal.width() < width {
        return design.zero_extend(signal, width).map_err(BuildError::Ir);
    }
    design.truncate(signal, width).map_err(BuildError::Ir)
}
