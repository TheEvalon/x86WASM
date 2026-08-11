# R14 storage note — MBR→VBR host chain

Companion to `docs/boot-r14-mbr-vbr-chain.md`.

Host `Machine::load_active_vbr_to_7c00` reads IDE LBA0, finds the first active
(`80h`) partition, copies that LBA’s 512-byte sector to phys `0x7C00`, requires
`0x55AA`, sets `CS:IP = 0000:7C00`.

**Not** guest INT 13h AH=02h, SeaBIOS INT 19h, or FreeDOS kernel load.
