//! Each run's process tree lives in its own Job Object, so Stop (and TopBar exiting, even by crashing) ends
//! everything Claude Code started: shells, `ping -t`, dev servers. `claude.exe` is created suspended, put in
//! the job, then resumed, so nothing it starts can slip out before it's assigned.

use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows::core::Result;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

use crate::win32::Handle;

/// Closing the last handle kills every process in the job (`KILL_ON_JOB_CLOSE`).
pub struct Job(Handle);

impl Job {
    pub fn new() -> Result<Self> {
        // SAFETY: an unnamed job, closed by the guard; the limit struct outlives the call.
        unsafe {
            let job = Handle(CreateJobObjectW(None, None)?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            Ok(Self(job))
        }
    }

    /// Assigns a child created with `CREATE_SUSPENDED` and lets it run.
    pub fn adopt(&self, child: &Child) -> Result<()> {
        let process = HANDLE(child.as_raw_handle());
        // SAFETY: `process` is the child's handle, owned by `child`, which outlives the call.
        unsafe { AssignProcessToJobObject(self.0 .0, process)? };
        resume(child.id())
    }

    /// Ends every process in the job now.
    pub fn kill(&self) {
        // SAFETY: our job handle; terminating an empty job is a no-op.
        let _ = unsafe { TerminateJobObject(self.0 .0, 1) };
    }
}

/// Resumes the threads of a suspended process (std doesn't hand out the main thread's handle).
fn resume(pid: u32) -> Result<()> {
    // SAFETY: the snapshot and thread handles are closed by their guards; `entry.dwSize` is set as required.
    unsafe {
        let snap = Handle(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)?);
        let mut entry = THREADENTRY32 {
            dwSize: size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut ok = Thread32First(snap.0, &mut entry).is_ok();
        while ok {
            if entry.th32OwnerProcessID == pid {
                let thread = Handle(OpenThread(
                    THREAD_SUSPEND_RESUME,
                    false,
                    entry.th32ThreadID,
                )?);
                ResumeThread(thread.0);
            }
            ok = Thread32Next(snap.0, &mut entry).is_ok();
        }
    }
    Ok(())
}
