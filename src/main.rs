use std::ffi::CStr;

use crate::{mem_info::MemoryInfo, process::Process, utils::clear_u8};

mod daemon;
mod error;
mod kill;
mod linux_version;
mod mem_info;
mod monitor;
mod process;
mod uname;
mod utils;

fn main() -> error::Result<()> {
    let uname_data = uname::UnameData::gather()?;
    let version = uname_data.version();
    dbg!(&version);

    println!("{}", MemoryInfo::new()?);

    // In order to correctly use `mlockall`, we'll try our best to avoid heap allocations and
    // reuse these buffers right here, even though it makes the code less readable
    
    // Buffer specific to process creation 
    let mut proc_buf = [0_u8; 50];
    // Buffer for anything else
    let mut buf = [0_u8; 200];

    let process = Process::this(&mut buf);
    // {
    //     dbg!(process.comm_wip(&mut buf)?);   
    // }
    let oom_score = process.oom_score;
    dbg!(oom_score);
    let oom_score = Process::oom_score_from_pid(process.pid, &mut buf)?;
    dbg!(oom_score);
    // daemon::daemonize()?;

    println!("Daemon started successfully");
    println!("{}", uname_data);

    let victim = kill::choose_victim(&mut proc_buf, &mut buf)?;
    kill::kill_and_wait(victim)?;
    Ok(())
}
