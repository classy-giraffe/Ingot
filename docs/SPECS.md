# Formal Design Document

**Project:** Immutable systemd-first workstation/server OS  
**Target architecture:** x86_64 UEFI  
**Image strategy:** Immutable A/B root or `/usr` payload updates via Unified Kernel Images  
**Update mechanism:** `systemd-sysupdate`  
**Userland policy:** systemd-native where possible, Rust tools where beneficial  
**Document status:** Draft 1.0  
**Intended audience:** OS architects, systems engineers, build engineers, release engineers

---

## 1. Scope

This document defines the formal design for a minimal, immutable, UEFI-only Linux operating system intended for headless servers and terminal-driven technical workstations.

The design covers:

1. Overall system philosophy
2. Core architectural model
3. Component selection
4. Storage and partition layout
5. Live ISO behavior
6. Installer behavior
7. Boot flow
8. Update and rollback behavior
9. Configuration and state model
10. Security model
11. Build and release strategy
12. Operational requirements

The document does not define desktop GUI behavior, graphical session management, or application store mechanisms.

---

## 2. Normative Language

The keywords **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**, **SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **MAY**, and **OPTIONAL** in this document are to be interpreted as descriptive requirements for system design.

---

## 3. System Goals

### 3.1 Primary goals

The operating system SHALL:

1. Boot only on x86_64 UEFI firmware.
2. Use Unified Kernel Images as the primary boot artifact.
3. Use `systemd-boot` as the bootloader.
4. Maintain an immutable A/B OS payload.
5. Separate OS payload from persistent state.
6. Use `systemd` as the primary userspace orchestrator.
7. Prefer systemd-native subsystems over independent daemons.
8. Provide a terminal-first administrative experience.
9. Use Rust-based tools where they improve safety, maintainability, or operator ergonomics.
10. Support atomic updates and automatic rollback.

### 3.2 Secondary goals

The operating system SHOULD:

1. Minimize the number of independently maintained long-running daemons.
2. Minimize mutable configuration inside the OS image.
3. Support declarative deployment and installation.
4. Support remote administration via SSH.
5. Support containerized workloads without polluting the base OS.
6. Support signed boot artifacts where Secure Boot is enabled.
7. Support TPM2-based protection where available.

---

## 4. Non-Goals

The operating system SHALL NOT:

1. Provide a graphical desktop environment.
2. Include Wayland, X11, or desktop application stacks by default.
3. Support legacy BIOS/CSM boot in the primary target configuration.
4. Treat the base OS image as a mutable package-managed system.
5. Require a traditional Linux distribution package manager for normal operation.
6. Replace `systemd` with an alternative init system.
7. Require Rust implementations where they increase complexity without clear benefit.

---

## 5. Design Philosophy

### 5.1 Immutable OS payload

The OS payload SHALL be treated as a versioned build artifact. The running system SHALL NOT mutate core OS files under normal operation.

Consequences:

1. `/usr` or the full root payload is read-only.
2. Updates replace whole images, not individual files.
3. Rollback is performed by selecting the previous boot slot.
4. System integrity is easier to reason about.

### 5.2 Explicit state separation

Persistent state SHALL be separated from OS code.

The system recognizes three major classes of data:

1. **OS payload:** immutable, versioned, replaceable
2. **System state:** persistent, host-specific, stored in `/var`
3. **User state:** persistent, user-specific, stored in `/home`

### 5.3 systemd-first userspace

The system SHALL prefer systemd-native mechanisms for core system behavior.

Preferred systemd subsystems include:

1. `systemd-journald`
2. `systemd-udevd`
3. `systemd-networkd`
4. `systemd-resolved`
5. `systemd-timesyncd`
6. `systemd-logind`
7. `systemd-homed` where applicable
8. `systemd-creds`
9. `systemd-sysext`
10. `systemd-confext`
11. `systemd-repart`
12. `systemd-sysupdate`
13. `systemd-boot`
14. `systemd-bless-boot`

### 5.4 Rust where beneficial

Rust SHALL be used where it materially improves:

1. memory safety
2. CLI ergonomics
3. maintainability
4. installer quality
5. interactive administration

Rust SHALL NOT be used solely for ideological consistency if doing so increases fragility or maintenance burden.

### 5.5 Terminal-first administration

The primary operator interface SHALL be the terminal. The system MUST provide:

1. reliable console access
2. SSH access
3. structured logging
4. strong service introspection
5. modern shell and editor tooling

---

## 6. Reference Architecture

### 6.1 Layer model

The system is organized into the following layers:

1. **Firmware layer**
   - UEFI firmware
   - Secure Boot policy, if enabled

2. **Boot layer**
   - `systemd-boot`
   - Unified Kernel Images

3. **Early boot layer**
   - Linux kernel
   - Dracut/systemd initramfs
   - early device discovery and root payload mount

4. **Core userspace layer**
   - systemd as PID 1
   - udev, journal, network, DNS, time, session, and service management

5. **Administrative layer**
   - shell environment
   - core utilities
   - editor
   - privilege escalation
   - system maintenance tooling

6. **Extension/workload layer**
   - `systemd-sysext`
   - `systemd-confext`
   - `systemd-nspawn`
   - OCI containers via systemd-integrated tooling where required

---

## 7. Component Registry

### 7.1 Core platform components

| Component | Language/Type | Role | Requirement Level |
|---|---:|---|---|
| `systemd-boot` | C | UEFI boot manager | REQUIRED |
| Linux kernel | C/asm | Kernel and EFI stub boot | REQUIRED |
| `linux-firmware` | Blobs | Device firmware and microcode | REQUIRED |
| Dracut | Shell/C | Initramfs generation | REQUIRED |
| `glibc` | C | Core C library | REQUIRED |
| `systemd` | C | Init and service orchestration | REQUIRED |
| `util-linux` | C | Low-level system utilities | REQUIRED |
| `kmod` | C | Kernel module management | REQUIRED |
| `e2fsprogs` | C | ext filesystem tooling | REQUIRED if ext4 is used |
| `dosfstools` | C | FAT filesystem tooling | REQUIRED |
| `btrfs-progs` | C | btrfs tooling | REQUIRED if btrfs is used |
| `xfsprogs` | C | XFS tooling | REQUIRED if XFS is used |
| `dbus-broker` | C | D-Bus broker | REQUIRED |
| `zstd` | C | Compression | REQUIRED |
| `libarchive` | C | Archive handling | REQUIRED |

### 7.2 Rust-leaning userland components

| Component | Language | Role | Requirement Level |
|---|---:|---|---|
| `uutils/coreutils` | Rust | Core CLI utilities | REQUIRED |
| `brush` | Rust | System shell / `/bin/sh` | REQUIRED for installed host |
| `nushell` | Rust | Interactive admin shell | REQUIRED |
| `helix` | Rust | Terminal editor | REQUIRED |
| `zellij` | Rust | Terminal multiplexer | RECOMMENDED |
| Installer | Rust | Installation orchestration | REQUIRED |

### 7.3 Authentication and privilege components

| Component | Language | Role | Requirement Level |
|---|---:|---|---|
| Linux-PAM | C | Authentication framework | REQUIRED |
| `shadow` or systemd-homed integration | C/systemd | User account management | REQUIRED |
| `run0` | C/systemd | Primary privilege escalation | RECOMMENDED |
| `sudo-rs` | Rust | Compatibility privilege tool | OPTIONAL |
| OpenSSH | C | Remote administration | REQUIRED for server profile |
| `nftables` | C | Host firewall | REQUIRED for server profile |

### 7.4 systemd-native subsystems

The following systemd subsystems SHALL be used where their functionality is required:

1. `systemd-journald` for logging
2. `systemd-udevd` for device management
3. `systemd-networkd` for networking
4. `systemd-resolved` for DNS resolution
5. `systemd-timesyncd` for time synchronization
6. `systemd-logind` for session management
7. `systemd-oomd` for cgroup-aware OOM management
8. `systemd-coredump` for crash collection
9. `systemd-creds` for service credentials
10. `systemd-sysext` for OS extension images
11. `systemd-confext` for configuration extensions
12. `systemd-nspawn` for native container workloads
13. `systemd-repart` for declarative partitioning
14. `systemd-sysupdate` for A/B updates
15. `systemd-bless-boot` for boot validation

---

## 8. Storage and Partition Layout

### 8.1 Target disk model

The installed host SHALL use a GPT partition table.

The system SHOULD follow the Discoverable Partitions Specification where practical.

### 8.2 Recommended partition layout

| Partition | Label | Mountpoint | Filesystem | Purpose | Recommended Size |
|---|---|---|---|---|---:|
| 1 | `esp` | `/efi` | FAT32 | EFI System Partition | 1–2 GiB |
| 2 | `os-slot-a` | `/usr` or `/` | erofs or ext4 | Active OS payload | 4–8 GiB |
| 3 | `os-slot-b` | inactive | erofs or ext4 | Passive OS payload | 4–8 GiB |
| 4 | `var` | `/var` | btrfs or ext4 | Persistent system state | 15+ GiB |
| 5 | `home` | `/home` | btrfs or ext4 | User state | Remainder |

### 8.3 Partition type policy

The installer SHOULD use partition types aligned with systemd discoverability:

1. ESP for `/efi`
2. root-x86-64 or usr-x86-64 for OS payload slots
3. Linux Variable Data for `/var`
4. Linux Home for `/home`

Where exact discoverability is not practical, partition labels MUST still be stable and predictable.

### 8.4 Filesystem policy

1. The ESP MUST be FAT32.
2. OS payload slots SHOULD use `erofs`.
3. `ext4` MAY be used as a fallback for OS payload slots.
4. `/var` SHOULD use `btrfs` where snapshotting is desired.
5. `/home` MAY use `btrfs` or `ext4`.
6. `/tmp` SHOULD be mounted as `tmpfs`.
7. `/run` MUST be ephemeral runtime storage.

---

## 9. Filesystem Hierarchy and State Model

### 9.1 Canonical runtime hierarchy

The recommended runtime hierarchy is:

| Path | Mutability | Purpose |
|---|---|---|
| `/usr` | Read-only | OS payload |
| `/etc` | Persistent | Host configuration |
| `/var` | Persistent | System state |
| `/home` | Persistent | User state |
| `/tmp` | Ephemeral | Temporary files |
| `/run` | Ephemeral | Runtime state |
| `/efi` | Mutable boot partition | Bootloader and UKIs |

### 9.2 Recommended deployment mode

The system SHOULD use an immutable `/usr` model with a minimal or transient root.

In this mode:

1. `/usr` is mounted from the active OS slot.
2. `/` is generated or minimized at runtime.
3. `/etc` is persisted separately, preferably under `/var`.
4. `/var` and `/home` are separate persistent partitions.

### 9.3 `/etc` persistence policy

The base OS image SHALL NOT store mutable host configuration directly inside the immutable payload.

Recommended implementation:

1. Factory defaults are stored in `/usr/share/factory/etc`.
2. Host-specific configuration is stored under `/var/lib/etc`.
3. `/etc` is exposed by bind mount or overlay backed by `/var/lib/etc`.
4. First boot initializes `/etc` from factory defaults if empty.

This provides persistent configuration while preserving an immutable OS payload.

### 9.4 `/var` policy

`/var` SHALL contain persistent system state, including:

1. machine identity
2. system journals
3. service state
4. container images and volumes
5. extension images
6. persistent configuration backing store
7. update metadata where applicable

### 9.5 `/home` policy

`/home` SHALL contain user home directories.

If `systemd-homed` is enabled:

1. user homes MAY be managed as portable records
2. homes MAY be encrypted
3. homes MAY be stored as subvolumes or LUKS-backed storage

---

## 10. Live ISO Design

### 10.1 Purpose

The live ISO SHALL provide a bootable environment for:

1. installation
2. rescue
3. diagnostics
4. hardware validation
5. optional network-assisted deployment

### 10.2 Live ISO constraints

The live ISO:

1. MUST boot on UEFI x86_64 systems
2. MUST NOT require a GUI
3. MUST reset volatile state on reboot by default
4. SHOULD contain the same core administrative userland as the installed system
5. SHOULD include the installer and deployment tooling
6. MAY include SSH for remote installation workflows

### 10.3 ISO media layout

A recommended ISO layout is:

    /
    ├── EFI/
    │   ├── BOOT/
    │   │   └── BOOTX64.EFI
    │   └── Linux/
    │       └── live.efi
    ├── LiveOS/
    │   └── rootfs.erofs
    └── loader/
        └── loader.conf

Where:

1. `BOOTX64.EFI` is the fallback UEFI binary, typically `systemd-boot`.
2. `live.efi` is the live Unified Kernel Image.
3. `rootfs.erofs` is the read-only live userspace payload.

### 10.4 Live ISO boot flow

The live ISO boot sequence SHALL be:

1. UEFI firmware loads `BOOTX64.EFI`.
2. `systemd-boot` starts.
3. `systemd-boot` loads `live.efi`.
4. The kernel and initramfs start.
5. systemd starts in the initramfs.
6. The ISO media is discovered.
7. The live root payload is mounted read-only.
8. A writable overlay or tmpfs-backed runtime is created.
9. The system switches to the live userspace.
10. The live system reaches a terminal/multi-user state.

### 10.5 Live ISO state model

The live environment MUST be ephemeral by default.

Recommended behavior:

1. Root runtime is overlay-backed or tmpfs-backed.
2. `/var` is ephemeral.
3. `/home` is ephemeral.
4. No state persists across reboot unless explicitly configured.
5. Installer logs MAY be stored temporarily in RAM.

### 10.6 Live ISO services

The live image SHOULD enable only essential services:

1. `systemd-udevd`
2. `systemd-journald`
3. `systemd-networkd`
4. `systemd-resolved`
5. getty/console access
6. optional SSH service for automated installs

The live image SHOULD NOT enable unnecessary long-running services.

---

## 11. Installer Design

### 11.1 Installer objectives

The installer SHALL:

1. prepare the target disk
2. create the required partition layout
3. deploy the OS payload to A/B slots
4. initialize persistent state
5. install the bootloader and UKIs
6. configure initial system identity and users
7. leave the system ready for first boot

### 11.2 Installer implementation policy

The installer SHOULD be implemented in Rust.

The installer SHOULD NOT reimplement low-level mechanisms where robust systemd or filesystem tools already exist.

The installer SHOULD orchestrate:

1. `systemd-repart`
2. filesystem formatting utilities
3. `systemd-firstboot`
4. `systemd-sysusers`
5. `systemd-tmpfiles`
6. `bootctl`
7. `ukify`
8. optional `systemd-cryptenroll`
9. optional `systemd-homed`

### 11.3 Installer modes

The installer MUST support at least one of:

1. interactive terminal mode
2. declarative config-file mode

A production-grade implementation SHOULD support both.

### 11.4 Installer input model

The installer SHOULD accept the following categories of input:

1. target disk
2. OS image source
3. hostname
4. timezone
5. keymap/locale
6. partition sizes
7. filesystem choices
8. encryption choices
9. initial user configuration
10. SSH authorized keys
11. service enablement policy

### 11.5 Installation phases

#### Phase 1: Validation

The installer SHALL verify:

1. UEFI mode is active
2. target disk exists
3. target disk is large enough
4. required tools are available
5. source image is present and valid

#### Phase 2: Partitioning

The installer SHALL create a GPT partition layout consistent with Section 8.

Partition creation SHOULD be performed via declarative definitions where possible.

#### Phase 3: Formatting

The installer SHALL format:

1. ESP as FAT32
2. OS slots as erofs or ext4
3. `/var` as btrfs or ext4
4. `/home` as btrfs or ext4

#### Phase 4: Optional encryption

If encryption is requested:

1. `/var` MAY be encrypted with LUKS2
2. `/home` MAY be encrypted with LUKS2
3. TPM2 enrollment MAY be used where available
4. The ESP MUST NOT be encrypted

#### Phase 5: Payload deployment

The installer SHALL write the OS payload to both slot A and slot B.

The initial installation SHOULD deploy the same known-good version to both slots.

#### Phase 6: Persistent state initialization

The installer SHALL initialize:

1. `/etc` backing store
2. machine-id
3. hostname
4. base system users
5. initial administrator account
6. SSH authorized keys if provided
7. required service enablement state

#### Phase 7: Bootloader installation

The installer SHALL:

1. install `systemd-boot` into the ESP
2. place UKIs for slot A and slot B
3. configure default boot entry
4. configure boot attempt policy where supported

#### Phase 8: Finalization

The installer SHALL:

1. synchronize filesystem buffers
2. unmount all target filesystems
3. remove temporary secrets
4. report success or failure
5. optionally reboot

### 11.6 Installer safety requirements

The installer MUST:

1. clearly warn before destructive disk operations
2. require explicit confirmation in interactive mode
3. support dry-run where practical
4. log actions to a persistent or retrievable location
5. fail safely on unexpected errors

---

## 12. Installed Boot Flow

### 12.1 Normal boot sequence

The installed system SHALL boot in the following order:

1. UEFI firmware initializes hardware.
2. UEFI loads `systemd-boot`.
3. `systemd-boot` selects a UKI entry.
4. The UKI starts the kernel and embedded initramfs.
5. systemd starts in initrd.
6. Early device enumeration occurs.
7. The OS payload partition is discovered.
8. The payload is mounted.
9. The initrd transitions to the real userspace.
10. systemd becomes PID 1.
11. Core system services start.
12. The system reaches multi-user operation.
13. Boot success is recorded if validation passes.

### 12.2 Boot entry model

Each OS slot SHOULD have a corresponding UKI in the ESP:

1. `os-slot-a.efi`
2. `os-slot-b.efi`

Each UKI SHOULD embed:

1. kernel
2. initramfs
3. command line
4. CPU microcode
5. OS release metadata
6. optional signature

### 12.3 Slot selection

The bootloader SHALL select the default slot based on:

1. configured default entry
2. boot attempt counters
3. successful boot state
4. fallback policy

### 12.4 Boot validation

The system SHOULD use `systemd-bless-boot` to mark a boot as successful after critical services have started.

A successful boot SHOULD:

1. reset boot attempt counters
2. mark the slot as valid
3. permit normal future selection

A failed boot SHOULD:

1. trigger fallback where supported
2. preserve previous good slot
3. record diagnostic information where possible

---

## 13. Update and Rollback Model

### 13.1 Update strategy

The system SHALL use atomic A/B updates.

The active slot SHALL remain in service while the inactive slot is updated.

### 13.2 Update mechanism

The primary update mechanism SHALL be `systemd-sysupdate`.

The update process SHOULD include:

1. fetching update metadata
2. verifying integrity/authenticity
3. writing the new payload to the inactive slot
4. updating UKI artifacts in the ESP
5. updating boot selection state
6. rebooting into the new slot

### 13.3 Update artifacts

An update bundle SHOULD include:

1. OS payload image
2. UKI or UKI components
3. manifest
4. checksums
5. optional signature metadata

### 13.4 Rollback model

Rollback SHALL be possible by selecting the previous slot.

Rollback MAY be:

1. automatic via boot counters
2. manual via bootloader selection
3. scripted via administrative tooling

### 13.5 State compatibility

Because `/var` persists across slot switches, service authors MUST consider state compatibility between OS versions.

The system design SHOULD account for:

1. forward migrations
2. rollback safety
3. versioned state formats
4. idempotent first-boot migrations

---

## 14. Configuration Management

### 14.1 Configuration principles

1. The OS image MUST remain immutable.
2. Host configuration MUST persist independently of OS payload updates.
3. Configuration SHOULD be declarative where possible.
4. Runtime-generated state SHOULD remain separated from authored configuration.

### 14.2 Recommended `/etc` implementation

The system SHOULD implement `/etc` using one of the following:

1. bind mount from `/var/lib/etc`
2. overlayfs with lower factory defaults and upper persistent state

### 14.3 Factory defaults

Factory defaults SHOULD be shipped in:

1. `/usr/share/factory/etc`
2. `/usr/share/factory/var` where appropriate

First boot SHOULD materialize required state from these defaults.

### 14.4 Machine identity

The system MUST maintain a stable machine identity.

The machine-id SHALL be generated during installation or first boot and persisted appropriately.

---

## 15. System Extensions

### 15.1 Purpose

System extensions allow additional software to be added without mutating the base OS image.

### 15.2 Extension mechanisms

The system SHALL support:

1. `systemd-sysext` for `/usr` extensions
2. `systemd-confext` for configuration extensions where useful

### 15.3 Extension policy

Extensions SHOULD be used for:

1. diagnostic tools
2. administrative utilities
3. language runtimes
4. optional packages
5. site-specific tooling

Extensions SHOULD NOT be used to bypass the immutable base image model.

### 15.4 Extension storage

Extension images SHOULD be stored under:

1. `/var/lib/extensions`
2. `/etc/extensions` where appropriate

The system SHOULD provide tooling to list, refresh, enable, and disable extensions.

---

## 16. Privilege and User Model

### 16.1 User management

The system SHALL support:

1. root account control
2. administrative user creation
3. SSH key enrollment
4. service/system users via `systemd-sysusers`

### 16.2 Optional `systemd-homed`

The system MAY use `systemd-homed` for user management.

If enabled:

1. user homes SHOULD be portable
2. home encryption SHOULD be supported
3. user records SHOULD be recoverable according to policy

### 16.3 Privilege escalation

The preferred privilege escalation model is:

1. `run0` where systemd version supports it
2. optional `sudo-rs` for compatibility only

The system SHOULD minimize reliance on SUID binaries.

### 16.4 Shell assignment

The installed host SHALL provide:

1. `/bin/sh` implemented by `brush`
2. default interactive shell implemented by `nushell`

The initramfs MAY use `brush` if validated, or a minimal POSIX shell if maximum early-boot compatibility is required.

---

## 17. Security Model

### 17.1 Baseline security requirements

The system MUST provide:

1. UEFI boot
2. immutable OS payload
3. separated persistent state
4. service isolation via systemd units
5. structured auditability via journal logging
6. minimal attack surface in the base image

### 17.2 Secure Boot

The system SHOULD support Secure Boot by signing boot artifacts.

Signed artifacts MAY include:

1. bootloader binary
2. UKI binaries
3. optional extension images

### 17.3 Measured boot and TPM2

Where TPM2 is available, the system MAY support:

1. PCR measurement of boot phases
2. credential sealing
3. LUKS key protection
4. boot-state attestation

### 17.4 Disk encryption

The system MAY support encryption of:

1. `/var`
2. `/home`
3. user home storage

The ESP MUST remain unencrypted.

### 17.5 Credential management

Sensitive service credentials SHOULD NOT be stored in plaintext where avoidable.

The system SHOULD support `systemd-creds` for encrypted credential injection into services.

### 17.6 Remote access security

The server profile MUST support SSH.

Default SSH policy SHOULD include:

1. key-based authentication
2. no direct root password login
3. restricted administrative access
4. firewall filtering via `nftables`

---

## 18. Networking and Core Services

### 18.1 Network stack

The system SHALL use:

1. `systemd-networkd` for interface configuration
2. `systemd-resolved` for DNS resolution

The system MAY support:

1. DHCP
2. static addressing
3. VLANs
4. bridges
5. bonding
6. route policy configuration

### 18.2 Time synchronization

The system SHALL use `systemd-timesyncd` by default.

Alternative time services MAY be supported by extension or configuration if required.

### 18.3 Logging

The system SHALL use `systemd-journald`.

Persistent logs SHOULD be stored under `/var/log/journal`.

### 18.4 Firewall

The system SHALL use `nftables` for packet filtering in server deployments.

Firewall policy SHOULD be declarative and loaded at boot.

### 18.5 Remote administration

The server profile SHALL include OpenSSH.

The workstation profile MAY include OpenSSH depending on deployment policy.

---

## 19. Workload and Container Policy

### 19.1 Base image policy

The base image SHALL remain minimal.

Applications and services SHOULD NOT be installed by mutating the base OS payload.

### 19.2 Supported workload mechanisms

The system SHOULD support:

1. native systemd services
2. `systemd-nspawn` containers
3. OCI containers via systemd-integrated tooling
4. extension images for host-level utilities

### 19.3 Preferred container integration

For OCI workloads, the system SHOULD prefer declarative systemd integration, such as Quadlet-style unit definitions, where available.

This ensures containers are managed as first-class systemd services.

### 19.4 Workload isolation

Workloads SHOULD:

1. run with cgroup resource control
2. be observable through journald and systemd status
3. be restartable via systemd policies
4. avoid writing into OS payload directories

---

## 20. Build System

### 20.1 Image builder

The system SHALL use `mkosi` as the primary image builder.

`mkosi` SHALL be responsible for producing:

1. base OS images
2. live ISO images
3. system extension images
4. UKI bundles where integrated
5. update artifacts where applicable

### 20.2 Build inputs

Build definitions SHOULD specify:

1. package set
2. filesystem layout
3. enable/disable service policy
4. kernel configuration expectations
5. initramfs content
6. image compression
7. output formats

### 20.3 Build outputs

A release build SHOULD produce:

1. OS payload image
2. live ISO
3. UKI artifacts
4. manifest
5. checksums
6. optional signatures

### 20.4 Reproducibility

The build system SHOULD aim for reproducible outputs by:

1. pinning versions
2. controlling build environment
3. minimizing nondeterministic content
4. recording build metadata

---

## 21. Profiles

### 21.1 Server profile

The server profile SHALL include:

1. SSH enabled
2. firewall enabled
3. persistent journal
4. networkd/resolved enabled
5. minimal user tooling
6. container support available

### 21.2 Workstation profile

The workstation profile SHALL include:

1. local console access
2. modern shell environment
3. editor and multiplexer
4. larger home storage
5. optional SSH
6. optional `systemd-homed`

### 21.3 Appliance/minimal profile

The appliance profile SHALL include:

1. minimal package set
2. minimal persistent state
3. only required services enabled
4. optimized for fixed-function deployment

---

## 22. Operational Model

### 22.1 Administration interface

Primary administration SHALL occur via:

1. local console
2. SSH
3. systemd tooling
4. journal inspection
5. shell utilities

### 22.2 Service management

All long-running services MUST be managed by systemd units.

Services SHOULD define:

1. dependencies
2. restart policy
3. resource limits
4. sandboxing where appropriate
5. logging behavior

### 22.3 Update administration

The system SHALL provide simple administrative operations for:

1. listing updates
2. applying updates
3. checking boot status
4. selecting boot slots
5. performing rollback

### 22.4 Extension administration

The system SHALL provide mechanisms to:

1. list extensions
2. refresh extension state
3. enable/disable extensions
4. inspect extension content

---

## 23. Testing and Validation

### 23.1 Required test areas

The project MUST validate:

1. UEFI boot
2. live ISO boot
3. installation on bare metal
4. installation on virtual machines
5. A/B update flow
6. rollback flow
7. persistent state survival
8. SSH access
9. service startup correctness
10. filesystem integrity

### 23.2 Boot validation

Test criteria:

1. system reaches multi-user target
2. expected services are active
3. journal contains no fatal errors
4. boot is marked successful

### 23.3 Update validation

Test criteria:

1. inactive slot is correctly written
2. new UKI is installed
3. next boot selects new slot
4. successful boot marks new slot valid
5. failed boot falls back where configured

### 23.4 State validation

Test criteria:

1. `/var` persists across updates
2. `/home` persists across updates
3. `/etc` changes persist
4. logs persist according to policy
5. machine identity remains stable

---

## 24. Risks and Mitigations

### 24.1 Shell compatibility risk

**Risk:** System scripts may rely on subtle shell behavior.

**Mitigation:**
1. validate `brush` against actual system scripts
2. prefer systemd unit directives over shell indirection where possible
3. optionally use a minimal POSIX shell inside initramfs if required

### 24.2 State migration risk

**Risk:** Persistent `/var` state may become incompatible across OS versions.

**Mitigation:**
1. design state formats for version tolerance
2. use explicit migrations
3. test rollback paths
4. avoid fragile global state where possible

### 24.3 Extension sprawl risk

**Risk:** extensions may become an uncontrolled package system.

**Mitigation:**
1. define extension governance
2. sign extension images
3. limit official extension scope
4. document base vs extension boundaries

### 24.4 Firmware coverage risk

**Risk:** pruned firmware may omit required device blobs.

**Mitigation:**
1. maintain profile-based firmware sets
2. include broad virtualization/server firmware by default
3. allow optional firmware extensions

### 24.5 Bootloader complexity risk

**Risk:** A/B boot selection logic may become fragile.

**Mitigation:**
1. use standard `systemd-boot` mechanisms
2. avoid custom boot hacks
3. test boot counters and fallback thoroughly

---

## 25. Acceptance Criteria

A conforming implementation MUST satisfy the following:

1. Boots via UEFI on x86_64.
2. Boots from a Unified Kernel Image.
3. Uses `systemd` as PID 1.
4. Maintains immutable OS payload slots.
5. Supports A/B updates.
6. Supports rollback to a previous slot.
7. Persists `/var` and `/home` across updates.
8. Provides SSH administration in server profile.
9. Provides `brush` as `/bin/sh` on installed host.
10. Provides `nushell` as the default interactive shell.
11. Provides a working installer.
12. Produces a working live ISO.
13. Uses systemd-native subsystems for core functionality.
14. Avoids GUI components by default.
15. Builds via `mkosi`.

---

## 26. Recommended Implementation Order

A practical implementation order is:

1. Define base `mkosi` image with systemd and minimal toolset.
2. Integrate `uutils`, `brush`, `nushell`, and `helix`.
3. Build live ISO with ephemeral overlay.
4. Implement installer partitioning and payload deployment.
5. Integrate UKI generation and `systemd-boot`.
6. Implement persistent `/etc`, `/var`, and `/home` handling.
7. Validate installed boot flow.
8. Implement `systemd-sysupdate` A/B update flow.
9. Add rollback and boot validation testing.
10. Add extensions, SSH, firewall, and container integration.

---

## 27. Summary

This operating system is designed as a minimal, immutable, systemd-native Linux platform for servers and terminal-first workstations. It uses UEFI, Unified Kernel Images, `systemd-boot`, and A/B payload slots to achieve atomic updates and rollback safety. Persistent state is isolated from the OS image. The userland is modern and Rust-leaning, but the core system plumbing remains firmly grounded in systemd where it is strongest.

The resulting design is deliberately constrained:

1. no GUI
2. no mutable base image
3. no unnecessary daemon sprawl
4. strong separation of code, configuration, and state
5. first-class terminal administration
6. robust update and rollback behavior

This produces a system that is operationally predictable, secure by architecture, and maintainable over the long term.
