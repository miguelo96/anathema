//! Host-independent tests for the arm64 decoder and the x86-64 emitter.
//!
//! These never execute translated code and need neither QEMU nor objdump, so they run on any
//! host, Apple silicon included: `cargo test -p anathema-xlat --test decode_emit`.
//!
//! Expected values are hand-assembled from the Arm ARM (arm64) and the Intel SDM (x86-64).
//! The full behavioural check against QEMU lives in `tests/diff.rs` and needs x86-64 Linux.

use anathema_xlat::decode::{decode, Addr, ExcKind, Insn, LogOp, Mode, SP};
use anathema_xlat::x86::*;

// ---------------------------------------------------------------- arm64 decoder

#[test]
fn add_sub_immediate() {
    // add x0, x1, #1
    assert_eq!(
        decode(0x9100_0420),
        Insn::AddSubImm { sf: true, sub: false, set: false, rd: 0, rn: 1, imm: 1 }
    );
    // sub x0, x1, #1, lsl #12
    assert_eq!(
        decode(0xD140_0420),
        Insn::AddSubImm { sf: true, sub: true, set: false, rd: 0, rn: 1, imm: 4096 }
    );
}

#[test]
fn register_31_is_sp_or_zr_depending_on_the_instruction() {
    // add sp, sp, #0x10: both operands are SP.
    assert_eq!(
        decode(0x9100_43FF),
        Insn::AddSubImm { sf: true, sub: false, set: false, rd: SP, rn: SP, imm: 16 }
    );
    // cmp x1, #0 == subs xzr, x1, #0: the flag-setting form writes XZR, not SP.
    assert_eq!(
        decode(0xF100_003F),
        Insn::AddSubImm { sf: true, sub: true, set: true, rd: 31, rn: 1, imm: 0 }
    );
}

#[test]
fn move_wide() {
    // movz x0, #0x1234
    assert_eq!(decode(0xD282_4680), Insn::MovW { sf: true, kind: 2, rd: 0, imm16: 0x1234, hw: 0 });
    // movk x0, #0xbeef, lsl #16
    assert_eq!(decode(0xF2B7_DDE0), Insn::MovW { sf: true, kind: 3, rd: 0, imm16: 0xbeef, hw: 1 });
}

#[test]
fn logical_immediate_bitmasks() {
    // and x0, x1, #0xff (N=1, immr=0, imms=7)
    assert_eq!(decode(0x9240_1C20), Insn::LogImm { sf: true, op: LogOp::And, rd: 0, rn: 1, imm: 0xff });
    // orr w0, wzr, #0x55555555 (2-bit element, one bit set)
    assert!(matches!(
        decode(0x3200_F3E0),
        Insn::LogImm { sf: false, op: LogOp::Orr, rd: 0, rn: 31, imm: 0x5555_5555 }
    ));
    // N=1, imms=0b111111 would be "all ones", which is a reserved encoding.
    assert!(matches!(decode(0x9240_FC20), Insn::Unsup(_)));
}

#[test]
fn pc_relative_addresses() {
    assert_eq!(decode(0x1000_0020), Insn::Adr { rd: 0, imm: 4 }); // adr x0, #4
    assert_eq!(decode(0xB000_0000), Insn::Adrp { rd: 0, imm: 0x1000 }); // adrp x0, #0x1000
    assert_eq!(decode(0xF0FF_FFE1), Insn::Adrp { rd: 1, imm: -0x1000 }); // adrp x1, #-0x1000
}

#[test]
fn branches() {
    assert_eq!(decode(0x1400_0002), Insn::B { off: 8 }); // b #8
    assert_eq!(decode(0x17FF_FFFF), Insn::B { off: -4 }); // b #-4
    assert_eq!(decode(0x9400_0040), Insn::Bl { off: 0x100 }); // bl #0x100
    assert_eq!(decode(0x54FF_FFC1), Insn::Bcond { cond: 1, off: -8 }); // b.ne #-8
    assert_eq!(decode(0xB400_0080), Insn::Cbz { sf: true, nz: false, rt: 0, off: 16 }); // cbz x0, #16
    assert_eq!(decode(0x3728_0063), Insn::Tbz { nz: true, rt: 3, bit: 5, off: 12 }); // tbnz w3, #5, #12
    assert_eq!(decode(0xD65F_03C0), Insn::Ret { rn: 30 }); // ret
    assert_eq!(decode(0xD61F_0200), Insn::Br { rn: 16 }); // br x16
    assert_eq!(decode(0xD63F_0100), Insn::Blr { rn: 8 }); // blr x8
}

#[test]
fn branch_terminates_a_block() {
    assert!(decode(0x1400_0002).is_term()); // b
    assert!(decode(0xD65F_03C0).is_term()); // ret
    assert!(!decode(0x9100_0420).is_term()); // add
}

#[test]
fn exceptions_and_system() {
    assert_eq!(decode(0xD400_1001), Insn::Exc { kind: ExcKind::Svc, imm: 0x80 }); // svc #0x80
    assert_eq!(decode(0x0000_0000), Insn::Exc { kind: ExcKind::Udf, imm: 0 }); // udf #0
    assert_eq!(decode(0xD69F_03E0), Insn::Eret);
    assert_eq!(decode(0xD503_201F), Insn::Hint { op: 0 }); // nop
}

#[test]
fn loads_and_stores() {
    // ldr x0, [x1, #8]
    assert!(matches!(
        decode(0xF940_0420),
        Insn::Ld { size: 3, rt: 0, rn: 1, addr: Addr::Imm { off: 8, mode: Mode::Offset }, .. }
    ));
    // stp x29, x30, [sp, #-16]!
    assert!(matches!(
        decode(0xA9BF_7BFD),
        Insn::Stp { rt: 29, rt2: 30, rn: SP, off: -16, mode: Mode::Pre, .. }
    ));
}

/// Review finding: FCVT between single and double has the opcode/ftype pairing swapped.
/// Opcode bits 20:15 name the *destination* type, so `fcvt d0, s1` is ftype 0 with opcode 5.
/// Today the real encodings decode as Unsup (slow but correct, via the interpreter) and the
/// unallocated ones (`0x1e224020`) decode as conversions instead of UNDEFINED.
#[test]
#[ignore = "known bug: FCVT single/double decode (see review notes)"]
fn fcvt_between_single_and_double() {
    assert_eq!(decode(0x1E22_C020), Insn::Fcvt { from: 0, to: 1, rd: 0, rn: 1 }); // fcvt d0, s1
    assert_eq!(decode(0x1E62_4000), Insn::Fcvt { from: 1, to: 0, rd: 0, rn: 0 }); // fcvt s0, d0
    assert!(matches!(decode(0x1E22_4020), Insn::Unsup(_))); // unallocated
}

#[test]
fn every_word_decodes_without_panicking() {
    // The decoder sees whatever bytes the guest executes, including garbage. Walk a spread of
    // words across the whole encoding space (every top-byte, varied low bits) and make sure
    // decoding and the helper queries never panic.
    let mut x: u32 = 0x1234_5678;
    for i in 0..200_000u32 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let w = (i << 24) ^ x;
        let insn = decode(w);
        let _ = insn.is_term();
        let _ = insn.name();
        insn.reg_uses(&mut |_| {});
        insn.vreg_uses(&mut |_| {});
    }
}

// ---------------------------------------------------------------- x86-64 emitter

fn emit(f: impl FnOnce(&mut Asm)) -> Vec<u8> {
    let mut a = Asm::new();
    f(&mut a);
    assert!(a.cold.is_empty());
    a.hot
}

#[test]
fn alu_register_forms() {
    assert_eq!(emit(|a| a.alu_rr(Alu::Add, true, RAX, RCX)), [0x48, 0x01, 0xC8]); // add rax, rcx
    assert_eq!(emit(|a| a.alu_rr(Alu::Xor, false, RAX, RAX)), [0x31, 0xC0]); // xor eax, eax
    assert_eq!(emit(|a| a.alu_rr(Alu::Add, true, R8, R9)), [0x4D, 0x01, 0xC8]); // add r8, r9
}

#[test]
fn alu_immediates_pick_short_or_long_form() {
    assert_eq!(emit(|a| a.alu_ri(Alu::Sub, true, RSP, 8)), [0x48, 0x83, 0xEC, 0x08]); // sub rsp, 8
    assert_eq!(
        emit(|a| a.alu_ri(Alu::Cmp, true, RDX, 0x1000)),
        [0x48, 0x81, 0xFA, 0x00, 0x10, 0x00, 0x00] // cmp rdx, 0x1000
    );
    assert_eq!(emit(|a| a.alu_ri(Alu::And, false, RAX, -1)), [0x83, 0xE0, 0xFF]); // and eax, -1
    assert_eq!(
        emit(|a| a.alu_ri(Alu::Add, false, RAX, 128)),
        [0x81, 0xC0, 0x80, 0x00, 0x00, 0x00] // 128 does not fit in imm8
    );
}

#[test]
fn memory_operands_handle_the_rsp_rbp_r12_r13_special_cases() {
    // mov rax, [rsp+8]: rsp as base needs a SIB byte.
    assert_eq!(emit(|a| a.load(3, false, true, RAX, Mem::b(RSP, 8))), [0x48, 0x8B, 0x44, 0x24, 0x08]);
    // mov rax, [rbp]: rbp with mod=00 means RIP-relative, so a zero disp8 is required.
    assert_eq!(emit(|a| a.load(3, false, true, RAX, Mem::b(RBP, 0))), [0x48, 0x8B, 0x45, 0x00]);
    // r13 has the same low bits as rbp.
    assert_eq!(emit(|a| a.load(3, false, true, RAX, Mem::b(R13, 0))), [0x49, 0x8B, 0x45, 0x00]);
    // r12 has the same low bits as rsp.
    assert_eq!(emit(|a| a.load(3, false, true, RAX, Mem::b(R12, 0))), [0x49, 0x8B, 0x04, 0x24]);
    // mov rax, [rbx + rcx*8 + 0x100]
    assert_eq!(
        emit(|a| a.load(3, false, true, RAX, Mem::bi(RBX, RCX, 3, 0x100))),
        [0x48, 0x8B, 0x84, 0xCB, 0x00, 0x01, 0x00, 0x00]
    );
    // mov rax, [rax + r9]: an extended index register sets REX.X.
    assert_eq!(emit(|a| a.load(3, false, true, RAX, Mem::bi(RAX, R9, 0, 0))), [0x4A, 0x8B, 0x04, 0x08]);
    // lea rax, [rcx*4] (no base): mod=00, base=101 in the SIB, disp32 follows.
    assert_eq!(
        emit(|a| a.lea(true, RAX, Mem { base: NOIDX, index: RCX, scale: 2, disp: 0 })),
        [0x48, 0x8D, 0x04, 0x8D, 0x00, 0x00, 0x00, 0x00]
    );
}

/// Review finding: with neither base nor index, the SIB index field must be 100 ("none").
/// It is `NOIDX & 7` = 111 today, which means [rdi*1 + disp32]. No caller uses this form yet.
#[test]
#[ignore = "known bug: absolute [disp32] operand encodes rdi as index (see review notes)"]
fn absolute_disp32_operand() {
    assert_eq!(
        emit(|a| a.lea(true, RAX, Mem { base: NOIDX, index: NOIDX, scale: 0, disp: 0x1000 })),
        [0x48, 0x8D, 0x04, 0x25, 0x00, 0x10, 0x00, 0x00]
    );
}

#[test]
fn setcc_on_sil_needs_rex() {
    assert_eq!(emit(|a| a.setcc(CC_E, RSI)), [0x40, 0x0F, 0x94, 0xC6]); // sete sil
    assert_eq!(emit(|a| a.mov_rr(true, R8, R12)), [0x4D, 0x89, 0xE0]); // mov r8, r12
}

#[test]
fn loads_extend_correctly() {
    assert_eq!(emit(|a| a.load(0, false, false, RAX, Mem::b(RDI, 0))), [0x0F, 0xB6, 0x07]); // movzx eax, byte [rdi]
    assert_eq!(emit(|a| a.load(1, true, true, RAX, Mem::b(RDI, 0))), [0x48, 0x0F, 0xBF, 0x07]); // movsx rax, word [rdi]
    assert_eq!(emit(|a| a.load(2, true, true, RAX, Mem::b(RDI, 0))), [0x48, 0x63, 0x07]); // movsxd rax, dword [rdi]
    assert_eq!(emit(|a| a.load(2, false, true, RAX, Mem::b(RDI, 0))), [0x8B, 0x07]); // mov eax, [rdi] zero-extends
}

#[test]
fn byte_stores_from_sil_dil_need_a_rex_prefix() {
    // Without REX, register 6 in a byte op means DH, not SIL.
    assert_eq!(emit(|a| a.store(0, Mem::b(RAX, 0), RSI)), [0x40, 0x88, 0x30]); // mov [rax], sil
    assert_eq!(emit(|a| a.store(0, Mem::b(RAX, 0), RCX)), [0x88, 0x08]); // mov [rax], cl
    assert_eq!(emit(|a| a.store(1, Mem::b(RAX, 0), RCX)), [0x66, 0x89, 0x08]); // mov [rax], cx
    assert_eq!(emit(|a| a.store(3, Mem::b(RAX, 0), R8)), [0x4C, 0x89, 0x00]); // mov [rax], r8
}

#[test]
fn mov_immediate_picks_the_shortest_correct_form() {
    assert_eq!(emit(|a| a.mov_ri(RAX, 1)), [0xB8, 0x01, 0x00, 0x00, 0x00]); // mov eax, 1
    assert_eq!(emit(|a| a.mov_ri(R10, 0xFFFF_FFFF)), [0x41, 0xBA, 0xFF, 0xFF, 0xFF, 0xFF]); // mov r10d, -1 (zero-extends)
    assert_eq!(emit(|a| a.mov_ri(RAX, u64::MAX)), [0x48, 0xC7, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]); // sign-extended imm32
    assert_eq!(
        emit(|a| a.mov_ri(RAX, 0xFFFF_FFFF_8000_0000)),
        [0x48, 0xC7, 0xC0, 0x00, 0x00, 0x00, 0x80]
    );
    assert_eq!(
        emit(|a| a.mov_ri(RAX, 0x1_0000_0000)),
        [0x48, 0xB8, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00] // movabs
    );
    assert_eq!(emit(|a| a.mov_ri64(RCX, 1)).len(), 10); // always the 10-byte movabs form
}

#[test]
fn jumps_and_patching() {
    let code = emit(|a| {
        let p = a.jcc8(CC_E);
        a.ret();
        a.ret();
        a.patch8(p);
    });
    assert_eq!(code, [0x74, 0x02, 0xC3, 0xC3]); // je +2 over two rets

    let mut a = Asm::new();
    let p = a.jcc32(CC_NE);
    assert_eq!(p, 2);
    assert_eq!(a.hot, [0x0F, 0x85, 0, 0, 0, 0]);
    let p = a.jmp32();
    assert_eq!((p, &a.hot[6..]), (7, &[0xE9, 0, 0, 0, 0][..]));
}

#[test]
fn indirect_jumps_calls_and_stack() {
    assert_eq!(emit(|a| a.jmp_rm(Rm::R(RAX))), [0xFF, 0xE0]); // jmp rax
    assert_eq!(emit(|a| a.call_rm(Rm::R(R11))), [0x41, 0xFF, 0xD3]); // call r11
    assert_eq!(emit(|a| a.push_r(R15)), [0x41, 0x57]); // push r15
    assert_eq!(emit(|a| a.pop_r(RBX)), [0x5B]); // pop rbx
}
