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

static void *mk(void)
{
	pcreate_t pc = (pcreate_t)PMAP_CREATE_OPTIONS;
	mcreate_t mc = (mcreate_t)VM_MAP_CREATE_OPTIONS;
	return mc(pc(0, 0, 0), 0, AREA, 0);
}

__attribute__((section(".text.a"))) void anx_commpage_init(void)
{
	*(void **)MAP32 = mk();
	*(void **)MAP64 = mk();
	((v_t)COMMPAGE_POPULATE)();
}

__attribute__((section(".text.b"), aligned(256))) void anx_commpage_text_init(void)
{
	*(void **)TEXT32 = mk();
	*(void **)TEXT64 = mk();
	((v_t)COMMPAGE_TEXT_POPULATE)();
}
