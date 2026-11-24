import struct, sys
Z = bytes([0])
base = 0x100000000
text_off = 0x1000
m1 = b"step1: console ok\n"
m2 = b"step2: commpage readable\n"
code = bytearray()
leas = []

def lea(reg_op):
    leas.append((len(code),))
code += b"\xb8" + struct.pack("<I", 0x2000005)
p_lea = len(code); code += b"\x48\x8d\x3d" + bytes(4)
code += b"\xbe" + struct.pack("<I", 2)
code += b"\x31\xd2\x0f\x05\x89\xc7"
code += b"\xb8" + struct.pack("<I", 0x2000004)
a_lea = len(code); code += b"\x48\x8d\x35" + bytes(4)
code += b"\xba" + struct.pack("<I", len(m1)) + b"\x0f\x05"
code += b"\x49\x89\xfc"
code += b"\x48\xb8" + struct.pack("<Q", 0x00007fffffe00000)
code += b"\x8a\x18"
code += b"\xb8" + struct.pack("<I", 0x2000004)
code += b"\x44\x89\xe7"
b_lea = len(code); code += b"\x48\x8d\x35" + bytes(4)
code += b"\xba" + struct.pack("<I", len(m2)) + b"\x0f\x05"
code += b"\xf3\x90\xeb\xfc"
path_pos = len(code); code += b"/dev/console" + Z
m1_pos = len(code); code += m1
m2_pos = len(code); code += m2
code[p_lea+3:p_lea+7] = struct.pack("<i", path_pos - (p_lea + 7))
code[a_lea+3:a_lea+7] = struct.pack("<i", m1_pos - (a_lea + 7))
code[b_lea+3:b_lea+7] = struct.pack("<i", m2_pos - (b_lea + 7))
entry = base + text_off

def seg(name, vmaddr, vmsize, fileoff, filesize, prot, nsects, sects=b""):
    return struct.pack("<II16sQQQQiiII", 0x19, 72 + len(sects), name.ljust(16, Z), vmaddr, vmsize, fileoff, filesize, prot, prot, nsects, 0) + sects

def sect(sn, sg, addr, size, offset, flags):
    return struct.pack("<16s16sQQIIIIIIII", sn.ljust(16, Z), sg.ljust(16, Z), addr, size, offset, 4, 0, 0, flags, 0, 0, 0)

cmds = seg(b"__PAGEZERO", 0, base, 0, 0, 0, 0)
cmds += seg(b"__TEXT", base, 0x2000, 0, 0x1000 + len(code), 5, 1, sect(b"__text", b"__TEXT", entry, len(code), text_off, 0x80000400))
state = [0] * 21
state[16] = entry
cmds += struct.pack("<IIII", 0x5, 16 + 168, 4, 42) + struct.pack("<21Q", *state)
header = struct.pack("<IiiIIIII", 0xfeedfacf, 0x01000007, 3, 2, 3, len(cmds), 0x1, 0)
open(sys.argv[1], "wb").write(bytearray(header + cmds).ljust(text_off, Z) + code)
print("ok")
