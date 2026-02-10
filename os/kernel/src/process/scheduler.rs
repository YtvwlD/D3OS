/* ╔═════════════════════════════════════════════════════════════════════════╗
   ║ Module: scheduler                                                       ║
   ╟─────────────────────────────────────────────────────────────────────────╢
   ║ Implementation of a basic round-robin scheduler.                        ║
   ║                                                                         ║
   ║ Public functions                                                        ║
   ║   - active_thread_ids      get a list of all active thread IDs          ║
   ║   - current_thread         get the currently running thread             ║
   ║   - current_ids            get the (pid, tid) of the current thread     ║
   ║   - exit                   exit the calling thread                      ║
   ║   - join                   wait for a thread to finish                  ║
   ║   - kill                   kill a thread                                ║
   ║   - set_init               set the scheduler as initialized             ║
   ║   - thread                 get reference to a thread                    ║
   ║   - ready                  insert a thread in the ready queue           ║
   ║   - sleep                  put the caller into sleeping mode            ║
   ║   - start                  start the scheduler                          ║
   ║   - switch_thread_from_interrupt  switch thread, called from interrupt  ║
   ║   - switch_thread_no_interrupt    switch thread, not called from int.   ║
   ║   - current_ids            get the (pid, tid) of the current thread     ║
   ║   - block                  put the calling thread into blocked mode     ║
   ║   - deblock                wake up a blocked thread                     ║
   ╟─────────────────────────────────────────────────────────────────────────╢
   ║ Author: Fabian Ruhland, 05.09.2025, HHU                                 ║
   ╚═════════════════════════════════════════════════════════════════════════╝
*/
use alloc::boxed::Box;
use crate::process::thread::Thread;
use crate::{allocator, apic, cls, current_core_id, preempt_is_disabled, request_reschedule, scheduler, timer, tss};
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::{panic, ptr};
use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering::{Relaxed};
use log::{info, log};
use smallmap::Map;
use spin::{Mutex, MutexGuard, Once};
use thingbuf::mpsc;
use thingbuf::mpsc::{Receiver, Sender};
use crate::device::cpu::{disable_int_nested, enable_int_nested};
use crate::ipi::send_fixed_to_apic;

// thread IDs
pub static THREAD_ID_COUNTER: AtomicUsize = AtomicUsize::new(1);
pub static THREAD_KILLED_COUNTER: AtomicUsize = AtomicUsize::new(1);
static ACTIVE_CPUS: AtomicU32 = AtomicU32::new(1);  //BP automatically

pub fn next_thread_id() -> usize {
    THREAD_ID_COUNTER.fetch_add(1, Relaxed)
}

pub fn thread_removed() -> usize {
    THREAD_KILLED_COUNTER.fetch_add(1, Relaxed)
}
pub fn thread_deblocked() -> usize {
    THREAD_KILLED_COUNTER.fetch_sub(1, Relaxed)
}

pub fn cpu_mark_online() {
    ACTIVE_CPUS.fetch_add(1, Relaxed);
}

pub fn cpu_count() -> u32 {
    ACTIVE_CPUS.load(Relaxed)
}

/// Everything related to the threads in ready state in the scheduler
pub struct ReadyState {
    initialized: bool,
    current_thread: Option<Arc<Thread>>,
    ready_queue: VecDeque<Arc<Thread>>,
}

impl ReadyState {
    pub fn new() -> Self {
        Self {
            initialized: false,
            current_thread: None,
            ready_queue: VecDeque::new(),
        }
    }
}

/// Main struct of the scheduler
pub struct Scheduler {
    ready_state: Mutex<ReadyState>,
    sleep_list: Mutex<Vec<(Arc<Thread>, usize)>>,
    blocked_list: Mutex<Vec<Arc<Thread>>>,
    join_map: Mutex<Map<usize, Vec<Arc<Thread>>>>, // manage which threads are waiting for a thread-id to terminate
    has_started: bool,
}

unsafe impl Send for Scheduler {}
unsafe impl Sync for Scheduler {}

/// Called from assembly code, after the thread has been switched
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unlock_scheduler() {
    unsafe { scheduler().ready_state.force_unlock(); }
}

impl Scheduler {

    /// Create and initialize the scheduler.
    pub fn new() -> Self {
        Self {
            ready_state: Mutex::new(ReadyState::new()),
            sleep_list: Mutex::new(Vec::new()),
            blocked_list: Mutex::new(Vec::new()),
            join_map: Mutex::new(Map::new()),
            has_started: false,
        }
    }

    /// Called after the scheduler has been fully initialized
    pub fn set_init(&self) {
        self.get_ready_state().initialized = true;
    }

    /// returns the number of threads that are currently actively running on this CPU
    /// does not count sleeping threads
    pub fn active_thread_count(&self) -> usize {
        let mut sum: u32 = 0;
        let active = ACTIVE_CPUS.load(Relaxed);
        for i in 0..active {
            let rq = per_cpu_ref(i as usize).rq_len.load(core::sync::atomic::Ordering::Acquire);
            // Detect runaway counters without panicking the whole kernel in debug
            match sum.checked_add(rq) {
                Some(s) => sum = s,
                None => {
                    log::error!("active_thread_count overflow while adding cpu {} rq_len={}", i, rq);
                    return usize::MAX; // saturate to a sentinel
                }
            }
        }
        // include the currently-running thread on each CPU:
        //sum += active;

        // Clamp to usize on 32-bit safely (and give a clear sentinel on 32-bit if it ever overflows)
        if sum > usize::MAX as u32 {
            log::error!("active_thread_count exceeds usize: {}", sum);
            usize::MAX
        } else {
            sum as usize
        }
    }


    /// Get all active thread IDs
    pub fn active_thread_ids(&self) -> Vec<usize> {
        let state = self.get_ready_state();
        let sleep_list = self.sleep_list.lock();

        state.ready_queue.iter()
            .map(|thread| thread.id())
            .collect::<Vec<usize>>()
            .into_iter()
            .chain(sleep_list.iter().map(|entry| entry.0.id()))
            .collect()
    }

    /// Return reference to current thread
    pub fn current_thread(&self) -> Arc<Thread> {
        let state = self.get_ready_state();
        Scheduler::current(&state)
    }

    /// Return reference to current thread and if not possible, then first kernel thread
    pub fn try_current_thread(&self) -> Option<Arc<Thread>> {
        let state = self.get_ready_state();
        Scheduler::try_current(&state)
    }

    /// Return reference to thread identified by `thread_id`
    pub fn thread(&self, thread_id: usize) -> Option<Arc<Thread>> {
        self.ready_state.lock().ready_queue
            .iter()
            .find(|thread| thread.id() == thread_id)
            .cloned()
    }

    /// Return (pid, tid) of current thread
    pub fn current_ids(&self) -> (usize, usize) {
        let tid = self.current_thread().id();
        let pid = self.current_thread().process().id();
        (pid, tid)
    }


    /// Start the scheduler, called only once from `boot.rs`
    pub fn start(&mut self) {
        if self.has_started {
            return;
        }
        self.has_started = true;
        let mut state = self.get_ready_state();
        state.current_thread = state.ready_queue.pop_back();

        unsafe { Thread::start_first(state
            .current_thread.as_ref()
            .expect("Failed to dequeue first thread!").as_ref()
        ); }
    }

    /// Insert `thread` into the ready queue of the scheduler
    pub fn ready(&self, thread: Arc<Thread>) {
        let id = thread.id();

        // If we get the lock on 'self.state' but not on 'self.join_map' the system hangs.
        // The scheduler is not able to switch threads anymore, because of 'self.state' is locked,
        // and we will never be able to get the lock on 'self.join_map'.
        // To solve this, we need to release the lock on 'self.state' in case we do not get
        // the lock on 'self.join_map' and let the scheduler switch threads until we get both locks.
        let (mut state, mut join_map) = loop {
            let state = self.get_ready_state();
            if let Some(join_map) = self.join_map.try_lock() {
                break (state, join_map);
            }
            self.switch_thread_no_interrupt();
        };

        inc_rq_len();
        state.ready_queue.push_front(thread);
        join_map.insert(id, Vec::new());
    }

    /// Put calling thread to sleep for `ms` milliseconds
    pub fn sleep(&self, ms: usize) {
        let mut state = self.get_ready_state();

        if !state.initialized {
            // Scheduler is not initialized yet, so this function has been called during the boot process
            // So we do active waiting
            timer().wait(ms);
        }
        else {
            // Scheduler is initialized, so we can block the calling thread
            let thread = Scheduler::current(&state);
            let wakeup_time = timer().systime_ms() + ms;
            
            {
                // Execute in own block, so that the lock is released automatically (block() does not return)
                let mut sleep_list = self.sleep_list.lock();
                sleep_list.push((thread, wakeup_time));
            }

            self.block_and_switch(&mut state);
            dec_rq_len();
        }
    }

    /// Put calling thread to block
    pub fn block(&self) {
        let mut state = self.get_ready_state();

        if !state.initialized {
            // Scheduler is not initialized yet, so this function has been called during the boot process
            // We panic
            panic!("Scheduler: Cannot block thread before scheduler is initialized!");
        }
        else {
            // Scheduler is initialized, so we can block the calling thread
            let thread = Scheduler::current(&state);
            {
                // Execute in own block, so that the lock is released automatically (block() does not return)
                let mut block_list = self.blocked_list.lock();
                block_list.push(thread);
            } // drop lock for block_list
            //info!("Scheduler::block: switch to next thread");
            self.block_and_switch(&mut state);
            dec_rq_len();
        }
    }

    /// Requeue thread with `tid` from process with `pid` to the ready queue of the scheduler
    pub fn deblock(&self, pid: usize, tid: usize) {
        let mut block_list = self.blocked_list.lock();

        if let Some(pos) = block_list.iter().position(|thread| thread.id() == tid && thread.process().id() == pid) {
            let thread = block_list.remove(pos);
            self.ready(thread);
            inc_rq_len();
        }
    }

    /// Helper function for switching a thread not caused by an interrupt
    pub fn switch_thread_no_interrupt(&self) {
        self.switch_thread(false);
    }

    /// Helper function for switching a thread caused by an interrupt
    pub fn switch_thread_from_interrupt(&self) {
        self.switch_thread(true);
    }

    /// Calling thread will block until thread with `thread_id` has terminated
    pub fn join(&self, thread_id: usize) {
        let mut state = self.get_ready_state();
        let thread = Scheduler::current(&state);

        {
            // Execute in own block, so that the lock is released automatically (block() does not return)
            let mut join_map = self.join_map.lock();
            if let Some(join_list) = join_map.get_mut(&thread_id) {
                join_list.push(thread);
                dec_rq_len();
            } else {
                // Joining on a non-existent thread has no effect (i.e. the thread has already finished running)
                return;
            }
        }

        self.block_and_switch(&mut state);
        dec_rq_len();
    }

    /// Exit calling thread.
    pub fn exit(&self) -> ! {
        let mut ready_state;
        let current;

        {
            // Execute in own block, so that join_map is released automatically (block() does not return)
            let state = self.get_ready_state_and_join_map();
            ready_state = state.0;
            let mut join_map = state.1;

            current = Scheduler::current(&ready_state);
            let join_list = join_map.get_mut(&current.id()).expect("Missing join_map entry!");

            for thread in join_list {
                ready_state.ready_queue.push_front(Arc::clone(thread));
                inc_rq_len();
            }

            join_map.remove(&current.id());
        }

        dec_rq_len();
        drop(current); // Decrease Rc manually, because block() does not return
        self.block_and_switch(&mut ready_state);
        unreachable!()
    }

    /// Kill the thread with the id `thread_id`, if it is on the same Core
    /// TODO: implement search for all cores
    pub fn kill(&self, thread_id: usize) {
        {
            // Check if current thread tries to kill itself (illegal)
            let ready_state = self.get_ready_state();
            let current = Scheduler::current(&ready_state);

            if current.id() == thread_id {
                panic!("A thread cannot kill itself!");
            }
        }

        let state = self.get_ready_state_and_join_map();
        let mut ready_state = state.0;
        let mut join_map = state.1;

        let join_list = join_map.get_mut(&thread_id).expect("Missing join map entry!");

        for thread in join_list {
            ready_state.ready_queue.push_front(Arc::clone(thread));
            inc_rq_len();
        }

        join_map.remove(&thread_id);

        let before = ready_state.ready_queue.len();
        ready_state.ready_queue.retain(|thread| thread.id() != thread_id);
        let after = ready_state.ready_queue.len();

        if before != after {
            dec_rq_len();
        }
    }

    /// Gives out current thread id, then calls other debug methods
    pub fn debug_scheduler(&self) {
        let state = self.get_ready_state();
        let sleep_list = self.sleep_list.lock();
        let nested = disable_int_nested();
        let id = current_core_id();
        let nbr_threads = self.active_thread_count() as u32;
        let own_threads = read_rq_len() as u32;
        let nbr_cpus = ACTIVE_CPUS.load(Relaxed);
        info!("Scheduler {}: Current thread: {}", id, Scheduler::current(&state).id());
        info!("Scheduler{}: total_threads: {}, own_threads: {}, cpus: {}",
                id, nbr_threads, own_threads, nbr_cpus);
        info!("Scheduler {}: Ready queue:", id);
        for thread in &state.ready_queue {
            info!("  - {}", thread.id());
        }
        info!("Scheduler {}: Sleep list:", id);
        for thread in sleep_list.iter() {
            info!("  - {}, {}", thread.0.id(), thread.1);
        }
        enable_int_nested(nested);
    }

    /// Debugging function to print all threads in the ready queue.
    pub fn debug_ready_queue(&self) {
        let state = self.get_ready_state();
        let id = current_core_id();
        info!("Scheduler {}: Ready queue:", id);
        for thread in &state.ready_queue {
            info!("  - {}", thread.id());
        }
    }

    /// Debugging function to print all threads in the sleep list.
    pub fn debug_sleep_list(&self) {
        let _state_guard = self.get_ready_state();
        let sleep_list = self.sleep_list.lock();
        let id = current_core_id();
        info!("Scheduler {}: Sleep list:", id);
        for thread in sleep_list.iter() {
            info!("  - {}, {}", thread.0.id(), thread.1);
        }
    }

    /// Block calling thread and switch to next ready thread.
    fn block_and_switch(&self, state: &mut ReadyState) {
        let mut next_thread = state.ready_queue.pop_back();

        {
            // Execute in own block, so that the lock is released automatically (block() does not return)
            let mut sleep_list = self.sleep_list.lock();
            while next_thread.is_none() {
                Scheduler::check_sleep_list(state, &mut sleep_list);
                next_thread = state.ready_queue.pop_back();
            }
        }

        let current = Scheduler::current(state);
        let next = next_thread.unwrap();

        // Thread has enqueued itself into sleep list and waited so long, that it dequeued itself in the meantime
        if current.id() == next.id() {
            return;
        }

        let current_ptr = ptr::from_ref(current.as_ref());
        let next_ptr = ptr::from_ref(next.as_ref());

        state.current_thread = Some(next);
        drop(current); // Decrease Rc manually, because Thread::switch does not return

        unsafe {
            Thread::switch(current_ptr, next_ptr);
        }
    }

    /// Switch from current to next thread (from ready queue). \
    /// If `interrupt` is true, the function is called from an ISR and will send EOI to APIC otherwise not.
    fn switch_thread(&self, interrupt: bool) {
        if let Some(mut state) = self.ready_state.try_lock() {
            if !state.initialized {
                if interrupt {
                    apic().end_of_interrupt();
                }
                return;
            }

            if interrupt {
                if preempt_is_disabled() {
                    request_reschedule();
                    apic().end_of_interrupt();
                    return;
                }
            }
            else {
                if preempt_is_disabled() {
                    return;
                }
            }

            if let Some(mut sleep_list) = self.sleep_list.try_lock() {
                Scheduler::check_sleep_list(&mut state, &mut sleep_list);
            }

            // Get clone of the current thread
            let current = Scheduler::current(&state);

            // Current thread is initializing itself and may not be interrupted
            if current.stacks_locked() || tss().is_locked() {
                if interrupt {
                    apic().end_of_interrupt();
                }
                return;
            }

            // Try to get the next thread from the ready queue
            let next = match state.ready_queue.pop_back() {
                Some(thread) => thread,
                None => {
                    if interrupt {
                        apic().end_of_interrupt();
                    }
                    return;
                },
            };

            let current_ptr = ptr::from_ref(current.as_ref());
            let next_ptr = ptr::from_ref(next.as_ref());

            state.current_thread = Some(next);
            state.ready_queue.push_front(current);

            if interrupt {
                apic().end_of_interrupt();
            }

            unsafe {
                Thread::switch(current_ptr, next_ptr);
            }
        } else {
            if interrupt {
                apic().end_of_interrupt();
            }
        }
    }

    /// Return current running thread
    fn current(state: &ReadyState) -> Arc<Thread> {
        Arc::clone(state.current_thread.as_ref().expect("Trying to access current thread before initialization!"))
    }

    /// Return current running thread or None if not init yet
    fn try_current(state: &ReadyState) -> Option<Arc<Thread>> {
        if state.current_thread.is_some() {
            Some(Arc::clone(state.current_thread.as_ref().unwrap()))
        }
        else {
            None
        }
    }

    /// Check sleep list for threads that need to be waken up
    fn check_sleep_list(state: &mut ReadyState, sleep_list: &mut Vec<(Arc<Thread>, usize)>) {
        let time = timer().systime_ms();

        sleep_list.retain(|entry| {
            if time >= entry.1 {
                state.ready_queue.push_front(Arc::clone(&entry.0));
                inc_rq_len();
                false
            } else {
                true
            }
        });
    }

    /// Helper function returning `ReadyState` of scheduler in a MutexGuard
    fn get_ready_state(&self) -> MutexGuard<'_, ReadyState> {
        let state;

        // We need to make sure, that both the kernel memory manager and the ready queue are currently not locked.
        // Otherwise, a deadlock may occur: Since we are holding the ready queue lock,
        // the scheduler won't switch threads anymore, and none of the locks will ever be released
        loop {
            let state_tmp = self.ready_state.lock();
            if allocator().is_locked() {    //allocator can be locked again, but only on other cores -> no deadlock, but bottleneck
                continue;
            }

            state = state_tmp;
            break;
        }

        state
    }

    /// Description: Helper function returning `ReadyState` and `Map` of scheduler, each in a MutexGuard
    /// switches Thread on fail, loops back after
    fn get_ready_state_and_join_map(&self) -> (MutexGuard<'_, ReadyState>, MutexGuard<'_, Map<usize, Vec<Arc<Thread>>>>) {
        loop {
            let ready_state = self.get_ready_state();
            if let Some(join_map) = self.join_map.try_lock() {
                return (ready_state, join_map);
            } else {
                self.switch_thread_no_interrupt();
            }
        }
    }
}


//// Multicore support  ////

/// Helper struct to store shared public information of a core
#[repr(align(64))]
pub struct PerCpuSched {
    // Approximate runqueue length (owner updates, others read)
    pub rq_len: AtomicU32,
    pub resched_flag: AtomicBool,
    pub tx: Sender<Option<WorkItem>>,    // producers (remote cores)
    pub apic_id: AtomicUsize,
}

/// Wrapper struct to store a thread and possible future meta data
#[derive(Clone)]
pub struct WorkItem { pub thread: Arc<Thread>, }

impl WorkItem { pub fn new(thread: Arc<Thread>) -> Self { Self { thread } } }

// Global table indexed by core_id
static PER_CPU_SCHED: Once<&'static [PerCpuSched]> = Once::new();

static PER_CPU_RX: Once<&'static [Mutex<Option<Receiver<Option<WorkItem>>>>]> = Once::new();

unsafe impl Sync for PerCpuSched {}

pub fn per_cpu_init(cpu_count: usize, capacity: usize) {
    let mut publics = Vec::with_capacity(cpu_count);
    let mut receivers = Vec::with_capacity(cpu_count);

    for _ in 0..cpu_count {
        let (tx, rx) = mpsc::channel::<Option<WorkItem>>(capacity);
        publics.push(PerCpuSched {
            rq_len: AtomicU32::new(0),
            resched_flag: AtomicBool::new(false),
            tx,
            apic_id: AtomicUsize::new(0),
        });
        receivers.push(Mutex::new(Some(rx)));
    }

    let leaked_cpu_slice: &'static [PerCpuSched] = Box::leak(publics.into_boxed_slice());
    PER_CPU_SCHED.call_once(|| leaked_cpu_slice);
    let leaked_receiver_slice: &'static [Mutex<Option<Receiver<Option<WorkItem>>>>]
        = Box::leak(receivers.into_boxed_slice());
    PER_CPU_RX.call_once(|| leaked_receiver_slice);
}

// Called only once by each owner core during startup to move the RX into its CLS
pub fn take_inbox_receiver(id: usize) -> Receiver<Option<WorkItem>> {
    let bank = PER_CPU_RX.get().expect("per_cpu_init not called");
    let mut guard = bank[id].lock();
    guard.take().expect("Receiver already taken")
}

// Returns the apic_id of the core with the given id
pub fn per_cpu_apic_id(cpu_id: usize) -> usize {
    per_cpu_ref(cpu_id).apic_id.load(Ordering::Acquire)
}

// Returns a reference to the PerCpuSched struct of the core with the given id
#[inline]
pub fn per_cpu_ref(id: usize) -> &'static PerCpuSched {
    let slice = PER_CPU_SCHED.get().expect("per_cpu_init not called");
    &slice[id]
}

// Returns a reference to the PerCpuSched struct of the current core
#[inline]
pub fn per_cpu_ref_curr() -> &'static PerCpuSched {
    per_cpu_ref(current_core_id() as usize)
}

// Returns a reference to the sender out of the PerCpuSched struct of the core with the given id
#[inline]
pub fn per_cpu_sender(id: usize) -> &'static Sender<Option<WorkItem>> {
    &per_cpu_ref(id).tx
}

/// Schedules a thread (wrapped in a WorkItem) on a remote core with the target_id
/// Sends a reschedule IPI to wake that core up, if it was idle.
pub fn schedule_on(target_id: usize, item: WorkItem) -> Result<(), WorkItem> {
    let pc = per_cpu_ref(target_id);
    match pc.tx.try_send(Some(item)) {
        Ok(()) => {
            send_reschedule_ipi(target_id);
            Ok(())
        }
        Err(e) => Err(e.into_inner().unwrap()), // unwrap: we sent Some(_)
    }
}

/// Sends a Reschedule IPI to wake the core with the given id up, if it was idle.
fn send_reschedule_ipi(target_id: usize) {
    send_fixed_to_apic(per_cpu_apic_id(target_id),0xf1)
}
/// Owner function to increase the runqueue length of the current core.
pub fn inc_rq_len() {
    let id = current_core_id() as usize;
    //info!("rq increased by 1 on core {}:",id);
    per_cpu_ref(id).rq_len.fetch_add(1, Ordering::Relaxed);
}
/// Owner function to decrease the runqueue length of the current core.
pub fn dec_rq_len() {
    let id = current_core_id() as usize;
    //info!("rq decreased by 1 on core {}:",id);
    per_cpu_ref(id).rq_len.fetch_sub(1, Ordering::Release); // Release for stronger publication before donating
}
/// Owner function to read the runqueue length of the current core.
pub fn read_rq_len() -> u32 {
    per_cpu_ref(current_core_id() as usize).rq_len.load(Ordering::Acquire)
}
/// Owner function to set the reschedule flag of the current core.
pub fn set_resched_flag() {
    per_cpu_ref(current_core_id() as usize).resched_flag.store(true, Ordering::Release);
}
/// Owner function to clear the reschedule flag of the current core.
pub fn clear_resched_flag() {
    per_cpu_ref(current_core_id() as usize).resched_flag.store(false, Ordering::Release);
}
/// Owner function to read the reschedule flag of the current core.
pub fn read_resched_flag() -> bool {
    per_cpu_ref(current_core_id() as usize).resched_flag.load(Ordering::Acquire)
}
/// Remote Reader function to read the runqueue length of the given Core
pub fn read_rq_len_remote(target_id: usize) -> u32 {
    per_cpu_ref(target_id).rq_len.load(Ordering::Acquire)
}

//idle_thread thread that halts the cpu and until it gets woken up by interrupts
extern "sysv64" fn idle_thread () -> () {   //should never return but new_kernel_thread requires it
    loop {
        unsafe { asm!("hlt"); }
    }
}

//TODO: delete this method
pub fn debugger_breakpoint_outside_lib() -> usize {
    return 0;
}
