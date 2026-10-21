//! The node kinds that make up a signal graph, and their widths.

use core::cell::Cell;

use ferrite_lithic_bits::Bits;

use crate::id::NodeId;

/// A memory write port: what to write, where, and under what enable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WritePort {
    /// The data to write, one word wide.
    pub data: NodeId,
    /// The address to write to.
    pub address: NodeId,
    /// The write enable. One bit; low means the port does nothing this cycle.
    pub enable: NodeId,
}

impl WritePort {
    /// The write port's signals, in a fixed order.
    #[must_use]
    pub fn signals(&self) -> [NodeId; 3] {
        [self.data, self.address, self.enable]
    }

    /// Rewrite every signal through `f`.
    pub(crate) fn map_signals(&mut self, mut f: impl FnMut(NodeId) -> NodeId) {
        self.data = f(self.data);
        self.address = f(self.address);
        self.enable = f(self.enable);
    }
}

/// Which way an instance's port points.
///
/// A port is an input when the design drives it and the module reads it, and an
/// output when the module drives it and the design reads it. The distinction is
/// not cosmetic: an input port's signal is an *operand* of the instance, and an
/// output port's signal is a *net the instance drives*.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortDirection {
    /// Driven from inside the design, read by the module.
    Input,
    /// Driven by the module, read inside the design.
    Output,
}

/// A named connection to one port of an instance.
///
/// # Why names, and why the A1 positional scheme is gone
///
/// The A1 scheme bound a submodule positionally: `.i0`, `.i1`, …, `.o0`, because
/// the graph had nowhere to put a port's *name*. That was a workaround for a
/// limit, not a design position, and it cannot express the thing the competition
/// chip actually needs: a blackbox. A hard SRAM macro has seventeen ports with
/// names the foundry chose (`A_CLK`, `A_MEN`, `A_WEN`, `A_REN`, `A_ADDR`,
/// `A_DIN`, `A_DLY`, `A_DOUT`, `A_BM`, and eight `A_BIST_*` pins), and a RISC-V
/// core has twenty-odd outputs. Neither can be bound positionally to something
/// this project did not write.
///
/// So an instance's ports are named, and the name is the connection. A port
/// bound by name is also checkable: the width recorded here is the width the
/// emitter will connect, and a macro whose `A_ADDR` is ten bits wide cannot be
/// silently bound to a four-bit address.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstancePort {
    /// The port's name, as the module spells it.
    pub name: String,
    /// Which way the port points.
    pub direction: PortDirection,
    /// The signal on this side of the connection.
    ///
    /// For an input, the signal the module reads. For an output, the net the
    /// module drives — which must be an undriven wire when the instance is
    /// built.
    pub signal: NodeId,
    /// The port's width in bits.
    pub width: u32,
}

impl InstancePort {
    /// A port the design drives and the module reads.
    #[must_use]
    pub fn input(name: impl Into<String>, signal: NodeId, width: u32) -> Self {
        Self {
            name: name.into(),
            direction: PortDirection::Input,
            signal,
            width,
        }
    }

    /// A port the module drives and the design reads.
    #[must_use]
    pub fn output(name: impl Into<String>, signal: NodeId, width: u32) -> Self {
        Self {
            name: name.into(),
            direction: PortDirection::Output,
            signal,
            width,
        }
    }
}

/// How to build a memory out of a hard macro instead of an array of flip-flops.
///
/// # Why this is a property of the memory and not of the emitter
///
/// A hard macro is not a different *circuit*; it is a different *implementation*
/// of the same circuit, chosen by the process. The simulator has to model the
/// memory, the emitter has to build it, and the two must agree about the read
/// latency, the enable behaviour and the contents — otherwise the design that
/// is simulated is not the design that is fabricated.
///
/// Putting the macro's shape on the memory node means the shape travels with
/// the memory, and the emitter's choice is a lowering rather than a second
/// description of the design.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemMacro {
    /// The macro's module name, as the foundry spells it.
    pub module: String,
    /// Compile-time parameters, as literals.
    pub params: Vec<(String, Bits)>,
    /// The macro's port connections.
    pub ports: Vec<InstancePort>,
    /// How many clock cycles the macro's read takes.
    ///
    /// The macro says so, and the memory says so, and they must agree: a
    /// simulator that says the data arrives in one cycle and a macro that says
    /// two would make the two backends disagree about the design rather than
    /// about the implementation.
    pub read_latency: u32,
}

/// One arm of a `case`: a set of match values and the value they select.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseArm {
    /// The values this arm matches. An arm matches if the scrutinee equals *any*
    /// of them, which is how a multi-way compare is expressed without a chain of
    /// two-way `ite`s.
    pub matches: Vec<Bits>,
    /// The value selected when one of `matches` hits.
    pub value: NodeId,
}

impl CaseArm {
    /// A single-value arm.
    #[must_use]
    pub fn new(value: Bits, result: NodeId) -> Self {
        Self {
            matches: vec![value],
            value: result,
        }
    }
}

/// One node in the signal graph.
///
/// # The three decisions that shape this enum
///
/// **A register stores no output value.** [`Node::Reg`] holds its input `d`; the
/// register's *output* is the node itself
/// (`kernel/signal__type_intf.ml:243-296` in Hardcaml). This is inherited
/// deliberately, and it buys the cheapest possible answer to "is this stateful?":
/// match on the constructor. There is no flag to keep in sync and no way for a
/// node to claim to be combinational while holding state.
///
/// **`Wire` is the only node with a mutable field.** Every other variant is built
/// bottom-up over finished children
/// (`kernel/signal__type.ml:197-237`), so there is no other possible back edge.
/// That single fact is what makes a feedback loop expressible at all in a
/// bottom-up IR, and it is why `Wire` holds a [`Cell`] rather than a plain
/// `Option`: the driver is filled in later, through a shared handle, without
/// borrowing the arena.
///
/// **Memories hold write ports; read ports are separate nodes.** Hardcaml's
/// `Multiport_mem` holds write ports while reads are distinct
/// `Mem_read_port` nodes (`kernel/signal__type_intf.ml:298-319`), so reads are
/// asynchronous by construction and a synchronous read is the user composing a
/// register with a read port. We keep this because it keeps "what is stateful"
/// answerable from the constructor alone: a read port is combinational, full
/// stop, and never a hidden source of state.
#[derive(Clone, Debug)]
pub enum Node {
    /// A literal. Its value is stored, so its width needs no derivation.
    Constant {
        /// The value, which also carries the width.
        value: Bits,
    },

    /// An undriven, then driven, signal of a known width.
    ///
    /// The only node that can be a feedback edge, and the only one that is
    /// mutable after construction.
    Wire {
        /// The width, in bits.
        width: u32,
        /// The driver, once one is attached.
        driver: Cell<Option<NodeId>>,
    },

    /// Bitwise complement. Result width equals the operand's.
    Not {
        /// The operand.
        arg: NodeId,
    },

    /// A contiguous run of bits: `len` bits starting at `offset`.
    ///
    /// There is deliberately **no shift node**. Hardcaml desugars all three shifts
    /// into `select` plus `cat` plus a constant (`kernel/comb.ml:913-947`), and we
    /// keep that: a shift node would make the simulator faster but would emit
    /// Verilog the golden fixtures do not show, and shifters are better left to the
    /// synthesiser. The shift amount is a compile-time `u32` here, never a signal —
    /// a *variable* shift is a chain of muxes (`kernel/comb.ml:973-982`).
    Select {
        /// The signal being sliced.
        value: NodeId,
        /// Index of the lowest bit taken. Bit 0 is the least significant.
        offset: u32,
        /// How many bits taken, counting up from `offset`. This is the result's
        /// width, so it is stored rather than derived.
        len: u32,
    },

    /// Bitwise AND. Requires equal widths.
    BitAnd {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Bitwise OR. Requires equal widths.
    BitOr {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Bitwise XOR. Requires equal widths.
    BitXor {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Truncating add. Requires equal widths; result is the left width.
    Add {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Truncating subtract. Requires equal widths; result is the left width.
    Sub {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Full-precision multiply. Widens to the sum of the operand widths and
    /// cannot overflow, inherited from `ferrite-lithic-bits`.
    Mul {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Unsigned divide. Requires equal widths; result is the left width.
    UDiv {
        /// Dividend.
        left: NodeId,
        /// Divisor.
        right: NodeId,
    },

    /// Signed divide. Requires equal widths; result is the left width. Overflow
    /// is refused at simulation time, not here, because the graph is legal.
    SDiv {
        /// Dividend.
        left: NodeId,
        /// Divisor.
        right: NodeId,
    },

    /// Unsigned remainder. Requires equal widths; result is the left width.
    URem {
        /// Dividend.
        left: NodeId,
        /// Divisor.
        right: NodeId,
    },

    /// Signed remainder. Requires equal widths; result is the left width. Cannot
    /// overflow, so unlike `SDiv` it answers zero for `MIN % -1`.
    SRem {
        /// Dividend.
        left: NodeId,
        /// Divisor.
        right: NodeId,
    },

    /// Equality. One bit out.
    Eq {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Unsigned less-than. One bit out.
    Ult {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Unsigned less-or-equal. One bit out.
    Ule {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Unsigned greater-than. One bit out.
    Ugt {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Unsigned greater-or-equal. One bit out.
    Uge {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Signed less-than. One bit out.
    Slt {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Signed less-or-equal. One bit out.
    Sle {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Signed greater-than. One bit out.
    Sgt {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Signed greater-or-equal. One bit out.
    Sge {
        /// Left operand.
        left: NodeId,
        /// Right operand.
        right: NodeId,
    },

    /// Concatenation, high bits first. Widens to the sum of the operand widths.
    Cat {
        /// The high part.
        high: NodeId,
        /// The low part.
        low: NodeId,
    },

    /// One value repeated `count` times. Widens to `width * count`.
    Replicate {
        /// The value to repeat.
        value: NodeId,
        /// How many copies.
        count: u32,
    },

    /// Two-way select. Result width is the `then` width.
    Ite {
        /// The condition. Exactly one bit.
        condition: NodeId,
        /// Value when the condition is high.
        then_value: NodeId,
        /// Value when the condition is low.
        otherwise: NodeId,
    },

    /// Multi-way select against literal match values, with a default.
    ///
    /// The match values are literals rather than signals. That is a real
    /// restriction inherited from the shape Verilog wants, and it is the reason
    /// `Deps::WithoutCaseMatches` exists as a distinct relation: Hardcaml drops
    /// the match-constant edges when a case has been lowered, and with literal
    /// matches those edges are empty here. See [`Deps`](crate::Deps).
    Case {
        /// The value being matched.
        scrutinee: NodeId,
        /// The arms, in priority order.
        arms: Vec<CaseArm>,
        /// The value when no arm matches.
        default: NodeId,
        /// Width of the arms and the default, stored so width is a single match.
        width: u32,
    },

    /// A register. Its output is the node itself; `d` is its input.
    Reg {
        /// The data input. Determines the register's width.
        data: NodeId,
        /// The clock. Exactly one bit.
        clock: NodeId,
        /// Synchronous reset. Exactly one bit; high clears the register.
        reset: NodeId,
        /// Synchronous clear, which gates the update. Exactly one bit.
        clear: NodeId,
    },

    /// A memory. Holds its write ports; reads are separate read-port nodes.
    ///
    /// # A memory has three more properties than its shape
    ///
    /// The A1 memory was a shape and nothing else: a width, a depth and some
    /// write ports, starting as all zeros. Three things the competition chip
    /// needs are not shape.
    ///
    /// - **Contents.** A firmware image is not zeros, and neither is a protocol
    ///   lookup table. [`initial`](Self::Mem) holds them. The simulator seeds
    ///   the memory with them and the emitter writes them into the array, so
    ///   both backends start from the same state.
    /// - **Read latency.** An array read is combinational; a hard SRAM macro's
    ///   output register is not. [`read_latency`](Self::Mem) says how many cycles
    ///   a read takes, and it is the *memory's* property rather than the
    ///   backend's, because a design that reads an SRAM and a design that reads
    ///   an array must be the same design.
    /// - **A macro form.** [`macro_form`](Self::Mem) says how to build this
    ///   memory out of a foundry macro instead of flip-flops, when the target
    ///   process has one.
    Mem {
        /// Word width, in bits.
        data_width: u32,
        /// Number of words.
        depth: u32,
        /// The write ports.
        write_ports: Vec<WritePort>,
        /// The contents the memory starts with, in address order.
        ///
        /// `None` means "starts as all zeros", which is what a memory with no
        /// image has always meant. `Some` with fewer words than the depth is
        /// allowed: the words not listed are zero.
        initial: Option<Vec<Bits>>,
        /// How many clock cycles a read takes.
        ///
        /// Zero is an array read: the data is available in the same cycle as
        /// the address. One is a synchronous read: the data is captured at the
        /// clock edge and available in the next cycle, which is what every hard
        /// SRAM macro on the competition's process does.
        read_latency: u32,
        /// How to build this memory out of a hard macro, if the target has one.
        macro_form: Option<MemMacro>,
    },

    /// An asynchronous read port. Combinational: it holds no state.
    ReadPort {
        /// The memory being read.
        mem: NodeId,
        /// The word address.
        address: NodeId,
        /// The read enable. Exactly one bit; low forces the data output to zero.
        enable: NodeId,
        /// The memory's word width, stored so width is a single match.
        data_width: u32,
    },

    /// An instance of a named module.
    ///
    /// The instance is a *statement*, not a value: its outputs are separate wire
    /// nodes that it drives, and the design reads those. A1 made the instance
    /// node itself the output, which works for a one-output submodule and does
    /// not work for a blackbox at all — a hard SRAM macro has one output and
    /// seventeen inputs, and a RISC-V core has twenty-odd outputs.
    ///
    /// The instance's ports are named, and the names are the module's own. See
    /// [`InstancePort`] for why the positional scheme was replaced rather than
    /// kept alongside.
    Instance {
        /// The module's name, as it will appear in emitted Verilog.
        name: String,
        /// Compile-time parameters, as literals.
        params: Vec<(String, Bits)>,
        /// The port connections, in the order they were declared.
        ///
        /// The order is the emission order, so the emitted Verilog is stable.
        ports: Vec<InstancePort>,
    },
}

impl Node {
    /// This node's width in bits, derived from its operands.
    ///
    /// Infallible, and cheap: one match, no arena lookup. Widths that cannot be
    /// derived from operands alone are stored on the node, and every constructor
    /// computes them from the operands and stores the result, so the two cannot
    /// disagree without [`Circuit::check`](crate::Circuit::check) noticing.
    ///
    /// [`Circuit::check`](crate::Circuit::check) is not optional bookkeeping: the
    /// widths that are stored rather than derived are exactly the ones where a
    /// hand-built node could lie.
    #[must_use]
    pub fn width(&self, circuit: &crate::Circuit) -> u32 {
        match self {
            Node::Constant { value } => value.width(),
            Node::Wire { width, .. } | Node::Case { width, .. } => *width,
            // An instance is a statement, not a value: its outputs are separate
            // wire nodes that it drives, so there is no single width to report.
            // Zero is the honest answer, and a caller that wants an output
            // should ask for the port rather than for the instance.
            Node::Instance { .. } => 0,
            Node::Not { arg } => circuit.width_of(*arg),
            Node::Select { len, .. } => *len,
            Node::BitAnd { left, .. }
            | Node::BitOr { left, .. }
            | Node::BitXor { left, .. }
            | Node::Add { left, .. }
            | Node::Sub { left, .. }
            | Node::UDiv { left, .. }
            | Node::SDiv { left, .. }
            | Node::URem { left, .. }
            | Node::SRem { left, .. } => circuit.width_of(*left),
            Node::Mul { left, right } => circuit
                .width_of(*left)
                .saturating_add(circuit.width_of(*right)),
            Node::Eq { .. }
            | Node::Ult { .. }
            | Node::Ule { .. }
            | Node::Ugt { .. }
            | Node::Uge { .. }
            | Node::Slt { .. }
            | Node::Sle { .. }
            | Node::Sgt { .. }
            | Node::Sge { .. } => 1,
            Node::Cat { high, low } => circuit
                .width_of(*high)
                .saturating_add(circuit.width_of(*low)),
            Node::Replicate { value, count } => circuit.width_of(*value).saturating_mul(*count),
            Node::Ite { then_value, .. } => circuit.width_of(*then_value),
            Node::Reg { data, .. } => circuit.width_of(*data),
            Node::Mem { data_width, .. } => *data_width,
            Node::ReadPort { data_width, .. } => *data_width,
        }
    }

    /// Whether this node holds state.
    ///
    /// A single constructor match, inherited from Hardcaml's design. The output of
    /// a register *is* the register node, so there is no separate "this is
    /// stateful" flag that could disagree with the node's actual contents.
    #[must_use]
    pub const fn is_stateful(&self) -> bool {
        matches!(self, Node::Reg { .. } | Node::Mem { .. })
    }

    /// A short human-readable name for the node kind, for error messages.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Node::Constant { .. } => "Constant",
            Node::Wire { .. } => "Wire",
            Node::Not { .. } => "Not",
            Node::Select { .. } => "Select",
            Node::BitAnd { .. } => "BitAnd",
            Node::BitOr { .. } => "BitOr",
            Node::BitXor { .. } => "BitXor",
            Node::Add { .. } => "Add",
            Node::Sub { .. } => "Sub",
            Node::Mul { .. } => "Mul",
            Node::UDiv { .. } => "UDiv",
            Node::SDiv { .. } => "SDiv",
            Node::URem { .. } => "URem",
            Node::SRem { .. } => "SRem",
            Node::Eq { .. } => "Eq",
            Node::Ult { .. } => "Ult",
            Node::Ule { .. } => "Ule",
            Node::Ugt { .. } => "Ugt",
            Node::Uge { .. } => "Uge",
            Node::Slt { .. } => "Slt",
            Node::Sle { .. } => "Sle",
            Node::Sgt { .. } => "Sgt",
            Node::Sge { .. } => "Sge",
            Node::Cat { .. } => "Cat",
            Node::Replicate { .. } => "Replicate",
            Node::Ite { .. } => "Ite",
            Node::Case { .. } => "Case",
            Node::Reg { .. } => "Reg",
            Node::Mem { .. } => "Mem",
            Node::ReadPort { .. } => "ReadPort",
            Node::Instance { .. } => "Instance",
        }
    }

    /// The node's operands, in a fixed order, for generic traversals that do not
    /// care which relation is in force.
    #[must_use]
    pub fn operands(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.push_operands(&mut out);
        out
    }

    /// Rewrite every operand through `f`, leaving the node's kind and stored
    /// widths alone.
    ///
    /// Used by the second phase of
    /// [`Circuit::apply_replacements`](crate::Circuit::apply_replacements). The
    /// wire's driver is *not* an operand and is not touched here: re-attaching a
    /// driver is that function's third phase, and doing both in one pass would
    /// make the two phases indistinguishable.
    pub fn map_operands(&mut self, mut f: impl FnMut(NodeId) -> NodeId) {
        macro_rules! remap {
            ($($id:ident),* $(,)?) => {{
                $( *$id = f(*$id); )*
            }};
        }
        match self {
            Node::Constant { .. } | Node::Wire { .. } => {}
            Node::Mem { write_ports, .. } => {
                for port in write_ports {
                    port.map_signals(&mut f);
                }
            }
            Node::Not { arg } => remap!(arg),
            Node::Select { value, .. } => remap!(value),
            Node::BitAnd { left, right }
            | Node::BitOr { left, right }
            | Node::BitXor { left, right }
            | Node::Add { left, right }
            | Node::Sub { left, right }
            | Node::Mul { left, right }
            | Node::UDiv { left, right }
            | Node::SDiv { left, right }
            | Node::URem { left, right }
            | Node::SRem { left, right }
            | Node::Eq { left, right }
            | Node::Ult { left, right }
            | Node::Ule { left, right }
            | Node::Ugt { left, right }
            | Node::Uge { left, right }
            | Node::Slt { left, right }
            | Node::Sle { left, right }
            | Node::Sgt { left, right }
            | Node::Sge { left, right } => remap!(left, right),
            Node::Cat { high, low } => remap!(high, low),
            Node::Replicate { value, .. } => remap!(value),
            Node::Ite {
                condition,
                then_value,
                otherwise,
            } => {
                remap!(condition, then_value, otherwise);
            }
            Node::Case {
                scrutinee,
                arms,
                default,
                ..
            } => {
                *scrutinee = f(*scrutinee);
                for arm in arms.iter_mut() {
                    arm.value = f(arm.value);
                }
                *default = f(*default);
            }
            Node::Reg {
                data,
                clock,
                reset,
                clear,
            } => remap!(data, clock, reset, clear),
            Node::ReadPort {
                mem,
                address,
                enable,
                ..
            } => remap!(mem, address, enable),
            Node::Instance { ports, .. } => {
                // *Every* port is remapped here, not only the inputs. An output
                // port's signal is a wire the instance drives, so if a rewrite
                // replaces that wire the instance has to drive the replacement
                // instead — otherwise the rewritten graph has an undriven net
                // and the original still driven.
                for port in ports.iter_mut() {
                    port.signal = f(port.signal);
                }
            }
        }
    }

    pub(crate) fn push_operands(&self, out: &mut Vec<NodeId>) {
        match self {
            Node::Constant { .. } | Node::Wire { .. } => {}
            Node::Not { arg } => out.push(*arg),
            Node::Select { value, .. } => out.push(*value),
            Node::BitAnd { left, right }
            | Node::BitOr { left, right }
            | Node::BitXor { left, right }
            | Node::Add { left, right }
            | Node::Sub { left, right }
            | Node::Mul { left, right }
            | Node::UDiv { left, right }
            | Node::SDiv { left, right }
            | Node::URem { left, right }
            | Node::SRem { left, right }
            | Node::Eq { left, right }
            | Node::Ult { left, right }
            | Node::Ule { left, right }
            | Node::Ugt { left, right }
            | Node::Uge { left, right }
            | Node::Slt { left, right }
            | Node::Sle { left, right }
            | Node::Sgt { left, right }
            | Node::Sge { left, right } => {
                out.push(*left);
                out.push(*right);
            }
            Node::Cat { high, low } => {
                out.push(*high);
                out.push(*low);
            }
            Node::Replicate { value, .. } => out.push(*value),
            Node::Ite {
                condition,
                then_value,
                otherwise,
            } => {
                out.push(*condition);
                out.push(*then_value);
                out.push(*otherwise);
            }
            Node::Case {
                scrutinee,
                arms,
                default,
                ..
            } => {
                out.push(*scrutinee);
                for arm in arms {
                    out.push(arm.value);
                }
                out.push(*default);
            }
            Node::Reg {
                data,
                clock,
                reset,
                clear,
            } => {
                out.push(*data);
                out.push(*clock);
                out.push(*reset);
                out.push(*clear);
            }
            Node::Mem { write_ports, .. } => {
                // The write ports are the memory's operands. Leaving them out
                // makes `Mem` a leaf under every relation, which silently hides a
                // feedback path through a memory — and the `WithoutCaseMatches`
                // relation is exactly where that shows up.
                for port in write_ports {
                    out.extend(port.signals());
                }
            }
            Node::ReadPort {
                mem,
                address,
                enable,
                ..
            } => {
                out.push(*mem);
                out.push(*address);
                out.push(*enable);
            }
            // Only an *input* port is a dependency. An output port's signal is a
            // net the instance drives, so the edge runs the other way: the net
            // depends on the instance, not the instance on the net. Counting it
            // as an operand would make every instance look like a combinational
            // loop through its own output.
            Node::Instance { ports, .. } => {
                for port in ports.iter() {
                    if port.direction == PortDirection::Input {
                        out.push(port.signal);
                    }
                }
            }
        }
    }
}

/// Address width needed to select any word of a memory `depth` words deep.
///
/// A depth of one needs one bit, not zero: there is no zero-width value domain
/// anywhere in this stack, and a memory with a single word still has an address
/// input that the graph must be able to carry.
#[must_use]
pub const fn address_width_for_depth(depth: u32) -> u32 {
    if depth <= 1 {
        1
    } else {
        32 - (depth - 1).leading_zeros()
    }
}

#[cfg(test)]
mod tests {
    use super::{Node, address_width_for_depth};
    use crate::NodeId;

    #[test]
    fn address_width_covers_every_word() {
        assert_eq!(address_width_for_depth(1), 1);
        assert_eq!(address_width_for_depth(2), 1);
        assert_eq!(address_width_for_depth(3), 2);
        assert_eq!(address_width_for_depth(4), 2);
        assert_eq!(address_width_for_depth(5), 3);
        assert_eq!(address_width_for_depth(256), 8);
        assert_eq!(address_width_for_depth(257), 9);
    }

    #[test]
    fn address_width_can_select_the_whole_depth() {
        for depth in 1..=1024u32 {
            let width = address_width_for_depth(depth);
            let selectable = 1u64.checked_shl(width).unwrap_or(u64::MAX);
            assert!(
                selectable >= u64::from(depth),
                "depth {depth} does not fit in {width} address bits"
            );
        }
    }

    #[test]
    fn only_reg_and_mem_are_stateful() {
        assert!(
            Node::Mem {
                data_width: 8,
                depth: 4,
                write_ports: Vec::new(),
                initial: None,
                read_latency: 0,
                macro_form: None,
            }
            .is_stateful()
        );
        // A read port reads state but holds none, which is the whole reason reads
        // are separate nodes.
        assert!(
            !Node::ReadPort {
                mem: NodeId::from_index(0),
                address: NodeId::from_index(1),
                enable: NodeId::from_index(2),
                data_width: 8,
            }
            .is_stateful()
        );
    }
}
