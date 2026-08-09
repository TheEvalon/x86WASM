# Double fault and triple fault (Milestone 2, round 5)

When protected-mode **exception** delivery fails, escalate to `#DF`
(vector 8, error code 0). A failure while delivering `#DF` is a host-visible
triple fault rather than a silent continue.

## Authority

| Rule | Section |
|---|---|
| `#DF` vector 8, error code 0 | Vol. 3 §6.15 Interrupt 8 |
| Fault during `#DF` delivery → triple fault / shutdown | Vol. 3 §6.15 |
| Delivery frame / gate mechanics | Vol. 3 §6.12.1 |

No implementation from another emulator was read or copied.

## Supported

* `deliver_fault` retries through vector 8 with error code 0 after a
  `ProtectedModeExceptionDelivery` on any other exception vector.
* A second `ProtectedModeExceptionDelivery` while entering `#DF` becomes
  `ExecError::TripleFault`.
* Software INT / NMI / IRQ delivery failures are **not** escalated (they stay
  `ProtectedModeExceptionDelivery`).

## Not supported

* The full contributory-exception decision table for `#DF` when the *first*
  exception itself was successfully entered; task-gate `#DF`; machine check
  after triple fault.
