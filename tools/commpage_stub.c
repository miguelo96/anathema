typedef unsigned long u64;
typedef void *(*pcreate_t)(void *, u64, unsigned);
typedef void *(*mcreate_t)(void *, u64, u64, unsigned);
typedef void (*v_t)(void);

#define PMAP_CREATE_OPTIONS 0xffffff80003d4410ul
#define VM_MAP_CREATE_OPTIONS 0xffffff80003416f0ul
#define COMMPAGE_POPULATE 0xffffff8000402040ul
#define COMMPAGE_TEXT_POPULATE 0xffffff8000402ad0ul
#define MAP32 0xffffff80010f4eb8ul
#define MAP64 0xffffff80010f4ec0ul
#define TEXT32 0xffffff80010f4ec8ul
#define TEXT64 0xffffff80010f4ed0ul
#define AREA 0x200000ul
#define RANDOM 0xffffff800045cdb0ul
#define LOC32 0xffffff800110d1e8ul
#define LOC64 0xffffff800110d1e0ul

static void *mk(void)
{
	pcreate_t pc = (pcreate_t)PMAP_CREATE_OPTIONS;
	mcreate_t mc = (mcreate_t)VM_MAP_CREATE_OPTIONS;
	unsigned char *m = (unsigned char *)mc(pc(0, 0, 0), 0, AREA, 0);
	/* vmmap_sealed = VM_MAP_WILL_BE_SEALED: a 2 bit field at bit 0x5d5 of struct vm_map */
	m[0xba] = (unsigned char)((m[0xba] & ~0x60) | 0x20);
	return m;
}

__attribute__((section(".text.a"))) void anx_commpage_init(void)
{
	*(void **)MAP32 = mk();
	*(void **)MAP64 = mk();
	((v_t)COMMPAGE_POPULATE)();
}

__attribute__((section(".text.b"), aligned(256))) void anx_commpage_text_init(void)
{
	unsigned (*rnd)(void) = (unsigned (*)(void))RANDOM;
	*(unsigned *)LOC32 = 0xffff1000u + (rnd() % 14u) * 4096u;
	*(u64 *)LOC64 = 0x7fffffe01000ul + (u64)(rnd() % 510u) * 4096ul;
	*(void **)TEXT32 = mk();
	*(void **)TEXT64 = mk();
	((v_t)COMMPAGE_TEXT_POPULATE)();
}
