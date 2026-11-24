typedef unsigned long u64;
typedef void *P;

#define STORE 0x1122334455667788ul
#define KMAP_SLOT 0xffffff8001181648ul
#define MAKE_ENTRY 0xffffff80003633f0ul
#define ENTER_MEM 0xffffff8000344000ul
#define VM_PROTECT 0xffffff80003a2be0ul
#define ALIAS_SLOT 0xffffff8000ff7f88ul
#define TEXT64_LOC 0xffffff800110d1e0ul
#define TEXT32_LOC 0xffffff800110d1e8ul
#define FIXED_BEYOND_MAX 0x2000000001ul
#define JIT 0x4000000000ul
#define VM_MAP_EXEC_RESUME 0xffffff8000350e86ul

typedef int (*entry_t)(P, u64 *, u64, int, P *, P);
typedef int (*prot_t)(P, u64, u64, int, int);
typedef int (*enter_t)(P, u64 *, u64, u64, u64, P, u64, int, int, int, int, void *, unsigned);

static void handles(P *h)
{
	P kmap = *(P *)KMAP_SLOT;
	for (int i = 0; i < 4; i++) {
		u64 alias = *(u64 *)(ALIAS_SLOT + 8 * i);
		u64 size = 4096;
		int text = i >= 2;
		if (text)
			((prot_t)VM_PROTECT)(kmap, alias, 4096, 0, 5);
		((entry_t)MAKE_ENTRY)(kmap, &size, alias, text ? 5 : 1, &h[i], 0);
		if (text)
			((prot_t)VM_PROTECT)(kmap, alias, 4096, 0, 3);
	}
}

void anx_enter(P map, P task, int is64)
{
	P *h = (P *)STORE;
	(void)task;
	if (!h[4]) {
		handles(h);
		h[4] = (P)1;
	}
	int i = is64 ? 1 : 0;
	u64 d = is64 ? 0x7fffffe00000ul : 0xffff0000ul;
	u64 t = is64 ? *(u64 *)TEXT64_LOC : *(unsigned *)TEXT32_LOC;
	enter_t e = (enter_t)ENTER_MEM;
	int k1 = e(map, &d, 4096, 0, FIXED_BEYOND_MAX, h[i], 0, 0, 1, 1, 0, 0, 0);
	int k2 = e(map, &t, 4096, 0, FIXED_BEYOND_MAX | JIT, h[i + 2], 0, 0, 5, 5, 0, 0, 0);
	static const char fmt[] __attribute__((section(".text"))) = "anx enter %d: %d %d %p %p\n";
	((int (*)(const char *, ...))0xffffff8000299590ul)(fmt, is64, k1, k2, h[i], h[i + 2]);
}

__asm__(
	".section .anxcave,\"ax\"\n"
	".globl anx_cave\n"
	"anx_cave:\n"
	"push %rdi\n push %rsi\n push %rdx\n push %rcx\n push %r8\n push %r9\n"
	"sub $8,%rsp\n"
	"call anx_enter\n"
	"add $8,%rsp\n"
	"pop %r9\n pop %r8\n pop %rcx\n pop %rdx\n pop %rsi\n pop %rdi\n"
	"push %rbp\n mov %rsp,%rbp\n push %r15\n"
	"jmp *1f(%rip)\n"
	"1: .quad 0xffffff8000350e86\n"
	".text\n");
__asm__(
	".section .anxcave2,\"ax\"\n"
	".globl anx_cave2\n"
	"anx_cave2:\n"
	"push %rbp\n mov %rsp,%rbp\n push %r15\n push %r14\n push %r13\n push %r12\n push %rbx\n"
	"sub $0x38,%rsp\n"
	"mov 0x18(%rdi),%r10\n"
	"movabs $0x2000000000,%rax\n"
	"test %rax,%r8\n"
	"jz 2f\n"
	"cmp $4096,%rdx\n"
	"jne 2f\n"
	"mov $0xffff0000,%eax\n"
	"cmp %rax,%rsi\n"
	"je 1f\n"
	"movabs $0x7fffffe00000,%rax\n"
	"cmp %rax,%rsi\n"
	"je 1f\n"
	"movabs $0xffffff800110d1e8,%rax\n"
	"mov (%rax),%eax\n"
	"cmp %rax,%rsi\n"
	"je 1f\n"
	"movabs $0xffffff800110d1e0,%rax\n"
	"cmp (%rax),%rsi\n"
	"jne 2f\n"
	"1: lea 4096(%rsi),%rax\n"
	"cmp %rax,%r10\n"
	"cmovb %rax,%r10\n"
	"2: jmp *3f(%rip)\n"
	"3: .quad 0xffffff80003433b5\n"
	".text\n");
