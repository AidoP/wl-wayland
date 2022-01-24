//! Signal handling primitives
//! 
//! As Wayland uses shared memory for efficient sharing of resources between client and compositor, a unique problem is introduced.
//! The client, potentially maliciously, can resize shared memory buffers from under the compositor. Fortunately, in such a case
//! where the original area is still memory mapped, a SIGBUS signal is sent rather than a SIGSEGV, or worse, a erroneous memory access.
//! Signal handling, however, produces it's own set of issues.

use std::{cell::RefCell, rc::Rc};
use libc::*;

use once_cell::sync::OnceCell;
static DEFAULT_SIGBUS_HANDLER: OnceCell<sigaction> = OnceCell::new();
fn init_sigbus_handler() -> sigaction {
    unsafe {
        let mut old = std::mem::MaybeUninit::uninit();
        let mut mask = std::mem::MaybeUninit::uninit();
        sigemptyset(mask.as_mut_ptr());
        let action = sigaction {
            sa_sigaction: sigbus_handler as usize,
            sa_flags: SA_SIGINFO | SA_NODEFER,
            sa_mask: mask.assume_init(),
            sa_restorer: None
        };
        sigaction(SIGBUS, &action, old.as_mut_ptr());
        old.assume_init()
    }
}
thread_local! {
    static SIGBUS_HANDLER_DATA: RefCell<Option<Rc<RefCell<super::ShmPool>>>> = RefCell::new(None);
}

unsafe fn raise_default() {
    // Cannot get to this point if the once_fn was not executed
    sigaction(SIGBUS, DEFAULT_SIGBUS_HANDLER.get_unchecked(), std::ptr::null_mut());
    raise(SIGBUS);
}

// It is not possible to handle a SIGBUS safely
// We have no choice but to rely on undefined behaviour (mainly, the signal-safety of thread_local and mmap)
unsafe extern "C" fn sigbus_handler(_: i32, siginfo: &siginfo_t, _: *const libc::c_void) {
    let addr = siginfo.si_addr();
    // Are closures re-entrant? This is even more dodgy than assuming that backing thread_local implementation is signal-safe
    SIGBUS_HANDLER_DATA.with(|lock| {
        let mut lock = lock.borrow_mut();
        if let Some(pool) = lock.as_mut() {
            let addr = siginfo.si_addr();
            // There is no choice but to potentially violate memory safety
            // As a signal handler, there is no guarantee that the RefCell is in a useable state
            let pool = &mut *pool.as_ptr();
            if addr >= pool.buffer && addr < (pool.buffer as usize + pool.size) as _ {
                pool.did_fault = true;
                // Replace the existing buffer with valid memory so that execution can continue up to the point of dropping the client
                if mmap(pool.buffer, pool.size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_FIXED | MAP_ANONYMOUS, 0, 0) == MAP_FAILED {
                    // There is nothing more that can be done to save the program
                    raise_default()
                }
                return
            } else {
                raise_default()
            }
        } else {
            raise_default()
        }
    })
}
/// Register a lock for the current thread. Returns false if a lock is already installed
pub(super) fn sigbus_lock(pool: Rc<RefCell<super::ShmPool>>) -> bool {
    // Pray the side-effect doesn't get optimised out
    DEFAULT_SIGBUS_HANDLER.get_or_init(init_sigbus_handler);
    SIGBUS_HANDLER_DATA.with(|lock| {
        let mut lock = lock.borrow_mut();
        if lock.is_none() {
            *lock = Some(pool);
            true
        } else {
            false
        }
    })
}
pub(super) fn sigbus_unlock() {
    SIGBUS_HANDLER_DATA.with(|lock| {
        let mut lock = lock.borrow_mut();
        *lock = None;
    })
}