//! Child processes of a recording (ffmpeg, parec): never through a shell,
//! killed by the kernel when the thread that started them goes away, and
//! (ffmpeg) handed the sound pipe as file descriptor 3.
//!
//! `PR_SET_PDEATHSIG(SIGKILL)` fires when the *thread* that forked the
//! child exits, not the process. Both children are therefore started on a
//! thread that outlives them: the recording worker (`rec_ui::worker`, which
//! opens the parts, runs the session and finishes the file before it
//! returns) or, in the live tests, the test's own thread. A crash or kill
//! of rustshot takes them along; parec would also die of SIGPIPE on its
//! next write.
//!
//! The sound pipe is passed as fd 3 (`pipe:3` for ffmpeg) rather than as
//! a named pipe: no file to create, protect or clean up. The pipe is made
//! close-on-exec, so no other child (parec) inherits it; in ffmpeg's child
//! the read end is `dup2`ed onto 3, which clears the flag there only.

use core::ffi::{c_int, c_ulong};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::Command;

unsafe extern "C" {
    fn prctl(option: c_int, arg2: c_ulong, arg3: c_ulong, arg4: c_ulong, arg5: c_ulong) -> c_int;
    fn getppid() -> c_int;
    fn pipe2(fds: *mut c_int, flags: c_int) -> c_int;
    fn dup2(old: c_int, new: c_int) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, arg: c_int) -> c_int;
}

const PR_SET_PDEATHSIG: c_int = 1;
const SIGKILL: c_ulong = 9;
const O_CLOEXEC: c_int = 0o2_000_000;
const F_SETFD: c_int = 2;
/// "No such process": the error a child reports when rustshot went away
/// before its death signal was armed.
const ESRCH: i32 = 3;

/// The file descriptor ffmpeg reads the sound from (`-i pipe:3`).
pub const SOUND_FD: RawFd = 3;

/// A close-on-exec pipe: (read end, write end).
pub fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1 as c_int; 2];
    // SAFETY: `fds` has room for the two descriptors pipe2 writes.
    if unsafe { pipe2(fds.as_mut_ptr(), O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: both descriptors were just opened and are owned by nobody else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// How the child gets the sound pipe as fd 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fd3 {
    /// No sound pipe.
    None,
    /// The pipe already is fd 3: only its close-on-exec flag is cleared.
    Keep,
    /// `dup2` it onto 3 (the copy has no close-on-exec flag; the original
    /// closes at exec).
    Dup(RawFd),
}

/// The plan for `fd`.
pub fn fd3_plan(fd: Option<RawFd>) -> Fd3 {
    match fd {
        None => Fd3::None,
        Some(SOUND_FD) => Fd3::Keep,
        Some(fd) => Fd3::Dup(fd),
    }
}

/// Make `cmd`'s child die with the calling thread (see the module docs)
/// and, with `fd3`, read that descriptor as its fd 3. `fd3` must stay
/// open until the child is spawned.
pub fn prepare(cmd: &mut Command, fd3: Option<&OwnedFd>) {
    let parent = std::process::id() as c_int;
    let plan = fd3_plan(fd3.map(|f| f.as_raw_fd()));
    // SAFETY: the closure runs in the child between fork and exec, where
    // only async-signal-safe work is allowed: no allocation, no locks, no
    // stdio. It reads two plain integers it captured (`parent`, `plan`)
    // and makes raw system calls only: prctl and getppid (single syscalls;
    // getppid is on POSIX's async-signal-safe list, prctl is a Linux
    // syscall wrapper with no user-space state), fcntl and dup2 (both on
    // the list). Its errors are `io::Error`s built from an errno value
    // (`last_os_error` reads errno; `from_raw_os_error` stores the
    // number): neither allocates, and std hands the number to the parent,
    // where spawn returns it.
    unsafe {
        cmd.pre_exec(move || {
            if prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent went away before the death signal was armed.
            if getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(ESRCH));
            }
            let r = match plan {
                Fd3::None => 0,
                Fd3::Keep => fcntl(SOUND_FD, F_SETFD, 0),
                Fd3::Dup(fd) => dup2(fd, SOUND_FD),
            };
            if r < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::process::Stdio;

    #[test]
    fn fd3_is_dup_ed_unless_it_already_is_three() {
        assert_eq!(fd3_plan(None), Fd3::None);
        assert_eq!(fd3_plan(Some(3)), Fd3::Keep, "dup2(3, 3) would leave close-on-exec set");
        assert_eq!(fd3_plan(Some(7)), Fd3::Dup(7));
        assert_eq!(fd3_plan(Some(0)), Fd3::Dup(0));
    }

    #[test]
    fn the_child_reads_the_pipe_as_fd_3_and_others_do_not_inherit_it() {
        let (r, mut w) = {
            let (r, w) = pipe().unwrap();
            (r, std::fs::File::from(w))
        };
        // cat reads its fd 3: the pipe.
        let mut cmd = Command::new("/bin/cat");
        cmd.arg("/dev/fd/3").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        prepare(&mut cmd, Some(&r));
        let mut child = cmd.spawn().unwrap();
        drop(r);
        w.write_all(b"sound").unwrap();
        drop(w);
        let mut out = String::new();
        child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(out, "sound");
        // A child without the plan has no fd 3 from the pipe (close-on-exec).
        let (r2, w2) = pipe().unwrap();
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "test -e /dev/fd/3 && readlink /dev/fd/3 || echo none"]).stdout(Stdio::piped());
        prepare(&mut cmd, None);
        let o = cmd.output().unwrap();
        let seen = String::from_utf8_lossy(&o.stdout).trim().to_string();
        let ours = std::fs::read_link(format!("/proc/self/fd/{}", r2.as_raw_fd())).unwrap();
        assert_ne!(seen, ours.to_string_lossy(), "the pipe leaked into the child");
        drop((r2, w2));
    }
}
