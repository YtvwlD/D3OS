use log::info;
use crate::{apic, cls, current_core_id, debug_cls, init_gdt_for_this_core, install_gs_base, new_core_local_storage, scheduler, scheduler_start, timer, APIC, PREEMPT_COUNT_OFFSET};
use raw_cpuid::CpuId;
use crate::device::apic::Apic;
use crate::process::scheduler;
use crate::process::thread::Thread;
use crate::syscall::syscall_dispatcher;

// First rust function called from assembly boot code for an
// application core
//
#[unsafe(no_mangle)]
pub extern "C" fn startup_ap(cpu_id: u32) {
    //info!("    Application processor executing 'startup_ap'");
    //timer().wait(1000);

    // installs the cpu_id in a cpuLocal struct on the GS segment
    install_gs_base(new_core_local_storage(cpu_id, false));
    init_gdt_for_this_core();
    syscall_dispatcher::init();
    scheduler::cpu_mark_online();

    // Wait until the bootstrap processor has fully initialized the global APIC
    let _apic_ref = APIC.wait();

    info!("    APIC is ready!");
    Apic::enable_local_apic(cls().local_apic());

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

    //debug_cls();

        scheduler().ready(Thread::new_kernel_thread(idle_thread, "idle"));
        scheduler().ready(Thread::new_kernel_thread(idle_thread2, "idle"));

            apic().start_timer(10);
            scheduler_start();
    loop {}
}

//dummyThreads for testing
pub(crate) extern "sysv64" fn idle_thread() {
    loop {
        scheduler().sleep(100);
        //info!("idling..");
        let id = current_core_id();
        let tid = scheduler().current_thread().id();
        //info!("Current core: {}, in thread: {}", id, tid);
        //sleep and block is part of the issue
    }
}

//vecdeque gleiczeitig entfernen und hnzu? locken? -> no

extern "sysv64" fn idle_thread2() {
    loop {
        scheduler().sleep(100);
        //info!("idling.. (but different)");
        let id = current_core_id();
        let tid = scheduler().current_thread().id();
        //info!("Current core: {}, in thread: {}", id, tid);
    }
}
