"""tiny x86_64 mach-o: open /dev/console, write a line, spin. usage: mkinit_console.py out_file"""
import struct, sys

msg = b"launchd: hello from anathema userspace\n"
base = 0x100000000
text_off = 0x1000
code = bytearray()
path = b"/dev/console" + bytes([0])
code += b"\xb8" + struct.pack("<I", 0x2000005)
p_lea = len(code)
code += b"\x48\x8d\x3d" + bytes(4)
code += b"\xbe" + struct.pack("<I", 2)
code += b"\x31\xd2"
code += b"\x0f\x05"
code += b"\x89\xc7"
code += b"\xb8" + struct.pack("<I", 0x2000004)
m_lea = len(code)
code += b"\x48\x8d\x35" + bytes(4)
code += b"\xba" + struct.pack("<I", len(msg))
code += b"\x0f\x05"
code += b"\xf3\x90" + b"\xeb\xfc"
path_pos = len(code)
code += path
msg_pos = len(code)
code += msg
code[p_lea + 3:p_lea + 7] = struct.pack("<i", path_pos - (p_lea + 7))
code[m_lea + 3:m_lea + 7] = struct.pack("<i", msg_pos - (m_lea + 7))

entry = base + text_off
Z = bytes([0])

def seg(name, vmaddr, vmsize, fileoff, filesize, prot, nsects, sects=b""):
    c = struct.pack("<II16sQQQQiiII", 0x19, 72 + len(sects), name.ljust(16, Z), vmaddr, vmsize, fileoff, filesize, prot, prot, nsects, 0)
    return c + sects

def sect(sectname, segname, addr, size, offset, flags):
    return struct.pack("<16s16sQQIIIIIIII", sectname.ljust(16, Z), segname.ljust(16, Z), addr, size, offset, 4, 0, 0, flags, 0, 0, 0)

pagezero = seg(b"__PAGEZERO", 0, base, 0, 0, 0, 0)
text_sect = sect(b"__text", b"__TEXT", entry, len(code), text_off, 0x80000400)
text = seg(b"__TEXT", base, 0x2000, 0, 0x1000 + len(code), 5, 1, text_sect)
state = [0] * 21
state[16] = entry
unix_thread = struct.pack("<IIII", 0x5, 16 + 168, 4, 42) + struct.pack("<21Q", *state)
cmds = pagezero + text + unix_thread
header = struct.pack("<IiiIIIII", 0xfeedfacf, 0x01000007, 3, 2, 3, len(cmds), 0x1, 0)
blob = bytearray(header + cmds).ljust(text_off, Z) + code
open(sys.argv[1], "wb").write(blob)
print("wrote", sys.argv[1], len(blob))
