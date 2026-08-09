# POST probe event trace

`Machine::probe_post` answers "where did firmware die". This adds an answer to
"what was it doing".

## Why

The probe stops at the first thing the machine cannot do and reports that one
site. Everything the firmware did on the way there — the ports it drove, the
PCI functions it enumerated, whether it ever reached PAM programming or the VGA
aperture — is gone by the time the report is printed. Round 2 ended with three
seams (PAM shadowing, the VGA MMIO aperture, the CMOS/fw_cfg configuration
bytes) wired but never exercised by firmware, and no way to observe them being
exercised short of adding assertions to a device and re-running.

## API

```rust
let traced = machine.probe_post_traced(max_steps, Some(PostTraceConfig::with_capacity(256)));
println!("{traced}");                       // report, then the trace
let trace = traced.trace.expect("armed");
trace.total();                              // events observed
trace.dropped();                            // events aged out of the ring
for (seq, event) in trace.events() { /* ... */ }
```

`probe_post(max_steps)` is now `probe_post_traced(max_steps, None).report` and
behaves exactly as before: nothing is recorded and each access pays one branch.

## What is recorded

| Event | Source |
|---|---|
| `PortIn` / `PortOut` | every port access that is not Mechanism #1 |
| `PciConfigAddress` | `0xCF8`–`0xCFB`, with the resulting latch |
| `PciConfigData` | `0xCFC`–`0xCFF`, with the decoded bus/device/function/register and the Enable bit |
| `PamProgram` | an i440FX PMC PAM register that actually changed value |
| `VgaAperture` | a CPU access the VGA device claimed in `0xA0000`–`0xBFFFF` |
| `MemoryFault` | a CPU access that decoded to nothing (in practice, a write into a ROM window outside the PAM range — reads never fault, unmapped physical space is open bus) |

PCI cycles get their own events because `OUT 0xCF8` followed by `IN 0xCFC` says
nothing on its own: what a reader needs is the function the latch decoded to.
PAM is recorded as a *state change* rather than an access, because a
configuration write that leaves a register unchanged re-attributes nothing.

Spec references: PCI Local Bus Specification Revision 3.0 §3.2.2.3.2 (Figure
3-2 field layout); Intel 440FX 82441FX (PMC) §3.2.18 (PAM); IBM PS/2 Video
Subsystems (the fixed display aperture).

## Bounding

The trace is a ring of the most recent `capacity` events (default 256). Older
events are dropped and counted, and each retained event carries its sequence
number in the full stream, so a reader can see both how much traffic there was
and exactly where the gap is. Capacity zero counts without retaining — a cheap
way to ask "how much platform traffic did this run generate".

A window on the *end* of the run is the right shape: the interesting question is
what led to the stop, not what happened 200,000 instructions earlier.

## Output format compatibility

`PostReport`'s `Display` is unchanged, byte for byte. `TracedPostReport`'s
`Display` prints the report first and then, on its own line, a
`post-trace: events=... kept=... dropped=... capacity=...` header followed by
one indented line per event. `crates/machine-pc/tests/post_trace.rs` asserts
that the traced output *starts with* the untraced text, so a parser reading the
existing `--post-probe` format keeps working whether or not tracing is armed.

## CLI surface this wants (not implemented here — `emulator-cli` is not this
slice's to edit)

```text
--post-trace [N]   Record the last N platform events (default 256) leading up
                   to the POST stop and print them after the --post-probe
                   output. Implies --post-probe.
```

Wiring: `run_post_probe` calls `machine.probe_post_traced(max_steps, opts.post_trace)`
and prints the returned `TracedPostReport`. With the flag absent the option is
`None` and the printed text is identical to today's.

## Unsupported (explicit)

- No instruction trace. Events are platform accesses, not retired instructions;
  correlating them with code needs the failure site the report already prints.
- No timestamps. There is no time source with a defined relationship to real
  time; the sequence number is the only ordering claim made.
- Memory accesses that succeed against RAM or ROM are not recorded — only the
  VGA aperture and faults. A full memory trace would be dominated by instruction
  fetch and is a different tool.
- fw_cfg selector/DMA traffic appears only as ordinary port I/O at `0x510`-`0x51B`;
  it is not decoded into item names.
