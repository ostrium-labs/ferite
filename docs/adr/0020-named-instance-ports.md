# Named instance ports, and the end of the positional scheme

## Status

Accepted. Supersedes the positional port scheme described in ADR-0006 and
`PortNames::Positional`.

## Context

A1 had to answer a question the IR could not: **how does an instantiation site
bind to a module's ports?**

`Node::Instance` recorded a module's name, its parameters, and the *order* of its
inputs. It did not record the names of the module's ports, because the graph had
no field for them. So an instantiation site could not say what it was connecting
to, and a submodule emitted with its own design's port names could not be bound by
one at all.

The A1 answer was positional: inputs are `.i0`, `.i1`, …, and the single output is
`.o0`, with `Module::with_port_names(PortNames::Positional)` emitting a submodule
under exactly those names and reserving them before any user name so a signal
called `i0` could not shadow a port.

That answer was correct for the question A1 could ask. The competition chip asks a
different one.

The Jane Street protocol emulator competition (announced 10 September 2026) asks
for *"a tiny CPU with an instruction set designed for reading pins, writing pins,
counting cycles, and hitting timing precisely"*. That CPU has to run on a chip
with a hard memory, and the hard memory on the submission process is the IHP
SG13G2 SRAM macro. That macro's interface is not ours:

```
RM_IHPSG13_1P_1024x8_c2_bm_bist sram (
    .A_CLK(clk), .A_MEN(men), .A_WEN(wen), .A_REN(ren),
    .A_ADDR(addr), .A_DIN(din), .A_DLY(1'b1), .A_DOUT(dout), .A_BM(8'hff),
    .A_BIST_CLK(1'b0), .A_BIST_EN(1'b0), .A_BIST_MEN(1'b0), .A_BIST_WEN(1'b0),
    .A_BIST_REN(1'b0), .A_BIST_ADDR(10'b0), .A_BIST_DIN(8'b0), .A_BIST_BM(8'b0)
);
```

Seventeen ports, named by the foundry, and a RISC-V core has twenty-odd outputs.
**Neither can be bound positionally to a module this project did not write.** A
positional binding to a module it *did* write is a guess about an interface that a
name states exactly.

The same requirement appears everywhere a real design meets the outside world. The
Tiny Tapeout wrapper has to connect to a design's ports by name. A protocol
emulator's firmware image has to be laid out in a memory with a known shape.

## Decision

**An instance's ports are named, and the name is the connection.**

`Node::Instance` carries a list of `InstancePort`s, each with a name, a
direction, a signal and a width. The emitter writes `.A_CLK(clk)`. The
`PortNames::Positional` scheme is removed rather than kept alongside.

Three things follow, and each of them is a real change rather than a rename.

**An instance is a statement, not a value.** A1's instance node *was* its single
output: `Node::width` returned `output_width` and the emitter declared a wire for
it. With several outputs that cannot hold, so the instance's outputs are separate
wire nodes that it drives, and the instance node itself has no width. The
reference's packed-output vector becomes a list of named output ports, and the
bundle type it wanted is not needed: a name says what the signal is, which is
more than a position ever did.

**Only an input port is a dependency.** An output port's signal is a net the
instance drives, so the edge runs the other way. This was found by the first test
of the new scheme, not by reading the code: counting an output port as an operand
makes every instance look like a combinational loop through its own output, and
`topological_order` says so.

**A port's width is checked against the signal on this side.** A macro whose
`A_ADDR` is ten bits wide cannot be silently bound to a four-bit address. The
check is in `Circuit::instance`, not in the emitter, so it applies to every
backend.

## Consequences

**The positional scheme's tests are gone and the named scheme's tests are
stricter.** `a_positional_module_needs_no_port_names` became
`a_module_needs_names_for_its_ports`: an unnamed port is now an error, because a
port's identifier *is* its name. `an_instantiation_site_binds_to_a_module_by_its_port_names`
replaces the positional binding test and checks the same thing by name.

**The emitter's reserved-name pool shrinks.** The positional scheme reserved
`i0`, `i1`, …, `o0`, … before any user name. Nothing reserves them now, so a
signal called `i0` is an ordinary signal named `i0`.

**The cosimulator's driver generation simplifies.** It had a positional branch
that produced `i0`-style names for a positional module; there is one naming scheme
now, and the generated C++ assigns the identifiers the emitter actually wrote.

**A design with an instance cannot be simulated.** That was already true
(`Error::NoInstanceModel`) and it stays true: a blackbox is a module this
simulator has no model of. The blackbox path is verified by Verilator
cosimulation against a software reference, which is what ADR-0018's architecture
is for.

**`Design::instance` is replaced by `Design::blackbox`.** The old name suggested
a submodule of this design; the new one says what it is. The old one also took a
single output width, which the new one does not have.
