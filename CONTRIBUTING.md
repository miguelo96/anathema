# Contributing to Anathema

Anathema boots macOS 27 on PCs with its own loader. Come say hi in the Discord first: https://discord.gg/ZSQxYC8RT7

## Ways to help
- **Test on real hardware.** Tell us your CPU, board, GPU and what happens in the `hardware-support` channel. Photos of the boot screen are gold.
- **Pick something from [TODO.md](TODO.md).** Say in `dev-talk` which one you take so nobody doubles up.
- **Report bugs.** Use `bug-reports` with the exact boot text and your hardware.
- **Suggest ideas** in `ideas`.

## Working on the code
- `crates/anathema-xboot` is the UEFI loader that starts the kernel. `tools/` has the scripts that build the kext collection, the root image and the test disks.
- Test in an emulator first (QEMU). See `tools/xrun.sh` and `tools/rundisk.sh`. A boot takes several minutes in the emulator, so run several at once.
- Keep changes small and focused, one idea per pull request.
- Commit messages: short and lowercase, for example `xboot: fix tsc ratio on zen`.
- Please don't add Apple binaries or keys to the repo. The build tools fetch what they need.

## Licensing
Anathema is open source (see LICENSE). Some pieces we build on, such as the NullMoth NVIDIA driver, have their own licenses and stay separate add-ons with credit.
