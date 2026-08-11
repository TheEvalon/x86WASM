# LAPIC SVR stub polish — Milestone 2 Round 14

## Spec

SDM Vol. 3A §10.9 — vector + soft-enable + Focus sticky; EOI-Broadcast Suppression dropped. CPUID.APIC stays clear.

## Tests

svr_store_readback_drops_eoi_suppress; svr_soft_enable_gates_timer_and_inject
