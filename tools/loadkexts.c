typedef unsigned long u64;

#define LOAD_KEXT 0xffffff80009850f0ul
#define KPRINTF 0xffffff8000299590ul

typedef int (*load_t)(const char *, int, int, unsigned short, unsigned short, void *);

static const char k1[] __attribute__((section(".text"))) = "com.apple.filesystems.tmpfs";
static const char k2[] __attribute__((section(".text"))) = "com.apple.AppleFSCompression.AppleFSCompressionTypeZlib";
static const char k3[] __attribute__((section(".text"))) = "com.apple.AppleFSCompression.AppleFSCompressionTypeDataless";
static const char fmt[] __attribute__((section(".text"))) = "anx loadkext %s: %d\n";

static void one(const char *id)
{
	int r = ((load_t)LOAD_KEXT)(id, 0, 0, 0, 2, 0);
	((int (*)(const char *, ...))KPRINTF)(fmt, id, r);
}

void anx_loadkexts(void)
{
	one(k1);
	one(k2);
	one(k3);
}

__asm__(
	".section .anxcave,\"ax\"\n"
	".globl anx_cave3\n"
	"anx_cave3:\n"
	"push %rdi\n push %rsi\n push %rdx\n push %rcx\n push %r8\n push %r9\n"
	"sub $8,%rsp\n"
	"call anx_loadkexts\n"
	"add $8,%rsp\n"
	"pop %r9\n pop %r8\n pop %rcx\n pop %rdx\n pop %rsi\n pop %rdi\n"
	"push %rbp\n mov %rsp,%rbp\n push %r15\n"
	"jmp *1f(%rip)\n"
	"1: .quad 0xffffff80007e0bc6\n"
	".text\n");
