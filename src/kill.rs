use std::collections::HashSet;
use std::fs;
use std::time::Duration;
use std::{ffi::OsStr, time::Instant};

use libc::kill;
use libc::{EINVAL, EPERM, ESRCH, SIGKILL, SIGTERM};

use crate::errno::errno;
use crate::error::{Error, Result};
use crate::process::Process;
use crate::utils;

lazy_static! {
    static ref UNTOUCHABLES: HashSet<&'static str> = {
        let mut hs = HashSet::new();
        hs.insert("qemu-system-x86_64");
        hs.insert("qemu-system-x86");
        hs.insert("emerge");
        hs
    };
}

pub fn choose_victim(mut proc_buf: &mut [u8], mut buf: &mut [u8]) -> Result<Process> {
    let now = Instant::now();


    let processes = fs::read_dir("/proc/")?
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            entry
                .path()
                .file_name()
                .unwrap_or_else(|| &OsStr::new("0"))
                .to_str()
                .unwrap_or_else(|| "0")
                .trim()
                .parse::<u32>()
                .ok()
        })
        .filter(|pid| *pid > 1)
        .filter_map(|pid| Process::from_pid(pid, &mut proc_buf).ok());

    let mut victim = Option::<Process>::None;
    let mut victim_vm_rss_kib = 0;

    for process in processes {
        let comm = process.comm(&mut buf)?;
        if UNTOUCHABLES.contains(comm) {
            println!("skipping: {}", comm);
            continue;
        }

        let cur_vm_rss_kib = match process.vm_rss_kib(&mut buf) {
            Ok(vm_rss_kib) => vm_rss_kib,
            Err(_) => continue,
        };

        if cur_vm_rss_kib == 0 {
            // Current process is a kernel thread
            continue;
        }

        if victim_vm_rss_kib == 0 {
            victim_vm_rss_kib = cur_vm_rss_kib;
        }

        if let Some(victim) = &mut victim {
            if victim.oom_score > process.oom_score {
                // Our current victim is less innocent than the process being analysed
                continue;
            }

            if process.oom_score == victim.oom_score && cur_vm_rss_kib <= victim_vm_rss_kib {
                continue;
            }
        }

        let cur_oom_score_adj = match process.oom_score_adj(&mut buf) {
            Ok(oom_score_adj) => oom_score_adj,
            Err(_) => continue,
        };

        if cur_oom_score_adj == -1000 {
            // Follow the behaviour of the standard OOM killer: don't kill processes with oom_score_adj equals to -1000
            continue;
        }

        eprintln!("[DBG] New victim with PID={}!", process.pid);
        victim = Some(process);
        victim_vm_rss_kib = cur_vm_rss_kib;
    }

    if let Some(victim) = victim {
        println!("[LOG] Found victim in {} secs.", now.elapsed().as_secs());
        println!(
            "[LOG] Victim => pid: {}, comm: {}, oom_score: {}",
            victim.pid, victim.comm(&mut buf)?, victim.oom_score
        );

        Ok(victim)
    } else {
        // Likely an impossible scenario but we found no process to kill!
        Err(Error::ProcessNotFound("choose_victim"))
    }
}

pub fn kill_process(pid: i32, signal: i32) -> Result<()> { 
    let res = unsafe { kill(pid, signal) };

    if res == -1 {
        Err(match errno() {
            // An invalid signal was specified
            EINVAL => Error::InvalidSignal,
            // Calling process doesn't have permission to send signals to any
            // of the target processes
            EPERM => Error::NoPermission,
            // The target process or process group does not exist.
            ESRCH => Error::ProcessNotFound("kill"),
            _ => Error::UnknownKillError,
        })?
    }

    Ok(())
}

pub fn kill_process_group(process: Process) -> Result<()> {
    let pid = process.pid;

    let pgid = utils::get_process_group(pid as i32)?;

    // TODO: kill and wait
    let _ = kill_process(-pgid, SIGTERM);

    Ok(())
}

/// Tries to kill a process and wait for it to exit
/// Will first send the victim a SIGTERM and escalate to SIGKILL if necessary
/// Returns Ok(true) if the victim was successfully terminated
pub fn kill_and_wait(process: Process) -> Result<bool> {
    let pid = process.pid;
    let now = Instant::now();

    let _ = kill_process(pid as i32, SIGTERM);

    let half_a_sec = Duration::from_secs_f32(0.5);
    let mut sigkill_sent = false;

    for _ in 0..20 {
        std::thread::sleep(half_a_sec);
        if !process.is_alive() {
            println!("[LOG] Process with PID {} has exited.\n", pid);
            return Ok(true);
        }
        if !sigkill_sent {
            let _ = kill_process(pid as i32, SIGKILL);
            sigkill_sent = true;
            println!(
                "[LOG] Escalated to SIGKILL after {} nanosecs",
                now.elapsed().as_nanos()
            );
        }
    }

    Ok(false)
}
