"""build a tiny static x86_64 mach-o executable that prints a line from user mode and then spins.
usage: mkinit.py out_file"""
import struct, sys

msg = b"launchd: hello from anathema userspace\n"

# code at file offset 0x1000 (vm 0x100000000 + 0x1000)
base = 0x100000000
text_off = 0x1000
code = bytearray()
# mov eax, 0x2000004 ; write
code += b"\xb8" + struct.pack("<I", 0x2000004)
# mov edi, 1
code += b"\xbf" + struct.pack("<I", 1)
# lea rsi, [rip + msg]
lea_pos = len(code)
code += b"\x48\x8d\x35" + b"\x00\x00\x00\x00"
# mov edx, len
code += b"\xba" + struct.pack("<I", len(msg))
# syscall
code += b"\x0f\x05"
# spin: pause; jmp spin
spin = len(code)
code += b"\xf3\x90" + b"\xeb\xfc"
msg_pos = len(code)
code += msg
# fix lea rel32: target = msg_pos, next ip = lea_pos + 7
rel = msg_pos - (lea_pos + 7)
code[lea_pos + 3:lea_pos + 7] = struct.pack("<i", rel)

entry = base + text_off

# load commands
def seg(name, vmaddr, vmsize, fileoff, filesize, prot, nsects, sects=b""):
    c = struct.pack("<II16sQQQQiiII", 0x19, 72 + len(sects), name.ljust(16, b"\0"), vmaddr, vmsize, fileoff, filesize, prot, prot, nsects, 0)
    return c + sects

def sect(sectname, segname, addr, size, offset, flags):
    return struct.pack("<16s16sQQIIIIIIII", sectname.ljust(16, b"\0"), segname.ljust(16, b"\0"), addr, size, offset, 4, 0, 0, flags, 0, 0, 0)

pagezero = seg(b"__PAGEZERO", 0, base, 0, 0, 0, 0)
text_sect = sect(b"__text", b"__TEXT", entry, len(code), text_off, 0x80000400)
text = seg(b"__TEXT", base, 0x2000, 0, 0x1000 + len(code), 5, 1, text_sect)
# LC_UNIXTHREAD x86_THREAD_STATE64: flavor 4, count 42
state = [0] * 21
state[16] = entry  # rip
unix_thread = struct.pack("<IIII", 0x5, 16 + 168, 4, 42) + struct.pack("<21Q", *state)
cmds = pagezero + text + unix_thread
ncmds = 3
header = struct.pack("<IiiIIIII", 0xfeedfacf, 0x01000007, 3, 2, ncmds, len(cmds), 0x1, 0)
blob = bytearray(header + cmds)
blob = blob.ljust(text_off, b"\0")
blob += code
open(sys.argv[1], "wb").write(blob)
print("wrote", sys.argv[1], len(blob), "bytes, entry", hex(entry))
