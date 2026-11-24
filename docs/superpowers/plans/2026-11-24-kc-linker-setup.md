# KDK Linker Port and Setup Stick Flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `anathema-setup` builds a complete xboot stick on Windows (`EFI/BOOT/BOOTX64.EFI` + `anathema/xkernel`, `xkc.bin`, `xcp.bin`) from KDKs it downloads itself, with a pure-Rust port of `tools/kcbuild.py`.

**Architecture:** New crate `anathema-kc` (Mach-O + linker + Python-compatible plist writer, lib + CLI). New module `anathema-media::kdk` (pinned downloads, UDIF > HFS+ > xar > pbzx > cpio unpack, lib + CLI subcommands). `anathema-setup` gains a platform-neutral lib (`fat32`, `kernel_files`) used by its Windows GUI and by a cross-platform dev bin `anathema-esp` that writes an ESP image for QEMU and SD card tests.

**Tech Stack:** Rust 2021, `anyhow`, `lzfse_rust` (already in media), `flate2` (already), `sha2` (already), `lzma-rs = "0.3"` (new, pure Rust xz), `reqwest` blocking (already). Python 3 + `hdiutil` only for reference outputs and test fixtures on macOS.

**Spec:** `docs/superpowers/specs/2026-11-24-kc-linker-setup-design.md`

## Global Constraints

- No Apple file (kernel, kext, plist, dmg, pkg) is committed or embedded. Tests build their fixtures at run time; tests that need real KDKs read paths from env vars and skip when unset.
- Pins, verbatim:
  - `https://github.com/dortania/KdkSupportPkg/releases/download/25G342/Kernel_Debug_Kit_26.7.3_build_25G342.dmg`, size 1181028172, sha256 `219f27c7e039b6ab3426496e0d5852d55a31d00b3d3984d855cba04377c5dba0`
  - `https://github.com/dortania/KdkSupportPkg/releases/download/26B131/Kernel_Debug_Kit_27.2.1_build_26B131.dmg`, size 1251401698, sha256 `287377a4c52ae3ad04476a35195fc143129a18a2d34cd585cbec7b35cfc42c81`
  - Kernel: `System/Library/Kernels/kernel` from 26B131, LC_UUID `037E7B86-0AC8-3C67-AFB8-7C2C86435E9A`, sha256 `3c9317518535791503970fbec322b9bba70e91f3a233536205ccd77b047ffc83`, size 21128776.
- Linker constants from `kcbuild.py`: page `0x4000`, `PLIST_RESERVE = 0x80000`, magic `0x434b58414e41`, `MH_DYLIB_IN_CACHE = 0x80000000`, x86_64 cputype `0x01000007`, spare tail `0x2000`.
- The setup link uses the `tools/mkkc.sh` options exactly:
  - `--ext` 25G342 tree first, then the 26B131 tree
  - `--prefer-last` for `com.apple.kec.corecrypto`, `com.apple.kec.Compression`, `com.apple.kec.pthread`, `com.apple.kec.Libm`, `com.apple.kec.AppleEncryptedArchive`, `com.apple.kec.XrtHostedXnu`, `com.apple.security.AppleImage4`
  - `--strip com.apple.security.AppleImage4:AppleSecurityExtension` and `--strip com.apple.kext.CoreTrust:AppleSecurityExtension`
  - `--codeless com.apple.driver.AppleMobileFileIntegrity`
  - `--base 0xffffff8002000000`
  - roots, in order: `com.apple.driver.AppleACPIPlatform com.apple.kec.corecrypto com.apple.kec.AppleEncryptedArchive com.apple.kec.XrtHostedXnu com.apple.kec.Libm com.apple.kec.Compression com.apple.kec.pthread com.apple.security.AppleImage4 com.apple.kext.CoreTrust`, then every line of `tools/kc-roots.txt` in file order (as `mkkc.sh $(cat tools/kc-roots.txt)`; see ledger ruling)
- ESP file paths: `EFI/BOOT/BOOTX64.EFI`, `anathema/xkernel`, `anathema/xkc.bin`, `anathema/xcp.bin`. Update-boot keeps also writing `EFI/anathema/anathema.efi`.
- Pure-Rust dependencies only (the Windows exe is cross-built; no C toolchain).
- Error types: `anyhow::Result` in `anathema-media` and `anathema-kc`; `Result<_, String>` in `anathema-setup`, as today.
- `cargo check --target x86_64-pc-windows-msvc -p anathema-setup` must pass. Everything else is tested on macOS with `cargo test`.
- Walk order is sorted by file-name bytes, depth-first, directories not followed through symlinks.
- Reference trees for parity live outside the repo (paths below are the execution machine's scratchpad; pass them through env vars).

## Review Focus

- **Corrupt or partial download on disk** (killed setup, disk error, wrong file at the cache path): it must never be used. A sha256 mismatch deletes it and fails with a message naming the file. Test: Task 7, `verify_or_discard_rejects_corrupt_file`.
- **Not enough free space on the cache drive** (about 3.5 GB peak): stop before downloading, saying how much is needed and how much is free. Test: Task 9, `space_check_reports_need_and_free`.
- **Existing stick's ESP too small or too full** for about 45 MB of kernel files (update-boot): fail before writing any file, so a stick is never half-updated. Test: Task 9, `update_plan_rejects_full_esp`.
- **Kext plists with unusual XML** (hex `<integer>`, CRLF, duplicate keys, `&amp;` entities, empty `<array/>`/`<dict/>`, `<real>`, `<date>`): the output must equal Python's `xml()`. Tests: Task 3, the `plistxml_*` tests.
- **Re-running setup after a partial run** (a stale `.part`, an extracted tree without its marker, cached outputs from older pins or options): resume or rebuild, never mix old and new. Test: Task 9, `stamp_mismatch_forces_rebuild`.

---

### Task 1: Commit the design, fix kcbuild.py kmod size, make reference outputs

**Files:**
- Commit: `docs/superpowers/specs/2026-11-24-kc-linker-setup-design.md`, `docs/superpowers/plans/2026-11-24-kc-linker-setup.md`
- Modify: `tools/kcbuild.py:320`, `tools/kcbuild.py:333`
- Scratch (not committed): `$S/kcsorted.py`, a copy of `kcbuild.py` with `dn.sort()` as the first line inside the `os.walk` loop of `find_bundles`. `$S` is the session scratchpad, and the reference trees are `$S/ref26/kdk_files`, `$S/ref27/kdk_files`.

**Interfaces:**
- Produces: `$S/refout/xkc.bin`, `xkc.bin.map`, `stdout.txt` (fixed Python, sorted walk), and `$S/refout-orig/…` (unfixed Python, sorted walk).

- [ ] **Step 1:** Commit the spec and this plan: `docs: kdk linker and setup stick design and plan`.
- [ ] **Step 2:** Run the unfixed sorted copy with the mkkc.sh options (Global Constraints), `--kernel $S/ref27/kdk_files/System/Library/Kernels/kernel`, `--ext $S/ref26/kdk_files/System/Library/Extensions --ext $S/ref27/kdk_files/System/Library/Extensions`, output `$S/refout-orig/xkc.bin`, stdout to `stdout.txt`. Expected: "kexts selected: N (with code: M)" and no "unhandled" exit.
- [ ] **Step 3:** In `kcbuild.py`, change the `__LINKEDIT` fixup to `load + e["size"]` and the `_kmod_info` write to `(load, e["size"], hdr_size)`. Add a matching copy to `$S/kcsorted.py`.
- [ ] **Step 4:** Rerun into `$S/refout/`. Expected: `.map` identical to `refout-orig`; `cmp -l` differences only inside kext headers and `_kmod_info` structs (spot-check one kext's offsets with the map).
- [ ] **Step 5:** Commit: `tools: kcbuild uses each kext's own size for kmod_info and __LINKEDIT`.

### Task 2: anathema-kc crate, Mach-O reader

**Files:**
- Create: `crates/anathema-kc/Cargo.toml` (deps: `anyhow`, `base64 = "0.22"`, `anathema-media = { path = "../anathema-media" }`), `crates/anathema-kc/src/lib.rs`, `crates/anathema-kc/src/macho.rs`, `crates/anathema-kc/tests/support/mod.rs`, `crates/anathema-kc/tests/macho.rs`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Produces (in `anathema_kc::macho`):
  - `pub fn x86_64_slice(data: &[u8]) -> Option<Vec<u8>>`: fat (BE `0xcafebabe`, 20-byte arch entries) or thin MH_MAGIC_64 with cputype `0x01000007`.
  - `pub struct Segment { pub name: String, pub vm: u64, pub vs: u64, pub fo: u64, pub fs: u64, pub lc: usize, pub nsects: u32 }`
  - `pub struct Section { pub seg: String, pub name: String, pub addr: u64, pub size: u64, pub lc: usize }`
  - `pub struct Symbol { pub name: String, pub n_type: u8, pub n_desc: u16, pub value: u64 }`
  - `pub struct Macho { pub data: Vec<u8>, pub ncmds: u32, pub segs: Vec<Segment>, pub sects: Vec<Section>, pub symtab: Option<[u32; 4]>, pub dysymtab: Option<[u32; 18]> }`
  - `Macho::parse(data: Vec<u8>) -> Result<Macho>`, `sym_at(&self, i: u32) -> Result<Symbol>`, `exports(&self) -> Result<Vec<(String, u64)>>` (extdef range, `n_type & 0x0e == 0x0e`, in table order), `find_sym(&self, name: &str) -> Result<Option<u64>>`, `uuid(&self) -> Option<[u8; 16]>`
  - `pub fn kernel_symbols(m: &Macho) -> Result<HashMap<String, u64>>`: locals first-wins, externals (`n_type & 1`) override.
  - `tests/support/mod.rs`: `pub struct KextSpec { pub name: &'static str, pub text: Vec<u8>, pub exports: Vec<(&'static str, u64)>, pub imports: Vec<&'static str>, pub ext_relocs: Vec<(i32, u32 /*symnum*/, u8 /*type*/, u8 /*len*/, bool /*pcrel*/)>, pub local_relocs: Vec<i32>, pub kmod_info_at: Option<u64> }` and `pub fn build_kext(spec: &KextSpec) -> Vec<u8>`. It builds a thin x86_64 MH_KEXT_BUNDLE with `__TEXT` at vm 0 (the header is inside it), `__DATA`, `__LINKEDIT`, LC_SYMTAB and LC_DYSYMTAB. Also `pub fn fat(slices: &[(u32, &[u8])]) -> Vec<u8>`.

- [ ] **Step 1: Write failing tests** in `tests/macho.rs`:
  - `slice_picks_x86_64_from_fat`: `fat(&[(0x0100000c, arm), (0x01000007, x86)])` slices to `x86`.
  - `slice_rejects_arm_only` returns `None`.
  - `parse_reads_segments_sections_symbols`: segment names `["__TEXT","__DATA","__LINKEDIT"]`.
  - `exports(...)` equals the spec's exports in order.
  - `find_sym("_kmod_info")` is `Some(addr)`.
  - `kernel_symbols_prefers_external_over_local`: same name local=1, external=2 → 2.
  - `uuid_reads_lc_uuid`.
- [ ] **Step 2:** `cargo test -p anathema-kc --test macho`. Expected: compile failure (crate/functions missing).
- [ ] **Step 3:** Implement `macho.rs`, matching `kcbuild.py`'s `Macho` (LC_SEGMENT_64 `0x19`, LC_SYMTAB `2`, LC_DYSYMTAB `0xb`, LC_UUID `0x1b`). Bounds-check every read and return errors rather than panicking.
- [ ] **Step 4:** `cargo test -p anathema-kc --test macho`. Expected: all pass.
- [ ] **Step 5:** Commit: `kc: mach-o reader for the kext collection linker`.

### Task 3: Python-compatible plist writer

**Files:**
- Create: `crates/anathema-kc/src/plistxml.rs`, `crates/anathema-kc/tests/plistxml.rs`
- Modify: `crates/anathema-media/src/plist.rs` (`<integer>` accepts a `0x`/`0X` prefix as hex, like plistlib)

**Interfaces:**
- Consumes: `anathema_media::plist::{parse, Value}`
- Produces (in `anathema_kc::plistxml`):
  - `pub fn normalize(v: Value) -> Value`: recursively merges duplicate dict keys (last value, first position), turns `\r\n` and lone `\r` in strings and keys into `\n`, and changes `Date("…Z")` to `Date` in isoformat (`YYYY-MM-DDTHH:MM:SS`, like `datetime.isoformat()`).
  - `pub fn py_float_repr(f: f64) -> String`
  - `pub fn write(v: &Value, ind: usize) -> String`: Python `xml(v, ind)` exactly (tabs; `<integer size="64">` printing `i as u64`; `<real>` with `py_float_repr`; strings and keys escape only `& < >`; `<data>` standard base64 on one line; a date becomes `<string>` of its isoformat; an empty dict is `<dict>\n</dict>`; an empty array is `<array>\n\n</array>` with the pad).
  - `pub fn document(infos: &[Value]) -> Vec<u8>`: the exact header (`<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0">\n`), then `write({"_PrelinkInfoDictionary": infos}, 0)`, then `"\n</plist>\n\0"`.

- [ ] **Step 1: Write failing tests:**
  - `plistxml_matches_python_for_mixed_dict`: input with nested dict, array, bool, int `-1`, int `0xffffff8002000000` (as hex text), data, string `a&b<c>`. Expected text is pasted verbatim from `python3 -c` running `kcbuild.py`'s `xml()` on `plistlib.loads` of the same input (generate once, embed as a string literal).
  - `plistxml_empty_containers`: `<array/>` and `<dict/>` inside a dict.
  - `plistxml_duplicate_keys_last_wins_first_position`.
  - `plistxml_crlf_normalized`.
  - `plistxml_date_isoformat`: `2026-10-14T23:40:09Z` gives `<string>2026-10-14T23:40:09</string>`.
  - `py_float_repr_cases`: `1.0→"1.0"`, `0.5→"0.5"`, `1e-05→"1e-05"`, `0.0001→"0.0001"`, `1e16→"1e+16"`, `123456789012345.6→"123456789012345.6"`, `-0.0→"-0.0"`, `inf→"inf"`.
  - `media_plist_parses_hex_integer` (in media's `tests/parsers.rs`).
- [ ] **Step 2:** Run `cargo test -p anathema-kc --test plistxml` and `cargo test -p anathema-media --test parsers`. Expected: FAIL.
- [ ] **Step 3:** Implement. `py_float_repr` takes the shortest digits from `format!("{:e}", f)`. With decimal exponent `e`, it uses fixed notation when `-4 <= e < 16` (appending `.0` if there's no fraction), otherwise `d[.ddd]e±XX` with at least two exponent digits.
- [ ] **Step 4:** Rerun both. Expected: PASS.
- [ ] **Step 5:** Commit: `kc: plist writer matching kcbuild.py's xml(); media plist reads hex integers`.

### Task 4: Bundle discovery, linker, CLI, parity on real KDKs

**Files:**
- Create: `crates/anathema-kc/src/bundles.rs`, `crates/anathema-kc/src/link.rs`, `crates/anathema-kc/src/main.rs`, `crates/anathema-kc/tests/link.rs`, `tools/kc_parity.sh`
- Modify: `crates/anathema-kc/src/lib.rs`, `crates/anathema-kc/Cargo.toml` (`[[bin]] name = "anathema-kc"`)

**Interfaces:**
- Consumes: Tasks 2 and 3.
- Produces:
  - `anathema_kc::bundles::Bundle { pub path: PathBuf, pub plist: PathBuf, pub info: Value, pub root: PathBuf }`
  - `pub fn find_bundles(roots: &[PathBuf]) -> Vec<(String, Bundle)>`: insertion-ordered, first id wins, sorted DFS, unreadable plists skipped.
  - `anathema_kc::Config { pub kernel: PathBuf, pub ext: Vec<PathBuf>, pub overrides: Vec<(String, PathBuf)>, pub base: u64, pub roots: Vec<String>, pub no_start: Vec<String>, pub strip: Vec<(String, String)>, pub set_true: Vec<(String, String)>, pub rename: Vec<(String, String)>, pub add_lib: Vec<(String, String, String)>, pub codeless: Vec<String>, pub prefer_last: Vec<String> }`
  - `anathema_kc::Output { pub xkc: Vec<u8>, pub map: String, pub report: Report }`
  - `anathema_kc::Report { pub lines: Vec<String> }` with `impl Display`: every line `kcbuild.py` prints, in its order.
  - `pub fn link(cfg: &Config) -> Result<Output>`
  - CLI `anathema-kc` takes the same flags and positional roots as `kcbuild.py`. It writes `--out` and `--out` + `.map` and prints the report.

Port notes (behavior the signature does not show; copy from `kcbuild.py`):
- Executable lookup: tries `Contents/MacOS/<exe>`, then `<path>/<exe>`. The last one that exists wins.
- Symbol resolution order: own bundle, then the DFS closure of its libraries, then kernel symbols, then every linked bundle in link order, then the stub address `base + total + 0x80000 + 0x1800`.
- Reloc fixups:
  - external `(0,3,!pc)`: write `S+A` as a u64.
  - external `(2,2,pc)`: write `S+A-(P+4)` as an i32; an out-of-range value is an error.
  - local `(0,3,!pc)`: add the load address.
  - any other kind: error `unhandled ext reloc (t, l, pc) in <bid>`.
- Map lines are `"{bid} {load:#x} {size:#x} {exe}\n"`. Paths are built the same way the Python builds them (from the `--ext` strings as given, joined with `/`).
- `_kmod_info` gets the kext's own size (Task 1 fix).

- [ ] **Step 1: Write failing tests** in `tests/link.rs`, building kexts with `support::build_kext` in a temp dir:
  - `links_two_kexts_and_resolves_dependency`: B imports `_foo` exported by A, through an ext64 reloc. Expected: the u64 at B's reloc site equals A's load + `_foo`.
  - `branch_reloc_is_pc_relative`.
  - `local_reloc_adds_load_address`.
  - `unresolved_symbol_points_at_stub_and_is_reported`: the report contains `UNRESOLVED symbols (1):`.
  - `kmod_info_gets_each_kexts_own_size`: two kexts of different sizes. Each `_kmod_info + 156` holds `(load, own_size, hdr_size)`.
  - `header_layout`: the first 64 bytes are the eight u64 fields (magic, base, total, plist_off, plist_len, cc_start, cc_kmod, blob_len), and `blob_len == plist_off + 0x80000 + 0x2000`.
  - `codeless_and_strip_edit_plist`.
- [ ] **Step 2:** `cargo test -p anathema-kc --test link`. Expected: FAIL.
- [ ] **Step 3:** Implement `bundles.rs`, `link.rs` and `main.rs`.
- [ ] **Step 4:** `cargo test -p anathema-kc`. Expected: PASS.
- [ ] **Step 5:** Write `tools/kc_parity.sh KERNEL EXT1 EXT2 OUTDIR`. It runs `anathema-kc` with the mkkc.sh options into `OUTDIR/rust/` and prints `sha256` of `xkc.bin` and `.map` plus `diff` of stdout against `OUTDIR/python/` (the user supplies the Python output). Run it against `$S/refout`. Expected: `cmp` reports identical `xkc.bin`, identical `.map`, identical stdout.
- [ ] **Step 6:** Commit: `kc: kext collection linker in rust (kcbuild.py port) with parity script`.

### Task 5: cpio and pbzx readers

**Files:**
- Create: `crates/anathema-media/src/kdk/mod.rs`, `src/kdk/cpio.rs`, `src/kdk/pbzx.rs`, `crates/anathema-media/tests/kdk_streams.rs`
- Modify: `crates/anathema-media/src/lib.rs` (`pub mod kdk;`), `crates/anathema-media/Cargo.toml` (`lzma-rs = "0.3"`)

**Interfaces:**
- Produces:
  - `kdk::cpio::Entry { pub name: String /* leading "./" stripped */, pub mode: u32, pub size: u64 }`; `pub fn for_each(r: impl Read, f: impl FnMut(&Entry, &mut dyn Read) -> Result<()>) -> Result<u64 /*entries*/>`. This reads odc `070707` headers, stops at `TRAILER!!!`, and skips unread file bytes after each callback.
  - `kdk::pbzx::Pbzx::new<R: Read + Send + 'static>(inner: R, threads: usize) -> Result<Pbzx>`, which implements `Read`. It checks the `pbzx` magic and reads chunks as `[u64 BE usize][u64 BE csize][data]`: stored if `csize == usize`, otherwise xz. Chunks are decoded on `threads` workers and output in order, with at most `2 * threads` in flight. It also has `pub fn consumed(&self) -> Arc<AtomicU64>`, the compressed bytes read so far, for progress.

- [ ] **Step 1: Write failing tests:**
  - `cpio_reads_files_dirs_and_trailer`: build an odc archive in the test with a dir, a 0-byte file, a 3-byte file, and a symlink. The callback sees names without `./`, the right modes, and exact bytes.
  - `cpio_rejects_bad_magic`.
  - `pbzx_round_trips_mixed_chunks`: build pbzx from 3 chunks (xz via `lzma_rs::xz_compress`, one stored, one xz). The output equals the concatenated input with `threads = 1` and `threads = 4`.
  - `pbzx_rejects_bad_magic`.
  - `pbzx_truncated_chunk_errors`.
- [ ] **Step 2:** `cargo test -p anathema-media --test kdk_streams`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Rerun. Expected: PASS.
- [ ] **Step 5:** Commit: `media: kdk cpio and parallel pbzx readers`.

### Task 6: UDIF, HFS+ and xar readers

**Files:**
- Create: `src/kdk/udif.rs`, `src/kdk/hfs.rs`, `src/kdk/xar.rs`, `crates/anathema-media/tests/kdk_images.rs`

**Interfaces:**
- Consumes: `readat::{ReadAt, FileAt, Section}`, `codec::lzfse_stream`, `plist::parse`, `flate2`.
- Produces:
  - `kdk::udif::Udif::open(r: Arc<dyn ReadAt + Send + Sync>) -> Result<Udif>`
  - `Udif::partitions(&self) -> &[Partition]`, where `Partition { pub name: String, pub sectors: u64 }`
  - `Udif::open_partition(&self, index: usize) -> Result<PartitionReader>`. `PartitionReader` implements `ReadAt + Send + Sync`, with an LRU of the 64 most recent decompressed chunks.
  - `Udif::find_hfs(&self) -> Result<usize>`: the first partition whose name contains `Apple_HFS`.
  - Chunk types: `0x0` and `0x2` are zeros, `0x1` raw, `0x80000005` zlib, `0x80000007` lzfse. `0x80000004`, `0x80000006` and `0x80000008` fail with `dmg chunk type {t:#x} is not supported`. `0x7ffffffe` and `0xffffffff` are skipped.
  - `kdk::hfs::Hfs::open(r: Arc<dyn ReadAt + Send + Sync>) -> Result<Hfs>` accepts signatures `H+` and `HX`.
  - `Hfs::open_file(&self, path: &str) -> Result<HfsFile>`, where `HfsFile` implements `ReadAt + Send + Sync`. It covers the data fork, with the extents overflow file used when the 8 inline extents fall short. A zero-length fork on a file with nonzero blocks fails with `file is compressed (decmpfs), not supported`.
  - `kdk::xar::Xar::open(r: Arc<dyn ReadAt + Send + Sync>) -> Result<Xar>`
  - `Xar::file(&self, path: &str) -> Result<XarFile>`, where `XarFile { pub offset: u64 /*absolute*/, pub length: u64, pub encoding: String }`
  - `Xar::reader(&self, f: &XarFile) -> Result<XarReader>`. `XarReader` implements `Read + Send + 'static` (it owns the `Arc`), so it can feed `Pbzx::new`. Encoding `application/octet-stream` is used as is; any other encoding fails with a message naming it.

- [ ] **Step 1: Write failing tests.** On macOS (`#[cfg(target_os = "macos")]`), build fixtures in a temp dir at run time:
  - A source folder with `pkgroot/a.txt` (10 KB) and `big.bin` (3 MB, pseudo-random from a seed).
  - `pkgbuild --root pkgroot --identifier test.kdk --install-location / pkg/test.pkg`, then `hdiutil create -fs HFSX -format ULFO -srcfolder <folder> img.dmg`, and the same with `-format UDZO` and `-format UDRO`.

  Tests:
  - `udif_lzfse_zlib_raw_partitions_read_identically`: the HFS partition bytes from all three dmgs are equal at 3 random offsets.
  - `hfs_reads_big_file_exactly`: `big.bin` equals the source.
  - `xar_finds_payload_in_pkg`: `test.pkg` contains `Payload` with encoding `application/octet-stream`. (`pkgbuild` puts `Payload` at the root of a component package. Note in the test that the KDK uses `KDK.pkg/Payload` inside a product archive.)
  - `pkg_payload_chain`: from `Payload` through pbzx and cpio, the list contains `./a.txt`.

  Platform-neutral tests:
  - `udif_rejects_bzip2_chunk`, built from a hand-made koly + plist with one `0x80000006` chunk.
  - `xar_rejects_bad_magic`.
- [ ] **Step 2:** `cargo test -p anathema-media --test kdk_images`. Expected: FAIL.
- [ ] **Step 3:** Implement. For HFS+, scan the catalog leaf nodes from `firstLeafNode` along `fLink`, collect `(parentID, name) → (kind, id, fork)`, and resolve paths from folder id 2. For the extents overflow, scan the leaves for `fileID` and fork type 0, in `startBlock` order. Parse the xar TOC with a small element walker over the zlib-inflated XML (`<file>` nesting, `<name>`, `<type>`, `<data>/<offset|length|encoding style=…>`).
- [ ] **Step 4:** Rerun. Expected: PASS.
- [ ] **Step 5:** Commit: `media: kdk dmg (udif), hfs+ and xar readers`.

### Task 7: Pins, verified download, extract, CLI, parity with Python trees

**Files:**
- Create: `src/kdk/pin.rs`, `src/kdk/extract.rs`
- Modify: `src/download.rs`, `src/kdk/mod.rs`, `src/main.rs`, `tests/kdk_streams.rs`

**Interfaces:**
- Produces:
  - `kdk::pin::Kdk { pub build: &'static str, pub url: &'static str, pub size: u64, pub sha256: &'static str }`
  - `pub const KDK_TAHOE: Kdk` (25G342) and `pub const KDK_27: Kdk` (26B131), with values from Global Constraints
  - `pub const KERNEL_UUID: [u8; 16]`, `pub const KERNEL_SHA256: &str`, `pub const KERNEL_PATH: &str = "System/Library/Kernels/kernel"`
  - `download::download_verified(url: &str, target: &Path, size: u64, sha256: &str, progress: &mut dyn FnMut(u64, u64)) -> Result<()>`. It resumes `<target>.part` like `download`, hashes with sha256, and leaves the result only at `target`.
  - `download::verify_or_discard(path: &Path, sha256: &str) -> Result<bool>`. It deletes the file on a mismatch and returns `false`.
  - `kdk::extract::wanted(name: &str) -> bool`: true for `KERNEL_PATH`, or for a name under `System/Library/Extensions/` where some component ends with `.kext` and none ends with `.dSYM`.
  - `kdk::extract::extract(dmg: &Path, out: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<ExtractReport>`, where `ExtractReport { pub files: u64, pub bytes: u64, pub longest_path: usize }`. It runs UDIF, HFS, `KernelDebugKit.pkg`, xar `KDK.pkg/Payload`, pbzx and cpio, writing only the files `wanted` accepts. Writes are atomic per file (`.tmp` then rename). It writes `out/.anathema-extracted` holding the dmg's sha256 last.
  - CLI: `anathema-media kdk fetch <25G342|26B131> [--out DIR]` and `anathema-media kdk extract <dmg> <outdir>`.

- [ ] **Step 1: Write failing tests:**
  - `verify_or_discard_rejects_corrupt_file` (temp file, wrong hash: returns `false` and the file is gone).
  - `verify_or_discard_accepts_good_file`.
  - `wanted_filter_cases`:
    - kernel: yes
    - `System/Library/Kernels/kernel.development`: no
    - `System/Library/Extensions/A.kext/Contents/Info.plist`: yes
    - `.../A.kext.dSYM/Contents/Info.plist`: no
    - `.../A.kext/Contents/PlugIns/B.kext/Contents/MacOS/B`: yes
    - `System/Library/Extensions/README`: no
  - `real_kdk_extract_matches_python` (`#[ignore]` unless `KDK_DMG` and `KDK_REF` are set). It extracts `$KDK_DMG`. Every extracted file must exist in `$KDK_REF` with equal bytes, and every `wanted` file in `$KDK_REF` must have been extracted.
- [ ] **Step 2:** `cargo test -p anathema-media --test kdk_streams`. Expected: FAIL.
- [ ] **Step 3:** Implement. Refactor `download()` to share its resume loop with `download_verified` (CLI output for `download()` unchanged).
- [ ] **Step 4:** Run the unit tests (expected: PASS). Then run `KDK_DMG=$S/kdk-dl/Kernel_Debug_Kit_27.2.1_build_26B131.dmg KDK_REF=$S/ref27/kdk_files cargo test -p anathema-media --release --test kdk_streams -- --ignored`, and the same for 25G342 with `ref26`. Expected: PASS. Print the extract time and `longest_path` in the test output.
- [ ] **Step 5:** Commit: `media: pinned kdk download and pure rust kdk extraction`.

### Task 8: Setup lib, FAT32 file tree, xcp.bin, ESP dev tool

**Files:**
- Create: `crates/anathema-setup/src/lib.rs`, `crates/anathema-setup/src/bin/esp.rs`, `crates/anathema-setup/assets/xcp.bin`, `tools/mkxcp.sh`, `crates/anathema-setup/tests/fat32.rs`
- Modify: `crates/anathema-setup/src/fat32.rs`, `crates/anathema-setup/src/main.rs` (use the lib's `fat32`), `crates/anathema-setup/src/install.rs` (call sites), `crates/anathema-setup/Cargo.toml` (`[lib] path = "src/lib.rs"`, `[[bin]] name = "anathema-esp" path = "src/bin/esp.rs"`)

**Interfaces:**
- Produces:
  - `anathema_setup::fat32::FileSpec<'a> { pub path: &'a str /* "EFI/BOOT/BOOTX64.EFI", '/' separated */, pub data: &'a [u8] }`
  - `fat32::build(total_sectors: u64, hidden: u32, label: &str, files: &[FileSpec]) -> Result<Vec<Region>, String>`: makes parent dirs with `.` and `..`, upper-case 8.3 names (an error if a name doesn't fit 8.3), contiguous clusters per file, and directories that may span more than one cluster.
  - `fat32::image(regions: &[Region], total_sectors: u64) -> Vec<u8>`: the flat partition image, for tests and the dev bin.
  - `pub const XCP: &[u8] = include_bytes!("../assets/xcp.bin")` in `lib.rs`.
  - Dev bin: `anathema-esp --efi PATH --kernel PATH --xkc PATH --out esp.img [--size-mib 200]`. Later (Task 9): `anathema-esp --efi PATH --cache DIR --out esp.img` uses `kernel_files::prepare`.

- [ ] **Step 1: Write failing tests** in `tests/fat32.rs`:
  - `tree_with_four_files_round_trips`: build with `BOOTX64.EFI` (300 KB), `anathema/xkernel` (21 MB), `xkc.bin` (40 MB), `xcp.bin` (512 B). Walk the image with an in-test FAT32 reader and check every file's bytes.
  - On macOS, also run `fsck_msdos -n` on the image file. Expected: exit 0.
  - `rejects_files_larger_than_partition`: an error containing `does not fit`.
  - `rejects_non_8_3_name`.
- [ ] **Step 2:** `cargo test -p anathema-setup --test fat32`. Expected: FAIL.
- [ ] **Step 3:** Implement. Keep `install::fresh_install` working through `FileSpec` (BOOTX64 only until Task 9).
- [ ] **Step 4:** Rerun. Expected: PASS.
- [ ] **Step 5:** `tools/mkxcp.sh` compiles `tools/commpage_stub.c` for `x86_64-unknown-linux-gnu` (`-O2 -ffreestanding -fno-pic -fno-stack-protector -mno-red-zone -mcmodel=kernel -c`). It checks there are no relocations in `.text.a`/`.text.b`, then writes `.text.a` at offset 0 and `.text.b` at 0x100 into `crates/anathema-setup/assets/xcp.bin` using a short `python3 -I` ELF section copier inside the script. Run it. Expected: a file between `0x100 + 1` and `0x200` bytes. The `.text.a` bytes disassemble (`llvm-objdump` or `otool -t` on the object) to calls through the absolute addresses in the C source.
- [ ] **Step 6:** Commit: `setup: fat32 file tree builder, embedded xcp.bin, esp image dev tool`.

### Task 9: Kernel files phase and installer integration

**Files:**
- Create: `crates/anathema-setup/src/kernel_files.rs`, `crates/anathema-setup/tests/kernel_files.rs`
- Modify: `src/lib.rs`, `src/install.rs`, `src/main.rs`, `src/bin/esp.rs`, `Cargo.toml` (deps `anathema-media`, `anathema-kc`)

**Interfaces:**
- Consumes: `kdk::pin::*`, `download::download_verified`, `kdk::extract::extract`, `anathema_kc::{Config, link}`, `macho::{Macho, x86_64_slice}`, `fat32::FileSpec`, `XCP`.
- Produces (in `kernel_files`):
  - `pub struct KernelFiles { pub xkernel: Vec<u8>, pub xkc: Vec<u8> }`
  - `pub fn kc_config(tahoe_ext: &Path, k27_ext: &Path, kernel: &Path) -> anathema_kc::Config`: the mkkc.sh options from Global Constraints.
  - `pub const PEAK_CACHE_BYTES: u64 = 3_600_000_000`: two extracted trees (about 1.04 GB + 0.99 GB) plus one dmg (1.25 GB) plus margin.
  - `pub fn space_check(free: u64, need: u64) -> Result<(), String>`. The message is `Not enough free space for the kernel files: {need/1e9:.1} GB needed, {free/1e9:.1} GB free on {drive}`.
  - `pub fn stamp() -> String`: sha256 hex of the pins, kernel hash, `kc_config` debug text and `CARGO_PKG_VERSION`.
  - `pub fn prepare(cache: &Path, progress: &mut dyn FnMut(f32, &str), log: &mut dyn FnMut(String)) -> Result<KernelFiles, String>`. The flow:
    - If `cache/out/stamp` equals `stamp()`, return the cached files.
    - Otherwise, for each KDK: download, extract, then delete the dmg.
    - Read the kernel, check its UUID and sha256, and link.
    - Write `cache/out/{xkernel,xkc.bin}` and then `stamp`.
    - Delete the extracted trees.
  - `pub fn esp_files<'a>(efi: &'a [u8], k: &'a KernelFiles) -> Vec<FileSpec<'a>>`: the four ESP paths.
  - `install::update_plan(esp_free: u64, existing: &[(String, u64)], files: &[FileSpec]) -> Result<(), String>`. It fails with `The EFI partition is too small: {need} MB needed, {have} MB free` before any write.
  - `install::fresh_install(disk, files: &[FileSpec], ctx)` and `install::update_boot(disk, efi: &[u8], files: &[FileSpec], ctx)`.

- [ ] **Step 1: Write failing tests** in `tests/kernel_files.rs`:
  - `space_check_reports_need_and_free`.
  - `stamp_mismatch_forces_rebuild`: write `out/stamp` = "old", and `prepare` with a fake fetcher seam returns an error from the fetcher (proving it didn't take the cache). To make this testable, `prepare` is a thin wrapper over `prepare_with(cache, fetch: &mut dyn FnMut(&Kdk, &Path) -> Result<(), String>, …)`.
  - `stamp_match_uses_cache`: with `out/` populated and the stamp matching, the fetcher is never called.
  - `update_plan_rejects_full_esp`: free 10 MB, files 60 MB, so an error mentioning `too small`.
  - `esp_files_paths`: the paths equal the four Global Constraints paths in order.
  - `kc_config_matches_mkkc` checks roots, prefer-last, strip, codeless and base.
- [ ] **Step 2:** `cargo test -p anathema-setup --test kernel_files`. Expected: FAIL.
- [ ] **Step 3:** Implement `kernel_files.rs`. Then wire it in:
  - `main.rs`:
    - `run_install` calls `prepare` (cache `%LOCALAPPDATA%\Anathema\kdk`, from `std::env::var_os("LOCALAPPDATA")`) before the mode-specific step, with progress mapped to 0.0–0.6 of the bar.
    - In a dry run, log what would be downloaded instead.
    - Welcome text becomes "boot macOS on this pc".
    - The Review page shows "Downloads about 2.4 GB of Apple kernel files from dortania's GitHub releases (once)" and lists `anathema\xkernel`, `xkc.bin`, `xcp.bin`.
    - Remove "nothing is downloaded".
  - `install.rs`:
    - Fresh install writes all `esp_files`.
    - Update-boot runs `update_plan`, then for each anathema file writes `.new`, renames, reads back and compares (the pattern already used for `BOOTX64.EFI`).
  - `bin/esp.rs` gets the `--cache` mode.
  - `build.rs`: the panic text becomes `set ANATHEMA_EFI to the anathema-xboot .efi to embed`.
  - Spec update: the spec's "Setup changes" bullet about markers now says what is built. Setup caches only the final `xkernel`/`xkc.bin` under a stamp, and deletes the dmgs and extracted trees after a successful link. A rerun with the same stamp needs no download. A new pin or new options mean one fresh download. This is the builder's call, for disk space: about 2 GB less left on the user's drive.
- [ ] **Step 4:** Run `cargo test -p anathema-setup --lib --tests` (expected: PASS). Then `cargo check --target x86_64-pc-windows-msvc -p anathema-setup --bins` (expected: no errors).
- [ ] **Step 5:** Commit: `setup: download kdks, link the kext collection and write kernel files to the stick`.

### Task 10: End-to-end verification

**Files:**
- Create: `tools/xrun_esp.sh`, a copy of `tools/xrun_usb.sh` that takes an ESP image path and the edk2 firmware path (macOS Homebrew `/opt/homebrew/share/qemu/edk2-x86_64-code.fd`), and boots it as `usb-storage`.

**Interfaces:**
- Consumes: the `anathema-esp` dev bin, `anathema-xboot` built for `x86_64-unknown-uefi`.

- [ ] **Step 1:** Build xboot: `cargo build --release -p anathema-xboot --target x86_64-unknown-uefi`.
- [ ] **Step 2:** `anathema-esp --efi <xboot.efi> --cache $S/cache --out $S/esp.img`. Use the already-downloaded dmgs by copying them to the cache names first, so the run exercises extract, link and the checks without downloading again. Expected: `stamp` written, `xkc.bin` equal to `$S/refout/xkc.bin` (the same sorted Python run), UUID check passes.
- [ ] **Step 3:** `fsck_msdos -n $S/esp.img`. Expected: no errors. `hdiutil attach` the image and `cmp` the four files.
- [ ] **Step 4:** `tools/xrun_esp.sh $S/esp.img`. Expected serial log: `anathema-xboot: start`, `kext block installed`, then kernel output continuing to the same point the collaborator reports (at least past kext start; with no root disk, a root-mount wait). Save the log to `$S/qemu-serial.log`.
- [ ] **Step 5:** SD card, with owner approval on record (`disk6`, SanDisk SDSU08G, 7.95 GB):
  - Re-check with `diskutil info disk6` that it is still the built-in-reader SDHC card.
  - Run `diskutil partitionDisk disk6 GPT FAT32 ESP 200M FREE rest R` (or the equivalent that makes a 200 MiB first partition). Then write `esp.img` over `/dev/rdisk6s1` with `/usr/libexec/authopen -w`; the user approves the macOS password prompt.
  - Mount it and `cmp` the four files.
  - Run `diskutil verifyVolume disk6s1`.
- [ ] **Step 6:** Commit `tools/xrun_esp.sh`: `tools: boot an esp image in qemu on macos or linux`. Report the results: parity, QEMU log excerpt, SD card check. Also report what remains manual (the Windows GUI on a real stick, and real PC hardware).
