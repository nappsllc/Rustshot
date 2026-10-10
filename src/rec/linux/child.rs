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

/// Make `cmd`'s child die with the calling thread (see the module docs)
/// and, with `fd3`, read that descriptor as its fd 3. `fd3` must stay
/// open until the child is spawned.
pub fn prepare(cmd: &mut Command, fd3: Option<&OwnedFd>) {
    let parent = std::process::id() as c_int;
    let fd3 = fd3.map(|f| f.as_raw_fd());
    // SAFETY: the closure runs between fork and exec and only makes
    // async-signal-safe calls (prctl, getppid, dup2, fcntl).
    unsafe {
        cmd.pre_exec(move || {
            if prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent went away before the death signal was armed.
            if getppid() != parent {
                return Err(std::io::Error::other("the recording stopped"));
            }
            if let Some(fd) = fd3 {
                let r = if fd == SOUND_FD { fcntl(fd, F_SETFD, 0) } else { dup2(fd, SOUND_FD) };
                if r < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}
