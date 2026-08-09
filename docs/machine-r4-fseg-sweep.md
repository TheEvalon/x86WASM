# The `0xF0000000` write sweep — Milestone 2, Round 4, slice 2

Root cause of the 64 KiB write sweep the round-3 POST probe reported at
`0xF0000000`–`0xF000FFFF`. It is **not** a memory-map gap and **not** a
segmentation bug. It is a guest wild pointer, caused by a CPU-side defect in
`x86-interpreter`'s handling of the `MOV` absolute-offset forms.

## What round 3 recorded

```text
unmapped-mmio  wr page=0x00000000F000F000 count=2048
unmapped-mmio  wr page=0x00000000F000E000 count=4096
... fourteen further fully-written pages, descending ...
unmapped-mmio  wr page=0x00000000F0000000 count=4096
```

62 KiB filled downward from `0xF000F7FF`, between 50,000 and 100,000 steps,
generating no trace events because a write to unclaimed space succeeds
silently. The round-3 note recorded the shape — "an f-segment-sized copy landing
sixteen bits too high" — explicitly as a hypothesis. **That hypothesis was
wrong**, and so was the "sixteen bits too high" reading of the address.

## What it actually is

The writes come from the guest, from a `memset` loop in the shadowed BIOS:

```text
000F1F97  85 C9        test %ecx,%ecx
000F1F99  74 06        je   000F1FA1
000F1F9B  49           dec  %ecx
000F1F9C  88 14 08     mov  %dl,(%eax,%ecx,1)
000F1F9F  EB F6        jmp  000F1F97
000F1FA1  C3           ret
```

At the first sweep write (step 51,151) `EAX = 0x000C0000`, `ECX = 0xEFF4F7FF`,
`EDX = 0`. Five instructions per byte, descending — which is exactly the
observed spacing. The caller, at `000EF4F4`, is:

```text
000EF4F4  E8 8B 1A 00 00    call 000F0F84            ; -> upper bound
000EF4F9  8D 88 00 00 F4 FF lea  -0xC0000(%eax),%ecx ; length
000EF4FF  31 D2             xor  %edx,%edx           ; fill 0
000EF501  B8 00 00 0C 00    mov  $0xC0000,%eax       ; base
000EF506  E8 8C 2A 00 00    call 000F1F97            ; memset
```

That is firmware zeroing the option-ROM area from `0xC0000` up to a computed
bound. The bound came back as `0xF000F800`, so the length was `0xEFF4F800`
(3.75 GiB) and the fill started at `0xF000F7FF` and walked down. Sixteen
unmapped pages is simply as far as 2,000,000 steps get.

## Where `0xF000F800` came from

The bound function is five instructions:

```text
000F0F84  A1 50 85 0F 00    mov  0x000F8550,%eax   ; a memory-zone head pointer
000F0F89  8B 40 0C          mov  0x0C(%eax),%eax   ; -> its allocation-info dataend
000F0F8C  83 E8 10          sub  $0x10,%eax
000F0F8F  25 00 F8 FF FF    and  $0xFFFFF800,%eax  ; align down to 2 KiB
000F0F94  C3                ret
```

`0x000F8550` held **zero**, so `mov 0x0C(%eax),%eax` dereferenced a null
pointer and read linear `0x0000000C` — IVT vector 3, whose contents are
`F000:FF53` stored as the packed doubleword `0xF000FF53`. Minus `0x10`, aligned
down to 2 KiB, that is `0xF000F800`.

So the "sixteen bits too high" appearance is a coincidence: the value is not a
mis-scaled address at all, it is a real-mode interrupt vector being used as a
flat pointer by a guest that dereferenced NULL.

## Why the zone head was zero

Tracing every write to the relevant globals over a full run:

1. The last-resort zone head at `0x000F8550` is written **once**, with the
   value zero, at the end of a walk over a zone list that was empty.
2. That zone's head at `0x000F0B4C` is written exactly twice: once with a
   stack-local temporary at step 18,654, and once with zero at step 18,722
   when the firmware unwound the temporary because it could not allocate
   permanent bookkeeping for it.
3. The permanent allocation is taken from two earlier zones, at `0x000F0B3C`
   and `0x000F0B40`. The allocator read both heads and got zero from both, so
   it returned failure.
4. `0x000F0B40` had been given a valid stack-local temporary at step 18,530 —
   and reading `PhysMem` directly at that moment confirms the store landed:
   physical `0x000F0B40` holds `0x00006F38`.

Step 18,565 executes the read of that same location:

```text
000E6F6A  A1 40 0B 0F 00    mov 0x000F0B40,%eax
```

with memory holding `0x00006F38` — and `EAX` afterwards is `0x00000000`.

## The defect

`crates/x86-interpreter/src/lib.rs`:

```rust
fn moffs_offset(insn: &DecodedInsn) -> u64 {
    if insn.prefixes.addr_size_override {
        insn.immediate as u32 as u64
    } else {
        u64::from(insn.immediate as u16)
    }
}
```

The width of the absolute offset in `MOV` opcodes `A0`–`A3` is taken from the
**presence of the `67H` prefix** instead of from the resolved effective
address-size attribute. Intel SDM Vol. 1 §3.6 Table 3-4: the effective address
size is the code segment's `D` default, *inverted* by `67H`. In SeaBIOS's
32-bit flat segments `CS.D = 1`, so an unprefixed `A1` carries a `moffs32` —
and this function truncates it to 16 bits. `A1 40 0B 0F 00` therefore read
linear `0x00000B40` (which holds zero) instead of `0x000F0B40`.

The decoder is correct: it consumed four immediate bytes (the traced
instruction length is 5). Only the interpreter's address computation is wrong.
The fix is one line — use the same `asize32(insn)` helper the ModRM path
already uses — but `crates/x86-*` is not this slice's to change.

`crates/machine-pc/tests/moffs_address_size.rs` is a self-contained reproducer:
a 64 KiB test BIOS that enters a flat `CS.D=1` ring-0 segment and runs
`MOV EAX, moffs32`. `moffs32_is_the_default_in_a_32_bit_code_segment` is
`#[ignore]`d because it fails today and the correction is out of scope;
`address_size_override_selects_moffs16_in_a_32_bit_code_segment` passes now and
guards against a fix that merely inverts the condition.

## Measured effect of the one-line fix (applied locally, not committed)

With slice 1 plus that one line:

```text
post-probe: steps=150360 stop=halted
  post-codes=[] last=none
  com1="" debug=""
```

The sweep is **gone** — no `unmapped-mmio` lines at all — and SeaBIOS runs
150,360 instructions and reaches a `HLT` in the middle of PCI BAR sizing
(`00:01.3` registers `0x10`–`0x30`, write-all-ones / read-back), which is
exactly where round 3's PCI agent predicted the next misbehavior. Compare the
2,000,000-step budget exhaustion before.

## What was ruled out

- **The linear-address wrap fix.** Round 3 already established the sweep
  survives it byte for byte; confirmed again here, since the faulting address
  is formed by an ordinary base+index in a base-0 flat segment.
- **Real-mode segment base arithmetic.** The code is in a `CS.D=1` protected
  mode segment with every segment base zero; no `selector << n` is involved.
- **A memory-map gap at `0xF0000000`.** Nothing claims that range on an
  i440FX with no BAR assigned there, and dropping the writes is correct (PCI
  Local Bus Specification Revision 3.0 §3.2.2.3.4). The address is wrong, not
  the decode.
- **Slice 1's write-semantics change.** The sweep is to *unclaimed* space,
  which was already silent, so the probe output is byte-identical before and
  after slice 1.
- **A dropped store.** Reading `PhysMem` directly at step 18,530 shows the
  store that "went missing" was accepted; only the later load was wrong.
