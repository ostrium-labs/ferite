//! The Verilog writer: one wire per node, in a fixed order.
//!
//! # Emission order is fixed
//!
//! `port list`, `declarations`, `statements`, `alias assigns`, `output assigns`,
//! `endmodule` — inherited from `src/rtl_verilog_of_ast.ml:391-399`. The order is
//! not cosmetic: declarations must precede the assigns that read them, and the two
//! kinds of assign have to be separable so a reader (or a diff) can tell "this is
//! the logic" from "this is the wiring".
//!
//! # One wire per node
//!
//! Every reachable node except a [`Constant`](ferrite_lithic_ir::Node::Constant)
//! gets an identifier and a statement, and every operand is referenced by that
//! identifier rather than inlined. That is what makes the output reviewable: an
//! eight-bit add is `assign signal_Add = a + b;` and not a nested expression, so
//! each line of the emitted Verilog is one line of the design. It is also why there
//! is no precedence table here — an operand is always a primary expression, so
//! there is nothing to parenthesise.
//!
//! # Registers
//!
//! One `always` block per register, never grouped by clock domain
//! (`src/rtl_ast.ml:329-390`), with a fixed nesting: reset, then clear, then the
//! data. Nonblocking `<=` inside, blocking `=` outside.
//!
//! Two deliberate divergences from the reference's text, both forced by our IR:
//!
//! - **The sensitivity list is the clock edge only.** Our reset is *synchronous*
//!   ([`Node::Reg`](ferrite_lithic_ir::Node::Reg)), so putting reset in the
//!   sensitivity list would make it asynchronous and change the circuit.
//!   Hardcaml's clock and reset conditions become the sensitivity list
//!   (`src/rtl_verilog_of_ast.ml:153-157`) because its reset can be either.
//! - **`clear` holds; it does not clear.** The reference emits
//!   `else if (clear) q <= clear_to;` and our IR has no `clear_to` field to put in
//!   it: `Node::Reg` documents clear as gating the update, the front end's truth
//!   table says a cleared register keeps its value, and the simulator implements it
//!   that way. So the branch is emitted as `q <= q;`. A zero here would have been a
//!   register that disagrees with its own simulator.
//!
//! A reset that is a literal zero is dropped: `if (1'b0 == 1'b1)` in every
//! register of a design whose reset is tied low is noise, and dropping a test that
//! can never be taken changes nothing.
//!
//! # Memories
//!
//! `reg [w-1:0] name [0:depth-1]`, one `always` per write port, and
//! `assign q = en ? name[addr] : '0;` for reads (`src/rtl_ast.ml:392-434`).
//!
//! A write port carries no clock ([`WritePort`](ferrite_lithic_ir::WritePort) is
//! data, address and enable), so write ports are clocked by the module's clock.
//! That is the IR's single-domain assumption made explicit rather than inferred,
//! and it is why [`Module`](crate::Module) has a clock field at all.
//!
//! # `case` nodes
//!
//! A [`Case`](ferrite_lithic_ir::Node::Case) is emitted as a `reg` plus an
//! `always @*` `case`, inherited from `src/rtl_ast.ml:501-510`. It is correct
//! Verilog and it is worth naming the consequence: "is this signal a register?" is
//! no longer answerable from the emitted text, because a combinational node can be
//! one. Nothing in this crate reads the text back, so nothing is misled by it.

use std::collections::BTreeSet;

use ferrite_lithic_bits::Bits;
use ferrite_lithic_ir::{CaseArm, Circuit, Deps, InstancePort, MemMacro, Node, NodeId, WritePort};

use crate::error::Error;
use crate::module::{Module, Signature};
use crate::naming::{NameMap, legalise};

/// Two spaces per level.
const INDENT: &str = "  ";

/// The identifiers [`emit_module`] will write for `module`'s ports.
pub(crate) fn port_identifiers(module: &Module) -> Result<Vec<String>, Error> {
    let reachable = reachable_nodes(module)?;
    let names = NameMap::build(module, &reachable)?;
    let identifiers = |ids: &[ferrite_lithic_ir::NodeId]| {
        ids.iter()
            .map(|id| names.name(*id).to_string())
            .collect::<Vec<_>>()
    };
    let mut out = identifiers(module.inputs());
    out.extend(identifiers(module.outputs()));
    Ok(out)
}

/// Emit `module`, and nothing else.
pub(crate) fn emit_module(module: &Module) -> Result<String, Error> {
    if module.name().is_empty() {
        return Err(Error::EmptyModuleName);
    }
    let circuit = module.circuit();
    circuit.check_with_inputs(module.inputs())?;
    let reachable = reachable_nodes(module)?;
    let names = NameMap::build(module, &reachable)?;
    let mut ports = BTreeSet::new();
    ports.extend(module.inputs().iter().copied());
    ports.extend(module.outputs().iter().copied());
    let mut emitter = Emitter {
        module,
        names,
        reachable,
        ports,
        out: String::new(),
    };
    emitter.run()?;
    Ok(emitter.out)
}

/// The nodes a module needs emitted, in id order.
///
/// The outputs' cone under [`Deps::WithoutCaseMatches`] — which follows a
/// register's inputs and a memory's write ports, so state is not cut off from its
/// own drivers — plus any input port the cone did not reach. A design can declare
/// an input it never uses, and that port still belongs in the port list.
fn reachable_nodes(module: &Module) -> Result<Vec<NodeId>, Error> {
    let circuit = module.circuit();
    let mut ids = circuit.reachable(module.outputs(), Deps::WithoutCaseMatches)?;
    let mut seen: BTreeSet<NodeId> = ids.iter().copied().collect();
    let extra: Vec<NodeId> = module
        .inputs()
        .iter()
        .copied()
        .filter(|id| !seen.contains(id))
        .collect();
    for id in extra {
        seen.insert(id);
        ids.push(id);
    }
    // `Circuit::reachable` already returns id order; sorting makes that this
    // module's guarantee rather than a property of someone else's arena.
    ids.sort_by_key(|id| id.index());
    Ok(ids)
}

/// The writer, holding everything the statements section needs.
struct Emitter<'a> {
    module: &'a Module,
    names: NameMap,
    reachable: Vec<NodeId>,
    ports: BTreeSet<NodeId>,
    out: String,
}

impl Emitter<'_> {
    fn run(&mut self) -> Result<(), Error> {
        // No timestamp, no host, no version, no node count. A preamble that varied
        // between runs would defeat the one property this crate exists to provide.
        self.line("// Generated by ferrite-lithic-rtl. Do not edit; regenerate from the design.");
        self.line(&format!("// module `{}`", self.module.name()));
        self.header();
        // A blank line between sections, so the fixed order is visible in the output
        // rather than only in this file.
        let mark = self.out.len();
        self.declarations()?;
        self.separate(mark);
        let mark = self.out.len();
        self.statements()?;
        self.separate(mark);
        let mark = self.out.len();
        self.aliases()?;
        self.separate(mark);
        let mark = self.out.len();
        self.output_assigns()?;
        self.separate(mark);
        self.line("endmodule");
        Ok(())
    }

    /// A blank line, if `mark` is not where the output currently ends.
    fn separate(&mut self, mark: usize) {
        if self.out.len() != mark {
            self.out.push('\n');
        }
    }

    // ------------------------------------------------------------------ header

    fn header(&mut self) {
        self.line(&format!("module {} (", self.module.name()));
        let mut ports: Vec<(&str, NodeId)> = Vec::new();
        for id in self.module.inputs() {
            ports.push(("input ", *id));
        }
        for id in self.module.outputs() {
            ports.push(("output", *id));
        }
        let last = ports.len().saturating_sub(1);
        for (index, (direction, id)) in ports.into_iter().enumerate() {
            let width = self.circuit().width_of(id);
            let comma = if index == last { "" } else { "," };
            self.line(&format!(
                "{INDENT}{direction} wire [{}:0] {}{comma}",
                width.saturating_sub(1),
                self.names.name(id),
            ));
        }
        self.line(");");
    }

    // ------------------------------------------------------------ declarations

    fn declarations(&mut self) -> Result<(), Error> {
        for index in 0..self.reachable.len() {
            let id = self.reachable[index];
            let Some(node) = self.circuit().get(id).ok() else {
                continue;
            };
            let name = self.names.name(id).to_string();
            match node {
                // A constant is rendered inline, and a port is declared in the port
                // list. Neither needs a declaration here.
                Node::Constant { .. } => continue,
                Node::Wire { .. } if self.ports.contains(&id) => continue,
                // An instance is a statement, not a value: it has no net of its
                // own, only the ports it drives. The nets it drives are wires
                // and are declared as such.
                Node::Instance { .. } => continue,
                // A `case` node is a `reg`, not a net: it is assigned inside an
                // `always @*` block.
                Node::Reg { .. } | Node::Case { .. } => {
                    self.line(&decl("reg", self.circuit().width_of(id), &name));
                }
                // A synchronous read port is a `reg` too: the capture that makes
                // the read synchronous is an `always` block, and a net driven by
                // an `always` block has to be a register. A `wire` here emits
                // text that does not elaborate.
                Node::ReadPort { mem, .. } if self.is_synchronous_read(*mem) => {
                    self.line(&decl("reg", self.circuit().width_of(id), &name));
                }
                // A memory built out of a hard macro has no array: the macro is
                // the storage, and it is instantiated in the statements section.
                Node::Mem { macro_form, .. } if macro_form.is_some() => continue,
                Node::Mem {
                    data_width, depth, ..
                } => {
                    self.line(&format!(
                        "{INDENT}reg [{}:0] {name} [0:{}];",
                        data_width.saturating_sub(1),
                        depth.saturating_sub(1)
                    ));
                }
                _ => {
                    self.line(&decl("wire", self.circuit().width_of(id), &name));
                }
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------- statements

    fn statements(&mut self) -> Result<(), Error> {
        for index in 0..self.reachable.len() {
            let id = self.reachable[index];
            let node = self.circuit().get(id)?.clone();
            match node {
                // A wire's value comes from its alias or output assign, and a
                // constant is inline: neither is a statement here.
                Node::Constant { .. } | Node::Wire { .. } => continue,
                Node::Reg {
                    data,
                    clock,
                    reset,
                    clear,
                } => self.register(id, data, clock, reset, clear)?,
                // A memory built out of a hard macro is instantiated, not
                // declared: the macro is the storage.
                Node::Mem {
                    macro_form: Some(macro_form),
                    ..
                } => self.memory_macro(id, &macro_form)?,
                Node::Mem { write_ports, .. } => {
                    self.memory_initial(id)?;
                    self.memory_writes(id, &write_ports)?;
                }
                Node::ReadPort {
                    mem,
                    address,
                    enable,
                    ..
                } => {
                    self.read_port(id, mem, address, enable)?;
                }
                Node::Instance {
                    name,
                    params,
                    ports,
                } => {
                    self.instantiate(id, &name, &params, &ports)?;
                }
                Node::Case {
                    scrutinee,
                    arms,
                    default,
                    width: _,
                } => self.case_block(id, scrutinee, &arms, default)?,
                combinational => {
                    let name = self.names.name(id).to_string();
                    let text = self.operation(id, &combinational)?;
                    self.line(&format!("{INDENT}assign {name} = {text};"));
                }
            }
        }
        Ok(())
    }

    /// One read port.
    ///
    /// Three shapes, and the difference between them is the memory's read
    /// latency:
    ///
    /// - **Latency zero, no macro.** `assign q = en ? mem[addr] : '0;` — the
    ///   array read, in the same cycle as the address.
    /// - **Latency zero, with a macro.** Nothing is emitted: the macro drives
    ///   the read port's net itself, because the macro *is* the read.
    /// - **Latency one.** A register: `always @(posedge clk) q <= en ? mem[addr]
    ///   : q;`. The capture is at the edge and the data arrives in the next
    ///   cycle, which is what a hard SRAM macro does and what the simulator
    ///   models. The `q` on the hold branch is the macro's behaviour too: a
    ///   disabled read holds the last value rather than driving zero.
    fn read_port(
        &mut self,
        id: NodeId,
        mem: NodeId,
        address: NodeId,
        enable: NodeId,
    ) -> Result<(), Error> {
        let name = self.names.name(id).to_string();
        let memory = self.names.name(mem).to_string();
        let enable = self.value(enable)?;
        let address = self.value(address)?;

        let Node::Mem {
            read_latency,
            macro_form,
            ..
        } = self.circuit().get(mem)?.clone()
        else {
            return Err(Error::NotAValue {
                id: mem,
                kind: self.circuit().get(mem)?.kind(),
            });
        };

        if macro_form.is_some() {
            // The macro drives this net. Emitting an assign for it as well
            // would give the net two drivers.
            return Ok(());
        }

        match read_latency {
            0 => self.line(&format!(
                "{INDENT}assign {name} = {enable} ? {memory}[{address}] : '0;"
            )),
            1 => {
                let clock = self.module.clock().ok_or(Error::NoClock {
                    kind: "memory read",
                    id: mem,
                })?;
                let clock = self.clock_of(clock, mem, "memory read")?;
                self.line(&format!("{INDENT}always @(posedge {clock}) begin"));
                self.line(&format!(
                    "{INDENT}{INDENT}if ({enable}) {name} <= {memory}[{address}];"
                ));
                self.line(&format!("{INDENT}{INDENT}else {name} <= {name};"));
                self.line(&format!("{INDENT}end"));
            }
            other => {
                return Err(Error::UnsupportedReadLatency {
                    mem,
                    latency: other,
                });
            }
        }
        Ok(())
    }

    /// A memory built out of a hard macro.
    ///
    /// The macro is instantiated with the ports the caller bound, and the
    /// memory's own write ports and read port are *not* emitted: the macro does
    /// the reading and the writing. The macro's output port is bound to the
    /// read port's net, so the design reads the macro exactly as it would have
    /// read the array.
    fn memory_macro(&mut self, id: NodeId, macro_form: &MemMacro) -> Result<(), Error> {
        let label = self.names.label(id).to_string();
        let module = legalise(&macro_form.module);
        let mut text = format!("{INDENT}{module}");
        if !macro_form.params.is_empty() {
            let rendered: Vec<String> = macro_form
                .params
                .iter()
                .map(|(name, value)| format!(".{}({})", legalise(name), literal(value)))
                .collect();
            text.push_str(&format!(" #({})", rendered.join(", ")));
        }
        text.push_str(&format!(" {label} (\n"));
        for port in &macro_form.ports {
            let value = self.value(port.signal)?;
            text.push_str(&format!(
                "{INDENT}{INDENT}.{} ({value}),\n",
                legalise(&port.name)
            ));
        }
        // The last connection has no trailing comma.
        if text.ends_with(",\n") {
            let trimmed = text.trim_end_matches('\n');
            let trimmed = trimmed.strip_suffix(',').unwrap_or(trimmed);
            text = format!("{trimmed}\n");
        }
        text.push_str(&format!("{INDENT});"));
        self.out.push_str(&text);
        self.out.push('\n');
        Ok(())
    }

    /// One `always` block for one register.
    fn register(
        &mut self,
        id: NodeId,
        data: NodeId,
        clock: NodeId,
        reset: NodeId,
        clear: NodeId,
    ) -> Result<(), Error> {
        let clock = self.clock_of(clock, id, "register")?;
        let name = self.names.name(id).to_string();
        let width = self.circuit().width_of(id);
        let reset_value = literal(&Bits::zeros(width)?);

        self.line(&format!("{INDENT}always @(posedge {clock}) begin"));
        // A control input tied low is a test that can never fire, so the branch goes
        // with it. A tied-*high* reset or clear is left in: it is a design that does
        // not do what it looks like it does, and the emitted text is where that
        // should become visible.
        let mut first = true;
        if !self.is_constant_zero(reset) {
            let text = self.value(reset)?;
            self.line(&format!(
                "{INDENT}{INDENT}if ({text} == 1'b1) {name} <= {reset_value};"
            ));
            first = false;
        }
        if !self.is_constant_zero(clear) {
            let text = self.value(clear)?;
            let keyword = if first { "if" } else { "else if" };
            self.line(&format!(
                "{INDENT}{INDENT}{keyword} ({text}) {name} <= {name};"
            ));
        }
        let data = self.value(data)?;
        let keyword = if first { "" } else { "else " };
        self.line(&format!("{INDENT}{INDENT}{keyword}{name} <= {data};"));
        self.line(&format!("{INDENT}end"));
        Ok(())
    }

    /// One `always` block per memory write port.
    fn memory_writes(&mut self, id: NodeId, ports: &[WritePort]) -> Result<(), Error> {
        // A read-only memory is a constant array with nothing to clock, so it does
        // not need a module clock. Only a memory that is actually written does.
        if ports.is_empty() {
            return Ok(());
        }
        let Some(clock) = self.module.clock() else {
            return Err(Error::NoClock { kind: "memory", id });
        };
        let clock = self.clock_of(clock, id, "memory")?;
        let memory = self.names.name(id).to_string();
        for port in ports {
            let enable = self.value(port.enable)?;
            let address = self.value(port.address)?;
            let data = self.value(port.data)?;
            self.line(&format!("{INDENT}always @(posedge {clock}) begin"));
            self.line(&format!(
                "{INDENT}{INDENT}if ({enable}) {memory}[{address}] <= {data};"
            ));
            self.line(&format!("{INDENT}end"));
        }
        Ok(())
    }

    /// The `initial` block that gives a memory its contents.
    ///
    /// A memory with an image is not a memory of zeros: a firmware image and a
    /// protocol lookup table are both data, and both have to be in the Verilog
    /// rather than assumed. The block is emitted once per memory, after the
    /// declarations and before the write blocks, and it assigns each word
    /// literally.
    ///
    /// A macro-backed memory has no array to initialise — the macro is the
    /// storage, and a foundry SRAM has no initialisation port — so nothing is
    /// emitted for it. The image is loaded by the firmware instead.
    fn memory_initial(&mut self, id: NodeId) -> Result<(), Error> {
        let Node::Mem {
            initial: Some(contents),
            data_width,
            ..
        } = self.circuit().get(id)?.clone()
        else {
            return Ok(());
        };
        let memory = self.names.name(id).to_string();
        self.line(&format!("{INDENT}initial begin"));
        for (index, word) in contents.iter().enumerate() {
            self.line(&format!(
                "{INDENT}{INDENT}{memory}[{index}] = {};",
                literal(word)
            ));
        }
        self.line(&format!("{INDENT}end"));
        let _ = data_width;
        Ok(())
    }

    /// An instantiation site.
    ///
    /// Ports are connected **by name**. The name is the module's own, and it is
    /// the connection: `.A_CLK(clk)`, `.A_DOUT(dout)`.
    ///
    /// This replaced a positional scheme (`.i0`, `.i1`, `.o0`) that existed
    /// because the graph had nowhere to put a port's name. A hard SRAM macro
    /// has seventeen ports named by the foundry and a RISC-V core has
    /// twenty-odd outputs; neither can be bound positionally to a module this
    /// project did not write.
    fn instantiate(
        &mut self,
        id: NodeId,
        module: &str,
        params: &[(String, Bits)],
        ports: &[InstancePort],
    ) -> Result<(), Error> {
        let label = self.names.label(id).to_string();
        // The instantiation site spells the submodule by the name in the graph, so
        // it has to legalise it exactly the way the definition did. If it did not,
        // `module signed_` would be instantiated as `signed` and nothing would say
        // why.
        let module = legalise(module);
        let mut text = format!("{INDENT}{module}");
        if !params.is_empty() {
            let rendered: Vec<String> = params
                .iter()
                .map(|(name, value)| format!(".{}({})", legalise(name), literal(value)))
                .collect();
            text.push_str(&format!(" #({})", rendered.join(", ")));
        }
        text.push_str(&format!(" {label} (\n"));
        for (index, port) in ports.iter().enumerate() {
            let value = self.value(port.signal)?;
            let comma = if index + 1 == ports.len() { "" } else { "," };
            text.push_str(&format!(
                "{INDENT}{INDENT}.{} ({value}){comma}\n",
                legalise(&port.name)
            ));
        }
        text.push_str(&format!("{INDENT});"));
        self.out.push_str(&text);
        self.out.push('\n');
        Ok(())
    }

    /// One `always @*` block for one `case`.
    fn case_block(
        &mut self,
        id: NodeId,
        scrutinee: NodeId,
        arms: &[CaseArm],
        default: NodeId,
    ) -> Result<(), Error> {
        let name = self.names.name(id).to_string();
        let scrutinee = self.value(scrutinee)?;
        self.line(&format!("{INDENT}always @* begin"));
        self.line(&format!("{INDENT}{INDENT}case ({scrutinee})"));
        for arm in arms {
            // One arm can carry several match values, which is how a multi-way
            // compare is written without a chain of two-way `ite`s. Verilog wants
            // them comma-separated in a single item.
            let matches: Vec<String> = arm.matches.iter().map(literal).collect();
            let value = self.value(arm.value)?;
            self.line(&format!(
                "{INDENT}{INDENT}{INDENT}{}: {name} = {value};",
                matches.join(", ")
            ));
        }
        let default = self.value(default)?;
        self.line(&format!(
            "{INDENT}{INDENT}{INDENT}default: {name} = {default};"
        ));
        self.line(&format!("{INDENT}{INDENT}endcase"));
        self.line(&format!("{INDENT}end"));
        Ok(())
    }

    // ----------------------------------------------------------- alias assigns

    /// `assign` for every internal wire: the graph's wiring, kept separate from
    /// the logic so a diff of one does not churn the other.
    fn aliases(&mut self) -> Result<(), Error> {
        for index in 0..self.reachable.len() {
            let id = self.reachable[index];
            if self.ports.contains(&id) {
                continue;
            }
            // Only a wire has a driver to alias. Asking any other node is an IR
            // error, and every other kind already has a statement above.
            if !matches!(self.circuit().get(id)?, Node::Wire { .. }) {
                continue;
            }
            let Some(driver) = self.circuit().driver_of(id)? else {
                continue;
            };
            // A wire driven by an instance is driven by the *instantiation*, not
            // by an expression: the instance's port connection is the assignment.
            // Aliasing it as well would give the net two drivers.
            if matches!(self.circuit().get(driver)?, Node::Instance { .. }) {
                continue;
            }
            let name = self.names.name(id).to_string();
            let text = self.value(driver)?;
            self.line(&format!("{INDENT}assign {name} = {text};"));
        }
        Ok(())
    }

    /// `assign` for every output port, in port-list order.
    fn output_assigns(&mut self) -> Result<(), Error> {
        for id in self.module.outputs() {
            let driver = self
                .circuit()
                .driver_of(*id)?
                .ok_or(Error::UndrivenPort { id: *id })?;
            let name = self.names.name(*id).to_string();
            let text = self.value(driver)?;
            self.line(&format!("{INDENT}assign {name} = {text};"));
        }
        Ok(())
    }

    // ------------------------------------------------------------------ values

    /// A node's value, as it appears on the right of an `=` or a `<=`.
    ///
    /// Every operand is a primary expression: either an identifier this module
    /// declared, or an inline literal. That follows from the one-wire-per-node rule
    /// and is why there is no precedence table here.
    fn value(&self, id: NodeId) -> Result<String, Error> {
        match self.circuit().get(id)? {
            Node::Constant { value } => Ok(literal(value)),
            Node::Mem { .. } => Err(Error::NotAValue {
                id,
                kind: self.circuit().get(id)?.kind(),
            }),
            // An instance is a statement, not a value. Its outputs are separate
            // nets it drives, and those nets are what a design reads.
            Node::Instance { .. } => Err(Error::NotAValue {
                id,
                kind: self.circuit().get(id)?.kind(),
            }),
            _ => Ok(self.names.name(id).to_string()),
        }
    }

    /// The right-hand side of a combinational `assign`.
    ///
    /// The kinds that are driven by their own block are readable by name, so they
    /// are answered here rather than refused: a nested `case` or a memory read is a
    /// perfectly good operand.
    fn operation(&self, id: NodeId, node: &Node) -> Result<String, Error> {
        // Operands go through `value`, not straight to the name map: a resize is
        // lowered to a concatenation with a *constant* (a zero pad, or the high half
        // being kept), so an operand is very often a literal rather than a wire.
        let name = |other: &NodeId| self.value(*other);
        let signed = |other: &NodeId| -> Result<String, Error> {
            Ok(format!("$signed({})", self.value(*other)?))
        };
        Ok(match node {
            Node::Not { arg } => format!("~{}", name(arg)?),
            Node::Select {
                value: operand,
                offset,
                len,
            } => {
                // `a[0 +: 0]` is not Verilog. `Circuit::select` refuses a zero
                // length, so this only fires for a hand-built node.
                if *len == 0 {
                    return Err(Error::NotAValue {
                        id,
                        kind: node.kind(),
                    });
                }
                // **A part-select of a literal is not Verilog.** Constants are
                // inlined wherever they are used -- a resize is lowered to a
                // concatenation with a constant, so an operand is very often a
                // literal rather than a wire -- and `32'h510e527f[6 +: 26]` does not
                // parse. So when the operand is a constant the selection is evaluated
                // here and emitted as a fresh literal of the selection's own width.
                //
                // This was found by SHA-256, where a shift or a truncate of the IV
                // emits uncompilable Verilog while the simulator evaluates the same
                // node correctly. The two backends agreed on the value and disagreed
                // on whether the output was Verilog at all, which is the worst
                // possible way for them to disagree: the failure surfaces at
                // elaboration, in a file nobody reads, about an operation three
                // layers below the one that was written.
                if let Node::Constant { value } = self.circuit().get(*operand)? {
                    let selected = value.select(*offset, *len).map_err(Error::Bits)?;
                    return Ok(literal(&selected));
                }
                format!("{}[{offset} +: {len}]", name(operand)?)
            }
            Node::BitAnd { left, right } => bin(&name(left)?, "&", &name(right)?),
            Node::BitOr { left, right } => bin(&name(left)?, "|", &name(right)?),
            Node::BitXor { left, right } => bin(&name(left)?, "^", &name(right)?),
            Node::Add { left, right } => bin(&name(left)?, "+", &name(right)?),
            Node::Sub { left, right } => bin(&name(left)?, "-", &name(right)?),
            // No cast, deliberately. Verilog sizes a multiply's operands from the
            // assignment's left-hand side, and the wire this assigns to is the sum
            // of the operand widths, so the product is already full precision. The
            // declared width is what makes that true, and a test asserts it.
            Node::Mul { left, right } => bin(&name(left)?, "*", &name(right)?),
            Node::UDiv { left, right } => bin(&name(left)?, "/", &name(right)?),
            Node::URem { left, right } => bin(&name(left)?, "%", &name(right)?),
            // The only casts this emitter writes. Every other operand in the graph
            // is an unsigned net, so `$signed` appears exactly where the graph is
            // signed and nowhere else.
            Node::SDiv { left, right } => bin(&signed(left)?, "/", &signed(right)?),
            Node::SRem { left, right } => bin(&signed(left)?, "%", &signed(right)?),
            Node::Slt { left, right } => bin(&signed(left)?, "<", &signed(right)?),
            Node::Sle { left, right } => bin(&signed(left)?, "<=", &signed(right)?),
            Node::Sgt { left, right } => bin(&signed(left)?, ">", &signed(right)?),
            Node::Sge { left, right } => bin(&signed(left)?, ">=", &signed(right)?),
            Node::Eq { left, right } => bin(&name(left)?, "==", &name(right)?),
            Node::Ult { left, right } => bin(&name(left)?, "<", &name(right)?),
            Node::Ule { left, right } => bin(&name(left)?, "<=", &name(right)?),
            Node::Ugt { left, right } => bin(&name(left)?, ">", &name(right)?),
            Node::Uge { left, right } => bin(&name(left)?, ">=", &name(right)?),
            Node::Cat { high, low } => format!("{{{}, {}}}", name(high)?, name(low)?),
            Node::Replicate { value, count } => {
                // `{0{a}}` is zero bits wide, so the wire it would be assigned to is
                // a lie. `Circuit::replicate` refuses a zero count.
                if *count == 0 {
                    return Err(Error::NotAValue {
                        id,
                        kind: node.kind(),
                    });
                }
                format!("{{{count}{{{}}}}}", name(value)?)
            }
            Node::Ite {
                condition,
                then_value,
                otherwise,
            } => format!(
                "{} ? {} : {}",
                name(condition)?,
                name(then_value)?,
                name(otherwise)?
            ),
            // Driven by their own block, so reading one reads its wire.
            Node::Wire { .. }
            | Node::Reg { .. }
            | Node::Case { .. }
            | Node::ReadPort { .. }
            | Node::Instance { .. } => self.names.name(id).to_string(),
            // A memory has no value expression, and a constant is never the right
            // side of its own `assign`.
            Node::Mem { .. } | Node::Constant { .. } => {
                return Err(Error::NotAValue {
                    id,
                    kind: node.kind(),
                });
            }
        })
    }

    // ------------------------------------------------------------------ clocks

    /// The clock to put in a sensitivity list, refusing anything that is not the
    /// module's own clock.
    fn clock_of(&self, clock: NodeId, owner: NodeId, kind: &'static str) -> Result<String, Error> {
        if matches!(self.circuit().get(clock)?, Node::Constant { .. }) {
            return Err(Error::ConstantClock { reg: owner });
        }
        match self.module.clock() {
            None => Err(Error::NoClock { kind, id: owner }),
            Some(expected) if expected != clock => Err(Error::ClockMismatch {
                reg: owner,
                found: clock,
                expected,
            }),
            Some(_) => Ok(self.names.name(clock).to_string()),
        }
    }

    fn is_constant_zero(&self, id: NodeId) -> bool {
        matches!(self.circuit().get(id), Ok(Node::Constant { value }) if value.is_zero())
    }

    /// Whether a memory's read is synchronous.
    ///
    /// A memory with a read latency of one is a register at its read port: the
    /// data is captured at the clock edge and arrives in the next cycle. That
    /// is what a hard SRAM macro does, and it is why the two kinds of read
    /// have to be told apart in the emitted text — one is a wire and one is a
    /// register.
    fn is_synchronous_read(&self, mem: NodeId) -> bool {
        matches!(
            self.circuit().get(mem),
            Ok(Node::Mem { read_latency, .. }) if *read_latency > 0
        )
    }

    fn circuit(&self) -> &Circuit {
        self.module.circuit()
    }

    fn line(&mut self, text: &str) {
        self.out.push_str(text);
        self.out.push('\n');
    }
}

/// Emit every module into one file, collapsing same-named definitions.
///
/// # What the collapse does and does not prove
///
/// The reference emits one module per *instance*, deduplicated by name, because it
/// keeps a table of module definitions (`src/rtl.ml:75-167`). We have no such table
/// and no way to build one: [`Circuit`] does not implement `PartialEq`, so two
/// graphs cannot be compared for equality here.
///
/// So the identity used is the name plus the port signature. Two modules agreeing on
/// both are emitted once — the case that matters, and what makes a design with two
/// instances of one submodule produce one definition. Two modules sharing a name and
/// disagreeing about their ports cannot both be defined, so that is
/// [`Error::ModuleSignatureConflict`].
///
/// The residual gap is stated rather than papered over: two *different* circuits with
/// the same name *and* the same port signature collapse to the first one, and this
/// crate cannot detect it. Giving `Circuit` a `PartialEq` is the fix, and it belongs
/// to the IR crate.
///
/// # Errors
///
/// Whatever [`Module::emit`] reports, plus [`Error::EmptyModuleName`] and
/// [`Error::ModuleSignatureConflict`].
pub fn emit_library(modules: &[&Module]) -> Result<String, Error> {
    let mut seen: Vec<(&str, Signature)> = Vec::new();
    let mut out = String::new();
    for module in modules {
        if module.name().is_empty() {
            return Err(Error::EmptyModuleName);
        }
        let signature = module.signature()?;
        if let Some((_, first)) = seen.iter().find(|(name, _)| *name == module.name()) {
            if *first == signature {
                continue;
            }
            return Err(Error::ModuleSignatureConflict {
                name: module.name().to_string(),
                first: (first.inputs, first.outputs),
                second: (signature.inputs, signature.outputs),
            });
        }
        seen.push((module.name(), signature));
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&module.emit()?);
    }
    Ok(out)
}

/// A declaration line for a net or register of `width` bits.
///
/// A one-bit signal is declared `[0:0]` rather than as a scalar net, because the
/// alternative is a special case in three places — the port list, the register
/// block and the memory — and a special case in a width is where a slice goes
/// wrong.
fn decl(kind: &str, width: u32, name: &str) -> String {
    format!("{INDENT}{kind} [{}:0] {name};", width.saturating_sub(1))
}

/// `left op right`, with no parentheses because no operand needs them.
fn bin(left: &str, op: &str, right: &str) -> String {
    format!("{left} {op} {right}")
}

/// A sized hexadecimal literal.
///
/// [`Bits::to_hex`] emits `ceil(width / 4)` digits and Verilog truncates an
/// over-wide literal to the *low* bits, so a five-bit value renders as `5'h1f` and
/// keeps every bit it has.
fn literal(value: &Bits) -> String {
    format!("{}'h{}", value.width(), value.to_hex())
}
