use log::info;
use crate::{apic, cls, scheduler, WorkItem, APIC};
use crate::device::apic::Apic;
use crate::process::core_local_storage::{current_core_id, init_gdt_for_this_core, install_gs_base, scheduler_start};
use crate::process::scheduler;
use crate::process::scheduler::{read_rq_len, schedule_on};
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
    install_gs_base(cpu_id, false);
    init_gdt_for_this_core();
    syscall_dispatcher::init();
    scheduler::cpu_mark_online();

    // Wait until the bootstrap processor has fully initialized the global APIC
    let _apic_ref = APIC.wait();

    //info!("    APIC is ready!");
    Apic::enable_local_apic(cls().local_apic());

    //debug_cls();

    //scheduler().ready(Thread::new_kernel_thread(idle_thread, "idle"));
    scheduler().ready(Thread::new_kernel_thread(debug_thread2, "debug2"));
    //scheduler().ready(Thread::new_kernel_thread(idle_thread2, "idle"));

    info!("Starting scheduler{}",cpu_id);
    apic().start_timer(10);
    scheduler_start();

    loop {}
}

//dummyThreads for testing
pub(crate) extern "sysv64" fn idle_thread() {
    loop {
        scheduler().sleep(1000);
        //info!("idling..");
        let id = current_core_id();
        //let tid = scheduler().current_thread().id();
        //info!("Current core: {}, in thread: {}", id, tid);
        //sleep and block is part of the issue

        if id == 2 {
            let thread = Thread::new_kernel_thread(idle_thread2, "idle");
            let tid = thread.id();

            if let Ok(r) = schedule_on(3,WorkItem::new(thread)){
                info!("Thread sent: id: {}", tid);
            }
            loop{}
        }
        else if id == 1 {
            scheduler().ready(Thread::new_kernel_thread(idle_thread2, "idle"));
            scheduler().ready(Thread::new_kernel_thread(idle_thread2, "idle"));
            loop{}
        }
    }
}

extern "sysv64" fn idle_thread2() {
    loop {
        info!("idling.. (but different) - id: {:?} - rq_len: {}",
            current_core_id(), read_rq_len());
        scheduler().sleep(1000);
        let mut seven = 700000000;
        while seven > 0 {
            seven = seven -1;
        }

        /*let id = current_core_id();
        let tid = scheduler().current_thread().id();
        info!("Current core: {}, in thread: {}", id, tid);
*/
    }
}

pub(crate) extern "sysv64" fn debug_thread() {
    loop {
        scheduler().sleep(1000);
        info!("debug thread");
        scheduler().debug_scheduler();

        /*let id = current_core_id();
        let tid = scheduler().current_thread().id();
        info!("Current core: {}, in thread: {}", id, tid);
*/
    }
}

pub(crate) extern "sysv64" fn debug_thread2() {
    loop {
        scheduler().sleep(1000);
        info!("debug thread2");
        scheduler().debug_scheduler();

        /*let id = current_core_id();
        let tid = scheduler().current_thread().id();
        info!("Current core: {}, in thread: {}", id, tid);
*/
    }
}