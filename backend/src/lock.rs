//! Poison-tolerant locking.
//!
//! `Mutex::lock()` returns `Err` once any thread panicked while holding that
//! mutex, and the idiomatic `.unwrap()` then panics in every later caller: one
//! bad item or one failed model load takes the whole API down until a restart.
//! That is exactly what happened here (2026-10-18): the inspire warm-up thread
//! panicked on a missing ONNX Runtime library, the vault connection mutex was
//! poisoned with it, and every request afterwards panicked at `with_conn`.
//!
//! The data behind these locks is a SQLite connection or an ONNX session. Both
//! are usable after a panic: SQLite rolls the aborted statement back and leaves
//! the file consistent. So recovering the guard is correct — the panic is
//! already reported by the thread that caused it, and a poisoned lock must not
//! turn a local failure into a permanent outage.

use std::sync::{Mutex, MutexGuard};

pub trait LockExt<T> {
    /// Lock, ignoring poisoning.
    fn lock_ok(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_ok(&self) -> MutexGuard<'_, T> {
        match self.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_panicked_holder_does_not_kill_later_callers() {
        let m = Arc::new(Mutex::new(7u32));
        let other = Arc::clone(&m);
        let _ = std::thread::spawn(move || {
            let _g = other.lock().unwrap(); // the real lock: this is what poisons
            panic!("boom while holding the lock");
        })
        .join(); // the panic happens on that thread, the mutex is now poisoned
        assert!(m.lock().is_err(), "the mutex should be poisoned");
        assert_eq!(*m.lock_ok(), 7, "lock_ok must still hand out the guard");
    }
}
