# Anathema FAQ

Short answers for people who just found the project. For the technical details, read the [README](README.md).

* [The basics](#the-basics)
* [Trying it](#trying-it)
* [Hardware](#hardware)
* [Trust, safety and legal](#trust-safety-and-legal)
* [Helping](#helping)

## The basics

### What is Anathema?

A research project that tries to boot **arm64 macOS on an ordinary x86-64 PC**. PCs can't run arm64 code directly, so Anathema translates the macOS kernel's arm64 instructions into x86-64 as it runs, and emulates the small virtual machine that macOS expects (interrupt controller, timer, serial port, disk). It also has its own UEFI boot loader for a USB stick.

### Does it boot macOS?

**No, not yet.** The macOS kernel starts, gets through most of its early setup, finds the disk and mounts it, and then fails when it tries to start the first user program (`/sbin/launchd`). No desktop, no installer, no apps. The [Status section of the README](README.md#status) has the exact details and is kept up to date.

### Is this a Hackintosh?

Not in the usual sense. A Hackintosh runs **Intel** macOS directly on PC hardware. Apple has stopped making Intel versions of macOS, so a Hackintosh can't run new releases. Anathema goes the other way: it runs the **arm64** version of macOS (the one for Apple silicon Macs) by translating it.

### Which macOS version?

The VirtualMac2,1 build of macOS 27.2. That is the version Apple ships for macOS virtual machines. It expects simpler hardware than a real Mac, which makes it the easiest arm64 macOS to start without Apple silicon.

### Why is it so slow?

Every memory access the guest makes leaves the translated code and goes through a software memory manager. That is simple and easy to check for correctness, but slow: reaching the current stopping point takes minutes. Making it fast is planned work, after it boots correctly.

### Is it related to Stela?

No. Stela is a separate project. Anathema looked at it only for ideas, contains none of its code, and is not a fork.

### When will it boot to a desktop?

There is no date. The next step is getting `launchd` to run. After that, graphics is the biggest open problem (see the [display write-up in the ideas repo](https://github.com/riftaway7-code/anathema-ideas/blob/main/writeups/display-output.md)).

## Trying it

### Can I try it on my PC?

Not as a normal user yet. Right now it is for developers:

* The main test setup (`rt-boot`) runs on **Linux** and prints the macOS kernel's boot log to your terminal.
* The USB loader can show a hardware report and a readiness check for your PC, and has run its hardware bring-up test on a real Ryzen 5 5500. Its "Boot macOS" entry has only been tried under QEMU, not on a real PC.

If you just want to watch progress, see [How to follow along](#how-do-i-follow-along) below.

### Do I need my own copy of macOS?

Yes. The repo contains no Apple software. To run the Linux test setup you need a macOS 27 IPSW (Apple's restore image) that you got yourself, and you extract three files from it. The README's "Running the harness" section lists them. Please don't share extracted Apple files in issues or in the Discord.

### Can I build it on a Mac?

You can build it and run the unit tests on any machine, including Apple silicon Macs:

```
cargo test -p anathema-media
cargo test -p anathema-xlat
```

Running translated code needs an x86-64 host, because the translator produces x86-64 code. The full instruction checker (`rt-boot fbcheck`) also needs Linux with `qemu-aarch64`.

## Hardware

### Will my GPU / Wi-Fi / Ethernet / sound card work?

None of these work yet, for any hardware. But here is the important part: **macOS inside Anathema never sees your real hardware.** It sees a few virtual devices that Anathema emulates. Anathema's own loader is what talks to your real PC.

So "support for my card" means Anathema needs:

1. a virtual device that macOS already has a driver for, and
2. code in Anathema that drives your real card.

That is also why Hackintosh drivers (kexts) like IntelMausi or itlwm don't plug into Anathema: they are for Intel macOS talking directly to the card, and here macOS never touches the card.

The [ideas repo index](https://github.com/riftaway7-code/anathema-ideas/blob/main/INDEX.md) shows what is planned and in what order.

### AMD or NVIDIA?

Right now the loader shows the screen through your PC's standard UEFI graphics, which works on **any** card that shows a boot screen, AMD, NVIDIA or Intel. A real accelerated desktop is a much bigger problem that doesn't depend on the card brand yet. The [display write-up](https://github.com/riftaway7-code/anathema-ideas/blob/main/writeups/display-output.md) explains why.

### Will iCloud, iMessage or the App Store work?

That idea is parked. Apple services check the machine's identity, and making one up (or borrowing a real Mac's) goes against Apple's terms and can get your Apple account locked. The project won't ship fake or borrowed serial numbers.

### Can it install to my internal SSD?

Not yet, and on purpose. Today every write macOS makes goes to memory and is thrown away at power-off, so nothing reaches your real disks. Installing to an internal drive is planned, with strict safety rules. See the [install write-up](https://github.com/riftaway7-code/anathema-ideas/blob/main/writeups/install-to-internal-drive.md).

## Trust, safety and legal

### The README says it is "fully vibe coded". What does that mean?

Every line was written by an AI (Claude Code) under a human's direction. The maintainer says this up front so nobody mistakes it for audited code. It has bugs.

What keeps it honest:

* every translated instruction is checked against QEMU, an independent, widely used emulator, with random inputs, and that has found real bugs;
* unit tests for the file parsers and the translator's decoder run on any machine;
* outside reviews are welcome. Open an issue if you find something.

Read it as a research log that happens to compile, not as a product.

### Is it safe to run?

* The **Linux test setup** is an ordinary program. It reads the files you give it. The only files it writes are debug output you ask for (such as a framebuffer dump) and scratch files under `/tmp` for the QEMU checker.
* The **USB loader** only reads your internal drives (its NVMe driver is read-only). It writes its reports and logs to the USB stick. The **bring-up test** also saves its log (up to about 12 KB) to a UEFI variable named `AnathemaBringup` several times during the run, so the log survives a hang. The variable is deleted the next time you boot Anathema. Some boards have very little variable storage, so don't run the bring-up test over and over, and boot Anathema once more afterwards to clear it.
* **anathema-setup** (the Windows tool that prepares the USB stick) **erases the disk you pick**. It also lets you pick external USB SSDs and hard drives up to 256 GB, not only sticks. **Unplug every USB drive except the stick** before you start, and check the model and size on the Review screen.

Don't run experimental boot loaders on a machine that holds data you can't afford to lose without a backup. That's good advice for any project at this stage.

### Is it legal?

Anathema contains no Apple code, firmware or images. Apple's license for macOS says it may be used on Apple-branded hardware. Anathema is research into how the platform works. What you do with it is your responsibility. This is not legal advice.

### What is the license?

MIT, with one exception: `crates/anathema-rt/src/aesdev.rs` follows the behaviour of a file from QEMU, which is GPL-2.0-or-later, so that file should be treated as GPL-2.0-or-later. See the README's "Credits and license" section.

## Helping

### How can I help without writing code?

* **Hardware reports.** Boot the USB loader's hardware report and readiness check on your PC and share the result (board, CPU, GPU, what passed and what failed).
* **Ideas.** Add them to the [ideas repo](https://github.com/riftaway7-code/anathema-ideas). Check the index first, and if your idea is already there, add a +1 to that file instead of a new one.
* **Docs.** If something in the README or this FAQ confused you, say so. That's a bug too.

### How can I help with code?

* Read the [Status section](README.md#status) for the current problem.
* Run the tests (`cargo test -p anathema-media`, `cargo test -p anathema-xlat`) and add more. Parsers and the instruction decoder are good places to start.
* Open an issue before big changes, so work doesn't overlap.

### How do I follow along?

* **Watch the GitHub repo** (Watch → Custom → Releases, or All Activity if you want every commit).
* **Join the Discord** for the weekly devlog and announcements.
* The README's **Status** section is the source of truth for how far it boots.
