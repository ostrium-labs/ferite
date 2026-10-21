# A memory's read latency belongs to the memory, and a macro is a lowering

## Status

Accepted. Refines ADR-0017, which asked for a memory abstraction that emits a
macro instantiation when the selected PDK provides one and falls back to a
generated latch or DFF array otherwise. This says what "the same memory" means.

## Context

ADR-0017 asks for a PDK-conditional memory. It does not say what happens when
the PDK's memory is not combinational, and on the submission process it is not.

The IHP SG13G2 SRAM macro is a **synchronous** memory. The read enable and the
address are sampled at the clock edge, and the data appears in the *next* cycle.
The foundry's own simulation model says so, and the macro's `A_REN` and `A_WEN`
are clocked inputs.

Our IR's read port is the opposite: `Node::ReadPort` is an ordinary combinational
node, and A1 inherited that from Hardcaml, where `Multiport_mem` holds write ports
while reads are distinct `Mem_read_port` nodes and *"a read port is combinational,
full stop"*.

So a design that reads an SRAM and a design that reads an array of flip-flops
are **different circuits**, and the difference is a whole cycle of latency. Three
ways to deal with it, and each of them has a failure mode:

1. **The macro substitutes for the array, silently.** Then the design the
   simulator ran is not the design that was fabricated, and every differential
   test that used the memory was testing a different circuit. This is the worst
   answer, and it is the one that is easiest to write by accident.
2. **The design holds the extra register.** Then the macro path has one cycle
   too many, and the difference is in the *design* rather than in the tool, so
   the two paths are two designs.
3. **The latency is a property of the memory, and both backends honour it.**

The third is the only one where the simulator and the fabricated chip are
describing the same circuit, which is the property the whole differential-testing
architecture exists to provide.

There is a second thing the macro forces. A single-port macro has **one** address
port, and a design that reads and writes has two. The macro's own answer is that
it cannot do both at once, so the design has to mux the two addresses. If the mux
lives in the emitter, the simulator does not see it; if it lives in the design,
both backends do.

And a third: the macro has no initialisation port. A memory with a firmware image
has to be loaded by something, and the loader is a different mechanism from an
`initial` block.

## Decision

**`Node::Mem` carries three things beyond its shape: its contents, its read
latency, and optionally a macro form.**

- **`initial: Option<Vec<Bits>>`.** The contents, in address order. The simulator
  seeds the memory with them and the emitter writes them into the array, so both
  backends start from the same state. A memory with no image still starts as
  all zeros, which is what A1 meant.
- **`read_latency: u32`.** Zero is an array read: the data is there in the same
  cycle as the address. One is a synchronous read: the data is captured at the
  clock edge and arrives in the next cycle. The simulator implements the capture
  as a register fed by the array read, with the enable's complement as the
  register's *hold* — which is exactly a macro's output register, including the
  detail that a disabled read **holds** the last value rather than driving zero.
- **`macro_form: Option<MemMacro>`.** The foundry macro, with its ports bound by
  name. The emitter builds the macro *instead of* the array; the simulator still
  models the memory. The macro declares its own read latency and the memory
  declares one, and `Circuit::set_mem_macro` refuses a mismatch — because the
  latency is when the data arrives, and two backends disagreeing about that is
  the failure this whole mechanism exists to prevent.

**The address mux is built in the design.** `ferrite-lithic-chip`'s
`sram::macro_memory` builds the mux as ordinary graph nodes, so the simulator and
the emitter see the same circuit.

**A macro-backed memory has no `initial` block.** The macro is the storage and a
foundry SRAM has no initialisation port; the image is loaded by the firmware
instead. This is stated rather than papered over.

## Consequences

**The simulator's read port can hold.** `Op::ReadPort` grew a `hold` flag. A
combinational read forces zero when it is disabled, because an array read has no
previous value to hold; a synchronous read holds, because a macro's output
register holds. That difference is the whole reason the flag exists, and it is
observable: a design that reads a macro and then stops reading sees the last
value, not zero.

**The simulator's read port can be a register.** A latency-bearing read port
takes its storage from the registers section rather than the combinational one,
and the capture is a `RegUpdate`. The capture reads the array read's word, which
is computed during the settle pass *before* the edge and therefore before the
memory's writes are applied — which is a macro's read-during-write behaviour: a
read issued in the same cycle as a write to the same address returns the old
data, because the capture latched before the write landed.

**The clock is the module's property, and the simulator now takes it as one.**
The simulator used to infer the clock from the first register it found, which is
wrong for a design whose only state is a memory: such a design has no register,
so there is no clock to infer, so the memory's writes — which happen *at* the
edge — never happen, and the design reads as though the memory were empty.

That was not a theoretical gap. The cosimulator found it: a design with a memory
and no register reported zeros from the simulator and the real value from
Verilator, on the same stimulus, because the cosimulator's register-free path
settles without an edge and so never wrote the memory. The fix is
`Program::compile_with_clock` and `Sim::new_with_clock`, and the cosimulator
passes the plan's clock through. The clock is a field on the module in the
emitter and a field on the plan in the cosimulator, because that is what it is;
inferring it from the graph was always a shortcut, and this is the design that
found it out.

**The macro is the storage, so there is no array.** The emitter emits the
instantiation in place of the array declaration and the write blocks, and the
read port's net is driven by the macro's output port rather than by an assign.
The macro's own output is bound to the read port's net, so the design reads the
macro exactly as it would have read the array.

**A macro's port list is the foundry's, and every port is connected.** The
`A_BIST_*` pins are tied off and `A_DLY` is tied high, because the foundry's own
model stops the simulation if `A_DLY` is anything else. The details live in
`ferrite-lithic-chip`'s `sram` module rather than in a Verilog file nobody
reads.
