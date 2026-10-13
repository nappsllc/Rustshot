//! A few threads that stay up for the net's lifetime and share the big
//! operators' rows ([`Team::run`]): starting threads per operator would
//! cost more than the operators (a run has about a hundred). The calling
//! thread works too; `run` returns when every part is done, so the job
//! may borrow from the caller's stack.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

type Job = dyn Fn(usize) + Sync;

/// The job of the current run, its lifetime erased: valid until `run`
/// returns, which waits for every part (see [`Team::run`]).
#[derive(Clone, Copy)]
struct JobPtr(*const Job);
// SAFETY: the job is `Sync` and outlives every use (run waits for all).
unsafe impl Send for JobPtr {}

#[derive(Default)]
struct State {
    job: Option<JobPtr>,
    /// Bumped by every run (a worker serves each run once).
    run: u64,
    next: usize,
    parts: usize,
    left: usize,
    panicked: bool,
    quit: bool,
}

#[derive(Default)]
struct Inner {
    st: Mutex<State>,
    go: Condvar,
    done: Condvar,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    /// Take and do parts until none is left; then count them done.
    fn work<'a>(&'a self, mut st: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        while let Some(job) = st.job.filter(|_| st.next < st.parts) {
            let i = st.next;
            st.next += 1;
            drop(st);
            // SAFETY: the job lives until its run's `left` reaches 0,
            // which is after this call (see `JobPtr`).
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe { (*job.0)(i) })).is_ok();
            st = lock(&self.st);
            st.panicked |= !ok;
            st.left -= 1;
            if st.left == 0 {
                self.done.notify_all();
            }
        }
        st
    }
}

pub struct Team {
    inner: Arc<Inner>,
    threads: Vec<JoinHandle<()>>,
}

impl Team {
    /// `helpers` threads besides the caller's (none: everything runs on
    /// the caller); threads that cannot be started are done without.
    pub fn new(helpers: usize) -> Team {
        let inner = Arc::new(Inner::default());
        let threads = (0..helpers)
            .filter_map(|k| {
                let inner = inner.clone();
                std::thread::Builder::new()
                    .name(format!("segment-{k}"))
                    .spawn(move || {
                        let mut seen = 0u64;
                        let mut st = lock(&inner.st);
                        loop {
                            if st.quit {
                                return;
                            }
                            if st.run != seen && st.job.is_some() {
                                seen = st.run;
                                st = inner.work(st);
                                continue;
                            }
                            st = inner.go.wait(st).unwrap_or_else(|e| e.into_inner());
                        }
                    })
                    .ok()
            })
            .collect();
        Team { inner, threads }
    }

    /// The threads working on a run (the caller's included).
    pub fn size(&self) -> usize {
        self.threads.len() + 1
    }

    /// `f(0) .. f(parts - 1)`, spread over the team; returns when all are
    /// done (a part that panicked panics here).
    pub fn run<'a>(&self, parts: usize, f: &'a (dyn Fn(usize) + Sync + 'a)) {
        if parts <= 1 || self.threads.is_empty() {
            (0..parts).for_each(f);
            return;
        }
        // SAFETY (lifetime): `f` is only reached through the state until
        // `left` is 0 below, and the job is cleared before returning.
        let job = JobPtr(unsafe { std::mem::transmute::<*const (dyn Fn(usize) + Sync + 'a), *const Job>(f) });
        let mut st = lock(&self.inner.st);
        st.job = Some(job);
        st.run += 1;
        (st.next, st.parts, st.left, st.panicked) = (0, parts, parts, false);
        self.inner.go.notify_all();
        st = self.inner.work(st);
        while st.left > 0 {
            st = self.inner.done.wait(st).unwrap_or_else(|e| e.into_inner());
        }
        st.job = None;
        let panicked = st.panicked;
        drop(st);
        assert!(!panicked, "a segmentation thread panicked");
    }
}

impl Drop for Team {
    fn drop(&mut self) {
        lock(&self.inner.st).quit = true;
        self.inner.go.notify_all();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// A pointer the parts of a run write through, each to its own rows.
#[derive(Clone, Copy)]
pub struct Rows(pub *mut f32);
// SAFETY: the parts touch disjoint rows (each caller's promise).
unsafe impl Send for Rows {}
unsafe impl Sync for Rows {}

impl Rows {
    /// The pointer (a method, so a closure captures the whole `Rows`,
    /// which is `Sync`, not its field).
    pub fn get(self) -> *mut f32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn every_part_runs_once_on_any_team() {
        for helpers in [0, 1, 3] {
            let team = Team::new(helpers);
            assert_eq!(team.size(), helpers + 1);
            for parts in [0, 1, 2, 7, 64] {
                let hits: Vec<AtomicUsize> = (0..parts).map(|_| AtomicUsize::new(0)).collect();
                // Borrowing from this stack frame is fine: run waits.
                team.run(parts, &|i| {
                    hits[i].fetch_add(1, Ordering::SeqCst);
                });
                assert!(hits.iter().all(|h| h.load(Ordering::SeqCst) == 1), "{helpers} helpers, {parts} parts");
            }
        }
    }

    #[test]
    fn a_panicking_part_is_reported_and_the_team_goes_on() {
        let team = Team::new(2);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| team.run(4, &|i| assert!(i != 2, "part 2"))));
        assert!(r.is_err());
        let n = AtomicUsize::new(0);
        team.run(5, &|_| {
            n.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(n.load(Ordering::SeqCst), 5);
    }
}
