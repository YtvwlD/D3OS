/* ╔═════════════════════════════════════════════════════════════════════════╗
   ║ Module: lib                                                             ║
   ╟─────────────────────────────────────────────────────────────────────────╢
   ║ Descr.: Main rust file of OS. Includes the panic handler as well as all ║
   ║         globals with init functions.                                    ║
   ╟─────────────────────────────────────────────────────────────────────────╢
   ║ Author: Fabian Ruhland & Michael Schoettner, HHU                        ║
   ╚═════════════════════════════════════════════════════════════════════════╝
*/
#![feature(allocator_api)]
#![feature(alloc_layout_extra)]
#![feature(fmt_internals)]
#![feature(abi_x86_interrupt)]
#![feature(map_try_insert)]
#![feature(str_split_remainder)]
#![feature(core_intrinsics)]
#![allow(internal_features)]
#![no_std]

use alloc::boxed::Box;
use crate::device::apic::{get_apic_id, Apic};
use crate::device::cpu::Cpu;
use crate::device::lfb_terminal::{CursorThread, LFBTerminal};
use crate::device::pci::PciBus;
use crate::device::pit::Timer;
use crate::device::ps2::{Keyboard, PS2};
use crate::device::serial;
use crate::device::serial::{BaudRate, ComPort, SerialPort};
use crate::device::speaker::Speaker;
use crate::device::terminal::Terminal;
use crate::interrupt::interrupt_dispatcher::InterruptDispatcher;
use crate::log::Logger;
use crate::memory::PAGE_SIZE;
use crate::memory::acpi_handler::AcpiHandler;
use crate::memory::heap::KernelAllocator;
use crate::process::process_manager::ProcessManager;
use crate::process::scheduler::{debugger_breakpoint_outside_lib, per_cpu_apic_id, per_cpu_ref, per_cpu_ref_curr, read_resched_flag, take_inbox_receiver, Scheduler, WorkItem};
use crate::process::thread::Thread;
use ::log::{Level, Log, Record, error, info};
use acpi::AcpiTables;
use alloc::sync::Arc;
use x86_64::instructions::interrupts;
use core::fmt::Arguments;
use core::hint::spin_loop;
use core::mem::offset_of;
use core::ops::Deref;
use core::panic::PanicInfo;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use multiboot2::ModuleTag;
use raw_cpuid::CpuId;
use spin::{Mutex, Once, RwLock};
use tar_no_std::TarArchiveRef;
use thingbuf::mpsc::Receiver;
use x2apic::lapic::LocalApic;
use x86_64::{PhysAddr, VirtAddr};
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, FS, GS, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::PrivilegeLevel::Ring0;
use x86_64::registers::model_specific::KernelGsBase;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::idt::InterruptDescriptorTable;
use x86_64::structures::paging::PhysFrame;
use x86_64::structures::paging::frame::PhysFrameRange;
use x86_64::structures::tss::TaskStateSegment;

extern crate alloc;
extern crate llfree;

#[macro_use]
pub mod device;
pub mod boot;
pub mod consts;
pub mod interrupt;
pub mod log;
pub mod memory;
pub mod naming;
pub mod network;
pub mod process;
pub mod storage;
pub mod syscall;
pub mod sync;
pub mod ipi;
pub mod boot_ap;

pub mod built_info {
    // The file has been placed there by the build script
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

static PANIC_LOCK: Mutex<()> = Mutex::new(());

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {

    let lock = PANIC_LOCK.lock();
    // make sure we never exit
    interrupts::disable();

    // write the panic directly out to the serial port
    // this needs no allocations, no locks and should always work
    error!("Panic:");
    let args = [info.message().as_str().unwrap_or("(no message provided)")];
    let record = Record::builder()
        .level(Level::Error)
        .file(info.location().map(|l| l.file()))
        .line(info.location().map(|l| l.line()))
        .args(Arguments::new_const(&args))
        .build();

    logger().log(&record);

    // if we do have an allocator already, try to use it to print more information
    // this might fail, but we got the basic information out already
    if allocator().is_initialized() {
        error!("{info}");
        if terminal_initialized() {
            println!("Panic: {}", info);
        }
    }
    drop(lock);

    loop {
        spin_loop();
    }
}

/*
╔═════════════════════════════════════════════════════════════════════════╗
║ Static kernel structures.                                               ║
║ These structures are need for the kernel to work. Since they only exist ║
║ once, they are shared as static lifetime references.                    ║
╚═════════════════════════════════════════════════════════════════════════╝ */

/// CPU caps.
static CPU: Once<Cpu> = Once::new();


pub fn init_cpu_info() {
    CPU.call_once(|| {
        Cpu::new()
    });
}

/// Returns a reference to the CPU info struct.
pub fn cpu() -> &'static Cpu {
    CPU.get()
        .expect("Trying to access CPU info before initialization!")
}


/// Check if EFI system table (and thus runtime services) are available.
pub fn efi_services_available() -> bool {
    uefi::table::system_table_raw().is_some()
}

/// Global Descriptor Table.
/// Needed to set up basic segmentation (flat model) and the TSS.
/// Since multicore is implemented, we need one TSS per core, so it was moved to cls.
//static GDT: Mutex<GlobalDescriptorTable> = Mutex::new(GlobalDescriptorTable::new());

pub fn gdt() -> &'static Mutex<GlobalDescriptorTable> {
    &cls().gdt
}

/// Task State Segment.
/// Needed to set up kernel/user mode switching.
/// Since multicore is implemented, we need one TSS per core, so it was moved to cls.
//static TSS: Mutex<TaskStateSegment> = Mutex::new(TaskStateSegment::new());

pub fn tss() -> &'static Mutex<TaskStateSegment> {
    &cls().tss
}

/// Set up the GDT
/// Initialize the per-core GDT by copying a fixed template layout.
/// Must be called on the current core after GS/CLS is installed and before enabling interrupts/scheduler.
pub fn init_gdt_for_this_core() {
    //use x86_64::PrivilegeLevel::Ring0;

    let cls = cls_mut();
    let mut gdt = cls.gdt.lock();

    // Rebuild a new GDT
    gdt.append(Descriptor::kernel_code_segment()); // selector index 1
    gdt.append(Descriptor::kernel_data_segment()); // selector index 2
    gdt.append(Descriptor::user_data_segment());   // selector index 3
    gdt.append(Descriptor::user_code_segment());   // selector index 4

    unsafe {
        // We need to obtain a static reference to the TSS and GDT for the following operations.
        // We know, that they have a static lifetime, since they are declared as static variables in 'kernel/mod.rs'.
        // However, since they are hidden behind a Mutex, the borrow checker does not see them with a static lifetime.
        let gdt_ref = ptr::from_ref(gdt.deref()).as_ref().unwrap();
        let tss_ref = ptr::from_ref(tss().lock().deref()).as_ref().unwrap();
        gdt.append(Descriptor::tss_segment(tss_ref));
        gdt_ref.load();
    }

    unsafe {
        // Load task state segment
        load_tss(SegmentSelector::new(5, Ring0));

        // Set code and stack segment register
        CS::set_reg(SegmentSelector::new(1, Ring0));
        SS::set_reg(SegmentSelector::new(2, Ring0));

        // Other segment registers are not used in long mode (set to 0)
        DS::set_reg(SegmentSelector::new(0, Ring0));
        ES::set_reg(SegmentSelector::new(0, Ring0));
        FS::set_reg(SegmentSelector::new(0, Ring0));
        GS::set_reg(SegmentSelector::new(0, Ring0));
    }
}

/// Interrupt Descriptor Table.
/// Tells the CPU which interrupt handler to call for each interrupt.
static IDT: Mutex<InterruptDescriptorTable> = Mutex::new(InterruptDescriptorTable::new());

pub fn idt() -> &'static Mutex<InterruptDescriptorTable> {
    &IDT
}


/// Core Local Storage.
/// Contains information, which is needed by the syscall handler.
/// The TSS address is never accessed directly, but via the swapgs instruction.
/// 'boot.rs' sets up the gs base register with a pointer to this struct for the boot processor.
/// Since multicore is implemented, we have one of these per core.
#[repr(C)]
pub struct CoreLocalStorage {
    self_ptr: *const CoreLocalStorage, //easy return through GS Segment with deref (with base)
    tss_rsp0_ptr: VirtAddr,
    user_rsp: VirtAddr,
    id: u32,
    local_apic: Mutex<LocalApic>,
    timer_ticks_per_ms: usize,  // currently unused => needs new calibration method
    preempt_count: AtomicUsize,
    scheduler: Scheduler,
    tss: Mutex<TaskStateSegment>,
    gdt: Mutex<GlobalDescriptorTable>,
    rx: Receiver<Option<WorkItem>>,  // single owner (this core)
}

const PREEMPT_COUNT_OFFSET: usize = offset_of!(CoreLocalStorage, preempt_count);

impl CoreLocalStorage {
    pub fn new(id: u32, kernel_core: bool) -> Self {
        Self {
            self_ptr: 0 as *const CoreLocalStorage,
            tss_rsp0_ptr: VirtAddr::zero(),
            user_rsp: VirtAddr::zero(),
            id: id,
            local_apic: Apic::new_local_apic(kernel_core),
            timer_ticks_per_ms: 0,
            preempt_count: AtomicUsize::new(0),
            scheduler: Scheduler::new(),
            tss: Mutex::new(TaskStateSegment::new()),
            gdt: Mutex::new(GlobalDescriptorTable::new()),
            rx: take_inbox_receiver(id as usize),
        }
    }

    #[inline(always)]
    pub fn local_apic(&self) -> &Mutex<LocalApic> {
        &self.local_apic
    }

    pub fn set_timer_ticks_per_ms(&mut self, ticks: usize) {
        assert_ne!(ticks, 0);
        self.timer_ticks_per_ms = ticks;
    }
}

/// Returns a new CoreLocalStorage Struct with static lifetime
fn new_core_local_storage(id: u32, boot_processor: bool) -> *mut CoreLocalStorage {
    let cpu_local = Box::new(CoreLocalStorage::new(id, boot_processor));
    let addr = Box::leak(cpu_local) as *mut CoreLocalStorage;
    unsafe { (*addr).self_ptr = addr; }
    addr as *mut CoreLocalStorage
}

/// Installs a Cpu Local Sorage on the GS segment
fn install_gs_base(core_local_ptr: *mut CoreLocalStorage) {
    unsafe { (*core_local_ptr).self_ptr = core_local_ptr; }
    KernelGsBase::write(VirtAddr::from_ptr(core_local_ptr));
    set_inbox_apic_id();    //sets the apic_id of the current core in the PER_CPU_SCHED Array
}

// Called only once by each owner core during startup to set the PER_CPU_SCHED apic_id
fn set_inbox_apic_id() {
    let curr_id = current_core_id() as usize;
    let apic_id = get_apic_id();
    per_cpu_ref(curr_id).apic_id.store(apic_id, Ordering::Release);
}

#[inline(always)]
pub fn preempt_disable() {
    cls().preempt_count.fetch_add(1, Ordering::SeqCst);
}

#[inline(always)]
fn preempt_disable_no_swap() {
    let base = read_kernel_gs_base() as *mut u8;
    unsafe {
        let cnt_ptr = base.add(PREEMPT_COUNT_OFFSET) as *mut AtomicUsize;
        (*cnt_ptr).fetch_add(1, Ordering::SeqCst);
    }
}

#[inline(always)]
fn preempt_enable_no_swap() {
    let base = read_kernel_gs_base() as *mut u8;
    let prev_counter;
    unsafe {
        let cnt_ptr = base.add(PREEMPT_COUNT_OFFSET) as *mut AtomicUsize;
        prev_counter = (*cnt_ptr).fetch_sub(1, Ordering::SeqCst);
    }
    debug_assert!(prev_counter > 0);
    // drain possible pending reschedule
    if prev_counter == 1 && read_resched_flag() {
        scheduler().switch_thread_no_interrupt();
    }
}


#[inline(always)]
pub fn preempt_enable() {
    let prev = cls().preempt_count.fetch_sub(1, Ordering::SeqCst);
    debug_assert!(prev > 0);
    // drain possible pending reschedule
    if prev == 1 && read_resched_flag() {
        scheduler().switch_thread_no_interrupt();
    }
}

#[inline(always)]
pub fn preempt_is_disabled() -> bool {
    cls().preempt_count.load(Ordering::SeqCst) != 0
}

#[inline(always)]
fn read_kernel_gs_base() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
        "rdmsr",
        in("ecx") 0xC000_0102u32, // IA32_KERNEL_GS_BASE
        out("eax") lo,
        out("edx") hi,
        options(nomem, nostack, preserves_flags)
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}


// Returns the whole struct from the GS segment
#[inline(always)]
fn cls_ptr_from_gs() -> *mut CoreLocalStorage {
    let struct_ptr: u64;
    unsafe {
        core::arch::asm!(
        "mov {}, gs:[0]",
        out(reg) struct_ptr,
        options(nostack, preserves_flags)
        );
    }
    struct_ptr as *mut CoreLocalStorage
}
#[inline(always)]
pub fn cls_ptr() -> *mut CoreLocalStorage {
    with_kernel_gs( || { cls_ptr_from_gs()})
}
#[inline(always)]
pub fn cls<'a>() -> &'a CoreLocalStorage {
    let cls = cls_ptr();
    debugger_breakpoint_outside_lib();
    unsafe { &*cls_ptr() }
}
#[inline(always)]
pub fn cls_mut<'a>() -> &'a mut CoreLocalStorage {
    unsafe { &mut *cls_ptr() }
}


// Returns the core id from the GS segment after switching back and forth
#[inline(always)]
pub fn current_core_id() -> u32 {
    with_kernel_gs( || { current_core_id_from_gs()})
}

// Returns the core id from the GS segment
#[inline(always)]
pub fn current_core_id_from_gs() -> u32 {
    let id: u32;
    unsafe {
        core::arch::asm!(
        "mov {tmp:e}, gs:[24]",    //id is after 3 pointers (3*8 bytes)
        tmp = out(reg) id,
        options(nostack, preserves_flags, readonly)
        );
    }
    id
}

// wraps the code of another method with "swapgs" calls to get access to the
// kernelGS-Base instead of the GS-Base during execution
// also saves the current IF and restores it afterwards
#[inline(always)]
pub fn with_kernel_gs<R>(f: impl FnOnce() -> R) -> R {
    unsafe {
        //save current rflags to restore interrupt flag afterwards
        let mut rflags: u64;
        core::arch::asm!(
        "pushfq",
        "pop {rflags}",
        rflags = out(reg) rflags,
        options(preserves_flags)
        );
        preempt_disable_no_swap();
        core::arch::asm!("cli", options(nomem, nostack));
        core::arch::asm!("swapgs", options(nomem, nostack, preserves_flags));
        debugger_breakpoint_outside_lib();

        let ret = f();

        // Swap GS back first, then restore IF if it was previously set
        core::arch::asm!("swapgs", options(nomem, nostack, preserves_flags));
        if (rflags & (1 << 9)) != 0 {
            core::arch::asm!("sti", options(nomem, nostack));
        }
        preempt_enable_no_swap();
        ret
    }
}

fn init_tss_cls() {
    let tss_rsp0_ptr =
        VirtAddr::new(ptr::from_ref(tss().lock().deref()) as u64 + size_of::<u32>() as u64);
    cls_mut().tss_rsp0_ptr = tss_rsp0_ptr;
}

fn debug_cls() {

    let cls = cls();
    let tss_rsp0 = cls.tss_rsp0_ptr;
    let user_rsp = cls.user_rsp;
    let id = cls.id;
    let timer_ticks_per_ms = cls.timer_ticks_per_ms;

    if let Some(feat) = CpuId::new().get_feature_info() {
        let has_tsc = feat.has_tsc();
        let has_apic = feat.has_apic();
        let apic_id = feat.initial_local_apic_id();
        info!("\tNew Core {} going online:\n\tTSC: {}    APIC: {}    APIC-id: {}    \
            Ticks/ms: {}\n\tCLS_addr: {:p}    TSS_rsp0: {:p}    user_rsp: {:p}",
        id, has_tsc, has_apic, apic_id, timer_ticks_per_ms, cls_ptr(), tss_rsp0, user_rsp);
        info!(" Cpu ID should be {} and cpuLocal says {}", id, current_core_id());
        info!(" APIC ID should be {} and PER_CPU says {}", apic_id, per_cpu_ref_curr().apic_id.load(Ordering::Acquire));
        info!(" translateCpuToApic: {} and translate says {}", id, per_cpu_apic_id(id as usize));
    }
    //info!("\t Local APIC:", local_apic.lock().deref());
    //cls.scheduler.debug_scheduler();
}

/// ACPI Tables.
/// These contain information about some of the hardware in the system (e.g. the APIC or HPET).
/// 'boot.rs' initializes the global struct by calling 'init_acpi_tables()' after obtaining the RSDP address from the bootloader.
static ACPI_TABLES: Once<Mutex<AcpiTables<AcpiHandler>>> = Once::new();

pub fn init_acpi_tables(rsdp_addr: usize) {
    ACPI_TABLES.call_once(|| {
        let handler = AcpiHandler::default();

        unsafe {
            let tables = AcpiTables::from_rsdp(handler, rsdp_addr);
            match tables {
                Ok(tables) => Mutex::new(tables),
                Err(_) => panic!("Failed to parse ACPI tables"),
            }
        }
    });
}

pub fn acpi_tables() -> &'static Mutex<AcpiTables<AcpiHandler>> {
    ACPI_TABLES
        .get()
        .expect("Trying to access ACPI tables before initialization!")
}

/// Initial Ramdisk.
/// The initial ramdisk is TAR archive, loaded into memory by the bootloader.
/// It contains all programs that D3OS can execute.
/// 'boot.rs' initializes this struct by calling 'init_initrd()' after obtaining the corresponding multiboot2 tag.
static INIT_RAMDISK: Once<TarArchiveRef> = Once::new();

pub fn get_initrd_frames(module: &ModuleTag) -> PhysFrameRange {
    PhysFrameRange {
        start: PhysFrame::from_start_address(PhysAddr::new(module.start_address() as u64))
            .expect("Initial ramdisk is not page aligned"),
        end: PhysFrame::from_start_address(
            PhysAddr::new(module.end_address() as u64).align_up(PAGE_SIZE as u64),
        )
        .unwrap(),
    }
}

pub fn init_initrd(module: &ModuleTag) {
    INIT_RAMDISK.call_once(|| {
        let initrd_bytes = unsafe {
            core::slice::from_raw_parts(
                module.start_address() as *const u8,
                (module.end_address() - module.start_address()) as usize,
            )
        };

        TarArchiveRef::new(initrd_bytes)
            .expect("Failed to create TarArchiveRef from Multiboot2 module")
    });
}

pub fn initrd() -> &'static TarArchiveRef<'static> {
    INIT_RAMDISK
        .get()
        .expect("Trying to access initial ramdisk before initialization!")
}

/// Kernel Allocator.
/// Used for dynamic memory allocation in the kernel.
#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator::new();

pub fn allocator() -> &'static KernelAllocator {
    &ALLOCATOR
}

/// Kernel logger.
/// Used to log kernel messages. During the boot process, log messages are printed to the serial port.
/// 'boot.rs' sets up the log-crate to use this logger, so that macros like 'error!' or 'info!' can be used.
static LOGGER: Once<Logger> = Once::new();

pub fn logger() -> &'static Logger {
    LOGGER.call_once(Logger::new)
}

/// Process Manager.
/// Holds all active processes and allows to create new ones.
static PROCESS_MANAGER: RwLock<ProcessManager> = RwLock::new(ProcessManager::new());

pub fn process_manager() -> &'static RwLock<ProcessManager> {
    &PROCESS_MANAGER
}

/// Scheduler.
/// Manages the execution of threads and switches between them.
/// Allows to access active threads, put threads to sleep, exit/kill threads and creates new ones.
//static SCHEDULER: Once<Scheduler> = Once::new();

pub fn scheduler() -> &'static Scheduler {
    &cls().scheduler
}

// returns without starting if scheduler is already running
pub fn scheduler_start() {
    cls_mut().scheduler.start();
}

/// Interrupt Dispatcher.
/// This dispatcher is called when an interrupt occurs and calls the corresponding interrupt handler.
/// Device drivers can register their interrupt handlers at the dispatcher.
static INTERRUPT_DISPATCHER: Once<InterruptDispatcher> = Once::new();

pub fn interrupt_dispatcher() -> &'static InterruptDispatcher {
    INTERRUPT_DISPATCHER.call_once(InterruptDispatcher::new);
    INTERRUPT_DISPATCHER.get().unwrap()
}

/*
╔═════════════════════════════════════════════════════════════════════════╗
║ Device driver instances.                                                ║
║ We currently do not have a device driver framework, so all driver       ║
║ instances are created here.                                             ║
║ Most device drivers use reference counting, which will (hopefully)      ║
║ make it easier to integrate them into a dynamic driver framework later. ║
║ Our current plan is to use the name service for holding driver          ║
║ instances, allowing us to load/unload device drivers at runtime.        ║
╚═════════════════════════════════════════════════════════════════════════╝ */

/// Advanced Programmable Interrupt Controller.
/// The APIC consists of an IO-APIC for device interrupts and one Local APIC per core.
/// The Local APIC is used to send inter-processor interrupts (IPIs) and to receive interrupts from the IO-APIC.
static APIC: Once<Apic> = Once::new();

pub fn init_apic() {
    APIC.call_once(Apic::new);
}

pub fn apic() -> &'static Apic {
    APIC.get()
        .expect("Trying to access APIC before initialization!")
}

/// Programmable Interval Timer.
/// The timer generates an interrupt each millisecond to keep track of the system time.
/// In the future, we will probably replace it with the HPET or TSC.
static TIMER: Once<Arc<Timer>> = Once::new();

pub fn timer() -> Arc<Timer> {
    TIMER.call_once(|| Arc::new(Timer::new()));
    Arc::clone(TIMER.get().unwrap())
}

/// PC Speaker.
/// A very simple device that generate square waves at a certain frequency, thus creating beep sounds.
static SPEAKER: Once<Arc<Speaker>> = Once::new();

pub fn speaker() -> Arc<Speaker> {
    SPEAKER.call_once(|| Arc::new(Speaker::new()));
    Arc::clone(SPEAKER.get().unwrap())
}

/// Serial Port.
/// Currently only one serial port is initialized. Once we have a driver framework, multiple serial ports can be supported.
/// At the moment, the serial port is only used to print kernel log messages.
static SERIAL_PORT: Once<Arc<SerialPort>> = Once::new();

pub fn init_serial_port() {
    let mut serial: Option<SerialPort> = None;
    if serial::check_port(ComPort::Com1) {
        serial = Some(SerialPort::new(ComPort::Com1, BaudRate::Baud115200, 128));
    } else if serial::check_port(ComPort::Com2) {
        serial = Some(SerialPort::new(ComPort::Com2, BaudRate::Baud115200, 128));
    } else if serial::check_port(ComPort::Com3) {
        serial = Some(SerialPort::new(ComPort::Com3, BaudRate::Baud115200, 128));
    } else if serial::check_port(ComPort::Com4) {
        serial = Some(SerialPort::new(ComPort::Com4, BaudRate::Baud115200, 128));
    }

    if let Some(s) = serial {
        SERIAL_PORT.call_once(|| Arc::new(s));
    }
}

pub fn serial_port() -> Option<Arc<SerialPort>> {
    match SERIAL_PORT.get() {
        Some(port) => Some(Arc::clone(port)),
        None => None,
    }
}

/// Terminal.
/// The terminal is the main input/output device of the kernel. It can print text to the screen and
/// reads keyboard input. Applications can use the 'read' system call to get keyboard input from the terminal.
static TERMINAL: Once<Arc<dyn Terminal>> = Once::new();

pub fn init_terminal(buffer: *mut u8, pitch: u32, width: u32, height: u32, bpp: u8) {
    let lfb_terminal = Arc::new(LFBTerminal::new(buffer, pitch, width, height, bpp));
    lfb_terminal.clear();
    TERMINAL.call_once(|| lfb_terminal);

    extern "sysv64" fn cursor() {
        let mut cursor_thread = CursorThread::new(terminal());
        cursor_thread.run();
    }
    scheduler().ready(Thread::new_kernel_thread(cursor, "cursor"));
}

pub fn terminal_initialized() -> bool {
    TERMINAL.get().is_some()
}

pub fn terminal() -> Arc<dyn Terminal> {
    let terminal = TERMINAL
        .get()
        .expect("Trying to access terminal before initialization!");
    Arc::clone(terminal)
}

/// PS/2 Controller.
/// Used to access PS/2 devices like the keyboard or mouse. Currently only the keyboard is supported.
static PS2: Once<Arc<PS2>> = Once::new();

pub fn keyboard() -> Option<Arc<Keyboard>> {
    PS2.call_once(|| {
        info!("Initializing PS/2 devices");
        let mut ps2 = PS2::new();
        match ps2.init_controller() {
            Ok(_) => match ps2.init_keyboard() {
                Ok(_) => {}
                Err(error) => error!("Keyboard initialization failed: {error:?}"),
            },
            Err(error) => error!("PS/2 controller initialization failed: {error:?}"),
        }

        Arc::new(ps2)
    });

    PS2.get()
        .expect("Trying to access PS/2 devices before initialization!")
        .keyboard()
}

/// PCI Bus.
/// Used to access PCI devices.
/// 'boot.rs' call 'init_pci()' to scan the PCI bus and initialize this struct.
static PCI: Once<PciBus> = Once::new();

pub fn init_pci() {
    PCI.call_once(PciBus::scan);
}

pub fn pci_bus() -> &'static PciBus {
    PCI.get()
        .expect("Trying to access PCI bus before initialization!")
}
