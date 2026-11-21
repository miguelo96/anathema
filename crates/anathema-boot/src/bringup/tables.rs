use super::isr::Pointer;
use uefi::boot::MemoryType;
use uefi::mem::memory_map::MemoryMap;

pub static GDT: [u64; 3] = [0, 0x00af_9a00_0000_ffff, 0x00cf_9200_0000_ffff];

pub fn gdt_pointer() -> Pointer {
    Pointer { limit: (3 * 8 - 1) as u16, base: GDT.as_ptr() as u64 }
}

pub unsafe fn load_gdt() {
    let pointer = gdt_pointer();
    core::arch::asm!(
        "lgdt [{p}]",
        "push 0x08",
        "lea {t}, [rip + 2f]",
        "push {t}",
        "retfq",
        "2:",
        "mov ax, 0x10",
        "mov ds, ax",
        "mov es, ax",
        "mov ss, ax",
        "xor eax, eax",
        "mov fs, ax",
        "mov gs, ax",
        p = in(reg) &pointer,
        t = out(reg) _,
        out("rax") _,
    );
}

pub unsafe fn load_idt() {
    let pointer = super::isr::idt_pointer();
    core::arch::asm!("lidt [{p}]", p = in(reg) &pointer);
}

pub const TABLE_PAGES: usize = 8;
const PRESENT: u64 = 1;
const WRITE: u64 = 2;
const HUGE: u64 = 0x80;
const UNCACHED: u64 = 0x18;
const GIB: u64 = 1 << 30;
const MIB2: u64 = 1 << 21;
const LIMIT: u64 = 512 * GIB;

pub struct Ram {
    pub ranges: [(u64, u64); 256],
    pub count: usize,
    pub top: u64,
}

impl Ram {
    pub fn collect(map: &impl MemoryMap) -> Ram {
        let mut ram = Ram { ranges: [(0, 0); 256], count: 0, top: 0 };
        for desc in map.entries() {
            let ty = desc.ty;
            let ram_like = matches!(
                ty,
                MemoryType::LOADER_CODE
                    | MemoryType::LOADER_DATA
                    | MemoryType::BOOT_SERVICES_CODE
                    | MemoryType::BOOT_SERVICES_DATA
                    | MemoryType::RUNTIME_SERVICES_CODE
                    | MemoryType::RUNTIME_SERVICES_DATA
                    | MemoryType::CONVENTIONAL
                    | MemoryType::UNUSABLE
                    | MemoryType::ACPI_RECLAIM
                    | MemoryType::ACPI_NON_VOLATILE
                    | MemoryType::PERSISTENT_MEMORY
            );
            if !ram_like {
                continue;
            }
            let start = desc.phys_start;
            let end = start + desc.page_count * 4096;
            ram.top = ram.top.max(end);
            if ram.count > 0 && ram.ranges[ram.count - 1].1 == start {
                ram.ranges[ram.count - 1].1 = end;
            } else if ram.count < ram.ranges.len() {
                ram.ranges[ram.count] = (start, end);
                ram.count += 1;
            }
        }
        ram
    }

    pub fn overlaps(&self, start: u64, end: u64) -> bool {
        for i in 0..self.count {
            let (a, b) = self.ranges[i];
            if a < end && start < b {
                return true;
            }
        }
        false
    }
}

pub struct PagingStats {
    pub cr3: u64,
    pub pages_2m: u32,
    pub pages_2m_uc: u32,
    pub pages_1g: u32,
    pub pages_1g_uc: u32,
    pub ram_top: u64,
    pub ram_ranges: u32,
}

pub unsafe fn build_paging(tables: u64, ram: &Ram) -> PagingStats {
    let base = tables as *mut u64;
    core::ptr::write_bytes(base as *mut u8, 0, TABLE_PAGES * 4096);
    let pml4 = base;
    let pdpt = base.add(512);
    *pml4 = (tables + 4096) | PRESENT | WRITE;
    let mut stats = PagingStats {
        cr3: tables,
        pages_2m: 0,
        pages_2m_uc: 0,
        pages_1g: 0,
        pages_1g_uc: 0,
        ram_top: ram.top,
        ram_ranges: ram.count as u32,
    };
    for gib in 0..4u64 {
        let pd = base.add(512 * (2 + gib as usize));
        *pdpt.add(gib as usize) = (tables + 4096 * (2 + gib)) | PRESENT | WRITE;
        for entry in 0..512u64 {
            let address = gib * GIB + entry * MIB2;
            let wb = ram.overlaps(address, address + MIB2);
            let mut value = address | PRESENT | WRITE | HUGE;
            stats.pages_2m += 1;
            if !wb {
                value |= UNCACHED;
                stats.pages_2m_uc += 1;
            }
            *pd.add(entry as usize) = value;
        }
    }
    let mut gib = 4u64;
    while gib * GIB < LIMIT {
        let address = gib * GIB;
        let wb = ram.overlaps(address, address + GIB);
        let mut value = address | PRESENT | WRITE | HUGE;
        stats.pages_1g += 1;
        if !wb {
            value |= UNCACHED;
            stats.pages_1g_uc += 1;
        }
        *pdpt.add(gib as usize) = value;
        gib += 1;
    }
    stats
}

pub unsafe fn load_cr3(cr3: u64) {
    core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
}
