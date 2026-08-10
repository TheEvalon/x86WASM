# R8 El Torito no-emulation load to 0x7C00

Milestone 2, round 8, boot-guest lane, slice 3.

Canonical detail: [`boot-r8-el-torito-load.md`](boot-r8-el-torito-load.md).

## Honesty

Host-side helper only — **not** guest INT 13h CD / SeaBIOS CD boot. Restored
explicitly after R7 merge kept inspect-only (`inspect_atapi_el_torito`).
