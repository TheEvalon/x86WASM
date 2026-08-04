# Browser-Based x86/x64 Emulator Development Plan

## 0. Cursor Vibe-Coding Contract

This project is intended to be built in **Cursor** with short agent sessions ("vibe coding"), not as a single monolithic AI rewrite.

**Operating rules:**

1. `AGENTS.md` is the agent entrypoint. Read it at the start of every implementation chat.
2. Project rules live in `.cursor/rules/` and apply automatically.
3. Project skills live in `.cursor/skills/` â€” invoke them by name for recurring workflows.
4. One bounded slice per chat. Prefer a new chat over expanding an old thread.
5. Plan Mode first (files + acceptance tests), then Agent Mode, then `quality-gate`.
6. Never prompt "implement x86-64" or "boot Windows". Split work until a slice fits in one focused session.
7. Specs and oracles beat model memory. No copying code from other emulators.

**Session recipe:**

```text
@AGENTS.md  â†’  /next-slice  â†’  Plan Mode  â†’  implement  â†’  /quality-gate  â†’  stop
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
- A20 gate
- Port 0x92
- Reset control
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

- COM1 serial port
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

Status (2026-08-02, branch `feat/real-mode-int-iret`): **in progress** — real-mode foundation **complete for M2 CPU scope** (RM-D/E/F/G, REP + per-instruction external IRQ poll via `pending_irq` / `Bus::poll_external_irq`, word/dword strings, full 0x66 opsize-32 tranches, #UD/#GP/#SS/#BR/#DE/#OF IVT including code-fetch MemoryFault→#GP, INS/OUTS, BCD, INTO/BOUND, ENTER nesting>0, 0F AF IMUL, address-size 0x67 ModRM/SIB + string ESI/EDI/ECX + JECXZ/LOOP*/XLAT + moffs32, unreal segment limits / sticky DS/ES/SS/FS/GS + string/CS-fetch limit checks). **not SeaBIOS-ready**. M2 exit criteria are not met (PIC/devices/PM still open).

Progress against implement list:

- [x] Partial real-mode foundation (software INT/IRET/INT3, PUSHF/POPF, far CALL/RETF/JMP, segment MOV/PUSH/POP, Jcc, XCHG, LOOP/JCXZ, Group 1/2/3 full F6/F7 /0-/7 TEST/NOT/NEG/MUL/IMUL/DIV/IDIV with #DE, Group 4/5 INC/DEC r/m FE/FF /0-/1, Group 5 CALL/JMP/PUSH r/m FF /2,/4,/6, Group 5 far CALL/JMP m16:16 FF /3,/5, string byte/word/dword ops MOVSB/W/D STOSB/W/D LODSB/W/D CMPSB/W/D SCASB/W/D (A4–A7/AA–AF) with REP/REPE/REPNE (CX=0 nop, CX loop, ZF early-exit, DF; 0x66 → dword), INS/OUTS string port I/O INSB/W/D OUTSB/W/D (6C–6F) with REP (DX port; ES:DI dest / DS:SI src with seg override on OUTS; DF steps; 0x66 → dword; `MachineBus` size-aware port_in/out), BCD adjust DAA/DAS/AAA/AAS/AAM/AAD (27/2F/37/3F/D4/D5; AAM base-0 #DE via IVT), LEA, CBW/CWD, flag ops, PUSH imm, SAHF/LAHF, INC/DEC r16, AND/OR ModRM 08-0B/20-23, AND/OR AL/AX imm 0C/0D/24/25, ADC/SBB ModRM 10-13/18-1B, ADC/SBB AL/AX imm 14/15/1C/1D, XOR ModRM 30-33, ADD/SUB ModRM byte 00/02/28/2A, SUB/XOR/CMP AL/AX imm 2C/2D/34/35/3C/3D, CMP ModRM byte 38/3A, ADD AX imm 05, related ALU ModRM forms, legacy high-byte ModR/M AL..BH via shared `gpr_u8`/`set_gpr_u8`, MOV C6/C7 r/m imm, MOV A0-A3 moffs16/moffs32 (0x67), TEST A8/A9 AL/AX/EAX imm (0x66→imm32), RM-E stack/frame: PUSHA/POPA/PUSHAD/POPAD 60/61, ENTER/ENTERD nesting 0–31 / LEAVE/LEAVE32 C8/C9 (0x66 dword display), PUSHF/POPF/PUSHFD/POPFD 9C/9D, RET/RETF iw C2/CA (0x66 far → EIP32+CS16), POP r/m16 8F /0, real-mode exception delivery via IVT for #DE/#UD/#GP/#SS (architectural decode-miss + ModRM/stack/string/moffs MemoryFault classify + cached segment-limit #GP/#SS), interruptible REP (`pending_irq` stub), RM-F LES/LDS/XLAT C4/C5/D7 (0x66→r32,m16:32), RM-G IMUL imm 69/6B, IMUL 0F AF two-operand r16/r32↔r/m (0x66→32), INTO CE (#OF trap) / BOUND 62 (#BR fault; 0x66→r32,m32&32), **0x66 opsize-32 tranche** for MOV r/m↔r / imm (89/8B/B8-BF/C7), PUSH/POP r32/imm (50-5F/68/6A), ALU ModRM+accum ADD/SUB/XOR/CMP/AND/OR/ADC/SBB + Group1 81/83, near JMP/CALL/RET, INC/DEC r32 40–4F, CWDE/CDQ 98/99, XCHG EAX,r32 91–97, **tranche-2** ENTERD/LEAVE32 + PUSHAD/POPAD + PUSHFD/POPFD, **tranche-3** Group5 INC/DEC/CALL/JMP/PUSH r/m32 + far CALL/JMP m16:32 (FF) + far CALL/JMP/RETF ptr16:32 (9A/EA/CA/CB), **tranche-4** MOV moffs EAX A1/A3 + POP r/m32 8F + MOV r32←Sreg zero-extend 8C + Group2 D1/C1/D3 + Group3 F7 opsize-32 + IMUL 69/6B imm opsize-32, **unreal limits**: sticky `load_real_mode_selector` for DS/ES/SS/FS/GS + `checked_linear_addr` on ModRM/moffs/stack/XLAT/string/CS-fetch; **asize32**: JECXZ/LOOP*/XLAT ECX/EBX)
- [x] Complete real-mode foundation (**audit gaps landed**: RM-D/E/F/G, REP/REPE/REPNE + interruptible REP (`pending_irq` / `Bus::poll_external_irq`), word/dword strings, 0x66 opsize-32 tranche + tranche-2 ENTERD/PUSHAD/PUSHFD + tranche-3 INC/DEC/XCHG/CWDE/CDQ/TEST EAX/LES/LDS/BOUND/far ptr16:32/Group5 r/m32 + tranche-4 moffs EAX/POP r/m32/MOV Sreg→r32/Group2 D1/C1/D3/Group3 F7/IMUL 69/6B, #UD IVT, INS/OUTS, BCD, INTO/BOUND, ENTER nesting>0, 0F AF IMUL, decode-miss architectural #UD + stack/ModRM/string/moffs/code-fetch #GP/#SS classify, address-size 0x67 ModRM/SIB EA + string ESI/EDI/ECX + moffs32 + JECXZ/LOOP* ECX + XLAT EBX, unreal segment limits / sticky data-seg cache + string-op + CS-fetch limit checks, per-instruction + REP external IRQ poll) — **not SeaBIOS-ready**; **M2 CPU real-mode foundation complete**. Honest remaining (not blocking foundation checkbox): MOVSQ/STOSQ/… qword strings N/A in REX-less real mode (long-mode / REX.W later); asize64 (RCX/JRCXZ/RBX); ENTER/PUSHA/POPA/LEAVE asize32 under 0x67 (`Unsupported` — needs ESP stack helpers); full 8259 PIC / hardware IRQ routing; LGDT/SGDT GDTR load/store landed (still no PE/PM entry; unreal limit *application* via PM descriptor loads still open — tests may poke `SegmentReg.limit` directly).
- [ ] Protected mode — **partial:** real-mode `LGDT`/`SGDT`/`LIDT`/`SIDT` (`0F 01 /2`,`/0`,`/3`,`/1`) load/store `CpuState::gdtr`/`idtr` (m16&32; opsize-16 → 24-bit base, `0x66` → 32-bit; mod=11 `#UD`); `SMSW`/`LMSW` (`0F 01 /4`,`/6`) read/write `CR0[15:0]` (LMSW cannot clear PE); `MOV r32,CR0`/`MOV CR0,r32` (`0F 20`/`0F 22` `/0`) read/write full 32-bit `CR0` (unlike LMSW, MOV CR0 **can** clear PE; mod field architecturally ignored, always register-direct; CR1 → `#UD`; CR2/CR3/CR4 → explicit `Unsupported`); `CLTS` (`0F 06`) clears `CR0.TS` (bit 3) only (other CR0 bits including PE preserved); PE bit still does **not** enter protected-mode execution (segment loads stay real-mode/sticky-unreal `selector<<4`); **not** far jumps to PM, descriptors, privilege, paging (CR0.PG), CR2/CR3/CR4, PM CLTS `#GP`
- [ ] Segmentation (beyond real-mode base<<4)
- [ ] GDT — **partial:** GDTR programmable via `LGDT`/`SGDT`; **not** descriptor loads / protected-mode segment checks
- [ ] IDT — **partial:** IDTR programmable via `LIDT`/`SIDT` (real-mode IVT via `idtr.base`); **not** protected-mode gate descriptors / privilege
- [ ] LDT
- [ ] TSS
- [x] Exceptions (real-mode IVT fault delivery for #DE/#UD/#BR/#GP/#SS; #OF trap via INTO; software INT/INT3/INTO) â€” protected-mode / full fault taxonomy still open
- [ ] Interrupts (hardware / PIC-driven) — **partial:** `CpuState::pending_irq` / `request_interrupt` + per-instruction + REP poll when IF=1; `CpuState::pending_nmi` / `request_nmi` + `Machine::inject_nmi` (CMOS-gated `#NMI` vector 2, IF-independent); `MachineBus::poll_external_irq` syncs PIT ch0 OUT→IRQ0 + 8042 OBF∧INT1→IRQ1 + FDC IRQ6 + CMOS IRQF→IRQ8 + 8042 AUX OBF∧INT12→IRQ12 + primary IDE→IRQ14 + secondary IDE→IRQ15 then `DualPic::poll_irq`
- [ ] 32-bit paging
- [ ] 8259 PIC — **partial:** `devices::DualPic` ICW1–ICW4 + OCW1 IMR + OCW2 EOI + OCW3 IRR/ISR read + OCW3 poll command (`P=1` one-shot acknowledging command-port read → `0x80|level`, IMR/fully-nested aware, software-sequenced cascade poll; no-pending byte `0x00` documented model choice) + edge IRQ assert + cascade + `MachineBus` ports / `poll_external_irq`; **not** Auto-EOI/rotate/special-mask/special-fully-nested/level-triggered runtime
- [ ] 8254 PIT — **partial:** `devices::Pit8254` ch0 programming + `ce`/OUT tick (modes 0/1/2/3/4/5 incl. mode 1 retriggerable one-shot + mode 4/5 one-CLK strobe + GATE-pin summary semantics) + `Machine::tick_pit` → IRQ0→PIC; ch2 GATE/OUT + port `0x61` speaker bits (no host audio); ch1 stub accept; **not** host-real-time / host speaker audio / mode 3 exact 50% duty / BCD counting during tick / read-back command / one-CLK count-load+trigger delay / ch0-ch1 GATE input (tied high, so modes 1/5 trigger only on ch2)
- [ ] RTC — **partial:** `devices::CmosRtc` index/data + PIE/AIE/UIE + status C read-to-clear + Status A UIP + `tick_second` full BCD calendar cascade (sec→min→hour→date→month→year→century `0x32`, day-of-week 1–7, Gregorian leap years, SET inhibits) + `Machine::tick_cmos`/`tick_cmos_second` → IRQ8→PIC; port `0x70` bit7 NMI mask R/W + `nmi_masked` / `Machine::nmi_delivery_enabled` + `Machine::inject_nmi` → `#NMI` IVT vector 2 (IF-independent; masked drop); **not** host wall-clock/NTP sync / SMRAM/SMI/NMI nesting / exact crystal UIP width / binary (DM) + 12-hour modes
- [ ] DMA — **partial:** `devices::Dma8237` dual 8237A addr/count/mode/mask + AT page regs on `MachineBus`; **not** transfer engine / DREQ/DACK/TC / device integration
- [ ] PS/2 controller — **partial:** `devices::I8042` + `Machine::kbd` on `MachineBus` ports `0x60`/`0x64` (self-test/config/enable + OBF∧INT1→IRQ1 + make-code inject stub + `0xD0`/`0xD1` output-port A20 → `PhysMem`); second (auxiliary) port controller side: `0xA7`/`0xA8` config bit5 aux clock disable/enable, `0xA9` test-aux → `0x00` on normal OBF, `0xD4` host→aux byte recorded (`last_aux_device_write`/`aux_device_writes`, no response), status bit5 AUX OBF + `inject_aux_byte`/`Machine::kbd_inject_aux_byte` → AUX OBF∧config bit1 → IRQ12 (slave IR4) via `poll_external_irq` (keyboard data drives IRQ1 only, aux data IRQ12 only; `0x60` read clears both); **not** any PS/2 mouse device (`0xFF`/`0xFA`/`0xF4`, sample rate, resolution, movement packets, wheel/5-button), aux clock gating of `0xD4` writes, full AT keyboard protocol / Set2↔Set1 translation, or pulse-reset
- [ ] PCI configuration space — **partial:** `devices::PciConfig` Mechanism #1 (`0xCF8`/`0xCFC`) + host bridge `00:00.0` `8086:1237` + PIIX stubs `00:01.0` ISA `8086:7000` / `00:01.1` IDE `8086:7010` / `00:01.2` USB `8086:7020` / `00:01.3` ACPI `8086:7113` on `MachineBus`; **not** USB host / ACPI PM I/O / SMI / tables / BAR MMIO / bus mastering / caps/MSI/PCIe
- [ ] PIIX IDE — **partial:** `devices::IdePrimary` IDENTIFY (`0xEC`) + READ SECTORS (`0x20`) + WRITE SECTORS (`0x30`) PIO on ports `0x1F0`–`0x1F7`/`0x3F6` with backing image + DRQ/BSY/DRDY + IRQ14→`DualPic` when nIEN=0; IDENTIFY PACKET (`0xA1`) → ERR+ABRT on ATA master; `devices::IdeSecondary` same PIO on `0x170`–`0x177`/`0x376` + IRQ15→`DualPic`; **not** PACKET media/DMA/slave/PCI BARs/SeaBIOS
- [ ] ATAPI — **partial:** ATA master rejects IDENTIFY PACKET DEVICE (`0xA1`) with ABRT (SeaBIOS probe); **not** PACKET (`0xA0`) / CD-ROM / slave ATAPI identify buffer / ISO boot
- [ ] VGA text mode — **partial:** `devices::VgaText` 32 KiB plane at `0xB8000` + CRTC index/data `0x3D4`/`0x3D5` noop register file + Misc Output stub write `0x3C2` / readback `0x3CC` (reset default `0x67`, store only) on `MachineBus` (80×25 reset fill); **not** sequencer/GC/ATC/Misc bit side effects/timing/graphics/VBE/host render
- [ ] Initial VGA graphics
- [ ] Initial VBE
- [ ] SeaBIOS integration
- [ ] Floppy boot — **partial:** `devices::Fdc82077` port stub `0x3F0`–`0x3F5`/`0x3F7` (DOR/MSR/FIFO/DIR/CCR accept; MSR RQM when out of DOR reset; Specify `0x03` → 2 param bytes stored, no result/IRQ; Recalibrate stub `0x07` → unit param, `pcn=0`, Seek End ST0 latch + IRQ6; Seek stub `0x0F` → HD|US + NCN, `pcn=NCN`, Seek End ST0 latch + IRQ6; Sense Interrupt Status `0x08` → ST0+PCN result + IRQ clear; Sense Drive Status `0x04` → HD|US param, no execution phase, ST3 result stub (T0 when shared `pcn==0`, WP always 0, no IRQ); Version stub `0x10` → no params, result `0x90` (82077AA id), no IRQ; Configure stub `0x13` → 3 params stored (EIS|FIFO_DIS|POLL_DIS|FIFOTHR/PRETRK), no result/IRQ; LOCK stub `0x14`/`0x94` → LOCK in cmd bit7, no params, result `LOCK<<4`, no IRQ; soft DOR reset preserves LOCK and Configure EFIFO/FIFOTHR/PRETRK when locked; `assert_irq6`→DualPic IRQ6 when DOR DMA/IRQ enable); **not** READ/WRITE/Configure bit enforcement (FIFO/EIS/POLL beyond LOCK soft-reset protection)/media engine/DMA ch2 transfers/SeaBIOS floppy path
- [ ] Hard-disk boot
- [ ] CD-ROM boot

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
- Hard-disk boot
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
- Cursor rules and skills (see Â§0 / Â§23; initially checked in)
- `AGENTS.md`
- Cargo workspace
- Web build skeleton
- CI pipeline
- Stub docs listed in Â§28

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
- Cursor rules + skills + `AGENTS.md` (bootstrap complete when this plan Â§0 / Â§23 matches `.cursor/`).

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
