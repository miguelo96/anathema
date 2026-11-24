# KDK linker port and setup stick flow

Date: 2026-11-24. Branch: `kc-linker-setup` (from `origin/main` at 9e274b8).

## Intent

The native x86_64 path (`anathema-xboot`) now boots the macOS 27 Intel kernel end to end in QEMU, and USB boot works. Today the boot stick is built by hand on a Linux box with Python (`tools/kcbuild.py`, `tools/kdk_*.py`, `tools/mkkc.sh`) and mtools. `anathema-setup` is the Windows GUI people actually run, and it cannot produce the kernel files yet.

Goal: `anathema-setup` builds a complete bootable stick on Windows with no Python and no other tools. It downloads Apple's Kernel Debug Kits on the user's machine, extracts the kernel and kexts, links the kext collection, and writes the FAT32 EFI partition with the loader and the kernel files. No Apple files ever go into this repo or the setup exe.

Two jobs, from the collaborator's handoff:

1. Rust port of `tools/kcbuild.py` (the kext collection linker).
2. Setup flow: download KDKs, extract, link, write the stick.

## Goals

- `xkc.bin` and `xkc.bin.map` from the Rust linker are byte-identical to `kcbuild.py` (with its kmod size fix, below) given the same inputs and flags, apart from one deliberate, documented difference: directory walk order.
- The Rust linker has a CLI that accepts the same flags as `kcbuild.py`, so `tools/mkkc.sh` can switch to it.
- KDK extraction runs on Windows (and macOS/Linux for testing) in pure Rust: dmg (UDIF) > HFS+ > xar > pbzx (xz) > cpio.
- The extracted files are byte-identical to what macOS sees when it mounts the same dmg and the Python scripts unpack it.
- Setup writes these files to the stick's EFI partition, matching `tools/mkdisk.sh`:
  - `EFI/BOOT/BOOTX64.EFI` (the embedded loader, as today)
  - `anathema/xkernel`, `anathema/xkc.bin`, `anathema/xcp.bin`
- Downloads are resumable and verified by sha256 before use.

## Non-goals

- The HFS+ root partition (`tools/mkroot.sh`, `mkdisk.sh` partition 2, launchd userspace). That is the collaborator's current work. The data partition stays as setup makes it today.
- Changing `anathema-xboot` or its patch tables.
- Supporting other kernel builds. The loader patches fixed addresses in one exact kernel.
- `xcp2.bin`, `xargs.txt`, `xpatch-*` flag files: not written by setup in this change (loader defaults apply). Easy to add later.
- Relocation kinds beyond the three `kcbuild.py` handles. Anything else is a hard error, as in Python.

## Anti-goals

- Committing or embedding any Apple binary, plist or kext.
- Silently producing a stick for the wrong kernel. Any mismatch must stop the install with a clear message.
- Needing Python, 7-Zip, WSL, a C toolchain or admin tools beyond what setup already uses.

## Constraints and facts found

- **The kernel.** The loader targets exactly `xnu-13432.41.4~25/RELEASE_X86_64`, Darwin 27.2.0, LC_UUID `037E7B86-0AC8-3C67-AFB8-7C2C86435E9A`. The collaborator extracted it from the 27.2 IPSW.
  - **Checked:** KDK 26B123's `System/Library/Kernels/kernel` is a thin x86_64 Mach-O with that LC_UUID. Every one of the 13 literal `expect` byte strings in xboot's patch tables matches it at its address.
  - Its sha256 is `3c9317518535791503970fbec322b9bba70e91f3a233536205ccd77b047ffc83` and it has 24,400 symbols.
  - Setup takes `xkernel` from this KDK and checks both its UUID and its sha256. No IPSW is needed.
  - The fat-slice path stays in the code for safety.
- **The kexts** come from two KDKs, as in `tools/mkkc.sh`:
  - first `--ext`: Tahoe KDK **25G342** (26.7.3), the platform drivers
  - second `--ext`: macOS 27 KDK **26B123** (27.2), used via `--prefer-last` for corecrypto, Compression, pthread, Libm, AppleEncryptedArchive, XrtHostedXnu, AppleImage4
- **Pinned downloads** (GitHub release asset digests, dortania/KdkSupportPkg):
  - `26B123/Kernel_Debug_Kit_27.2_build_26B123.dmg`, 1,251,288,981 bytes, sha256 `c582c072188a15d8dbe0b878ab84563af8fb2baa25da2ea9565140de3df2228c`
  - `25G342/Kernel_Debug_Kit_26.7.3_build_25G342.dmg`, 1,181,028,172 bytes, sha256 `219f27c7e039b6ab3426496e0d5852d55a31d00b3d3984d855cba04377c5dba0`
- **dmg format** (checked on 26B123 via its trailer): UDIF, `koly` trailer, XML resource fork with `blkx` tables; chunk types LZFSE (`0x80000007`), zero fill (`0x2`), terminator (`0xffffffff`). The partition with the files is `Apple_HFSX` (case-sensitive HFS+). The reader must also accept raw (`0x1`), ignore (`0x2`), zlib (`0x80000005`) and bzip2/ADC as errors with a clear message, in case dortania repackages.
- **Inside the HFSX volume** there is a `.pkg` (xar archive). Its TOC is zlib-compressed XML; the KDK payload is a pbzx stream of xz chunks, which decodes to an odc cpio archive (`070707` headers).
- `anathema-media` already has: `lzfse_rust`, sha256 (`sha2`), an order-preserving XML plist parser (`plist::Value::Dict` is a `Vec`), a resumable HTTP downloader with ETag/If-Range (`download.rs`, sha1 only, prints to stderr), and `ReadAt` random access over files and HTTP.
- `anathema-setup` is Windows-only (uses `std::os::windows`, `windows-sys`). It embeds the loader via `ANATHEMA_EFI` in `build.rs`. Its FAT32 writer (`fat32.rs`) builds a fixed tree with one file (`EFI/BOOT/BOOTX64.EFI`, 512-byte clusters, 200 MiB ESP). The update-boot mode writes through the ESP's drive letter.
- Disk space on the user's machine: about 2.4 GB of downloads, plus the extracted kexts and kernel. Setup must check free space before downloading.

## Approaches considered

1. **In-process Rust libraries used by setup (chosen).** Linker in a new crate. KDK download and unpack in `anathema-media`. Setup calls both. One exe to ship, testable on macOS/Linux through CLIs.
2. A separate helper exe that setup runs. Rejected: two files to ship and keep in sync, with no benefit.
3. Building the kernel files on a server and downloading them. Rejected: it redistributes Apple files.

Within approach 1:
- xz decoding uses a pure-Rust decoder, so cross-building the Windows exe needs no C toolchain. pbzx chunks are independent, so they are decoded on several threads.
- The dmg is downloaded to disk (resumable) rather than streamed, because UDIF puts its chunk table at the end and HFS+ needs random access.

## Design

### Crate `anathema-kc` (new, lib + bin)

A faithful port of `kcbuild.py`.

- Mach-O: fat slice selection (`0x01000007`), segments and sections, `LC_SYMTAB`, `LC_DYSYMTAB`, exported symbol lookup, external and local relocations.
- Linking, header fixups, `_kmod_info` fields, the three relocation kinds, the stub address for unresolved symbols, corecrypto start and kmod, all exactly as the Python does.
- Bundle discovery: walk `--ext` roots for `*.kext` with `Contents/Info.plist` or `Info.plist`; first bundle id wins; `--prefer-last`; `--override`.
- Plist output: same XML text as the Python `xml()` writer: tabs, `<integer size="64">`, negative ints wrapped to u64, only `& < >` escaped, base64 `<data>` on one line, dates as `<string>` isoformat, reals as Python `repr`.
- Output: `xkc.bin` (eight u64 header + blob) and `xkc.bin.map`. The stdout summary lines match the Python's.
- CLI flags: everything `kcbuild.py` accepts (`--kernel --ext --override --out --base --no-start --strip --set-true --rename --add-lib --codeless --prefer-last` and root ids).
- Library entry point takes a config struct and returns the bytes plus a report (counts, unresolved symbols, kexts without `_kmod_info`), so setup can log it.
- Depends on `anathema-media` for plist parsing only.

**Bug fix (owner's decision): kmod size.** In `kcbuild.py`'s relocation and header loop, `size` is left over from the link loop, so it holds the *last* kext's size. Every kext's `kmod_info.size` and its `__LINKEDIT` vmaddr (`load + size`) get that value. Both the Rust port and `kcbuild.py` use the kext's own `e["size"]`. The Python fix is a separate commit the collaborator can review or revert, and parity is tested against the fixed Python. The QEMU boot test confirms the kernel still gets as far as it did before.

**Deliberate difference: walk order.** Python uses `os.walk`, whose order depends on the filesystem (hash order on ext4). Rust walks in sorted byte order of names. This only affects (a) which bundle wins when two bundles share an id within the `--ext` set and (b) the plist position of codeless pseudo-kexts (`OSKernelResource`). Link order and every address come from the root closure, which does not depend on walk order. Parity tests run the Python with a sorted walk.

**Checked:**
- 25G342 has 904 kexts and 26B123 has 871.
- All of their Info.plists are XML.
- No bundle id appears twice within a KDK.
- No `.kext` lies outside `System/Library/Extensions`.

So in practice walk order only moves pseudo-kexts within the plist. `xkc.bin` stays the same whatever the walk order, apart from the plist text.

### `anathema-media` additions: module `kdk`

- `udif`: parse the `koly` trailer and `blkx` tables; `ReadAt` over a partition with an LRU of decompressed chunks.
- `hfs`: read-only HFS+/HFSX: volume header, catalog B-tree, extents overflow, data forks. Enough to find a file by path and read it.
- `xar`: header, zlib TOC, find a file by path, `ReadAt` over its heap data (raw encoding only; anything else is an error).
- `pbzx`: chunked reader; xz chunks decoded in parallel; stored chunks passed through.
- `cpio`: odc reader that streams entries and extracts the ones a filter selects.
- `pin`: the table of pinned KDKs (build, URL, size, sha256).
- `extract(dmg, filter, out_dir, progress)`: runs the whole chain.
- The filter keeps:
  - `System/Library/Kernels/kernel`
  - regular files under `System/Library/Extensions/` that are inside a `.kext` and not inside a `.dSYM`
- Output paths keep the KDK layout, so `kcbuild.py` can run on the same folder.
- Downloads: generalize `download.rs` so it takes the expected sha256 and a progress callback (the existing CLI keeps its sha1 behavior and stderr output).
- CLI: `anathema-media kdk fetch <build> [--out DIR]` and `anathema-media kdk extract <dmg> <out>`, for testing and for the Linux workflow.

### `anathema-setup` changes

- **The loader is xboot (owner's decision).** `BOOTX64.EFI` (and the `EFI\anathema\anathema.efi` copy that update-boot already writes) becomes `anathema-xboot.efi`: `ANATHEMA_EFI` points at the xboot build. The arm64 translator path leaves the shipped setup; someone can still build an arm setup exe by pointing `ANATHEMA_EFI` at `anathema-boot`.
- **Text changes that follow from that:**
  - The welcome line becomes "boot macOS on this pc" instead of "boot arm macOS on this pc".
  - The review page stops promising "nothing is downloaded".
  - The data partition is left exactly as setup makes it today.
- A new "kernel files" phase before any disk writes, in both install modes:
  1. Check free space on the cache drive.
  2. Download both KDKs to `%LOCALAPPDATA%\Anathema\kdk\` (resumable, sha256 checked).
  3. Extract each to its own folder.
  4. Thin and check the kernel UUID.
  5. Run the linker with the `mkkc.sh` options.
  6. Hold `xkernel` and `xkc.bin` in memory or in the cache folder.
  - A marker file per KDK records a finished extraction, so a second run skips steps 2 and 3.
  - Downloaded dmgs are deleted after a verified extraction, to give the space back.
- The `mkkc.sh` options live in one Rust constant (roots, prefer-last list, strip, codeless, base `0xffffff8002000000`), so they are easy to keep in step with the shell script.
- `xcp.bin` is our own code, built from `tools/commpage_stub.c`. It is embedded in the exe from a committed binary, with a script that rebuilds it.
- `fat32.rs` becomes a general builder for a small file tree:
  - directories `EFI`, `EFI/BOOT`, `anathema`
  - files placed in contiguous clusters
  - upper-case 8.3 names (UEFI's FAT driver is case-insensitive)
  - a space check before writing
- Fresh install writes the whole tree in one pass. Update-boot also writes `anathema\xkernel`, `xkc.bin` and `xcp.bin` through the drive letter, with the same stage, rename and verify pattern used for `BOOTX64.EFI`.
- Review page: says what will be downloaded (about 2.4 GB, from dortania's GitHub releases), and lists the extra files that will be written.
- Dry run downloads and builds nothing; it only reports what would happen.

### Verification plan

1. On macOS: mount each dmg with `hdiutil`, run `tools/kdk_xar_extract.py`, `kdk_pbzx.py` (or `pbzxcat.py`) and `kdk_cpio_extract.py` to make reference trees. The Rust `kdk extract` output must be byte-identical for every file the filter selects.
2. Run `kcbuild.py` (with a sorted walk) using the `mkkc.sh` flags on the reference trees. Run `anathema-kc` with the same flags on the Rust trees. `xkc.bin`, `xkc.bin.map` and stdout must match byte for byte.
3. Check the kernel's LC_UUID is `037E7B86-0AC8-3C67-AFB8-7C2C86435E9A`.
4. Build the ESP image with the new `fat32.rs` (through a test hook that writes to a file), check it with `fsck_msdos`, then mount it and compare files.
5. Boot that image in QEMU (`qemu-system-x86_64`, edk2, USB storage, as in `tools/xrun_usb.sh`) with the xboot loader, and compare the serial log with the collaborator's expected progress (kernel start, kext block installed, and on to the root mount wait).
6. Write the full GPT + ESP image to the spare SD card (`disk6`, 8 GB SanDisk, owner approved) and check that macOS reads it.
7. `cargo check --target x86_64-pc-windows-msvc -p anathema-setup`, plus unit tests for each format reader (small synthetic inputs, no Apple files in tests).

What this does not cover: running setup itself on Windows against a real USB stick, and booting real PC hardware. Both stay manual.

## Decided by the owner

- Approach 1, pure Rust, in-process.
- KDK pins: 25G342 for the platform drivers, a 27.x KDK for corecrypto and the others (26B123 chosen, matching the kernel's xnu build).
- The kernel must be xnu-13432.41.4~25 RELEASE_X86_64, UUID above.
- The stick boots xboot only. The arm path leaves the shipped setup.
- The kmod size bug is fixed in both the Rust port and `kcbuild.py`.
- The spare SD card may be erased for testing.

## Left to the builder

- Module and function names, error types (follow `anyhow` in media, `String` errors in setup as today).
- Chunk cache size, thread count for xz.
- How progress is split between download, extract and link in the GUI.
- Exact wording of new GUI text (follow the existing tone).

## Open items for the collaborator

- **xcp.bin build recipe.** The repo has the C source but not the compile and link command that puts `.text.a` at offset 0 and `.text.b` at 0x100. Until they send their binary or command, the builder makes one with clang and documents the command. The QEMU boot test shows whether the commpage stubs run.
- **The 27.x KDK for kdk2.** 26B123 is assumed. If they used 26B118 or 26B131, swap the pin.
- **Is their kernel the same file?** Compare the sha256 of `~/anathema-work/x86k/kernel` with the KDK kernel's (`3c93…fc83`). If the two differ (same UUID, different symbol table), kext imports may resolve differently from the build that booted.
- **Their known-good outputs.** The sha256 of their `xkc.bin` or their `xkc.bin.map`, and the `kdk_cpio_extract.py` patterns they used for `~/kdk2` and `~/kdk3`. Until these arrive, the parity test (Python vs Rust on the same trees) plus the QEMU boot are the checks.
