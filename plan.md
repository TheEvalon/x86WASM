# Browser-Based x86/x64 Emulator Development Plan

## 0. Cursor Vibe-Coding Contract

This project is intended to be built in **Cursor** with short agent sessions ("vibe coding"), not as a single monolithic AI rewrite.

**Operating rules:**

1. `AGENTS.md` is the agent entrypoint. Read it at the start of every implementation chat.
2. Project rules live in `.cursor/rules/` and apply automatically.
3. Project skills live in `.cursor/skills/` �?��?� invoke them by name for recurring workflows.
4. One bounded slice per chat. Prefer a new chat over expanding an old thread.
5. Plan Mode first (files + acceptance tests), then Agent Mode, then `quality-gate`.
6. Never prompt "implement x86-64" or "boot Windows". Split work until a slice fits in one focused session.
7. Specs and oracles beat model memory. No copying code from other emulators.

**Session recipe:**

```text
@AGENTS.md  �??  /next-slice  �??  Plan Mode  �??  implement  �??  /quality-gate  �??  stop
```

**Repo root:** this plan assumes the Cargo workspace and `web/` live at the repository root (`x86WASM/`), not under a nested `emulator/` folder.

---

## 1. Project Summary

Build a new full-system x86 PC emulator that runs in modern web browsers and can eventually boot operating systems through the Intel Core 2 era, including 64-bit operating systems.

The emulator should provide functionality comparable to v86 while adding:

- x86-64, also called x64 or Intel 64
- A Core 2 compatible CPU profile
- One virtual CPU initially, then two virtual CPUs
- BIOS and UEFI boot
- Better debugging and observability
- Persistent browser storage
- A clean JavaScript and TypeScript API
- A compatibility adapter for the existing v86-based website
- A modular architecture suitable for future devices and CPU profiles

The emulator should be a functional architectural emulator. It should not attempt to reproduce the physical Core 2 pipeline, exact cache hierarchy, timing, branch predictor, heat behavior, or exact clock speed.

The primary long-term success target is:

> Windows 10 x64 reliably reaches the desktop in a browser with keyboard, mouse, storage, networking, snapshots, and acceptable performance.

---

## 2. Recommended Development Strategy

Use the following architecture:

- Write the emulator core in Rust.
- Build both native and WebAssembly targets.
- Start with a correct interpreter.
- Add a WebAssembly JIT only after the interpreter is reliable.
- Reuse standard open-source firmware such as SeaBIOS and OVMF.
- Implement a QEMU-compatible subset of a classic PC machine model.
- Keep the browser layer separate from CPU and device emulation.
- Build a v86-compatible JavaScript adapter for gradual migration of existing operating-system definitions.

Do not begin by modifying v86 deeply. Use v86, QEMU, Intel XED, and real hardware as references and test oracles, but keep the new implementation independent.

---

## 3. Project Goals

### 3.1 Required goals

- Boot from the x86 reset vector.
- Support real mode, protected mode, compatibility mode, and 64-bit long mode.
- Support legacy BIOS boot with SeaBIOS.
- Support UEFI boot with OVMF.
- Support common 16-bit, 32-bit, and 64-bit operating systems.
- Provide a complete interpreter.
- Provide a WebAssembly JIT for performance.
- Support VGA and a linear framebuffer.
- Support floppy, IDE hard disk, ATAPI CD-ROM, and ISO images.
- Support browser keyboard, mouse, display, audio, storage, and networking.
- Support save and restore state.
- Support at least one virtual CPU, then two virtual CPUs.
- Provide deterministic execution for debugging.
- Provide a stable public JavaScript and TypeScript API.
- Integrate with the existing oses.ioblako.com project.

### 3.2 Secondary goals

- HTTP range-backed disk images.
- Copy-on-write disk overlays.
- IndexedDB or OPFS persistent disks.
- E1000 or VirtIO networking.
- Sound Blaster 16 and a newer audio device.
- Built-in debugger and execution tracing.
- Serial console and debug port logging.
- Performance statistics and profiling.
- Optional parallel virtual CPU execution with Web Workers.
- Optional execution record and replay.

### 3.3 Explicit non-goals for the first version

- Cycle-accurate Core 2 simulation.
- Exact CPU frequency emulation.
- Intel VT-x or nested virtualization.
- AVX, AVX2, or AVX-512.
- Full hardware performance monitoring.
- 3D GPU acceleration.
- Windows 11 as an initial target.
- TPM 2.0 in the initial machine model.
- Secure Boot in the initial release.
- Reimplementing BIOS or UEFI firmware from scratch.

---

## 4. CPU Profiles

Expose fixed CPU profiles instead of a vague setting such as "up to Core 2". Each profile must report only features that are fully implemented and tested.

### 4.1 Profile: core2-conroe

Initial 64-bit CPU profile.

Required architectural support:

- Real mode
- Virtual 8086 mode
- 16-bit protected mode
- 32-bit protected mode
- Compatibility mode
- 64-bit long mode
- 16 general-purpose registers in long mode
- REX prefixes
- 64-bit RIP and RFLAGS
- x87
- MMX
- SSE
- SSE2
- SSE3
- SSSE3
- PAE
- NX or XD
- Four-level long-mode paging
- 4 KiB pages
- 2 MiB pages
- CPUID
- RDTSC
- RDMSR and WRMSR
- SYSENTER and SYSEXIT
- SYSCALL and SYSRET
- CMPXCHG8B
- CMPXCHG16B
- FXSAVE and FXRSTOR
- LAHF and SAHF in long mode
- Local APIC
- I/O APIC
- One or two virtual CPUs

### 4.2 Profile: core2-penryn

Includes everything in core2-conroe plus:

- SSE4.1
- Penryn-like CPUID family, model, and stepping
- Additional model-specific registers required by target operating systems

Do not advertise SSE4.2 for this profile.

### 4.3 Profile: win10-x64-compatible

A practical compatibility profile rather than a claim of exact retail CPU reproduction.

Includes:

- All core2-penryn features
- CMPXCHG16B
- LAHF and SAHF
- PREFETCHW, implemented as a safe cache hint or no-op
- PAE
- NX
- SSE2 or later
- Stable TSC behavior
- APIC and ACPI support expected by newer Windows versions

### 4.4 Features hidden until fully implemented

Do not expose these through CPUID until implementation and tests are complete:

- VMX
- SSE4.2
- POPCNT
- AVX
- AES-NI
- XSAVE and XRSTOR
- MONITOR and MWAIT
- RDTSCP
- Hardware performance counters
- Machine-check architecture
- Full hardware debug-register behavior

Incorrect CPUID reporting is considered a critical bug.

---

## 5. Operating-System Support Ladder

### Tier 0: Emulator validation

- Custom reset ROMs
- Custom boot sectors
- Small protected-mode kernels
- Small long-mode kernels
- kvm-unit-tests
- CPU and paging microtests

### Tier 1: Legacy operating systems

- FreeDOS
- MS-DOS
- Windows 3.x
- Windows 95
- Windows 98
- Windows ME
- Windows NT 4.0
- Windows 2000
- Windows XP 32-bit
- 32-bit Linux

Purpose:

- Establish v86-level legacy compatibility.
- Validate BIOS, VGA, floppy, IDE, interrupts, timers, and protected mode.

### Tier 2: 64-bit operating systems

- Windows XP x64
- Windows Vista x64
- Windows 7 x64
- x86-64 Linux
- x86-64 BSD
- ReactOS x64 where practical

Purpose:

- Validate long mode, PAE, APIC, ACPI, 64-bit exceptions, and 64-bit firmware paths.

### Tier 3: Newer operating systems

- Windows 8.1 x64
- Windows 10 x64

Purpose:

- Harden CPU feature reporting, ACPI, APIC, timers, storage, networking, and browser performance.

### Later research target

- Windows 11

Windows 11 should be treated as a separate later project because it introduces newer CPU expectations, UEFI requirements, TPM 2.0, Secure Boot capability, and modern graphics-driver expectations.

---

## 6. Technology Stack

### 6.1 Core implementation

- Rust
- Cargo workspace
- Stable Rust toolchain
- Native test executable
- wasm32-unknown-unknown target
- wasm-bindgen for the browser boundary
- wasm-encoder for JIT module generation
- cargo-fuzz for fuzzing
- Serde or a carefully versioned binary format for state metadata

### 6.2 Browser implementation

- TypeScript
- Vite or another static bundler
- Web Worker for emulator execution
- Canvas for initial display
- OffscreenCanvas where supported
- AudioWorklet for audio
- IndexedDB or OPFS for persistent storage
- WebSocket or WebTransport relay for networking
- Playwright for browser automation

### 6.3 Development and validation tools

- Cursor IDE (primary implementation environment)
- Project rules in `.cursor/rules/` and skills in `.cursor/skills/`
- `AGENTS.md` as the agent entrypoint
- Git
- Git worktrees (required for parallel agents)
- GitHub Actions or another CI platform
- Intel XED as a decoder oracle
- QEMU TCG as a behavioral reference
- kvm-unit-tests
- Real x86-64 hardware test harness where practical
- cargo clippy
- cargo fmt
- cargo test
- cargo bench or Criterion for benchmarks

### 6.4 Hosting model

The final build should be deployable as static files:

- JavaScript
- WebAssembly
- Firmware images
- Disk images or image manifests
- Worker scripts
- HTML and CSS

It should be possible to host the release on the existing XAMPP and Apache environment. Node.js is required for development and building, not for serving the finished emulator.

---

## 7. Repository Structure

```text
.   # repository root (x86WASM)
|-- Cargo.toml
|-- AGENTS.md
|-- plan.md
|-- README.md
|-- LICENSE
|-- third_party/
|   `-- NOTICE
|
|-- .cursor/
|   |-- rules/
|   |   |-- emulator-core.mdc
|   |   |-- testing.mdc
|   |   |-- vibe-sessions.mdc
|   |   |-- rust-core.mdc
|   |   |-- web-boundary.mdc
|   |   |-- licensing.mdc
|   |   `-- instruction-metadata.mdc
|   `-- skills/
|       |-- next-slice/
|       |-- implement-instruction/
|       |-- implement-device/
|       |-- guest-boot-debug/
|       `-- quality-gate/
|
|-- crates/
|   |-- x86-spec/             # Declarative instruction definitions
|   |-- x86-decode/           # Prefix, opcode, ModRM, and SIB decoder
|   |-- x86-core/             # CPU state, modes, exceptions, privileges
|   |-- x86-mmu/              # Segmentation, paging, TLB
|   |-- x86-ir/               # Typed internal representation
|   |-- x86-interpreter/      # Reference execution engine
|   |-- x86-jit-wasm/         # IR to WebAssembly JIT
|   |-- machine-pc/           # Complete virtual PC
|   |-- devices/              # PIC, PIT, IDE, VGA, APIC, and others
|   |-- firmware-interface/   # fw_cfg, ACPI, firmware loading
|   |-- emulator-cli/         # Native command-line runner
|   `-- emulator-web/         # WebAssembly exports
|
|-- web/
|   |-- src/
|   |   |-- worker/
|   |   |-- display/
|   |   |-- audio/
|   |   |-- storage/
|   |   |-- network/
|   |   |-- debugger/
|   |   `-- v86-compat/
|   `-- tests/
|
|-- tests/
|   |-- instruction/
|   |-- differential/
|   |-- paging/
|   |-- firmware/
|   |-- devices/
|   |-- boot/
|   `-- performance/
|
|-- firmware/
|   |-- manifests/
|   |-- build-scripts/
|   |-- seabios/
|   |-- seavgabios/
|   `-- ovmf/
|
|-- docs/
|   |-- architecture.md
|   |-- scope.md
|   |-- cpu-profile-core2.md
|   |-- machine-model-pc-v1.md
|   |-- instruction-format.md
|   |-- testing.md
|   |-- licensing.md
|   |-- sources.md
|   `-- adr/
|
`-- tools/
    |-- xed-oracle/
    |-- qemu-oracle/
    |-- rom-builder/
    `-- trace-diff/
```

---

## 8. High-Level Architecture

```text
Browser UI
    |
    |-- Screen
    |-- Keyboard
    |-- Mouse
    |-- Audio
    |-- Disk and ISO backends
    |-- Network relay
    `-- v86-compatible JavaScript API
                |
                v
        Web Worker and Rust/Wasm
                |
        +------- Machine -------+
        | vCPU 0                |
        | vCPU 1                |
        | Physical memory       |
        | I/O bus               |
        | MMIO bus              |
        | Timers                |
        | Devices               |
        +-----------------------+
                |
        x86 instruction bytes
                |
          Generated decoder
                |
        Typed internal IR
          +-----+-----+
          |           |
    Interpreter    Wasm JIT
    reference      optimized
```

### Critical architectural rule

All CPU registers and architectural addresses must use 64-bit-capable representations from the first commit, even while the first code only runs in 16-bit real mode.

Do not build a 32-bit CPU core and plan to extend it later.

---

## 9. CPU State Design

The initial CPU state should account for all major architectural categories:

```rust
pub struct CpuState {
    // General-purpose state
    pub gpr: [u64; 16],
    pub rip: u64,
    pub rflags: u64,

    // Segmentation and descriptor tables
    pub segments: SegmentState,
    pub gdtr: DescriptorTable,
    pub idtr: DescriptorTable,
    pub ldtr: SystemSegment,
    pub tr: SystemSegment,

    // Control registers
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,

    // Model-specific registers
    pub efer: u64,
    pub star: u64,
    pub lstar: u64,
    pub cstar: u64,
    pub sfmask: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub kernel_gs_base: u64,

    // Floating-point and SIMD
    pub x87: X87State,
    pub xmm: [u128; 16],
    pub mxcsr: u32,

    // Timing and interrupts
    pub tsc: u64,
    pub local_apic: LocalApicState,

    // Internal execution state
    pub halted: bool,
    pub pending_exception: Option<Exception>,
}
```

The exact field layout can change, but every architectural category must be represented deliberately.

---

## 10. Instruction Decoder

### 10.1 Design rule

Do not manually implement thousands of instruction patterns as large nested match statements.

Use declarative instruction metadata and generate decoder tables and related code.

Example metadata:

```yaml
- mnemonic: ADD
  opcode_map: primary
  opcode: 0x01
  modrm: required
  operands:
    - type: rm
      width: operand_size
      access: read_write
    - type: reg
      width: operand_size
      access: read
  flags:
    written: [CF, PF, AF, ZF, SF, OF]
  modes: [real, protected, compatibility, long]
  feature: base
  semantic: add
```

### 10.2 Generated outputs

The build-time generator should produce:

- Decoder tables
- Operand extraction code
- Disassembler metadata
- Instruction coverage reports
- Interpreter dispatch metadata
- JIT lowering skeletons
- Test-case templates
- Feature-profile validation

### 10.3 Decoder requirements

The decoder must eventually support:

- Legacy prefixes
- Operand-size override
- Address-size override
- Segment overrides
- LOCK
- REP
- REPE
- REPNE
- REX prefixes
- Primary opcode map
- 0F opcode map
- 0F 38 opcode map
- 0F 3A opcode map
- ModRM
- SIB
- 16-bit addressing
- 32-bit addressing
- 64-bit addressing
- Displacements
- Immediates
- Register-high-byte restrictions under REX
- Instruction-length limit handling
- Invalid instruction detection
- Truncated instruction handling
- CPU feature gating
- CPU mode gating

### 10.4 Decoder validation

Use Intel XED as a native test oracle for:

- Instruction length
- Mnemonic
- Operand count
- Operand type
- Register selection
- Effective operand size
- Effective address size
- Immediate values
- Displacement
- Mode validity

Intel XED must not be required in the browser runtime.

---

## 11. Interpreter

The interpreter is the permanent reference implementation, not temporary prototype code.

### 11.1 Interpreter responsibilities

- Decode one instruction or a bounded block.
- Validate CPU mode and feature requirements.
- Calculate effective addresses.
- Apply segmentation.
- Perform virtual-to-physical translation.
- Execute instruction semantics.
- Update flags.
- Deliver precise exceptions.
- Check interrupts at defined boundaries.
- Maintain deterministic behavior.

### 11.2 Required tests per instruction

Each instruction implementation should test:

1. Normal result
2. Every modified flag
3. Preserved flags
4. Undefined flag handling policy
5. 8-bit width
6. 16-bit width
7. 32-bit width
8. 64-bit width where applicable
9. Register form
10. Memory form
11. Cross-page memory form
12. Segment-limit behavior
13. Page-fault behavior
14. Privilege checks
15. Alignment behavior
16. Exception ordering
17. Instruction-pointer update behavior
18. Valid CPU modes
19. Invalid CPU modes
20. Feature-profile gating

### 11.3 Semantic ownership

Instruction semantics should have one authoritative implementation or one authoritative semantic description used by:

- Interpreter execution
- JIT lowering
- Constant folding
- Differential tests
- Instruction documentation

---

## 12. Internal Representation

Do not generate WebAssembly directly from scattered decoder code. Introduce a small typed intermediate representation.

Example:

```text
block 0x7C00:
    r0 = read_reg16(AX)
    r1 = load16(DS, BX)
    r2 = add16(r0, r1)
    write_reg16(AX, r2)
    set_lazy_flags_add16(r0, r1, r2)
    branch_if_zero(target_7C20, target_7C05)
```

The IR should represent:

- Integer operations
- Floating-point helper calls
- Register reads and writes
- Guest virtual-memory operations
- Guest physical-memory operations
- Port I/O
- MMIO
- Lazy flags
- Explicit exception exits
- Interrupt checkpoints
- Conditional branches
- Direct branches
- Indirect branches
- Translation-block exits
- Slow helper calls

The IR should have a validator to detect malformed or unsafe blocks before JIT generation.

---

## 13. WebAssembly JIT

### 13.1 JIT sequence

1. Begin execution in the interpreter.
2. Count translation-block entries.
3. Mark frequently executed blocks as hot.
4. Decode a bounded basic block.
5. Lower it to typed IR.
6. Apply safe local optimizations.
7. Generate a WebAssembly module.
8. Compile and cache the module.
9. Execute the compiled block on future entries.
10. Return to the dispatcher for interrupts, exceptions, device events, or invalidation.

### 13.2 Translation-block key

A compiled block must be keyed by more than RIP.

Include at least:

- RIP
- CPU mode
- CS attributes
- Default operand size
- Default address size
- Current privilege level
- Relevant CR0 bits
- Relevant CR4 bits
- Relevant EFER bits
- Paging generation
- Code-page generation
- CPU feature profile

### 13.3 Required JIT features

- Hot-block profiling
- Typed Wasm generation
- Software TLB integration
- Fast RAM path
- MMIO slow path
- Lazy flags
- Register caching in Wasm locals
- Controlled state spill at block exits
- Translation-block cache
- Code-page generation counters
- Self-modifying-code invalidation
- Bounded LRU cache
- Direct block dispatch where practical
- Automatic fallback to interpreter
- JIT statistics
- Debug mode with JIT disabled

### 13.4 JIT correctness rule

Every JIT block must be testable against the interpreter from the same initial machine state.

A JIT mismatch is a release-blocking defect.

---

## 14. Memory and MMU

### 14.1 General design

A 64-bit guest does not require more than 4 GiB of host WebAssembly memory.

The emulator can:

1. Store guest virtual addresses in u64 values.
2. Walk guest page tables in software.
3. Produce a guest physical address.
4. Map that guest physical address into a smaller physical RAM array.

### 14.2 Initial physical-memory targets

- Minimum test configuration: 128 MiB
- Legacy default: 256 MiB
- x64 default: 1 GiB
- Experimental upper setting: 2 GiB

Do not make Memory64 a blocker for the first x86-64 release.

### 14.3 MMU implementation order

1. No paging
2. 32-bit paging
3. 32-bit paging with 4 MiB pages
4. PAE paging
5. Four-level long-mode paging
6. NX permissions
7. Global pages
8. Accessed-bit updates
9. Dirty-bit updates
10. INVLPG
11. CR3 reload invalidation
12. Large pages
13. Cross-page fetches
14. Cross-page reads
15. Cross-page writes
16. Canonical-address checks

### 14.4 Page-fault accuracy

Every page fault must produce the correct:

- CR2 value
- Present bit
- Read or write bit
- User or supervisor bit
- Reserved-bit violation bit
- Instruction-fetch bit where applicable
- Exception vector
- Exception error code
- Fault ordering

---

## 15. Virtual PC Machine Model

Start with a QEMU-compatible i440FX and PIIX3-style subset.

This is a compatibility target, not permission to copy implementation code.

### 15.1 Initial machine name

```text
pc-i440fx-v1
```

### 15.2 Foundation devices

- Physical RAM
- ROM regions
- Port I/O bus
- MMIO bus
- A20 gate ? **partial:** `PhysMem` bit20 mask; 8042 output-port bit1 (`0xD1`) and System Control Port A (`0x92`) bit1 Fast Gate A20 (mirrored on `MachineBus`)
- Port 0x92 ? **partial:** `devices::Port92` on `MachineBus`; bit1 Fast Gate A20 ? `PhysMem` (mirrored to 8042); bit0 write-1 fast reset ? `take_system_reset_request` / `Machine::service_8042_pulse_reset` (same latch as 8042 `0xFE`); other bits RMW store only
- Reset control ? **partial:** 8042 pulse-reset `0xFE` + port `0x92` bit0 ? `Machine::reset` after each `step`
- ISA bus
- PCI configuration space
- i440FX host bridge subset
- PIIX3 ISA bridge subset
- fw_cfg interface

### 15.3 Interrupt and timer devices

- 8259 PIC
- 8254 PIT
- CMOS and RTC
- Local APIC
- I/O APIC
- APIC timer
- HPET
- TSC
- ACPI power-management timer

### 15.4 Input devices

- 8042 controller
- PS/2 keyboard
- PS/2 mouse
- Browser pointer lock
- Optional later USB tablet

### 15.5 Storage devices

Initial:

- Floppy controller
- PIIX IDE
- ATA hard disk
- ATAPI CD-ROM
- Raw disk images
- ISO images

Later:

- HTTP range-backed images
- Copy-on-write overlays
- IndexedDB or OPFS persistent disks
- AHCI
- VirtIO block

### 15.6 Display devices

Initial:

- VGA text mode
- Planar VGA modes
- Bochs VBE-compatible linear framebuffer
- Canvas renderer
- Scaling
- Fullscreen
- Screenshot API

Later:

- OffscreenCanvas rendering
- Additional VBE modes
- Damage tracking
- Optional WebGL or WebGPU blitting without guest 3D acceleration

### 15.7 Debug devices

- COM1 serial port (`0x3F8`?`0x3FF`, 16550 THR/RBR/LSR stub)
- COM2 serial port (`0x2F8`?`0x2FF`, same stub / separate sink)
- Debug port 0x402
- Guest log capture
- Register viewer
- Memory viewer
- I/O trace
- MMIO trace
- Breakpoints
- Single-step mode
- Translation-block inspector

### 15.8 Networking devices

Recommended order:

1. NE2000 for older operating systems
2. E1000 for newer operating systems
3. VirtIO network for optimized guests

Browser networking should use a controlled relay, not direct raw Ethernet access.

### 15.9 Audio devices

Recommended order:

1. PC speaker
2. Sound Blaster 16
3. AC97 or Intel HDA

Use AudioWorklet for low-latency browser audio.

### 15.10 Later modern machine

After pc-i440fx-v1 is stable, define:

```text
pc-q35-v1
```

Potential components:

- Q35 or ICH9-style chipset
- PCI Express
- AHCI
- HDA
- EHCI
- UEFI-first boot
- TPM 2.0 only if Windows 11 becomes a real target

Do not mix both machine models into the initial implementation.

---

## 16. Firmware Strategy

### 16.1 Legacy BIOS

Use:

- SeaBIOS
- SeaVGABIOS or another compatible VGA option ROM

### 16.2 UEFI

Use:

- OVMF from EDK II

### 16.3 Firmware interface

Implement a QEMU-compatible subset of fw_cfg so firmware can receive:

- RAM size
- CPU count
- Boot order
- ACPI information
- SMBIOS information
- Optional kernel or initrd metadata
- Machine-specific data

### 16.4 Firmware separation

Firmware files should remain separate from the emulator core:

```text
firmware/
|-- seabios/
|   |-- manifest.json
|   `-- bios.bin
|-- seavgabios/
|   `-- vgabios.bin
`-- ovmf/
    |-- OVMF_CODE.fd
    `-- OVMF_VARS_TEMPLATE.fd
```

### 16.5 Licensing requirements

- Preserve firmware licenses.
- Preserve source locations.
- Maintain third_party/NOTICE.
- Record exact firmware build revisions.
- Do not embed third-party firmware without documented licensing review.

---

## 17. Dual-Core and SMP Design

Design the machine for multiple virtual CPUs from the beginning:

```rust
pub struct Machine {
    pub vcpus: Vec<Vcpu>,
    pub memory: PhysicalMemory,
    pub devices: DeviceSet,
    pub scheduler: Scheduler,
}
```

Expose one virtual CPU until single-core execution is stable.

### 17.1 First SMP implementation

Use deterministic cooperative scheduling:

```text
vCPU 0: execute N instructions
process device events
vCPU 1: execute N instructions
process device events
repeat
```

Benefits:

- Reproducible failures
- Easier debugging
- Simpler snapshots
- Simpler atomic instruction model
- No immediate SharedArrayBuffer dependency
- Guest can still detect and use two CPUs

This provides functional dual-core support but not true parallel speedup.

### 17.2 Required SMP features

- Local APIC per virtual CPU
- Unique APIC IDs
- ACPI MADT
- INIT IPI
- SIPI
- Application processor startup vector
- Interprocessor interrupts
- TLB shootdown behavior
- APIC timers
- LOCK semantics
- Atomic read-modify-write operations
- MFENCE
- LFENCE
- SFENCE
- Shared-memory ordering
- HLT wake-up behavior

### 17.3 Later parallel execution

Optional later implementation:

- Shared WebAssembly memory
- SharedArrayBuffer
- One Web Worker per virtual CPU
- Atomic synchronization
- Controlled device ownership
- Parallel scheduler
- Retained deterministic mode for debugging

Parallel execution may require cross-origin isolation headers such as:

```apache
<IfModule mod_headers.c>
    Header always set Cross-Origin-Opener-Policy "same-origin"
    Header always set Cross-Origin-Embedder-Policy "require-corp"
</IfModule>
```

All related scripts, workers, firmware, and disk resources must be served with compatible origin and resource policies.

---

## 18. Browser API

Provide two API layers:

```text
NativeEmulatorAPI
V86StarterCompat
```

### 18.1 Native API example

```typescript
interface Emulator {
    start(): Promise<void>;
    pause(): void;
    resume(): void;
    reset(): void;
    stop(): void;

    mountFloppy(image: DiskSource): Promise<void>;
    mountCdrom(image: DiskSource): Promise<void>;
    mountHardDisk(index: number, image: DiskSource): Promise<void>;
    eject(device: StorageDevice): void;

    saveState(): Promise<ArrayBuffer>;
    restoreState(state: ArrayBuffer): Promise<void>;

    sendScancode(code: number): void;
    sendText(text: string): void;

    setScale(x: number, y: number): void;
    requestFullscreen(): Promise<void>;
    captureScreenshot(): Promise<Blob>;

    addEventListener(name: string, handler: EventListener): void;
    removeEventListener(name: string, handler: EventListener): void;

    getStatistics(): EmulatorStatistics;
}
```

### 18.2 Required browser events

- emulator-ready
- emulator-started
- emulator-paused
- emulator-resumed
- emulator-stopped
- screen-size-changed
- serial-output
- boot-progress
- storage-attached
- storage-error
- network-status
- audio-status
- snapshot-created
- snapshot-restored
- fatal-error

### 18.3 v86 compatibility adapter

The compatibility layer should translate common v86 configuration options into the new emulator configuration.

Initial compatibility priorities:

- BIOS URL
- VGA BIOS URL
- Memory size
- VGA memory size
- Hard disk URL
- CD-ROM URL
- Floppy URL
- Autostart
- Screen container
- Serial output
- Save and restore state
- Basic event names

---

## 19. Integration with oses.ioblako.com

Keep v86 and the new emulator available at the same time.

Example existing engine entry:

```json
{
  "name": "Windows 98",
  "engine": "v86"
}
```

Example new engine entry:

```json
{
  "name": "Windows 7 x64",
  "engine": "new-x86",
  "cpuProfile": "core2-penryn",
  "memoryMb": 1024,
  "vcpus": 1,
  "firmware": "seabios",
  "machine": "pc-i440fx-v1"
}
```

Migration strategy:

1. Keep all existing v86 operating systems unchanged.
2. Add the new engine behind a feature flag.
3. Add development-only test systems.
4. Migrate one legacy OS at a time.
5. Add x64-only systems that v86 cannot run.
6. Compare boot behavior and performance.
7. Retain a fallback to v86 until parity is proven.

---

## 20. Feature-Parity Matrix

| Capability | Initial | Later |
|---|---:|---:|
| Legacy BIOS boot | Yes | |
| UEFI boot | | Yes |
| Floppy support | Yes | |
| IDE hard disk | Yes | |
| ATAPI CD-ROM and ISO | Yes | |
| Raw disk upload | Yes | |
| HTTP range disk | | Yes |
| Persistent writable disk | | Yes |
| VGA text mode | Yes | |
| VGA graphics | Yes | |
| VBE linear framebuffer | Yes | |
| Fullscreen and scaling | Yes | |
| Keyboard and mouse | Yes | |
| Serial console | Yes | |
| Save and restore state | | Yes |
| Networking | | Yes |
| Sound Blaster | | Yes |
| AC97 or HDA | | Yes |
| Shared filesystem | | Yes |
| x86-64 | | Yes |
| Two virtual CPUs | | Yes |
| Basic debugger | Yes | |
| Advanced debugger | | Yes |
| Execution record and replay | | Optional |
| v86 API adapter | Basic | Broad |

---

## 21. Development Roadmap

The estimates below assume solo development with Cursor assistance (many small vibe-coded slices). They include architecture, implementation, testing, debugging, and integration. Calendar time shrinks if slices stay tiny and tests stay honest; it expands if prompts become milestone-sized.

### Milestone 0: Architecture and scope

Estimated effort: 2 to 3 weeks

Deliverables:

- docs/scope.md
- docs/architecture.md
- Core 2 CPU profile
- Machine-model ADR
- Interpreter and JIT ADR
- SMP ADR
- Firmware decision
- Licensing policy
- Source-provenance policy
- Operating-system test matrix
- Definition of done for each milestone
- Cursor rules, skills, and `AGENTS.md`
- CI skeleton
- Empty native CLI
- Empty browser Wasm module

Exit criteria:

- No unresolved foundational decision about CPU state, decoder format, machine model, firmware, JIT architecture, or testing.
- Native and browser targets build successfully.
- A new Cursor chat can run `next-slice` and get a concrete first implementation task.

### Milestone 1: CPU laboratory and minimal machine

Estimated effort: 6 to 8 weeks

Implement:

- Cargo workspace
- CPU state
- Reset values
- Physical RAM
- ROM mapping
- Port I/O bus
- MMIO bus
- Reset vector
- Minimal instruction fetch
- Prefix parser framework
- Opcode decoder framework
- COM1 serial port
- COM2 serial port (same 16550 stub; added in M2)
- Debug port 0x402
- Custom ROM test harness
- Native and browser execution

Initial instruction subset:

- MOV
- XOR
- ADD
- SUB
- CMP
- TEST
- JMP
- Basic conditional jumps
- CALL
- RET
- PUSH
- POP
- IN
- OUT
- CLI
- STI
- HLT

Exit criteria:

- A custom reset ROM starts at the x86 reset vector.
- It prints `HELLO FROM EMULATOR` through serial and debug port.
- It works in both the native CLI and browser.
- All implemented instructions have tests.

### Milestone 2: 16-bit and 32-bit interpreter plus legacy PC

Estimated effort: 3 to 5 months

Status (2026-08-11, round-14 platform-kbd lane `slice/r14-platform-kbd` on base `9934398`): **in progress -- R14 platform-kbd lane.** Host INT 16h AH=02h shift flags from BDA `40:17`; AH=12h extended shift (Table 00588 synth from `40:18`+`40:96`); BDA equipment/keyboard flag seed for FreeDOS (bit0=floppy honesty, bit2=keyboard); 8042/IRQ1 modifier→BDA flags + ring-full drain + LED mirror. See `docs/kbd-r14-int16-shift-status.md`, `docs/kbd-r14-int16-ext-shift.md`, `docs/platform-r14-bda-equipment-kbd.md`, `docs/kbd-r14-8042-irq1-polish.md`. **Not** guest INT 09h/16h body; no buffer-full beep; no full AT keyboard; no mouse.

Status (2026-08-11, round-14 usb-timer lane `slice/r14-usb-timer` on base `9934398`): **in progress -- R14 usb-timer lane.** UHCI→PIRQD→classic ISA IRQ11 PIC wire; IOC/USBSTS path raises wired IRQ; LAPIC SVR Focus presence + EOI-suppress drop (no CPUID.APIC); HPET LEG_RT_CAP/CNF clear so IRQ0/IRQ8 stay PIT/CMOS (MSI still out). See `docs/usb-r14-*.md`, `docs/apic-r14-lapic-svr.md`, `docs/timer-r14-hpet-legacy.md`.

Status (2026-08-10, round-13 boot-guest lane `slice/r13-boot-guest` on base `f579e8c`): **in progress -- R13 boot-guest lane.** INT19-candidate HD/floppy attach helpers; FreeDOS measure v6 media readiness (beyond no-media reboot loop); INT 13h AH=00h HD+floppy deepen (IDE/FDC reset + BDA status); Linux bzImage setup deepen (`cmd_line_ptr`/`init_size`) + El Torito media boot classify. See `docs/boot-r13-*.md`, `docs/storage-r13-*.md`. **Not** FreeDOS prompt / Linux shell; host INT 13h still not SeaBIOS.

Status (2026-08-10, round-13 platform-io lane `slice/r13-platform-io` on base `f579e8c`): **in progress -- R13 platform-io lane.** LPT1 control idle default `0x0C` deepen; LPT2 independence + LPT3 `0x3BC` open-bus honesty; COM3/COM4 `0x3E8`/`0x2E8` 16550 probe stubs (IER sites claimed; no shared IRQ); CMOS `0Fh` `set_shutdown_status` + Machine helpers survive CF9 pulse-reset without dispatch. See `docs/lpt-r13-*.md`, `docs/platform-r13-*.md`. **Not** IRQ7/ECP; COM3/4 IRQ share; Machine soft-reset JMP via `40:67`.


Status (2026-08-10, round-13 parallel integration `merge/m2-r13-parallel-16` on base `f579e8c`): **in progress -- thirteenth parallel campaign merged.** Four lanes: LPT1/2 + COM3/4 probe stubs + CMOS 0Fh; INT10 AH=0E/13 + CRTC cursor sync + VBE 4F01 no-LFB; INT19 bootable-media helpers + FreeDOS v6 media measure + INT13 AH=00 + Linux/El Torito deepen; VME CLI/STI VIF + PUSHF/POPF + redirect deepen + INT3. See `docs/m2-r13-parallel-integration.md`. **CF9 remains; no-media POST still `F000:9842`.** FreeDOS prompt / Linux serial shell still open. Honesty: no guest LFB; no CPUID.VME/APIC; ADR-0008 table-loader absent.
Status (2026-08-10, round-12 parallel integration `merge/m2-r12-parallel-16` on base `01050d2`): **in progress -- twelfth parallel campaign merged (+ POST CF9).** Five lanes: ICH `0xCF9` reset (`F000:C897` was `wait_irq`; causal miss was CF9/`qemu_reboot`); UHCI QH depth4 + USBSTS/USBINTR + LAPIC ICR + HPET MSI honesty; INT10 AH=01/09/0A + BDA video + VBE 4F00 no-LFB; INT13 AH=04/41 + FreeDOS/Linux measure deepen; CR4.VME sticky + redirect bitmap + 16-bit IDT from VM86 + INTO/VME. See `docs/m2-r12-parallel-integration.md`, `docs/post-c897-*.md`. **20M POST remeasure stops at `F000:9842` no-media reboot loop (past C897).** FreeDOS prompt / Linux serial shell still open. Honesty: no guest LFB; no CPUID.VME/APIC; ADR-0008 table-loader absent.
Status (2026-08-10, round-13 cpu-vm86 lane `slice/r13-cpu-vm86` on base `f579e8c`): **in progress -- R13 cpu-vm86 lane.** VME `CLI`/`STI` → `VIF`; `PUSHF`/`POPF` method-4; soft-int redirect method-5/6 deepen; `INT3` ignores redirect bitmap (TF/#DB under VME skipped); still **no** `CPUID.VME`. See `docs/cpu-r13-*.md`.
Status (2026-08-10, round-12 cpu-vm86 lane `slice/r12-cpu-vm86` on base `01050d2`): **in progress -- R12 cpu-vm86 lane.** `CR4.VME` sticky without `CPUID.VME`; soft-int TSS redirect-bitmap stub; 16-bit IDT gate from VM86 (9-word frame); `INTO`/#OF IOPL+bitmap honesty vs `INT n`. See `docs/cpu-r12-*.md`. **Not** full VME (no VIF/`CLI`/`STI`, no `CPUID.VME`); POST still open.

Status (2026-08-10, round-12 boot-guest lane `slice/r12-boot-guest` on base `01050d2`): **in progress -- R12 boot-guest lane.** INT 13h AH=04h VERIFY (HD+floppy), AH=41h CX bitmap honesty (packet+EDD only; AH=44/47 rejected), FreeDOS measure next-gap classify (v5: INT13/BDA/IVT/image), Linux bzImage early classify + real-mode setup load helper. See `docs/storage-r12-*.md`, `docs/boot-r12-*.md`. **Not** FreeDOS prompt / Linux shell; host INT 13h still not SeaBIOS; no POST C897 work in this lane.

Status (2026-08-10, round-12 display-fw lane `slice/r12-display-fw` on base `01050d2`): **in progress -- R12 display-fw lane.** Host INT 10h AH=01h cursor type (BDA+CRTC), AH=09h/0Ah write char/attr, BDA columns/page/CRT-base/rows polish, VBE AX=4F00h controller-info deepen with Capabilities/VideoModePtr honesty and **no guest LFB**. See `docs/vga-r12-*.md`, `docs/firmware-r12-vbe-host-stub.md`. **Not** SeaVGABIOS VBE body; PhysBasePtr stays zero.

Status (2026-08-11, round-14 display-fw lane `slice/r14-display-fw` on base `9934398`): **in progress -- R14 display-fw lane.** Host INT 10h AH=06h/07h scroll window, AH=08h read char/attr, VBE AX=4F02h set-mode for `03h`/`13h` with LFB-bit fail honesty, mode-03 bring-up font + Max Scan Line polish + CRTC Start Address setter; **no guest LFB**. See `docs/vga-r14-*.md`. **Not** SeaVGABIOS VBE body; planar VBE set-mode still fail; PhysBasePtr stays zero; bare `reset()` still installs no font.

Status (2026-08-10, round-13 display-fw lane `slice/r13-display-fw` on base `f579e8c`): **in progress -- R13 display-fw lane.** Host INT 10h AH=0Eh teletype scroll/attr deepen, AH=13h write-string stub (bounded), CRTC↔BDA cursor Location sync after writes, VBE AX=4F01h mode-info delivery with OffScreenMem*/PhysBasePtr honesty and **no guest LFB**. See `docs/vga-r13-*.md`. **Not** SeaVGABIOS VBE body; no LFB aperture.

Status (2026-08-10, round-12 usb-timer lane `slice/r12-usb-timer` on base `01050d2`): **in progress -- R12 usb-timer lane.** UHCI QH horizontal depth 4 + USBSTS/USBINTR gating; LAPIC ICR presence/self-IPI stub; HPET FSB/MSI capability clear + IRQ-route honesty. See `docs/uhci-r12-*.md`, `docs/lapic-r12-icr.md`, `docs/hpet-r12-msi-irq.md`. **Not** UHCI?PIC, multi-APIC IPI, HPET MSI delivery; CPUID APIC stays clear.

Status (2026-08-10, round-11 parallel integration `merge/m2-r11-parallel-16` on base `b46e5f0`): **in progress -- eleventh parallel campaign merged.** Four agents landed 16 bounded slices: UHCI frame-list/PORTSC + LAPIC LVT + HPET wrap; BDA keyboard ring + APM INT15 AH=53 stub + port61 NMI + POST remeasure; INT13 AH=4A/4B + HD AH=08 + FreeDOS BDA seed + Linux header inspect; VM86 PUSH/POP Sreg + VIP/VIF-without-VME + INT3/ICEBP + opsize-32 far transfers. See `docs/m2-r11-parallel-integration.md`. **SeaBIOS POST still does not complete; every M2 exit criterion remains open.** Honesty that must survive: no real SMM; no guest LFB; `CR4.VME` sticky only via R12 cpu lane (no `CPUID.VME`); FreeDOS/Linux stubs not prompt/shell; ADR-0008 table-loader absent; POST historically `F000:C897` until R12 CF9.


Status (2026-08-10, round-11 boot-guest lane `slice/r11-boot-guest` on base `b46e5f0`): **in progress -- R11 boot-guest lane.** INT 13h AH=4Ah/4Bh El Torito terminate+status (AL corrected to El Torito/RBIL), HD AH=08h media-required CHS deepen, FreeDOS measure BDA equipment seed + host-notes on synthetic-halt, Linux boot-protocol header inspect helper. See `docs/storage-r11-*.md`, `docs/boot-r11-*.md`. **Not** FreeDOS prompt / Linux shell; host INT 13h still not SeaBIOS.

Status (2026-08-10, round-10 parallel integration `merge/m2-r10-parallel-16` on base `d832290`): **in progress -- tenth parallel campaign merged.** Four agents landed 16 bounded slices: LAPIC TMR + TPR/PPR + HPET?IOAPIC Fixed wire + IOAPIC non-Fixed honesty; host INT 10h AH=02/03/0F + BDA video polish + option-ROM checksum honesty; INT 13h floppy AH=08/15 + CD El Torito AH=41/42/48/4B + FreeDOS/Linux first-failure classify; VM86?CPL0 9-dword INT frame + far JMP/CALL/RETF while VM=1 + INT n IOPL. See `docs/m2-r10-parallel-integration.md`. **SeaBIOS POST still does not complete; every M2 exit criterion remains open.** Honesty that must survive: host INT 10h/13h/16h are not SeaBIOS; no guest LFB; VME/PVI still out; no real SMM; FreeDOS/Linux measure stubs classify `synthetic-halt` only (not prompt/shell); `etc/table-loader` intentionally absent (ADR-0008); CPUID APIC bit remains clear.

Status (2026-08-10, round-9 parallel integration `merge/m2-r9-parallel-16` on base `dbf1dcb`, AH=48 join from R8 tip `f9af9d9`/`5ce989d`): **in progress -- ninth parallel campaign merged + INT 13h AH=48h EDD joined.** Four agents landed 16 bounded slices: PM_TMR freerun + APM deepen + INT 16h/8042 queue + port61 speaker/refresh; VBE PhysBasePtr honesty + INT 10h stub + option-ROM POST scan + SeaVGABIOS Linux smoke; INT 13h AH=43 + floppy AH=02/03 + FreeDOS/Linux measure harnesses; VM86 IRETD enter + CLI/STI/PUSHF/POPF IOPL + VM86 IRET exit. Cherry-picked AH=48h EDD (`5ce989d`/`deb030a`) from alternate R8 tip `f9af9d9` onto R9 (R9 base `dbf1dcb` lacked it). See `docs/m2-r9-parallel-integration.md`. **SeaBIOS POST still does not complete; every M2 exit criterion remains open.** Honesty that must survive: host INT 10h/13h/16h are not SeaBIOS; no guest LFB; VM86 interrupt-from-VM86 frame still out (addressed in R10); no real SMM; FreeDOS/Linux harnesses do not claim prompt/shell; SeaVGABIOS Windows build infeasible; `etc/table-loader` intentionally absent (ADR-0008); AH=48 is EDD v1.x subset only.

Status (2026-08-10, round-8 parallel integration `merge/m2-r8-parallel-16` tip `f9af9d9` / R9-base `dbf1dcb` on base `8249092`): **in progress -- eighth parallel campaign merged.** Four agents landed 16 bounded slices: UHCI one-TD + LAPIC EOI/ISR + HPET periodic + IOAPIC Remote IRR; PM1a sleep + PCI Status errors + SCI/PIRQ + fw_cfg system-states; INT 13h write/extensions (incl. AH=48h EDD on tip `f9af9d9`) + El Torito load->0x7C00 + guest measure v2; CALL-form TSS + IRET NT + ARPL + paging A/D. See `docs/m2-r8-parallel-integration.md` (dual-tip note). **SeaBIOS POST still does not complete; every M2 exit criterion remains open.** Honesty that must survive: VM86 enter/leave deferred (ARPL only); El Torito is host load not guest CD BIOS; INT 13h is host-installed subset; UHCI one-TD only; `etc/table-loader` intentionally absent (ADR-0008); SeaVGABIOS Windows build infeasible; no real SMM.

Status (2026-08-10, round-7 parallel integration `merge/m2-r7-parallel-16` on base `def88ae`): **in progress ? seventh parallel campaign merged.** Four agents landed 15 bounded slices (UHCI one-TD skipped): HPET comparator IRQ + LAPIC timer/LVT + IOAPIC Fixed RTE; option-ROM far-call + bring-up font + host VGA frame; INT 13h HD subset + floppy?0x7C00 + UART RX/RDA + guest measure harness; JMP TSS/task-gate + call-gate + VERR/VERW + LLDT/LDT call-gate. See `docs/m2-r7-parallel-integration.md`. **SeaBIOS POST still does not complete; every M2 exit criterion remains open.** Honesty that must survive: El Torito remains inspect-only (no `load_eltorito_to_7c00`); UHCI deferred; SeaVGABIOS Windows build infeasible; INT 13h is host-installed HD subset only; `etc/table-loader` intentionally absent (ADR-0008); no real SMM.

Status (2026-08-10, round-6 parallel integration `merge/m2-r6-parallel-16` on base `0a2bd20`): **in progress ? sixth parallel campaign merged.** Four agents landed 16 bounded slices (LPT+LAPIC/HPET/IOAPIC presence; CMPXCHG8B+`IA32_APIC_BASE`+LAR/LSL+same-CPL far CALL; fw_cfg table-loader honesty ADR + PM1a_EN SCI + bootorder; ATAPI MODE SENSE/TOC/PREVENT + El Torito inspect-only). See `docs/m2-r6-parallel-integration.md`. **SeaBIOS POST still does not complete; every M2 exit criterion remains open.** Round-5 measured LPT/LAPIC/HPET blockers now have presence stubs ? timer IRQ / IOAPIC RTE / UHCI / guest LFB / INT 13h remain follow-ons. Honesty that must survive: APIC/HPET/IOAPIC are MMIO stubs only; `etc/table-loader` intentionally absent (ADR-0008); El Torito is host-side only; far CALL is same-CPL; no guest LFB/`PhysBasePtr`; no font at reset; no real SMM.

Status (2026-08-09, round-5 parallel integration `merge/m2-r5-parallel-16` on base `e170b7e`): **in progress ? end of the five-round parallel campaign.** Four agents landed 16 more bounded slices (privilege/TSS/#DF; APM/XBCS/PM_TMR/POST idle; SMRAM/FDHC/ACPI PM; VBE host info + ATAPI CD-ROM medium). **APM ports `0xB2`/`0xB3` are unblocked.** Post-merge measurement at 2,000,000 steps reports `busy-steps=1,276,960` (+ `halt-idle` 723,040 ? 36% idle) stopping at `cs:ip=F000:C897` (`cs.d=0`, linear `0xFC897`) with LPT probe ports (`0x378`/`0x278`/`0x3E9`/`0x2E9`) unclaimed and unmapped MMIO reads of Local APIC (`0xFEE00000`) and HPET (`0xFED00000`). **SeaBIOS POST still does not complete; every M2 exit criterion remains open** (SeaBIOS POST complete, FreeDOS prompt, 32-bit Linux serial shell). Honesty that must survive: no guest LFB/`PhysBasePtr`; ATAPI type `05h` only with READ CAPACITY/READ(10), type `1Fh` remains for the minimal packet path; no font at reset; no real SMM (APM is a handshake stub); SMRAM/FDHC PhysMem wiring landed this merge; privilege path has no task gates/VM86/call gates; POST codes/COM1 stay empty because this SeaBIOS build has diagnostics compiled out.

Status (2026-08-09, round-4 parallel integration `merge/m2-r4-parallel-16` on base `652281e`): **in progress** ? four parallel agents landed 16 more bounded slices. **Headline misattribution, independently corrected by two agents:** the write-to-ROM `#GP` storm that dominated round 3 was a CPU `moffs` address-size bug under `CS.D=1` (interpreter keyed absolute-offset width on the presence of `0x67` rather than the effective address-size attribute), **not** a machine/ROM-write model bug. With that fixed alone, SeaBIOS reached a clean halt at **150,360** steps in PCI resource assignment ? a BAR-sizing defect the machine agent diagnosed and the PCI agent independently implemented in the same round. **Post-merge measurement (moffs fixed and BAR sizing landed):** `--post-probe` / `--post-trace` / `--post-spin` at 2,000,000 steps all exhaust the budget at `cs:ip=0008:25C2` (`cs.d=1`, linear `0xF25C2`), spinning a 3-instruction cycle (~1,365 repeats in a 4096 window) after one `OUT 0xB3,1` / `OUT 0xB2,0` and **641,633** `IN 0xB3` polls that return open-bus `0xFF` ? past the BAR-sizing halt; **current first blocker is the unmodeled APM/SMI ports `0xB2`/`0xB3`**. **32-bit paging is now wired** end to end for a flat same-privilege kernel (`PagedBus`, `#PF` delivery, restartability, persistent `Machine`-held TLB). **POST still does not complete and every M2 exit criterion remains open.** Honesty that must survive: paging is not enough for a real OS (no privilege-changing gates, no TSS stack switch, no `#DF`/triple-fault); the A/D-versus-fault deviation survived contact with real execution; CPUID now advertises `PSE`/`PGE`/`CMOV` and family 6; write-to-ROM silent-drop is a model-consistency fix that buys **zero POST progress** once moffs is fixed; the `0xF0000000` sweep was the same moffs defect seen through a load; HLT with IF=1 is an idle quantum; empty post-codes/COM1 are properties of this pinned SeaBIOS build (diagnostics compiled out); BAR sizing was a measured halt cause and is now implemented ? confirmed by the post-merge probe; ATAPI still reports peripheral type `1Fh` with exactly three packet commands; planar VGA geometry is CRTC-derived; **no font at reset, by decision**, with `font_installed` reporting; fw_cfg `etc/table-loader` is unimplemented; CMOS `5Bh`?`5Dh` is de-facto standard per ADR-0006 and nothing configures >4 GB so the path is test-only.

Status (2026-08-09, round-3 parallel integration `merge/m2-r3-parallel-16` on base `0195f78`): **in progress** ? four parallel agents landed 16 more bounded slices covering accumulator port I/O and the remaining common two-byte opcodes, a standalone 32-bit paging engine, PCI configuration access widths and CMOS disk/floppy configuration, a bounded POST event trace, and the VGA character generator, unified display memory and mode 13h. **SeaBIOS POST no longer stops on any CPU instruction. It now exhausts a 2,000,000-step budget ? byte-identical at 50,000,000 ? rather than hitting a decode gap, which means it is spinning inside firmware rather than being blocked by the interpreter. POST still does not complete and every M2 exit criterion remains open.** Integration also fixed a measured architectural defect: `x86_mmu::linear_addr` summed segment base and offset in 64 bits with no truncation, so outside 64-bit mode a `CS.base = 0xFFFF0000` reference escaped the 4-GiB linear address space (SDM Vol. 3 �3.3.1). With the wrap in place the probe's two above-4-GiB page touches disappear entirely ? they resolve into ordinary low memory, one of them the top of the BIOS f-segment. The trace shows firmware now reaching **three of the four** seams round 2 left wired-but-untouched: PAM programming, CMOS, and fw_cfg are all exercised; the VGA aperture is still never touched. What is *not* true: paging is a library nothing calls, no MSR is implemented, there is no host display or VBE, no option ROM is ever executed, no SeaVGABIOS binary has been built, ATAPI is detection-only, and protected mode is still CPL 0 only with no privilege switching, call gate, LDT, or TSS.

Status (2026-08-09, round-2 parallel integration `merge/m2-r2-parallel-16` on base `133191b`): **in progress** ? four parallel agents landed 16 more bounded slices covering the two-byte `0F` opcode map, i440FX PAM and BIOS shadowing, CMOS and fw_cfg configuration data, and the VGA guest-facing MMIO entry point. **SeaBIOS POST advanced from 2 retired instructions to 17,218, and now enters 32-bit protected mode and completes CPU identification ? but POST still does not complete, and every M2 exit criterion remains open.** The new blocker is measured and recorded below. What is *not* true: no MSR is implemented, no VGA renderer or display fetch exists, no option ROM is ever executed, no SeaVGABIOS binary has been built, and protected mode is still CPL 0 only with no privilege switching, call gate, LDT, TSS, or paging.

Status (2026-08-09, round-1 parallel integration `merge/m2-r1-parallel-16` on base `d95a4f5`): four parallel agents landed 16 bounded slices covering same-CPL **32-bit** protected mode, VGA plane/Graphics-Controller memory, IDE device-selection and 48-bit PIO, FDC READ ID / implied seek, and machine glue (COM IRQ routing, fw_cfg DMA, port `0x80`, POST probe). Protected mode became default-32 capable at CPL 0 only.


Integrated 16-slice round 5 (`merge/m2-r5-parallel-16`, base `e170b7e`):

1. Host-side VBE 2.0 `VbeInfoBlock` / `ModeInfoBlock` for the six renderable VGA modes; Capabilities clear; ModeAttributes never claim LFB; `PhysBasePtr` = 0. No INT 10h. See `docs/vga-r5-vbe-info-blocks.md`.
2. Banked mode-13h host linear view via `vbe_host_linear_framebuffer` (existing `VgaFrame`); no invented guest LFB aperture. See `docs/vga-r5-vbe-banked-framebuffer.md`.
3. ATAPI CD-ROM medium model: `attach_atapi_cdrom` / image load, peripheral type `05h` + RMB, `READ CAPACITY` / `READ (10)` on 2048-byte blocks. Minimal PACKET path stays `1Fh`. See `docs/atapi-r5-cdrom-medium.md`.
4. TEST UNIT READY / sense honesty: empty CD-ROM ? NOT READY / ASC `3Ah`; loaded medium ? GOOD. See `docs/atapi-r5-medium-sense.md`.
5. `LTR` / `STR` load a 32-bit available TSS into TR (busy-bit update); no hardware task switch. See `docs/cpu-r5-tss-ltr-str.md`.
6. Privilege-changing interrupt/trap gate delivery with TSS stack switch (SDM Vol. 3 �6.12.1 Fig 6-5). No task gates / call gates / VM86. See `docs/cpu-r5-priv-gate-delivery.md`.
7. Outer-privilege `IRET` / `IRETD` restoring the outer stack. See `docs/cpu-r5-outer-iret.md`.
8. Delivery-failure escalation to `#DF` / triple-fault synthesis (Vol. 3 �6.15). See `docs/cpu-r5-double-fault.md`.
9. APM/SMI ports `0xB2`/`0xB3` store/readback + SMM handshake stub that clears the SeaBIOS poll (no real SMM). See `docs/apm-r5-smi-ports.md`.
10. ACPI `PM_TMR` freerun from the instruction-count step clock at 3� PIT into `acpi_pm_io[+8]` (joined at integration to `PciConfig::tick_acpi_pm`). See `docs/machine-r5-acpi-pm-tmr.md`.
11. PIIX XBCS `4Eh` BIOS write-protect mirrored into `PhysMem`. See `docs/machine-r5-xbcs.md`.
12. POST halt-idle accounting hardened (`--post-probe` / `--post-spin` report idle vs busy). See `docs/machine-r5-post-idle.md`.
13. i440FX SMRAM Control `0x72` host accessor + Table 4 decode; PhysMem wired at integration. See `docs/pci-r5-smram.md`.
14. i440FX FDHC `0x68` DRAM hole control; PhysMem wired at integration. See `docs/pci-r5-fdhc.md`.
15. PIIX4 ACPI PM1a stubs beyond store/readback: `PM_TMR` in `acpi_pm_io[+8]`, `tick_acpi_pm`, TMR_STS/PWRBTN_STS, SCI level poll. See `docs/pci-r5-acpi-pm.md`.
16. PCI Received Master Abort status bit on config Master-Abort. See `docs/pci-r5-master-abort-status.md`.

Integration work joined during this merge (coordinator, not a slice):

- **One PM_TMR clock path.** Replaced the APM agent's direct `acpi_pm_io` poke with `pci.tick_acpi_pm(pit_clocks * 3)` so MSB toggle can set `TMR_STS`.
- **SMRAM/FDHC PhysMem seam.** `PhysMem::apply_smram` / `apply_fdhc_hole` and `Machine::sync_*`; MachineBus CONFIG_DATA writes sync like PAM; VGA aperture skipped when SMRAM steers to DRAM.
- **Re-exports** for VBE_*, ATAPI CD-ROM consts, SmramRegion/FdhcHole/ACPI_PM_* (already partially present), APM.
- **`Machine::attach_atapi_cdrom_image`** thin wrapper (no CMOS fixed-disk claim).
- **Docs:** `docs/sources.md` and this section updated as the sole writers.

Integrated 16-slice round 4 (`merge/m2-r4-parallel-16`, base `652281e`):

1. Control-register guest state and truthful CPUID: `CR2`/`CR3`/`CR4` become guest-readable/writable at CPL 0; `CR4` reserved bits are derived from the CPUID feature word so �4.1.4 cannot drift; only `PSE` and `PGE` are settable (`PAE` refused with `#GP(0)`). CPUID leaf 1 EDX now advertises `PSE`/`MSR`/`PGE`/`CMOV` and the signature moves to family 6 so the bits and the family still agree. `INVLPG` is no longer a real-mode NOP by special case. **Not:** PAE, SMEP/SMAP, any MSR content, or privilege-changing CR3 loads via task switch.
2. Paging on the data and fetch path: `PagedBus` wraps the machine bus whether or not `CR0.PG` is set; with paging on, data, instruction fetch and descriptor-table reads translate, and a fault becomes `#PF` (vector 14) with `CR2` and the �4.7 error code through the existing 386 gate. Segment-limit `#GP`/`#SS` precede `#PF` because the linear address is formed first. **Not:** a real OS ? same-CPL delivery only, no TSS stack switch, no `#DF`/triple-fault.
3. Faulting instructions are restartable: with `CR0.PG=1` a checkpoint restores architectural state on a translation fault; `REP` publishes a new checkpoint after every completed iteration so a suspended string resumes where the SDM says. Page-straddling stores probe both halves first; `INS` probes the destination before the port read. **The A/D-versus-fault deviation survived contact with real execution** and is kept.
4. Page-crossing accesses and fetches are split so a second-half fault leaves neither the first half's bytes nor its A/D flags written; single-page accesses keep their original width on the machine bus (MMIO-friendly). Fixed in passing: `moffs` offset width follows the address-size attribute (Vol. 2 MOV), which was the write-to-ROM storm and the `0xF0000000` sweep.
5. Write-to-ROM / unclaimed-write semantics: a store the platform cannot accept completes the instruction and stores nothing (SDM Vol. 3A �6.15 has no `#GP` for "platform declined"; PCI Master-Abort is the closest bus analogy). **This is a model-consistency fix that buys zero POST progress once moffs is fixed** ? with the offset computed correctly SeaBIOS never targets ROM.
6. The `0xF0000000` sweep root-caused to the same `moffs` defect seen through a load: a truncated absolute offset turned a null-zone pointer into IVT vector 3 used as a flat address. Independently reached the same one-line interpreter fix as the paging agent.
7. POST probe reports the stop program counter and a trailing spin summary (default window 4096; cycle detection; hot PCs). Header line stays byte-identical; the spin block appears only for halt / step-budget stops. CLI `--post-spin [N]` (0 disables, implies `--post-probe`) wired at integration.
8. HLT with `IF=1` is an idle quantum that advances the step clock until IRQ0 (or another wake) rather than ending the probe; `IF=0` still stops. Diagnosed the 150,360-step halt as `CLI; HLT; JMP $-1` after PCI resource assignment rejected a garbage BAR-derived base against the I/O APIC floor ? handed to the PCI agent's BAR-sizing fix. Empty `post-codes`/`com1` are properties of this pinned SeaBIOS build (diagnostics compiled out).
9. PCI BAR sizing via the all-ones protocol (PCI 3.0 �6.2.5.1 / �6.2.5.2): implemented BARs keep high address bits writable and hardwire size bits; unimplemented BARs and the Expansion ROM BAR hardwire to zero. PIIX IDE BMIBA (16 B) and UHCI (32 B) answer correctly; model choice that bits 31:16 of I/O BARs stay writable so the documented size calculation works, with decode refused outside 16-bit I/O space.
10. Bus-0 enumeration surface made honest and read-only where �6.2.1 / �6.2.4 require it (BIST, CardBus CIS, subsystem IDs, Cap pointer, Min_Gnt/Max_Lat, Interrupt Pin). Multi-function bit on `00:01.0` remains load-bearing. Known overstatement: IDE Prog IF `0x80` still claims bus-master while no ATA command starts DMA.
11. fw_cfg CPU-count selectors and host-settable items (ADR-0005): `NB_CPUS` / `max-cpus` / `etc/max-cpus` publish `1`; UUID / nographic / `bootorder` / `etc/system-states` are host-settable and absent by default. **`etc/table-loader` is unimplemented.** `Machine::sync_firmware_configuration` publishes the CPU-count views.
12. CMOS memory above 4 GB at `5Bh`?`5Dh` per ADR-0006 (de-facto Bochs/QEMU/SeaBIOS convention, not silicon). Encoding is a 24-bit count of 64 KiB blocks. **Nothing configures >4 GB today, so the path is test-only**; e820 still publishes no above-4 GiB range.
13. ATAPI PACKET protocol: on a PACKET device `A0h` transfers a 12-byte command packet and runs TEST UNIT READY and INQUIRY; DMA/OVL Features bits and unknown opcodes abort with sense. Peripheral type remains **`1Fh`**, not `05h` CD-ROM. SFF-8020i cited for the packet set.
14. REQUEST SENSE, the sense-data model, and DEVICE RESET (`08h`) for PACKET devices. Exactly three packet commands total. ATA disks still abort `PACKET`/`A1h`.
15. Planar 16-color display fetch (modes `0Dh`/`0Eh`/`10h`/`12h`): `Graphics16Planar` only when the whole planar signature is present; geometry **derived from CRTC** End Horizontal Display / Vertical Display Enable End (unlike fixed text/mode-13h). Color Plane Enable applies on this path. No CGA modes, no mode X, no VBE.
16. Reset font question resolved: **no font, by decision** (licensing + architecture ? glyphs are guest/video-BIOS state). `VgaFrame::font_installed` / `text_font_installed` report it; `install_font_bank` lets a entitled host supply one. No `third_party/NOTICE` addition. CLI `--vga-frame` surfaces `font_installed` at integration.

Integration work joined during this merge (coordinator, not a slice):

- **Persistent TLB in `Machine`.** `Machine` holds `x86_mmu::paging::Mmu` and calls `step_with_mmu`, so translations persist across instructions; reset replaces the MMU.
- **Un-ignored the machine agent's `moffs` reproducer** (`crates/machine-pc/tests/moffs_address_size.rs`); both cases pass against the merged interpreter.
- **fw_cfg CPU-count wiring** through `Machine::sync_firmware_configuration`.
- **CLI `--post-spin` and `font_installed` reporting**; kept the ATAPI agent's `Graphics16Planar` match arm and honest "no renderer" message (items 2?4 still accurate with planar present).
- **Re-exports** in `crates/devices/src/lib.rs` for the round-4 public ATAPI/VGA/PCI/fw_cfg/CMOS items.
- **Docs:** `docs/mmu-r3-32bit-paging.md` status corrected from "standalone / not wired"; `docs/sources.md` and this section updated as the sole writers.

Integrated 16-slice round 3 (`merge/m2-r3-parallel-16`, base `0195f78`):

1. Accumulator port I/O: `IN eAX, imm8` (`E5`), `OUT imm8, eAX` (`E7`), `IN eAX, DX` (`ED`) and `OUT DX, eAX` (`EF`) at both operand sizes. Before this the primary table held only the fixed-`AL` byte forms `E4`/`E6`/`EC`/`EE`, so `OUT DX, AX` was as undecodable as `OUT DX, EAX` ? this is exactly what stopped POST at 17,218 instructions. A 16-bit `IN` writes `AX` and leaves `EAX[31:16]` untouched (SDM Vol. 1 �3.4.1.1); the `imm8` port number stays one byte at every operand size, so `E5`/`E7` reach only ports `0x00`?`0xFF`. All four route through the same width-specific `Bus::port_in_*`/`port_out_*` accessors the `INS`/`OUTS` string forms already use, so a device sees one access of the right width rather than several byte cycles. **Not:** I/O permission bitmap checks, `IOPL` enforcement, or any VM86 I/O behavior.
2. Segment `PUSH`/`POP` operand-size consistency. Round 2 left a known inconsistency: `PUSH`/`POP FS`/`GS` sized the stack slot from the operand-size attribute while the primary-map `ES`/`CS`/`SS`/`DS` forms (`06`/`0E`/`16`/`1E`, `07`/`17`/`1F`) always used a 16-bit slot. On a 32-bit stack that is live pointer corruption ? two bytes of drift per operation against any code that mixes the two encodings. All seven primary forms now take the slot width from the operand-size attribute, writing the upper half as zero on a doubleword `PUSH` and ignoring it on `POP`, with the stack-pointer width still coming from `SS.B` independently. **Not:** `POP CS`, which does not exist.
3. `CMOVcc` (`0F 40`?`0F 4F`), all sixteen conditions at both operand sizes with register and memory sources. The condition goes through the **same** low-nibble evaluator as short `Jcc`, near `Jcc` and `SETcc`, so the four families cannot disagree; a test walks all sixteen conditions against all thirty-two meaningful flag combinations and compares each to the short `Jcc` outcome. A taken 16-bit move leaves the upper half of the destination untouched and an untaken move of either width changes nothing. **CPUID leaf 1 EDX bit 15 (CMOV) deliberately stays clear** even though the instructions execute, because that bit also claims the x87 `FCMOVcc` forms, which do not exist here (ADR-0007). **Not:** the memory-operand read that real hardware performs even on an untaken move, or any byte form.
4. `SHLD`/`SHRD` (`0F A4`/`A5`/`AC`/`AD`), all four encodings at both operand sizes with register or memory destinations. The count masks to 5 bits *independently of the operand size*, so a 16-bit shift can legally receive a count of 17?31; a count masking to zero is a no-operation with no flag change. **The SDM leaves several results undefined and this tree picks deterministic ones**, because the interpreter is the JIT's semantic reference: the out-of-range 16-bit counts, `OF`, and `AF` all have stated model choices recorded in `docs/cpu-r3-port-io-and-two-byte-extensions.md`. Each is a legal instance of the undefined behavior, not a claim about silicon.
5. A 32-bit paging walk engine in `crates/x86-mmu/src/paging/`: `CR3` decode (Table 4-3), page-directory and page-table entry decode (Tables 4-4 through 4-6), and the �4.3 walk to a physical address with Figures 4-2 and 4-3 as the reference. `PageTableMemory` is a caller-supplied trait, so the crate keeps its no-browser, no-machine-pc boundary. **`CR0.PG = 0` returns `Unsupported(PagingDisabled)` rather than the identity mapping**, so a caller cannot forget the check by accident.
6. Paging access rights and page faults: �4.6.1 supervisor/user and read/write determination including `CR0.WP`, and the �4.7 Figure 4-12 error code (P, W/R, U/S, RSVD, I/D) with `CR2` carrying the faulting linear address offset included, not the page base. Accessed and dirty flags follow �4.8 Table 4-5 ? with one **documented deviation**: read literally, �4.8 would set Accessed on entries visited by a walk that then fails its permission check, while this tree checks access rights first and commits A/D only for a translation that succeeds, following �4.10.2.3's rule that a TLB entry may be created only for a translation that would not fault. That choice is recorded rather than hidden.
7. Large pages and PSE-36: 4-MiB pages via `CR4.PSE` with the whole 22-bit offset taken from the linear address, `PS` ignored when `CR4.PSE` is clear, and the reserved-bit rules that differ between the PSE and PSE-36 profiles. **PSE-36 is implemented but off by default** in the processor profile, so a machine gets the plain 32-bit behavior unless it opts in.
8. A TLB with �4.10.2.2 entry contents, �4.10.2.4 global pages, and the �4.10.4 invalidation rules for `INVLPG`, `MOV to CR3` and `MOV to CR4` ? including that a `MOV to CR3` without `CR4.PGE` leaves global entries alone. **Nothing in the machine calls any of items 5?8.** The interpreter's memory path still treats a linear address as physical, `CR3`/`CR4` are not guest-writable, `#PF` is never delivered, and no firmware in the tree enables paging, so this is tested library behavior rather than observed machine behavior. `docs/mmu-r3-integration-surface.md` is the contract a round-4 wiring slice has to satisfy.
9. PCI configuration Mechanism #1 access widths follow PCI 3.0 �3.2.2.3.2 exactly. Only a full-DWORD write to `0xCF8` latches CONFIG_ADDRESS; a 32-bit access at `0xCF9`?`0xCFB` is not CONFIG_ADDRESS at all, and every non-DWORD access anywhere in `0xCF8`?`0xCFB` is an ordinary I/O transaction that nothing on this machine claims, so it reads all ones and drops writes. Reserved bits 30:24 and 1:0 are never stored. At CONFIG_DATA the "falls inside the DWORD beginning at CONFIG_DATA" rule and the byte-enables-copied-from-the-processor-bus rule give `0xCFD`/`0xCFE`/`0xCFF` their byte-lane meaning, and footnote 15 / �3.2.2.3.4 Master-Abort give all-ones reads and dropped writes for an unimplemented function. **Not:** extended (MMCONFIG) configuration space.
10. CMOS durability across `CmosRtc::reset` now covers the **whole** checksum range. Reset previously preserved thirteen scattered indices while clearing most of `10h`?`2Dh`, which is incoherent: keeping a checksum while erasing part of what it describes makes the stored value silently wrong, and a host that programmed a disk geometry got a checksum that validated before a reset and failed after one with no event to attribute it to. The battery-backed set is now `0Eh`, `0Fh`, the full `10h`?`2Dh` checksum range, `2Eh`/`2Fh`, `30h`/`31h` and `34h`/`35h`, which is what a system battery actually does (IBM PC/AT Technical Reference).
11. The CMOS floppy, fixed-disk and boot-option bytes are populated from the machine's real configuration: `10h` floppy type (Tables C0007/C0008), `12h` hard-disk type (Table C0014), `19h`/`1Ah` extended types (Table C0020), the AMI per-drive parameter blocks at `1Bh`?`23h` and `24h`?`2Ch` (Table C0025), and `2Dh` boot options (Table C0032). RBIL's own `29h` note conflicts with the AMI block layout by claiming that byte holds `80h`; this tree follows Table C0025, because two adjacent nine-byte blocks only fit if `29h` is the sixth byte of the second one, and the conflict is recorded in `docs/sources.md`. **Nothing reads these bytes back** ? SeaBIOS takes drive configuration from fw_cfg ? so they exist for AMI-style POST compatibility and are in practice write-only.
12. A bounded POST event trace: `Machine::probe_post_traced` records the most recent port I/O, PCI configuration cycles, PAM programming, VGA aperture accesses and memory faults in a ring that drops oldest-first and counts what it dropped, so a reader sees the sequence rather than only the last instruction. `probe_post` is now `probe_post_traced(max, None)` and the existing single-line report is byte-identical either way ? the trace is appended as a delimited section, never a change to the format other tooling parses. This is the instrument that produced the round-3 blocker measurement below.
13. A VGA character generator and text-mode display fetch. `VgaText::render_frame` walks the CRTC address counter (`Start Address + row * pitch + col`), takes character codes from map 0 and attributes from map 1, selects a font bank in map 2 through Sequencer Character Map Select and attribute bit 3, and produces a `VgaFrame` of DAC indices that have already passed the ATC Internal Palette, Color Select and PEL Mask path; `frame_rgba8` expands those to 8-bit RGBA. **There is no font at reset** ? a freshly reset device renders no glyphs at all, because this tree has no built-in character ROM ? and the 80�25 / 720�400 geometry is fixed rather than derived from CRTC timing. Two source conflicts are recorded rather than silently resolved: Line Graphics Enable polarity (IBM Figure 2-79 versus FreeVGA, **this tree follows IBM**) and 9/8 Dot Mode polarity (FreeVGA's register page versus its text-mode page, **the register page wins**).
14. One display memory. `VgaText::mem` ? the separate 32 KiB interleaved text buffer ? is gone, along with `mmio_uses_text_buffer`; the four maps are now the only backing store. A guest write at `0xB8000` reaches display memory through the Graphics Controller like any other aperture access, with odd/even addressing plus Map Mask `0x03` placing the character in map 0 and the attribute in map 1 at the same even offset, which is exactly where the character generator fetches them. This retires the model artifact round 2 recorded under "Text-buffer split". The reset fill of 80�25 spaces with attribute `0x07` moved with it and is now visible through `plane_byte`; that fill remains a **model choice, not hardware** ? real display memory holds whatever was there at power-on.
15. The chain-4 256-color display fetch (mode 13h), the second and **only other** programming this model renders. `render_mode` returns `Graphics256Chain4` only when the entire signature is present ? GC Miscellaneous Graphics/Alphanumeric, Graphics Mode `C256`, Sequencer Chain 4, and ATC `ATGE` plus `8BIT` ? and anything less reports `Unsupported` with `render_frame` returning `None` rather than rendering something the hardware would not show. Each display byte is one pixel through the PEL Mask to the DAC. Like text, the 320�200 frame size is fixed, not derived from CRTC timing. **There is still no planar 16-color renderer** (modes 0Dh/0Eh/10h/12h), no VBE, no host display, and no timing-accurate raster.
16. ATAPI detection, done properly and no further. A configured PACKET device writes the `01h`/`01h`/`14h`/`EBh` signature at reset, on SRST and on EXECUTE DEVICE DIAGNOSTIC; IDENTIFY DEVICE and READ SECTOR(S) abort with the signature written per �8.15.5.2 and �8.34.5.2 respectively; IDENTIFY PACKET DEVICE returns a truthful 256-word block. **That block reports peripheral device type `1Fh` "unknown or no device type", not `05h` CD-ROM**, because no command packet set exists ? claiming CD-ROM would be exactly the kind of lie the truthful-CPUID rule forbids. `PACKET` (`A0h`) is **still aborted on both device types**, and DEVICE RESET (`08h`) remains unimplemented despite being mandatory for a real packet device. No media, no MMC/SFF-8020i command set, no ISO boot.

Integration work joined during this merge (coordinator, not a slice):

- **The 32-bit linear-address wrap.** `x86_mmu::linear_addr` computed `seg.base.wrapping_add(offset)` in `u64` with no truncation, so outside 64-bit mode the sum could leave the 4-GiB linear address space the SDM defines (Vol. 3 �3.3.1). The CPU agent found this through the POST probe, which reported page touches at `0x1_000D5000` and `0x1_000FF000`; masked to 32 bits those are `0x000D5000` and `0x000FF000`, the latter the top of the `0xF0000`?`0xFFFFF` BIOS segment SeaBIOS keeps f-segment data in. `linear_addr` now masks to 32 bits and a new `linear_addr64` keeps the untruncated form for when long mode lands, so the 32-bit wrap is not silently inherited. Both above-4-GiB entries disappear from the probe report: the accesses now resolve into mapped low memory instead of being logged as unimplemented MMIO.
- **The known-absent-opcode stand-in.** Four `post_probe` tests hard-coded `0F 40` as "a two-byte opcode this build cannot decode", and slice 3 implemented it ? the **second consecutive round** the stand-in was implemented out from under those tests. They are re-pointed at `0F C7` (Group 9 `CMPXCHG8B`, verified absent from the decode tables rather than assumed) through a single named constant, with a guard test that fails first and by name when that opcode lands. The next occurrence is one edit instead of four mysterious failures.
- **The CONFIG_ADDRESS byte-lane compatibility policy is gone.** It existed only because the interpreter could not issue a 32-bit `OUT`; slice 1 fixed that, so the switch, its accessors and its reset-survival special case were deleted and the two guests that used it now program CONFIG_ADDRESS with the single 32-bit store hardware requires.
- **Re-exports and CLI surface.** `crates/devices/src/lib.rs` re-exports the round-3 public items (`VgaFrame`, `VgaRenderMode`, and the `VGA_TEXT_*`, `VGA_FONT_*`, `VGA_SEQ_CHAR_MAP_*`, `VGA_ATC_MODE_*`, `VGA_CRTC_MODE_*`, `VGA_LINE_GRAPHICS_*`, `VGA_GC_MODE_SHIFT256` and `VGA_MODE13_*` families; the four `ATAPI_SIGNATURE_*`; the CMOS disk, floppy and boot-option register indices). `emulator-cli` gained `--post-trace [N]`, which implies `--post-probe` and appends the trace (with the flag absent the output is byte-identical to before), and `--vga-frame`, which renders through the display fetch and **reports honestly when there is no renderer** instead of inventing a picture.

Integrated 16-slice round 2 (`merge/m2-r2-parallel-16`, base `133191b`):

1. Two-byte near `Jcc` (`0F 80`?`0F 8F`) and `SETcc` (`0F 90`?`0F 9F`) decode and execute. All sixteen condition codes go through one shared evaluator keyed on the low nibble, so the short `70`+cc form, the near form, and `SETcc` cannot disagree. Near `Jcc` takes a rel16 displacement under a 16-bit operand size (clearing `EIP[31:16]` on a taken branch) or rel32 under a 32-bit one, and writes no flags; `SETcc r/m8` writes exactly one byte to a register (including `AH`/`CH`/`DH`/`BH`) or memory in 16- and 32-bit addressing, ignores `ModR/M.reg`, and writes no flags. **Not:** branch-time `CS`-limit `#GP` (still detected on the next fetch), 64-bit `RIP`-relative forms, or the `2E`/`3E` branch-hint prefixes.
2. `MOVZX`/`MOVSX` (`0F B6`/`B7`/`BE`/`BF`) in all eight source/destination width combinations, `PUSH`/`POP FS`/`GS` (`0F A0`/`A1`/`A8`/`A9`), and `LSS`/`LFS`/`LGS` (`0F B2`/`B4`/`B5`) with `m16:16` and `m16:32` pointers. The far-pointer loads reuse the `LDS`/`LES` descriptor helpers, so `SS` gets the stack-segment rules (null selector `#GP(0)`, writable ring-matched data required, `P=0` reported as `#SS`) and `FS`/`GS` the DS/ES data rules, with nothing committed until the pointer is read and the descriptor validates; `LSS` arms the same maskable-interrupt shadow as `MOV SS`/`POP SS` while `LFS`/`LGS` do not, and the register form is `#UD`. **Not:** `MOVSXD` or REX.W destinations, loading `SS` from the LDT, or any privilege change. **Known inconsistency:** the primary-map `PUSH`/`POP` of `ES`/`CS`/`SS`/`DS` still uses a 16-bit stack slot regardless of operand size, which now disagrees with the `0F A0`-family behavior ? a round-3 item.
3. `BT`/`BTS`/`BTR`/`BTC` in both the register bit-offset forms (`0F A3`/`AB`/`B3`/`BB`) and the Group 8 immediate forms (`0F BA /4`?`/7`, with `/0`?`/3` reserved and raising `#UD`), plus `BSF`/`BSR` (`0F BC`/`BD`), `BSWAP` (`0F C8`?`0F CF`), `XADD` (`0F C0`/`C1`) with full ADD flag results, and `CMPXCHG` (`0F B0`/`B1`) including the destination write-back on a mismatch. A register bit base takes `BitOffset MOD OperandSize`; a memory bit base is a bit string per SDM Vol. 2 �3.1.1.9, so the addressed byte is `BitBase + (BitOffset DIV 8)` with signed division rounding toward negative infinity ? a register offset reaches bits above *and below* the nominal operand, and the segment-limit check applies to that displaced byte. `CF` commits only after the read-modify-write cannot fault. **The SDM leaves several results undefined; this tree picks deterministic ones so the interpreter can serve as the JIT's semantic reference, and each is a legal instance of the undefined behavior:** the `BT` family leaves `OF`/`SF`/`ZF`/`AF`/`PF` unchanged, `BSF`/`BSR` leave the destination and `CF`/`OF`/`SF`/`AF`/`PF` unchanged when the source is zero, and 16-bit `BSWAP` performs the full 32-bit reversal. **Not:** REX.W forms, `CMPXCHG8B`/`CMPXCHG16B`, `TZCNT`/`LZCNT`; and **`LOCK` is decoded with no atomicity effect** (single-processor model) and does not raise `#UD` on a register destination.
4. `CPUID` (`0F A2`), `RDMSR`/`WRMSR` (`0F 32`/`0F 30`), `UD2` (`0F 0B`), and `INVD`/`WBINVD` (`0F 08`/`0F 09`). CPUID reports the highest basic leaf as 1, a **deliberately non-Intel vendor string** (`x86WASM Emu `, chosen so software cannot infer capabilities from a familiar vendor plus family/model), family 5 / model 0 / stepping 0, and **exactly one feature bit: EDX bit 5 `MSR`**. Everything else is clear because nothing else exists ? no x87, TSC, DE, VME, PSE, PAE, PGE, PAT, MTRR, APIC, SEP, CX8, CMOV, CLFSH, MMX, SSE, SSE2, or HTT. Leaf `0x8000_0000` is enumerable with no content, and out-of-range leaves return leaf 1, which is why the hypervisor probe at `0x4000_0000` finds nothing (see ADR-0007). **`RDMSR`/`WRMSR` implement the full instruction mechanics but no MSR is implemented ? every address raises `#GP(0)`.** The `MSR` bit claims the instructions exist, not that any register does. `UD2` raises `#UD` in every mode; `INVD`/`WBINVD` are architectural no-ops because no cache is modeled, so only the CPL 0 requirement is observable. **Not:** any MSR (and therefore any `WRMSR` reserved-bit `#GP`), CPUID sub-leaf selection, extended leaves `0x8000_0001`+ including the brand string, the `#UD` a `LOCK` prefix should raise on these opcodes, or VM86 behavior.
5. `PhysMem` gained thirteen independently attributed PAM regions in ascending address order (twelve 16 KiB from `0xC0000`, plus the 64 KiB BIOS area at index 12), each with a read attribute (`Rom`/`ShadowRam`) and a write attribute (`Ignored`/`ShadowRam`), decoded from the 440FX register nibbles. Reset leaves every region reading ROM with writes dropped, so a stray guest write to the BIOS area **no longer returns `RomWrite`** and no longer stops POST at a memory fault; outside the window a ROM write still faults, so the lab-ROM diagnostic is unchanged. The A20 mask applies before the PAM decode, and `is_mapped` reports a shadow-reading region as mapped so the POST probe does not log shadowed firmware as unimplemented MMIO. **Model choices, not 440FX behavior:** with RE clear this model reads the covering ROM window and *falls through to the ordinary RAM / open-bus decode when no ROM window covers the address* ? real silicon would never return DRAM content for RE=0 ? and a 256 KiB auxiliary buffer backs shadow writes on machines with no DRAM at that physical address so small lab machines can still exercise shadowing.
6. BIOS shadowing works end to end: a guest can set the BIOS region to read-ROM/write-DRAM, copy the region onto itself with `REP MOVSB`, then set read-DRAM/write-disabled and execute from the copy. The top-of-4 GiB window is outside PAM and keeps returning the original image, so `prepare_bios_rom`'s dual placement survives the whole sequence. **Not:** cacheability, write combining, or any fetch-versus-data distinction during the transition.
7. An **instruction-count** step clock can drive the PIT and the RTC so timer-polling firmware terminates deterministically. Each retired instruction charges a configurable number of PIT input clocks (default 1); accumulated clocks run RTC periodic quanta at the nominal 1024 Hz POST default and one-second update cycles at 1,193,182 clocks, with remainders carried. **This is a model choice and explicitly not accurate timing:** the default ratio implies a guest CPU retiring ~1.19 million instructions per emulated second, a number derived from no processor's IPC and unrelated to host real time, so firmware that measures the CPU against the PIT computes nonsense. The clock is **off by default** so every existing hand-ticked test is unaffected; `probe_post` arms it for the duration of a run only when the host has not configured one. Only a *retired* instruction charges it. **Not:** any wall-clock or host-monotonic source, TSC/HPET/APIC timer, per-instruction cost model, or RTC quanta derived from the guest's current Status A rate.
8. Option ROMs can be validated and mapped at `0xC0000`. `firmware_interface::prepare_option_rom` checks the `55 AA` signature, a non-zero 512-byte block count that fits the supplied image, the zero-mod-256 checksum over the declared extent, 2 KiB base alignment, and containment in `0xC0000`?`0xDFFFF`, naming each rejection; `Machine::map_option_rom` / `map_vga_option_rom` place the window alongside the BIOS windows, and PAM can shadow the region exactly like the BIOS area. **Not:** the PCI expansion-ROM BAR (`0x30`) or any ROM discovery, the PnP header at offset `0x1A`, runtime-versus-initialization size, BEV/BCV entries, or a window registry surviving a BIOS reload. **Nothing executes the ROM** ? there is no INT 19h or INT 10h dispatch and no call to `base+3`.
9. The i440FX PMC PAM register file (`00:00.0` config `0x59`?`0x5F`) is implemented with the datasheet's reserved-bit treatment: Table 2 bits `[7, 6, 3, 2]` and `PAM0[3:0]` are masked on write and read back zero, per the PCI Local Bus Specification rule for reserved configuration fields. `PciConfig::pam_regions` decodes the register file into the thirteen Table 3 segments in ascending address order and recomputes on every call, since there is no change notification to subscribe to.
10. The CMOS memory-size registers are populated from a supplied RAM size: `15h`/`16h` base memory in KB clamped to the 640 KB DOS area, `17h`/`18h` and `30h`/`31h` extended memory in KB saturating at `3C00h` (15 MB), and `34h`/`35h` memory above 16 MB in 64 KB blocks. The 15 MB / 16 MB split follows RBIL's INT 15h AX=E801h description rather than the AMI reading of `34h`/`35h` as *total* extended memory, because only that reading is consistent with the cap on the KB pairs. RBIL documents `17h`/`18h` as user-configured and `30h`/`31h` as POST-measured; with no setup utility in the loop both report the same figure. **Memory above 4 GB is not reported at all** ? see ADR-0006.
11. fw_cfg publishes `etc/e820` and the name-keyed file directory that carries it. Each map entry is the 20-byte ACPI �15 Table 15.4 address range descriptor (little-endian 64-bit base, 64-bit length, 32-bit �15.2 type); the ACPI 3.0 extended-attributes dword is not emitted, which the specification allows. The directory rejects duplicate names, keeps a selector stable across a content replacement, and does not recycle a freed selector so a stale guest reference reads an unknown item rather than another file's bytes. **The device never synthesizes a map from the RAM-size item** ? the fw_cfg specification defines the transport, not what a machine model must put in the blob, and a guessed map risks a guest double-counting RAM it already learned from CMOS. **Not:** the numeric keys the specification defers to QEMU source for, or `etc/max-cpus`, `etc/system-states`, `etc/table-loader` and `bootorder`; `etc/table-loader` could not be filled truthfully in any case because there are no ACPI tables. ADR-0005 now records the boundary that unblocks the rest.
12. The CMOS equipment byte (`14h`, RBIL Table C0019), diagnostic status byte (`0Eh`, Table C0005), and AT standard checksum (`2Eh`/`2Fh`, byte-wise additive over `10h`?`2Dh`) are implemented, with `equipment_floppy_field` encoding the awkward drive-count field (bits 7-6 count from `00b` = 1 drive, bit 0 reports a drive installed at all). **The device never sets a diagnostic bit and never recomputes a checksum on its own** ? turning a stale checksum into `DIAG_BAD_CHECKSUM` is POST's decision, not a register file's. `CmosRtc::reset` preserves the battery-backed set `0Eh`, `0Fh`, `14h`, `15h`?`18h`, `2Eh`/`2Fh`, `30h`/`31h`, `34h`/`35h`. **That set is incomplete:** the floppy-type byte `10h`, hard-disk type bytes `12h` and `19h`?`2Ch`, and boot-device byte `2Dh` are inside the checksum range but are not preserved and are not populated at all, so a host that programs them must re-store the checksum after a reset.
13. `VgaText` gained a single guest-facing display-memory MMIO entry point: a fixed `0xA0000`?`0xBFFFF` aperture, a runtime `mmio_claims` predicate, and `mmio_read_u8` / `mmio_write_u8` running the whole CPU-side pipeline in one call ? Misc Output RAM Enable gating, Graphics Controller Miscellaneous window decode, Sequencer plane addressing, then the Graphics Controller data path. The claimed sub-range moves with RAM Enable and Memory Map Select, so a bus registers the whole aperture once and lets the device answer per access. `mmio_read_u8` takes `&mut self` because a graphics read loads all four latches. **Model artifact, stated plainly:** this model keeps two backing stores, so an address in `0xB8000`?`0xBFFFF` that the selected window covers is served from the legacy interleaved text buffer byte for byte and does **not** load the latches or apply write modes. Real hardware has one memory; software that programs a graphics window at `0xB8000` hits the text buffer here. Unifying them waits for a character generator and a display fetch, neither of which exists.
14. The two Graphics Controller data-path gaps round 1 recorded are closed. **Write mode 3 now applies Function Select**: the expanded Set/Reset byte passes through the same ALU as modes 0 and 2 before the synthesized mask selects between it and the latch. **The sources genuinely conflict here** ? OSDev's write-mode-3 step list has no ALU stage, while Abrash's *Graphics Programming Black Book* chapter 26 forces the ALU function to "move" in write mode 3, which is only necessary if the stage is live, and IBM's figures show one ALU whose inputs the write mode selects. **This tree follows Abrash**; the reset default Function Select is replace, so the difference is visible only when a non-zero Function Select is left in the Data Rotate register. Second, **Graphics Mode bit 4 now steers read-mode-0 map selection**: when Chain 4 is inactive, host address bit A0 (taken relative to the selected window base, not the decoded per-map offset) replaces bit 0 of Read Map Select. The mode-03h reset default sets this bit, so a read-mode-0 access returns the character map at even addresses and the attribute map at odd ones ? the CGA text emulation the bit exists for. One round-1 test asserted the pre-fix behavior and was updated.
15. A pinned SeaVGABIOS build script, manifest, and licensing record landed: `rel-1.16.3` commit `a6ed6b701f0a57db0569ab98b0661c12a6ec3ff8`, the `vgasrc/` tree, built as plain 256 KB standard VGA with no VBE, SVGA, or PCI ROM header, plus a `check-option-rom.py` header validator. Licensing is recorded in `docs/licensing.md` and `third_party/NOTICE` with the **dual copyright flagged explicitly** ? Kevin O'Connor 2009-2013 *and* the LGPL VGABios developers Team 2001-2008, so the ROM descends from the older LGPL VGABios project as well as from SeaBIOS and a review of SeaBIOS alone does not cover it. **The build script has never been executed and no binary is committed.**
16. `emulator-cli` gained `--vga-text` (dump the 80�25 text buffer, with a `nonblank_rows` signal and rows bracketed so trailing spaces stay visible) and `--option-rom` / `--option-rom-base` (map an image and report its header, marking a malformed one `status=invalid` while still mapping it, because inspecting a broken ROM is a legitimate bring-up step). Diagnostics print after the existing output in a fixed order, so passing neither flag leaves stdout byte-identical and the `--post-probe` format other tooling parses is unchanged. **Not shown:** attributes and color, CP437 glyphs (non-ASCII renders as `.`), anything in a graphics mode (the dump reads the text buffer and there is no renderer), or option-ROM execution.

Integration work joined during this merge (coordinator, not a slice):

- **The PAM seam.** Slices 5/6 and 9 built two halves in different crates that nothing connected, so PAM programming remapped no memory. `MachineBus` now detects a configuration write overlapping host-bridge `00:00.0` `0x59`?`0x5F` and mirrors the register file onto `PhysMem`; `Machine::apply_pam_register` writes through to the configuration bytes as well, so the two views cannot diverge. The two halves' region orderings were **verified against each other in a test** rather than assumed ? they agree, including index 12 as the BIOS area and the register/nibble-to-region mapping. A guest now programs PAM through `0xCF8`/`0xCFC` and shadows and locks the BIOS region end to end.
- **The VGA aperture seam.** `MachineBus` registers the fixed `0xA0000`?`0xBFFFF` aperture once and routes it to `mmio_read_u8`/`mmio_write_u8` (A20 applied first, falling through on `None`/`false`), so a guest reaches the Graphics Controller write modes and Map Mask through ordinary memory accesses. Text-mode behavior at `0xB8000` is unchanged and the HELLO ROM path still passes.
- **The configuration-data seam.** `Machine::new` now calls the CMOS memory-size, equipment-byte and checksum populators and publishes `etc/e820` from the machine's actual RAM size, and attaching floppy media re-derives the equipment byte and checksum. Before this, every byte those slices added read back as zero to a guest and `etc/e820` was absent. The equipment byte leaves the math-coprocessor bit clear because there is no x87 ? the CMOS equivalent of the truthful-CPUID rule.
- **POST probe 32-bit reporting fix.** The probe computed its linear PC and opcode window from `CS.base + (RIP as u16)`, valid only in the real-mode `IP16` window. Once the guest reached 32-bit protected mode it reported an all-zero window at the wrong address, and the CPU agent had to read `bios.bin` by hand to find the real instructions. The window now follows the `CS.D` execution window and the report carries `cs.d` and `eip` explicitly.

Integrated 16-slice round 1 (`merge/m2-r1-parallel-16`, base `d95a4f5`):

1. Protected-mode `CS.D=1` default-32 execution: decode resolves operand and address size from the code-segment default so `0x66`/`0x67` invert under `D=1`, fetch and near `JMP`/`Jcc`/`LOOP`/`CALL`/`RET` run in a 32-bit `EIP` window with 16-bit operand sizes clearing `EIP[31:16]`, and direct far `JMP` (`ptr16:16`, `ptr16:32`, `m16:16`, `m16:32`) enters or leaves a present nonconforming ring-0 `D=1` GDT code segment; `L=1` targets, protected far `CALL`, and branch-time `CS`-limit `#GP` remain unsupported.
2. `SS.B=1` 32-bit stacks: pushes and pops step the full 32-bit `ESP` with 2^32 wrap for `PUSH`/`POP` reg/imm/`r/m`/Sreg, `PUSHF(D)`/`POPF(D)`, `PUSHA(D)`/`POPA(D)` including the `Temp` slot, near `CALL`/`RET`, the `RET`/`RETF imm16` release, and `ENTER`/`LEAVE` with nested displays, checking the stack limit before committing the pointer; `B=0` keeps the 16-bit `SP` window, `0x67` no longer makes `ENTER`/`LEAVE`/`PUSHA`/`POPA` unsupported because it does not change the stack address size, and 64-bit stacks, privilege stack switching, and expand-down stacks remain unsupported.
3. Same-CPL 32-bit IDT delivery: 386 interrupt gates (`0xE`) and trap gates (`0xF`) coexist with the 16-bit types, taking `EIP` from the gate offset high and low words and pushing a 32-bit `EFLAGS`/`CS`/`EIP` frame plus a doubleword error code where applicable, with `IF` cleared only for interrupt gates, gate DPL checked for software `INT`/`INT3`/`INTO` and ignored for NMI/IRQ, frame width from the gate type and pointer width from `SS.B`, and atomic delivery with rollback; 16-bit gates still require `D=0` current and target code, and privilege switching, task gates, LDT gates, VM86 delivery, and #DF/triple-fault synthesis remain unsupported.
4. Same-CPL ring-0 `IRETD`: the effective operand size selects a 12-byte `EIP`/`CS`/`EFLAGS` or 6-byte `IP`/`CS`/`FLAGS` frame on either a `B=0` or `B=1` stack, validating the whole frame and a nonconforming present ring-0 `L=0` return descriptor before atomically restoring `EIP`, the `CS` cache including D/B, AVL, and G, the defined `EFLAGS` bits through `ID`, and the adjusted `SP`/`ESP`; a `VM=1` frame image and `NT=1` nested task returns are reported rather than ignored, and outer-level returns, conforming targets, LDT selectors, real-mode `IRETD`, and `IRETQ` remain unsupported.
5. Sequencer Memory Mode Chain-4 / Odd-Even / Extended Memory plus Map Mask now decode CPU display-window addresses into map targets and per-map offsets (chain-4 selects the map from A1:A0 with the low two address bits cleared from the offset, odd/even sends even addresses to maps 0+2 and odd to maps 1+3 with A0 cleared, planar passes the offset through, and clearing Extended Memory wraps offsets inside 16 KiB per map); QEMU's alternative `addr >> 2` chain-4 offset, any chain-4 or doubleword effect on the display fetch, and the CRTC byte/word compensation are not modeled.
6. The Graphics Controller data path runs host-called accesses over four 64 KiB maps and the four data latches: reads load all latches and return Read Map Select (or the chain-4 address map) or a Color Compare / Color Don't Care result, and writes apply modes 0-3 with Set/Reset, Enable Set/Reset, Data Rotate plus Function Select, Bit Mask, and Map Mask plane write enables; the path is not wired to `MachineBus` CPU MMIO, write mode 3 does not apply Function Select, Graphics Mode bit4 host odd/even read addressing does not steer read-mode-0 map selection, and there is no display fetch from plane memory.
7. Graphics Controller Miscellaneous `0x06` bits 3:2 select the CPU display window (`A0000` 128 KB, `A0000` 64 KB, `B0000` 32 KB, `B8000` 32 KB) for the plane decode, the GC data path, and the text-buffer MMIO path so accesses outside the selected window are no longer claimed, while bit 1 Chain Odd/Even acts as a second source of odd/even host addressing and Misc Output RAM Enable gating is preserved; only the 32 KiB `0xB8000` buffer backs the CPU text path, `MachineBus` routing still uses the static text range, and bit 0 Graphics/Alphanumeric has no character-generator effect.
8. The primary BMIDE PRD walker gained the write direction (`start_bm_write` / `run_prd_write_stub`) filling a device buffer from EOT-terminated PRD regions with BMICOM SSBM plus RWCON set, BMISTA Active while walking, zero count = 64 KiB, the 256-entry missing-EOT cap, 32-bit wrap rejection with no partial copy, and BMISTA Error latching; there is still no ATA DMA command engine, no secondary-channel PRDT engine, no BMIDE interrupt reporting, and no PCI abort modeling.
9. IDE device selection follows ATA/ATAPI-6 �9.16.1 "Device 0 only configurations" on both channels: with the absent Device 1 selected, Device Control writes and non-Command Command Block writes complete as if Device 0 were selected, Command register writes are ignored except EXECUTE DEVICE DIAGNOSTIC (`0x90` ? Error `01h` plus the �9.12 non-PACKET signature, also written on SRST), non-status Command Block reads return Device 0 content with the Device register reading back DEV=1 (Table 18), Status/Alternate Status read `00h` without clearing Device 0 interrupt pending, INTRQ is released while Device 0 is deselected and reasserted on reselect, and Data port cycles for Device 1 are ignored so an in-progress Device 0 DRQ block survives a probe; not an actual Device 1, the PDIAG-/DASP- detection handshake, IDENTIFY word 93, diagnostic codes `8xh`, �9.16.2 "Device 1 only" configurations, the PACKET-device all-`00h` read rule, or DEVICE RESET (`0x08`).
10. IDE 48-bit Address feature set covers READ SECTOR(S) EXT (`0x24`) and WRITE SECTOR(S) EXT (`0x34`) PIO: the Features/Sector Count/LBA Low/Mid/High registers are two-byte deep FIFOs read back through Device Control HOB (bit7), any task-file register write clears HOB, the 16-bit Sector Count treats `0000h` as 65,536 sectors, Device register bits 3:0 are reserved and the LBA bit is required (CHS ? ERR+ABRT), one DRQ block and one nIEN-gated INTRQ occur per sector, and an out-of-range range reports ERR+IDNF before any DRQ so no partial write reaches media; IDENTIFY word 83 bit10 and word 86 bit10 are set with words (103:100) carrying the 48-bit capacity and words (61:60) capped at 268,435,455, and READ NATIVE MAX ADDRESS clamps to 268,435,454; not READ/WRITE DMA EXT, DMA QUEUED EXT, READ/WRITE MULTIPLE EXT, READ VERIFY SECTOR(S) EXT, READ NATIVE MAX ADDRESS EXT, SET MAX ADDRESS EXT, HPA interaction between the 28- and 48-bit SET MAX forms, error-output LBA reporting, HOB clearing on a Data port write, or Device Configuration Overlay word 7 bit 8.
11. FDC READ ID (`0x0A`|MFM) performs a per-drive track ID-field scan instead of the fixed `R=1` stub: with media present and the head over a formatted cylinder the result is ST0 IC=00|H|US, ST1=ST2=0, C=`pcn[unit]`, H from the HD bit, R = the next ID field advancing 1..=18 and wrapping, N=2, and the position then advances; when no ID Address Mark can be found (no media, or the head parked past cylinder 79) the command terminates with ST0 IC=01|H|US, ST1 MA|ND, C/H/R/N=0 and leaves the position unchanged; the position is per unit, survives Seek/Recalibrate, and restarts on hardware and DOR/DSR software reset; not real INDX#/rotational-latency or data-rate timing, ID fields written by FORMAT TRACK, ID-field CRC errors (ST1 DE, ST2 CRC), ST2 WC/BC reporting, or media formats other than 1.44MB.
12. FDC Configure EIS (byte1 bit6) is enforced at runtime rather than stored only: a command carrying a C parameter ? READ DATA, READ TRACK, READ DELETED DATA, VERIFY, SCAN, WRITE DATA, WRITE DELETED DATA ? performs an implied Seek setting `pcn[unit] = C` before execution, observable through DUMPREG PCN0-3, Sense Drive Status ST3 T0 and READ ID's C byte; the seek is mechanical so it applies even when the transfer terminates abnormally, it queues no Seek End ST0 latch and raises no extra interrupt, FORMAT TRACK has no cylinder parameter and never implies a seek, and an unlocked DOR/DSR software reset restores the EIS default; not SRT/HLT step and head-settle timing, DSKCHG clearing from an implied seek, head-position gating of transfers or ST2 WC/BC reporting, or Configure POLL / EFIFO / FIFOTHR runtime effects.
13. COM1/COM2 16550 THRE lines are routed through `MachineBus::poll_external_irq` to 8259A master IR4 (COM1 `0x3F8`, vector `0x0C`) and IR3 (COM2 `0x2F8`, vector `0x0B`) with `Machine::sync_com1_irq4`/`sync_com2_irq3` host helpers and end-to-end IVT delivery; received-data-available, line-status, modem-status, and FIFO interrupts remain absent because the UART subset has no receive path.
14. The QEMU fw_cfg DMA interface is implemented on the big-endian 64-bit address register at `0x514`/`0x518` with signature reads, `FWCfgDmaAccess` select/read/skip, zero-fill past item end, control writeback and register clear, and `MachineBus` `PhysMem` callbacks honoring the A20 gate; ID bit1 is set only while that register is live, and the write direction (control bit 4) is rejected with the spec error bit because item writeability is not modeled.
15. `Machine::probe_post` and `emulator-cli --post-probe` produce a structured POST first-contact report ? retired steps, classified stop reason, first-failure kind with `CS:IP`/RIP/linear PC and an eight-byte wrapping opcode window, bounded unclaimed-port and unmapped-MMIO logs, POST codes, and COM1/`0x402` output ? gated to skip when `firmware/seabios/bios.bin` is absent; it is a diagnostic only and does not advance any time source or resume past a failure.
16. Port `0x80` is claimed as the IBM PC/AT manufacturing diagnostic port with a host-readable checkpoint latch (last code, 256-entry ordered history with overflow flag, and an unbounded write count for I/O-delay traffic), cleared by `Machine::reset` and reported by the POST probe; reads remain ISA open bus and no POST-card display, extended POST ports, or checkpoint-vs-delay classification is modeled.

Progress against implement list:

- [x] Partial real-mode foundation (software INT/IRET/INT3, PUSHF/POPF, far CALL/RETF/JMP, segment MOV/PUSH/POP, Jcc, XCHG, LOOP/JCXZ, Group 1/2/3 full F6/F7 /0-/7 TEST/NOT/NEG/MUL/IMUL/DIV/IDIV with #DE, Group 4/5 INC/DEC r/m FE/FF /0-/1, Group 5 CALL/JMP/PUSH r/m FF /2,/4,/6, Group 5 far CALL/JMP m16:16 FF /3,/5, string byte/word/dword ops MOVSB/W/D STOSB/W/D LODSB/W/D CMPSB/W/D SCASB/W/D (A4?A7/AA?AF) with REP/REPE/REPNE (CX=0 nop, CX loop, ZF early-exit, DF; 0x66 ? dword), INS/OUTS string port I/O INSB/W/D OUTSB/W/D (6C?6F) with REP (DX port; ES:DI dest / DS:SI src with seg override on OUTS; DF steps; 0x66 ? dword; `MachineBus` size-aware port_in/out), BCD adjust DAA/DAS/AAA/AAS/AAM/AAD (27/2F/37/3F/D4/D5; AAM base-0 #DE via IVT), LEA, CBW/CWD, flag ops, PUSH imm, SAHF/LAHF, INC/DEC r16, AND/OR ModRM 08-0B/20-23, AND/OR AL/AX imm 0C/0D/24/25, ADC/SBB ModRM 10-13/18-1B, ADC/SBB AL/AX imm 14/15/1C/1D, XOR ModRM 30-33, ADD/SUB ModRM byte 00/02/28/2A, SUB/XOR/CMP AL/AX imm 2C/2D/34/35/3C/3D, CMP ModRM byte 38/3A, ADD AX imm 05, related ALU ModRM forms, legacy high-byte ModR/M AL..BH via shared `gpr_u8`/`set_gpr_u8`, MOV C6/C7 r/m imm, MOV A0-A3 moffs16/moffs32 (0x67), TEST A8/A9 AL/AX/EAX imm (0x66?imm32), RM-E stack/frame: PUSHA/POPA/PUSHAD/POPAD 60/61, ENTER/ENTERD nesting 0?31 / LEAVE/LEAVE32 C8/C9 (0x66 dword display), PUSHF/POPF/PUSHFD/POPFD 9C/9D, RET/RETF iw C2/CA (0x66 far ? EIP32+CS16), POP r/m16 8F /0, real-mode exception delivery via IVT for #DE/#UD/#GP/#SS (architectural decode-miss + ModRM/stack/string/moffs MemoryFault classify + cached segment-limit #GP/#SS), interruptible REP (`pending_irq` stub), RM-F LES/LDS/XLAT C4/C5/D7 (0x66?r32,m16:32), RM-G IMUL imm 69/6B, IMUL 0F AF two-operand r16/r32�??r/m (0x66?32), INTO CE (#OF trap) / BOUND 62 (#BR fault; 0x66?r32,m32&32), **0x66 opsize-32 tranche** for MOV r/m�??r / imm (89/8B/B8-BF/C7), PUSH/POP r32/imm (50-5F/68/6A), ALU ModRM+accum ADD/SUB/XOR/CMP/AND/OR/ADC/SBB + Group1 81/83, near JMP/CALL/RET, INC/DEC r32 40?4F, CWDE/CDQ 98/99, XCHG EAX,r32 91?97, **tranche-2** ENTERD/LEAVE32 + PUSHAD/POPAD + PUSHFD/POPFD, **tranche-3** Group5 INC/DEC/CALL/JMP/PUSH r/m32 + far CALL/JMP m16:32 (FF) + far CALL/JMP/RETF ptr16:32 (9A/EA/CA/CB), **tranche-4** MOV moffs EAX A1/A3 + POP r/m32 8F + MOV r32�?�Sreg zero-extend 8C + Group2 D1/C1/D3 + Group3 F7 opsize-32 + IMUL 69/6B imm opsize-32, **unreal limits**: sticky `load_real_mode_selector` for DS/ES/SS/FS/GS + `checked_linear_addr` on ModRM/moffs/stack/XLAT/string/CS-fetch; **asize32**: JECXZ/LOOP*/XLAT ECX/EBX)
- [x] Complete real-mode foundation (**audit gaps landed**: RM-D/E/F/G, REP/REPE/REPNE + interruptible REP (`pending_irq` / `Bus::poll_external_irq`), word/dword strings, 0x66 opsize-32 tranche + tranche-2 ENTERD/PUSHAD/PUSHFD + tranche-3 INC/DEC/XCHG/CWDE/CDQ/TEST EAX/LES/LDS/BOUND/far ptr16:32/Group5 r/m32 + tranche-4 moffs EAX/POP r/m32/MOV Sreg?r32/Group2 D1/C1/D3/Group3 F7/IMUL 69/6B, #UD IVT, INS/OUTS, BCD, INTO/BOUND, ENTER nesting>0, 0F AF IMUL, decode-miss architectural #UD + stack/ModRM/string/moffs/code-fetch #GP/#SS classify, address-size 0x67 ModRM/SIB EA + string ESI/EDI/ECX + moffs32 + JECXZ/LOOP* ECX + XLAT EBX, unreal segment limits / sticky data-seg cache + string-op + CS-fetch limit checks, per-instruction + REP external IRQ poll) ? **not SeaBIOS-ready**; **M2 CPU real-mode foundation complete**. Honest remaining (not blocking foundation checkbox): MOVSQ/STOSQ/�?� qword strings N/A in REX-less real mode (long-mode / REX.W later); asize64 (RCX/JRCXZ/RBX); ENTER/PUSHA/POPA/LEAVE asize32 under 0x67 (`Unsupported` ? needs ESP stack helpers); full 8259 PIC / hardware IRQ routing; LGDT/SGDT GDTR load/store landed; PE=1 now has the bounded GDT/IDT/transfer path described below; that does not change the real-mode-foundation checkbox. Full protected mode and default-32 execution remain open.
- [ ] Protected mode ? **partial, 16-bit *and* 32-bit, now with privilege-changing gates / TSS / `#DF` (moffs address-size fixed; CPUID family 6 + PSE/PGE/CMOV):** Round 5 adds `LTR`/`STR` (32-bit TSS), privilege-changing IDT delivery with TSS stack switch, outer `IRET`/`IRETD`, and `#DF`/triple-fault on nested delivery failure. **Still not:** task gates, call gates, VM86, protected far CALL, LDT. Prior same-CPL path remains: `LGDT`/`SGDT`/`LIDT`/`SIDT`, `SMSW`/`LMSW`, `MOV CR0`, `CLTS`, and the real-mode TLB-less `INVLPG` memory-form NOP remain supported as documented. With `CR0.PE=1`, GDT-backed `MOV`/`POP` segment loads plus `LDS`/`LES` validate null/type/present/RPL/CPL/DPL/limit rules and preserve access + AVL/L/D-B/G cache state; successful `MOV SS`/`POP SS` arm the maskable-interrupt shadow. Decode now resolves operand and address size from the `CS.D` default so `0x66`/`0x67` invert under `D=1`, fetch and near `JMP`/`Jcc`/`LOOP`/`CALL`/`RET` run in a 32-bit `EIP` window (a 16-bit operand size clears `EIP[31:16]`), and direct far `JMP` (`ptr16:16`, `ptr16:32`, `m16:16`, `m16:32`) enters or leaves a present nonconforming ring-0 `D=0` or `D=1` GDT code segment. `SS.B=1` gives full 32-bit `ESP` stacks with 2^32 wrap and pre-commit limit checks. Architectural faults, software `INT`/`INT3`/taken `INTO`, NMI, and maskable IRQs enter same-CPL 16-bit (`0x6`/`0x7`) or 386 (`0xE`/`0xF`) IDT interrupt/trap gates, with frame width from the gate type and pointer width from `SS.B`; selector error codes are pushed where applicable, software gate DPL is checked, hardware gate DPL is ignored, and interrupt gates clear IF. Same-CPL ring-0 `IRET16` and `IRETD` restore validated frames including the CS D/B, AVL, and G cache attributes. Descriptor/frame/stack transfers are atomic; nested-delivery failure is reported instead of synthesizing #DF/triple fault. Round 2 adds `POP FS`/`GS` and `LSS`/`LFS`/`LGS` on the same descriptor paths (`SS` gets the stack-segment rules including `P=0` reported as `#SS`, `FS`/`GS` the DS/ES data rules, `LSS` arming the maskable-interrupt shadow), and the CPL 0 requirement of `RDMSR`/`WRMSR`/`INVD`/`WBINVD`. **Not supported:** privilege switching or outer-level return, protected far CALL/call gates/tasks, LDT/TSS, paging, CR2/CR3/CR4, PM TLB invalidation, conforming or `L=1` targets, expand-down stacks, branch-time `CS`-limit `#GP`, VM86, `IRETQ`, or the full PM exception taxonomy. **SeaBIOS now reaches this path**: the round-2 POST probe stops in a `CS.D=1` segment at 17,218 retired instructions, so 32-bit protected-mode entry and CPU identification are exercised by real firmware rather than only by tests. Round 3 adds accumulator port I/O (`E5`/`E7`/`ED`/`EF`), operand-size-correct segment `PUSH`/`POP` on the primary map, `CMOVcc` and `SHLD`/`SHRD`, and fixes the linear-address computation to wrap at 4 GiB outside 64-bit mode per �3.3.1; after those, **no CPU instruction stops POST at all** ? firmware runs until the step budget ends. Paging exists as a standalone engine but is still not reachable from this path: `CR0.PG`, `CR2`, `CR3` and `CR4` remain unimplemented in the interpreter.
- [ ] Segmentation (beyond real-mode base<<4) ? **partial:** PE=1 `MOV`/`POP` Sreg and `LDS`/`LES` load GDT-backed data/stack caches with null/type/present/RPL/CPL/DPL checks and full access + AVL/L/D-B/G attributes; `POP FS`/`GS` (`0F A1`/`A9`) and `LSS`/`LFS`/`LGS` (`0F B2`/`B4`/`B5`, `m16:16` and `m16:32`) reuse those helpers, with nothing committed until the whole pointer is read and the descriptor validates and the register form raising `#UD`. Direct far JMP, IDT entry, and IRET/IRETD load `D=0` or `D=1` CS caches, and `SS.B` selects the 16- or 32-bit stack window. PE=0 sticky-unreal `selector<<4` is unchanged. Round 3 fixes the linear-address computation itself: `base + offset` is now formed **modulo 2^32** outside 64-bit mode, so a reference from a segment based near the top of the space wraps into low memory instead of escaping the 4-GiB linear address space (SDM Vol. 3 �3.3.1). `x86_mmu::linear_addr64` keeps the untruncated form for when long mode lands. This was a measured defect, not a theoretical one ? it is what made SeaBIOS f-segment accesses appear as above-4-GiB page touches in the POST probe. Round 3 also makes the primary-map segment `PUSH`/`POP` forms size their stack slot from the operand-size attribute, matching the `0F A0` family. **Not:** LDT, conforming/outer-level control transfers, privilege stack switching, expand-down stacks, `L=1` long-mode segments, the canonical-address check, or the full segmentation taxonomy.
- [ ] GDT ? **partial:** GDTR is programmable via `LGDT`/`SGDT`; PE=1 `MOV`/`POP` Sreg and `LDS`/`LES` load data or readable-code descriptors with present/type/privilege checks, while SS requires same-CPL writable data and its `B` bit now selects the stack address size. Direct far JMP, IDT entry, and IRET/IRETD load nonconforming `D=0` or `D=1` code descriptors with their full cached attributes. **Not:** LDT, call gates/tasks, conforming/outer-level transfers, privilege stack switching, or `L=1` descriptors.
- [ ] IDT ? **partial:** IDTR is programmable via `LIDT`/`SIDT`; PE=0 uses the IVT, while PE=1 supports atomic same-CPL delivery through both 286 16-bit gates (`0x6`/`0x7`) and 386 32-bit gates (`0xE`/`0xF`) for architectural faults, software interrupts, NMI, and IRQ, including gate DPL rules and applicable word or doubleword error-code frames; frame width comes from the gate type and stack-pointer width from `SS.B`, and a 16-bit gate still requires `D=0` current and target code rather than truncating a 32-bit return `EIP`. Round 2 adds no gate mechanics, but two new sources now enter through these paths: `UD2`'s architecturally guaranteed `#UD`, and the `#GP(0)` every `RDMSR`/`WRMSR` address raises because no MSR is implemented. **Not:** task gates, gates in the LDT, VM86 delivery, or the IST mechanism. Round 5 adds privilege-stack switching, outer-level delivery, and nested `#DF`/triple-fault synthesis.
- [ ] LDT
- [ ] TSS ? **partial:** round 5 `LTR`/`STR` 32-bit available TSS + privilege stack switch; no task gates / hardware task switch
- [x] Exceptions ? real-mode IVT delivery remains complete for #DE/#UD/#BR/#GP/#SS and #OF via INTO. PE=1 has bounded atomic delivery through same-CPL *and* privilege-changing 16-bit and 386 32-bit IDT gates, including selector error codes, and round-5 `#DF`/triple-fault synthesis on nested delivery failure. Round 2?4 sources (`UD2`, MSR `#GP`, paging `#PF`, etc.) remain. The full protected-mode taxonomy beyond that path remains open.
- [ ] Interrupts (hardware / PIC-driven) ? **partial (HLT+IF=1 is idle):** round 4 treats a wake-able HLT as an idle quantum that advances the step clock until IRQ0 rather than ending a POST probe (SDM Vol. 2 HLT / Vol. 3A �6.8.1). `pending_irq`/`request_interrupt` and `pending_nmi`/`request_nmi` are polled per instruction and during REP; NMI precedes IRQ and is IF-independent. IRQ requires IF and is delayed through the instruction after a successful `MOV SS`/`POP SS`; NMI is not delayed. PE=1 delivers NMI/IRQ through same-CPL 16-bit or 386 32-bit IDT gates, can wake HLT, saves the interrupted `IP`/`EIP`, and ignores gate DPL for hardware sources. `MachineBus::poll_external_irq` routes PIT0?IRQ0, 8042 keyboard?IRQ1, COM2 THRE?IRQ3, COM1 THRE?IRQ4, FDC?IRQ6, CMOS?IRQ8, 8042 aux?IRQ12, and IDE?IRQ14/15, with `Machine::sync_com1_irq4`/`sync_com2_irq3` host helpers and end-to-end IVT delivery for the COM lines. THRE is the only 16550 interrupt source, since the UART subset has no receive path. Round 2 adds an optional instruction-count step clock that lets retired instructions drive PIT channel 0 and the RTC, so a timer-polling firmware loop terminates instead of spinning out the step budget; it is **off by default** and the step-to-tick ratio is a model choice, not timing. **Not:** privilege switching or stack switching, task gates, APIC, or full interrupt-window timing.
- [ ] 32-bit paging ? **partial, wired; ring-3 `#PF` now possible with TSS stack switch:** round-4 `PagedBus` / `#PF` / TLB plus round-5 privilege delivery. CPUID advertises `PSE`/`PGE`/`CMOV` and family 6. The A/D-versus-fault deviation survived contact with real execution. **Still not:** PAE/long mode, task-switch `CR3`, SMEP/SMAP. See `docs/cpu-r4-paging-integration.md`.
- [ ] 8259 PIC ? **partial:** `devices::DualPic` ICW1?ICW4 + OCW1 IMR + OCW2 EOI + OCW3 IRR/ISR read + OCW3 poll command (`P=1` one-shot acknowledging command-port read ? `0x80|level`, IMR/fully-nested aware, software-sequenced cascade poll; no-pending byte `0x00` documented model choice) + Automatic EOI (ICW4.AEOI clears ISR at end of INTA / OCW3-poll ack via shared `ack_ir`) + Special Mask Mode (OCW3 `ESMM`/`SMM`; masked in-service IR does not nested-block lower unmasked IRQs; non-specific EOI skips masked IS bits; ICW1 clears SMM) + Special Fully Nested Mode (ICW4.SFNM on master; slave-connected IS bit does not lock cascade IR out of master priority logic so higher-priority IR on same slave can deliver without master EOI; ICW1 clears SFNM) + Automatic Rotation (OCW2 Rotate on Non-Specific EOI `R=1,SL=0,EOI=1` + Rotate in Automatic EOI Mode set/clear `R=1/0,SL=0,EOI=0`; priority resolver / nested blocking / non-specific EOI follow rotated bottom; ICW1 resets `lowest_priority=7` and rotate-in-AEOI off) + Specific Rotation (OCW2 Set Priority Command `R=1,SL=1,EOI=0` + L2?L0 ? lowest without EOI; Rotate on Specific EOI `R=1,SL=1,EOI=1` + L2?L0 clears named ISR and assigns that IR lowest) + edge IRQ assert (ICW1.LTIM=0 and ELCR bit clear) + level-triggered runtime (ICW1.LTIM=1 or PIIX ELCR per-IR bit via DualPic::set_elcr_level_mask: IRR follows IR level; deassert clears IRR; ack while held re-pends IRR for post-EOI redelivery; ELCR survives ICW1; reserved IRQ0/1/2/8/13 hardwired edge) + spurious / DEFAULT IR7 when IR pin is low at first INTA (vector IR7, ISR bit7 not set; real IR7 sets ISR bit7; cascade master IR never remapped ? empty/spurious slave still sets master cascade IS) + `DualPic::irr_isr_snapshot` / `ocw3_read_irr_isr` host IRR/ISR views + cascade + `MachineBus` ports / `poll_external_irq` (master IR0 PIT0, IR1 8042 keyboard, IR3 COM2 THRE `0x2F8`?vector `0x0B`, IR4 COM1 THRE `0x3F8`?vector `0x0C`, IR6 FDC, IR14 IDE primary; slave IR8 CMOS, IR12 8042 aux, IR15 IDE secondary); **not** host-visible INT-raise vs INTA race beyond pin-low-at-ack / empty-slave-cascade model / PCI device INTx beyond the PIRQRC software stub
- [ ] 8254 PIT ? **partial:** `devices::Pit8254` ch0 programming + `ce`/OUT tick (modes 0/1/2/3/4/5 incl. mode 1 retriggerable one-shot + mode 4/5 one-CLK strobe + GATE-pin summary semantics) + one-CLK CR?CE load delay after count write (modes 0/2/3/4) and after GATE rising-edge arm (modes 1/2/3/5: Mode 1 OUT low on load CLK; NULL COUNT clears on load CLK) + mode 3 square-wave approximate 50% duty (even N: N/2 high + N/2 low; odd N: (N+1)/2 high + (N?1)/2 low; binary and BCD periods via `reload_ce`) + BCD counting during tick (control-word BCD bit: four-decade countdown, written `0` ? 10_000; latch/read-back report BCD decades) + Read-Back command on `0x43` (`SC=11`, COUNT/STATUS + CNTn select; status byte OUT/NULL_COUNT/RW/M/BCD; status-then-count read order) + `Machine::tick_pit` ? IRQ0?PIC; ch2 GATE/OUT + port `0x61` speaker bits (no host audio); ch1 mode-2-ish DRAM-refresh countdown via `tick_ch1` + `refresh_out`/`ch1_out` (no IRQ) with each rising edge toggling read-only port `0x61` bit4 (writes ignored; reset low); DRAM-refresh bus-cycle side effects remain out; round 2 adds the optional instruction-count step clock that can advance `tick_pit` from retired instructions (off by default; ~1.19 MIPS implied by the default ratio is a **model choice, not timing**); **not** host-real-time / host speaker audio / mode 3 hardware decrement-by-two CE micro-timing / ch0-ch1 GATE input (tied high, so modes 1/5 trigger only on ch2) / DRAM refresh bus cycles
- [ ] RTC ? **partial (incl. ADR-0006 above-4 GB CMOS):** round 4 adds `5Bh`?`5Dh` as a 24-bit count of 64 KiB blocks above 4 GB (de-facto Bochs/QEMU/SeaBIOS convention). Nothing configures >4 GB today (test-only). Prior MC146818 / memory-size / disk-byte content remains. `devices::CmosRtc` index/data + PIE/AIE/UIE + status C read-to-clear + Status A UIP approximate high window (`tick_second` sets UIP + advances calendar + latches UF; UIP stays readable high for `UIP_WINDOW_PERIODS` (=2) subsequent `tick` periods then clears ? order-of-magnitude model, not �s-accurate) + `tick_second` full calendar cascade (sec?min?hour?date?month?year?century `0x32`, day-of-week 1?7, Gregorian leap years, SET inhibits) with Status B `DM` (bit 2) BCD (`DM=0`) or binary (`DM=1`) field encoding + Status B `24/12` (bit 1) 24-hour `0?23` (set; reset default) or 12-hour `1?12` + AM/PM bit7 (clear; BCD `$01`?`$12`/`$81`?`$92`, binary `$01`?`$0C`/`$81`?`$8C`) + alarm registers `0x01`/`0x03`/`0x05` match on `tick`/`tick_second` (byte equality for BCD/binary/12-hour AM/PM; don't-care `C0h`?`FFh`; AF on match, AIE?IRQF) + `Machine::tick_cmos`/`tick_cmos_second` ? IRQ8?PIC; port `0x70` bit7 NMI mask R/W + `nmi_masked` / `Machine::nmi_delivery_enabled` + `Machine::inject_nmi` ? `#NMI` IVT vector 2 (IF-independent; masked drop); auto hour (+ hour-alarm) conversion on Status B `24/12` toggle (BCD/binary per `DM`; alarm don't-care `C0h`?`FFh` unchanged; silicon leaves reinit to software ? model converts for coherent wall time); CMOS shutdown status `0x0F` R/W store/readback + `shutdown_status()` (IBM PC/AT / RBIL reset codes; SeaBIOS soft-reset `0x0A` etc.; preserved across `CmosRtc::reset`); round 2 adds the configuration bytes POST reads and the machine wiring that fills them: `Machine::new` calls `set_memory_size` with the configured RAM, composes the equipment byte (`14h`) from what the machine actually has, and stores the AT standard checksum (`2Eh`/`2Fh`) over `10h`?`2Dh`, so a guest reading `15h`?`18h`/`30h`/`31h`/`34h`/`35h` through `0x70`/`0x71` gets a coherent memory description instead of zeros; attaching floppy media re-derives the equipment byte and checksum; the math-coprocessor bit stays clear because there is no x87; the diagnostic status byte (`0Eh`) is plain storage the device never sets on its own; the optional instruction-count step clock can advance `tick`/`tick_second` from retired instructions; round 3 closes the durability and disk-byte gaps round 2 recorded: `CmosRtc::reset` now preserves the **whole** `10h`?`2Dh` checksum range (plus `0Eh`, `0Fh`, `2Eh`/`2Fh`, `30h`/`31h`, `34h`/`35h`), so a stored checksum can no longer describe bytes a reset erased, and `Machine::sync_firmware_configuration` fills the floppy type `10h` (Tables C0007/C0008), hard-disk type `12h` (C0014), extended types `19h`/`1Ah` (C0020), the AMI per-drive parameter blocks `1Bh`?`23h` and `24h`?`2Ch` (C0025, following that table where RBIL's `29h` note conflicts with it), and boot options `2Dh` (C0032) from what the machine actually has; **nothing reads those bytes back** ? SeaBIOS takes drive configuration from fw_cfg, so they are in practice write-only AMI-style POST compatibility; the round-3 POST trace does show real firmware driving `0x70`/`0x71`; **not** host wall-clock/NTP sync / SMRAM/SMI/NMI nesting / exact crystal UIP width / memory above 4 GB (ADR-0006) / Machine POST action on shutdown code

- [ ] DMA ? **partial:** `devices::Dma8237` dual 8237A addr/count/mode/mask + AT page regs on `MachineBus` + Base Address/Word Count loaded with Current on program + software Request Register set/reset for each master/slave-local channel (independent of masks) + Status request bits 7:4 persistent until request reset/Master Clear while TC bits 3:0 alone clear on read + `latch_tc` + `transfer_block` software helper (8-bit ch0?3 length `count+1` phys `(page<<16)|addr`; ISA ch4 AT cascade channel mode/page/addr/count programming (Cascade bits 7:6 = 11 on master or slave ch0 store/readback; transfer_block always rejects ch4); 16-bit ch5?7 AT cascade slave data channels word address/count length `2*(count+1)` phys `(page<<16)|(addr<<1)`; Demand|Single|Block (mode bits 7:6)+Increment|Decrement (mode bit 5)+Verify|Read|Write + optional Autoinitialize mode bit 4; Verify advances addr/count + TC/auto-mask without `mem_read`/`mem_write`; with Autoinit, after TC Current reloads from Base and channel stays unmasked/ready; without Autoinit Current ends at post-step addr/`0xFFFF` and channel is hardware-masked; address wraps within page (byte or word); Read/Write use `mem_read`/`mem_write` callbacks + TC latch) + `Machine::dma_transfer` / `MachineBus::dma_transfer` wires those callbacks to `PhysMem` (A20 honored) + MachineBus FDC READ DATA auto-wire ? ISA ch2 Write (`Machine::fdc_dma_read_sector` / `try_fdc_dma_ch2_write` after FDC `port_write` when `last_sector` pending + DOR DMA/IRQ; machine-pc e2e single + multi-sector R..=EOT into PhysMem) + MachineBus FDC WRITE DATA auto-wire ? ISA ch2 Read (`Machine::fdc_dma_write_sector` / `try_fdc_dma_ch2_read` after FDC `port_write` when `dma_write_pending` + DOR DMA/IRQ); **not** DREQ/DACK cycle timing / Cascade or ISA-ch4 data transfer via transfer_block (programming accepted) / IDE automatic BM-DMA / SeaBIOS floppy e2e
- [ ] PS/2 controller ? **partial:** `devices::I8042` + `Machine::kbd` on `MachineBus` ports `0x60`/`0x64` (self-test/config/enable + OBF?INT1?IRQ1 + make-code inject + Set2?Set1 (config bit6) + `0xD0`/`0xD1` output-port A20 ? `PhysMem` + bit0-low system-reset ? `Machine::service_8042_pulse_reset`); first-port **keyboard stub** on data-port writes when no controller pending: `0xF4`/`0xF5` Enable/Disable Scanning ? ACK `0xFA` (store scanning; inject drops when disabled), `0xF2` Get Keyboard ID ? ACK + MF2 `0xAB` `0x83`, `0xFF` Reset ? ACK + BAT `0xAA` (restores scanning/LEDs/typematic/scancode-set defaults), `0xED` Set LEDs (+ mask) / `0xF3` Set Typematic Rate/Delay (+ byte) / `0xF0` Get/Set Scancode Set (+ get|1|2|3; default 2) ? ACK each byte and store state, `0xEE` Echo ? `0xEE`, `0xFE` Resend ? requeue last keyboard OBF byte, `0xF6` Set Default ? ACK + restore defaults (no BAT), `0xF7`/`0xF8`/`0xF9`/`0xFA` Set All Keys Typematic/Make-Break/Make/Typematic-Make-Break ? ACK, `0xFB`/`0xFC`/`0xFD` Set Key Type ? ACK + one ignored scancode (tracks presentation; Reset clears last to 0 then ACK/BAT update it; default last=0); controller pulse-reset `0xFE` on `0x64` increments `pulse_reset_commands` + latches system-reset ? `Machine::service_8042_pulse_reset` / `Machine::reset` (CPU CS:IP reset vector + devices; after each `step`; same latch from `0xD1` output-port bit0 low); responses on keyboard OBF (queued while clock disabled; `0xAE` flushes); second (auxiliary) port: `0xA7`/`0xA8` config bit5 aux clock disable/enable (disabled: queue host?aux `0xD4`; `0xA8`/config clear bit5 flushes to mouse stub; enabled: immediate delivery), `0xA9` test-aux ? `0x00` on normal OBF, `0xAB` test-kbd ? `0x00` on normal OBF, `0xAC` diagnostic dump ? 16-byte zero stub on normal OBF (one byte per `0x60` read; internal RAM not modeled), `0xD4` host?aux + status bit5 AUX OBF + `inject_aux_byte`/`Machine::kbd_inject_aux_byte` ? AUX OBF?config bit1 ? IRQ12 (slave IR4) via `poll_external_irq` (keyboard data drives IRQ1 only, aux data IRQ12 only; `0x60` read clears both); PS/2 **mouse stub** behind aux: `0xFF` Reset ? ACK `0xFA` + BAT `0xAA` + ID `0x00` (restores defaults), `0xF2` Get Device ID ? ACK + `0x00`, `0xF4`/`0xF5` Enable/Disable Data Reporting ? ACK and store reporting flag, `0xF6` Set Defaults ? ACK + restore defaults (sample/res/scaling/reporting off/stream/wrap off; **no BAT**), `0xF3` Set Sample Rate (+ value) / `0xE8` Set Resolution (+ value) store+ACK, `0xE9` Status Request ? ACK + 3-byte status (OSDev flags/res/rate), `0xE6`/`0xE7` Set Scaling 1:1/2:1 ? ACK, `inject_mouse_packet(dx,dy,buttons)` ? standard 3-byte stream packet on AUX OBF when reporting enabled (dropped when `0xF5`/disabled); keyboard `inject_scancode` applies Scan Set 2?Set 1 when config bit6 set (Brouwer/Konzak table + `F0` break / `E0` prefix; passthrough when clear); mouse Resend `0xFE` requeues last AUX OBF byte; mouse Set Stream Mode `0xEA` ? ACK (clears remote and wrap); mouse Set Remote Mode `0xF0` ? ACK + store remote flag (Status Request bit6; clears wrap); mouse Read Data `0xEB` ? ACK + 3-byte movement packet (last inject or zeros; stream or remote); mouse Set Wrap Mode `0xEE` ? ACK + store wrap flag (while wrap: host?aux bytes via `0xD4` echoed on AUX OBF with no ACK except Reset Wrap/`0xFF`; Reset/`0xEA`/`0xF0`/`0xF6` clear wrap when executed); mouse Reset Wrap Mode `0xEC` ? ACK (clears wrap; stream/remote unchanged); **not** wheel/5-button / full remote protocol beyond `0xEB`, inject streams varying by stored scancode set / host-visible LED hardware / get-current set translation via config bit6
- [ ] PCI configuration space ? **partial (BAR sizing + honest Type 0 header):** round 4 implements PCI 3.0 �6.2.5.1/�6.2.5.2 all-ones BAR sizing for PIIX IDE BMIBA and UHCI, hardwires unimplemented BARs/ROM BAR to zero, and makes identity/BIST/CIS/subsystem/Cap/Min_Gnt/Max_Lat/Interrupt Pin read-only. Prior Mechanism #1 / PAM / BMIDE / PIRQ content remains. `devices::PciConfig` Mechanism #1 (`0xCF8`/`0xCFC`) + host bridge `00:00.0` `8086:1237` (Command sticky IO|MEM|BusMaster; Status CapList=0/FastB2B/DevSel=medium `0x0280` + RW1C; Cache Line Size `0x0C` + Latency Timer `0x0D` store/readback reset `0x00`) + PIIX stubs `00:01.0` ISA `8086:7000` (Command sticky IO|MEM|BusMaster ? `PCI_PIIX_ISA_COMMAND_MASK`=`0x0007` mirrors host bridge; Status CapList=0/FastB2B/DevSel=medium `PCI_PIIX_ISA_STATUS_STUB`=`0x0280` + RW1C; PIRQRC `0x60`?`0x63` default `0x80` store/readback + `assert_pirq`/`sync_pirq_to_pic`?DualPic ISA IRQ when bit7 clear (PCI INTx stub; valid IRQs 3?7/9?12/14/15); ELCR `0x4D0`/`0x4D1` store/readback reset `0x00` + MachineBus?DualPic per-IR level mask (SeaBIOS/PIIX edge/level; OR ICW1.LTIM; reserved IRQ0/1/2/8/13 hardwired edge)) / `00:01.1` IDE `8086:7010` (Command sticky IO|BusMaster; Status CapList=0/FastB2B/DevSel=medium `PCI_PIIX_IDE_STATUS_STUB`=`0x0280` + RW1C; BMIBA I/O BAR config `0x20` store/readback, bit0 forced; BMIDE 16-byte I/O at BMIBA when Command.IO set ? command/status/PRD store/readback + bounded primary PRDT walker in **both** directions (`decode_bmide_prd` / `PciConfig::start_bm_read` + `run_prd_read_stub` and `start_bm_write` + `run_prd_write_stub`; BMICOM SSBM starts and RWCON selects memory?device vs memory?device, BMISTA Active while walking, EOT-terminated, zero count = 64 KiB, 256-entry missing-EOT cap, 32-bit wrap rejected with no partial copy, BMISTA Error latched on failure; no ATA DMA command engine, no secondary PRDT engine, no BMIDE interrupt reporting, no PCI abort modeling); IDETIM `0x40` word store/readback) / `00:01.2` USB `8086:7020` (Command sticky IO|MEM|BusMaster ? `PCI_PIIX_USB_COMMAND_MASK`=`0x0007` mirrors host bridge; Status CapList=0/FastB2B/DevSel=medium `PCI_PIIX_USB_STATUS_STUB`=`0x0280` + RW1C; UHCI BAR0 I/O BAR config `0x20` store/readback, 32-byte align, bit0 forced; UHCI 32-byte I/O at BAR0 when Command.IO set ? USBCMD/USBSTS/USBINTR/FRNUM/FLBASEADD/SOFMOD/PORTSC store/readback noop stub, no schedule/DMA; LEGSUP `0xC0` dword store/readback) / `00:01.3` ACPI `8086:7113` (Command sticky IO|MEM|BusMaster ? `PCI_PIIX_ACPI_COMMAND_MASK`=`0x0007` mirrors host bridge; Status CapList=0/FastB2B/DevSel=medium `PCI_PIIX_ACPI_STATUS_STUB`=`0x0280` + RW1C; PMBASE I/O BAR config `0x40` mask `0xFFC0`|bit0 store/readback + 64-byte PM I/O decode stub at PMBASE when Command.IO set ? `PM1a_EVT`/`PM1a_CNT` stubs + live `PM_TMR` via `tick_acpi_pm`; SCI polled level only; no sleep-state machine) on `MachineBus`; round 2 adds the i440FX PMC **Programmable Attribute Map** on the host bridge (`0x59`?`0x5F`, PAM0?PAM6): the register file masks the datasheet's reserved bits to read back zero, `pam_regions` decodes the thirteen Table 3 segments in ascending address order with index 12 as the `0xF0000` BIOS area, and **`MachineBus` joins that register file to `PhysMem`'s region attributes**, so a guest configuration write to `00:00.0` `0x59`?`0x5F` actually re-attributes memory and BIOS shadowing works end to end from the guest side (`Machine::apply_pam_register` writes through to the configuration bytes too, so the host and guest views cannot diverge); round 3 makes the access-width behavior spec-exact and removes the escape hatch: only a full-DWORD write to `0xCF8` latches CONFIG_ADDRESS, a 32-bit access at `0xCF9`?`0xCFB` is not CONFIG_ADDRESS, every non-DWORD access in `0xCF8`?`0xCFB` is an ordinary I/O transaction (all ones on read, dropped on write), reserved bits 30:24 and 1:0 are never stored, CONFIG_DATA byte lanes follow the "falls inside the DWORD beginning at CONFIG_DATA" rule with byte enables copied from the processor bus, and an unimplemented function master-aborts to all ones / dropped writes per footnote 15 and �3.2.2.3.4; the opt-in byte-lane compatibility policy that existed only because the interpreter had no 32-bit `OUT` was **deleted** once `EF` landed, so the spec rules are now the only behavior; **real firmware exercises this** ? the round-3 POST trace shows SeaBIOS reading the host-bridge identity and programming PAM0?PAM6 over Mechanism #1; **not** extended (MMCONFIG) configuration space / the PMC's other legacy-memory registers (DRB, DRAMC, DRAMT, MTT; FDHC `0x68` and SMRAM `0x72` are round-5 decode+PhysMem) / any distinction between a CPU access and a PCI initiator access / full PCI device INTx storm from IDE/UHCI engines (PIRQRC software `assert_pirq` stub only) / USB host engine (schedule/TD/QH DMA/port connect/IRQ) / ACPI sleep-state machine / tables / BAR MMIO / bus mastering engine / other Command decode side effects / Status error signaling / caps/MSI/PCIe / ELCR reserved-bit hardwire
- [ ] PIIX IDE ? **partial:** `devices::IdePrimary` IDENTIFY (`0xEC`) + READ SECTORS (`0x20`) + WRITE SECTORS (`0x30`) + READ BUFFER (`0xE4`) / WRITE BUFFER (`0xE8`) 512-byte sector-buffer PIO + FLUSH CACHE (`0xE7`)/FLUSH CACHE EXT (`0xEA`) + READ NATIVE MAX ADDRESS (`0xF8`) success + WRITE VERIFY SECTORS (`0x3C`) / READ VERIFY SECTORS (`0x40`) non-data LBA28 range verify (in-range success/no DRQ; OOB IDNF) + SET MAX ADDRESS (`0xF9`) ? ERR+ABRT (no HPA) + MEDIA LOCK/UNLOCK (`0xDE`/`0xDF`) + NOP (`0x00`) + IDLE (`0xE3`)/IDLE IMMEDIATE (`0xE1`)/STANDBY IMMEDIATE (`0xE0`)/STANDBY (`0xE2`)/SLEEP (`0xE6`) + CHECK POWER MODE (`0xE5`?sector_count=`0xFF`) + RECALIBRATE (`0x10`)/SEEK (`0x70`)/INITIALIZE DEVICE PARAMETERS (`0x91`) + EXECUTE DEVICE DIAGNOSTIC (`0x90`) + SET FEATURES (`0xEF`) success stubs PIO on ports `0x1F0`?`0x1F7`/`0x3F6` with backing image + DRQ/BSY/DRDY + IRQ14?`DualPic` when nIEN=0; SET MULTIPLE MODE (`0xC6`) stores Sector Count block factor (powers of two `1..=16` per IDENTIFY word 47; IDENTIFY word 59 reports setting; invalid ? ERR+ABRT) + READ MULTIPLE (`0xC4`) / WRITE MULTIPLE (`0xC5`) LBA28 multi-sector DRQ PIO (`multiple_count` sectors/DRQ, short final block; `multiple_count==0` ? ERR+ABRT; nIEN gates INTRQ) + PACKET (`0xA0`) + IDENTIFY PACKET (`0xA1`) + SMART (`0xB0`) + READ DMA (`0xC8`) + WRITE DMA (`0xCA`) + SECURITY SET PASSWORD (`0xF1`) + SECURITY UNLOCK (`0xF2`) + SECURITY ERASE PREPARE (`0xF3`) + SECURITY ERASE UNIT (`0xF4`) + SECURITY FREEZE LOCK (`0xF5`) + SECURITY DISABLE PASSWORD (`0xF6`) + DOWNLOAD MICROCODE (`0x92`) + READ LOG EXT (`0x2F`) + WRITE LOG EXT (`0x3F`) + DATA SET MANAGEMENT (`0x06`) + TRUSTED RECEIVE (`0x5C`) + TRUSTED SEND (`0x5E`) ? ERR+ABRT on ATA master (no packet/SMART/BM-DMA/SECURITY/microcode/GPL-log/TRIM-DSM/Trusted Computing/SET MAX HPA; nIEN gates INTRQ); 48-bit Address feature set READ SECTOR(S) EXT (`0x24`) / WRITE SECTOR(S) EXT (`0x34`) PIO (two-byte-deep Features/Sector Count/LBA Low/Mid/High FIFOs read back through Device Control HOB bit7; any task-file write clears HOB; 16-bit Sector Count `0000h` = 65,536 sectors; Device bits 3:0 reserved and LBA required ? CHS ? ERR+ABRT; one DRQ block and one nIEN-gated INTRQ per sector; out-of-range ? ERR+IDNF before any DRQ so no partial write reaches media; IDENTIFY word 83 bit10 + word 86 bit10 set, words (103:100) = 48-bit capacity, words (61:60) capped at 268,435,455; READ NATIVE MAX ADDRESS clamps to 268,435,454); ATA/ATAPI-6 �9.16.1 "Device 0 only" selection rules on both channels (with the absent Device 1 selected: Device Control and non-Command Command Block writes act as if Device 0 were selected, Command writes ignored except EXECUTE DEVICE DIAGNOSTIC `0x90` ? Error `01h` + �9.12 non-PACKET signature (also on SRST), non-status reads return Device 0 content with the Device register reading back DEV=1 per Table 18, Status/Alternate Status read `00h` without clearing Device 0 interrupt pending, INTRQ released while Device 0 is deselected and reasserted on reselect, Data port cycles ignored so an in-progress Device 0 DRQ block survives a probe); `devices::IdeSecondary` same PIO incl. READ BUFFER (`0xE4`) / WRITE BUFFER (`0xE8`) + READ/WRITE MULTIPLE on `0x170`?`0x177`/`0x376` + IRQ15?`DualPic`; round 3 adds an optional PACKET (ATAPI) Device 0 on either channel that firmware can detect ? signature, IDENTIFY/READ abort behavior, and IDENTIFY PACKET DEVICE ? but `PACKET` itself is still aborted and there is no media (see the ATAPI entry); **not** an actual Device 1 / PDIAG-/DASP- detection handshake / IDENTIFY word 93 / diagnostic codes `8xh` / �9.16.2 "Device 1 only" / PACKET-device all-`00h` read rule / DEVICE RESET (`0x08`) / READ-WRITE DMA EXT / DMA QUEUED EXT / READ-WRITE MULTIPLE EXT / READ VERIFY SECTOR(S) EXT / READ NATIVE MAX ADDRESS EXT / SET MAX ADDRESS EXT / 28-vs-48-bit HPA interaction / LBA error-output reporting / HOB clear on a Data port write / Device Configuration Overlay word 7 bit 8 / PACKET media engine/SMART feature set/SECURITY feature set (passwords/SET PASSWORD PIO/unlock PIO/erase-prepare/ERASE UNIT/freeze state/DISABLE PASSWORD PIO)/DOWNLOAD MICROCODE transfer/READ/WRITE LOG EXT GPL pages/DATA SET MANAGEMENT TRIM range-list PIO/TRUSTED RECEIVE/SEND Security Protocol PIO/SET MAX ADDRESS HPA apply/SET MAX ADDRESS EXT/real BM-DMA (PRD)/slave/PCI BARs/SeaBIOS
- [ ] ATAPI ? **partial: minimal packet + CD-ROM medium + R6 packets.** Minimal PACKET path: TUR / REQUEST SENSE / INQUIRY, type `1Fh`. CD-ROM capable path (`attach_atapi_cdrom`): type `05h`, READ CAPACITY / READ(10), MODE SENSE(6)/(10) page `01h`, READ TOC format 0/1, START STOP UNIT soft eject, PREVENT/ALLOW lock (ASC `3Ah`/`53h`), DEVICE RESET (`08h`). ATA disks still abort `PACKET`/`A1h`. Host-side El Torito parse + no-emul `load_eltorito_to_7c00`; no INT 13h CD. See `docs/atapi-r4-packet-protocol.md`, `docs/atapi-r5-cdrom-medium.md`, `docs/atapi-r6-*.md`.
- [ ] VGA text mode ? **partial:** `devices::VgaText` 32 KiB plane at `0xB8000` + CRTC index/data register file via Misc Output IOAS (color `0x3D4`/`0x3D5` / mono `0x3B4`/`0x3B5`; shared file; Maximum Scan Line `0x09` store/readback with mode-03h reset default `0x0F` (Protect does not block); Offset `0x13` store/readback with mode-03h reset default `0x28` (80-col; Protect does not block; host `char_at`/`attr_at`/`put_char` row stride via `text_row_pitch_chars` = Offset�2 words?character cells); Underline Location `0x14` store/readback with mode-03h reset default `0x1F` (Protect does not block; no DW/DIV4 side effects/host underline); Start Address High/Low `0x0C`/`0x0D` store/readback with mode-03h reset default `0x0000` + `text_start_address`/`text_start_plane_offset` helpers (Protect does not block; host `char_at`/`attr_at`/`put_char` viewport relative to start; CPU `0xB8000` MMIO absolute); cursor `0x0A`/`0x0B`/`0x0E`/`0x0F` store/readback + text-mode location/offset helpers; Overflow `0x07` named const + FreeVGA bit consts (VT/VDE/VRS/SVB bit8, Line Compare bit8, VT/VDE/VRS bit9) store/readback (under Protect only bit4 / Line Compare bit8 writable); Vertical Retrace End `0x11` bit7 Protect blocks writes to indexes `0x00`?`0x07` except Overflow bit4 Line Compare; no host cursor glyph/max-scan glyph height/Line Compare split-screen/full CRTC timing) + Sequencer index/data `0x3C4`/`0x3C5` noop register file (indexes `0x00`?`0x04`, mode-03h reset defaults `03/00/03/00/02`, Clocking Mode `0x01` bit0 8/9-dot store/readback, Map Mask `0x02` with mode-03h reset default `0x03` now ANDed into the plane decode and GC write path, Character Map Select `0x03` store/readback with mode-03h reset default `0x00`, Memory Mode `0x04` with mode-03h reset default `0x02` now driving Chain-4 / Odd-Even / planar CPU address decode and the Extended Memory per-map wrap) + Graphics Controller index/data `0x3CE`/`0x3CF` noop register file (indexes `0x00`?`0x08`, mode-03h reset defaults `00/00/00/00/00/10/0E/00/FF`, Set/Reset `0x00` store/readback with mode-03h reset default `0x00`, Enable Set/Reset `0x01` store/readback with mode-03h reset default `0x00`, Data Rotate `0x03` store/readback with mode-03h reset default `0x00`, Graphics Mode `0x05` store/readback with mode-03h reset default `0x10`, Miscellaneous `0x06` with mode-03h reset default `0x0E` whose bits 3:2 Memory Map Select now choose the CPU display window for the plane decode, the GC data path, and the text MMIO path while bit1 Chain Odd/Even is a second odd/even source, Bit Mask `0x08` with mode-03h reset default `0xFF` applied on write modes 0/2 and forming the mask on write mode 3; Set/Reset, Enable Set/Reset, Color Compare, Data Rotate + Function Select, Read Map Select, and Graphics Mode now drive the `gc_read_u8`/`gc_write_u8` data path rather than storing only) + Attribute Controller address/data flip-flop `0x3C0` / data read `0x3C1` noop register file (indexes `0x00`?`0x14`, Mode Control `0x10` store/readback with mode-03h reset default `0x0C`, Overscan Color `0x11` store/readback with mode-03h reset default `0x00`, Color Plane Enable `0x12` store/readback with mode-03h reset default `0x0F`, Horizontal PEL Panning `0x13` store/readback with mode-03h reset default `0x08` + host `text_pel_pan` (9-dot Pixel Shift Count ? left-shift pels; `char_at` grid unchanged), Color Select `0x14` store/readback with mode-03h reset default `0x00` + host text DAC composition (bits 3:2 ? DAC 7:6; Mode Control P54S bit7 chooses Internal Palette or Color Select bits for DAC 5:4), mode-03h reset defaults palette `00..05/14/07/38..3F` + `0C/00/0F/08/00`, PAS default `0x20`) + DAC/PEL store/readback write index `0x3C8` / data `0x3C9` (R?G?B, 6-bit, auto-inc) / read index write + state read `0x3C7` (256�3 RAM; mode-03h-ish CGA-16 defaults + black remainder) + PEL Mask `0x3C6` R/W store/readback (default `0xFF`) + display-path AND on host text attr?DAC index helpers (`display_dac_index` / `text_attr_fg_dac_index` / `text_attr_bg_dac_index` / `display_dac_rgb`; does not alter `0x3C9` palette programming) + ATC Mode Control `0x10` bit3 (BLINK) on host text attr helpers (bg bits 6:4 / attr bit7 blink; `text_attr_fg_dac_index_for_phase` for blink-off half; no VR�32 timer) + ATC Internal Palette `0x00`?`0x0F` + Color Select/P54S host text attr?DAC composition (`atc_palette_dac_index` / fg/bg helpers; PEL Mask applied afterward) + Input Status #1 (ATC flip-flop reset + deterministic Display Disabled bit0 / Vertical Retrace bit3 via read-phase counter; not CRTC-timed; Misc Output IOAS bit0 selects color `0x3DA` / mono `0x3BA`) + Misc Output stub write `0x3C2` / readback `0x3CC` (reset default `0x67`; IOAS remaps CRTC + status port ownership; RAM Enable bit1 gates CPU text-plane `read_u8`/`write_u8`; Clock Select bits 3:2 + HSYNC/VSYNC polarity bits 6/7 store/readback) on `MachineBus` (80�25 reset fill); the CPU text path is still the legacy interleaved 32 KiB buffer and is claimed only when the selected GC Misc window covers `0xB8000`; round 2 changes **how** the bus reaches it ? `MachineBus` now registers the fixed `0xA0000`?`0xBFFFF` aperture once and routes it to `VgaText::mmio_read_u8`/`mmio_write_u8` (A20 applied first, falling through to RAM / open bus when the device does not claim the access), and Misc Output RAM Enable gates that whole aperture rather than just the text plane; a `0xB8000` access that the selected window covers still resolves to the text buffer byte for byte, does not load the Graphics Controller latches, and does not apply write modes, so the HELLO ROM path and every existing text test are unaffected; `emulator-cli --vga-text` dumps the 80�25 buffer (characters only, non-ASCII as `.`, viewport following CRTC Start Address and Offset); **round 3 makes this a real text mode**: the separate 32 KiB interleaved buffer (`VgaText::mem`) and `mmio_uses_text_buffer` are **gone**, so there is one display memory of four maps and a `0xB8000` access goes through the Graphics Controller like any other aperture access ? odd/even plus Map Mask `0x03` put the character in map 0 and the attribute in map 1 at the same even offset ? and `VgaText::render_frame` performs the actual alphanumeric display fetch, walking the CRTC address counter, selecting a font bank in map 2 from Sequencer Character Map Select and attribute bit 3, and emitting a `VgaFrame` of DAC indices already through the ATC Internal Palette, Color Select and PEL Mask, with `frame_rgba8` for a host; `emulator-cli --vga-frame` reports the rendered geometry, or says plainly that the current programming has no renderer; **there is no font at reset**, so a freshly reset device renders no glyphs, and the 80�25 / 720�400 geometry is fixed rather than derived from CRTC timing; two source conflicts are recorded rather than resolved silently (Line Graphics Enable polarity ? IBM Figure 2-79 wins; 9/8 Dot Mode polarity ? the FreeVGA register page wins); the reset fill of space + `0x07` is a model choice, not hardware; **not** a host display or window of any kind, plane-enable/overscan display side effects beyond host text helpers, ATC timing side effects/CRTC-timed status/full CRTC timing/blanking/host cursor glyph/max-scan glyph height/host underline/DW/DIV4 addressing/Misc polarity timing side effects/VR IRQ/VBE/host render
- [ ] Initial VGA graphics ? **partial (text + mode 13h + planar 16-color):** prior content remains. Round 4 planar 16-color; **no font at reset**. No CGA modes, no mode X, no host display, no timing-accurate raster.
- [ ] Initial VBE ? **partial (host-side only):** VBE 2.0 info blocks for renderable modes; banked mode-13h host linear view; **no guest LFB / `PhysBasePtr`**, no INT 10h. See `docs/vga-r5-vbe-info-blocks.md`.
- [ ] SeaBIOS integration ? **partial (POST past APM; stops at `F000:C897` with LPT + unmapped LAPIC/HPET):** round-4 integration fixes the `moffs` address-size bug that caused the write-to-ROM storm and implements PCI BAR sizing that caused the 150,360 halt; post-merge measurement exhausts 2M steps polling unclaimed ports `0xB2`/`0xB3` (see Measured SeaBIOS POST blockers). fw_cfg publishes CPU-count selectors; `--post-spin` diagnoses halt/spin. POST still does not complete. `firmware_interface::prepare_bios_rom` + `Machine::load_bios_rom` / `with_bios_rom` dual-map a BIOS image at top-of-4�?�GiB and below-1�?�MiB alias (`0xF0000` for 64�?�KiB; last 128�?�KiB at `0xE0000` when larger); synthetic blob tests; `firmware/build-scripts/build-seabios.sh` fetches/builds pinned SeaBIOS `rel-1.16.3` into `firmware/seabios/` (Linux/WSL; Git Bash if `-m32` gcc present); `firmware/manifests/seabios.json` + `docs/licensing.md` / `third_party/NOTICE` (LGPL; sources not vendored into crates); `firmware/README.md` layout; fw_cfg on `0x510`/`0x511` with signature, configured RAM size, and named-file directory (host configuration survives reset) **plus** the DMA interface on the big-endian 64-bit address register at `0x514`/`0x518` (signature read, `FWCfgDmaAccess` select/read/skip, zero-fill past item end, control writeback + register clear, guest memory through `MachineBus` `PhysMem` callbacks honoring A20; ID bit1 set only while that register is live; control bit4 write direction rejected with the spec error bit because item writeability is not modeled); `Machine::probe_post` / `emulator-cli --post-probe` give a structured first-contact POST report (retired steps, classified stop reason, first-failure `CS:IP`/RIP/linear PC + eight-byte opcode window, bounded unclaimed-port and unmapped-MMIO logs, port `0x80` POST codes, COM1 and `0x402` output; skipped when `firmware/seabios/bios.bin` is absent); port `0x80` is claimed as the IBM PC/AT diagnostic checkpoint latch (last code, 256-entry history with overflow flag, write count; cleared by `Machine::reset`; reads remain ISA open bus); round 2 adds BIOS shadowing end to end through i440FX PAM (a guest programs `0x59`?`0x5F` over `0xCF8`/`0xCFC`, copies the region onto itself, and locks it), validated option-ROM mapping at `0xC0000` via `prepare_option_rom` / `Machine::map_vga_option_rom`, the CMOS memory-size / equipment / checksum bytes and the fw_cfg `etc/e820` map published from the machine's configured RAM, an optional instruction-count step clock so timer-polling firmware terminates, a pinned SeaVGABIOS build script and licensing record, and a POST probe that now reports the correct linear PC and opcode window in a 32-bit code segment instead of an `IP16`-truncated one; round 3 adds the bounded POST event trace (`Machine::probe_post_traced` / `emulator-cli --post-trace [N]`), which records the platform sequence leading to a stop rather than only its last instruction and leaves the existing `--post-probe` output byte-identical, plus `--vga-frame` for the display fetch; **SeaBIOS POST still does not complete, but it is no longer the CPU stopping it** ? after accumulator port I/O, `CMOVcc`, `SHLD`/`SHRD` and the 4-GiB linear-address wrap fix there is **no unsupported opcode at all**: the run exhausts a 2,000,000-step budget with output byte-identical at 50,000,000, spinning in a write-to-ROM `#GP` storm at `0xFFFF6E06`, measured below; the trace shows firmware now reaching PAM, PCI configuration, CMOS and fw_cfg, while the VGA aperture is still never touched; **not** option ROM scanning or execution (nothing looks for `55 AA` headers and nothing calls a ROM's entry point, so mapping a VGA BIOS does not make INT 10h work) / any SeaVGABIOS binary (the build script has never been run) / the full fw_cfg blob set or writeable items / POST-card display or checkpoint-vs-I/O-delay classification / OVMF
- [ ] Floppy boot �?? **partial:** `devices::Fdc82077` port stub `0x3F0`�??`0x3F5`/`0x3F7` (DOR/MSR|DSR/FIFO/DIR|CCR accept; DSR bit7 performs an immediate self-clearing software reset while DOR reset remains held until bit2 release; both abort command/result/DMA state, reset PCNs, preserve media/WP/DSKCHG, and queue four ready-change Sense statuses with one IRQ; MSR RQM when out of DOR reset; Specify `0x03` �?? 2 param bytes stored, no result/IRQ; Recalibrate stub `0x07` �?? unit param, `pcn[unit]=0`, Seek End ST0 latch + IRQ6; Seek stub `0x0F` �?? HD|US + NCN, `pcn[unit]=NCN`, Seek End ST0 latch + IRQ6; Relative Seek stub `0x8F`/`0xCF` (�5.2.9) �?? DIR in cmd bit6 + HD|US + RCN, `pcn[unit]` �= RCN clamp 0..=255, Seek End ST0 latch + IRQ6; Sense Interrupt Status `0x08` �?? ST0+`pcn[ST0 US]` result + IRQ clear; Sense Drive Status `0x04` �?? HD|US param, no execution phase, ST3 result (T0 when `pcn[unit]==0`, WP when media+`write_protected`/`set_write_protected`, else 0; no IRQ); Version stub `0x10` �?? no params, result `0x90` (82077AA id), no IRQ; Configure stub `0x13` �?? 3 params stored (EIS|FIFO_DIS|POLL_DIS|FIFOTHR/PRETRK), no result/IRQ; LOCK stub `0x14`/`0x94` �?? LOCK in cmd bit7, no params, result `LOCK<<4`, no IRQ; DOR/DSR software reset preserves LOCK; when locked only Configure EFIFO/FIFOTHR/PRETRK persist while EIS/POLL reset, and when unlocked Configure returns to EFIFO=1/FIFOTHR=0/PRETRK=0; PERPENDICULAR Mode stub `0x12` �?? 1 param `OW|0|D3�??D0|GAP|WGATE` stored (OW gates Dn; soft reset clears GAP|WGATE only, preserves Dn), no result/IRQ; DUMPREG stub `0x0E` �?? no params, 10-byte result from stored state (distinct PCN0�??3, Specify, `sc_eot`, LOCK|D3�??D0|GAP|WGATE, Configure, PRETRK), no IRQ; READ DATA stub `0x06` (MT/MFM/SK forms) �?? 8 params, no-media immediate result ST0 IC=01|ST1 ND|ST2|C/H/R/N + IRQ6 (cleared on first result byte; EOT�??`sc_eot`); READ TRACK `0x02` (MFM forms, �5.1.3) �?? same 8-param/7-result; with media + N=2 + EOT in 1..=18 �?? ST0 IC=00|ST1=0|ST2=0|C/H/R=EOT/N + IRQ6 + last_sector latch of physical sectors 1..=EOT on C/H (EOT=18 full 1.44MB track; smaller EOT documented subset; MT/SK ignored) + pending_dma; no-media/wrong-N/OOR �?? ST1 ND + IRQ6; VERIFY `0x16` (MT/MFM/SK, Table 5-1) �?? same 8-param/7-result as READ DATA; with media + N=2 + valid CHS for MT=0 R..=EOT or MT=1 head0�??head1 after EOT �?? ST0 IC=00|H|US (ST0.H=final head; no DMA/host buffer; stub: success if sectors readable; ENDaddress=last) + IRQ6; no-media/wrong-N/OOR �?? ST1 ND + IRQ6; SCAN EQUAL/LOW/HIGH OR EQUAL stubs `0x11`/`0x19`/`0x1D` (MT/MFM/SK) �?? VERIFY no-media path; READ ID `0x0A` (MFM forms) �?? 1 param HD|US; with media and pcn on a formatted cylinder: ST0 IC=00|H|US, ST1=0, ST2=0, C=pcn[unit], H from param, R = next ID field from a per-unit track scan advancing 1..=18 and wrapping, N=2 + IRQ6 (position then advances; per unit; survives Seek/Recalibrate; restarts on hardware and DOR/DSR software reset); no media or pcn past cylinder 79: ST0 IC=01|H|US, ST1 MA|ND, C/H/R/N=0, position unchanged + IRQ6; Configure EIS (byte1 bit6) enforced as an implied Seek setting `pcn[unit] = C` before READ DATA / READ TRACK / READ DELETED DATA / VERIFY / SCAN / WRITE DATA / WRITE DELETED DATA (observable via DUMPREG PCN0-3, ST3 T0, and READ ID's C; mechanical so it applies even when the transfer aborts; no Seek End ST0 latch and no extra IRQ; FORMAT TRACK has no C parameter and never implies a seek; unlocked DOR/DSR software reset restores the EIS default); READ DELETED DATA `0x0C` (MT/MFM/SK, �5.1.3) �?? same 8-param / 7-result as READ DATA; with or without media �?? ST0 IC=01|ST1 ND|ST2|C/H/R/N + IRQ6 (deleted-AM engine unsupported �?? raw images have no deleted AM; honest ND not READ DATA fall-through; single-sector, MT ignored; cleared on first result byte; EOT�??`sc_eot`); WRITE DATA `0x05` (MT/MFM forms) �?? 8 params; with media + N=2 + valid CHS + WP clear �?? ST0 IC=00|ST1=0|ST2=0|C/H/R/N + IRQ6 + `latch_write_data`/`latch_write_sector`/`last_write`/`write_sector` MT=0 same-head R..=EOT or MT=1 head0�??head1 after EOT (ST0.H=final head) image write (EOT==R �?? one sector; device pre-latch or MachineBus auto-wire ISA DMA ch2 Read via `pending_dma_write_byte_count` + `dma_transfer` memory�??device + `commit_dma_write_sector` when DOR DMA/IRQ and no pre-latch; result C/H/R/N = last sector ENDaddress); media + `write_protected` �?? ST0 IC=01|ST1 NW (Not Writable, 82077AA �6.2)|ST2|C/H/R/N + IRQ6 (no image write; clear `last_write`/`dma_write_pending`); no-media/wrong-N/OOR �?? ST0 IC=01|ST1 NW|ST2|C/H/R/N + IRQ6 (EOT�??`sc_eot`); WRITE DELETED DATA `0x09` (MT/MFM, �5.1.4) �?? same 8-param / 7-result as WRITE DATA; with or without media �?? ST0 IC=01|ST1 NW|ST2|C/H/R/N + IRQ6 (deleted-AM engine unsupported �?? raw images have no deleted AM; prefer ST1 NW for write path, not ND; media+`write_protected` also NW; no image write; clear `last_write`/`dma_write_pending`; single-sector, MT ignored; cleared on first result byte; EOT�??`sc_eot`);; FORMAT TRACK `0x0D` (MFM forms) �?? 5 params (HD|US, N, SC, GPL, D); media + `write_protected` �?? ST0 IC=01|ST1 NW (Not Writable, 82077AA �6.2)|ST2|4�?undefined=0 + IRQ6 (no format write); media+!WP+N=2 �?? programmed fill sectors R=1..=SC on pcn[unit]/head with fill byte D (GPL accepted; per-sector ID DMA deferred), ST0 IC=00|ST1=0|ST2=0|4�?undefined=0 + IRQ6; no-media / unsupported N / OOR �?? ST0 IC=01|ST1 NW + IRQ6 (cleared on first result byte; SC�??`sc_eot`); `assert_irq6`�??DualPic IRQ6 when DOR DMA/IRQ enable); 1.44MB media attach/eject + `Machine::attach_floppy_image`/`with_floppy` helpers wrapping `fdc.attach_image` + CHS/read_sector/write_sector helpers + DIR DSKCHG stub (set on eject; preserved on re-attach/`reset`; cleared by Recalibrate/Seek/Relative Seek with media) + `write_protected`/`set_write_protected` for ST3 WP and WRITE DATA / FORMAT TRACK ST1 NW (reset preserves media + WP flag); READ DATA with media (N=2, valid CHS) �?? ST0 IC=00|ST1=0|ST2=0|C/H/R/N ENDaddress (last sector; ST0.H=final head) + IRQ6 + last_sector latch (MT=0 same-head R..=EOT; MT=1 from head 0 continues head 1 after EOT same cylinder; EOT==R one sector before switch) + pending_dma_byte_count/take_pending_dma_sector Vec + MachineBus auto-wire ISA DMA ch2 Write (`dma_transfer` device�??PhysMem when DOR DMA/IRQ enable; TC latched; machine-pc e2e single + MT=0 multi-sector R..=EOT); WRITE DATA with media + DOR DMA/IRQ (no device pre-latch) �?? MachineBus auto-wire ISA DMA ch2 Read (`dma_transfer` PhysMem�??sector + `commit_dma_write_sector`; TC latched); FDC READ/WRITE DATA DMA early-stop when ISA ch2 Word Count TC undershoots pending sector bytes (partial transfer; result ST0 IC=01 + ST1 EN + partial ENDaddress); no-media/wrong-N/OOR-R still ND/NW abnormal (no DMA); **not** READ TRACK full IDX/ID-sequence ND compare while transferring/INDX# rotational-latency or data-rate timing/ID fields written by FORMAT TRACK/ID-field CRC errors (ST1 DE, ST2 CRC)/ST2 WC/BC reporting/media formats other than 1.44MB/SRT/HLT step and head-settle timing/DSKCHG clearing from an implied seek/head-position gating of transfers/WRITE DELETED deleted-AM/write engine (media stays NW)/READ DELETED deleted-AM engine (media stays ND)/Gap2/WGATE timing/Configure POLL/EFIFO/FIFOTHR runtime behavior beyond reset/LOCK state/DSR POWER DOWN and data-rate timing/DREQ/DACK timing/SeaBIOS floppy e2e
- [ ] Hard-disk boot �?? **partial:** `Machine::attach_ide_image` / `with_ide` + `Machine::load_mbr_to_7c00` copies primary IDE LBA0 (prefer) or floppy CHS `(0,0,1)` into phys `0x7C00`, requires `0x55AA` signature, sets `CS:IP = 0000:7C00` for synthetic MBR handoff tests; the IDE side now also answers ATA/ATAPI-6 �9.16.1 Device 0 only selection probes and 48-bit READ/WRITE SECTOR(S) EXT PIO, which is what firmware disk enumeration exercises first; round 3 fills the CMOS fixed-disk configuration bytes (`12h`, `19h`/`1Ah`, and the AMI parameter blocks `1Bh`?`23h` / `24h`?`2Ch`) an AMI-style POST would read, though **nothing in this tree reads them** because SeaBIOS takes drive configuration from fw_cfg; **not** SeaBIOS INT 13h / partition walk / FreeDOS disk boot / CD-ROM / BM-DMA transfers driven by an ATA command
- [ ] CD-ROM boot ? **partial (host-side El Torito).** `Machine::inspect_atapi_el_torito` + `Machine::load_eltorito_to_7c00` validate the catalog and copy a no-emulation image to the load segment (default phys `0x7C00`); INT 13h CD / floppy-HDD emulation / SeaBIOS CD boot remain open. See `docs/atapi-r6-el-torito.md`.


Measured SeaBIOS POST blockers ? round 5 measurement (2026-08-09, `merge/m2-r5-parallel-16`, base `e170b7e`):

Run against the pinned SeaBIOS `rel-1.16.3` `firmware/seabios/bios.bin` on the integrated tree (APM ports, PM_TMR freerun, XBCS, SMRAM/FDHC PhysMem, privilege/TSS all present):

```text
cargo run -p emulator-cli -- --bios firmware/seabios/bios.bin --post-probe --steps 2000000

post-probe: steps=1276960 stop=step-budget-exhausted
  stop-pc        cs:ip=F000:C897 cs.d=0 eip=0x0000C897 linear_pc=0x00000000000FC897 bytes=[FA FC 66 C3 66 55 66 57]
  halt-idle      idle-steps=723040 busy-steps=1276960 idle-pct=36%
  spin           sampled=4096 window=4096 distinct=399 cycle=none
  unclaimed-port in/out LPT-ish 0x378/0x37A, 0x278/0x27A, 0x3E9, 0x2E9 (count=1 each)
  unmapped-mmio  rd page=0x00000000FEE00000 count=4
  unmapped-mmio  rd page=0x00000000FED00000 count=8
  post-codes=[] last=none
  com1="" debug=""
```

`--post-trace` / `--post-spin` share the same probe header (`post-trace: events=6321 kept=256 dropped=6065`).

**1. Progression this round:**

1. Round 4 exit: 2,000,000-step budget exhausted polling APM `0xB2`/`0xB3` at `0008:25C2`.
2. After APM stub + PM_TMR freerun (and the rest of round 5): **past the APM poll**; stop at **`F000:C897`** in a real-mode (`cs.d=0`) BIOS segment after ~1.28M busy steps with substantial HLT idle.
3. **Current first blocker:** firmware has returned to the f-segment and is no longer in a tight 3-instruction APM spin. The probe surfaces **unclaimed LPT ports** and **unmapped Local APIC (`0xFEE00000`) / HPET (`0xFED00000`)** MMIO reads. Empty `post-codes` / COM1 remain properties of this pinned SeaBIOS build (diagnostics compiled out).

**2. Five-round campaign POST arc (measured):**

| Round | Base | Stop (2M budget) | Headline |
|---|---|---|---|
| 1 | `d95a4f5` | early decode gaps ? into PM work | 32-bit same-CPL foundation |
| 2 | `133191b` | **17,218** @ `EF` (`OUT DX,eAX`) | entered 32-bit PM + CPUID |
| 3 | `0195f78` | **2,000,000** write-to-ROM `#GP` storm | no CPU stop; moffs misattributed |
| 4 | `652281e` | **2,000,000** APM poll `0008:25C2` | moffs+BAR fixed; APM next |
| 5 | `e170b7e` | **1,276,960 busy** @ `F000:C897` | APM unblocked; LPT+LAPIC/HPET |

**3. What is *not* the current blocker:** APM `0xB2`/`0xB3`, missing primary-map opcodes, moffs, BAR sizing, or a stuck-at-zero `PM_TMR`. **Standing honesty:** no guest LFB; ATAPI `05h` only with READ; `1Fh` minimal packet remains; no font; no real SMM; privilege path has no task/call gates/VM86; POST incomplete.

Measured SeaBIOS POST blockers ? round 4 measurement (2026-08-09, `merge/m2-r4-parallel-16`, base `652281e`):

Run against the pinned SeaBIOS `rel-1.16.3` `firmware/seabios/bios.bin` on the integrated tree (moffs address-size fix **and** PCI BAR sizing both present):

```text
cargo run -p emulator-cli -- --bios firmware/seabios/bios.bin --post-probe --steps 2000000

post-probe: steps=2000000 stop=step-budget-exhausted
  stop-pc        cs:ip=0008:25C2 cs.d=1 eip=0x000F25C2 linear_pc=0x00000000000F25C2 bytes=[84 C0 75 FA B8 00 FE 03]
  spin           sampled=4096 window=4096 distinct=3 cycle=3 repeats=1365
  unclaimed-port out port=0x00B3 size=1 count=1 first_value=0x00000001
  unclaimed-port out port=0x00B2 size=1 count=1 first_value=0x00000000
  unclaimed-port in  port=0x00B3 size=1 count=641633 first_value=0x00000000
  post-codes=[] last=none
  com1="" debug=""
```

`--post-trace` and `--post-spin` at the same budget are byte-identical on the probe header (trace: `events=642367 kept=256 dropped=642111`; spin uses the default 4096-sample window above).

**1. Progression this round (dated 2026-08-09):**

1. Round 3 exit: **2,000,000-step budget exhausted** in a write-to-ROM `#GP` storm at `0xFFFF6E06` (misattributed as a machine/ROM-write model bug).
2. After the CPU `moffs` address-size fix under `CS.D=1` (independently found by the **paging** and **machine** agents): clean halt at **150,360** steps ? `CLI; HLT; JMP $-1` after PCI resource assignment rejected a garbage BAR-derived base against the I/O APIC floor.
3. After PCI BAR sizing (PCI 3.0 �6.2.5.1 / �6.2.5.2) landed in the same round and was merged: **past 150,360**; the 2,000,000-step budget is exhausted again, but the stop is a different spin.
4. **Post-merge first blocker: tight poll of APM/SMI ports `0xB2`/`0xB3`.** One `OUT 0xB3, 0x01` and one `OUT 0xB2, 0x00`, then **641,633** `IN 0xB3` reads that return open-bus `0xFF`. Stop PC `0008:25C2` is `TEST AL,AL` / `JNZ` (`84 C0 75 FA`) in a 3-instruction cycle repeated ~1,365 times inside the default spin window. Empty `post-codes` / COM1 / `0x402` remain properties of this pinned SeaBIOS build (diagnostics compiled out), not evidence of a new machine gap at those surfaces.

**2. Two agents found the moffs bug independently.** The paging agent fixed absolute-offset width to follow the address-size attribute (Intel SDM Vol. 2 MOV); the machine agent reproduced the same defect with `crates/machine-pc/tests/moffs_address_size.rs` (failing case `#[ignore]`d until merge, then un-ignored and green). The write-to-ROM silent-drop model change buys **zero** further POST progress once moffs is correct ? SeaBIOS never targets ROM with a correct offset.

**3. The PCI agent's earlier prediction is confirmed.** Round 3 recorded that BAR sizing via all-ones write/read-back would be the next misbehavior after enumeration; the machine agent's 150,360 halt diagnosis matched that prediction exactly; implementing the protocol unblocked that halt on the merged tree.

**4. What is *not* the current blocker:** missing primary-map opcodes, the write-to-ROM `#GP` storm, the `0xF0000000` sweep (same moffs defect through a load), or BAR sizing. **Current first blocker for Round 5:** model whatever SeaBIOS expects on APM ports `0xB2`/`0xB3` (or an equivalent truthful response path) so the poll can complete. Standing gaps that did not move this measurement: PIIX3 XBCS BIOS write-protect not modeled; fw_cfg `etc/table-loader` absent; ATAPI peripheral type still `1Fh` with three packets; no font at reset; no privilege-changing gates / TSS / `#DF`; POST incomplete.

Measured SeaBIOS POST blockers ? round 3 measurement (2026-08-09, `merge/m2-r3-parallel-16`, base `0195f78`):

Run against the pinned SeaBIOS `rel-1.16.3` `firmware/seabios/bios.bin`, after the linear-address wrap fix:

```text
cargo run -p emulator-cli -- --bios firmware/seabios/bios.bin --post-probe --steps 2000000

post-probe: steps=2000000 stop=step-budget-exhausted
  unmapped-mmio  wr page=0x00000000F000F000 count=2048
  unmapped-mmio  wr page=0x00000000F000E000 count=4096
  ... thirteen further fully-written pages, descending ...
  unmapped-mmio  wr page=0x00000000F0000000 count=4096
  post-codes=[] last=none
  com1="" debug=""
```

**1. Read the headline before the prose: there is no CPU-level blocker any more.** For the first time the probe reports no unsupported opcode, no unsupported encoding, and no architectural fault. It exhausts the step budget instead. The same run at `--steps 50000000` ? twenty-five times the budget ? produces output that is **byte-identical apart from the step count itself**, including every page count. That is the finding: SeaBIOS is in a closed loop, and running it longer changes nothing. POST does not complete, nothing has been written to port `0x80`, COM1 or the `0x402` debug console, and there is still no firmware-authored progress signal.

**2. Measured progression across all three rounds**, one entry per CPU slice, so the effect of each is attributable:

1. Round 2 entry baseline: **2 steps**, stopped on `0F 85` (near `JNZ`) ? SeaBIOS could not pass its first branch.
2. After near `Jcc`/`SETcc`: **61 steps**, stopped on `0F BA` (Group 8).
3. After the extension moves and segment loads: **unchanged at 61 steps**.
4. After the bit and exchange instructions: **63 steps**, stopped on `0F A2` (`CPUID`).
5. After `CPUID`/`RDMSR`/`WRMSR`/`UD2`/`INVD`/`WBINVD`: **17,218 steps**, stopped on `EF` (`OUT DX, eAX`). This carried POST into 32-bit protected mode and CPU identification.
6. Round 3, after accumulator port I/O (`E5`/`E7`/`ED`/`EF`): **50,511 steps**, stopped on `0F AC` (`SHRD`).
7. After the segment `PUSH`/`POP` stack-slot fix: **unchanged at 50,511 steps** ? exactly zero.
8. After `CMOVcc`: **unchanged at 50,511 steps** ? also exactly zero. Both are recorded because a slice that moves the number by nothing is worth knowing about, and two in one round is worth knowing about twice.
9. After `SHLD`/`SHRD`: **no CPU stop at all**; the 2,000,000-step budget is exhausted instead.

**3. What firmware actually does now, from the event trace.** Round 2 could only report the last instruction. The `--post-trace` instrument (slice 12) records the sequence, and it changes the picture. In the first 17,218 instructions of round 2, SeaBIOS made only **four** platform accesses: the CMOS shutdown byte `0Fh` through `0x70`/`0x71`, then A20 through `0x92`. In round 3 the same run produces **74,435** platform events. Within the first fifty it has read the i440FX host bridge identity (`8086:1237`) over Mechanism #1, written PAM0?PAM6 to `0x33` (read/write enable ? `make_bios_writable`), read the `QEMU` fw_cfg signature at `0x511`, and driven the fw_cfg DMA register at `0x518`; CMOS reads follow, then the PIT is programmed and both 8259 interrupt-mask registers are set.

**4. Three of the four round-2 seams are now exercised by real firmware.** PAM programming (7 events), PCI configuration cycles (106 address + 104 data), CMOS `0x70`/`0x71` (41), and fw_cfg `0x51x` (26) are all reached. **The VGA aperture is still never touched ? zero events** ? which is consistent with POST never reaching video initialization.

**5. Current first blocker: a write-to-ROM `#GP` storm, not a missing instruction.** From event 315 onward the trace is a 1:1 alternation of `mem-fault wr addr=0xFFFF6E06` and `OUT 0x20, 0x20` (master PIC EOI), 37,060 times, consuming the rest of the budget. `0xFFFF6E06` is `CS.base = 0xFFFF0000` plus `0x6E06` ? SeaBIOS updating a BIOS variable in the f-segment while still executing from the top-of-4-GiB alias. This machine maps that alias read-only and the interpreter classifies a bus write fault as `#GP` (vector 13), which SeaBIOS's handler EOIs and returns from, only to retry the same store forever. Note the storm begins immediately after the run unmasks IRQ0 and programs PIT channel 0, so it is reached through interrupt delivery. **The suspect is the fault itself:** a real PC drops a write to ROM silently rather than raising `#GP`, and this model already absorbs writes to *unmapped* physical space ? it faults only on writes to *mapped* ROM, which is the inconsistent half. Changing that touches fault semantics and invalidates tests this round added, so it is a round-4 slice, not an integration-time edit.
6. **The `0xF0000000`?`0xF000FFFF` write sweep is a separate finding, not a second symptom of the wrap bug.** The wrap fix removed both above-4-GiB pages and left these sixteen pages byte-identical, which settles the question. Bisecting the step budget places the sweep between 50,000 and 100,000 steps and shows it walking **downward** from `0xF000F7FF`, so it also starts *before* the `#GP` storm at ~100,000 steps rather than being caused by it. It produces no trace events at all, because a write to unmapped physical space succeeds silently here and only the probe's page log notices. Sixty-two kibibytes of a 64 KiB region, filled downward, is the shape of an f-segment-sized copy landing sixteen bits too high, but that is a hypothesis and not a measurement; the cause is not established, so nothing was changed for it. It is a genuine memory-map gap in the sense that the machine claims nothing at `0xF0000000` ? on a real i440FX that is PCI space with no BAR assigned, where writes are master-aborted and dropped, which is what this model does.
7. **The PCI agent's prediction to test next**, recorded so it can be checked rather than assumed: SeaBIOS enters `pci_probe_devices`, enumeration should work, and then **BAR sizing via an all-ones write followed by a read-back** is expected to be the first misbehavior, because the device stubs return fixed masks and implement no sizing protocol.

Two standing caveats on every number above. The probe stops at the first CPU-level failure and does not resume, so anything past a stop is unmeasured ? that caveat now bites less, since there is no stop. And the instruction-count step clock is a model choice with no relation to real time, so a step count measures progress through the instruction stream and nothing else.

Measured SeaBIOS POST blockers ? round 2 measurement (2026-08-09, `merge/m2-r2-parallel-16`):

The POST probe harness (`Machine::probe_post` / `emulator-cli --post-probe`) replaces guesswork with a measurement. Run against the pinned SeaBIOS `rel-1.16.3` `firmware/seabios/bios.bin` on this integration branch:

```text
cargo run -p emulator-cli -- --bios firmware/seabios/bios.bin --post-probe --steps 2000000

post-probe: steps=17218 stop=unsupported opcode 0xEF cs:ip=0008:417B cs.d=1 eip=0x000F417B rip=0x00000000000F417B linear_pc=0x00000000000F417B opcode_bytes=[EF 89 F2 66 ED 48 66 83]
  post-codes=[] last=none
  com1="" debug=""
```

Read the numbers before the prose: **17,218 retired instructions, stopped in a 32-bit code segment (`cs.d=1`) at linear `0x000F417B`.** POST does not complete. Nothing has been written to port `0x80`, COM1, or the `0x402` debug console yet, so there is still no firmware-authored progress signal.

Measured progression across round 2, one entry per CPU slice, so the effect of each is attributable:

1. Baseline entering the round: **2 steps**, stopped on `0F 85` (near `JNZ`) at `F000:E062` ? SeaBIOS could not pass its first branch.
2. After near `Jcc` / `SETcc` landed: **61 steps**, stopped on `0F BA` (Group 8 `BT` family).
3. After the extension moves and segment loads landed: **unchanged at 61 steps** ? that slice added nothing on SeaBIOS's early path. Recorded because a slice that moves the number zero is worth knowing about.
4. After the bit and exchange instructions landed: **63 steps**, stopped on `0F A2` (`CPUID`).
5. After `CPUID`/`RDMSR`/`WRMSR`/`UD2`/`INVD`/`WBINVD` landed: **17,218 steps**, stopped on `OUT DX, eAX`. This is the jump that carried POST into 32-bit protected mode and through CPU identification.

The coordinator's seam wiring and the probe fix did not change the step count: it is 17,218 both before and after. That is itself a finding ? SeaBIOS reaches the port I/O gap *before* it can program PAM, touch the VGA aperture, or read the new configuration bytes, so all three newly joined seams are wired but unexercised by this firmware so far.

**Current first blocker: the accumulator port I/O forms in the PRIMARY opcode map.** The decoder's `M1_SUBSET` has only the byte forms `E4` (`IN AL, imm8`), `E6` (`OUT imm8, AL`), `EC` (`IN AL, DX`) and `EE` (`OUT DX, AL`). Missing entirely are:

- `ED` (`IN eAX, DX`) and `EF` (`OUT DX, eAX`)
- `E5` (`IN eAX, imm8`) and `E7` (`OUT imm8, eAX`)

Note that these opcodes are absent at **both** operand sizes, not just 32-bit: `EF` under a 16-bit operand size is `OUT DX, AX`, and it is equally undecodable. The captured window shows why this stops POST here ? `EF` is followed by `89 F2` (`MOV EDX, ESI`) and then `66 ED` (`IN AX, DX`), which is SeaBIOS driving PCI Configuration Mechanism #1 at `0xCF8`/`0xCFC`. Nothing about the platform can be enumerated until these four opcodes decode, which makes them the top item for round 3.

Likely-next `0F` gaps, from reading the map rather than from measurement: `CMOVcc` (`0F 40`?`0F 4F`) and `SHLD`/`SHRD` (`0F A4`/`A5`/`AC`/`AD`). Both are common in compiled firmware and neither is implemented. Treat that as a prediction, not a measurement ? the only reliable method is to re-run the probe after each slice.

Two standing caveats on every number above. The probe stops at the first CPU-level failure and does not resume, so everything past the current blocker is unmeasured. And while the probe now arms the instruction-count step clock so timer-polling loops terminate, that clock is a model choice with no relation to real time, so a step count is a measure of progress through the instruction stream and nothing else.

Implement (full M2 deliverables):

- Complete real-mode foundation
- Protected mode
- Segmentation
- GDT
- IDT
- LDT
- TSS
- Exceptions
- Interrupts
- 32-bit paging
- 8259 PIC
- 8254 PIT
- RTC
- DMA
- PS/2 controller
- PCI configuration space
- PIIX IDE
- ATAPI
- VGA text mode
- Initial VGA graphics
- Initial VBE
- SeaBIOS integration
- Floppy boot
- Hard-disk boot (MBR?`0x7C00` handoff helper partial; full BIOS disk boot later)
- CD-ROM boot

Exit criteria:

- SeaBIOS completes POST.
- FreeDOS reaches the command prompt.
- A small 32-bit Linux kernel reaches a serial shell.
- Instruction and exception test suites pass.
- Instruction coverage is tracked and no supported instruction is unlisted.

### Milestone 3: x86-64 interpreter

Estimated effort: 4 to 6 months

Implement:

- REX prefixes
- R8 through R15
- Long-mode entry and exit
- Compatibility mode
- Four-level paging
- Canonical-address checks
- NX
- EFER
- 64-bit exception delivery
- 64-bit interrupt delivery
- SYSCALL and SYSRET
- FS and GS base MSRs
- SWAPGS
- CMPXCHG16B
- x87
- MMX
- SSE
- SSE2
- SSE3
- SSSE3
- SSE4.1 profile
- Local APIC
- I/O APIC
- Minimal ACPI
- HPET
- OVMF bring-up

Exit criteria:

- An x86-64 Linux kernel boots to a serial shell under the interpreter.
- PAE and long-mode paging tests pass.
- Windows 7 x64 installer reaches graphical setup, or the remaining blocker is clearly isolated to a known device.

### Milestone 4: WebAssembly JIT

Estimated effort: 4 to 6 months

Implement:

- Typed IR
- IR validator
- Interpreter-to-IR semantic mapping
- Hot-block profiling
- WebAssembly emitter
- Translation-block cache
- Software TLB
- Lazy flags
- Self-modifying-code detection
- Code-page invalidation
- Bounded JIT cache
- JIT statistics
- Debug fallback to interpreter

Exit criteria:

- Every JIT block can be compared with interpreter execution.
- Randomized interpreter-versus-JIT tests produce no mismatches.
- BIOS, Linux, and Windows behave identically under interpreter and JIT.
- Benchmarks show a repeatable performance improvement.

### Milestone 5: Browser feature parity

Estimated effort: 4 to 6 months

Implement:

- Worker-based execution
- Display renderer
- AudioWorklet
- Disk upload
- ISO upload
- HTTP range-backed disks
- Persistent writable overlays
- Snapshot save and restore
- WebSocket network relay
- E1000 or VirtIO network
- Sound Blaster and newer audio device
- Fullscreen
- Scaling
- Performance panel
- v86 compatibility API
- Website integration

Exit criteria:

- Windows 7 x64 reliably installs and boots.
- Linux has networking and persistent storage.
- Snapshot restore reproduces CPU, memory, devices, and disk overlay state.
- The existing website can launch either v86 or the new emulator.

### Milestone 6: Two virtual CPUs

Estimated effort: 3 to 5 months

Status (2026-08-10, round-13 parallel integration `merge/m2-r13-parallel-16` on base `f579e8c`): **in progress -- thirteenth parallel campaign merged.** Four lanes: LPT1/2 + COM3/4 probe stubs + CMOS 0Fh; INT10 AH=0E/13 + CRTC cursor sync + VBE 4F01 no-LFB; INT19 bootable-media helpers + FreeDOS v6 media measure + INT13 AH=00 + Linux/El Torito deepen; VME CLI/STI VIF + PUSHF/POPF + redirect deepen + INT3. See `docs/m2-r13-parallel-integration.md`. **CF9 remains; no-media POST still `F000:9842`.** FreeDOS prompt / Linux serial shell still open. Honesty: no guest LFB; no CPUID.VME/APIC; ADR-0008 table-loader absent.
Status (2026-08-10, round-12 parallel integration `merge/m2-r12-parallel-16` on base `01050d2`): **in progress -- twelfth parallel campaign merged (+ POST CF9).** Five lanes: ICH `0xCF9` reset (`F000:C897` was `wait_irq`; causal miss was CF9/`qemu_reboot`); UHCI QH depth4 + USBSTS/USBINTR + LAPIC ICR + HPET MSI honesty; INT10 AH=01/09/0A + BDA video + VBE 4F00 no-LFB; INT13 AH=04/41 + FreeDOS/Linux measure deepen; CR4.VME sticky + redirect bitmap + 16-bit IDT from VM86 + INTO/VME. See `docs/m2-r12-parallel-integration.md`, `docs/post-c897-*.md`. **20M POST remeasure stops at `F000:9842` no-media reboot loop (past C897).** FreeDOS prompt / Linux serial shell still open. Honesty: no guest LFB; no CPUID.VME/APIC; ADR-0008 table-loader absent.
Implement:

- Application processor startup
- INIT and SIPI
- ACPI MADT
- Interprocessor interrupts
- Per-vCPU local APIC
- Atomic instructions
- Deterministic cooperative scheduler
- SMP snapshot state
- SMP stress tests
- Optional parallel worker prototype

Exit criteria:

- Linux detects and actively uses two CPUs.
- Windows detects two CPUs.
- SMP unit tests pass.
- Deterministic mode produces identical traces across repeated runs.

### Milestone 7: Windows 8.1 and Windows 10 hardening

Estimated effort: 4 to 8 months

Implement or correct:

- Remaining x64 instruction edge cases
- Precise exception ordering
- PREFETCHW
- Windows-compatible CPUID
- ACPI edge cases
- Timer behavior
- APIC race conditions
- FPU and SSE exceptions
- Storage timeout behavior
- Modern network and audio drivers
- Large-disk handling
- Browser memory-pressure handling
- Snapshot version migrations
- Performance profiling and optimization

Exit criteria:

- Windows 10 x64 reaches the desktop repeatedly from a clean boot.
- Keyboard and mouse work.
- Disk access works.
- Networking works.
- Pause and restore work.
- The guest survives an extended stability test.

### Overall scale estimate

- First custom ROM: 1 to 2 months
- FreeDOS and early legacy boot: 4 to 6 months
- x86-64 Linux interpreter boot: 10 to 16 months
- JIT and Windows 7 x64 browser build: 18 to 30 months
- Two CPUs, polished v86-like functionality, and Windows 10: 24 to 42 months

Cursor can accelerate scaffolding, test generation, repetitive opcode work, refactoring, and documentation. It does not remove the need for careful debugging of paging, exception ordering, APIC behavior, firmware assumptions, and operating-system boot failures.

---

## 22. Testing Strategy

Testing should receive at least as much engineering attention as instruction implementation.

### 22.1 Decoder tests

For every tested byte sequence, compare:

- Length
- Mnemonic
- Operand count
- Operand type
- Registers
- Operand width
- Address width
- Immediate
- Displacement
- Valid modes
- Invalid modes

Use Intel XED as the native comparison oracle.

### 22.2 Single-instruction semantic tests

Create a known initial state:

- Registers
- Flags
- Memory
- Segments
- Descriptor tables
- Control registers
- CPU mode
- Privilege level

Execute the instruction on:

- The new interpreter
- Real x86-64 hardware where practical
- QEMU TCG
- A small native oracle harness

Compare:

- General registers
- RIP
- RFLAGS
- Memory
- Segment state
- Control state
- Exception vector
- Exception error code
- Floating-point state
- SIMD state

### 22.3 Interpreter and JIT lockstep tests

For each translation block:

1. Clone the complete machine state.
2. Execute with the interpreter.
3. Execute with the JIT.
4. Compare all visible architectural state.
5. Save a minimized reproducer on mismatch.

Run these tests continuously in CI.

### 22.4 CPU-system tests

Use kvm-unit-tests for:

- Real mode
- Protected mode
- Paging
- PAE
- Long mode
- MSRs
- APIC
- SMP
- Privilege transitions
- x87
- SSE
- Exceptions

### 22.5 Firmware tests

Capture and validate:

- SeaBIOS debug stream
- Port I/O trace
- PCI enumeration
- IDE commands
- APIC state
- ACPI checksums
- Boot order
- fw_cfg reads

Compare important behavior with QEMU using the same firmware build.

### 22.6 Operating-system boot tests

Automated redistributable tests:

- FreeDOS
- Linux
- ReactOS
- BSD
- Custom kernels

Private or manual tests:

- Windows installation media
- Windows disk images
- Proprietary operating-system images

Do not commit proprietary Windows media or license keys.

### 22.7 Fuzzing

Fuzz:

- Prefix parser
- Instruction decoder
- ModRM parser
- SIB parser
- Page-table walker
- Segment translation
- MMIO dispatcher
- Port I/O dispatcher
- PCI configuration writes
- IDE command parser
- Snapshot parser
- Network-frame parser

### 22.8 Performance regression tests

Track:

- Guest instructions per second
- Translation blocks generated
- JIT cache size
- JIT compilation time
- TLB hit rate
- MMIO exits
- Port I/O exits
- Device-event frequency
- UI frame time
- Audio underruns
- Disk-cache hit rate
- Snapshot size
- Snapshot creation time
- Snapshot restore time

---

## 23. Cursor IDE Workflow (Vibe Coding)

Rules and skills are **checked into the repo** (not just described here). Keep this section and those files in sync when process changes.

### 23.1 Installed project rules

| Rule | Applies | Purpose |
|---|---|---|
| `.cursor/rules/emulator-core.mdc` | always | Spec authority, no-copy, 64-bit state, interpreter truth, CPUID honesty |
| `.cursor/rules/testing.mdc` | always | Test-first, quality gates, done report |
| `.cursor/rules/vibe-sessions.mdc` | always | One slice per chat, Plan Mode, stop conditions |
| `.cursor/rules/licensing.mdc` | always | Firmware/third-party provenance |
| `.cursor/rules/rust-core.mdc` | `crates/**/*.rs` | Core Rust conventions |
| `.cursor/rules/web-boundary.mdc` | `web/**/*` | Worker/UI/Wasm boundary |
| `.cursor/rules/instruction-metadata.mdc` | spec/decode crates | Metadata-only edits, regenerate tables |

### 23.2 Installed project skills

| Skill | Invoke when |
|---|---|
| `next-slice` | Starting a session; need the next bounded backlog item |
| `implement-instruction` | Opcode / flag / decode / interpreter work |
| `implement-device` | PIC, PIT, IDE, VGA, serial, APIC, etc. |
| `guest-boot-debug` | Firmware or OS boot hang / triple fault |
| `quality-gate` | Before calling a slice done |

### 23.3 Prompting rules

Do **not** ask Cursor for broad tasks such as:

```text
Implement x86-64 support.
Boot Windows 10.
```

Use small tasks with explicit acceptance criteria. Prefer:

```text
@AGENTS.md
Use skill next-slice (or paste a named slice).
Then Plan Mode only: files + acceptance tests.
```

Example decoder task:

```text
Use Plan Mode.

Read:
- docs/architecture.md
- docs/cpu-profile-core2.md
- docs/instruction-format.md
- docs/testing.md
- applicable Intel manual sections

Task:
Implement decoding only for legacy MOV opcodes 0x88 through 0x8B.

Constraints:
- Do not implement execution semantics.
- Support 16-bit and 32-bit address forms.
- Do not add REX handling in this task.
- Use generated instruction metadata.
- Add comparison tests against the XED wrapper.
- Add invalid and truncated instruction tests.

Before editing:
- Produce a file-level plan.
- List acceptance tests.

After editing:
- Run skill quality-gate.
- Report unsupported cases.
```

Example instruction task:

```text
Use skill implement-instruction.

Implement ADD r/m32, r32 in the interpreter.

Acceptance criteria:
- Register destination
- Memory destination
- Correct CF, PF, AF, ZF, SF, and OF
- Cross-page memory access
- Segment-limit fault before modification
- Read-only page fault before modification
- EIP updated only after successful completion
- Differential tests against the native oracle
- No JIT implementation in this issue
```

Example device task:

```text
Use skill implement-device.

Implement only the 8259 PIC initialization command words ICW1 through ICW4.
Do not implement OCW commands in this issue.

Add tests for:
- Reset state
- Single mode
- Cascaded mode
- Vector offsets
- Invalid initialization sequence
- Master and slave wiring
- Snapshot round trip
```

### 23.4 Parallel agents and worktrees

Use separate git worktrees for concurrent agents on:

- Architecture and ADRs
- Decoder and generated tables
- CPU semantics
- MMU and paging
- Device emulation
- Fuzzing
- Browser UI and integrations
- Performance profiling

**Hard lock:** do not let two agents modify CPU-state representation or instruction metadata simultaneously.

### 23.5 Git workflow

Branch naming:

```text
feat/decoder-mov-88-8b
feat/cpu-add-rm32-r32
feat/device-pic-icw
fix/mmu-page-fault-error-code
perf/jit-lazy-flags
```

Pull-request requirements:

- One bounded feature or fix
- Linked specification sections
- New tests
- No unrelated refactoring
- CI fully passing
- Unsupported behavior documented

### 23.6 Recommended Cursor modes

| Mode | Use for |
|---|---|
| Plan | Slice design, ADR drafts, file-level approach |
| Agent | Implementing one approved slice |
| Ask / Debug | Boot failures, oracle mismatches (then `guest-boot-debug`) |
| New chat | Every new slice |

---

## 24. First 30 Days

### Week 1: Architecture and repository

Create and approve:

- Project scope
- Core 2 and Penryn CPU profiles
- Initial operating-system matrix
- Machine-model ADR
- Interpreter and JIT ADR
- SMP ADR
- Firmware decision
- Licensing policy
- Source-provenance policy
- Cursor rules and skills (see �0 / �23; initially checked in)
- `AGENTS.md`
- Cargo workspace
- Web build skeleton
- CI pipeline
- Stub docs listed in �28

Do not implement devices or a large instruction set yet.

### Week 2: Machine foundation

Implement:

- CpuState
- Reset values
- RAM
- ROM mapping
- Port I/O dispatch
- MMIO dispatch
- Machine clock abstraction
- Native CLI
- Browser worker entry point

Do not work on VGA, IDE, Windows support, JIT, networking, audio, or multicore.

### Week 3: Instruction metadata and decoding

Implement:

- Instruction-definition schema
- Code generator
- Legacy-prefix parser
- Opcode-map selection
- Initial ModRM parser
- Truncated-instruction handling
- Invalid-instruction handling
- Decoder test harness
- XED comparison wrapper

### Week 4: First executable ROM

Implement enough instructions to:

- Initialize a stack
- Perform basic arithmetic
- Call functions
- Write bytes to COM1
- Write bytes to COM2 (same 16550 THR path)
- Write bytes to port 0x402
- Halt cleanly

Month-one success path:

```text
Reset vector
  -> decoder
  -> interpreter
  -> port I/O
  -> serial and debug console
  -> visible output in CLI and browser
```

Month-one definition of done:

- Native build passes.
- WebAssembly build passes.
- Custom ROM prints a message in both environments.
- Decoder tests pass.
- Instruction semantic tests pass.
- CI is green.
- Architecture documentation matches the implementation.

---

## 25. First Backlog

### Epic A: Repository and architecture

- Create Cargo workspace.
- Create native CLI crate.
- Create WebAssembly crate.
- Add formatting and clippy checks.
- Add CI.
- Add docs directory.
- Add ADR template.
- Add third-party notice process.
- Cursor rules + skills + `AGENTS.md` (bootstrap complete when this plan �0 / �23 matches `.cursor/`).

### Epic B: CPU state

- Define general-purpose registers.
- Define RIP and RFLAGS.
- Define segment selector and hidden cache structures.
- Define descriptor tables.
- Define control registers.
- Define MSR storage.
- Define reset state.
- Add serialization strategy.
- Add state comparison helper.

### Epic C: Memory buses

- Implement physical RAM.
- Implement ROM mapping.
- Implement port I/O bus.
- Implement MMIO bus.
- Implement bounds checks.
- Implement trace hooks.
- Add unit tests.

### Epic D: Decoder framework

- Define instruction metadata schema.
- Build metadata parser.
- Build code generator.
- Parse legacy prefixes.
- Parse primary opcode map.
- Parse ModRM.
- Parse 16-bit addressing.
- Parse 32-bit addressing.
- Detect truncated instructions.
- Detect overlong instructions.
- Create XED comparison tool.

### Epic E: Minimal interpreter

- MOV
- XOR
- ADD
- SUB
- CMP
- TEST
- JMP
- Jcc subset
- CALL
- RET
- PUSH
- POP
- IN
- OUT
- CLI
- STI
- HLT

### Epic F: Debug output

- COM1 model
- COM2 model (same 16550 debug-UART stub as COM1)
- Port 0x402 model
- Browser serial-output event
- CLI serial output
- Trace log format
- Register dump command

### Epic G: ROM test harness

- Small assembler-based ROM project
- Reset-vector layout
- Linker script
- Build script
- Automatic ROM test execution
- Expected-output comparison

---

## 26. Main Risks and Controls

| Risk | Control |
|---|---|
| Instruction complexity grows uncontrollably | Declarative metadata and generated coverage reports |
| x64 becomes a retrofit | Use 64-bit-capable state from the first commit |
| JIT defects are hard to isolate | Keep interpreter as the permanent reference |
| Firmware refuses to boot | Implement a known compatible machine subset and fw_cfg |
| Windows executes unsupported instructions | Conservative and truthful CPUID |
| SMP creates nondeterministic bugs | Deterministic cooperative scheduling first |
| Browser memory is exhausted | Initial 1 GiB target and bounded JIT cache |
| Cursor invents instruction behavior | Require specification citations and differential tests |
| Multiple agents create conflicting architecture | One issue and one worktree per bounded change |
| Snapshot files become incompatible | Versioned schema and migration tests |
| Licensing becomes unclear | Separate firmware and maintain NOTICE and sources records |
| Scope expands into Windows 11 and 3D graphics | Explicitly exclude them from the first machine |
| Existing website must be rewritten | Build a v86 compatibility adapter |
| Performance work hides correctness bugs | Interpreter-only mode and lockstep comparison |
| Device emulation becomes tightly coupled | Strict bus and device interfaces |
| Browser-specific code leaks into the core | Separate crates and dependency rules |

---

## 27. Definition of Done

### 27.1 Instruction feature

An instruction feature is done only when:

- Metadata exists.
- Decoder tests pass.
- Interpreter implementation exists.
- Result tests pass.
- Flag tests pass.
- Exception tests pass.
- Mode tests pass.
- Oracle comparison passes where practical.
- Documentation and coverage reports are updated.
- CPUID exposure is consistent.

### 27.2 JIT feature

A JIT feature is done only when:

- IR lowering exists.
- IR validation passes.
- Interpreter and JIT produce identical state.
- Randomized differential tests pass.
- Self-modifying-code behavior is tested.
- Translation invalidation is tested.
- Performance impact is measured.

### 27.3 Device feature

A device feature is done only when:

- Register behavior is documented.
- Reset state is tested.
- Read and write behavior is tested.
- Interrupt behavior is tested.
- DMA or memory behavior is tested where applicable.
- Snapshot round trip is tested.
- Firmware or guest-driver integration is tested.
- Trace output exists for debugging.

### 27.4 Milestone release

A milestone release is done only when:

- All exit criteria pass.
- Native tests pass.
- WebAssembly tests pass.
- Browser automation passes.
- No advertised CPU feature is unimplemented.
- No critical known corruption bug remains.
- Documentation is updated.
- Third-party licenses are current.
- Reproducible test images and procedures are documented.

---

## 28. Recommended Initial Documentation

Create these documents before significant implementation:

### docs/scope.md

- Project goals
- Non-goals
- Supported operating-system tiers
- Browser support policy
- CPU-profile policy

### docs/architecture.md

- Crate boundaries
- CPU execution pipeline
- Interpreter and JIT relationship
- Memory model
- Device model
- Browser boundary

### docs/cpu-profile-core2.md

- CPUID leaves
- Family, model, and stepping
- Supported instruction features
- Hidden features
- MSR list
- Known deviations

### docs/machine-model-pc-v1.md

- Memory map
- I/O port map
- PCI layout
- Interrupt routing
- Firmware interface
- Device list

### docs/instruction-format.md

- Metadata schema
- Operand model
- Mode restrictions
- Feature gating
- Code-generation rules

### docs/testing.md

- Oracle hierarchy
- Decoder tests
- Semantic tests
- JIT lockstep tests
- Firmware tests
- OS boot tests
- Fuzzing
- Performance tests

### docs/licensing.md

- Project license
- Firmware licenses
- Dependency policy
- Code-provenance rules
- Notice-file requirements

### docs/sources.md

Track approved authoritative references, including:

- Intel 64 and IA-32 Software Developer Manuals
- Intel processor datasheets
- PCI specifications
- ACPI specifications
- ATA and ATAPI specifications
- PS/2 and 8042 references
- 8259 and 8254 references
- SeaBIOS documentation
- OVMF and EDK II documentation
- QEMU machine documentation
- Intel XED documentation
- WebAssembly specifications
- Browser API documentation

---

## 29. Recommended Project Principles

1. Correctness before speed.
2. Interpreter before JIT.
3. Specification before implementation.
4. Tests before feature claims.
5. Truthful CPUID at all times.
6. Small issues instead of broad rewrites.
7. Deterministic execution before parallel execution.
8. Browser independence in the core.
9. Version every persistent format.
10. Keep firmware and third-party code clearly separated.
11. Preserve v86 compatibility through an adapter, not by copying architecture limitations.
12. Treat every successful OS boot as the start of testing, not the end.

---

## 30. Final Recommended Sequence

```text
Architecture and specification
    -> CPU state and buses
    -> Generated decoder
    -> Reference interpreter
    -> Minimal debug machine
    -> Legacy BIOS and devices
    -> 32-bit operating systems
    -> x86-64 and long mode
    -> APIC and ACPI
    -> x86-64 Linux
    -> WebAssembly JIT
    -> Browser storage, networking, and audio
    -> Windows 7 x64
    -> Two virtual CPUs
    -> Windows 8.1 and Windows 10 hardening
    -> Optional modern machine and Windows 11 research
```

The first executable objective is not Windows. It is a reset ROM that reliably prints through serial in both the native runner and the browser.

The first major operating-system objective is FreeDOS.

The first x86-64 objective is a small Linux kernel reaching a serial shell under the interpreter.

The first performance objective is a tested JIT that produces exactly the same architectural state as the interpreter.

The first polished release objective is Windows 7 x64 with browser storage, input, networking, snapshots, and website integration.

The long-term objective is stable Windows 10 x64 operation with one or two virtual CPUs.
