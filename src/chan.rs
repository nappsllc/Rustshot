//! Channels between threads: the part of `std::sync::mpsc` the app uses,
//! same names and behavior, on a `Mutex<VecDeque>` and a `Condvar`.
//!
//! std's channels are generic over three flavors (list, array, zero) with
//! lock-free waiting, and every message type instantiates all of it: about
//! 5 KB a type, 70 KB in all. This is a few hundred bytes a type. Every
//! use is a handful of messages between UI and worker threads, where a
//! lock costs nothing.
//!
//! As std's: `send` fails (giving the value back) once the receiver is
//! gone; receiving fails once the queue is empty and every sender is gone;
//! dropping the receiver drops the queued messages at once (so a reply
//! sender inside one disconnects its reply channel).

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

struct State<T> {
    q: VecDeque<T>,
    senders: usize,
    receiver: bool,
    /// `sync_channel`'s bound (`usize::MAX`: none).
    bound: usize,
}

struct Shared<T> {
    m: Mutex<State<T>>,
    /// Signalled on a send, a disconnect and (bounded) a receive.
    cv: Condvar,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        // A panic while locked cannot leave the queue half-changed.
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The sending half of [`channel`]; clone it for more senders.
pub struct Sender<T> {
    s: Arc<Shared<T>>,
}

/// The sending half of [`sync_channel`]: `send` waits while the queue is
/// full.
#[cfg_attr(not(windows), allow(dead_code))] // WASAPI's
pub struct SyncSender<T> {
    s: Arc<Shared<T>>,
}

/// The receiving half.
pub struct Receiver<T> {
    s: Arc<Shared<T>>,
}

fn new<T>(bound: usize) -> Arc<Shared<T>> {
    Arc::new(Shared {
        m: Mutex::new(State { q: VecDeque::new(), senders: 1, receiver: true, bound }),
        cv: Condvar::new(),
    })
}

/// An unbounded channel.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let s = new(usize::MAX);
    (Sender { s: s.clone() }, Receiver { s })
}

/// A channel holding at most `bound` (at least 1) messages.
#[cfg_attr(not(windows), allow(dead_code))] // WASAPI's
pub fn sync_channel<T>(bound: usize) -> (SyncSender<T>, Receiver<T>) {
    let s = new(bound.max(1));
    (SyncSender { s: s.clone() }, Receiver { s })
}

/// The value of a failed send: the receiver is gone.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SendError<T>(pub T);

/// Nothing more will come: the queue is empty and every sender is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecvError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TryRecvError {
    Empty,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecvTimeoutError {
    Timeout,
    Disconnected,
}

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SendError { .. }")
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sending on a closed channel")
    }
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("receiving on a closed channel")
    }
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TryRecvError::Empty => "receiving on an empty channel",
            TryRecvError::Disconnected => "receiving on a closed channel",
        })
    }
}

impl fmt::Display for RecvTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RecvTimeoutError::Timeout => "timed out waiting on a channel",
            RecvTimeoutError::Disconnected => "receiving on a closed channel",
        })
    }
}

impl<T> std::error::Error for SendError<T> {}
impl std::error::Error for RecvError {}
impl std::error::Error for TryRecvError {}
impl std::error::Error for RecvTimeoutError {}

impl<T> Sender<T> {
    pub fn send(&self, t: T) -> Result<(), SendError<T>> {
        let mut st = self.s.lock();
        if !st.receiver {
            return Err(SendError(t));
        }
        st.q.push_back(t);
        drop(st);
        self.s.cv.notify_all();
        Ok(())
    }
}

#[cfg_attr(not(windows), allow(dead_code))] // WASAPI's
impl<T> SyncSender<T> {
    pub fn send(&self, t: T) -> Result<(), SendError<T>> {
        let mut st = self.s.lock();
        while st.receiver && st.q.len() >= st.bound {
            st = self.s.cv.wait(st).unwrap_or_else(|e| e.into_inner());
        }
        if !st.receiver {
            return Err(SendError(t));
        }
        st.q.push_back(t);
        drop(st);
        self.s.cv.notify_all();
        Ok(())
    }
}

fn clone_sender<T>(s: &Arc<Shared<T>>) -> Arc<Shared<T>> {
    s.lock().senders += 1;
    s.clone()
}

fn drop_sender<T>(s: &Shared<T>) {
    let mut st = s.lock();
    st.senders -= 1;
    let last = st.senders == 0;
    drop(st);
    if last {
        s.cv.notify_all();
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender { s: clone_sender(&self.s) }
    }
}

impl<T> Clone for SyncSender<T> {
    fn clone(&self) -> Self {
        SyncSender { s: clone_sender(&self.s) }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        drop_sender(&self.s);
    }
}

impl<T> Drop for SyncSender<T> {
    fn drop(&mut self) {
        drop_sender(&self.s);
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut st = self.s.lock();
        st.receiver = false;
        let queued = std::mem::take(&mut st.q);
        drop(st);
        self.s.cv.notify_all();
        drop(queued); // outside the lock: a message's drop may use channels
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Sender { .. }")
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Receiver { .. }")
    }
}

impl<T> Receiver<T> {
    /// Take a message from `st`, telling a bounded sender there is room.
    fn take(&self, st: &mut State<T>) -> Option<T> {
        let t = st.q.pop_front()?;
        if st.bound != usize::MAX {
            self.s.cv.notify_all();
        }
        Some(t)
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut st = self.s.lock();
        match self.take(&mut st) {
            Some(t) => Ok(t),
            None if st.senders == 0 => Err(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    pub fn recv(&self) -> Result<T, RecvError> {
        let mut st = self.s.lock();
        loop {
            if let Some(t) = self.take(&mut st) {
                return Ok(t);
            }
            if st.senders == 0 {
                return Err(RecvError);
            }
            st = self.s.cv.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        let Some(deadline) = Instant::now().checked_add(timeout) else {
            return self.recv().map_err(|_| RecvTimeoutError::Disconnected);
        };
        let mut st = self.s.lock();
        loop {
            if let Some(t) = self.take(&mut st) {
                return Ok(t);
            }
            if st.senders == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RecvTimeoutError::Timeout);
            }
            st = self.s.cv.wait_timeout(st, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// The messages there now, without waiting.
    pub fn try_iter(&self) -> impl Iterator<Item = T> + '_ {
        std::iter::from_fn(|| self.try_recv().ok())
    }
}

impl<T> IntoIterator for Receiver<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;
    fn into_iter(self) -> IntoIter<T> {
        IntoIter { rx: self }
    }
}

/// [`Receiver`]'s owning iterator.
pub struct IntoIter<T> {
    rx: Receiver<T>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.rx.recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn messages_arrive_in_order_across_threads() {
        let (tx, rx) = channel();
        let hs: Vec<_> = (0..4)
            .map(|k| {
                let tx = tx.clone();
                thread::spawn(move || {
                    for i in 0..1000 {
                        tx.send((k, i)).unwrap();
                    }
                })
            })
            .collect();
        drop(tx);
        let mut last = [-1i32; 4];
        let mut n = 0;
        for (k, i) in rx {
            assert!(i > last[k], "in order per sender");
            last[k] = i;
            n += 1;
        }
        assert_eq!(n, 4000, "everything, then the end once the senders are gone");
        for h in hs {
            h.join().unwrap();
        }
    }

    #[test]
    fn disconnection() {
        let (tx, rx) = channel::<u8>();
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(rx.recv_timeout(Duration::from_millis(5)), Err(RecvTimeoutError::Timeout));
        tx.send(1).unwrap();
        drop(tx);
        assert_eq!(rx.try_recv(), Ok(1), "queued messages outlive the senders");
        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
        assert_eq!(rx.recv(), Err(RecvError));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Err(RecvTimeoutError::Disconnected));
        assert_eq!(rx.recv_timeout(Duration::MAX), Err(RecvTimeoutError::Disconnected));
        let (tx, rx) = channel::<u8>();
        drop(rx);
        assert_eq!(tx.send(7).unwrap_err().0, 7, "the value comes back");
    }

    #[test]
    fn a_waiting_receiver_wakes() {
        let (tx, rx) = channel();
        let h = thread::spawn(move || rx.recv_timeout(Duration::from_secs(10)));
        thread::sleep(Duration::from_millis(20));
        tx.send("hi").unwrap();
        assert_eq!(h.join().unwrap(), Ok("hi"));
        let (tx, rx) = channel::<()>();
        let h = thread::spawn(move || rx.recv());
        thread::sleep(Duration::from_millis(20));
        drop(tx);
        assert_eq!(h.join().unwrap(), Err(RecvError), "the last sender's drop wakes it");
    }

    #[test]
    fn dropping_the_receiver_drops_queued_messages() {
        // A queued reply sender disconnects its channel when the receiver goes.
        let (tx, rx) = channel();
        let (reply, answer) = channel::<u8>();
        tx.send(reply).unwrap();
        drop(rx);
        assert_eq!(answer.recv(), Err(RecvError));
        assert!(tx.send(channel().0).is_err());
    }

    #[test]
    fn try_iter_takes_what_is_there() {
        let (tx, rx) = channel();
        for i in 0..5 {
            tx.send(i).unwrap();
        }
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert_eq!(rx.try_iter().count(), 0);
        tx.send(9).unwrap();
        drop(tx);
        assert_eq!(rx.into_iter().collect::<Vec<_>>(), vec![9]);
    }

    #[test]
    fn sync_channel_waits_for_room() {
        let (tx, rx) = sync_channel(1);
        tx.send(1).unwrap();
        let h = thread::spawn(move || {
            tx.send(2).unwrap(); // waits until 1 is taken
            tx.send(3).unwrap();
        });
        thread::sleep(Duration::from_millis(20));
        assert_eq!(rx.recv(), Ok(1));
        assert_eq!(rx.recv(), Ok(2));
        assert_eq!(rx.recv(), Ok(3));
        h.join().unwrap();
        assert_eq!(rx.recv(), Err(RecvError));
        let (tx, rx) = sync_channel(1);
        tx.send(1).unwrap();
        let h = thread::spawn(move || tx.send(2));
        thread::sleep(Duration::from_millis(20));
        drop(rx);
        assert_eq!(h.join().unwrap(), Err(SendError(2)), "a waiting sender fails when the receiver goes");
    }
}
