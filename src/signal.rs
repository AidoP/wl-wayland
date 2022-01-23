//! Signal handling primitives
//! 
//! As Wayland uses shared memory for efficient sharing of resources between client and compositor, a unique problem is introduced.
//! The client, potentially maliciously, can resize shared memory buffers from under the compositor. Fortunately, in such a case
//! where the original area is still memory mapped, a SIGBUS signal is sent rather than a SIGSEGV, or worse, a erroneous memory access.
//! Signal handling, however, produces it's own set of issues.



use std::{sync::atomic::{AtomicBool, Ordering}, mem::MaybeUninit, ops::{Deref, DerefMut}};

use once_cell::sync::OnceCell;

static SIGBUS_WATCHER: OnceCell<SigbusWatcher> = OnceCell::new();

#[derive(Default)]
struct SigbusWatcher {
    gated: AtomicBool,
    fd: i32,
}
impl SigbusWatcher {
    unsafe fn new() -> Self {
        use libc::*;
        let mut signals = std::mem::MaybeUninit::uninit();
        sigemptyset(signals.as_mut_ptr());
        sigaddset(signals.as_mut_ptr(), SIGBUS);
        let signals = signals.assume_init();
        let fd = signalfd(-1, &signals, SFD_CLOEXEC);
        assert!(fd >= 0);
        Self {
            gated: AtomicBool::new(false),
            fd
        }
    }
}
impl Drop for SigbusWatcher {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

// Data structure that allows thread-safe checking of a signal flag and dropping of the flag

/// Gate access to a shared-memory buffer
/// 
/// Allows for safe recovery from SIGBUS signals due to clients maliciously shrinking the backing file of shared memory pools.
/// Individual accesses to shared memory must be gated so that the offending client can be identified. Gates should be used
/// and dropped as soon as possible to prevent other clients from waiting for the lock on the gate.
pub struct SigbusGate<T>(T);
impl<T> SigbusGate<T> {
    pub fn new(t: T) -> Option<Self> {
        let watcher = SIGBUS_WATCHER.get_or_init(|| unsafe { SigbusWatcher::new() });
        let gated = watcher.gated.swap(true, Ordering::Acquire);
        if !gated {
            unsafe { Self::clear_signals(watcher.fd) };
            Some(Self(t))
        } else {
            None
        }
    }
    unsafe fn clear_signals(fd: i32) {
        use libc::*;
        let mut siginfo: MaybeUninit<signalfd_siginfo> = MaybeUninit::uninit();
        // Flush the signals
        // The man pages do not say it is safe to use any other mechanism to clear the buffer
        while read(fd, siginfo.as_mut_ptr() as _, std::mem::size_of::<signalfd_siginfo>()) > 0 {}
    }
    pub fn did_sigbus(self) -> bool {
        use libc::*;
        // An instance of SigbusGate can only exist if SIGBUS_WATCHER has been initialised
        let watcher = SIGBUS_WATCHER.get().unwrap();
        let mut siginfo: MaybeUninit<signalfd_siginfo> = MaybeUninit::uninit();
        let size = std::mem::size_of::<signalfd_siginfo>();
        unsafe {
            if size as isize == read(watcher.fd, siginfo.as_mut_ptr() as _, size) {
                let siginfo = siginfo.assume_init();
                let fd = siginfo.ssi_fd;
                let pid = siginfo.ssi_pid;
                let addr = siginfo.ssi_addr;
                eprintln!("FAULT\n  fd: {fd}\n  pid: {pid}\n  address: {addr}");
                // Assume SIGBUS is the only possible signal
                true
            } else {
                false
            }
        }
    }
}
impl<T> Drop for SigbusGate<T> {
    fn drop(&mut self) {
        // An instance of SigbusGate can only exist if SIGBUS_WATCHER has been initialised
        let watcher = SIGBUS_WATCHER.get().unwrap();
        watcher.gated.store(false, Ordering::Release);
    }
}
impl<T> Deref for SigbusGate<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T> DerefMut for SigbusGate<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}