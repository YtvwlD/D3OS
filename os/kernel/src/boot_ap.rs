use core::any::Any;
use log::info;
use crate::{current_core_id, debug_cls, install_gs_base, new_core_local_storage, timer};
use raw_cpuid::CpuId;
use crate::process::scheduler;

// First rust function called from assembly boot code for an
// application core
//
#[unsafe(no_mangle)]
pub extern "C" fn startup_ap(cpu_id: u32) {
    //info!("    Application processor executing 'startup_ap'");

    // installs the cpu_id in a cpuLocal struct on the GS segment
    install_gs_base(new_core_local_storage(cpu_id));
    scheduler::cpu_mark_online();

    //new local apic
    //start timer

    let mut l = 5; //loop l times;
    loop{
        timer().wait(1000);
        info!("    Application processor still running.. (id:{})", cpu_id);
        //terminal().write_str("Hello World!");
        l -= 1;
        if l ==0 { break;}
    }

    let cpuid = CpuId::new();   //CpuId-Crate

    let curr_cpu_id = current_core_id();

    if let Some(feat) = cpuid.get_feature_info() {
        let has_tsc = feat.has_tsc();
        let has_apic = feat.has_apic();
        let id = feat.initial_local_apic_id();
        info!("  Core: {:?}\n\
            TSC: {}    APIC: {}    APIC-id: {}    Cpu-id: {}"
            , cpuid.type_id(), has_tsc, has_apic, id, curr_cpu_id);
        //info!(" Cpu ID should be {} and cpuLocal says {}", cpu_id, current_core_id());
    }

    debug_cls();
    loop {}
}
