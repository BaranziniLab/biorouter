use anyhow::{Context, Result};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

pub(super) struct ProcessJob(OwnedHandle);

impl ProcessJob {
    pub fn attach(child: &tokio::process::Child) -> Result<Self> {
        // The owned job handle is never inherited. Closing it terminates every helper descendant, including on future drop.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error())
                .context("computer_use_process_isolation: cannot create process job");
        }
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if configured == 0 {
            return Err(std::io::Error::last_os_error())
                .context("computer_use_process_isolation: cannot configure process job");
        }
        let process = child
            .raw_handle()
            .context("computer_use_process_isolation: helper already exited")?;
        if unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), process) } == 0 {
            return Err(std::io::Error::last_os_error())
                .context("computer_use_process_isolation: cannot assign helper to process job");
        }
        resume_initial_thread(child.id().context("helper exited before resume")?)?;
        Ok(job)
    }

    pub async fn wait_empty(&self) {
        for _ in 0..200 {
            let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION =
                unsafe { std::mem::zeroed() };
            let queried = unsafe {
                QueryInformationJobObject(
                    self.0.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    std::mem::size_of_val(&accounting) as u32,
                    std::ptr::null_mut(),
                )
            };
            if queried == 0 || accounting.ActiveProcesses == 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    pub fn terminate(&self) {
        // This handle owns only the exact helper tree created for this chat connection.
        unsafe {
            TerminateJobObject(self.0.as_raw_handle(), 1);
        }
    }
}

fn resume_initial_thread(pid: u32) -> Result<()> {
    // CREATE_SUSPENDED guarantees the helper cannot spawn an uncontained child before Job assignment.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error())
            .context("computer_use_process_isolation: cannot enumerate initial thread");
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
    while found != 0 {
        if entry.th32OwnerProcessID == pid {
            let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if raw.is_null() {
                return Err(std::io::Error::last_os_error())
                    .context("computer_use_process_isolation: cannot open initial thread");
            }
            let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
            if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                return Err(std::io::Error::last_os_error())
                    .context("computer_use_process_isolation: cannot resume helper");
            }
            return Ok(());
        }
        found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
    }
    anyhow::bail!("computer_use_process_isolation: initial helper thread missing")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, PROCESS_SYNCHRONIZE,
    };

    #[tokio::test]
    #[ignore = "child process fixture, invoked only by the containment test"]
    async fn windows_job_fixture() {
        let marker =
            std::env::var_os("BIOROUTER_JOB_TEST_MARKER").expect("fixture marker required");
        let mut child = tokio::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 60",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        std::fs::write(marker, child.id().unwrap().to_string()).unwrap();
        child.wait().await.unwrap();
    }

    #[tokio::test]
    async fn suspended_startup_contains_and_terminates_real_descendants() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("child.pid");
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "computer_use::windows_job::tests::windows_job_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("BIOROUTER_JOB_TEST_MARKER", &marker)
            .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !marker.exists(),
            "suspended helper executed before containment"
        );
        let job = ProcessJob::attach(&child).unwrap();
        let pid = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(pid) = std::fs::read_to_string(&marker)
                    .ok()
                    .and_then(|text| text.parse::<u32>().ok())
                {
                    break pid;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("contained fixture failed to start descendant");
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!raw.is_null(), "cannot monitor exact descendant process");
        let descendant = unsafe { OwnedHandle::from_raw_handle(raw) };
        drop(job);
        tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(descendant.as_raw_handle(), 5000) },
            WAIT_OBJECT_0,
            "native descendant survived job teardown"
        );
    }
}
