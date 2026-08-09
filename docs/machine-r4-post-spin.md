# Making a step-budget stop diagnosable — Milestone 2, Round 4, slice 3

## Why

`Machine::probe_post` reported a failure site down to the byte and a
step-budget stop not at all:

```text
post-probe: steps=2000000 stop=step-budget-exhausted
  post-codes=[] last=none
  com1="" debug=""
```

No `CS:IP`, no `EIP`, no linear address, nothing about what was executing. Round
3 knew SeaBIOS was spinning only because the *platform* trace happened to show
a repeating pair of events; turning that into an address took a hand bisection
over the step budget. A run that spins in code touching no device — which is
the normal case for a firmware bug — would have produced nothing at all.

## What was added

`PostReport` gains two fields:

- `stop_site: PostPcSite` — the architectural `CS:EIP` at the stop, with the
  `CS.D` execution window and the resolved linear address, printed the same way
  `PostFailure` prints its site. Populated for every stop.
- `spin: Option<PostSpinSummary>` — over the trailing `window` retired
  instructions (default 4096): how many distinct program counters there were,
  the most frequent ones, and any tight repeating cycle.

Cycle detection takes the **smallest** period whose repetition covers the end
of the window at least three times, so a self-jump reports period 1 rather than
an arbitrary multiple, and a loop entered late in the window is still found
(the run is measured backwards from the newest sample). The reported revolution
ends with the last retired instruction, which means it *begins* with the
instruction that would run next — the same site the `stop-pc` line names.

The histogram lists only program counters seen more than once. In straight-line
code every entry would read `count=1`, which tells a reader nothing.

Sampling costs one ring push per retired instruction and only while a probe is
running. `Machine::probe_post_options(max_steps, trace, spin)` lets a caller
size or disable the window; `probe_post` and `probe_post_traced` use the
default.

## Output compatibility

The `post-probe: steps=… stop=…` header line is **byte-identical**, and a run
that stops on a failure prints **exactly** what it printed before — the header
already names the failure site, so the new block is suppressed there. The new
lines appear only for `halted` and `step-budget-exhausted`, the two stops that
previously reported nothing, and they follow the existing convention of
two-space-indented `key value` lines. `tests/post_spin.rs` pins both halves of
that contract.

## What it says about SeaBIOS today

Same run, same budget, after this slice:

```text
post-probe: steps=2000000 stop=step-budget-exhausted
  stop-pc        cs:ip=F000:FF53 cs.d=0 eip=0x0000FF53 linear_pc=0x00000000000FFF53
  spin           sampled=4096 window=4096 distinct=2 cycle=2 repeats=2048
  spin-cycle     [0] cs:ip=F000:FF53 cs.d=0 eip=0x0000FF53 linear_pc=0x00000000000FFF53
  spin-cycle     [1] cs:ip=0000:004C cs.d=0 eip=0x0000004C linear_pc=0x000000000000004C
  spin-pc        count=2048 cs:ip=0000:004C cs.d=0 eip=0x0000004C linear_pc=0x000000000000004C
  spin-pc        count=2048 cs:ip=F000:FF53 cs.d=0 eip=0x0000FF53 linear_pc=0x00000000000FFF53
```

Two instructions, alternating 2,048 times in the sampled window: `F000:FF53` is
the bare `IRET` every unclaimed IVT vector in this image points at, and
`0000:004C` is inside the interrupt vector table itself — the guest is
executing IVT data as code and taking an interrupt on every instruction. That
is a whole diagnosis from one run, where round 3 got "it spins".

## CLI surface this wants

`crates/emulator-cli` is not this slice's to edit. The flag it should grow:

```text
--post-spin [N]    Size of the trailing program-counter window used for the
                   spin summary (default 4096; 0 disables it). Implies
                   --post-probe.
```

Wiring: `run_post_probe` calls

```rust
machine.probe_post_options(max_steps, opts.post_trace, opts.post_spin)
```

where `opts.post_spin` defaults to `Some(PostSpinConfig::default())` and
`--post-spin 0` sets `None`. The stop program counter needs no flag: it is
reported unconditionally and costs nothing.

## Unsupported (explicit)

- **No call stack.** The summary is program counters, not frames; there is no
  frame-pointer walk and no symbol table.
- **No instruction text.** The report names addresses; reading the bytes there
  is still a separate step (`PostFailure` carries an opcode window, the spin
  summary does not).
- **The window is instructions, not time.** With the instruction-count step
  clock a spin of 4096 instructions is not a defined amount of emulated time.
- **A cycle claim is bounded by the window.** `repeats` counts revolutions
  inside the sampled window only; it is a lower bound on the loop's trip count,
  never the whole of it.
- **Interrupt delivery is not distinguished from a branch.** A two-site cycle
  formed by a fault or an IRQ looks exactly like a two-site cycle formed by a
  jump; the trace is what tells them apart.
