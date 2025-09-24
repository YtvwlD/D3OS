use alloc::boxed::Box;
use core::any::Any;
use log::info;
use crate::timer;
use raw_cpuid::CpuId;

// First rust function called from assembly boot code for an
// application core
//
#[unsafe(no_mangle)]
pub extern "C" fn startup_ap(cpu_id: u32) {
    //info!("    Application processor executing 'startup_ap'");

    // installs the cpu_id in the cpuLocal struct on the GS segment
    unsafe { install_gs_base(new_cpu_local(cpu_id)); }

    let mut l = 5; //cpu_id;
    loop{
        timer().wait(1000);
        info!("    Application processor still running.. (id:{})", cpu_id);
        //terminal().write_str("Hello World!");
        l -= 1;
        if l ==0 { break;}
    }

    let cpuid = CpuId::new();

    if let Some(feat) = cpuid.get_feature_info() {
        let has_tsc = feat.has_tsc();
        let has_apic = feat.has_apic();
        let id = feat.initial_local_apic_id();
        info!("  Core: {:?}\n\
            TSC: {}    APIC: {}    APIC-id: {}    Cpu-id: {}"
            , cpuid.type_id(), has_tsc, has_apic, id, cpu_id);
        //info!(" Cpu ID should be {} and cpuLocal says {}", cpu_id, current_core_id());
    }

    loop {}
}

#[repr(C, align(64))]
pub struct CpuLocal {
    self_ptr: *const CpuLocal, //convenient at offset 0
    pub id: u32,
}

// Looks through the GS segment to find the current core's CpuLocal
//
// The CpuLocal is stored in the GS segment, which is a special segment
// that is accessible to all cores.
// The CpuLocal is stored at offset 8 in the GS segment.
//
#[inline(always)]
pub fn current_core_id() -> u32 {
    let id: u32;
    unsafe {
        core::arch::asm!(
        "mov {tmp}, gs:[8]",    //id is after self_ptr (8 bytes)
        tmp = out(reg) id,
        options(nostack, preserves_flags, readonly)
        );
    }
    id
}

// installs a Cpu Local Struct on the GS segment
//
// The CpuLocal is stored at offset 8 in the GS segment, which is a special segment
// that is accessible to all cores.
// This function is called from assembly code, so it must be unsafe.
fn install_gs_base(cpu_local_ptr: *mut CpuLocal) {
    unsafe {(*cpu_local_ptr).self_ptr = cpu_local_ptr as *const CpuLocal};

    // Write IA32_GS_BASE MSR with this pointer
    // If you use the x86_64 crate:
    use x86_64::registers::model_specific::GsBase;
    use x86_64::VirtAddr;
    GsBase::write(VirtAddr::from_ptr(cpu_local_ptr));
}

// Allocates a new CpuLocal struct and returns a pointer to it
//
// This function creates a Box and then uses into_raw(),
// which makes its lifetime static
fn new_cpu_local(id: u32) -> *mut CpuLocal {
    let cpu_local = Box::new(CpuLocal {
        self_ptr: 0 as *const CpuLocal,
        id: id,
    });
    Box::into_raw(cpu_local) as *mut CpuLocal
}
