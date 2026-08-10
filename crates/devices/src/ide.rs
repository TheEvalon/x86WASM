//! Primary ATA IDE channel — IDENTIFY + READ/WRITE SECTORS PIO + IRQ14 stub.
//!
//! Classic PC primary command block `0x1F0`–`0x1F7` and control block `0x3F6`.
//!
//! # Spec refs
//!
//! - ATA / ATAPI Command Set — IDENTIFY DEVICE (`0xEC`), READ SECTORS (`0x20`),
//!   WRITE SECTORS (`0x30`), WRITE VERIFY SECTORS (`0x3C`), READ VERIFY
//!   SECTORS (`0x40`), SET MULTIPLE MODE (`0xC6`), READ MULTIPLE (`0xC4`),
//!   WRITE MULTIPLE (`0xC5`), PACKET (`0xA0`), IDENTIFY PACKET DEVICE (`0xA1`),
//!   SMART (`0xB0`), READ DMA (`0xC8`), WRITE DMA (`0xCA`), SECURITY SET
//!   PASSWORD (`0xF1`), SECURITY UNLOCK (`0xF2`), SECURITY ERASE PREPARE
//!   (`0xF3`), SECURITY ERASE UNIT (`0xF4`), SECURITY FREEZE LOCK (`0xF5`),
//!   SECURITY DISABLE PASSWORD (`0xF6`), DOWNLOAD MICROCODE (`0x92`), READ LOG
//!   EXT (`0x2F`), WRITE LOG EXT (`0x3F`),
//!   DATA SET MANAGEMENT (`0x06`), TRUSTED RECEIVE (`0x5C`), TRUSTED SEND
//!   (`0x5E`), READ BUFFER (`0xE4`), WRITE BUFFER (`0xE8`), task-file
//!   registers, status bits BSY/DRDY/DRQ/ERR, error ABRT,
//!   LBA28 addressing;
//!   device control nIEN; INTRQ when drive needs attention.
//! - OSDev ATA PIO Mode — primary port map, IDENTIFY/READ/WRITE IRQ+PIO sequence,
//!   status read clears IRQ / alternate status does not, 256-word PIO,
//!   sector-count `0` = 256 sectors; primary channel → ISA IRQ14;
//!   WRITE: host fills data port after DRQ; ATAPI probe via `0xA1` / PACKET.
//! - IBM PC/AT IDE — alternate status / device control at `0x3F6`; IRQ14.
//! - Intel 8259A — DualPic IR14 (slave IR6) vectoring via MachineBus.
//! - ATA/ATAPI-6 (T13/1410D r3b) §7.7 DEV bit, §7.8.5 Device Control,
//!   §5.2.9 INTRQ, §9.12 Signature and persistence, §9.16.1 "Device 0 only
//!   configurations", Table 18 "Device 1 is selected and Device 0 is
//!   responding for Device 1", §8.11 / Table 26 EXECUTE DEVICE DIAGNOSTIC —
//!   see `docs/ide-device-selection.md`.
//! - ATA/ATAPI-6 §6.20 "48-bit Address feature set" + Table 11, §6.2.1 /
//!   §6.2.2 capacity and addressing-error rules, §7.8.6 Device Control HOB,
//!   §8.35 READ SECTOR(S) EXT (`24h`), §8.63 WRITE SECTOR(S) EXT (`34h`),
//!   Table 27 IDENTIFY words 83/86 bit10 and words (103:100) —
//!   see `docs/ide-lba48.md`.
//! - `docs/machine-model-pc-v1.md`, `plan.md` §15.5 / §21 PIIX IDE / ATAPI.
//!
//! # Scope (this slice)
//!
//! - Primary channel master only; optional backing image (`Vec<u8>`)
//! - Commands: IDENTIFY (`0xEC`), READ SECTORS (`0x20`), WRITE SECTORS (`0x30`) PIO
//! - PACKET Command feature set **detection only**
//!   ([`IdePrimary::attach_atapi_device`]): a configured packet device reports
//!   the ATA/ATAPI-6 §9.12 signature (`01h`/`01h`/`14h`/`EBh`) with Status
//!   `00h` after every reset and EXECUTE DEVICE DIAGNOSTIC, aborts IDENTIFY
//!   DEVICE with that signature in place (§6.8.1 / §8.15.5.2), aborts READ
//!   SECTOR(S) with the LBA Mid/High signature (§8.34.5.2), and answers
//!   IDENTIFY PACKET DEVICE (`0xA1`) with a 256-word block (§8.16). See
//!   `docs/atapi-r3-identify-and-signature.md`
//! - IDENTIFY PACKET DEVICE (`0xA1`): ATA master → ERR+ABRT (§8.16.2 "use
//!   prohibited" for non-PACKET devices)
//! - PACKET (`0xA0`): on a configured packet device, the ATA/ATAPI-6 §8.21 /
//!   §9.8 protocol — Features DMA/OVL rejection, Byte Count Limit from
//!   Cylinder Low/High, a 12-byte command-packet DRQ phase with Interrupt
//!   Reason C/D=1 I/O=0 and **no** INTRQ, a byte-count-limited data-in phase
//!   with C/D=0 I/O=1 and one INTRQ per block, and completion with C/D=1
//!   I/O=1. Three packet commands are implemented — `TEST UNIT READY`
//!   (`00h`), `REQUEST SENSE` (`03h`) and `INQUIRY` (`12h`) — and every other
//!   operation code is CHECK CONDITION with sense key ILLEGAL REQUEST. On an
//!   ATA disk the command is still ERR+ABRT (§8.21.2 "use prohibited"). See
//!   `docs/atapi-r4-packet-protocol.md`
//! - DEVICE RESET (`0x08`): on a configured packet device, ATA/ATAPI-6 §8.7 —
//!   Error `01h`, the §9.12 PACKET signature, Status `00h`, any command in
//!   progress and any pending sense dropped, and **no INTRQ**; Device Control
//!   and the Device register are untouched. On an ATA disk → ERR+ABRT
//!   (§8.7.2 "use prohibited"). IDENTIFY PACKET DEVICE words 82 and 85 bit 9
//!   now claim it. See `docs/atapi-r4-sense-and-device-reset.md`
//! - SMART (`0xB0`): ATA master → ERR+ABRT (no SMART feature-set data);
//!   absent/slave → status 0; INTRQ follows nIEN like PACKET/READ MULTIPLE abort
//! - READ DMA (`0xC8`): ATA master → ERR+ABRT (no BM-DMA/PRD engine);
//!   absent/slave → status 0; INTRQ follows nIEN like SMART/PACKET abort
//! - WRITE DMA (`0xCA`): ATA master → ERR+ABRT (no BM-DMA/PRD engine);
//!   absent/slave → status 0; INTRQ follows nIEN like READ DMA abort
//! - SECURITY SET PASSWORD (`0xF1`): ATA master → ERR+ABRT (no SECURITY feature
//!   set / password PIO); absent/slave → status 0; INTRQ follows nIEN like
//!   SECURITY UNLOCK / FREEZE LOCK
//! - SECURITY ERASE PREPARE (`0xF3`): ATA master → ERR+ABRT (no SECURITY erase
//!   prepare / password state); absent/slave → status 0; INTRQ follows nIEN like
//!   SECURITY SET PASSWORD
//! - SECURITY ERASE UNIT (`0xF4`): ATA master → ERR+ABRT (no SECURITY erase /
//!   password PIO); absent/slave → status 0; INTRQ follows nIEN like
//!   SECURITY ERASE PREPARE
//! - SECURITY FREEZE LOCK (`0xF5`): ATA master → ERR+ABRT (no SECURITY feature
//!   set / freeze state); absent/slave → status 0; INTRQ follows nIEN like SMART
//! - SECURITY DISABLE PASSWORD (`0xF6`): ATA master → ERR+ABRT (no SECURITY
//!   password disable / password PIO); absent/slave → status 0; INTRQ follows
//!   nIEN like SECURITY ERASE UNIT
//! - DOWNLOAD MICROCODE (`0x92`): ATA master → ERR+ABRT (no microcode download /
//!   vendor transfer); absent/slave → status 0; INTRQ follows nIEN like SMART
//! - READ LOG EXT (`0x2F`): ATA master → ERR+ABRT (no GPL / log page PIO);
//!   absent/slave → status 0; INTRQ follows nIEN like SMART
//! - WRITE LOG EXT (`0x3F`): ATA master → ERR+ABRT (no GPL / log page PIO);
//!   absent/slave → status 0; INTRQ follows nIEN like READ LOG EXT
//! - DATA SET MANAGEMENT (`0x06`): ATA master → ERR+ABRT (no TRIM / DSM range
//!   list PIO); absent/slave → status 0; INTRQ follows nIEN like WRITE LOG EXT
//! - TRUSTED RECEIVE (`0x5C`): ATA master → ERR+ABRT (no Trusted Computing /
//!   Security Protocol PIO); absent/slave → status 0; INTRQ follows nIEN like DSM
//! - TRUSTED SEND (`0x5E`): ATA master → ERR+ABRT (no Trusted Computing /
//!   Security Protocol send PIO); absent/slave → status 0; INTRQ follows nIEN
//!   like TRUSTED RECEIVE
//! - READ BUFFER (`0xE4`): ATA master → 512-byte DRQ PIO from device sector
//!   buffer (synced from READ/WRITE SECTORS / WRITE BUFFER); absent/slave →
//!   status 0; INTRQ follows nIEN like other PIO data-in commands
//! - WRITE BUFFER (`0xE8`): ATA master → 512-byte host→device DRQ PIO into the
//!   sector buffer (no LBA / no media write); absent/slave → status 0; INTRQ
//!   follows nIEN like other PIO data-out commands (WRITE SECTORS)
//! - SET MULTIPLE MODE (`0xC6`): store Sector Count block factor when a power of
//!   two in `1..=16` (IDENTIFY word 47 max); IDENTIFY word 59 reports setting;
//!   invalid → ERR+ABRT
//! - READ MULTIPLE (`0xC4`) / WRITE MULTIPLE (`0xC5`): LBA28 PIO using stored
//!   `multiple_count` sectors per DRQ block (last block may be shorter);
//!   `multiple_count==0` → ERR+ABRT; INTRQ once per block / completion when nIEN=0
//! - READ SECTOR(S) EXT (`0x24`) / WRITE SECTOR(S) EXT (`0x34`): 48-bit
//!   Address feature set PIO using the two-byte deep Features / Sector Count /
//!   LBA Low / LBA Mid / LBA High FIFOs and Device Control HOB (bit7); 16-bit
//!   Sector Count with `0000h` = 65,536; one DRQ block and one INTRQ per
//!   sector; Device register LBA bit required (CHS → ERR+ABRT); a range
//!   outside the user-addressable sectors → ERR+IDNF before any DRQ. IDENTIFY
//!   word 83 bit10 / word 86 bit10 and words 100–103 report the feature set
//!   and 48-bit capacity, and words 60–61 are capped at 268,435,455
//! - Status: BSY/DRDY/DRQ/ERR; alt status at `0x3F6` (no IRQ clear)
//! - Device control: SRST (bit2) software reset; nIEN gates IRQ14
//! - Device selection (DEV bit4 of the Device register): this channel models
//!   Device 0 only, so ATA/ATAPI-6 §9.16.1 "Device 0 only configurations"
//!   applies when the host selects Device 1 — Device Control and non-Command
//!   Command Block writes land in Device 0; Command register writes are
//!   ignored except EXECUTE DEVICE DIAGNOSTIC (`0x90`); non-status Command
//!   Block reads return Device 0 content (the Device register reads back with
//!   DEV set); Status and Alternate Status read `00h`; INTRQ is released while
//!   Device 0 is deselected and reasserted on reselect (§5.2.9) without losing
//!   interrupt pending. Data port cycles for Device 1 are ignored (Table 18
//!   defines only BSY=0/DRQ=0 cases; documented model choice)
//! - Signature: power-on / software reset and EXECUTE DEVICE DIAGNOSTIC write
//!   the non-PACKET signature Sector Count `01h` / LBA Low `01h` / LBA Mid
//!   `00h` / LBA High `00h`, or the PACKET signature `01h`/`01h`/`14h`/`EBh`
//!   on a configured packet device (ATA/ATAPI-6 §9.12)
//! - IRQ14: assert when DRQ ready / error / command-complete if nIEN=0;
//!   status register read clears pending IRQ; `irq_line()` for MachineBus
//! - `PortDevice` for MachineBus wiring
//!
//! # Unsupported (explicit)
//!
//! - Every packet command except `TEST UNIT READY`, `REQUEST SENSE` and
//!   `INQUIRY`. There is no
//!   medium of any kind — no `READ (10)`, no `READ TOC`, no `READ CAPACITY`,
//!   no tray, no medium-change notification, no ISO image path and no CD-ROM
//!   boot. IDENTIFY PACKET DEVICE word 0 and INQUIRY byte 0 both report
//!   peripheral device type `1Fh` "unknown or no device type" rather than
//!   `05h` CD-ROM, because that is what this device is
//! - Packet data-out (host → device) transfers; only data-in is implemented
//! - PACKET with the Features DMA or OVL bit set (ERR+ABRT), overlap, command
//!   queuing, SERVICE, and release interrupts
//! - Unit Attention: a reset does not queue a `6h` UNIT ATTENTION sense
//!   condition, it clears the sense data instead
//! - Deferred errors, descriptor-format sense data, and every sense key other
//!   than NO SENSE and ILLEGAL REQUEST
//! - A packet device on Device 1 (slave); the packet configuration applies to
//!   Device 0 of a channel
//! - SMART feature set (thresholds, return data, enable/disable subcommands)
//! - SECURITY feature set (passwords, SET PASSWORD PIO, FREEZE LOCK state,
//!   unlock/ERASE PREPARE/ERASE UNIT)
//! - DOWNLOAD MICROCODE transfer / vendor microcode apply (ABRT-only stub)
//! - READ/WRITE LOG EXT / General Purpose Logging (log pages, LBA48 HOB) (ABRT-only stubs)
//! - DATA SET MANAGEMENT / TRIM range-list PIO (ABRT-only stub)
//! - TRUSTED RECEIVE / TRUSTED SEND Security Protocol PIO (ABRT-only stubs for
//!   `0x5C` / `0x5E`)
//! - Real BM-DMA / UDMA/MDMA / PRD engine (READ/WRITE DMA are ABRT-only)
//! - The rest of the 48-bit Address feature set: READ/WRITE DMA EXT, READ/WRITE
//!   DMA QUEUED EXT, READ/WRITE MULTIPLE EXT, READ VERIFY SECTOR(S) EXT,
//!   READ NATIVE MAX ADDRESS EXT, SET MAX ADDRESS EXT, FLUSH CACHE EXT is a
//!   success stub only. Only READ SECTOR(S) EXT / WRITE SECTOR(S) EXT are
//!   implemented, which is what IDENTIFY word 83 bit10 claims
//! - Error-output LBA reporting for the failing sector of an EXT command (the
//!   task file keeps the command address instead)
//! - HOB clearing on a Data port write (task-file register writes only)
//! - An actual Device 1 on either channel — only the ATA/ATAPI-6 §9.16.1
//!   Device-0-only responses are modeled; there is no second drive, no
//!   PDIAG-/DASP- device-1 detection handshake, no ATA/ATAPI-6 §9.16.2
//!   "Device 1 only" configuration, and no per-device task file
//! - Data port behavior for Device 1 while Device 0 has BSY or DRQ set
//!   (outside Table 18; modeled as an ignored cycle)
//! - SeaBIOS / PCI IDE BAR remapping
//!
//! Secondary channel (`IdeSecondary`) remaps the same ATA PIO stub — including
//! READ BUFFER (`0xE4`) / WRITE BUFFER (`0xE8`) sector-buffer PIO and
//! READ/WRITE MULTIPLE multi-sector DRQ — to ports `0x170`–`0x177` / `0x376`
//! and ISA IRQ15 (see below).

use crate::PortDevice;

/// Primary ATA data port (16-bit PIO).
pub const IDE_PRIMARY_DATA: u16 = 0x1F0;
/// Error (R) / Features (W).
pub const IDE_PRIMARY_ERROR: u16 = 0x1F1;
/// Sector count.
pub const IDE_PRIMARY_SECCOUNT: u16 = 0x1F2;
/// LBA 7:0 / sector number.
pub const IDE_PRIMARY_LBA_LO: u16 = 0x1F3;
/// LBA 15:8 / cylinder low.
pub const IDE_PRIMARY_LBA_MID: u16 = 0x1F4;
/// LBA 23:16 / cylinder high.
pub const IDE_PRIMARY_LBA_HI: u16 = 0x1F5;
/// Drive/head select + LBA 27:24.
pub const IDE_PRIMARY_DRIVE: u16 = 0x1F6;
/// Status (R) / Command (W).
pub const IDE_PRIMARY_STATUS: u16 = 0x1F7;
/// Alternate status (R) / Device control (W).
pub const IDE_PRIMARY_CTRL: u16 = 0x3F6;

/// Secondary ATA data port (16-bit PIO).
pub const IDE_SECONDARY_DATA: u16 = 0x170;
/// Secondary error (R) / Features (W).
pub const IDE_SECONDARY_ERROR: u16 = 0x171;
/// Secondary sector count.
pub const IDE_SECONDARY_SECCOUNT: u16 = 0x172;
/// Secondary LBA 7:0.
pub const IDE_SECONDARY_LBA_LO: u16 = 0x173;
/// Secondary LBA 15:8.
pub const IDE_SECONDARY_LBA_MID: u16 = 0x174;
/// Secondary LBA 23:16.
pub const IDE_SECONDARY_LBA_HI: u16 = 0x175;
/// Secondary drive/head select.
pub const IDE_SECONDARY_DRIVE: u16 = 0x176;
/// Secondary status (R) / Command (W).
pub const IDE_SECONDARY_STATUS: u16 = 0x177;
/// Secondary alternate status (R) / Device control (W).
pub const IDE_SECONDARY_CTRL: u16 = 0x376;

/// Status: busy.
pub const ATA_SR_BSY: u8 = 0x80;
/// Status: drive ready.
pub const ATA_SR_DRDY: u8 = 0x40;
/// Status: drive seek complete (stub always set with DRDY when ready).
pub const ATA_SR_DSC: u8 = 0x10;
/// Status: data request.
pub const ATA_SR_DRQ: u8 = 0x08;
/// Status: error.
pub const ATA_SR_ERR: u8 = 0x01;

/// IDENTIFY DEVICE.
pub const ATA_CMD_IDENTIFY: u8 = 0xEC;
/// PACKET (ATAPI) — rejected with ABRT on ATA master (no packet PIO).
pub const ATA_CMD_PACKET: u8 = 0xA0;
/// IDENTIFY PACKET DEVICE (ATAPI) — rejected with ABRT on ATA master.
pub const ATA_CMD_IDENTIFY_PACKET: u8 = 0xA1;
/// READ SECTORS (with retry) — LBA28 PIO.
pub const ATA_CMD_READ_SECTORS: u8 = 0x20;
/// WRITE SECTORS (with retry) — LBA28 PIO.
pub const ATA_CMD_WRITE_SECTORS: u8 = 0x30;
/// WRITE VERIFY SECTORS (with retry) — LBA28 non-data; verifies writable range.
/// Spec: ATA/ATAPI Command Set — WRITE VERIFY SECTORS (`0x3C`).
pub const ATA_CMD_WRITE_VERIFY_SECTORS: u8 = 0x3C;
/// READ VERIFY SECTORS (with retry) — LBA28 non-data; verifies range without PIO.
/// Spec: ATA/ATAPI Command Set — READ VERIFY SECTORS (`0x40`).
pub const ATA_CMD_READ_VERIFY_SECTORS: u8 = 0x40;
/// FLUSH CACHE — non-data command; completes with success on ATA master.
/// Spec: ATA/ATAPI Command Set — FLUSH CACHE (`0xE7`).
pub const ATA_CMD_FLUSH_CACHE: u8 = 0xE7;
/// EXECUTE DEVICE DIAGNOSTIC — error=0x01 means passed (master).
/// Spec: ATA/ATAPI Command Set — EXECUTE DEVICE DIAGNOSTIC (`0x90`).
pub const ATA_CMD_DIAGNOSTIC: u8 = 0x90;
/// Diagnostic passed code in error register.
pub const ATA_DIAG_PASSED: u8 = 0x01;
/// SET FEATURES — non-data; this stub accepts and succeeds (no feature side effects).
/// Spec: ATA/ATAPI Command Set — SET FEATURES (`0xEF`).
pub const ATA_CMD_SET_FEATURES: u8 = 0xEF;
/// NOP — non-data success on ATA master (no side effects).
/// Spec: ATA/ATAPI Command Set — NOP (`0x00`).
pub const ATA_CMD_NOP: u8 = 0x00;
/// READ MULTIPLE — LBA28 multi-sector PIO (`multiple_count` sectors per DRQ).
/// Spec: ATA/ATAPI Command Set — READ MULTIPLE (`0xC4`).
pub const ATA_CMD_READ_MULTIPLE: u8 = 0xC4;
/// WRITE MULTIPLE — LBA28 multi-sector PIO (`multiple_count` sectors per DRQ).
/// Spec: ATA/ATAPI Command Set — WRITE MULTIPLE (`0xC5`).
pub const ATA_CMD_WRITE_MULTIPLE: u8 = 0xC5;
/// IDLE IMMEDIATE — non-data success.
/// Spec: ATA/ATAPI Command Set — IDLE IMMEDIATE (`0xE1`).
pub const ATA_CMD_IDLE_IMMEDIATE: u8 = 0xE1;
/// IDLE — non-data success (timer value in sector_count ignored by stub).
/// Spec: ATA/ATAPI Command Set — IDLE (`0xE3`).
pub const ATA_CMD_IDLE: u8 = 0xE3;
/// STANDBY IMMEDIATE — non-data success.
/// Spec: ATA/ATAPI Command Set — STANDBY IMMEDIATE (`0xE0`).
pub const ATA_CMD_STANDBY_IMMEDIATE: u8 = 0xE0;
/// CHECK POWER MODE — non-data; sector_count ← `0xFF` (Active/Idle).
/// Spec: ATA/ATAPI Command Set — CHECK POWER MODE (`0xE5`).
pub const ATA_CMD_CHECK_POWER_MODE: u8 = 0xE5;
/// CHECK POWER MODE result: device is Active or Idle.
pub const ATA_POWER_ACTIVE_OR_IDLE: u8 = 0xFF;
/// STANDBY — non-data success (timer in sector_count ignored).
/// Spec: ATA/ATAPI Command Set — STANDBY (`0xE2`).
pub const ATA_CMD_STANDBY: u8 = 0xE2;
/// SLEEP — non-data success.
/// Spec: ATA/ATAPI Command Set — SLEEP (`0xE6`).
pub const ATA_CMD_SLEEP: u8 = 0xE6;
/// RECALIBRATE — non-data success stub.
/// Spec: ATA/ATAPI Command Set — RECALIBRATE (`0x10`).
pub const ATA_CMD_RECALIBRATE: u8 = 0x10;
/// SEEK — non-data success stub.
/// Spec: ATA/ATAPI Command Set — SEEK (`0x70`).
pub const ATA_CMD_SEEK: u8 = 0x70;
/// INITIALIZE DEVICE PARAMETERS — non-data success stub.
/// Spec: ATA/ATAPI Command Set — INITIALIZE DEVICE PARAMETERS (`0x91`).
pub const ATA_CMD_INIT_DEV_PARAMS: u8 = 0x91;
/// FLUSH CACHE EXT — same non-data success as FLUSH CACHE in this stub.
/// Spec: ATA/ATAPI Command Set — FLUSH CACHE EXT (`0xEA`).
pub const ATA_CMD_FLUSH_CACHE_EXT: u8 = 0xEA;
/// READ NATIVE MAX ADDRESS — returns max LBA28 in task-file registers.
/// Spec: ATA/ATAPI Command Set — READ NATIVE MAX ADDRESS (`0xF8`).
pub const ATA_CMD_READ_NATIVE_MAX: u8 = 0xF8;
/// SET MAX ADDRESS — Host Protected Area set; this stub aborts.
/// Spec: ATA/ATAPI Command Set — SET MAX ADDRESS (`0xF9`).
pub const ATA_CMD_SET_MAX_ADDRESS: u8 = 0xF9;
/// SET MULTIPLE MODE — store Sector Count block factor (READ/WRITE MULTIPLE DRQ size).
/// Spec: ATA/ATAPI Command Set — SET MULTIPLE MODE (`0xC6`).
pub const ATA_CMD_SET_MULTIPLE_MODE: u8 = 0xC6;
/// Max sectors per READ/WRITE MULTIPLE interrupt (IDENTIFY word 47 bits 7:0).
/// Spec: ATA IDENTIFY DEVICE — word 47 = `0x8000 | max_sectors_per_drq`.
pub const ATA_MULTIPLE_MAX_SECTORS: u8 = 16;
/// MEDIA LOCK (DOOR LOCK) — non-data success noop (no media tray).
/// Spec: ATA/ATAPI Command Set — MEDIA LOCK (`0xDE`).
pub const ATA_CMD_MEDIA_LOCK: u8 = 0xDE;
/// MEDIA UNLOCK (DOOR UNLOCK) — non-data success noop.
/// Spec: ATA/ATAPI Command Set — MEDIA UNLOCK (`0xDF`).
pub const ATA_CMD_MEDIA_UNLOCK: u8 = 0xDF;
/// SMART — feature-set command; this stub aborts (no SMART support).
/// Spec: ATA/ATAPI Command Set — SMART (`0xB0`).
pub const ATA_CMD_SMART: u8 = 0xB0;
/// READ DMA — bus-master DMA read; this stub aborts (no BM-DMA/PRD engine).
/// Spec: ATA/ATAPI Command Set — READ DMA (`0xC8`).
pub const ATA_CMD_READ_DMA: u8 = 0xC8;
/// WRITE DMA — bus-master DMA write; this stub aborts (no BM-DMA/PRD engine).
/// Spec: ATA/ATAPI Command Set — WRITE DMA (`0xCA`).
pub const ATA_CMD_WRITE_DMA: u8 = 0xCA;
/// SECURITY SET PASSWORD — SECURITY feature-set command; this stub aborts.
/// Spec: ATA/ATAPI Command Set — SECURITY SET PASSWORD (`0xF1`).
pub const ATA_CMD_SECURITY_SET_PASSWORD: u8 = 0xF1;
/// SECURITY UNLOCK — SECURITY feature-set command; this stub aborts.
/// Spec: ATA/ATAPI Command Set — SECURITY UNLOCK (`0xF2`).
pub const ATA_CMD_SECURITY_UNLOCK: u8 = 0xF2;
/// SECURITY ERASE PREPARE — SECURITY feature-set command; this stub aborts.
/// Spec: ATA/ATAPI Command Set — SECURITY ERASE PREPARE (`0xF3`).
pub const ATA_CMD_SECURITY_ERASE_PREPARE: u8 = 0xF3;
/// SECURITY ERASE UNIT — SECURITY feature-set command; this stub aborts.
/// Spec: ATA/ATAPI Command Set — SECURITY ERASE UNIT (`0xF4`).
pub const ATA_CMD_SECURITY_ERASE_UNIT: u8 = 0xF4;
/// SECURITY FREEZE LOCK — SECURITY feature-set command; this stub aborts.
/// Spec: ATA/ATAPI Command Set — SECURITY FREEZE LOCK (`0xF5`).
pub const ATA_CMD_SECURITY_FREEZE_LOCK: u8 = 0xF5;
/// SECURITY DISABLE PASSWORD — SECURITY feature-set command; this stub aborts.
/// Spec: ATA/ATAPI Command Set — SECURITY DISABLE PASSWORD (`0xF6`).
pub const ATA_CMD_SECURITY_DISABLE_PASSWORD: u8 = 0xF6;
/// DOWNLOAD MICROCODE — vendor microcode transfer; this stub aborts.
/// Spec: ATA/ATAPI Command Set — DOWNLOAD MICROCODE (`0x92`).
pub const ATA_CMD_DOWNLOAD_MICROCODE: u8 = 0x92;
/// READ LOG EXT — General Purpose Logging read; this stub aborts.
/// Spec: ATA/ATAPI Command Set — READ LOG EXT (`0x2F`).
pub const ATA_CMD_READ_LOG_EXT: u8 = 0x2F;
/// WRITE LOG EXT — General Purpose Logging write; this stub aborts.
/// Spec: ATA/ATAPI Command Set — WRITE LOG EXT (`0x3F`).
pub const ATA_CMD_WRITE_LOG_EXT: u8 = 0x3F;
/// DATA SET MANAGEMENT — TRIM / DSM range list; this stub aborts.
/// Spec: ATA/ATAPI Command Set — DATA SET MANAGEMENT (`0x06`).
pub const ATA_CMD_DATA_SET_MANAGEMENT: u8 = 0x06;
/// TRUSTED RECEIVE — Trusted Computing / Security Protocol receive; this stub aborts.
/// Spec: ATA/ATAPI Command Set — TRUSTED RECEIVE (`0x5C`).
pub const ATA_CMD_TRUSTED_RECEIVE: u8 = 0x5C;
/// TRUSTED SEND — Trusted Computing / Security Protocol send; this stub aborts.
/// Spec: ATA/ATAPI Command Set — TRUSTED SEND (`0x5E`).
pub const ATA_CMD_TRUSTED_SEND: u8 = 0x5E;
/// READ BUFFER — 512-byte sector buffer PIO data-in.
/// Spec: ATA/ATAPI Command Set — READ BUFFER (`0xE4`).
pub const ATA_CMD_READ_BUFFER: u8 = 0xE4;
/// WRITE BUFFER — 512-byte sector buffer PIO data-out.
/// Spec: ATA/ATAPI Command Set — WRITE BUFFER (`0xE8`).
pub const ATA_CMD_WRITE_BUFFER: u8 = 0xE8;

/// READ SECTOR(S) EXT — 48-bit Address feature set. Spec: ATA/ATAPI-6 §8.35.1.
pub const ATA_CMD_READ_SECTORS_EXT: u8 = 0x24;
/// WRITE SECTOR(S) EXT — 48-bit Address feature set. Spec: ATA/ATAPI-6 §8.63.1.
pub const ATA_CMD_WRITE_SECTORS_EXT: u8 = 0x34;

/// Error register: aborted command.
pub const ATA_ER_ABRT: u8 = 0x04;
/// Error register bit4: IDNF (requested address not found / out of range).
///
/// Spec: ATA/ATAPI-6 §8.35.6 / §8.63.6 — "IDNF shall be set to one if an
/// address outside of the range of user-accessible addresses is requested if
/// command aborted is not returned."
pub const ATA_ER_IDNF: u8 = 0x10;

/// Device control: software reset.
pub const ATA_DC_SRST: u8 = 0x04;
/// Device control: nIEN (1 = IRQ disabled / INTRQ not driven).
pub const ATA_DC_NIEN: u8 = 0x02;
/// Device control bit7: HOB (high order byte).
///
/// Spec: ATA/ATAPI-6 §7.8.6 / §6.20 — when set, reads of the Sector Count,
/// LBA Low, LBA Mid and LBA High registers return the "previous content" half
/// of their two-byte deep FIFO.
pub const ATA_DC_HOB: u8 = 0x80;

/// Largest LBA28 user-addressable sector count (IDENTIFY words 61:60 cap).
///
/// Spec: ATA/ATAPI-6 §6.2.1 — words (61:60) "shall be greater than or equal to
/// one and less than or equal to 268,435,455".
pub const ATA_LBA28_MAX_SECTORS: u64 = 268_435_455;
/// Largest LBA48 user-addressable sector count. Spec: ATA/ATAPI-6 §6.2.1 —
/// words (103:100) "shall not exceed 0000FFFFFFFFFFFFh".
pub const ATA_LBA48_MAX_SECTORS: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Drive/head: LBA mode bit.
pub const ATA_DRIVE_LBA: u8 = 0x40;
/// Drive/head bit4: DEV — device 1 ("slave") select; Device 0 = 0.
///
/// Spec: ATA/ATAPI-6 §7.7 — "When the DEV bit is cleared to zero, Device 0 is
/// selected. When the DEV bit is set to one, Device 1 is selected."
pub const ATA_DRIVE_SLAVE: u8 = 0x10;

/// Non-PACKET device signature Sector Count. Spec: ATA/ATAPI-6 §9.12.
pub const ATA_SIGNATURE_SECTOR_COUNT: u8 = 0x01;
/// Non-PACKET device signature LBA Low. Spec: ATA/ATAPI-6 §9.12.
pub const ATA_SIGNATURE_LBA_LOW: u8 = 0x01;
/// Non-PACKET device signature LBA Mid (`0x14` on ATAPI). Spec: ATA/ATAPI-6 §9.12.
pub const ATA_SIGNATURE_LBA_MID: u8 = 0x00;
/// Non-PACKET device signature LBA High (`0xEB` on ATAPI). Spec: ATA/ATAPI-6 §9.12.
pub const ATA_SIGNATURE_LBA_HIGH: u8 = 0x00;

/// PACKET (ATAPI) device signature Sector Count / Interrupt Reason.
///
/// Spec: ATA/ATAPI-6 §9.12 — "If the device implements the PACKET command
/// feature set, the signature shall be: Sector Count 01h, LBA Low 01h,
/// LBA Mid 14h, LBA High EBh".
pub const ATAPI_SIGNATURE_SECTOR_COUNT: u8 = 0x01;
/// PACKET device signature LBA Low. Spec: ATA/ATAPI-6 §9.12.
pub const ATAPI_SIGNATURE_LBA_LOW: u8 = 0x01;
/// PACKET device signature LBA Mid / Byte Count Low. Spec: ATA/ATAPI-6 §9.12.
pub const ATAPI_SIGNATURE_LBA_MID: u8 = 0x14;
/// PACKET device signature LBA High / Byte Count High. Spec: ATA/ATAPI-6 §9.12.
pub const ATAPI_SIGNATURE_LBA_HIGH: u8 = 0xEB;

/// IDENTIFY PACKET DEVICE word 0 bit15 — the device implements the PACKET
/// Command feature set (bits 15:14 = `10b`).
///
/// Spec: ATA/ATAPI-6 §8.16.9 — "Bit 15 shall be set to one and bit 14 shall be
/// cleared to zero to indicate the device implements the PACKET Command
/// feature set."
const IDENTIFY_PACKET_ATAPI_DEVICE: u16 = 0x8000;
/// IDENTIFY PACKET DEVICE word 0 bits (12:8) value `1Fh` — "Unknown or no
/// device type".
///
/// Spec: ATA/ATAPI-6 §8.16.9 — bits (12:8) name the command packet set
/// "following the peripheral device type value as defined in SCSI Primary
/// Commands". This device implements **no** command packet set, so it reports
/// the defined "unknown or no device type" value rather than claiming to be a
/// CD-ROM (`05h`) it cannot act as.
const IDENTIFY_PACKET_SET_UNKNOWN: u16 = 0x1F;
/// IDENTIFY PACKET DEVICE word 49 bit9 — "Shall be set to one".
///
/// Spec: ATA/ATAPI-6 §8.16.18.
const IDENTIFY_PACKET_WORD49_MANDATORY: u16 = 1 << 9;
/// IDENTIFY PACKET DEVICE words 50 / 83 / 84 / 87 bit14 — "Shall be set to one"
/// (with bit15 cleared) so the word counts as valid.
///
/// Spec: ATA/ATAPI-6 §8.16.19 and Table 29 words 83, 84, 87.
const IDENTIFY_PACKET_WORD_VALID: u16 = 0x4000;
/// IDENTIFY PACKET DEVICE words 82 / 85 bit4 — "Shall be set to one indicating
/// the PACKET Command feature set is supported".
///
/// Spec: ATA/ATAPI-6 Table 29 words 82 and 85.
const IDENTIFY_PACKET_FEATURE_SET: u16 = 1 << 4;
/// IDENTIFY PACKET DEVICE words 82 / 85 bit9 — DEVICE RESET is supported.
///
/// Spec: ATA/ATAPI-6 Table 29 — word 82 bit 9 "The DEVICE RESET command is
/// supported", word 85 bit 9 reports it enabled. Round 3 left this clear
/// because `08h` was unimplemented; [`IdePrimary::exec_device_reset`] now
/// implements it, so the claim is truthful.
const IDENTIFY_PACKET_DEVICE_RESET: u16 = 1 << 9;

/// DEVICE RESET (`08h`).
///
/// Spec: ATA/ATAPI-6 §8.7 — mandatory for devices implementing the PACKET
/// Command feature set, "use prohibited" for devices that do not.
pub const ATA_CMD_DEVICE_RESET: u8 = 0x08;

/// Command packet length this device accepts, in bytes.
///
/// Spec: ATA/ATAPI-6 §8.16.9 — IDENTIFY PACKET DEVICE word 0 bits (1:0) select
/// the command packet size; `00b` means a 12-byte packet. This device reports
/// `00b`, so it accepts exactly 12 bytes and nothing else.
pub const ATAPI_PACKET_BYTES: usize = 12;

/// Interrupt Reason (Sector Count) bit0 — C/D.
///
/// Spec: ATA/ATAPI-6 §7.13 "Interrupt Reason register" — C/D set means the
/// bytes transferred are a command packet (or, at completion, that the command
/// is complete); cleared means user data.
pub const ATAPI_IR_CD: u8 = 0x01;
/// Interrupt Reason (Sector Count) bit1 — I/O.
///
/// Spec: ATA/ATAPI-6 §7.13 — set means the transfer is to the host
/// (device → host), cleared means from the host.
pub const ATAPI_IR_IO: u8 = 0x02;
/// Interrupt Reason (Sector Count) bit2 — REL (release).
///
/// Spec: ATA/ATAPI-6 §7.13 — set when the device has released the bus before
/// completing a command. This model never overlaps a command, so it is never
/// set.
pub const ATAPI_IR_REL: u8 = 0x04;
/// The three Interrupt Reason bits occupy Sector Count bits 2:0.
///
/// Spec: ATA/ATAPI-6 §7.13. REL is never set by this model, so the assertion
/// also documents that the value is defined rather than unused.
const _: () = assert!(ATAPI_IR_CD | ATAPI_IR_IO | ATAPI_IR_REL == 0x07);

/// PACKET Features register bit0 — DMA.
///
/// Spec: ATA/ATAPI-6 §8.21.4 — "DMA: If set to one, the data transfer ... shall
/// be via DMA." There is no DMA engine here, so the command is aborted.
pub const ATAPI_FEATURE_DMA: u8 = 0x01;
/// PACKET Features register bit1 — OVL (overlap).
///
/// Spec: ATA/ATAPI-6 §8.21.4 — "OVL: If set to one, the command ... may be
/// overlapped." Overlap is not implemented, so the command is aborted.
pub const ATAPI_FEATURE_OVL: u8 = 0x02;

/// Packet command `TEST UNIT READY`.
///
/// Spec: SFF-8020i §10.8.24 / MMC — checks whether the logical unit is ready.
pub const ATAPI_CMD_TEST_UNIT_READY: u8 = 0x00;
/// Packet command `REQUEST SENSE`.
///
/// Spec: SFF-8020i §10.8.16 / MMC — returns the sense data describing the
/// CHECK CONDITION that preceded it.
pub const ATAPI_CMD_REQUEST_SENSE: u8 = 0x03;
/// Packet command `INQUIRY`.
///
/// Spec: SFF-8020i §10.8.4 / MMC — returns the standard INQUIRY data.
pub const ATAPI_CMD_INQUIRY: u8 = 0x12;
/// Packet command `READ CAPACITY` (10).
///
/// Spec: SFF-8020i / MMC — last logical block address and block length.
pub const ATAPI_CMD_READ_CAPACITY: u8 = 0x25;
/// Packet command `READ (10)`.
///
/// Spec: SFF-8020i / MMC — read logical blocks from CD-ROM medium.
pub const ATAPI_CMD_READ_10: u8 = 0x28;
/// Packet command `MODE SENSE (6)`.
///
/// Spec: MMC / SPC — 4-byte mode parameter header + page(s). CD-ROM capable
/// devices only; see `docs/atapi-r6-mode-sense.md`.
pub const ATAPI_CMD_MODE_SENSE_6: u8 = 0x1A;
/// Packet command `MODE SENSE (10)`.
///
/// Spec: SFF-8020i §9.8.4 — ATAPI CD-ROM MODE SENSE with 8-byte header.
pub const ATAPI_CMD_MODE_SENSE_10: u8 = 0x5A;
/// Packet command `START STOP UNIT`.
///
/// Spec: SFF-8020i §9.8.26 — LoEj/Start control medium load/eject readiness.
pub const ATAPI_CMD_START_STOP_UNIT: u8 = 0x1B;
/// Packet command `READ TOC/PMA/ATIP`.
///
/// Spec: SFF-8020i §9.8.20 — table of contents for the loaded data CD.
pub const ATAPI_CMD_READ_TOC: u8 = 0x43;

/// MODE SENSE page code `01h` — Read Error Recovery Parameters.
///
/// Spec: SFF-8020i §9.8.5.3 / Table 52.
pub const ATAPI_MODE_PAGE_ERROR_RECOVERY: u8 = 0x01;
/// Page length for Read Error Recovery (bytes after the length byte).
pub const ATAPI_MODE_PAGE_ERROR_RECOVERY_LEN: u8 = 0x06;
/// Medium type `70h` — door closed, no disc. Spec: SFF-8020i Table 46.
pub const ATAPI_MEDIUM_TYPE_NO_DISC: u8 = 0x70;
/// Medium type `01h` — 120 mm CD-ROM data only. Spec: SFF-8020i Table 46.
pub const ATAPI_MEDIUM_TYPE_120MM_DATA: u8 = 0x01;
/// START STOP UNIT byte 4 bit0 — Start. Spec: SFF-8020i §9.8.26.
pub const ATAPI_START_STOP_START: u8 = 0x01;
/// START STOP UNIT byte 4 bit1 — LoEj (load/eject). Spec: SFF-8020i §9.8.26.
pub const ATAPI_START_STOP_LOEJ: u8 = 0x02;
/// READ TOC ADR/Control for a Mode-1 data track (ADR=1, Control=4).
///
/// Spec: SFF-8020i Table 118 — `04h` = copy prohibited, digital data; ADR in
/// the high nibble of the combined ADR/Control byte is typically `1h`.
pub const ATAPI_TOC_ADR_CONTROL_DATA: u8 = 0x14;
/// READ TOC track number for the lead-out area.
pub const ATAPI_TOC_TRACK_LEAD_OUT: u8 = 0xAA;

/// Sense key `0h` NO SENSE. Spec: SFF-8020i Table "Sense Key Definitions".
pub const ATAPI_SENSE_NO_SENSE: u8 = 0x00;
/// Sense key `2h` NOT READY. Spec: SFF-8020i — empty CD-ROM medium.
pub const ATAPI_SENSE_NOT_READY: u8 = 0x02;
/// Sense key `5h` ILLEGAL REQUEST. Spec: SFF-8020i "Sense Key Definitions" —
/// "an illegal parameter in the command packet".
pub const ATAPI_SENSE_ILLEGAL_REQUEST: u8 = 0x05;
/// Additional sense code `20h` INVALID COMMAND OPERATION CODE.
///
/// Spec: SFF-8020i "ASC and ASCQ Definitions".
pub const ATAPI_ASC_INVALID_COMMAND_OPERATION_CODE: u8 = 0x20;
/// Additional sense code `21h` LOGICAL BLOCK ADDRESS OUT OF RANGE.
pub const ATAPI_ASC_LBA_OUT_OF_RANGE: u8 = 0x21;
/// Additional sense code `24h` INVALID FIELD IN COMMAND PACKET.
///
/// Spec: SFF-8020i "ASC and ASCQ Definitions".
pub const ATAPI_ASC_INVALID_FIELD_IN_CDB: u8 = 0x24;
/// Additional sense code `3Ah` MEDIUM NOT PRESENT.
pub const ATAPI_ASC_MEDIUM_NOT_PRESENT: u8 = 0x3A;

/// Fixed-format sense data length this device returns, in bytes.
///
/// Spec: SFF-8020i §10.8.16 — the request sense data is 18 bytes: an 8-byte
/// header plus the Additional Sense Length of 10 reported in byte 7.
pub const ATAPI_SENSE_DATA_BYTES: usize = 18;
/// Sense data byte 0 — response code `70h`, "current error", VALID cleared.
///
/// Spec: SFF-8020i §10.8.16 request sense data format. VALID (bit 7) stays
/// clear because there is no valid information field to report.
pub const ATAPI_SENSE_RESPONSE_CODE_CURRENT: u8 = 0x70;

/// Standard INQUIRY data length this device returns, in bytes.
///
/// Spec: SFF-8020i §10.8.4 — 36 bytes with an Additional Length of 31.
pub const ATAPI_INQUIRY_DATA_BYTES: usize = 36;
/// INQUIRY byte 1 bit0 — EVPD (enable vital product data).
///
/// Spec: SFF-8020i §10.8.4. No VPD pages exist here, so a set EVPD is an
/// invalid field.
pub const ATAPI_INQUIRY_EVPD: u8 = 0x01;
/// INQUIRY byte 3 — Response Data Format.
///
/// Spec: SFF-8020i §10.8.4 — the standard INQUIRY data layout this device
/// returns is the one the standard defines, reported as `02h`.
pub const ATAPI_INQUIRY_RESPONSE_DATA_FORMAT: u8 = 0x02;
/// Peripheral device type `1Fh` — "unknown or no device type".
///
/// Spec: SCSI Primary Commands peripheral device type table, referenced by
/// ATA/ATAPI-6 §8.16.9 for IDENTIFY PACKET DEVICE word 0 bits (12:8). This
/// minimal PACKET device (no CD-ROM command set) reports this rather than
/// claiming `05h` CD-ROM.
pub const ATAPI_PERIPHERAL_DEVICE_TYPE_UNKNOWN: u8 = 0x1F;
/// Peripheral device type `05h` — CD-ROM.
///
/// Spec: SCSI Primary Commands / ATA/ATAPI-6 §8.16.9. Used only when the
/// device is CD-ROM capable (`docs/atapi-r5-cdrom-medium.md`).
pub const ATAPI_PERIPHERAL_DEVICE_TYPE_CDROM: u8 = 0x05;
/// INQUIRY byte 1 bit7 — RMB (removable medium bit).
pub const ATAPI_INQUIRY_RMB: u8 = 0x80;
/// CD-ROM logical block length (Mode 1 user data). Spec: SFF-8020i / MMC.
pub const ATAPI_CDROM_BLOCK_BYTES: usize = 2048;
/// READ CAPACITY parameter data length in bytes.
pub const ATAPI_READ_CAPACITY_DATA_BYTES: usize = 8;

/// Where a PACKET command is in the ATA/ATAPI-6 §9.8 protocol.
///
/// Spec: ATA/ATAPI-6 §9.8 "PACKET command protocol" — a PACKET command runs
/// through a command-packet transfer, an optional data transfer, and command
/// completion. This model has no overlap or DMA, so there is no released or
/// bus-master state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PacketPhase {
    /// No PACKET command in progress.
    Idle,
    /// Awaiting the [`ATAPI_PACKET_BYTES`]-byte command packet from the host.
    Command,
    /// Presenting packet data to the host under DRQ.
    DataIn,
}

const SECTOR_SIZE: usize = 512;
const IDENTIFY_WORDS: usize = 256;
/// IDENTIFY words 83/86 bit10 — 48-bit Address feature set supported.
///
/// Spec: ATA/ATAPI-6 Table 27 — "If bit 10 of word 83 is set to one, the 48-bit
/// Address feature set is supported"; word 86 bit10 mirrors it.
const IDENTIFY_LBA48_SUPPORTED: u16 = 1 << 10;

/// Primary IDE channel (master drive stub).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdePrimary {
    /// When false, status reads as `0` (no device) until a drive is attached.
    pub present: bool,
    /// Backing image bytes (multiple of 512 preferred; short reads zero-pad).
    pub image: Vec<u8>,
    /// True when the attached device implements the PACKET Command feature set.
    ///
    /// Spec: ATA/ATAPI-6 §6.8 — such a device "exhibits responses different
    /// from those exhibited by devices not implementing this feature set":
    /// a different reset signature (§9.12), IDENTIFY DEVICE aborted, and
    /// IDENTIFY PACKET DEVICE answered. A minimal packet device stays type
    /// `1Fh`; see [`Self::atapi_cdrom`] for the CD-ROM capable path.
    /// See `docs/atapi-r3-identify-and-signature.md`.
    packet_device: bool,
    /// True when Device 0 is CD-ROM capable (peripheral type `05h`).
    ///
    /// Spec: ATA/ATAPI-6 §8.16.9 / SFF-8020i — only set when READ CAPACITY and
    /// READ (10) exist. See `docs/atapi-r5-cdrom-medium.md`.
    atapi_cdrom: bool,
    error: u8,
    features: u8,
    sector_count: u8,
    lba_lo: u8,
    lba_mid: u8,
    lba_hi: u8,
    /// "Previous content" halves of the two-byte deep Command Block FIFOs.
    ///
    /// Spec: ATA/ATAPI-6 §6.20 / Table 11 — the Features, Sector Count, LBA
    /// Low, LBA Mid and LBA High registers are each a two-byte deep FIFO; a
    /// write moves the old "most recently written" value here. The host reads
    /// them back with Device Control HOB set.
    features_prev: u8,
    sector_count_prev: u8,
    lba_lo_prev: u8,
    lba_mid_prev: u8,
    lba_hi_prev: u8,
    drive_head: u8,
    status: u8,
    dev_ctrl: u8,
    /// Latched INTRQ request (gated by nIEN on [`Self::irq_line`]).
    irq_pending: bool,
    /// Persistent ATA sector buffer (READ/WRITE BUFFER / last READ|WRITE SECTORS).
    ///
    /// Spec: ATA/ATAPI Command Set — READ/WRITE BUFFER transfer this 512-byte buffer.
    sector_buffer: [u8; SECTOR_SIZE],
    /// Current PIO transfer payload (512 bytes).
    pio: [u8; SECTOR_SIZE],
    pio_off: usize,
    /// Sectors still to present/accept after the current PIO block (incl. current).
    sectors_left: u32,
    /// Next LBA to load (READ) or LBA of current PIO block (WRITE).
    ///
    /// 64-bit so the same PIO engine serves LBA28 and the 48-bit Address
    /// feature set (ATA/ATAPI-6 §6.20).
    next_lba: u64,
    /// True while a 48-bit Address feature set transfer is in progress.
    ///
    /// Spec: ATA/ATAPI-6 §6.20 — the EXT commands use a 16-bit Sector Count
    /// spread across the two-deep FIFO, so the running count is mirrored into
    /// both halves instead of the single LBA28 byte.
    lba48_xfer: bool,
    /// True while host must drain/fill the data port under DRQ.
    transferring: bool,
    /// True = host→device WRITE PIO; false = device→host READ/IDENTIFY PIO.
    pio_in: bool,
    /// True = active WRITE BUFFER PIO (commit to `sector_buffer`, not media).
    sector_buffer_write: bool,
    /// Block factor from SET MULTIPLE MODE (`0` = not configured).
    ///
    /// Spec: ATA SET MULTIPLE MODE — Sector Count selects sectors per
    /// READ/WRITE MULTIPLE DRQ; IDENTIFY word 59 reports the current setting.
    pub multiple_count: u8,
    /// True while READ/WRITE MULTIPLE multi-sector DRQ transfer is active.
    multiple_xfer: bool,
    /// Sectors remaining in the current READ/WRITE MULTIPLE DRQ block
    /// (including the sector currently under DRQ).
    block_left: u32,
    /// Where a PACKET command is in the ATA/ATAPI-6 §9.8 protocol.
    packet_phase: PacketPhase,
    /// Command packet bytes received so far this PACKET command.
    packet_cmd: [u8; ATAPI_PACKET_BYTES],
    /// Bytes of [`Self::packet_cmd`] the host has written.
    packet_off: usize,
    /// Byte Count Limit latched from Cylinder Low/High when PACKET was written.
    ///
    /// Spec: ATA/ATAPI-6 §8.21.4 — "the Byte Count Limit ... is the maximum
    /// number of bytes that may be transferred in a single DRQ data block".
    packet_byte_count_limit: u16,
    /// Packet data still to present to the host (device → host only).
    packet_data: Vec<u8>,
    /// Read cursor into [`Self::packet_data`].
    packet_data_off: usize,
    /// End offset of the DRQ block currently under transfer.
    packet_block_end: usize,
    /// Sense key reported by the next REQUEST SENSE.
    ///
    /// Spec: SFF-8020i §10.8.16 — sense data describes the CHECK CONDITION
    /// that preceded it and is cleared once reported.
    sense_key: u8,
    /// Additional sense code reported by the next REQUEST SENSE.
    sense_asc: u8,
    /// Additional sense code qualifier reported by the next REQUEST SENSE.
    sense_ascq: u8,
}

impl Default for IdePrimary {
    fn default() -> Self {
        Self::new()
    }
}

impl IdePrimary {
    /// Empty channel (no drive) — status reads `0`.
    pub fn new() -> Self {
        Self {
            present: false,
            image: Vec::new(),
            packet_device: false,
            atapi_cdrom: false,
            error: 0,
            features: 0,
            sector_count: 0,
            lba_lo: 0,
            lba_mid: 0,
            lba_hi: 0,
            features_prev: 0,
            sector_count_prev: 0,
            lba_lo_prev: 0,
            lba_mid_prev: 0,
            lba_hi_prev: 0,
            drive_head: 0xA0,
            status: 0,
            dev_ctrl: ATA_DC_NIEN,
            irq_pending: false,
            sector_buffer: [0; SECTOR_SIZE],
            pio: [0; SECTOR_SIZE],
            pio_off: 0,
            sectors_left: 0,
            next_lba: 0,
            lba48_xfer: false,
            transferring: false,
            pio_in: false,
            sector_buffer_write: false,
            multiple_count: 0,
            multiple_xfer: false,
            block_left: 0,
            packet_phase: PacketPhase::Idle,
            packet_cmd: [0; ATAPI_PACKET_BYTES],
            packet_off: 0,
            packet_byte_count_limit: 0,
            packet_data: Vec::new(),
            packet_data_off: 0,
            packet_block_end: 0,
            sense_key: ATAPI_SENSE_NO_SENSE,
            sense_asc: 0,
            sense_ascq: 0,
        }
    }

    /// Attach a master disk image and mark the drive present / ready.
    ///
    /// Spec: ATA — after power-on / reset, DRDY set when ready to accept commands.
    pub fn with_image(image: Vec<u8>) -> Self {
        let mut ide = Self::new();
        ide.attach_image(image);
        ide
    }

    pub fn attach_image(&mut self, image: Vec<u8>) {
        self.image = image;
        self.present = true;
        // An attached disk image is an ATA device, not a packet device.
        self.packet_device = false;
        self.atapi_cdrom = false;
        self.reset_ready();
    }

    /// Configure Device 0 as a minimal PACKET (ATAPI) device with no media.
    ///
    /// The device becomes *detectable* as ATAPI: it reports the ATA/ATAPI-6
    /// §9.12 PACKET signature after every reset and EXECUTE DEVICE DIAGNOSTIC,
    /// aborts IDENTIFY DEVICE with that signature in place, and answers
    /// IDENTIFY PACKET DEVICE. Peripheral type stays `1Fh` — it is **not** a
    /// CD-ROM (`docs/atapi-r3-identify-and-signature.md`). For a CD-ROM
    /// capable device see [`Self::attach_atapi_cdrom`].
    pub fn attach_atapi_device(&mut self) {
        self.image = Vec::new();
        self.present = true;
        self.packet_device = true;
        self.atapi_cdrom = false;
        self.reset_ready();
    }

    /// A channel whose Device 0 is a PACKET (ATAPI) device with no media.
    pub fn with_atapi_device() -> Self {
        let mut ide = Self::new();
        ide.attach_atapi_device();
        ide
    }

    /// Configure Device 0 as a CD-ROM capable PACKET device with no medium.
    ///
    /// Spec: ATA/ATAPI-6 §8.16.9 — word 0 bits (12:8) = `05h` CD-ROM and RMB
    /// set because READ CAPACITY / READ (10) exist. The tray is empty until
    /// [`Self::load_atapi_medium`]. See `docs/atapi-r5-cdrom-medium.md`.
    pub fn attach_atapi_cdrom(&mut self) {
        self.image = Vec::new();
        self.present = true;
        self.packet_device = true;
        self.atapi_cdrom = true;
        self.reset_ready();
    }

    /// CD-ROM capable PACKET device with no medium loaded.
    pub fn with_atapi_cdrom() -> Self {
        let mut ide = Self::new();
        ide.attach_atapi_cdrom();
        ide
    }

    /// Configure Device 0 as a CD-ROM and load a raw 2048-byte-sector image.
    ///
    /// The image is truncated down to a whole number of
    /// [`ATAPI_CDROM_BLOCK_BYTES`] blocks. ISO 9660 is not parsed.
    pub fn attach_atapi_cdrom_image(&mut self, image: Vec<u8>) {
        self.attach_atapi_cdrom();
        self.load_atapi_medium(image);
    }

    /// CD-ROM capable PACKET device with a medium image attached.
    pub fn with_atapi_cdrom_image(image: Vec<u8>) -> Self {
        let mut ide = Self::new();
        ide.attach_atapi_cdrom_image(image);
        ide
    }

    /// Load (or replace) the CD-ROM medium image.
    ///
    /// Returns `false` without changing state when the device is not CD-ROM
    /// capable. Truncates to a whole number of 2048-byte blocks.
    pub fn load_atapi_medium(&mut self, image: Vec<u8>) -> bool {
        if !self.atapi_cdrom {
            return false;
        }
        let blocks = image.len() / ATAPI_CDROM_BLOCK_BYTES;
        self.image = image[..blocks * ATAPI_CDROM_BLOCK_BYTES].to_vec();
        true
    }

    /// Unload the CD-ROM medium (empty tray). No-op if not CD-ROM capable.
    pub fn unload_atapi_medium(&mut self) {
        if self.atapi_cdrom {
            self.image.clear();
        }
    }

    /// True when Device 0 implements the PACKET Command feature set.
    pub fn is_packet_device(&self) -> bool {
        self.packet_device
    }

    /// True when Device 0 is CD-ROM capable (type `05h`).
    pub fn is_atapi_cdrom(&self) -> bool {
        self.packet_device && self.atapi_cdrom
    }

    /// True when a CD-ROM medium image is loaded (at least one 2048-byte block).
    pub fn atapi_medium_loaded(&self) -> bool {
        self.is_atapi_cdrom() && !self.image.is_empty()
    }

    /// Number of 2048-byte logical blocks on the loaded CD-ROM medium.
    pub fn atapi_cdrom_blocks(&self) -> u64 {
        if !self.is_atapi_cdrom() {
            return 0;
        }
        (self.image.len() / ATAPI_CDROM_BLOCK_BYTES) as u64
    }

    pub fn reset(&mut self) {
        // Preserve backing image / presence / device type across Machine::reset.
        let image = std::mem::take(&mut self.image);
        let present = self.present;
        let packet_device = self.packet_device;
        let atapi_cdrom = self.atapi_cdrom;
        *self = Self::new();
        self.image = image;
        self.present = present;
        self.packet_device = packet_device;
        self.atapi_cdrom = atapi_cdrom;
        if self.present {
            self.reset_ready();
        }
    }

    /// Status a device presents at command completion.
    ///
    /// Spec: ATA/ATAPI-6 §8.16.5 — IDENTIFY PACKET DEVICE completes with DRDY
    /// set to one. Bit 4 is SERV rather than DSC on a PACKET device, so this
    /// model never sets it there: nothing here ever has a service request.
    fn ready_status(&self) -> u8 {
        if self.packet_device {
            ATA_SR_DRDY
        } else {
            ATA_SR_DRDY | ATA_SR_DSC
        }
    }

    /// Status a device presents after a reset or EXECUTE DEVICE DIAGNOSTIC.
    ///
    /// Spec: ATA/ATAPI-6 §9.10 / §9.11 / Figure 17 — "If the device implements
    /// the PACKET command feature set, the device shall clear bits 6, 5, 4, 3,
    /// 2, and 0 in the Status register to zero", so a PACKET device reads
    /// `00h` after a reset. A host distinguishes it from an empty channel by
    /// the §9.12 signature, not by Status.
    fn reset_status(&self) -> u8 {
        if self.packet_device {
            0
        } else {
            ATA_SR_DRDY | ATA_SR_DSC
        }
    }

    fn reset_ready(&mut self) {
        self.error = 0;
        self.features = 0;
        self.write_device_signature();
        // Spec: ATA/ATAPI-6 §6.20 — the "previous content" halves have no
        // defined value after reset; clear them so no stale HOB byte leaks
        // into the next 48-bit command.
        self.features_prev = 0;
        self.sector_count_prev = 0;
        self.lba_lo_prev = 0;
        self.lba_mid_prev = 0;
        self.lba_hi_prev = 0;
        self.drive_head = 0xA0;
        self.dev_ctrl = ATA_DC_NIEN;
        self.irq_pending = false;
        self.sector_buffer = [0; SECTOR_SIZE];
        self.pio = [0; SECTOR_SIZE];
        self.pio_off = 0;
        self.sectors_left = 0;
        self.next_lba = 0;
        self.lba48_xfer = false;
        self.transferring = false;
        self.pio_in = false;
        self.sector_buffer_write = false;
        self.multiple_count = 0;
        self.multiple_xfer = false;
        self.block_left = 0;
        // Spec: ATA/ATAPI-6 §9.10 / §9.11 — a reset ends any PACKET command in
        // progress, and §8.7.5 gives the same outcome for DEVICE RESET.
        self.clear_packet_state();
        self.clear_sense();
        self.status = if self.present { self.reset_status() } else { 0 };
    }

    /// Drop any PACKET command in progress.
    fn clear_packet_state(&mut self) {
        self.packet_phase = PacketPhase::Idle;
        self.packet_cmd = [0; ATAPI_PACKET_BYTES];
        self.packet_off = 0;
        self.packet_byte_count_limit = 0;
        self.packet_data.clear();
        self.packet_data_off = 0;
        self.packet_block_end = 0;
    }

    /// Reset the sense data to NO SENSE with no additional sense.
    ///
    /// Spec: SFF-8020i §10.8.16 — sense data is valid until it is reported, so
    /// clearing it means "nothing to report".
    fn clear_sense(&mut self) {
        self.sense_key = ATAPI_SENSE_NO_SENSE;
        self.sense_asc = 0;
        self.sense_ascq = 0;
    }

    /// Sense key, additional sense code and qualifier a REQUEST SENSE would
    /// report right now.
    ///
    /// Spec: SFF-8020i §10.8.16. Exposed so a host (or a test) can observe the
    /// CHECK CONDITION reason without running the packet command.
    pub fn atapi_sense(&self) -> (u8, u8, u8) {
        (self.sense_key, self.sense_asc, self.sense_ascq)
    }

    /// True if this device owns the I/O port.
    pub fn owns_port(port: u16) -> bool {
        matches!(port, 0x1F0..=0x1F7 | IDE_PRIMARY_CTRL)
    }

    /// ISA IRQ14 line level (INTRQ ∧ ¬nIEN ∧ device selected).
    ///
    /// Spec: ATA/ATAPI-6 §5.2.9 INTRQ — "When the nIEN bit is set to one or the
    /// device is not selected, the INTRQ signal shall be released." Selecting
    /// Device 1 therefore releases the line without clearing Device 0 interrupt
    /// pending; reselecting Device 0 asserts it again.
    /// Spec: OSDev ATA PIO — primary → IRQ14.
    pub fn irq_line(&self) -> bool {
        self.irq_pending && (self.dev_ctrl & ATA_DC_NIEN == 0) && !self.is_slave_selected()
    }

    fn raise_irq(&mut self) {
        // Spec: ATA — INTRQ asserted when drive needs attention; nIEN gates pin.
        self.irq_pending = true;
    }

    fn clear_irq(&mut self) {
        self.irq_pending = false;
    }

    /// True when the Device register DEV bit selects Device 1.
    ///
    /// This channel models Device 0 only, so Device 1 is always absent and
    /// ATA/ATAPI-6 §9.16.1 "Device 0 only configurations" applies.
    fn is_slave_selected(&self) -> bool {
        self.drive_head & ATA_DRIVE_SLAVE != 0
    }

    fn lba28(&self) -> u64 {
        let hi = u64::from(self.drive_head & 0x0F) << 24;
        hi | (u64::from(self.lba_hi) << 16)
            | (u64::from(self.lba_mid) << 8)
            | u64::from(self.lba_lo)
    }

    /// 48-bit LBA assembled from the two-byte deep Command Block FIFOs.
    ///
    /// Spec: ATA/ATAPI-6 §6.20 Table 11 — LBA Low current/previous supply
    /// LBA (7:0)/(31:24), LBA Mid supplies (15:8)/(39:32), LBA High supplies
    /// (23:16)/(47:40). Device register bits 3:0 are reserved and take no part.
    fn lba48(&self) -> u64 {
        u64::from(self.lba_lo)
            | (u64::from(self.lba_mid) << 8)
            | (u64::from(self.lba_hi) << 16)
            | (u64::from(self.lba_lo_prev) << 24)
            | (u64::from(self.lba_mid_prev) << 32)
            | (u64::from(self.lba_hi_prev) << 40)
    }

    fn sector_count_effective(&self) -> u32 {
        if self.sector_count == 0 {
            256
        } else {
            u32::from(self.sector_count)
        }
    }

    /// 16-bit Sector Count for a 48-bit command; `0000h` means 65,536 sectors.
    ///
    /// Spec: ATA/ATAPI-6 §6.20 Table 11 / §8.35.8 / §8.63.8.
    fn sector_count48_effective(&self) -> u32 {
        let count = u32::from(self.sector_count_prev) << 8 | u32::from(self.sector_count);
        if count == 0 {
            65_536
        } else {
            count
        }
    }

    fn total_sectors(&self) -> u64 {
        (self.image.len() / SECTOR_SIZE) as u64
    }

    /// IDENTIFY words (61:60) value for a capacity.
    ///
    /// Spec: ATA/ATAPI-6 §6.20 — "if the device contains greater than the
    /// capacity addressable with 28-bit commands, words (61:60) shall describe
    /// the maximum capacity that can be addressed by 28-bit commands", i.e.
    /// they are capped at 268,435,455 (§6.2.1).
    fn identify_lba28_capacity(total_sectors: u64) -> u32 {
        total_sectors.min(ATA_LBA28_MAX_SECTORS) as u32
    }

    /// Build a minimal IDENTIFY DEVICE payload (256 words, little-endian words).
    ///
    /// Spec: ATA IDENTIFY DEVICE — words 60–61 = total LBA28 user sectors;
    /// word 49 bit9 = LBA supported; model string words 27–46 (byte-swapped).
    fn fill_identify(&mut self) {
        let mut words = [0u16; IDENTIFY_WORDS];
        words[0] = 0x0040; // non-removable ATA disk (bit6)
        words[1] = 16383; // obsolete cylinders
        words[3] = 16; // obsolete heads
        words[6] = 63; // obsolete sectors/track
                       // Model: "x86WASM IDE STUB" padded, ATA byte-swap within words.
        let model = b"x86WASM IDE STUB                        ";
        for (i, chunk) in model.chunks(2).take(20).enumerate() {
            let a = chunk.first().copied().unwrap_or(b' ');
            let b = chunk.get(1).copied().unwrap_or(b' ');
            words[27 + i] = u16::from(a) << 8 | u16::from(b);
        }
        words[47] = 0x8000 | u16::from(ATA_MULTIPLE_MAX_SECTORS); // max sectors/DRQ
        words[49] = 1 << 9; // LBA supported
        words[53] = 0x0001; // words 54–58 valid (legacy)
                            // Spec: ATA IDENTIFY word 59 — bit8 = multiple setting valid; bits7:0 = current.
        if self.multiple_count != 0 {
            words[59] = 0x0100 | u16::from(self.multiple_count);
        }
        let total = self.total_sectors().max(1);
        // Spec: ATA/ATAPI-6 §6.2.1 / §6.20 — words (61:60) are the LBA28
        // user-addressable sector count, capped at 268,435,455.
        let lba28_total = Self::identify_lba28_capacity(total);
        words[60] = (lba28_total & 0xFFFF) as u16;
        words[61] = (lba28_total >> 16) as u16;
        words[63] = 0; // no multiword DMA
        words[80] = 1 << 4; // ATA/ATAPI-4 major version bit (informational)
        words[82] = 0;
        // Spec: ATA/ATAPI-6 §6.20 / Table 27 — word 83 bit14 shall be one and
        // bit10 reports the 48-bit Address feature set; word 86 bit10 mirrors
        // it. READ/WRITE SECTOR(S) EXT are implemented, so this is truthful.
        words[83] = 0x4000 | IDENTIFY_LBA48_SUPPORTED;
        words[85] = 0;
        words[86] = IDENTIFY_LBA48_SUPPORTED;
        // Spec: ATA/ATAPI-6 §6.2.1 — words (103:100) hold the 48-bit
        // user-addressable sector count (max LBA + 1).
        let lba48_total = total.min(ATA_LBA48_MAX_SECTORS);
        words[100] = (lba48_total & 0xFFFF) as u16;
        words[101] = ((lba48_total >> 16) & 0xFFFF) as u16;
        words[102] = ((lba48_total >> 32) & 0xFFFF) as u16;
        words[103] = ((lba48_total >> 48) & 0xFFFF) as u16;

        for (i, w) in words.iter().enumerate() {
            let off = i * 2;
            self.pio[off] = (*w & 0xFF) as u8;
            self.pio[off + 1] = (*w >> 8) as u8;
        }
    }

    fn begin_pio_out(&mut self) {
        self.pio_off = 0;
        self.pio_in = false;
        self.transferring = true;
        self.status = self.ready_status() | ATA_SR_DRQ;
        self.error = 0;
        // Spec: OSDev ATA PIO — IRQ when data ready (DRQ) if nIEN clear.
        self.raise_irq();
    }

    fn begin_pio_in(&mut self) {
        self.pio_off = 0;
        self.pio.fill(0);
        self.pio_in = true;
        self.transferring = true;
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_DRQ;
        self.error = 0;
        // Spec: OSDev ATA PIO WRITE — IRQ when DRQ set (host may fill data).
        self.raise_irq();
    }

    fn load_sector_into_pio(&mut self, lba: u64) -> bool {
        let total = self.total_sectors();
        if total == 0 || lba >= total {
            return false;
        }
        let start = (lba as usize) * SECTOR_SIZE;
        self.pio.fill(0);
        let end = (start + SECTOR_SIZE).min(self.image.len());
        if start < self.image.len() {
            let n = end - start;
            self.pio[..n].copy_from_slice(&self.image[start..end]);
        }
        // Spec: ATA — media read updates the sector buffer (READ BUFFER source).
        self.sector_buffer = self.pio;
        true
    }

    fn store_sector_from_pio(&mut self, lba: u64) -> bool {
        let total = self.total_sectors();
        if total == 0 || lba >= total {
            return false;
        }
        let start = (lba as usize) * SECTOR_SIZE;
        let end = start + SECTOR_SIZE;
        if end > self.image.len() {
            self.image.resize(end, 0);
        }
        self.image[start..end].copy_from_slice(&self.pio);
        // Spec: ATA — media write updates the sector buffer (READ BUFFER source).
        self.sector_buffer = self.pio;
        true
    }

    fn abort_command(&mut self, error: u8) {
        self.error = error;
        self.transferring = false;
        self.pio_in = false;
        self.sector_buffer_write = false;
        self.multiple_xfer = false;
        self.block_left = 0;
        self.sectors_left = 0;
        self.lba48_xfer = false;
        // Spec: ATA/ATAPI-6 §9.8 — an aborted command ends the packet protocol
        // wherever it was.
        self.clear_packet_state();
        self.status = self.ready_status() | ATA_SR_ERR;
        // Spec: ATA — INTRQ on error completion when interrupts enabled.
        self.raise_irq();
    }

    /// Sectors in the next READ/WRITE MULTIPLE DRQ block.
    ///
    /// Spec: ATA — block size is `multiple_count`, except the final block may
    /// be shorter when Sector Count is not divisible by the block factor.
    fn multiple_block_len(&self, sectors_remaining: u32) -> u32 {
        u32::from(self.multiple_count).min(sectors_remaining)
    }

    fn exec_identify(&mut self) {
        // Spec: OSDev ATA PIO — no device / slave → status 0 after IDENTIFY.
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.clear_irq();
            return;
        }
        self.fill_identify();
        self.multiple_xfer = false;
        self.block_left = 0;
        self.sectors_left = 1;
        self.next_lba = 0;
        self.begin_pio_out();
    }

    /// Build the IDENTIFY PACKET DEVICE payload (256 words, little-endian).
    ///
    /// Spec: ATA/ATAPI-6 §8.16 and Table 29. Only the mandatory fields this
    /// device can answer truthfully are filled; everything else is zero, which
    /// §8.16.8 requires of reserved words anyway.
    ///
    /// The interesting choice is word 0 bits (12:8), the command packet set:
    /// a minimal packet device reports `1Fh`; a CD-ROM capable device reports
    /// `05h` with RMB set because READ CAPACITY / READ (10) exist.
    /// Word 82 bit 4 is set because the PACKET Command feature set *is* what
    /// makes this device answer `0xA1` at all; word 82 bit 9 is set because
    /// DEVICE RESET (`0x08`) is implemented; word 53 reports words (70:64) and
    /// word 88 as invalid because there is no timing model.
    fn fill_identify_packet(&mut self) {
        let mut words = [0u16; IDENTIFY_WORDS];
        let peripheral = if self.atapi_cdrom {
            u16::from(ATAPI_PERIPHERAL_DEVICE_TYPE_CDROM)
        } else {
            IDENTIFY_PACKET_SET_UNKNOWN
        };
        // Word 0: ATAPI device (15:14 = 10b), packet set, RMB for CD-ROM,
        // 3 ms DRQ response, 12-byte command packet.
        let mut word0 = IDENTIFY_PACKET_ATAPI_DEVICE | (peripheral << 8);
        if self.atapi_cdrom {
            word0 |= 1 << 7; // RMB
        }
        words[0] = word0;
        let firmware = b"0001    ";
        for (i, chunk) in firmware.chunks(2).take(4).enumerate() {
            let a = chunk.first().copied().unwrap_or(b' ');
            let b = chunk.get(1).copied().unwrap_or(b' ');
            words[23 + i] = u16::from(a) << 8 | u16::from(b);
        }
        let model = if self.atapi_cdrom {
            let mut m = [b' '; 40];
            let s = b"x86WASM ATAPI CD-ROM";
            m[..s.len()].copy_from_slice(s);
            m
        } else {
            let mut m = [b' '; 40];
            let s = b"x86WASM ATAPI PACKET MINIMAL";
            m[..s.len()].copy_from_slice(s);
            m
        };
        for (i, chunk) in model.chunks(2).take(20).enumerate() {
            let a = chunk.first().copied().unwrap_or(b' ');
            let b = chunk.get(1).copied().unwrap_or(b' ');
            words[27 + i] = u16::from(a) << 8 | u16::from(b);
        }
        words[49] = IDENTIFY_PACKET_WORD49_MANDATORY;
        words[50] = IDENTIFY_PACKET_WORD_VALID;
        words[82] = IDENTIFY_PACKET_FEATURE_SET | IDENTIFY_PACKET_DEVICE_RESET;
        words[83] = IDENTIFY_PACKET_WORD_VALID;
        words[84] = IDENTIFY_PACKET_WORD_VALID;
        words[85] = IDENTIFY_PACKET_FEATURE_SET | IDENTIFY_PACKET_DEVICE_RESET;
        words[87] = IDENTIFY_PACKET_WORD_VALID;

        for (i, w) in words.iter().enumerate() {
            let off = i * 2;
            self.pio[off] = (*w & 0xFF) as u8;
            self.pio[off + 1] = (*w >> 8) as u8;
        }
    }

    /// IDENTIFY PACKET DEVICE (`0xA1`).
    ///
    /// Spec: ATA/ATAPI-6 §8.16 — "Use prohibited for devices not implementing
    /// the PACKET Command feature set", so an ATA disk aborts with ERR+ABRT.
    /// On a configured packet device the command is PIO data-in of 256 words
    /// (§8.16.3), is "accepted regardless of the state of DRDY" (§8.16.7), and
    /// completes with DRDY set to one (§8.16.5).
    fn exec_identify_packet(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.clear_irq();
            return;
        }
        if !self.packet_device {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        self.fill_identify_packet();
        self.multiple_xfer = false;
        self.block_left = 0;
        self.sectors_left = 1;
        self.next_lba = 0;
        self.begin_pio_out();
    }

    /// PACKET (`0xA0`) on a device that does **not** implement the PACKET
    /// Command feature set.
    ///
    /// Spec: ATA/ATAPI-6 §8.21.2 — "Use prohibited for devices not implementing
    /// the PACKET Command feature set", so an ATA disk aborts with ERR+ABRT.
    /// INTRQ follows the usual nIEN rules.
    fn exec_packet(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// PACKET (`0xA0`) on a configured packet device — enter the command-packet
    /// transfer phase.
    ///
    /// Spec: ATA/ATAPI-6 §8.21 PACKET and §9.8 "PACKET command protocol". The
    /// device latches the Features register (§8.21.4 DMA and OVL) and the Byte
    /// Count Limit from Cylinder Low / Cylinder High, then sets DRQ with the
    /// Interrupt Reason register reporting C/D = 1, I/O = 0 and REL = 0 so the
    /// host knows to write a command packet.
    ///
    /// **INTRQ is not asserted here.** IDENTIFY PACKET DEVICE word 0 bits (6:5)
    /// report `00b`, "the device shall set DRQ to one within 3 ms of receiving
    /// the PACKET command", not the `01b` "INTRQ DRQ" response; §9.8 asserts
    /// INTRQ for the command-packet DRQ only in the interrupt-DRQ case.
    ///
    /// Model choices, both because §8.21.4 leaves the case indeterminate rather
    /// than defining it: an odd Byte Count Limit is rounded **down** to even,
    /// and a limit that is zero (or rounds to zero) aborts the command instead
    /// of transferring an indeterminate amount.
    fn exec_packet_on_packet_device(&mut self) {
        // Spec: ATA/ATAPI-6 §8.21.4 — neither DMA nor overlap is implemented,
        // and IDENTIFY PACKET DEVICE words 49 and 0 say so. Requesting either
        // is a request for an unimplemented capability, so the command aborts.
        if self.features & (ATAPI_FEATURE_DMA | ATAPI_FEATURE_OVL) != 0 {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        let limit = ((u16::from(self.lba_hi) << 8) | u16::from(self.lba_mid)) & !1;
        if limit == 0 {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        self.clear_packet_state();
        self.packet_byte_count_limit = limit;
        self.packet_phase = PacketPhase::Command;
        self.transferring = false;
        self.pio_in = false;
        self.sector_buffer_write = false;
        self.multiple_xfer = false;
        self.block_left = 0;
        self.sectors_left = 0;
        self.error = 0;
        // Spec: ATA/ATAPI-6 §7.13 — C/D set, I/O clear: the host writes a
        // command packet to the device.
        self.sector_count = ATAPI_IR_CD;
        self.status = self.ready_status() | ATA_SR_DRQ;
        self.clear_irq();
    }

    /// Accept command-packet bytes written to the Data register.
    ///
    /// Spec: ATA/ATAPI-6 §9.8 — the host transfers the command packet under
    /// DRQ; the device clears DRQ and begins processing once the last byte of
    /// the [`ATAPI_PACKET_BYTES`]-byte packet arrives.
    fn write_packet_command(&mut self, size: u8, value: u32) {
        if self.status & ATA_SR_DRQ == 0 {
            return;
        }
        let nbytes = match size {
            4 => 4,
            2 => 2,
            _ => 1,
        };
        for i in 0..nbytes {
            if self.packet_off < ATAPI_PACKET_BYTES {
                self.packet_cmd[self.packet_off] = ((value >> (8 * i)) & 0xFF) as u8;
                self.packet_off += 1;
            }
        }
        if self.packet_off >= ATAPI_PACKET_BYTES {
            self.execute_packet_command();
        }
    }

    /// Dispatch the received command packet.
    ///
    /// Spec: SFF-8020i / MMC command packet set. Minimal PACKET devices run
    /// `TEST UNIT READY`, `REQUEST SENSE`, and `INQUIRY`. CD-ROM capable
    /// devices also run `READ CAPACITY`, `READ (10)`, and `MODE SENSE`.
    fn execute_packet_command(&mut self) {
        self.packet_phase = PacketPhase::Idle;
        match self.packet_cmd[0] {
            ATAPI_CMD_TEST_UNIT_READY => self.exec_packet_test_unit_ready(),
            ATAPI_CMD_REQUEST_SENSE => self.exec_packet_request_sense(),
            ATAPI_CMD_INQUIRY => self.exec_packet_inquiry(),
            ATAPI_CMD_READ_CAPACITY if self.atapi_cdrom => self.exec_packet_read_capacity(),
            ATAPI_CMD_READ_10 if self.atapi_cdrom => self.exec_packet_read10(),
            ATAPI_CMD_MODE_SENSE_6 if self.atapi_cdrom => self.exec_packet_mode_sense6(),
            ATAPI_CMD_MODE_SENSE_10 if self.atapi_cdrom => self.exec_packet_mode_sense10(),
            ATAPI_CMD_START_STOP_UNIT if self.atapi_cdrom => self.exec_packet_start_stop(),
            ATAPI_CMD_READ_TOC if self.atapi_cdrom => self.exec_packet_read_toc(),
            _ => self.complete_packet_check_condition(
                ATAPI_SENSE_ILLEGAL_REQUEST,
                ATAPI_ASC_INVALID_COMMAND_OPERATION_CODE,
                0,
            ),
        }
    }

    /// `TEST UNIT READY` — a non-data packet command.
    ///
    /// Spec: SFF-8020i §10.8.24.
    ///
    /// - Minimal PACKET (`1Fh`): GOOD — there is still no medium model.
    /// - CD-ROM capable, empty: CHECK CONDITION, NOT READY / `3Ah`.
    /// - CD-ROM capable, medium loaded: GOOD.
    ///
    /// See `docs/atapi-r5-medium-sense.md`.
    fn exec_packet_test_unit_ready(&mut self) {
        if self.atapi_cdrom && !self.atapi_medium_loaded() {
            self.complete_packet_check_condition(
                ATAPI_SENSE_NOT_READY,
                ATAPI_ASC_MEDIUM_NOT_PRESENT,
                0,
            );
            return;
        }
        self.complete_packet_good();
    }

    /// The fixed-format sense data describing the last CHECK CONDITION.
    ///
    /// Spec: SFF-8020i §10.8.16 request sense data format. Only the fields this
    /// device can fill truthfully are non-zero: there is no information field
    /// (so VALID stays clear), no command-specific information, no field
    /// replaceable unit code and no sense-key specific data.
    ///
    /// | Byte | Value |
    /// |---|---|
    /// | 0 | `70h` — current error, VALID clear |
    /// | 2 | sense key in bits (3:0); FILEMARK / EOM / ILI clear |
    /// | 7 | additional sense length `0Ah`, giving 18 bytes total |
    /// | 12 | additional sense code |
    /// | 13 | additional sense code qualifier |
    fn sense_data(&self) -> [u8; ATAPI_SENSE_DATA_BYTES] {
        let mut data = [0u8; ATAPI_SENSE_DATA_BYTES];
        data[0] = ATAPI_SENSE_RESPONSE_CODE_CURRENT;
        data[2] = self.sense_key & 0x0F;
        data[7] = (ATAPI_SENSE_DATA_BYTES - 8) as u8;
        data[12] = self.sense_asc;
        data[13] = self.sense_ascq;
        data
    }

    /// `REQUEST SENSE` — report and then clear the sense data.
    ///
    /// Spec: SFF-8020i §10.8.16 — the sense data stays valid until it is
    /// reported, which is what makes the CHECK CONDITION → REQUEST SENSE cycle
    /// work: a host that sees ERR issues this command and learns the reason.
    /// An allocation length of zero reports nothing, so it clears nothing.
    fn exec_packet_request_sense(&mut self) {
        let allocation = usize::from(self.packet_cmd[4]);
        let data = self.sense_data();
        if data.len().min(allocation) > 0 {
            self.clear_sense();
        }
        self.begin_packet_data_in(&data, allocation);
    }

    /// DEVICE RESET (`0x08`).
    ///
    /// Spec: ATA/ATAPI-6 §8.7 — mandatory for a device implementing the PACKET
    /// Command feature set and "use prohibited" (§8.7.2) without it, so an ATA
    /// disk aborts with ERR+ABRT. §8.7.5 normal outputs put the diagnostic code
    /// in the Error register and the §9.12 signature in the Command Block
    /// registers, and the command **shall not** assert INTRQ — unlike EXECUTE
    /// DEVICE DIAGNOSTIC, which does.
    ///
    /// This is a device reset, not a software reset: the Device Control
    /// register (nIEN, HOB) and the Device register are left as the host
    /// programmed them. Everything belonging to a command in progress — the
    /// packet protocol state, the PIO transfer, and the pending sense data — is
    /// dropped, because there is no longer a command to report on.
    fn exec_device_reset(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        if !self.packet_device {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        self.clear_packet_state();
        self.clear_sense();
        self.transferring = false;
        self.pio_in = false;
        self.sector_buffer_write = false;
        self.multiple_xfer = false;
        self.block_left = 0;
        self.sectors_left = 0;
        self.lba48_xfer = false;
        self.error = ATA_DIAG_PASSED;
        self.write_device_signature();
        self.status = self.reset_status();
        self.clear_irq();
    }

    /// `INQUIRY` — standard INQUIRY data as a packet data-in transfer.
    ///
    /// Spec: SFF-8020i §10.8.4 — byte 1 bit 0 is EVPD and byte 2 is the page
    /// code; byte 4 is the allocation length. No vital product data pages are
    /// implemented, so EVPD set or a non-zero page code is CHECK CONDITION with
    /// `24h` INVALID FIELD IN COMMAND PACKET rather than a wrong answer.
    fn exec_packet_inquiry(&mut self) {
        if self.packet_cmd[1] & ATAPI_INQUIRY_EVPD != 0 || self.packet_cmd[2] != 0 {
            self.complete_packet_check_condition(
                ATAPI_SENSE_ILLEGAL_REQUEST,
                ATAPI_ASC_INVALID_FIELD_IN_CDB,
                0,
            );
            return;
        }
        let allocation = usize::from(self.packet_cmd[4]);
        let data = self.inquiry_data();
        self.begin_packet_data_in(&data, allocation);
    }

    /// The standard INQUIRY data this device returns.
    ///
    /// Spec: SFF-8020i §10.8.4 standard INQUIRY data. Peripheral type and RMB
    /// match IDENTIFY PACKET DEVICE word 0.
    fn inquiry_data(&self) -> [u8; ATAPI_INQUIRY_DATA_BYTES] {
        let mut data = [0u8; ATAPI_INQUIRY_DATA_BYTES];
        if self.atapi_cdrom {
            data[0] = ATAPI_PERIPHERAL_DEVICE_TYPE_CDROM;
            data[1] = ATAPI_INQUIRY_RMB;
            data[16..32].copy_from_slice(b"ATAPI CD-ROM    ");
        } else {
            data[0] = ATAPI_PERIPHERAL_DEVICE_TYPE_UNKNOWN;
            data[16..32].copy_from_slice(b"ATAPI PACKET MIN");
        }
        data[3] = ATAPI_INQUIRY_RESPONSE_DATA_FORMAT;
        data[4] = (ATAPI_INQUIRY_DATA_BYTES - 5) as u8;
        data[8..16].copy_from_slice(b"x86WASM ");
        data[32..36].copy_from_slice(b"0001");
        data
    }

    /// `READ CAPACITY` — last LBA and 2048-byte block length.
    ///
    /// Spec: SFF-8020i / MMC. Empty medium → NOT READY / `3Ah`.
    fn exec_packet_read_capacity(&mut self) {
        if !self.atapi_medium_loaded() {
            self.complete_packet_check_condition(
                ATAPI_SENSE_NOT_READY,
                ATAPI_ASC_MEDIUM_NOT_PRESENT,
                0,
            );
            return;
        }
        let blocks = self.atapi_cdrom_blocks();
        let last_lba = (blocks - 1) as u32;
        let mut data = [0u8; ATAPI_READ_CAPACITY_DATA_BYTES];
        data[0..4].copy_from_slice(&last_lba.to_be_bytes());
        data[4..8].copy_from_slice(&(ATAPI_CDROM_BLOCK_BYTES as u32).to_be_bytes());
        self.begin_packet_data_in(&data, ATAPI_READ_CAPACITY_DATA_BYTES);
    }

    /// Medium type code for MODE SENSE. Spec: SFF-8020i Table 46.
    fn atapi_medium_type(&self) -> u8 {
        if self.atapi_medium_loaded() {
            ATAPI_MEDIUM_TYPE_120MM_DATA
        } else {
            ATAPI_MEDIUM_TYPE_NO_DISC
        }
    }

    /// Read Error Recovery page (`01h`). Spec: SFF-8020i Table 52.
    ///
    /// Defaults: error recovery parameter `00h` (maximum recovery, recovered
    /// errors not reported), read retry count `0`, PS clear (not savable).
    fn mode_page_error_recovery(&self) -> [u8; 8] {
        [
            ATAPI_MODE_PAGE_ERROR_RECOVERY,
            ATAPI_MODE_PAGE_ERROR_RECOVERY_LEN,
            0x00, // error recovery parameter
            0x00, // read retry count
            0x00,
            0x00,
            0x00,
            0x00,
        ]
    }

    /// Build MODE SENSE page payload for the requested page code.
    ///
    /// Spec: SFF-8020i §9.8.4 — unsupported page → ILLEGAL REQUEST / `24h`.
    /// Only page `01h` is implemented; page `3Fh` returns that single page.
    fn mode_sense_pages(&self, page_code: u8) -> Option<Vec<u8>> {
        match page_code & 0x3F {
            ATAPI_MODE_PAGE_ERROR_RECOVERY | 0x3F => {
                Some(self.mode_page_error_recovery().to_vec())
            }
            _ => None,
        }
    }

    /// `MODE SENSE (6)` — 4-byte header + page(s), no block descriptors.
    ///
    /// Spec: MMC / SPC CDB: byte 2 = PC|Page Code, byte 4 = allocation length.
    /// Empty medium still succeeds (SFF-8020i Table 8).
    fn exec_packet_mode_sense6(&mut self) {
        let page_code = self.packet_cmd[2] & 0x3F;
        let pc = (self.packet_cmd[2] >> 6) & 0x03;
        // Saved values are not implemented.
        if pc == 0x03 {
            self.complete_packet_check_condition(
                ATAPI_SENSE_ILLEGAL_REQUEST,
                ATAPI_ASC_INVALID_FIELD_IN_CDB,
                0,
            );
            return;
        }
        let Some(pages) = self.mode_sense_pages(page_code) else {
            self.complete_packet_check_condition(
                ATAPI_SENSE_ILLEGAL_REQUEST,
                ATAPI_ASC_INVALID_FIELD_IN_CDB,
                0,
            );
            return;
        };
        let allocation = usize::from(self.packet_cmd[4]);
        // Changeable values: page header retained, parameters zeroed.
        let pages = if pc == 0x01 {
            let mut mask = pages;
            if mask.len() > 2 {
                for b in &mut mask[2..] {
                    *b = 0;
                }
            }
            mask
        } else {
            pages
        };
        let mut data = Vec::with_capacity(4 + pages.len());
        data.push(0); // mode data length filled below
        data.push(self.atapi_medium_type());
        data.push(0); // device-specific parameter
        data.push(0); // block descriptor length
        data.extend_from_slice(&pages);
        data[0] = (data.len() - 1) as u8;
        self.begin_packet_data_in(&data, allocation);
    }

    /// `MODE SENSE (10)` — 8-byte header + page(s), no block descriptors.
    ///
    /// Spec: SFF-8020i §9.8.4 / Table 45. Allocation length at bytes 7–8.
    fn exec_packet_mode_sense10(&mut self) {
        let page_code = self.packet_cmd[2] & 0x3F;
        let pc = (self.packet_cmd[2] >> 6) & 0x03;
        if pc == 0x03 {
            self.complete_packet_check_condition(
                ATAPI_SENSE_ILLEGAL_REQUEST,
                ATAPI_ASC_INVALID_FIELD_IN_CDB,
                0,
            );
            return;
        }
        let Some(pages) = self.mode_sense_pages(page_code) else {
            self.complete_packet_check_condition(
                ATAPI_SENSE_ILLEGAL_REQUEST,
                ATAPI_ASC_INVALID_FIELD_IN_CDB,
                0,
            );
            return;
        };
        let allocation =
            usize::from(u16::from_be_bytes([self.packet_cmd[7], self.packet_cmd[8]]));
        let pages = if pc == 0x01 {
            let mut mask = pages;
            if mask.len() > 2 {
                for b in &mut mask[2..] {
                    *b = 0;
                }
            }
            mask
        } else {
            pages
        };
        let mut data = Vec::with_capacity(8 + pages.len());
        data.extend_from_slice(&[0, 0]); // mode data length filled below
        data.push(self.atapi_medium_type());
        data.push(0); // device-specific / reserved
        data.extend_from_slice(&[0, 0]); // reserved
        data.extend_from_slice(&[0, 0]); // block descriptor length
        data.extend_from_slice(&pages);
        let mode_len = (data.len() - 2) as u16;
        data[0..2].copy_from_slice(&mode_len.to_be_bytes());
        self.begin_packet_data_in(&data, allocation);
    }

    /// `START STOP UNIT` — start/stop spindle and soft load/eject.
    ///
    /// Spec: SFF-8020i §9.8.26 / Table 136:
    /// - LoEj=0 Start=0 → stop (no medium change)
    /// - LoEj=0 Start=1 → start (no medium change)
    /// - LoEj=1 Start=0 → eject / unload → medium not present
    /// - LoEj=1 Start=1 → load: no-op when already loaded; empty stays empty
    ///   (host must re-attach an image — there is no tray motor)
    fn exec_packet_start_stop(&mut self) {
        let loej = self.packet_cmd[4] & ATAPI_START_STOP_LOEJ != 0;
        let start = self.packet_cmd[4] & ATAPI_START_STOP_START != 0;
        if loej && !start {
            self.unload_atapi_medium();
        }
        // Load with empty tray cannot invent media; stop/start are no-ops.
        let _ = start;
        self.complete_packet_good();
    }

    /// Convert a logical block address to MSF (M, S, F) with the 150-frame offset.
    ///
    /// Spec: SFF-8020i §7.6 / Red Book — MSF addresses include the 2-second
    /// pre-gap; LBA 0 ↔ 00:02:00.
    fn lba_to_msf(lba: u32) -> (u8, u8, u8) {
        let abs = lba.saturating_add(150);
        let frame = (abs % 75) as u8;
        let sec = ((abs / 75) % 60) as u8;
        let min = (abs / (75 * 60)) as u8;
        (min, sec, frame)
    }

    /// `READ TOC` — single-session TOC for a data CD image.
    ///
    /// Spec: SFF-8020i §9.8.20 format `00b` (Table 112). One Mode-1 data track
    /// starting at LBA 0 and lead-out at `blocks`. Format is taken from MMC
    /// byte 2 bits (3:0) when non-zero, otherwise SFF-8020i byte 9 bits (7:6).
    /// Only format `0` (TOC) and `1` (multi-session summary for a single
    /// session) are implemented.
    fn exec_packet_read_toc(&mut self) {
        if !self.atapi_medium_loaded() {
            self.complete_packet_check_condition(
                ATAPI_SENSE_NOT_READY,
                ATAPI_ASC_MEDIUM_NOT_PRESENT,
                0,
            );
            return;
        }
        let msf = self.packet_cmd[1] & 0x02 != 0;
        let format = {
            let mmc = self.packet_cmd[2] & 0x0F;
            let sff = (self.packet_cmd[9] >> 6) & 0x03;
            if mmc != 0 {
                mmc
            } else {
                sff
            }
        };
        let allocation =
            usize::from(u16::from_be_bytes([self.packet_cmd[7], self.packet_cmd[8]]));
        let blocks = self.atapi_cdrom_blocks() as u32;
        let data = match format {
            0 => {
                let start_track = self.packet_cmd[6];
                if start_track != 0 && start_track != 1 && start_track != ATAPI_TOC_TRACK_LEAD_OUT
                {
                    self.complete_packet_check_condition(
                        ATAPI_SENSE_ILLEGAL_REQUEST,
                        ATAPI_ASC_INVALID_FIELD_IN_CDB,
                        0,
                    );
                    return;
                }
                self.build_toc_format0(msf, start_track, blocks)
            }
            1 => {
                // Single-session summary: first=last session 1, first track LBA 0.
                let mut data = vec![0u8; 12];
                data[0..2].copy_from_slice(&10u16.to_be_bytes());
                data[2] = 1;
                data[3] = 1;
                data[5] = ATAPI_TOC_ADR_CONTROL_DATA;
                data[6] = 1;
                if msf {
                    let (m, s, f) = Self::lba_to_msf(0);
                    data[9] = m;
                    data[10] = s;
                    data[11] = f;
                }
                data
            }
            _ => {
                self.complete_packet_check_condition(
                    ATAPI_SENSE_ILLEGAL_REQUEST,
                    ATAPI_ASC_INVALID_FIELD_IN_CDB,
                    0,
                );
                return;
            }
        };
        self.begin_packet_data_in(&data, allocation);
    }

    fn build_toc_format0(&self, msf: bool, start_track: u8, blocks: u32) -> Vec<u8> {
        let mut descriptors: Vec<[u8; 8]> = Vec::new();
        let include_track1 = start_track == 0 || start_track == 1;
        let include_lead_out =
            start_track == 0 || start_track == 1 || start_track == ATAPI_TOC_TRACK_LEAD_OUT;
        if include_track1 {
            let mut d = [0u8; 8];
            d[1] = ATAPI_TOC_ADR_CONTROL_DATA;
            d[2] = 1;
            if msf {
                let (m, s, f) = Self::lba_to_msf(0);
                d[5] = m;
                d[6] = s;
                d[7] = f;
            } else {
                d[4..8].copy_from_slice(&0u32.to_be_bytes());
            }
            descriptors.push(d);
        }
        if include_lead_out {
            let mut d = [0u8; 8];
            d[1] = ATAPI_TOC_ADR_CONTROL_DATA;
            d[2] = ATAPI_TOC_TRACK_LEAD_OUT;
            if msf {
                let (m, s, f) = Self::lba_to_msf(blocks);
                d[5] = m;
                d[6] = s;
                d[7] = f;
            } else {
                d[4..8].copy_from_slice(&blocks.to_be_bytes());
            }
            descriptors.push(d);
        }
        let body_len = 2 + descriptors.len() * 8;
        let mut data = Vec::with_capacity(2 + body_len);
        data.extend_from_slice(&(body_len as u16).to_be_bytes());
        data.push(1); // first track
        data.push(1); // last track
        for d in descriptors {
            data.extend_from_slice(&d);
        }
        data
    }

    /// `READ (10)` — transfer logical blocks from the attached medium.
    ///
    /// Spec: SFF-8020i / MMC. CDB: LBA at bytes 2–5, transfer length at 7–8
    /// (big-endian). Transfer length 0 transfers nothing and completes GOOD.
    fn exec_packet_read10(&mut self) {
        if !self.atapi_medium_loaded() {
            self.complete_packet_check_condition(
                ATAPI_SENSE_NOT_READY,
                ATAPI_ASC_MEDIUM_NOT_PRESENT,
                0,
            );
            return;
        }
        let lba = u32::from_be_bytes([
            self.packet_cmd[2],
            self.packet_cmd[3],
            self.packet_cmd[4],
            self.packet_cmd[5],
        ]) as u64;
        let count = u16::from_be_bytes([self.packet_cmd[7], self.packet_cmd[8]]) as u64;
        if count == 0 {
            self.complete_packet_good();
            return;
        }
        let blocks = self.atapi_cdrom_blocks();
        if lba >= blocks || count > blocks - lba {
            self.complete_packet_check_condition(
                ATAPI_SENSE_ILLEGAL_REQUEST,
                ATAPI_ASC_LBA_OUT_OF_RANGE,
                0,
            );
            return;
        }
        let start = (lba as usize) * ATAPI_CDROM_BLOCK_BYTES;
        let len = (count as usize) * ATAPI_CDROM_BLOCK_BYTES;
        let data = self.image[start..start + len].to_vec();
        self.begin_packet_data_in(&data, data.len());
    }

    /// Start a device-to-host packet data transfer, truncated to the command's
    /// allocation length.
    ///
    /// Spec: SFF-8020i — a packet command transfers no more than its allocation
    /// length. An allocation length of zero is not an error: nothing is
    /// transferred and the command completes.
    fn begin_packet_data_in(&mut self, data: &[u8], allocation_length: usize) {
        let len = data.len().min(allocation_length);
        if len == 0 {
            self.complete_packet_good();
            return;
        }
        self.packet_data = data[..len].to_vec();
        self.packet_data_off = 0;
        self.begin_packet_data_block();
    }

    /// Present the next DRQ block of packet data.
    ///
    /// Spec: ATA/ATAPI-6 §8.21.4 / §9.8 — a block is at most the Byte Count
    /// Limit; the device reports the actual byte count in Cylinder Low / High,
    /// sets the Interrupt Reason to C/D = 0, I/O = 1 (data to the host), sets
    /// DRQ, and asserts INTRQ.
    fn begin_packet_data_block(&mut self) {
        let remaining = self.packet_data.len() - self.packet_data_off;
        let block = remaining.min(usize::from(self.packet_byte_count_limit));
        self.packet_block_end = self.packet_data_off + block;
        self.lba_mid = (block & 0xFF) as u8;
        self.lba_hi = ((block >> 8) & 0xFF) as u8;
        self.sector_count = ATAPI_IR_IO;
        self.packet_phase = PacketPhase::DataIn;
        self.error = 0;
        self.status = self.ready_status() | ATA_SR_DRQ;
        self.raise_irq();
    }

    /// Data register read during a packet data-in phase.
    ///
    /// Bytes past the end of the current block read as zero: a Byte Count Limit
    /// or allocation length that is odd leaves half of the last 16-bit Data
    /// register cycle undefined, and this model pads rather than over-reading.
    fn read_packet_data(&mut self, size: u8) -> u32 {
        if self.status & ATA_SR_DRQ == 0 {
            return 0xFFFF_FFFF;
        }
        let nbytes = match size {
            4 => 4,
            2 => 2,
            _ => 1,
        };
        let mut val = 0u32;
        for i in 0..nbytes {
            if self.packet_data_off < self.packet_block_end {
                val |= u32::from(self.packet_data[self.packet_data_off]) << (8 * i);
                self.packet_data_off += 1;
            }
        }
        if self.packet_data_off >= self.packet_block_end {
            if self.packet_data_off >= self.packet_data.len() {
                self.complete_packet_good();
            } else {
                self.begin_packet_data_block();
            }
        }
        val
    }

    /// Command completion with GOOD status.
    ///
    /// Spec: ATA/ATAPI-6 §9.8 — at command completion the device clears BSY and
    /// DRQ, sets the Interrupt Reason to C/D = 1 and I/O = 1, and asserts
    /// INTRQ. §8.21.5 sets DRDY.
    fn complete_packet_good(&mut self) {
        self.clear_packet_state();
        self.error = 0;
        self.sector_count = ATAPI_IR_CD | ATAPI_IR_IO;
        self.status = self.ready_status();
        self.raise_irq();
    }

    /// Command completion with CHECK CONDITION.
    ///
    /// Spec: ATA/ATAPI-6 §8.21.6 — the Error register on a PACKET device holds
    /// the Sense Key in bits (7:4); bit 2 is ABRT, set here because the command
    /// was aborted for an invalid operation code or command-packet field.
    /// Status reports ERR, which is how a host knows to issue REQUEST SENSE.
    fn complete_packet_check_condition(&mut self, sense_key: u8, asc: u8, ascq: u8) {
        self.clear_packet_state();
        self.sense_key = sense_key & 0x0F;
        self.sense_asc = asc;
        self.sense_ascq = ascq;
        self.error = (self.sense_key << 4) | ATA_ER_ABRT;
        self.sector_count = ATAPI_IR_CD | ATAPI_IR_IO;
        self.status = self.ready_status() | ATA_SR_ERR;
        self.raise_irq();
    }

    /// IDENTIFY DEVICE (`0xEC`) on a configured packet device.
    ///
    /// Spec: ATA/ATAPI-6 §6.8.1 — "the IDENTIFY DEVICE command shall not be
    /// executed but shall be command aborted and shall return a signature
    /// unique to devices implementing the PACKET Command feature set"; §8.15.5.2
    /// repeats it and points at §9.12 for the full Command Block signature.
    fn exec_identify_on_packet_device(&mut self) {
        self.abort_command(ATA_ER_ABRT);
        self.write_device_signature();
    }

    /// READ SECTOR(S) (`0x20`) on a configured packet device.
    ///
    /// Spec: ATA/ATAPI-6 §8.34.5.2 — "devices that implement the PACKET
    /// Command feature set shall post command aborted and place the PACKET
    /// Command feature set signature in the LBA High and the LBA Mid register".
    /// Only those two registers, unlike IDENTIFY DEVICE.
    fn exec_read_sectors_on_packet_device(&mut self) {
        self.abort_command(ATA_ER_ABRT);
        self.lba_mid = ATAPI_SIGNATURE_LBA_MID;
        self.lba_hi = ATAPI_SIGNATURE_LBA_HIGH;
    }

    /// Dispatch a Command register write on a configured packet device.
    ///
    /// Six commands do something; everything else is aborted with ERR+ABRT,
    /// which is the ATA/ATAPI-6 §8.x response for an unimplemented command.
    fn exec_packet_device_command(&mut self, cmd: u8) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        match cmd {
            ATA_CMD_IDENTIFY_PACKET => self.exec_identify_packet(),
            ATA_CMD_IDENTIFY => self.exec_identify_on_packet_device(),
            ATA_CMD_READ_SECTORS => self.exec_read_sectors_on_packet_device(),
            ATA_CMD_PACKET => self.exec_packet_on_packet_device(),
            ATA_CMD_DEVICE_RESET => self.exec_device_reset(),
            ATA_CMD_DIAGNOSTIC => self.exec_diagnostic(),
            _ => self.abort_command(ATA_ER_ABRT),
        }
    }

    fn exec_read_sectors(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.clear_irq();
            return;
        }
        // Require LBA bit for this stub (CHS not implemented).
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        let count = self.sector_count_effective();
        let lba = self.lba28();
        if !self.load_sector_into_pio(lba) {
            self.abort_command(0x10); // IDNF / sector not found style
            return;
        }
        self.multiple_xfer = false;
        self.block_left = 0;
        self.lba48_xfer = false;
        self.sectors_left = count;
        self.next_lba = lba.wrapping_add(1);
        self.begin_pio_out();
    }

    /// READ SECTOR(S) EXT (`0x24`) — 48-bit Address feature set PIO data-in.
    ///
    /// Spec: ATA/ATAPI-6 §8.35 — 1 to 65,536 sectors (Sector Count `0000h` =
    /// 65,536) starting at the 48-bit LBA in the two-deep Command Block FIFOs
    /// (§6.20 Table 11); DRQ per sector and "the device shall interrupt for
    /// each DRQ block transferred". The Device register LBA bit shall be set
    /// (the feature set "operates in LBA only") — CHS aborts. A requested
    /// address outside the user-addressable range reports IDNF (§8.35.6).
    fn exec_read_sectors_ext(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.clear_irq();
            return;
        }
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        let count = self.sector_count48_effective();
        let lba = self.lba48();
        if !self.lba48_range_in_bounds(lba, count) {
            self.abort_command(ATA_ER_IDNF);
            return;
        }
        if !self.load_sector_into_pio(lba) {
            self.abort_command(ATA_ER_IDNF);
            return;
        }
        self.multiple_xfer = false;
        self.block_left = 0;
        self.lba48_xfer = true;
        self.sectors_left = count;
        self.next_lba = lba + 1;
        self.begin_pio_out();
        self.update_transfer_sector_count();
    }

    /// WRITE SECTOR(S) EXT (`0x34`) — 48-bit Address feature set PIO data-out.
    ///
    /// Spec: ATA/ATAPI-6 §8.63 — same addressing, 16-bit count and per-block
    /// interrupt rules as READ SECTOR(S) EXT; out-of-range reports IDNF
    /// (§8.63.6) and no media is written.
    fn exec_write_sectors_ext(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        let count = self.sector_count48_effective();
        let lba = self.lba48();
        if !self.lba48_range_in_bounds(lba, count) {
            self.abort_command(ATA_ER_IDNF);
            return;
        }
        self.multiple_xfer = false;
        self.block_left = 0;
        self.lba48_xfer = true;
        self.sectors_left = count;
        self.next_lba = lba;
        self.begin_pio_in();
        self.update_transfer_sector_count();
    }

    /// True when the whole `count`-sector range from `lba` is user-addressable.
    ///
    /// Spec: ATA/ATAPI-6 §6.2.2 — a command whose requested LBA is greater than
    /// or equal to the contents of words (103:100) shall report IDNF or ABRT.
    /// This tree validates the entire range before starting so no partial DRQ
    /// block is presented for an out-of-range request.
    fn lba48_range_in_bounds(&self, lba: u64, count: u32) -> bool {
        let total = self.total_sectors();
        if total == 0 || lba >= total {
            return false;
        }
        u64::from(count) <= total - lba
    }

    /// Mirror the remaining sector count back into the task file.
    ///
    /// Spec: ATA/ATAPI-6 §6.20 Table 11 — a 48-bit command's Sector Count spans
    /// both halves of the two-deep FIFO, so the running count is published to
    /// Sector Count (7:0) and (15:8). The LBA28 path keeps its single-byte
    /// decrement. Both are model-defined between DRQ blocks: §8.35.5 / §8.63.5
    /// mark the Sector Count outputs Reserved on completion.
    fn update_transfer_sector_count(&mut self) {
        if self.lba48_xfer {
            self.sector_count = (self.sectors_left & 0xFF) as u8;
            self.sector_count_prev = ((self.sectors_left >> 8) & 0xFF) as u8;
        } else if self.sector_count != 0 {
            self.sector_count = self.sector_count.wrapping_sub(1);
        }
    }

    /// Zero the Sector Count task-file value at successful command completion.
    ///
    /// Spec: ATA/ATAPI-6 §8.35.5 / §8.63.5 — the Sector Count outputs of the
    /// EXT commands are Reserved on normal completion; this tree reports zero
    /// (both FIFO halves) like the LBA28 path.
    fn clear_transfer_sector_count(&mut self) {
        if self.packet_device {
            // Spec: ATA/ATAPI-6 §8.16.5 lists the Command Block registers as
            // "na" after IDENTIFY PACKET DEVICE, and §9.12 only says the
            // signature *may* change once a command sets DRDY to one. This
            // model chooses never to change it, so a host that re-reads the
            // registers after identifying still sees `01h/01h/14h/EBh`.
            self.write_device_signature();
            self.lba48_xfer = false;
            return;
        }
        self.sector_count = 0;
        if self.lba48_xfer {
            self.sector_count_prev = 0;
        }
        self.lba48_xfer = false;
    }

    fn exec_write_sectors(&mut self) {
        // Spec: ATA WRITE SECTORS (0x30) — LBA28 PIO; host fills 256 words/sector.
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(ATA_ER_ABRT); // CHS unsupported
            return;
        }
        let count = self.sector_count_effective();
        let lba = self.lba28();
        let total = self.total_sectors();
        if total == 0 || lba >= total {
            self.abort_command(0x10); // IDNF
            return;
        }
        self.multiple_xfer = false;
        self.block_left = 0;
        self.sectors_left = count;
        self.next_lba = lba;
        self.begin_pio_in();
    }

    /// READ VERIFY SECTORS (`0x40`) / WRITE VERIFY SECTORS (`0x3C`) on ATA master
    /// — non-data LBA28 range check.
    ///
    /// Spec: ATA/ATAPI Command Set — READ/WRITE VERIFY SECTORS verify media
    /// without transferring sector data (no DRQ). This stub succeeds when the
    /// LBA28 range lies within the backing image; OOB → ERR+IDNF; CHS → ABRT.
    /// Absent/slave → status 0. INTRQ follows nIEN like FLUSH CACHE. No write
    /// to media on WRITE VERIFY (range check only).
    fn exec_verify_sectors(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(ATA_ER_ABRT); // CHS unsupported
            return;
        }
        let count = self.sector_count_effective();
        let lba = self.lba28();
        let total = self.total_sectors();
        if total == 0 || lba >= total || u64::from(count) > total - lba {
            self.abort_command(ATA_ER_IDNF);
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// FLUSH CACHE (`0xE7`) on ATA master — non-data success completion.
    ///
    /// Spec: ATA/ATAPI Command Set — FLUSH CACHE writes volatile cache to media.
    /// This stub has no volatile cache; it completes immediately with
    /// DRDY|DSC, error=0, no DRQ, and raises INTRQ when nIEN=0 (SeaBIOS-friendly).
    fn exec_flush_cache(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// NOP (`0x00`) on ATA master — non-data success (no side effects).
    ///
    /// Spec: ATA/ATAPI Command Set — NOP completes with success; this stub
    /// mirrors other non-data success completions (DRDY|DSC, error=0).
    fn exec_nop(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// READ MULTIPLE (`0xC4`) on ATA master — LBA28 multi-sector DRQ PIO.
    ///
    /// Spec: ATA/ATAPI Command Set — READ MULTIPLE transfers up to
    /// `multiple_count` sectors per DRQ interrupt after SET MULTIPLE MODE.
    /// `multiple_count==0` (not configured) → ERR+ABRT. Final DRQ block may be
    /// shorter when Sector Count is not divisible by the block factor. INTRQ
    /// once per DRQ block ready and on command completion when nIEN=0.
    fn exec_read_multiple(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        // Spec: ATA — READ MULTIPLE invalid if Multiple mode not set.
        if self.multiple_count == 0 {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(ATA_ER_ABRT); // CHS unsupported
            return;
        }
        let count = self.sector_count_effective();
        let lba = self.lba28();
        if !self.load_sector_into_pio(lba) {
            self.abort_command(0x10); // IDNF
            return;
        }
        self.multiple_xfer = true;
        self.sectors_left = count;
        self.block_left = self.multiple_block_len(count);
        self.next_lba = lba.wrapping_add(1);
        self.begin_pio_out();
    }

    /// WRITE MULTIPLE (`0xC5`) on ATA master — LBA28 multi-sector DRQ PIO.
    ///
    /// Spec: ATA/ATAPI Command Set — WRITE MULTIPLE transfers up to
    /// `multiple_count` sectors per DRQ interrupt after SET MULTIPLE MODE.
    /// `multiple_count==0` (not configured) → ERR+ABRT. Final DRQ block may be
    /// shorter when Sector Count is not divisible by the block factor. INTRQ
    /// once per DRQ block ready and on command completion when nIEN=0.
    fn exec_write_multiple(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        // Spec: ATA — WRITE MULTIPLE invalid if Multiple mode not set.
        if self.multiple_count == 0 {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        if self.drive_head & ATA_DRIVE_LBA == 0 {
            self.abort_command(ATA_ER_ABRT); // CHS unsupported
            return;
        }
        let count = self.sector_count_effective();
        let lba = self.lba28();
        let total = self.total_sectors();
        if total == 0 || lba >= total {
            self.abort_command(0x10); // IDNF
            return;
        }
        self.multiple_xfer = true;
        self.sectors_left = count;
        self.block_left = self.multiple_block_len(count);
        self.next_lba = lba;
        self.begin_pio_in();
    }

    /// SMART (`0xB0`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — SMART is a feature-set command (subcommands
    /// in Features). This stub does not implement SMART; ATA disks abort with
    /// ERR+ABRT and no return data / DRQ. Absent/slave → status 0. INTRQ follows
    /// nIEN like PACKET / READ MULTIPLE abort.
    fn exec_smart(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// READ DMA (`0xC8`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — READ DMA transfers sectors via bus-master
    /// DMA (PRD). This stub has no BM-DMA engine; ATA disks abort with ERR+ABRT
    /// and no DRQ / DMA start. Absent/slave → status 0. INTRQ follows nIEN like
    /// SMART / PACKET abort.
    fn exec_read_dma(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// WRITE DMA (`0xCA`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — WRITE DMA transfers sectors via bus-master
    /// DMA (PRD). This stub has no BM-DMA engine; ATA disks abort with ERR+ABRT
    /// and no DRQ / DMA start. Absent/slave → status 0. INTRQ follows nIEN like
    /// READ DMA abort.
    fn exec_write_dma(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// SECURITY SET PASSWORD (`0xF1`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — SECURITY SET PASSWORD is a SECURITY
    /// feature-set command that transfers a password identifier / password
    /// buffer to enable device security. This stub does not implement SECURITY
    /// passwords/state; ATA disks abort with ERR+ABRT and no password PIO /
    /// DRQ. Absent/slave → status 0. INTRQ follows nIEN like SECURITY UNLOCK /
    /// FREEZE LOCK abort.
    fn exec_security_set_password(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// SECURITY UNLOCK (`0xF2`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — SECURITY UNLOCK is a SECURITY feature-set
    /// command that transfers a password to unlock the device. This stub does
    /// not implement SECURITY passwords/state; ATA disks abort with ERR+ABRT
    /// and no unlock PIO / DRQ. Absent/slave → status 0. INTRQ follows nIEN like
    /// SECURITY FREEZE LOCK abort.
    fn exec_security_unlock(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// SECURITY ERASE PREPARE (`0xF3`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — SECURITY ERASE PREPARE is a SECURITY
    /// feature-set command that must precede SECURITY ERASE UNIT. This stub does
    /// not implement SECURITY erase/password state; ATA disks abort with
    /// ERR+ABRT and no DRQ. Absent/slave → status 0. INTRQ follows nIEN like
    /// SECURITY SET PASSWORD abort.
    fn exec_security_erase_prepare(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// SECURITY ERASE UNIT (`0xF4`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — SECURITY ERASE UNIT is a SECURITY
    /// feature-set command that erases user data after SECURITY ERASE PREPARE.
    /// This stub does not implement SECURITY erase/password PIO; ATA disks abort
    /// with ERR+ABRT and no DRQ. Absent/slave → status 0. INTRQ follows nIEN like
    /// SECURITY ERASE PREPARE abort.
    fn exec_security_erase_unit(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// SECURITY FREEZE LOCK (`0xF5`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — SECURITY FREEZE LOCK is a SECURITY
    /// feature-set command that freezes the security state until power cycle.
    /// This stub does not implement SECURITY; ATA disks abort with ERR+ABRT and
    /// no freeze state / DRQ. Absent/slave → status 0. INTRQ follows nIEN like
    /// SMART / READ DMA abort.
    fn exec_security_freeze_lock(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// SECURITY DISABLE PASSWORD (`0xF6`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — SECURITY DISABLE PASSWORD is a SECURITY
    /// feature-set command that clears a user/master password via a password
    /// PIO transfer. This stub does not implement SECURITY passwords; ATA disks
    /// abort with ERR+ABRT and no DRQ. Absent/slave → status 0. INTRQ follows
    /// nIEN like SECURITY ERASE UNIT abort.
    fn exec_security_disable_password(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// DOWNLOAD MICROCODE (`0x92`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — DOWNLOAD MICROCODE transfers vendor
    /// microcode (Features subcommand + sector count/buffer). This stub does
    /// not implement microcode download; ATA disks abort with ERR+ABRT and no
    /// data/DRQ transfer. Absent/slave → status 0. INTRQ follows nIEN like
    /// SMART / SECURITY FREEZE abort.
    fn exec_download_microcode(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// READ LOG EXT (`0x2F`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — READ LOG EXT reads a General Purpose Log
    /// page (LBA48 HOB + PIO). This stub does not implement GPL / log pages;
    /// ATA disks abort with ERR+ABRT and no data/DRQ. Absent/slave → status 0.
    /// INTRQ follows nIEN like SMART / DOWNLOAD MICROCODE abort.
    fn exec_read_log_ext(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// WRITE LOG EXT (`0x3F`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — WRITE LOG EXT writes a General Purpose Log
    /// page (LBA48 HOB + PIO). This stub does not implement GPL / log pages;
    /// ATA disks abort with ERR+ABRT and no data/DRQ. Absent/slave → status 0.
    /// INTRQ follows nIEN like READ LOG EXT abort.
    fn exec_write_log_ext(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// DATA SET MANAGEMENT (`0x06`) on ATA master — ABRT stub (TRIM).
    ///
    /// Spec: ATA/ATAPI Command Set — DATA SET MANAGEMENT transfers a DSM/TRIM
    /// range list (Features TRIM bit + sector-count blocks of LBA ranges). This
    /// stub does not implement TRIM; ATA disks abort with ERR+ABRT and no
    /// data/DRQ. Absent/slave → status 0. INTRQ follows nIEN like WRITE LOG EXT.
    fn exec_data_set_management(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// TRUSTED RECEIVE (`0x5C`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — TRUSTED RECEIVE returns Security Protocol
    /// data (SPSP / transfer length + PIO). This stub does not implement Trusted
    /// Computing; ATA disks abort with ERR+ABRT and no data/DRQ. Absent/slave →
    /// status 0. INTRQ follows nIEN like DATA SET MANAGEMENT abort.
    fn exec_trusted_receive(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// TRUSTED SEND (`0x5E`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — TRUSTED SEND transfers Security Protocol
    /// data (SPSP / transfer length + PIO). This stub does not implement Trusted
    /// Computing; ATA disks abort with ERR+ABRT and no data/DRQ. Absent/slave →
    /// status 0. INTRQ follows nIEN like TRUSTED RECEIVE abort.
    fn exec_trusted_send(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// READ BUFFER (`0xE4`) on ATA master — 512-byte PIO data-in.
    ///
    /// Spec: ATA/ATAPI Command Set — READ BUFFER returns the device sector
    /// buffer via 256-word PIO (no LBA). Buffer contents track the last
    /// READ/WRITE SECTORS or WRITE BUFFER transfer (zeros after reset).
    /// Absent/slave → status 0. INTRQ follows nIEN like READ SECTORS / IDENTIFY DRQ.
    fn exec_read_buffer(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.pio = self.sector_buffer;
        self.multiple_xfer = false;
        self.block_left = 0;
        self.sectors_left = 1;
        self.next_lba = 0;
        self.begin_pio_out();
    }

    /// WRITE BUFFER (`0xE8`) on ATA master — 512-byte PIO data-out.
    ///
    /// Spec: ATA/ATAPI Command Set — WRITE BUFFER accepts the device sector
    /// buffer via 256-word host→device PIO (no LBA / no media write). Buffer is
    /// readable via READ BUFFER. Absent/slave → status 0. INTRQ follows nIEN
    /// like WRITE SECTORS (DRQ ready + command complete).
    fn exec_write_buffer(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.sector_buffer_write = false;
            self.clear_irq();
            return;
        }
        self.multiple_xfer = false;
        self.block_left = 0;
        self.sectors_left = 1;
        self.next_lba = 0;
        self.sector_buffer_write = true;
        self.begin_pio_in();
    }

    /// IDLE / IDLE IMMEDIATE / STANDBY IMMEDIATE — non-data success stubs.
    ///
    /// Spec: ATA power-management commands complete with DRDY|DSC; this stub
    /// does not model timers or standby spin-down.
    fn exec_power_mgmt_success(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// CHECK POWER MODE (`0xE5`) — report Active/Idle via sector_count=`0xFF`.
    ///
    /// Spec: ATA CHECK POWER MODE returns power state in the sector count
    /// register (`0xFF` = Active or Idle). Stub always reports Active/Idle.
    fn exec_check_power_mode(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.sector_count = ATA_POWER_ACTIVE_OR_IDLE;
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// RECALIBRATE (`0x10`) / SEEK (`0x70`) — non-data success stubs.
    ///
    /// Spec: ATA RECALIBRATE/SEEK complete with DRDY|DSC; this stub does not
    /// model physical head motion (DSC always set when ready).
    fn exec_recalibrate_seek_success(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// INITIALIZE DEVICE PARAMETERS (`0x91`) — non-data success stub.
    ///
    /// Spec: ATA INITIALIZE DEVICE PARAMETERS programs sectors/heads from the
    /// task file; this stub accepts and succeeds without changing geometry.
    fn exec_init_dev_params(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// SET MULTIPLE MODE (`0xC6`) — store Sector Count as READ/WRITE MULTIPLE block factor.
    ///
    /// Spec: ATA/ATAPI Command Set — SET MULTIPLE MODE: Sector Count selects
    /// sectors per interrupt for READ/WRITE MULTIPLE. Accepted values are
    /// powers of two in `1..=ATA_MULTIPLE_MAX_SECTORS` (IDENTIFY word 47).
    /// Invalid → ERR+ABRT (prior `multiple_count` unchanged). Completes with
    /// DRDY|DSC and INTRQ when nIEN=0.
    fn exec_set_multiple_mode(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        let factor = self.sector_count;
        if !Self::is_valid_multiple_block_factor(factor) {
            self.abort_command(ATA_ER_ABRT);
            return;
        }
        self.multiple_count = factor;
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// Valid SET MULTIPLE MODE Sector Count: power of two ≤ IDENTIFY word 47 max.
    fn is_valid_multiple_block_factor(factor: u8) -> bool {
        factor > 0 && factor <= ATA_MULTIPLE_MAX_SECTORS && factor.is_power_of_two()
    }

    /// READ NATIVE MAX ADDRESS (`0xF8`) — write max LBA28 into task-file regs.
    ///
    /// Spec: ATA READ NATIVE MAX ADDRESS returns the native maximum address in
    /// LBA Low/Mid/High and Device bits 3:0. This stub uses `total_sectors-1`
    /// (or 0 if empty). Completes with DRDY|DSC and INTRQ when nIEN=0.
    fn exec_read_native_max(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        // Spec: ATA/ATAPI-6 §6.20 — "If the native maximum address is greater
        // than 268,435,455, a READ NATIVE MAX ADDRESS command shall cause the
        // device to return a maximum value of 268,435,454."
        let max = self
            .total_sectors()
            .saturating_sub(1)
            .min(ATA_LBA28_MAX_SECTORS - 1);
        self.lba_lo = (max & 0xFF) as u8;
        self.lba_mid = ((max >> 8) & 0xFF) as u8;
        self.lba_hi = ((max >> 16) & 0xFF) as u8;
        self.drive_head = (self.drive_head & 0xF0) | (((max >> 24) & 0x0F) as u8);
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// SET MAX ADDRESS (`0xF9`) on ATA master — ABRT stub.
    ///
    /// Spec: ATA/ATAPI Command Set — SET MAX ADDRESS sets the Host Protected
    /// Area maximum address from the task-file LBA (paired with READ NATIVE
    /// MAX ADDRESS `0xF8`). This stub does not implement HPA; ATA disks abort
    /// with ERR+ABRT and leave capacity unchanged. Absent/slave → status 0.
    /// INTRQ follows nIEN like WRITE BUFFER abort.
    fn exec_set_max_address(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.abort_command(ATA_ER_ABRT);
    }

    /// MEDIA LOCK/UNLOCK (`0xDE`/`0xDF`) — success noop (no tray lock state).
    fn exec_media_lock_unlock(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// EXECUTE DEVICE DIAGNOSTIC (`0x90`).
    ///
    /// Spec: ATA/ATAPI-6 §8.11 / Table 26 — diagnostic code `01h` in the Error
    /// register means "Device 0 passed, Device 1 passed or not present"; note 2
    /// adds that with Device 1 absent the host may see Device 0 information even
    /// though Device 1 is selected. §9.16.1(3) makes this the one Command
    /// register write Device 0 still executes while Device 1 is selected.
    /// Spec: ATA/ATAPI-6 §9.12 — the command also writes the non-PACKET
    /// signature (Sector Count `01h`, LBA Low `01h`, LBA Mid `00h`,
    /// LBA High `00h`) into the Command Block registers.
    fn exec_diagnostic(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        self.error = ATA_DIAG_PASSED;
        self.write_device_signature();
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        // Spec: ATA/ATAPI-6 §9.10 state D0ED3 — a PACKET device clears Status
        // bits 6, 5, 4, 3, 2 and 0, so it reads `00h` here too.
        self.status = self.reset_status();
        self.raise_irq();
    }

    /// Place this device's signature in the Command Block registers.
    ///
    /// Spec: ATA/ATAPI-6 §9.12 Signature and persistence — a device *not*
    /// implementing the PACKET command feature set writes Sector Count `01h`,
    /// LBA Low `01h`, LBA Mid `00h`, LBA High `00h`; a device implementing it
    /// writes Sector Count `01h`, LBA Low `01h`, LBA Mid `14h`, LBA High
    /// `EBh`. Both are written for power-on reset, hardware reset, software
    /// reset and EXECUTE DEVICE DIAGNOSTIC. The Device register keeps the
    /// obsolete ATA-1..5 bits 7/5 (`0xA0`-style) that classic PC firmware
    /// expects, and the DEV bit is left as the host selected it.
    ///
    /// §9.12 also notes the PACKET signature persists "until the device
    /// receives a command that sets DRDY to one"; this device changes it only
    /// when it writes a new one, so the persistence rule holds trivially.
    fn write_device_signature(&mut self) {
        if self.packet_device {
            self.sector_count = ATAPI_SIGNATURE_SECTOR_COUNT;
            self.lba_lo = ATAPI_SIGNATURE_LBA_LOW;
            self.lba_mid = ATAPI_SIGNATURE_LBA_MID;
            self.lba_hi = ATAPI_SIGNATURE_LBA_HIGH;
        } else {
            self.sector_count = ATA_SIGNATURE_SECTOR_COUNT;
            self.lba_lo = ATA_SIGNATURE_LBA_LOW;
            self.lba_mid = ATA_SIGNATURE_LBA_MID;
            self.lba_hi = ATA_SIGNATURE_LBA_HIGH;
        }
    }

    /// SET FEATURES (`0xEF`) — accept features register, succeed without side effects.
    ///
    /// Spec: ATA SET FEATURES uses the Features register as a subcommand.
    /// This stub completes successfully on present master (SeaBIOS-friendly
    /// accept); feature-specific behavior remains unsupported.
    fn exec_set_features(&mut self) {
        if !self.present {
            self.status = 0;
            self.transferring = false;
            self.pio_in = false;
            self.clear_irq();
            return;
        }
        let _subcmd = self.features; // accepted; no side effects yet
        self.error = 0;
        self.transferring = false;
        self.pio_in = false;
        self.sectors_left = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC;
        self.raise_irq();
    }

    /// Dispatch a Command register write.
    ///
    /// Spec: ATA/ATAPI-6 §9.16.1(3) / Table 18 — in a Device 0 only
    /// configuration, a write to the Command register while Device 1 is
    /// selected shall be ignored, "except for EXECUTE DEVICE DIAGNOSTIC".
    /// Ignoring means Device 0 keeps its status, Error register, interrupt
    /// pending state, and any in-progress PIO transfer.
    fn exec_command(&mut self, cmd: u8) {
        if self.is_slave_selected() && cmd != ATA_CMD_DIAGNOSTIC {
            return;
        }
        // Any new command leaves the 48-bit transfer mode; the EXT handlers
        // re-arm it (ATA/ATAPI-6 §6.20 — 28-bit and 48-bit commands intermix).
        self.lba48_xfer = false;
        // Spec: ATA/ATAPI-6 §6.8 — a device implementing the PACKET Command
        // feature set "exhibits responses different from those exhibited by
        // devices not implementing this feature set", so it gets its own
        // dispatch rather than the ATA command table.
        if self.packet_device {
            self.exec_packet_device_command(cmd);
            return;
        }
        match cmd {
            ATA_CMD_IDENTIFY => self.exec_identify(),
            ATA_CMD_PACKET => self.exec_packet(),
            // Spec: ATA/ATAPI-6 §8.7.2 — "use prohibited" without the PACKET
            // Command feature set.
            ATA_CMD_DEVICE_RESET => self.exec_device_reset(),
            ATA_CMD_IDENTIFY_PACKET => self.exec_identify_packet(),
            ATA_CMD_READ_SECTORS => self.exec_read_sectors(),
            ATA_CMD_WRITE_SECTORS => self.exec_write_sectors(),
            ATA_CMD_READ_SECTORS_EXT => self.exec_read_sectors_ext(),
            ATA_CMD_WRITE_SECTORS_EXT => self.exec_write_sectors_ext(),
            ATA_CMD_READ_VERIFY_SECTORS | ATA_CMD_WRITE_VERIFY_SECTORS => {
                self.exec_verify_sectors()
            }
            ATA_CMD_FLUSH_CACHE | ATA_CMD_FLUSH_CACHE_EXT => self.exec_flush_cache(),
            ATA_CMD_NOP => self.exec_nop(),
            ATA_CMD_READ_MULTIPLE => self.exec_read_multiple(),
            ATA_CMD_WRITE_MULTIPLE => self.exec_write_multiple(),
            ATA_CMD_SMART => self.exec_smart(),
            ATA_CMD_READ_DMA => self.exec_read_dma(),
            ATA_CMD_WRITE_DMA => self.exec_write_dma(),
            ATA_CMD_SECURITY_SET_PASSWORD => self.exec_security_set_password(),
            ATA_CMD_SECURITY_UNLOCK => self.exec_security_unlock(),
            ATA_CMD_SECURITY_ERASE_PREPARE => self.exec_security_erase_prepare(),
            ATA_CMD_SECURITY_ERASE_UNIT => self.exec_security_erase_unit(),
            ATA_CMD_SECURITY_FREEZE_LOCK => self.exec_security_freeze_lock(),
            ATA_CMD_SECURITY_DISABLE_PASSWORD => self.exec_security_disable_password(),
            ATA_CMD_DOWNLOAD_MICROCODE => self.exec_download_microcode(),
            ATA_CMD_READ_LOG_EXT => self.exec_read_log_ext(),
            ATA_CMD_WRITE_LOG_EXT => self.exec_write_log_ext(),
            ATA_CMD_DATA_SET_MANAGEMENT => self.exec_data_set_management(),
            ATA_CMD_TRUSTED_RECEIVE => self.exec_trusted_receive(),
            ATA_CMD_TRUSTED_SEND => self.exec_trusted_send(),
            ATA_CMD_READ_BUFFER => self.exec_read_buffer(),
            ATA_CMD_WRITE_BUFFER => self.exec_write_buffer(),
            ATA_CMD_SET_MULTIPLE_MODE => self.exec_set_multiple_mode(),
            ATA_CMD_IDLE
            | ATA_CMD_IDLE_IMMEDIATE
            | ATA_CMD_STANDBY_IMMEDIATE
            | ATA_CMD_STANDBY
            | ATA_CMD_SLEEP => self.exec_power_mgmt_success(),
            ATA_CMD_CHECK_POWER_MODE => self.exec_check_power_mode(),
            ATA_CMD_RECALIBRATE | ATA_CMD_SEEK => self.exec_recalibrate_seek_success(),
            ATA_CMD_INIT_DEV_PARAMS => self.exec_init_dev_params(),
            ATA_CMD_READ_NATIVE_MAX => self.exec_read_native_max(),
            ATA_CMD_SET_MAX_ADDRESS => self.exec_set_max_address(),
            ATA_CMD_MEDIA_LOCK | ATA_CMD_MEDIA_UNLOCK => self.exec_media_lock_unlock(),
            ATA_CMD_DIAGNOSTIC => self.exec_diagnostic(),
            ATA_CMD_SET_FEATURES => self.exec_set_features(),
            _ => self.abort_command(ATA_ER_ABRT), // unsupported command
        }
    }

    fn read_data(&mut self, size: u8) -> u32 {
        // Spec: ATA/ATAPI-6 Table 18 only defines Device 0 responses for
        // Device 1 with BSY=0 and DRQ=0, so a Data port cycle aimed at the
        // absent Device 1 has no defined answer. Documented model choice: the
        // cycle is ignored so an in-progress Device 0 DRQ block is preserved.
        if self.is_slave_selected() {
            return 0xFFFF_FFFF;
        }
        // Spec: ATA/ATAPI-6 §9.8 — a packet data-in phase uses the same Data
        // register but a variable byte count, not the 512-byte sector engine.
        if self.packet_phase == PacketPhase::DataIn {
            return self.read_packet_data(size);
        }
        if !self.transferring || self.pio_in || self.status & ATA_SR_DRQ == 0 {
            return 0xFFFF_FFFF;
        }
        let mut val = 0u32;
        let nbytes = match size {
            4 => 4,
            2 => 2,
            _ => 1,
        };
        for i in 0..nbytes {
            if self.pio_off < SECTOR_SIZE {
                val |= u32::from(self.pio[self.pio_off]) << (8 * i);
                self.pio_off += 1;
            }
        }
        if self.pio_off >= SECTOR_SIZE {
            self.finish_sector_pio_out();
        }
        val
    }

    fn write_data(&mut self, size: u8, value: u32) {
        // See `read_data`: Data port cycles for the absent Device 1 are ignored.
        if self.is_slave_selected() {
            return;
        }
        // Spec: ATA/ATAPI-6 §9.8 — the command packet arrives through the Data
        // register while the Interrupt Reason reports C/D = 1, I/O = 0.
        if self.packet_phase == PacketPhase::Command {
            self.write_packet_command(size, value);
            return;
        }
        if !self.transferring || !self.pio_in || self.status & ATA_SR_DRQ == 0 {
            return;
        }
        let nbytes = match size {
            4 => 4,
            2 => 2,
            _ => 1,
        };
        for i in 0..nbytes {
            if self.pio_off < SECTOR_SIZE {
                self.pio[self.pio_off] = ((value >> (8 * i)) & 0xFF) as u8;
                self.pio_off += 1;
            }
        }
        if self.pio_off >= SECTOR_SIZE {
            self.finish_sector_pio_in();
        }
    }

    fn finish_sector_pio_out(&mut self) {
        if self.sectors_left > 0 {
            self.sectors_left -= 1;
        }
        if self.multiple_xfer && self.block_left > 0 {
            self.block_left -= 1;
        }
        if self.sectors_left == 0 {
            self.transferring = false;
            self.pio_in = false;
            self.multiple_xfer = false;
            self.block_left = 0;
            self.pio_off = 0;
            self.status = self.ready_status();
            self.clear_transfer_sector_count();
            // Spec: ATA — INTRQ on command completion after final sector.
            self.raise_irq();
            return;
        }
        // Multi-sector READ / READ MULTIPLE: present next sector.
        if !self.load_sector_into_pio(self.next_lba) {
            self.abort_command(0x10);
            return;
        }
        self.next_lba = self.next_lba.wrapping_add(1);
        self.pio_off = 0;
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_DRQ;
        // Spec: sector count decrements as sectors transfer.
        self.update_transfer_sector_count();
        if self.multiple_xfer {
            if self.block_left == 0 {
                // Spec: ATA READ MULTIPLE — IRQ when next multi-sector DRQ ready.
                self.block_left = self.multiple_block_len(self.sectors_left);
                self.raise_irq();
            }
            // Within a Multiple DRQ block: keep DRQ, no IRQ between sectors.
        } else {
            // Spec: OSDev ATA PIO — IRQ again when next sector DRQ ready.
            self.raise_irq();
        }
    }

    fn finish_sector_pio_in(&mut self) {
        // Spec: ATA WRITE BUFFER — commit 512-byte PIO into sector buffer only.
        if self.sector_buffer_write {
            self.sector_buffer = self.pio;
            self.sector_buffer_write = false;
            self.sectors_left = 0;
            self.transferring = false;
            self.pio_in = false;
            self.pio_off = 0;
            self.status = ATA_SR_DRDY | ATA_SR_DSC;
            self.clear_transfer_sector_count();
            // Spec: ATA — INTRQ on WRITE BUFFER command completion.
            self.raise_irq();
            return;
        }
        // Spec: ATA WRITE SECTORS / WRITE MULTIPLE — commit filled sector.
        let lba = self.next_lba;
        if !self.store_sector_from_pio(lba) {
            self.abort_command(0x10);
            return;
        }
        if self.sectors_left > 0 {
            self.sectors_left -= 1;
        }
        if self.multiple_xfer && self.block_left > 0 {
            self.block_left -= 1;
        }
        if self.sectors_left == 0 {
            self.transferring = false;
            self.pio_in = false;
            self.multiple_xfer = false;
            self.block_left = 0;
            self.pio_off = 0;
            self.status = ATA_SR_DRDY | ATA_SR_DSC;
            self.clear_transfer_sector_count();
            // Spec: ATA — INTRQ on WRITE command completion.
            self.raise_irq();
            return;
        }
        self.next_lba = lba.wrapping_add(1);
        if self.next_lba >= self.total_sectors() {
            self.abort_command(0x10);
            return;
        }
        self.pio_off = 0;
        self.pio.fill(0);
        self.status = ATA_SR_DRDY | ATA_SR_DSC | ATA_SR_DRQ;
        self.update_transfer_sector_count();
        if self.multiple_xfer {
            if self.block_left == 0 {
                // Spec: ATA WRITE MULTIPLE — IRQ when next multi-sector DRQ ready.
                self.block_left = self.multiple_block_len(self.sectors_left);
                self.raise_irq();
            }
            // Within a Multiple DRQ block: keep DRQ, no IRQ between sectors.
        } else {
            // Spec: OSDev ATA PIO WRITE — IRQ when next sector DRQ ready.
            self.raise_irq();
        }
    }

    /// Device Control register write (`0x3F6` / `0x376`).
    ///
    /// Spec: ATA/ATAPI-6 §7.8.5 — "When the Device Control register is written,
    /// both devices respond to the write regardless of which device is
    /// selected", and §9.16.1(1) repeats that with Device 1 selected and absent
    /// the write completes as if Device 0 was selected. SRST and nIEN therefore
    /// take effect on Device 0 whatever the DEV bit says.
    fn write_dev_ctrl(&mut self, value: u8) {
        let prev = self.dev_ctrl;
        // Spec: ATA/ATAPI-6 §7.8.6 / §6.20 — bit7 is HOB for the 48-bit
        // Address feature set; bits 6:3 are reserved and bit0 is obsolete.
        self.dev_ctrl = value & (ATA_DC_HOB | ATA_DC_SRST | ATA_DC_NIEN | 0x01);
        // Spec: ATA device control — SRST high then low performs software reset.
        if prev & ATA_DC_SRST == 0 && value & ATA_DC_SRST != 0 {
            // Enter reset: BSY
            if self.present {
                self.status = ATA_SR_BSY;
            }
            self.clear_irq();
        } else if prev & ATA_DC_SRST != 0 && value & ATA_DC_SRST == 0 {
            if self.present {
                self.reset_ready();
            } else {
                self.status = 0;
                self.clear_irq();
            }
        }
    }

    /// Status / Alternate Status value presented to the host.
    ///
    /// Spec: ATA/ATAPI-6 §9.16.1(4) / Table 18 — in a Device 0 only
    /// configuration, a read of the Status or Alternate Status register while
    /// Device 1 is selected returns `00h`. An entirely empty channel also reads
    /// `00h` because no device drives the bus.
    fn status_byte(&self) -> u8 {
        if !self.present || self.is_slave_selected() {
            return 0;
        }
        self.status
    }

    /// Status register read (`0x1F7` / `0x177`).
    ///
    /// Spec: OSDev ATA PIO — reading Status (not Alternate Status) clears the
    /// pending interrupt. Spec: ATA/ATAPI-6 §9.16.1(4) — while Device 1 is
    /// selected this read only returns `00h`; Device 0 interrupt pending is
    /// left alone so a reselect still delivers the interrupt (§5.2.9).
    fn read_status_clear_irq(&mut self) -> u8 {
        if self.is_slave_selected() {
            return 0;
        }
        self.clear_irq();
        self.status_byte()
    }
}

impl IdePrimary {
    /// True when the host asked for the "previous content" FIFO half.
    ///
    /// Spec: ATA/ATAPI-6 §6.20 — "If HOB (bit 7) in the Device Control register
    /// is cleared to zero the host reads the 'most recently written' content."
    #[inline]
    fn hob(&self) -> bool {
        self.dev_ctrl & ATA_DC_HOB != 0
    }

    /// Push a Command Block register write into its two-byte deep FIFO.
    ///
    /// Spec: ATA/ATAPI-6 §6.20 — "the new content written is placed into the
    /// 'most recently written' location and the previous content of the
    /// register is moved to 'previous content' location. ... The 'most recently
    /// written' content always gets written by a register write regardless of
    /// the state of HOB."
    #[inline]
    fn fifo_write(current: &mut u8, previous: &mut u8, value: u8) {
        *previous = *current;
        *current = value;
    }

    /// Clear Device Control HOB after any Command Block register write.
    ///
    /// Spec: ATA/ATAPI-6 §6.20 — "A write to any Command Block register shall
    /// cause the device to clear the HOB bit to zero in the Device Control
    /// register." Data port writes are excluded in this tree so a PIO data-out
    /// block cannot clear HOB mid-transfer (documented model choice).
    #[inline]
    fn clear_hob(&mut self) {
        self.dev_ctrl &= !ATA_DC_HOB;
    }
}

impl PortDevice for IdePrimary {
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        let hob = self.hob();
        match port {
            IDE_PRIMARY_DATA => self.read_data(size),
            IDE_PRIMARY_ERROR => u32::from(self.error),
            // Spec: ATA/ATAPI-6 §6.20 Table 11 — HOB selects the FIFO half.
            IDE_PRIMARY_SECCOUNT if hob => u32::from(self.sector_count_prev),
            IDE_PRIMARY_SECCOUNT => u32::from(self.sector_count),
            IDE_PRIMARY_LBA_LO if hob => u32::from(self.lba_lo_prev),
            IDE_PRIMARY_LBA_LO => u32::from(self.lba_lo),
            IDE_PRIMARY_LBA_MID if hob => u32::from(self.lba_mid_prev),
            IDE_PRIMARY_LBA_MID => u32::from(self.lba_mid),
            IDE_PRIMARY_LBA_HI if hob => u32::from(self.lba_hi_prev),
            IDE_PRIMARY_LBA_HI => u32::from(self.lba_hi),
            IDE_PRIMARY_DRIVE => u32::from(self.drive_head),
            IDE_PRIMARY_STATUS => u32::from(self.read_status_clear_irq()),
            // Spec: alt status mirrors status without clearing IRQ.
            IDE_PRIMARY_CTRL => u32::from(self.status_byte()),
            _ => 0xFFFF_FFFF,
        }
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        match port {
            IDE_PRIMARY_DATA => self.write_data(size, value),
            IDE_PRIMARY_ERROR => {
                Self::fifo_write(&mut self.features, &mut self.features_prev, value as u8);
                self.clear_hob();
            }
            IDE_PRIMARY_SECCOUNT => {
                Self::fifo_write(
                    &mut self.sector_count,
                    &mut self.sector_count_prev,
                    value as u8,
                );
                self.clear_hob();
            }
            IDE_PRIMARY_LBA_LO => {
                Self::fifo_write(&mut self.lba_lo, &mut self.lba_lo_prev, value as u8);
                self.clear_hob();
            }
            IDE_PRIMARY_LBA_MID => {
                Self::fifo_write(&mut self.lba_mid, &mut self.lba_mid_prev, value as u8);
                self.clear_hob();
            }
            IDE_PRIMARY_LBA_HI => {
                Self::fifo_write(&mut self.lba_hi, &mut self.lba_hi_prev, value as u8);
                self.clear_hob();
            }
            IDE_PRIMARY_DRIVE => {
                // Spec: ATA/ATAPI-6 §7.7 — DEV selects the device; the Device
                // register is not part of the two-deep FIFO (§6.20 Table 11).
                self.drive_head = value as u8;
                self.clear_hob();
            }
            IDE_PRIMARY_STATUS => {
                // Command register.
                if self.status & ATA_SR_BSY != 0 {
                    return;
                }
                let cmd = value as u8;
                self.exec_command(cmd);
                self.clear_hob();
            }
            IDE_PRIMARY_CTRL => self.write_dev_ctrl(value as u8),
            _ => {}
        }
    }
}

/// Secondary ATA IDE channel — thin port remap of [`IdePrimary`] to `0x170`/`0x376`.
///
/// # Spec refs
///
/// - OSDev ATA PIO Mode — secondary command block `0x170`–`0x177`, control `0x376`;
///   secondary channel → ISA IRQ15.
/// - ATA / ATAPI — same IDENTIFY / READ / WRITE / READ BUFFER (`0xE4`) /
///   WRITE BUFFER (`0xE8`) PIO semantics as primary (via inner).
/// - Intel 8259A — DualPic IR15 (slave IR7) via MachineBus.
///
/// # Scope
///
/// - Master only; IDENTIFY / READ / WRITE / READ BUFFER / WRITE BUFFER /
///   PACKET+IDENTIFY PACKET ABRT via inner [`IdePrimary`]
/// - IRQ15 when INTRQ ∧ ¬nIEN (`irq_line`)
///
/// # Unsupported
///
/// - Slave drive, DMA, LBA48, PACKET media engine, PCI BAR remap
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct IdeSecondary {
    /// Shared ATA PIO engine (ports remapped in [`PortDevice`]).
    pub inner: IdePrimary,
}

impl IdeSecondary {
    /// Empty secondary channel (no drive) — status reads `0`.
    pub fn new() -> Self {
        Self {
            inner: IdePrimary::new(),
        }
    }

    pub fn with_image(image: Vec<u8>) -> Self {
        Self {
            inner: IdePrimary::with_image(image),
        }
    }

    pub fn attach_image(&mut self, image: Vec<u8>) {
        self.inner.attach_image(image);
    }

    /// Configure Device 0 as a PACKET (ATAPI) device with no media.
    ///
    /// See [`IdePrimary::attach_atapi_device`] — minimal packet set, no CD-ROM.
    pub fn attach_atapi_device(&mut self) {
        self.inner.attach_atapi_device();
    }

    /// A secondary channel whose Device 0 is a PACKET (ATAPI) device.
    pub fn with_atapi_device() -> Self {
        Self {
            inner: IdePrimary::with_atapi_device(),
        }
    }

    /// See [`IdePrimary::attach_atapi_cdrom`].
    pub fn attach_atapi_cdrom(&mut self) {
        self.inner.attach_atapi_cdrom();
    }

    /// See [`IdePrimary::with_atapi_cdrom`].
    pub fn with_atapi_cdrom() -> Self {
        Self {
            inner: IdePrimary::with_atapi_cdrom(),
        }
    }

    /// See [`IdePrimary::attach_atapi_cdrom_image`].
    pub fn attach_atapi_cdrom_image(&mut self, image: Vec<u8>) {
        self.inner.attach_atapi_cdrom_image(image);
    }

    /// See [`IdePrimary::with_atapi_cdrom_image`].
    pub fn with_atapi_cdrom_image(image: Vec<u8>) -> Self {
        Self {
            inner: IdePrimary::with_atapi_cdrom_image(image),
        }
    }

    /// See [`IdePrimary::load_atapi_medium`].
    pub fn load_atapi_medium(&mut self, image: Vec<u8>) -> bool {
        self.inner.load_atapi_medium(image)
    }

    /// See [`IdePrimary::unload_atapi_medium`].
    pub fn unload_atapi_medium(&mut self) {
        self.inner.unload_atapi_medium();
    }

    /// True when Device 0 implements the PACKET Command feature set.
    pub fn is_packet_device(&self) -> bool {
        self.inner.is_packet_device()
    }

    /// See [`IdePrimary::is_atapi_cdrom`].
    pub fn is_atapi_cdrom(&self) -> bool {
        self.inner.is_atapi_cdrom()
    }

    /// See [`IdePrimary::atapi_medium_loaded`].
    pub fn atapi_medium_loaded(&self) -> bool {
        self.inner.atapi_medium_loaded()
    }

    /// See [`IdePrimary::atapi_sense`].
    pub fn atapi_sense(&self) -> (u8, u8, u8) {
        self.inner.atapi_sense()
    }

    pub fn reset(&mut self) {
        self.inner.reset();
    }

    /// True if this device owns the secondary I/O port.
    pub fn owns_port(port: u16) -> bool {
        matches!(port, 0x170..=0x177 | IDE_SECONDARY_CTRL)
    }

    /// ISA IRQ15 line level (INTRQ ∧ ¬nIEN).
    ///
    /// Spec: ATA device control nIEN; OSDev ATA PIO — secondary → IRQ15.
    pub fn irq_line(&self) -> bool {
        self.inner.irq_line()
    }

    /// Map secondary ports onto the primary register file used by [`IdePrimary`].
    fn map_port(port: u16) -> u16 {
        match port {
            0x170..=0x177 => port - IDE_SECONDARY_DATA + IDE_PRIMARY_DATA,
            IDE_SECONDARY_CTRL => IDE_PRIMARY_CTRL,
            _ => port,
        }
    }
}

impl PortDevice for IdeSecondary {
    fn port_read(&mut self, port: u16, size: u8) -> u32 {
        self.inner.port_read(Self::map_port(port), size)
    }

    fn port_write(&mut self, port: u16, size: u8, value: u32) {
        self.inner.port_write(Self::map_port(port), size, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identify_word(pio: &[u8; SECTOR_SIZE], idx: usize) -> u16 {
        let off = idx * 2;
        u16::from(pio[off]) | (u16::from(pio[off + 1]) << 8)
    }

    fn clear_nien(ide: &mut IdePrimary) {
        ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    }

    /// Spec: ATA/ATAPI-6 §6.2.1 / §6.20 — IDENTIFY words (61:60) hold the LBA28
    /// user-addressable sector count and are capped at 268,435,455 when the
    /// device is larger than 28-bit addressing can reach. Exercised directly
    /// because a >128 GiB backing image cannot be allocated in a unit test.
    #[test]
    fn identify_lba28_capacity_caps_at_28_bit_maximum() {
        assert_eq!(IdePrimary::identify_lba28_capacity(0), 0);
        assert_eq!(IdePrimary::identify_lba28_capacity(4), 4);
        assert_eq!(
            IdePrimary::identify_lba28_capacity(ATA_LBA28_MAX_SECTORS),
            ATA_LBA28_MAX_SECTORS as u32
        );
        assert_eq!(
            IdePrimary::identify_lba28_capacity(ATA_LBA28_MAX_SECTORS + 1),
            ATA_LBA28_MAX_SECTORS as u32
        );
        assert_eq!(
            IdePrimary::identify_lba28_capacity(ATA_LBA48_MAX_SECTORS),
            ATA_LBA28_MAX_SECTORS as u32
        );
    }

    /// Spec: ATA/ATAPI-6 §6.20 Table 11 — the two-deep FIFO assembles the
    /// 48-bit LBA from current/previous halves, and the Device register bits
    /// 3:0 are reserved (they must not leak into the address like LBA28).
    #[test]
    fn lba48_assembles_from_fifo_halves_only() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0xEF); // LBA(31:24)
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0x12); // LBA(7:0)
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0xBE); // LBA(39:32)
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x34); // LBA(15:8)
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0xAD); // LBA(47:40)
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0x56); // LBA(23:16)
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA) | 0x0F);
        assert_eq!(ide.lba48(), 0x0000_ADBE_EF56_3412);
        // The LBA28 view still uses Device bits 3:0 (Table 12).
        assert_eq!(ide.lba28(), 0x0F56_3412);
    }

    /// Spec: ATA/ATAPI-6 §6.20 Table 11 / §8.35.8 — Sector Count `0000h`
    /// requests 65,536 sectors for a 48-bit command.
    #[test]
    fn sector_count48_zero_is_65536() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x00);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x00);
        assert_eq!(ide.sector_count48_effective(), 65_536);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x01); // count(15:8)
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x02); // count(7:0)
        assert_eq!(ide.sector_count48_effective(), 0x0102);
    }

    #[test]
    fn absent_drive_status_is_zero() {
        // Spec: OSDev ATA PIO — IDENTIFY on missing drive → status 0.
        let mut ide = IdePrimary::new();
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
    }

    #[test]
    fn identify_sets_drq_and_returns_256_words() {
        // Spec: ATA IDENTIFY DEVICE — 256 words via data port when DRQ set.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 4]);
        assert_eq!(
            ide.port_read(IDE_PRIMARY_STATUS, 1) as u8,
            ATA_SR_DRDY | ATA_SR_DSC
        );
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_BSY, 0);
        assert_ne!(st & ATA_SR_DRQ, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8, 0);

        let mut words = Vec::with_capacity(IDENTIFY_WORDS);
        for _ in 0..IDENTIFY_WORDS {
            words.push(ide.port_read(IDE_PRIMARY_DATA, 2) as u16);
        }
        assert_eq!(words[49] & (1 << 9), 1 << 9, "LBA supported");
        assert_eq!(words[60], 4);
        assert_eq!(words[61], 0);
        let st_done = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st_done & ATA_SR_DRQ, 0);
        assert_ne!(st_done & ATA_SR_DRDY, 0);
    }

    #[test]
    fn read_sectors_lba28_pio() {
        // Spec: ATA READ SECTORS (0x20) — LBA28, sector count, 256 words/sector.
        let mut img = vec![0u8; SECTOR_SIZE * 3];
        img[SECTOR_SIZE] = 0xAA;
        img[SECTOR_SIZE + 1] = 0x55;
        img[SECTOR_SIZE + 511] = 0xC3;
        let mut ide = IdePrimary::with_image(img);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        let w0 = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
        assert_eq!(w0, 0x55AA);
        for _ in 1..255 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        let w_last = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
        // Little-endian word at bytes 510–511: low=0x00, high=0xC3.
        assert_eq!(w_last, 0xC300);
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn read_sectors_multi_two() {
        let mut img = vec![0u8; SECTOR_SIZE * 2];
        img[0] = 0x11;
        img[SECTOR_SIZE] = 0x22;
        let mut ide = IdePrimary::with_image(img);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        let first = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
        assert_eq!(first & 0xFF, 0x11);
        for _ in 1..256 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        // Second sector should now be under DRQ.
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        let second = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
        assert_eq!(second & 0xFF, 0x22);
        for _ in 1..256 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn read_oob_sets_err() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 5);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
    }

    #[test]
    fn alt_status_mirrors_status() {
        // Spec: IBM PC/AT — 0x3F6 alternate status mirrors status without IRQ ack.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        assert_eq!(
            ide.port_read(IDE_PRIMARY_CTRL, 1) as u8,
            ide.port_read(IDE_PRIMARY_STATUS, 1) as u8
        );
    }

    #[test]
    fn srst_restores_ready() {
        // Spec: ATA device control SRST pulse → software reset.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_SRST | ATA_DC_NIEN));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_BSY, 0);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        assert_eq!(
            ide.port_read(IDE_PRIMARY_STATUS, 1) as u8,
            ATA_SR_DRDY | ATA_SR_DSC
        );
    }

    #[test]
    fn owns_primary_ports_only() {
        assert!(IdePrimary::owns_port(IDE_PRIMARY_DATA));
        assert!(IdePrimary::owns_port(IDE_PRIMARY_STATUS));
        assert!(IdePrimary::owns_port(IDE_PRIMARY_CTRL));
        assert!(!IdePrimary::owns_port(0x170));
        assert!(!IdePrimary::owns_port(0x3F7));
    }

    #[test]
    fn identify_total_sectors_in_words_60_61() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 0x1_0001]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_eq!(identify_word(&ide.pio, 60), 0x0001);
        assert_eq!(identify_word(&ide.pio, 61), 0x0001);
    }

    #[test]
    fn identify_asserts_irq14_when_nien_clear() {
        // Spec: ATA + OSDev ATA PIO — INTRQ when DRQ ready if nIEN=0.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn nien_set_masks_irq_line() {
        // Spec: ATA device control — nIEN=1 disables INTRQ pin.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        // Default reset leaves nIEN set.
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn status_read_clears_irq_alt_does_not() {
        // Spec: OSDev ATA PIO — Status clears IRQ; alternate status does not.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_PRIMARY_CTRL, 1);
        assert!(ide.irq_line(), "alt status must not clear IRQ");
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);
        assert!(!ide.irq_line());
    }

    #[test]
    fn read_sectors_asserts_irq_on_drq() {
        // Spec: ATA READ SECTORS — IRQ when sector data ready (DRQ).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert!(ide.irq_line());
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn error_completion_asserts_irq_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 5);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn multi_sector_raises_irq_per_drq_block() {
        // Spec: OSDev ATA PIO — IRQ for each sector DRQ when interrupts enabled.
        let mut img = vec![0u8; SECTOR_SIZE * 2];
        img[0] = 0x11;
        img[SECTOR_SIZE] = 0x22;
        let mut ide = IdePrimary::with_image(img);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1); // ack first IRQ
        assert!(!ide.irq_line());
        for _ in 0..256 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        // Second sector under DRQ → IRQ again.
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(ide.irq_line());
    }

    fn write_sector_words(ide: &mut IdePrimary, first: u16, last: u16) {
        ide.port_write(IDE_PRIMARY_DATA, 2, u32::from(first));
        for _ in 1..255 {
            ide.port_write(IDE_PRIMARY_DATA, 2, 0);
        }
        ide.port_write(IDE_PRIMARY_DATA, 2, u32::from(last));
    }

    #[test]
    fn write_sectors_lba28_pio() {
        // Spec: ATA WRITE SECTORS (0x30) — LBA28, 256 words/sector into media.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 3]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        write_sector_words(&mut ide, 0x55AA, 0xC300);
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_eq!(ide.image[SECTOR_SIZE], 0xAA);
        assert_eq!(ide.image[SECTOR_SIZE + 1], 0x55);
        assert_eq!(ide.image[SECTOR_SIZE + 511], 0xC3);
    }

    #[test]
    fn write_sectors_multi_two() {
        // Spec: ATA WRITE SECTORS — multi-sector PIO commits each sector in order.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 2]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        write_sector_words(&mut ide, 0x0011, 0);
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        write_sector_words(&mut ide, 0x0022, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        assert_eq!(ide.image[0], 0x11);
        assert_eq!(ide.image[SECTOR_SIZE], 0x22);
    }

    #[test]
    fn write_oob_sets_err() {
        // Spec: ATA — out-of-range LBA → ERR (IDNF-style).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 5);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
    }

    #[test]
    fn write_sectors_asserts_irq_on_drq_and_complete() {
        // Spec: ATA WRITE + OSDev — IRQ at DRQ and again at command complete.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        assert!(ide.irq_line());
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1); // ack DRQ IRQ
        assert!(!ide.irq_line());
        write_sector_words(&mut ide, 0xBEEF, 0);
        // Completion IRQ after final sector commit.
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);
        assert!(!ide.irq_line());
    }

    #[test]
    fn write_nien_masks_irq_line() {
        // Spec: ATA device control — nIEN=1 disables INTRQ during WRITE.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn write_then_read_round_trip() {
        // Spec: WRITE then READ SECTORS see committed media.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 2]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        write_sector_words(&mut ide, 0x55AA, 0xC300);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0x55AA);
        for _ in 1..255 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0xC300);
    }

    #[test]
    fn identify_packet_aborts_on_ata_master() {
        // Spec: ATA/ATAPI — IDENTIFY PACKET DEVICE (0xA1) on ATA disk → ERR+ABRT.
        // Master remains ATA; no ATAPI identify PIO buffer in this stub.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY_PACKET));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn identify_packet_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion (SeaBIOS may poll or use IRQ).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY_PACKET));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn identify_packet_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device probe → status 0.
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY_PACKET));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
    }

    #[test]
    fn packet_aborts_on_ata_master() {
        // Spec: ATA/ATAPI — PACKET (0xA0) is for ATAPI devices; ATA disk → ERR+ABRT.
        // No 12-byte packet PIO / DRQ phase on non-ATAPI master (SeaBIOS-friendly).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn packet_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match WRITE/IDENTIFY).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn packet_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match WRITE SECTORS).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn packet_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_PACKET));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — FLUSH CACHE (`0xE7`) non-data success on
    /// ATA master: DRDY|DSC, error=0, no DRQ; INTRQ when nIEN=0.
    /// Spec: ATA/ATAPI — READ VERIFY SECTORS (`0x40`) non-data success for
    /// in-range LBA28; no DRQ / no sector transfer.
    #[test]
    fn read_verify_sectors_succeeds_in_range() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 4]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_READ_VERIFY_SECTORS),
        );
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_ne!(st & ATA_SR_DSC, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
    }

    /// Spec: ATA — READ VERIFY SECTORS out-of-range → ERR+IDNF, no DRQ.
    #[test]
    fn read_verify_sectors_oob_sets_idnf() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 5);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_READ_VERIFY_SECTORS),
        );
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0x10);
    }

    /// Spec: ATA — READ VERIFY range that spills past image end → IDNF.
    #[test]
    fn read_verify_sectors_partial_spill_sets_idnf() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 2]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 1); // LBA 1..2 needs 2 sectors; only 0..1 exist
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_READ_VERIFY_SECTORS),
        );
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0x10);
    }

    /// Spec: ATA — nIEN gates INTRQ on READ VERIFY success.
    #[test]
    fn read_verify_sectors_nien_masks_irq() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_READ_VERIFY_SECTORS),
        );
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI — WRITE VERIFY SECTORS (`0x3C`) non-data success for
    /// in-range LBA28; no DRQ / no media write.
    #[test]
    fn write_verify_sectors_succeeds_in_range_no_write() {
        let img = vec![0x5Au8; SECTOR_SIZE * 2];
        let mut ide = IdePrimary::with_image(img.clone());
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_WRITE_VERIFY_SECTORS),
        );
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert_eq!(ide.image, img, "WRITE VERIFY must not alter media");
    }

    /// Spec: ATA — WRITE VERIFY SECTORS out-of-range → ERR+IDNF.
    #[test]
    fn write_verify_sectors_oob_sets_idnf() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 3);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_WRITE_VERIFY_SECTORS),
        );
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0x10);
    }

    #[test]
    fn flush_cache_succeeds_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_FLUSH_CACHE));
        // Spec: ATA — INTRQ asserted on completion; status read clears it.
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8; // alt status: no IRQ clear
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_ne!(st & ATA_SR_DSC, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(ide.irq_line(), "alt status must not clear INTRQ");
    }

    #[test]
    fn flush_cache_nien_masks_irq() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_FLUSH_CACHE));
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA — EXECUTE DEVICE DIAGNOSTIC (`0x90`) → error=0x01 passed.
    #[test]
    fn diagnostic_passes_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_DIAGNOSTIC));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_DIAG_PASSED);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
    }

    /// Spec: ATA — SET FEATURES (`0xEF`) succeeds on ATA master (no side effects).
    #[test]
    fn set_features_succeeds_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_ERROR, 1, 0x03); // features write via error port alias
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_FEATURES));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
    }

    #[test]
    fn diagnostic_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_DIAGNOSTIC));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
    }

    #[test]
    fn set_features_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_FEATURES));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
    }

    #[test]
    fn flush_cache_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_FLUSH_CACHE));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI — NOP (`0x00`) non-data success on ATA master.
    #[test]
    fn nop_succeeds_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_NOP));
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_ne!(st & ATA_SR_DSC, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
    }

    #[test]
    fn nop_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_NOP));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA — READ MULTIPLE (`0xC4`) without SET MULTIPLE MODE → ERR+ABRT.
    #[test]
    fn read_multiple_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_MULTIPLE));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
    }

    #[test]
    fn read_multiple_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_MULTIPLE));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA — WRITE MULTIPLE (`0xC5`) without SET MULTIPLE MODE → ERR+ABRT.
    #[test]
    fn write_multiple_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_MULTIPLE));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
    }

    /// Spec: ATA/ATAPI Command Set — SMART (`0xB0`) is a feature-set command.
    /// This stub has no SMART support; ATA master → ERR+ABRT, no data/DRQ.
    #[test]
    fn smart_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SMART));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn smart_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match PACKET/READ MULTIPLE).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SMART));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn smart_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match PACKET abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SMART));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn smart_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SMART));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — READ DMA (`0xC8`) needs bus-master DMA.
    /// This stub has no BM-DMA/PRD engine; ATA master → ERR+ABRT, no DRQ.
    #[test]
    fn read_dma_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_DMA));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn read_dma_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match SMART/PACKET).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_DMA));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn read_dma_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match SMART abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_DMA));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn read_dma_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_DMA));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — WRITE DMA (`0xCA`) needs bus-master DMA.
    /// This stub has no BM-DMA/PRD engine; ATA master → ERR+ABRT, no DRQ.
    #[test]
    fn write_dma_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_DMA));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn write_dma_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match READ DMA).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_DMA));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn write_dma_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match READ DMA abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_DMA));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn write_dma_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_DMA));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — SECURITY SET PASSWORD (`0xF1`) is a
    /// SECURITY feature-set command. This stub has no SECURITY passwords/state;
    /// ATA master → ERR+ABRT, no password PIO/DRQ.
    #[test]
    fn security_set_password_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_SET_PASSWORD),
        );
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn security_set_password_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match UNLOCK/FREEZE).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_SET_PASSWORD),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn security_set_password_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match UNLOCK abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_SET_PASSWORD),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn security_set_password_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_SET_PASSWORD),
        );
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — SECURITY ERASE PREPARE (`0xF3`) is a
    /// SECURITY feature-set command that must precede SECURITY ERASE UNIT. This
    /// stub has no SECURITY erase/password state; ATA master → ERR+ABRT, no DRQ.
    #[test]
    fn security_erase_prepare_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_ERASE_PREPARE),
        );
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn security_erase_prepare_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match SET PASSWORD).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_ERASE_PREPARE),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn security_erase_prepare_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match SET PASSWORD abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_ERASE_PREPARE),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn security_erase_prepare_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_ERASE_PREPARE),
        );
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — SECURITY ERASE UNIT (`0xF4`) is a
    /// SECURITY feature-set command that follows SECURITY ERASE PREPARE. This
    /// stub has no SECURITY erase/password PIO; ATA master → ERR+ABRT, no DRQ.
    #[test]
    fn security_erase_unit_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_ERASE_UNIT),
        );
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn security_erase_unit_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match ERASE PREPARE).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_ERASE_UNIT),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn security_erase_unit_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match ERASE PREPARE abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_ERASE_UNIT),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn security_erase_unit_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_ERASE_UNIT),
        );
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — SECURITY DISABLE PASSWORD (`0xF6`) is a
    /// SECURITY feature-set command that clears passwords via PIO. This stub
    /// has no SECURITY password PIO; ATA master → ERR+ABRT, no DRQ.
    #[test]
    fn security_disable_password_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_DISABLE_PASSWORD),
        );
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn security_disable_password_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match ERASE UNIT).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_DISABLE_PASSWORD),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn security_disable_password_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match ERASE UNIT abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_DISABLE_PASSWORD),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn security_disable_password_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_DISABLE_PASSWORD),
        );
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — SECURITY UNLOCK (`0xF2`) is a SECURITY
    /// feature-set command. This stub has no SECURITY passwords/state; ATA
    /// master → ERR+ABRT, no unlock PIO/DRQ.
    #[test]
    fn security_unlock_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SECURITY_UNLOCK));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn security_unlock_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match FREEZE LOCK).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SECURITY_UNLOCK));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn security_unlock_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match FREEZE LOCK abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SECURITY_UNLOCK));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn security_unlock_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SECURITY_UNLOCK));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — SECURITY FREEZE LOCK (`0xF5`) is a SECURITY
    /// feature-set command. This stub has no SECURITY support; ATA master →
    /// ERR+ABRT, no freeze state/DRQ.
    #[test]
    fn security_freeze_lock_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_FREEZE_LOCK),
        );
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn security_freeze_lock_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match SMART/READ DMA).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_FREEZE_LOCK),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn security_freeze_lock_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match SMART abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_FREEZE_LOCK),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn security_freeze_lock_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_SECURITY_FREEZE_LOCK),
        );
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — DOWNLOAD MICROCODE (`0x92`) transfers vendor
    /// microcode. This stub has no microcode path; ATA master → ERR+ABRT, no DRQ.
    #[test]
    fn download_microcode_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_DOWNLOAD_MICROCODE));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn download_microcode_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match SMART/SECURITY).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_DOWNLOAD_MICROCODE));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn download_microcode_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match SMART abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_DOWNLOAD_MICROCODE));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn download_microcode_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_DOWNLOAD_MICROCODE));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — READ LOG EXT (`0x2F`) reads a General
    /// Purpose Log page. This stub has no GPL/log path; ATA master → ERR+ABRT, no DRQ.
    #[test]
    fn read_log_ext_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_LOG_EXT));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn read_log_ext_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match SMART/DOWNLOAD).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_LOG_EXT));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn read_log_ext_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match SMART abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_LOG_EXT));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn read_log_ext_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_LOG_EXT));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — WRITE LOG EXT (`0x3F`) writes a General
    /// Purpose Log page. This stub has no GPL/log path; ATA master → ERR+ABRT, no DRQ.
    #[test]
    fn write_log_ext_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_LOG_EXT));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn write_log_ext_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match READ LOG EXT).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_LOG_EXT));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn write_log_ext_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match READ LOG EXT abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_LOG_EXT));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn write_log_ext_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_LOG_EXT));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — DATA SET MANAGEMENT (`0x06`) transfers a
    /// TRIM/DSM range list. This stub has no TRIM path; ATA master → ERR+ABRT, no DRQ.
    #[test]
    fn data_set_management_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_DATA_SET_MANAGEMENT),
        );
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn data_set_management_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match WRITE LOG EXT).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_DATA_SET_MANAGEMENT),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn data_set_management_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match WRITE LOG EXT abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_DATA_SET_MANAGEMENT),
        );
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn data_set_management_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(
            IDE_PRIMARY_STATUS,
            1,
            u32::from(ATA_CMD_DATA_SET_MANAGEMENT),
        );
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — TRUSTED RECEIVE (`0x5C`) returns Security
    /// Protocol data. This stub has no Trusted Computing path; ATA master →
    /// ERR+ABRT, no DRQ.
    #[test]
    fn trusted_receive_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_TRUSTED_RECEIVE));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn trusted_receive_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match DSM abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_TRUSTED_RECEIVE));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn trusted_receive_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match DSM abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_TRUSTED_RECEIVE));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn trusted_receive_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_TRUSTED_RECEIVE));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — TRUSTED SEND (`0x5E`) transfers Security
    /// Protocol data. This stub has no Trusted Computing path; ATA master →
    /// ERR+ABRT, no DRQ.
    #[test]
    fn trusted_send_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_TRUSTED_SEND));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
    }

    #[test]
    fn trusted_send_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match TRUSTED RECEIVE).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_TRUSTED_SEND));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn trusted_send_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match TRUSTED RECEIVE).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_TRUSTED_SEND));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn trusted_send_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_TRUSTED_SEND));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — READ BUFFER (`0xE4`) returns the 512-byte
    /// sector buffer via PIO (no LBA). Buffer is last WRITE/READ SECTORS payload.
    #[test]
    fn read_buffer_pio_returns_sector_buffer() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        // Seed sector buffer via WRITE SECTORS (same device buffer path).
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        write_sector_words(&mut ide, 0x55AA, 0xC300);

        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0x55AA);
        for _ in 1..255 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0xC300);
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn read_buffer_zeros_before_any_transfer() {
        // Spec: ATA READ BUFFER — sector buffer starts cleared; 256-word PIO of zeros.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        for _ in 0..256 {
            assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0);
        }
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn read_buffer_asserts_irq_on_drq_when_nien_clear() {
        // Spec: ATA / OSDev PIO — INTRQ when DRQ ready if nIEN=0 (match READ SECTORS).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn read_buffer_nien_masks_irq_on_drq() {
        // Spec: ATA device control — nIEN=1 masks INTRQ during READ BUFFER DRQ.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn read_buffer_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — WRITE BUFFER (`0xE8`) accepts 512 bytes via
    /// host→device DRQ PIO into the device sector buffer (no LBA / no media write).
    #[test]
    fn write_buffer_pio_fills_sector_buffer() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        let image_before = ide.image.clone();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        write_sector_words(&mut ide, 0x55AA, 0xC300);
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        // Spec: WRITE BUFFER updates the sector buffer only — not backing media.
        assert_eq!(ide.image, image_before);
    }

    /// Spec: ATA WRITE BUFFER then READ BUFFER — round-trip via shared sector buffer.
    #[test]
    fn write_buffer_read_buffer_round_trip() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        write_sector_words(&mut ide, 0x55AA, 0xC300);

        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_ne!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0x55AA);
        for _ in 1..255 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16, 0xC300);
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn write_buffer_asserts_irq_on_drq_and_complete_when_nien_clear() {
        // Spec: ATA / OSDev PIO WRITE — INTRQ at DRQ and again at command complete.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        assert!(ide.irq_line());
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1); // ack DRQ IRQ
        assert!(!ide.irq_line());
        write_sector_words(&mut ide, 0xBEEF, 0);
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);
        assert!(!ide.irq_line());
    }

    #[test]
    fn write_buffer_nien_masks_irq_on_drq() {
        // Spec: ATA device control — nIEN=1 masks INTRQ during WRITE BUFFER DRQ.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn write_buffer_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA — IDLE / IDLE IMMEDIATE / STANDBY IMMEDIATE / STANDBY / SLEEP
    /// succeed; CHECK POWER MODE sets sector_count=`0xFF` (Active/Idle).
    #[test]
    fn idle_standby_sleep_and_check_power_mode() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        for cmd in [
            ATA_CMD_IDLE,
            ATA_CMD_IDLE_IMMEDIATE,
            ATA_CMD_STANDBY_IMMEDIATE,
            ATA_CMD_STANDBY,
            ATA_CMD_SLEEP,
        ] {
            ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(cmd));
            assert!(ide.irq_line());
            let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8; // clears IRQ
            assert_eq!(st & ATA_SR_ERR, 0);
            assert_ne!(st & ATA_SR_DRDY, 0);
        }
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x00);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_CHECK_POWER_MODE));
        assert!(ide.irq_line());
        assert_eq!(
            ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8,
            ATA_POWER_ACTIVE_OR_IDLE
        );
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
    }

    /// Spec: ATA — RECALIBRATE (`0x10`) and SEEK (`0x70`) non-data success.
    #[test]
    fn recalibrate_and_seek_succeed() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        for cmd in [ATA_CMD_RECALIBRATE, ATA_CMD_SEEK] {
            ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(cmd));
            assert!(ide.irq_line());
            let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
            assert_eq!(st & ATA_SR_ERR, 0);
            assert_ne!(st & ATA_SR_DSC, 0);
        }
    }

    /// Spec: ATA — INITIALIZE DEVICE PARAMETERS (`0x91`) non-data success.
    #[test]
    fn init_device_parameters_succeeds() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 63);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_INIT_DEV_PARAMS));
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
    }

    /// Spec: ATA — FLUSH CACHE EXT (`0xEA`) same success path as FLUSH CACHE.
    #[test]
    fn flush_cache_ext_succeeds_like_flush_cache() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_FLUSH_CACHE_EXT));
        assert!(ide.irq_line());
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
    }

    /// Spec: ATA — READ NATIVE MAX ADDRESS (`0xF8`) writes max LBA into task file.
    #[test]
    fn read_native_max_address_writes_task_file() {
        // 4 sectors → max LBA = 3.
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 4]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_NATIVE_MAX));
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 3);
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_DRIVE, 1) as u8 & 0x0F, 0);
        let st = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
    }

    /// Spec: ATA/ATAPI Command Set — SET MAX ADDRESS (`0xF9`) sets HPA max from
    /// task-file LBA (paired with READ NATIVE MAX `0xF8` success). This stub has
    /// no HPA path; ATA master → ERR+ABRT, no DRQ, capacity unchanged.
    #[test]
    fn set_max_address_aborts_on_ata_master() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 4]);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        // Attempt to shrink max below native (would be HPA if implemented).
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MAX_ADDRESS));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
        // Capacity unchanged: READ NATIVE MAX still reports LBA 3.
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_NATIVE_MAX));
        assert_eq!(ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8, 3);
    }

    #[test]
    fn set_max_address_asserts_irq_on_abort_when_nien_clear() {
        // Spec: ATA — INTRQ on error completion when nIEN=0 (match WRITE BUFFER).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MAX_ADDRESS));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn set_max_address_nien_masks_irq_on_abort() {
        // Spec: ATA device control — nIEN=1 masks INTRQ (match WRITE BUFFER abort).
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MAX_ADDRESS));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn set_max_address_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MAX_ADDRESS));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA SET MULTIPLE MODE (`0xC6`) — Sector Count = block factor;
    /// valid powers of two within IDENTIFY word 47 max (16) succeed and store
    /// the factor (observable via `multiple_count` + IDENTIFY word 59).
    #[test]
    fn set_multiple_mode_stores_valid_block_factors() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        for factor in [1u8, 2, 4, 8, 16] {
            ide.port_write(IDE_PRIMARY_SECCOUNT, 1, u32::from(factor));
            ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MULTIPLE_MODE));
            assert!(ide.irq_line(), "factor {factor}");
            let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
            assert_eq!(st & ATA_SR_ERR, 0, "factor {factor}");
            assert_ne!(st & ATA_SR_DRDY, 0, "factor {factor}");
            assert_eq!(
                ide.port_read(IDE_PRIMARY_ERROR, 1) as u8,
                0,
                "factor {factor}"
            );
            assert_eq!(ide.multiple_count, factor, "factor {factor}");

            // IDENTIFY word 59: bit8 valid + bits7:0 = current multiple setting.
            ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
            assert_eq!(
                identify_word(&ide.pio, 59),
                0x0100 | u16::from(factor),
                "IDENTIFY word 59 for factor {factor}"
            );
            // Drain IDENTIFY PIO so the next command is not blocked on DRQ.
            for _ in 0..IDENTIFY_WORDS {
                let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
            }
        }
    }

    /// Spec: ATA SET MULTIPLE MODE — invalid Sector Count → ERR+ABRT; prior
    /// multiple_count unchanged.
    #[test]
    fn set_multiple_mode_invalid_factor_aborts() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 8);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MULTIPLE_MODE));
        assert_eq!(ide.multiple_count, 8);

        for bad in [0u8, 3, 5, 7, 9, 15, 17, 32, 255] {
            ide.port_write(IDE_PRIMARY_SECCOUNT, 1, u32::from(bad));
            ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MULTIPLE_MODE));
            assert!(ide.irq_line(), "bad {bad}");
            assert_ne!(
                ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_ERR,
                0,
                "bad {bad}"
            );
            assert_eq!(
                ide.port_read(IDE_PRIMARY_ERROR, 1) as u8,
                ATA_ER_ABRT,
                "bad {bad}"
            );
            assert_eq!(ide.multiple_count, 8, "bad {bad} must not clobber");
        }
    }

    #[test]
    fn set_multiple_mode_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing device → status 0 (not ABRT from catch-all).
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 16);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MULTIPLE_MODE));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0);
        assert_eq!(ide.multiple_count, 0);
        assert!(!ide.irq_line());
    }

    fn set_multiple_mode(ide: &mut IdePrimary, factor: u8) {
        ide.port_write(IDE_PRIMARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, u32::from(factor));
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_SET_MULTIPLE_MODE));
        assert_eq!(ide.multiple_count, factor);
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8 & ATA_SR_ERR, 0);
    }

    fn drain_pio_sector_after_first_word(ide: &mut IdePrimary) {
        for _ in 1..256 {
            let _ = ide.port_read(IDE_PRIMARY_DATA, 2);
        }
    }

    /// Spec: ATA READ MULTIPLE (`0xC4`) — after SET MULTIPLE MODE, transfer
    /// `multiple_count` sectors per DRQ; IRQ once per block (not per sector).
    #[test]
    fn read_multiple_multi_sector_drq_and_irq_per_block() {
        let mut img = vec![0u8; SECTOR_SIZE * 4];
        img[0] = 0xA1;
        img[SECTOR_SIZE] = 0xA2;
        img[SECTOR_SIZE * 2] = 0xA3;
        img[SECTOR_SIZE * 3] = 0xA4;
        let mut ide = IdePrimary::with_image(img);
        clear_nien(&mut ide);
        set_multiple_mode(&mut ide, 2);

        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 4);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_MULTIPLE));
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1); // ack first-block IRQ
        assert!(!ide.irq_line());

        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16 & 0xFF, 0xA1);
        drain_pio_sector_after_first_word(&mut ide);
        // Still inside first DRQ block — no IRQ between sectors.
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16 & 0xFF, 0xA2);
        drain_pio_sector_after_first_word(&mut ide);

        // Second block ready → IRQ.
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16 & 0xFF, 0xA3);
        drain_pio_sector_after_first_word(&mut ide);
        assert!(!ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16 & 0xFF, 0xA4);
        drain_pio_sector_after_first_word(&mut ide);
        // Completion IRQ after final sector; alt status does not clear INTRQ.
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
    }

    /// Spec: ATA READ MULTIPLE — short final DRQ when count % multiple_count != 0.
    #[test]
    fn read_multiple_short_final_block() {
        let mut img = vec![0u8; SECTOR_SIZE * 5];
        for (i, chunk) in img.chunks_mut(SECTOR_SIZE).enumerate() {
            chunk[0] = 0xB0 + i as u8;
        }
        let mut ide = IdePrimary::with_image(img);
        clear_nien(&mut ide);
        set_multiple_mode(&mut ide, 4);

        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 5);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_MULTIPLE));
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);

        for expected in [0xB0u8, 0xB1, 0xB2, 0xB3] {
            assert!(!ide.irq_line(), "no IRQ mid-block before {expected:#x}");
            assert_eq!(
                ide.port_read(IDE_PRIMARY_DATA, 2) as u16 & 0xFF,
                u16::from(expected)
            );
            drain_pio_sector_after_first_word(&mut ide);
        }
        // Final 1-sector block → IRQ then data.
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);
        assert_eq!(ide.port_read(IDE_PRIMARY_DATA, 2) as u16 & 0xFF, 0xB4);
        drain_pio_sector_after_first_word(&mut ide);
        assert!(ide.irq_line()); // completion
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
    }

    /// Spec: ATA WRITE MULTIPLE (`0xC5`) — multi-sector DRQ commits media; IRQ/block.
    #[test]
    fn write_multiple_multi_sector_drq_commits_media() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 4]);
        clear_nien(&mut ide);
        set_multiple_mode(&mut ide, 2);

        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 3);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_MULTIPLE));
        assert!(ide.irq_line());
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);

        write_sector_words(&mut ide, 0x1111, 0x2222);
        // Mid-block: still DRQ, no IRQ.
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
        write_sector_words(&mut ide, 0x3333, 0x4444);
        // Short final block ready → IRQ.
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_PRIMARY_STATUS, 1);
        write_sector_words(&mut ide, 0x5555, 0x6666);
        assert!(ide.irq_line()); // completion
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);

        assert_eq!(ide.image[0], 0x11);
        assert_eq!(ide.image[1], 0x11);
        assert_eq!(ide.image[510], 0x22);
        assert_eq!(ide.image[511], 0x22);
        assert_eq!(ide.image[SECTOR_SIZE], 0x33);
        assert_eq!(ide.image[SECTOR_SIZE + 1], 0x33);
        assert_eq!(ide.image[SECTOR_SIZE * 2], 0x55);
        assert_eq!(ide.image[SECTOR_SIZE * 2 + 1], 0x55);
    }

    /// Spec: ATA — nIEN gates INTRQ for READ MULTIPLE DRQ / completion.
    #[test]
    fn read_multiple_nien_masks_irq() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 2]);
        // Leave nIEN set (reset default).
        set_multiple_mode(&mut ide, 2);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_MULTIPLE));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA — nIEN gates INTRQ for WRITE MULTIPLE DRQ.
    #[test]
    fn write_multiple_nien_masks_irq() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE * 2]);
        set_multiple_mode(&mut ide, 2);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 2);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_MULTIPLE));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn write_multiple_absent_drive_status_zero() {
        let mut ide = IdePrimary::new();
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_WRITE_MULTIPLE));
        assert_eq!(ide.port_read(IDE_PRIMARY_STATUS, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA READ MULTIPLE OOB LBA → ERR+IDNF (error bit 4 style `0x10`).
    #[test]
    fn read_multiple_oob_sets_err() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        set_multiple_mode(&mut ide, 1);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 5);
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(ATA_CMD_READ_MULTIPLE));
        let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
        assert_ne!(st & ATA_SR_ERR, 0);
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, 0x10);
    }

    /// Spec: ATA — MEDIA LOCK/UNLOCK (`0xDE`/`0xDF`) success noop.
    #[test]
    fn media_lock_unlock_succeed_noop() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        clear_nien(&mut ide);
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
        for cmd in [ATA_CMD_MEDIA_LOCK, ATA_CMD_MEDIA_UNLOCK] {
            ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(cmd));
            assert!(ide.irq_line());
            let st = ide.port_read(IDE_PRIMARY_STATUS, 1) as u8;
            assert_eq!(st & ATA_SR_ERR, 0);
            assert_ne!(st & ATA_SR_DRDY, 0);
        }
    }

    #[test]
    fn secondary_absent_drive_status_is_zero() {
        // Spec: OSDev ATA PIO — secondary missing drive → status 0.
        let mut ide = IdeSecondary::new();
        assert!(IdeSecondary::owns_port(IDE_SECONDARY_STATUS));
        assert!(IdeSecondary::owns_port(IDE_SECONDARY_CTRL));
        assert!(!IdeSecondary::owns_port(IDE_PRIMARY_STATUS));
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8, 0);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8, 0);
    }

    #[test]
    fn secondary_identify_and_read_sectors() {
        // Spec: ATA IDENTIFY + READ on secondary ports 0x170–0x177.
        let mut sector = vec![0u8; SECTOR_SIZE];
        sector[0] = 0x11;
        sector[1] = 0x22;
        let mut ide = IdeSecondary::with_image(sector);
        ide.port_write(IDE_SECONDARY_CTRL, 1, 0); // clear nIEN
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        // Alt status does not clear IRQ15.
        assert_ne!(ide.port_read(IDE_SECONDARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_SECONDARY_STATUS, 1); // ack IRQ
        assert!(!ide.irq_line());
        for _ in 0..256 {
            let _ = ide.port_read(IDE_SECONDARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);

        ide.port_write(IDE_SECONDARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_SECONDARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_SECONDARY_LBA_LO, 1, 0);
        ide.port_write(IDE_SECONDARY_LBA_MID, 1, 0);
        ide.port_write(IDE_SECONDARY_LBA_HI, 1, 0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_READ_SECTORS));
        assert_eq!(ide.port_read(IDE_SECONDARY_DATA, 2) as u16, 0x2211);
    }

    #[test]
    fn secondary_alt_status_does_not_clear_irq() {
        // Spec: OSDev ATA PIO — alt status at 0x376 does not clear IRQ15.
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_SECONDARY_CTRL, 1, 0);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_IDENTIFY));
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_SECONDARY_CTRL, 1);
        assert!(ide.irq_line());
        let _ = ide.port_read(IDE_SECONDARY_STATUS, 1);
        assert!(!ide.irq_line());
    }

    fn write_sector_words_secondary(ide: &mut IdeSecondary, first: u16, last: u16) {
        ide.port_write(IDE_SECONDARY_DATA, 2, u32::from(first));
        for _ in 1..255 {
            ide.port_write(IDE_SECONDARY_DATA, 2, 0);
        }
        ide.port_write(IDE_SECONDARY_DATA, 2, u32::from(last));
    }

    /// Spec: ATA/ATAPI Command Set — READ BUFFER (`0xE4`) on secondary ports
    /// (`0x170`–`0x177`) returns the 512-byte sector buffer via PIO (IRQ15 path).
    #[test]
    fn secondary_read_buffer_pio_returns_sector_buffer() {
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, u32::from(0xA0 | ATA_DRIVE_LBA));
        ide.port_write(IDE_SECONDARY_SECCOUNT, 1, 1);
        ide.port_write(IDE_SECONDARY_LBA_LO, 1, 0);
        ide.port_write(IDE_SECONDARY_LBA_MID, 1, 0);
        ide.port_write(IDE_SECONDARY_LBA_HI, 1, 0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_WRITE_SECTORS));
        write_sector_words_secondary(&mut ide, 0x55AA, 0xC300);

        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        let st = ide.port_read(IDE_SECONDARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_SECONDARY_ERROR, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_SECONDARY_DATA, 2) as u16, 0x55AA);
        for _ in 1..255 {
            let _ = ide.port_read(IDE_SECONDARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_SECONDARY_DATA, 2) as u16, 0xC300);
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn secondary_read_buffer_zeros_before_any_transfer() {
        // Spec: ATA READ BUFFER — sector buffer cleared after reset; 256 zero words.
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_ne!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        for _ in 0..256 {
            assert_eq!(ide.port_read(IDE_SECONDARY_DATA, 2) as u16, 0);
        }
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn secondary_read_buffer_asserts_irq15_on_drq_when_nien_clear() {
        // Spec: ATA / OSDev PIO — secondary INTRQ → IRQ15 when DRQ ready if nIEN=0.
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_SECONDARY_CTRL, 1, 0);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_ne!(ide.port_read(IDE_SECONDARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(ide.irq_line());
    }

    #[test]
    fn secondary_read_buffer_nien_masks_irq15_on_drq() {
        // Spec: ATA device control — nIEN=1 masks INTRQ/IRQ15 during READ BUFFER DRQ.
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_SECONDARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_ne!(ide.port_read(IDE_SECONDARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn secondary_read_buffer_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing secondary device → status 0.
        let mut ide = IdeSecondary::new();
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_SECONDARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }

    /// Spec: ATA/ATAPI Command Set — WRITE BUFFER (`0xE8`) on secondary ports
    /// (`0x170`–`0x177`) accepts 512 bytes via host→device DRQ PIO into the
    /// device sector buffer (no LBA / no media write); INTRQ → IRQ15.
    #[test]
    fn secondary_write_buffer_pio_fills_sector_buffer() {
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        let image_before = ide.inner.image.clone();
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        let st = ide.port_read(IDE_SECONDARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRQ, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        assert_eq!(ide.port_read(IDE_SECONDARY_ERROR, 1) as u8, 0);
        write_sector_words_secondary(&mut ide, 0x55AA, 0xC300);
        let st = ide.port_read(IDE_SECONDARY_STATUS, 1) as u8;
        assert_eq!(st & ATA_SR_DRQ, 0);
        assert_eq!(st & ATA_SR_ERR, 0);
        assert_ne!(st & ATA_SR_DRDY, 0);
        // Spec: WRITE BUFFER updates the sector buffer only — not backing media.
        assert_eq!(ide.inner.image, image_before);
    }

    /// Spec: ATA WRITE BUFFER then READ BUFFER on secondary — round-trip via
    /// shared sector buffer (IRQ15 path).
    #[test]
    fn secondary_write_buffer_read_buffer_round_trip() {
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        assert_ne!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        write_sector_words_secondary(&mut ide, 0x55AA, 0xC300);

        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_READ_BUFFER));
        assert_ne!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
        assert_eq!(ide.port_read(IDE_SECONDARY_DATA, 2) as u16, 0x55AA);
        for _ in 1..255 {
            let _ = ide.port_read(IDE_SECONDARY_DATA, 2);
        }
        assert_eq!(ide.port_read(IDE_SECONDARY_DATA, 2) as u16, 0xC300);
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8 & ATA_SR_DRQ, 0);
    }

    #[test]
    fn secondary_write_buffer_asserts_irq15_on_drq_and_complete_when_nien_clear() {
        // Spec: ATA / OSDev PIO WRITE — secondary INTRQ → IRQ15 at DRQ and again
        // at command complete when nIEN=0.
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_SECONDARY_CTRL, 1, 0);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        assert!(ide.irq_line());
        assert_ne!(ide.port_read(IDE_SECONDARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        let _ = ide.port_read(IDE_SECONDARY_STATUS, 1); // ack DRQ IRQ
        assert!(!ide.irq_line());
        write_sector_words_secondary(&mut ide, 0xBEEF, 0);
        assert!(ide.irq_line());
        assert_eq!(ide.port_read(IDE_SECONDARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        let _ = ide.port_read(IDE_SECONDARY_STATUS, 1);
        assert!(!ide.irq_line());
    }

    #[test]
    fn secondary_write_buffer_nien_masks_irq15_on_drq() {
        // Spec: ATA device control — nIEN=1 masks INTRQ/IRQ15 during WRITE BUFFER DRQ.
        let mut ide = IdeSecondary::with_image(vec![0u8; SECTOR_SIZE]);
        ide.port_write(IDE_SECONDARY_CTRL, 1, u32::from(ATA_DC_NIEN));
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        assert_ne!(ide.port_read(IDE_SECONDARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
        assert!(!ide.irq_line());
    }

    #[test]
    fn secondary_write_buffer_absent_drive_status_zero() {
        // Spec: OSDev ATA PIO — missing secondary device → status 0.
        let mut ide = IdeSecondary::new();
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        ide.port_write(IDE_SECONDARY_STATUS, 1, u32::from(ATA_CMD_WRITE_BUFFER));
        assert_eq!(ide.port_read(IDE_SECONDARY_STATUS, 1) as u8, 0);
        assert_eq!(ide.port_read(IDE_SECONDARY_ERROR, 1) as u8, 0);
        assert!(!ide.irq_line());
    }
}

/// ATAPI detection: the ATA/ATAPI-6 §9.12 PACKET signature and IDENTIFY PACKET
/// DEVICE on a configured packet device (M2 round 3, slice 4).
///
/// This device implements the PACKET Command feature set only far enough to be
/// *detected*. PACKET (`0xA0`) is still aborted and no command packet set is
/// implemented, so these tests also pin what the device refuses to do.
///
/// The `ATAPI_SIGNATURE_*` constants are re-exported from
/// `crates/devices/src/lib.rs` as of round-3 integration, so these could move to
/// `crates/devices/tests/`. They stay here because they assert the device's
/// internal configuration (`present`, `is_packet_device`) alongside its port
/// behavior, which is a device-level rather than integration-level concern.
#[cfg(test)]
mod atapi_detection_tests {
    use super::*;

    fn clear_nien(ide: &mut IdePrimary) {
        ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
    }

    fn select_device0(ide: &mut IdePrimary) {
        ide.port_write(IDE_PRIMARY_DRIVE, 1, 0xA0);
    }

    fn command(ide: &mut IdePrimary, cmd: u8) {
        ide.port_write(IDE_PRIMARY_STATUS, 1, u32::from(cmd));
    }

    /// `(Sector Count, LBA Low, LBA Mid, LBA High)` as the host reads them.
    fn signature(ide: &mut IdePrimary) -> (u8, u8, u8, u8) {
        (
            ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8,
            ide.port_read(IDE_PRIMARY_LBA_LO, 1) as u8,
            ide.port_read(IDE_PRIMARY_LBA_MID, 1) as u8,
            ide.port_read(IDE_PRIMARY_LBA_HI, 1) as u8,
        )
    }

    fn identify_word(pio: &[u8; SECTOR_SIZE], idx: usize) -> u16 {
        let off = idx * 2;
        u16::from(pio[off]) | (u16::from(pio[off + 1]) << 8)
    }

    /// Drain a 256-word PIO data-in block through the Data port.
    fn drain_pio(ide: &mut IdePrimary) -> [u8; SECTOR_SIZE] {
        let mut buf = [0u8; SECTOR_SIZE];
        for pair in buf.chunks_mut(2) {
            let word = ide.port_read(IDE_PRIMARY_DATA, 2) as u16;
            pair[0] = (word & 0xFF) as u8;
            pair[1] = (word >> 8) as u8;
        }
        buf
    }

    /// A channel with no packet device configured keeps exactly the behavior
    /// this tree already had: the non-PACKET signature and ABRT for both
    /// `0xA0` and `0xA1`.
    ///
    /// Spec: ATA/ATAPI-6 §9.12 (non-PACKET signature `01h/01h/00h/00h`);
    /// §8.16.2 ("Use prohibited for devices not implementing the PACKET
    /// Command feature set").
    #[test]
    fn a_plain_ata_master_stays_non_atapi() {
        let mut ide = IdePrimary::with_image(vec![0u8; SECTOR_SIZE]);
        assert!(!ide.is_packet_device());
        clear_nien(&mut ide);
        select_device0(&mut ide);
        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x00, 0x00));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRDY, 0);

        for cmd in [ATA_CMD_IDENTIFY_PACKET, ATA_CMD_PACKET] {
            select_device0(&mut ide);
            command(&mut ide, cmd);
            let status = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
            assert_ne!(status & ATA_SR_ERR, 0, "cmd {cmd:#04X}");
            assert_eq!(status & ATA_SR_DRQ, 0, "cmd {cmd:#04X}");
            assert_eq!(
                ide.port_read(IDE_PRIMARY_ERROR, 1) as u8,
                ATA_ER_ABRT,
                "cmd {cmd:#04X}"
            );
        }
        // The non-PACKET signature is untouched by the aborts.
        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x00, 0x00));
    }

    /// Spec: ATA/ATAPI-6 §9.12 — a device implementing the PACKET command
    /// feature set places Sector Count `01h`, LBA Low `01h`, LBA Mid `14h`,
    /// LBA High `EBh` after power-on/hardware reset. §9.11 / Figure 17: it also
    /// clears Status bits 6, 5, 4, 3, 2 and 0, so Status reads `00h`.
    #[test]
    fn a_configured_packet_device_reports_the_atapi_signature() {
        let mut ide = IdePrimary::with_atapi_device();
        assert!(ide.is_packet_device());
        assert!(ide.present);
        clear_nien(&mut ide);
        select_device0(&mut ide);

        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x14, 0xEB));
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8, 0x00);
        assert_eq!(
            ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRDY,
            0,
            "a PACKET device does not report DRDY after reset"
        );
    }

    /// An empty channel and a packet device both read Status `00h`; the
    /// signature is what tells them apart, which is the whole point of §9.12.
    #[test]
    fn an_empty_channel_is_distinguishable_only_by_the_signature() {
        let mut empty = IdePrimary::new();
        clear_nien(&mut empty);
        select_device0(&mut empty);
        assert_eq!(empty.port_read(IDE_PRIMARY_CTRL, 1) as u8, 0x00);
        assert_eq!(signature(&mut empty), (0x00, 0x00, 0x00, 0x00));

        let mut atapi = IdePrimary::with_atapi_device();
        clear_nien(&mut atapi);
        select_device0(&mut atapi);
        assert_eq!(atapi.port_read(IDE_PRIMARY_CTRL, 1) as u8, 0x00);
        assert_eq!(signature(&mut atapi), (0x01, 0x01, 0x14, 0xEB));
    }

    /// Spec: ATA/ATAPI-6 §9.12 — the PACKET signature is written for software
    /// reset and EXECUTE DEVICE DIAGNOSTIC as well, and §9.10 state D0ED3
    /// leaves Status `00h` on a PACKET device. §8.11 puts diagnostic code
    /// `01h` in the Error register.
    #[test]
    fn software_reset_and_diagnostic_rewrite_the_atapi_signature() {
        let mut ide = IdePrimary::with_atapi_device();
        clear_nien(&mut ide);
        select_device0(&mut ide);

        // Host writes overwrite the signature (§9.12), then SRST restores it.
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x00);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0x00);
        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x00, 0x00));

        ide.port_write(IDE_PRIMARY_CTRL, 1, u32::from(ATA_DC_SRST));
        ide.port_write(IDE_PRIMARY_CTRL, 1, 0);
        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x14, 0xEB));
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8, 0x00);

        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x00);
        command(&mut ide, ATA_CMD_DIAGNOSTIC);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_DIAG_PASSED);
        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x14, 0xEB));
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8, 0x00);
    }

    /// Spec: ATA/ATAPI-6 §6.8.1 / §8.15.5.2 — IDENTIFY DEVICE on a PACKET
    /// device "shall not be executed but shall be command aborted and shall
    /// return a signature unique to devices implementing the PACKET Command
    /// feature set".
    #[test]
    fn identify_device_is_aborted_with_the_packet_signature_in_place() {
        let mut ide = IdePrimary::with_atapi_device();
        clear_nien(&mut ide);
        select_device0(&mut ide);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x99);

        command(&mut ide, ATA_CMD_IDENTIFY);
        let status = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_ne!(status & ATA_SR_ERR, 0);
        assert_eq!(status & ATA_SR_DRQ, 0, "no 256-word transfer starts");
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x14, 0xEB));
    }

    /// Spec: ATA/ATAPI-6 §8.34.5.2 — READ SECTOR(S) on a PACKET device posts
    /// command aborted and places the signature "in the LBA High and the LBA
    /// Mid register" — those two only.
    #[test]
    fn read_sectors_is_aborted_with_the_mid_and_high_signature_only() {
        let mut ide = IdePrimary::with_atapi_device();
        clear_nien(&mut ide);
        select_device0(&mut ide);
        ide.port_write(IDE_PRIMARY_SECCOUNT, 1, 0x42);
        ide.port_write(IDE_PRIMARY_LBA_LO, 1, 0x43);
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x44);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0x45);

        command(&mut ide, ATA_CMD_READ_SECTORS);
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_ERR, 0);
        assert_eq!(ide.port_read(IDE_PRIMARY_ERROR, 1) as u8, ATA_ER_ABRT);
        assert_eq!(
            signature(&mut ide),
            (0x42, 0x43, 0x14, 0xEB),
            "Sector Count and LBA Low keep what the host wrote"
        );
    }

    /// Spec: ATA/ATAPI-6 §8.16 — IDENTIFY PACKET DEVICE is PIO data-in of 256
    /// words, is accepted regardless of DRDY (§8.16.7), and completes with
    /// DRDY set to one (§8.16.5). Every word asserted here is checked against
    /// Table 29, not against what the device happens to produce.
    #[test]
    fn identify_packet_device_returns_a_truthful_256_word_block() {
        let mut ide = IdePrimary::with_atapi_device();
        clear_nien(&mut ide);
        select_device0(&mut ide);
        // Accepted even though DRDY is clear after reset.
        assert_eq!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRDY, 0);

        command(&mut ide, ATA_CMD_IDENTIFY_PACKET);
        let status = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_ne!(status & ATA_SR_DRQ, 0, "DRQ set for the data-in block");
        assert_eq!(status & ATA_SR_ERR, 0);
        assert!(ide.irq_line(), "INTRQ asserted with nIEN clear");

        let pio = drain_pio(&mut ide);

        // Word 0: bit15 set / bit14 clear = PACKET device (§8.16.9); bits 12:8
        // = 1Fh "unknown or no device type"; bit 7 clear = non-removable;
        // bits 6:5 = 00b (3 ms DRQ); bits 1:0 = 00b (12-byte packet).
        assert_eq!(identify_word(&pio, 0), 0x9F00);
        assert_eq!(identify_word(&pio, 0) & 0xC000, 0x8000);
        assert_eq!((identify_word(&pio, 0) >> 8) & 0x1F, 0x1F);
        // Word 49 bit9 "shall be set to one"; no DMA, IORDY or overlap claimed.
        assert_eq!(identify_word(&pio, 49), 1 << 9);
        // Word 50 bit15 clear / bit14 set.
        assert_eq!(identify_word(&pio, 50), 0x4000);
        // Word 53: words (70:64) and word 88 are reported invalid — this model
        // has no transfer-timing data to put there.
        assert_eq!(identify_word(&pio, 53), 0x0000);
        assert_eq!(identify_word(&pio, 63), 0x0000, "no multiword DMA");
        assert_eq!(identify_word(&pio, 88), 0x0000, "no Ultra DMA");
        // Word 82 bit4 "shall be set to one indicating the PACKET Command
        // feature set is supported"; bit9 is now set too, because round 4
        // slice 2 implemented DEVICE RESET. Round 3 asserted bit9 clear, which
        // was truthful then and is a retired model now.
        assert_eq!(identify_word(&pio, 82), (1 << 4) | (1 << 9));
        assert_eq!(identify_word(&pio, 83), 0x4000);
        assert_eq!(identify_word(&pio, 84), 0x4000);
        assert_eq!(identify_word(&pio, 85), (1 << 4) | (1 << 9));
        assert_eq!(identify_word(&pio, 87), 0x4000);
        // Serial number is optional and "shall be zeros" when not implemented.
        assert!((10..20).all(|w| identify_word(&pio, w) == 0));
        // Model number words 27-46, ASCII byte-swapped within each word.
        let model: Vec<u8> = (27..47)
            .flat_map(|w| {
                let word = identify_word(&pio, w);
                [(word >> 8) as u8, (word & 0xFF) as u8]
            })
            .collect();
        assert_eq!(&model[..28], b"x86WASM ATAPI PACKET MINIMAL");

        // Completion: DRQ cleared, DRDY set, no error (§8.16.5).
        let status = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(status & ATA_SR_DRQ, 0);
        assert_ne!(status & ATA_SR_DRDY, 0);
        assert_eq!(status & ATA_SR_ERR, 0);
        // The signature stays readable afterwards (documented model choice).
        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x14, 0xEB));
    }

    /// nIEN still gates INTRQ for the identify transfer.
    ///
    /// Spec: ATA/ATAPI-6 §5.2.9 — "When the nIEN bit is set to one ... the
    /// INTRQ signal shall be released."
    #[test]
    fn nien_gates_intrq_on_the_identify_packet_transfer() {
        let mut ide = IdePrimary::with_atapi_device();
        select_device0(&mut ide);
        command(&mut ide, ATA_CMD_IDENTIFY_PACKET);
        assert!(!ide.irq_line(), "nIEN is set after reset");
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRQ, 0);
    }

    /// PACKET is no longer refused: round 4 slice 1 implemented the
    /// ATA/ATAPI-6 §8.21 / §9.8 protocol, so the command now enters the
    /// command-packet DRQ phase instead of aborting. Round 3's assertion that
    /// it aborts encoded a model this tree has retired; the protocol itself is
    /// covered by `tests/atapi_packet_protocol.rs`.
    #[test]
    fn packet_now_enters_the_command_packet_phase() {
        let mut ide = IdePrimary::with_atapi_device();
        clear_nien(&mut ide);
        select_device0(&mut ide);
        // Byte Count Limit; the reset signature already supplies a large one,
        // but a host normally programs it (§8.21.4).
        ide.port_write(IDE_PRIMARY_LBA_MID, 1, 0x00);
        ide.port_write(IDE_PRIMARY_LBA_HI, 1, 0x02);
        command(&mut ide, ATA_CMD_PACKET);

        let status = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
        assert_eq!(status & ATA_SR_ERR, 0);
        assert_ne!(status & ATA_SR_DRQ, 0, "the command packet is accepted");
        // Spec: §7.13 — C/D set, I/O clear: host writes a command packet.
        assert_eq!(ide.port_read(IDE_PRIMARY_SECCOUNT, 1) as u8, ATAPI_IR_CD);
    }

    /// Everything outside the six implemented commands is aborted, including
    /// the ATA commands that succeed on a disk here. DEVICE RESET (`08h`) left
    /// this list in round 4 slice 2, which implemented it.
    #[test]
    fn non_detection_commands_are_aborted_on_a_packet_device() {
        for cmd in [
            ATA_CMD_SET_FEATURES,
            ATA_CMD_NOP,
            ATA_CMD_READ_MULTIPLE,
            ATA_CMD_WRITE_SECTORS,
            ATA_CMD_READ_SECTORS_EXT,
            ATA_CMD_CHECK_POWER_MODE,
        ] {
            let mut ide = IdePrimary::with_atapi_device();
            clear_nien(&mut ide);
            select_device0(&mut ide);
            command(&mut ide, cmd);
            let status = ide.port_read(IDE_PRIMARY_CTRL, 1) as u8;
            assert_ne!(status & ATA_SR_ERR, 0, "cmd {cmd:#04X} should abort");
            assert_eq!(status & ATA_SR_DRQ, 0, "cmd {cmd:#04X}");
            assert_eq!(
                ide.port_read(IDE_PRIMARY_ERROR, 1) as u8,
                ATA_ER_ABRT,
                "cmd {cmd:#04X}"
            );
        }
    }

    /// The device type survives `reset`, and attaching a disk image turns the
    /// channel back into an ATA device with the non-PACKET signature.
    #[test]
    fn device_type_survives_reset_and_switches_with_an_image() {
        let mut ide = IdePrimary::with_atapi_device();
        ide.reset();
        assert!(ide.is_packet_device());
        clear_nien(&mut ide);
        select_device0(&mut ide);
        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x14, 0xEB));

        ide.attach_image(vec![0u8; SECTOR_SIZE]);
        assert!(!ide.is_packet_device());
        clear_nien(&mut ide);
        select_device0(&mut ide);
        assert_eq!(signature(&mut ide), (0x01, 0x01, 0x00, 0x00));
        assert_ne!(ide.port_read(IDE_PRIMARY_CTRL, 1) as u8 & ATA_SR_DRDY, 0);
    }

    /// The secondary channel exposes the same configuration.
    #[test]
    fn the_secondary_channel_can_carry_a_packet_device() {
        let mut ide = IdeSecondary::with_atapi_device();
        assert!(ide.is_packet_device());
        ide.port_write(IDE_SECONDARY_CTRL, 1, 0);
        ide.port_write(IDE_SECONDARY_DRIVE, 1, 0xA0);
        assert_eq!(ide.port_read(IDE_SECONDARY_SECCOUNT, 1) as u8, 0x01);
        assert_eq!(ide.port_read(IDE_SECONDARY_LBA_LO, 1) as u8, 0x01);
        assert_eq!(ide.port_read(IDE_SECONDARY_LBA_MID, 1) as u8, 0x14);
        assert_eq!(ide.port_read(IDE_SECONDARY_LBA_HI, 1) as u8, 0xEB);
        assert_eq!(ide.port_read(IDE_SECONDARY_CTRL, 1) as u8, 0x00);
    }
}
