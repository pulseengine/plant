//! Running one command: `/bin/sh -c` in its own process group, combined
//! output to a log file, and the whole group killed on timeout and on exit.

use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::verdict::Outcome;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// On SIGINT/SIGTERM, only set a flag: the run loop then kills the check's
/// group and the caller unwinds, so the temp dir is removed on the way out.
pub fn install_signal_handlers() {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: the handler only stores to an atomic, which is signal-safe.
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

/// True once plant has received SIGINT or SIGTERM.
pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Run `cmd` from `cwd`; returns how it ended and its combined output.
pub fn run(
    cmd: &str,
    cwd: &Path,
    env: &BTreeMap<String, String>,
    timeout: f64,
    log: &Path,
) -> (Outcome, String) {
    let outcome = run_inner(cmd, cwd, env, timeout, log);
    let bytes = std::fs::read(log).unwrap_or_default();
    (outcome, String::from_utf8_lossy(&bytes).into_owned())
}

fn run_inner(
    cmd: &str,
    cwd: &Path,
    env: &BTreeMap<String, String>,
    timeout: f64,
    log: &Path,
) -> Outcome {
    let out = match File::create(log) {
        Ok(f) => f,
        Err(e) => return Outcome::NotStarted(format!("cannot create log: {e}")),
    };
    let err = match out.try_clone() {
        Ok(f) => f,
        Err(e) => return Outcome::NotStarted(format!("cannot create log: {e}")),
    };
    let mut child = match Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Outcome::NotStarted(e.to_string()),
    };
    let pgid = child.id() as libc::pid_t;
    // A timeout too far ahead to represent is no deadline at all.
    let deadline = Instant::now().checked_add(Duration::from_secs_f64(timeout));
    let timed_out = loop {
        if exited_unreaped(pgid) {
            break false;
        }
        if interrupted() || deadline.is_some_and(|d| Instant::now() >= d) {
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // The leader is either alive or an unreaped zombie, so its pid (the group
    // id) cannot have been reused: killing the group hits only its processes.
    kill_group(pgid);
    let status = match child.wait() {
        Ok(s) => s,
        Err(e) => return Outcome::NotStarted(format!("wait failed: {e}")),
    };
    if timed_out {
        Outcome::TimedOut
    } else if let Some(code) = status.code() {
        Outcome::Exited(code)
    } else {
        Outcome::Signaled(status.signal().unwrap_or(0))
    }
}

fn kill_group(pgid: libc::pid_t) {
    // SAFETY: plain syscall; a failure (no such group) is fine to ignore.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
}

/// Has the child exited? Checked without reaping it (WNOWAIT).
fn exited_unreaped(pid: libc::pid_t) -> bool {
    // SAFETY: siginfo_t is plain data; zeroed is a valid initial value.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if rc != 0 {
        // Anything but an interrupted call means waiting cannot continue;
        // report it as exited and let `wait` give the real status.
        return std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR);
    }
    si_pid(&info) != 0
}

#[cfg(target_os = "linux")]
fn si_pid(info: &libc::siginfo_t) -> libc::pid_t {
    // SAFETY: waitid fills the child-status union member.
    unsafe { info.si_pid() }
}

#[cfg(not(target_os = "linux"))]
fn si_pid(info: &libc::siginfo_t) -> libc::pid_t {
    info.si_pid
}
