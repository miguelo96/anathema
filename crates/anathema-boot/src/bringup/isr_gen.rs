use core::arch::global_asm;

pub const VECTORS: [u8; 81] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 255];

global_asm!(
    concat!("isr0:
 push 0
 push 0
 jmp isr_common
"),
    concat!("isr1:
 push 0
 push 1
 jmp isr_common
"),
    concat!("isr2:
 push 0
 push 2
 jmp isr_common
"),
    concat!("isr3:
 push 0
 push 3
 jmp isr_common
"),
    concat!("isr4:
 push 0
 push 4
 jmp isr_common
"),
    concat!("isr5:
 push 0
 push 5
 jmp isr_common
"),
    concat!("isr6:
 push 0
 push 6
 jmp isr_common
"),
    concat!("isr7:
 push 0
 push 7
 jmp isr_common
"),
    concat!("isr8:
 push 8
 jmp isr_common
"),
    concat!("isr9:
 push 0
 push 9
 jmp isr_common
"),
    concat!("isr10:
 push 10
 jmp isr_common
"),
    concat!("isr11:
 push 11
 jmp isr_common
"),
    concat!("isr12:
 push 12
 jmp isr_common
"),
    concat!("isr13:
 push 13
 jmp isr_common
"),
    concat!("isr14:
 push 14
 jmp isr_common
"),
    concat!("isr15:
 push 0
 push 15
 jmp isr_common
"),
    concat!("isr16:
 push 0
 push 16
 jmp isr_common
"),
    concat!("isr17:
 push 17
 jmp isr_common
"),
    concat!("isr18:
 push 0
 push 18
 jmp isr_common
"),
    concat!("isr19:
 push 0
 push 19
 jmp isr_common
"),
    concat!("isr20:
 push 0
 push 20
 jmp isr_common
"),
    concat!("isr21:
 push 21
 jmp isr_common
"),
    concat!("isr22:
 push 0
 push 22
 jmp isr_common
"),
    concat!("isr23:
 push 0
 push 23
 jmp isr_common
"),
    concat!("isr24:
 push 0
 push 24
 jmp isr_common
"),
    concat!("isr25:
 push 0
 push 25
 jmp isr_common
"),
    concat!("isr26:
 push 0
 push 26
 jmp isr_common
"),
    concat!("isr27:
 push 0
 push 27
 jmp isr_common
"),
    concat!("isr28:
 push 0
 push 28
 jmp isr_common
"),
    concat!("isr29:
 push 29
 jmp isr_common
"),
    concat!("isr30:
 push 30
 jmp isr_common
"),
    concat!("isr31:
 push 0
 push 31
 jmp isr_common
"),
    concat!("isr32:
 push 0
 push 32
 jmp isr_common
"),
    concat!("isr33:
 push 0
 push 33
 jmp isr_common
"),
    concat!("isr34:
 push 0
 push 34
 jmp isr_common
"),
    concat!("isr35:
 push 0
 push 35
 jmp isr_common
"),
    concat!("isr36:
 push 0
 push 36
 jmp isr_common
"),
    concat!("isr37:
 push 0
 push 37
 jmp isr_common
"),
    concat!("isr38:
 push 0
 push 38
 jmp isr_common
"),
    concat!("isr39:
 push 0
 push 39
 jmp isr_common
"),
    concat!("isr40:
 push 0
 push 40
 jmp isr_common
"),
    concat!("isr41:
 push 0
 push 41
 jmp isr_common
"),
    concat!("isr42:
 push 0
 push 42
 jmp isr_common
"),
    concat!("isr43:
 push 0
 push 43
 jmp isr_common
"),
    concat!("isr44:
 push 0
 push 44
 jmp isr_common
"),
    concat!("isr45:
 push 0
 push 45
 jmp isr_common
"),
    concat!("isr46:
 push 0
 push 46
 jmp isr_common
"),
    concat!("isr47:
 push 0
 push 47
 jmp isr_common
"),
    concat!("isr48:
 push 0
 push 48
 jmp isr_common
"),
    concat!("isr49:
 push 0
 push 49
 jmp isr_common
"),
    concat!("isr50:
 push 0
 push 50
 jmp isr_common
"),
    concat!("isr51:
 push 0
 push 51
 jmp isr_common
"),
    concat!("isr52:
 push 0
 push 52
 jmp isr_common
"),
    concat!("isr53:
 push 0
 push 53
 jmp isr_common
"),
    concat!("isr54:
 push 0
 push 54
 jmp isr_common
"),
    concat!("isr55:
 push 0
 push 55
 jmp isr_common
"),
    concat!("isr56:
 push 0
 push 56
 jmp isr_common
"),
    concat!("isr57:
 push 0
 push 57
 jmp isr_common
"),
    concat!("isr58:
 push 0
 push 58
 jmp isr_common
"),
    concat!("isr59:
 push 0
 push 59
 jmp isr_common
"),
    concat!("isr60:
 push 0
 push 60
 jmp isr_common
"),
    concat!("isr61:
 push 0
 push 61
 jmp isr_common
"),
    concat!("isr62:
 push 0
 push 62
 jmp isr_common
"),
    concat!("isr63:
 push 0
 push 63
 jmp isr_common
"),
    concat!("isr64:
 push 0
 push 64
 jmp isr_common
"),
    concat!("isr65:
 push 0
 push 65
 jmp isr_common
"),
    concat!("isr66:
 push 0
 push 66
 jmp isr_common
"),
    concat!("isr67:
 push 0
 push 67
 jmp isr_common
"),
    concat!("isr68:
 push 0
 push 68
 jmp isr_common
"),
    concat!("isr69:
 push 0
 push 69
 jmp isr_common
"),
    concat!("isr70:
 push 0
 push 70
 jmp isr_common
"),
    concat!("isr71:
 push 0
 push 71
 jmp isr_common
"),
    concat!("isr72:
 push 0
 push 72
 jmp isr_common
"),
    concat!("isr73:
 push 0
 push 73
 jmp isr_common
"),
    concat!("isr74:
 push 0
 push 74
 jmp isr_common
"),
    concat!("isr75:
 push 0
 push 75
 jmp isr_common
"),
    concat!("isr76:
 push 0
 push 76
 jmp isr_common
"),
    concat!("isr77:
 push 0
 push 77
 jmp isr_common
"),
    concat!("isr78:
 push 0
 push 78
 jmp isr_common
"),
    concat!("isr79:
 push 0
 push 79
 jmp isr_common
"),
    concat!("isr255:
 push 0
 push 255
 jmp isr_common
"),
    ".balign 8
.globl isr_table
isr_table:
",
    concat!(".quad isr0
"),
    concat!(".quad isr1
"),
    concat!(".quad isr2
"),
    concat!(".quad isr3
"),
    concat!(".quad isr4
"),
    concat!(".quad isr5
"),
    concat!(".quad isr6
"),
    concat!(".quad isr7
"),
    concat!(".quad isr8
"),
    concat!(".quad isr9
"),
    concat!(".quad isr10
"),
    concat!(".quad isr11
"),
    concat!(".quad isr12
"),
    concat!(".quad isr13
"),
    concat!(".quad isr14
"),
    concat!(".quad isr15
"),
    concat!(".quad isr16
"),
    concat!(".quad isr17
"),
    concat!(".quad isr18
"),
    concat!(".quad isr19
"),
    concat!(".quad isr20
"),
    concat!(".quad isr21
"),
    concat!(".quad isr22
"),
    concat!(".quad isr23
"),
    concat!(".quad isr24
"),
    concat!(".quad isr25
"),
    concat!(".quad isr26
"),
    concat!(".quad isr27
"),
    concat!(".quad isr28
"),
    concat!(".quad isr29
"),
    concat!(".quad isr30
"),
    concat!(".quad isr31
"),
    concat!(".quad isr32
"),
    concat!(".quad isr33
"),
    concat!(".quad isr34
"),
    concat!(".quad isr35
"),
    concat!(".quad isr36
"),
    concat!(".quad isr37
"),
    concat!(".quad isr38
"),
    concat!(".quad isr39
"),
    concat!(".quad isr40
"),
    concat!(".quad isr41
"),
    concat!(".quad isr42
"),
    concat!(".quad isr43
"),
    concat!(".quad isr44
"),
    concat!(".quad isr45
"),
    concat!(".quad isr46
"),
    concat!(".quad isr47
"),
    concat!(".quad isr48
"),
    concat!(".quad isr49
"),
    concat!(".quad isr50
"),
    concat!(".quad isr51
"),
    concat!(".quad isr52
"),
    concat!(".quad isr53
"),
    concat!(".quad isr54
"),
    concat!(".quad isr55
"),
    concat!(".quad isr56
"),
    concat!(".quad isr57
"),
    concat!(".quad isr58
"),
    concat!(".quad isr59
"),
    concat!(".quad isr60
"),
    concat!(".quad isr61
"),
    concat!(".quad isr62
"),
    concat!(".quad isr63
"),
    concat!(".quad isr64
"),
    concat!(".quad isr65
"),
    concat!(".quad isr66
"),
    concat!(".quad isr67
"),
    concat!(".quad isr68
"),
    concat!(".quad isr69
"),
    concat!(".quad isr70
"),
    concat!(".quad isr71
"),
    concat!(".quad isr72
"),
    concat!(".quad isr73
"),
    concat!(".quad isr74
"),
    concat!(".quad isr75
"),
    concat!(".quad isr76
"),
    concat!(".quad isr77
"),
    concat!(".quad isr78
"),
    concat!(".quad isr79
"),
    concat!(".quad isr255
"),
);
