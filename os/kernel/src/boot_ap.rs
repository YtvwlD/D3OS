use log::info;
use crate::{apic, cls, scheduler, APIC};
use crate::device::apic::Apic;
use crate::process::core_local_storage::{init_gdt_for_this_core, install_gs_base, scheduler_start};
use crate::process::scheduler;
use crate::syscall::syscall_dispatcher;

/// First rust function called from assembly boot code for an application core
#[unsafe(no_mangle)]
pub extern "C" fn startup_ap(cpu_id: u32) {
    info!("    Application processor executing 'startup_ap'");

    // installs the cpu_id in a cpuLocal struct on the GS segment
    install_gs_base(cpu_id, false);
    init_gdt_for_this_core();
    syscall_dispatcher::init();
    scheduler::cpu_mark_online();

    // Wait until the bootstrap processor has fully initialized the global APIC
    let _apic_ref = APIC.wait();
    Apic::enable_local_apic(cls().local_apic());

    info!("Starting scheduler{}",cpu_id);
    apic().start_timer(10);
    scheduler_start();

    loop {}
}

/// Thread for testing multicore scheduling
#[allow(dead_code)]
pub(crate) extern "sysv64" fn debug_thread() {
    loop {
        info!("debug thread");
        scheduler().debug_scheduler();
        scheduler().sleep(1000);
    }
}