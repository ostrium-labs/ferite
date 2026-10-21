# The program-memory budget is four kilobytes, and it is measured, not estimated

## Status

Accepted. Corrects the instruction-memory figure in
`docs/plans/jane-street-protocol-emulator.md` §1 and §4, and the firmware target
that was chosen because of it.

## Context

The competition plan budgets the instruction memory like this:

> | Instruction memory | **SRAM preferred over flip-flops**; ~3.7–8.8 Kbit/tile → **~11–26 KB** |

and then chooses the firmware target on the strength of it:

> The **JSON batch path is the firmware target.** Plain JSON over a framed serial
> link avoids both protobuf varints and HTTP/2 HPACK, which is what makes it fit
> 11–26 KB of program memory.

Both the arithmetic and the conclusion are wrong, and in the same direction: the
figure is an upper bound that assumes the entire die is memory.

The arithmetic is right. 24 tiles of 30 000 µm² at the best bits-per-area the
IHP macros achieve — 0.28 bits/µm² for the 8192x32 — gives 0.7 mm² × 0.28
bits/µm² ≈ 196 000 bits ≈ 24 KB, and the low end of the range matches the
1024x8 macro's 0.166 bits/µm² almost exactly. So "3.7–8.8 Kbit/tile" is right.

**The interpretation is wrong.** 24 tiles is the *whole die*, and the die also
holds the CPU, the protocol engine, the clock tree and the routing. A design
that is 100% SRAM has no CPU in it. The plan's own §3 puts a RISC-V core at
roughly 750 LUTs — under three tiles — and §4 wants a protocol engine, a pin and
timing ISA, a firmware loader, and a `SIMD` unit on top of that.

And the area is not the cost that matters. **A macro is placed on a tile grid,
so what it costs is its bounding box, not its area.** The 1024x8 macro is 147 µm
wide and 336 µm tall; on a 200 µm × 150 µm tile that is one tile wide and
three tiles tall, whatever its area says.

## Decision

**Budget the program memory from the measured macros and their bounding boxes,
and treat four kilobytes as the ceiling until a floorplan says otherwise.**

The numbers, from the IHP open PDK's own LEF files:

| Macro | Bytes | Area (µm²) | bits/µm² | Bounding box | Tiles |
|---|---:|---:|---:|---|---:|
| `RM_IHPSG13_1P_256x8_c3_bm_bist` | 256 | 17 547 | 0.117 | 237 × 75 | 2 |
| `RM_IHPSG13_1P_512x8_c3_bm_bist` | 512 | 26 138 | 0.157 | 237 × 110 | 2 |
| `RM_IHPSG13_1P_1024x8_c2_bm_bist` | 1 024 | 49 419 | 0.166 | 147 × 336 | 3 |
| `RM_IHPSG13_1P_1024x16_c2_bm_bist` | 2 048 | 79 674 | 0.206 | 237 × 336 | 6 |
| `RM_IHPSG13_1P_1024x32_c2_bm_bist` | 4 096 | 140 183 | 0.234 | 417 × 336 | 9 |
| `RM_IHPSG13_1P_4096x8_c3_bm_bist` | 4 096 | 146 413 | 0.224 | 237 × 618 | 10 |
| `RM_IHPSG13_1P_2048x32_c2_bm_bist` | 8 192 | 261 108 | 0.251 | 417 × 627 | 15 |
| `RM_IHPSG13_1P_8192x32_c4` | 32 768 | 939 915 | 0.279 | 1520 × 618 | **40** |

Three things fall out of that table.

**The biggest macro does not fit.** The 8192x32 is the most area-efficient one
and its bounding box is forty tiles: more than the whole allocation. It is not
a candidate for anything.

**Flip-flops are not an alternative.** The competition's own figure is about
320 flip-flops per tile, so 24 tiles is 7 680 flip-flops — 7 680 bits, 960
bytes — and that is the *entire* design's register budget, spent on nothing
else. A firmware image in flip-flops is not a trade-off, it is impossible.

**Four kilobytes is the realistic ceiling.** If the CPU and the protocol engine
take twelve to sixteen tiles, eight to twelve remain, and the table says eight
to twelve tiles is two to four kilobytes.

## Consequences

**The JSON batch path does not fit, and the firmware target changes.** A
CloudEvents batched-JSON codec was chosen because it fits 11–26 KB. It does not
fit 2–4 KB. What fits 2–4 KB is a *handful* of events — a single CloudEvent in
JSON is two to four hundred bytes — and a framing layer around them.

That is not a reason to abandon the entry. It is a reason to pick a different
rung of the ladder: the plan's own ladder is "UART transmit from firmware, then
receive, then a framed link", and a framed link carrying a bounded number of
JSON events is a legitimate rung. What does *not* fit is a general-purpose
CloudEvents stack, and the write-up must not claim one.

**The plan's §5 becomes more important, not less.** The audit that found the
arithmetic core of Loams to be "roughly 200 lines across six functions" was
already the reason the entry was honest. A four-kilobyte firmware budget is the
same conclusion from the other direction: there is no room for a stack, and the
thing to demonstrate is the *toolchain* and the *verification*, with a protocol
running in firmware on a chip that has no protocol logic in silicon at all.

**`ferrite-lithic-pdk` carries the numbers, not the conclusion.** The macro
table has the areas and the bounding boxes; `Pdk::tiles_for_sram` turns a macro
into a tile count; `Pdk::smallest_sram_for` picks the cheapest macro that holds
a given number of bytes. The *budget* — how many tiles the rest of the design
gets — is a floorplan decision, and it is made when there is a floorplan.

**The area question is now answerable from the tooling.** Before this, "will it
fit" was a question for a synthesiser. Now it is a question for a table of
measured numbers, and it can be asked before the design is written.
