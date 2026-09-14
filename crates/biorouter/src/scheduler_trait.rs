use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::path::PathBuf;

use crate::scheduler::{ScheduledJob, SchedulerError};
use crate::session::Session;

#[async_trait]
pub trait SchedulerTrait: Send + Sync {
    async fn add_scheduled_job(
        &self,
        job: ScheduledJob,
        copy_workflow: bool,
    ) -> Result<(), SchedulerError>;
    async fn schedule_workflow(
        &self,
        workflow_path: PathBuf,
        cron_schedule: Option<String>,
    ) -> anyhow::Result<(), SchedulerError>;
    async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob>;
    /// ⚠ `remove_owned_workflow` asks for the job's workflow file to go with it;
    /// it does not grant that. `scheduler::scheduler_owns_source` decides, so a
    /// caller that cannot tell the scheduler's own copy from a pointer at the
    /// user's workflow can pass `true` without destroying the latter.
    async fn remove_scheduled_job(
        &self,
        id: &str,
        remove_owned_workflow: bool,
    ) -> Result<(), SchedulerError>;
    async fn pause_schedule(&self, id: &str) -> Result<(), SchedulerError>;
    async fn unpause_schedule(&self, id: &str) -> Result<(), SchedulerError>;
    async fn run_now(&self, id: &str) -> Result<String, SchedulerError>;
    async fn sessions(
        &self,
        sched_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, Session)>, SchedulerError>;
    async fn update_schedule(&self, sched_id: &str, new_cron: String)
        -> Result<(), SchedulerError>;
    async fn kill_running_job(&self, sched_id: &str) -> Result<(), SchedulerError>;
    /// Stop a run ONLY while it is still the run in `expected_session_id`.
    ///
    /// Issue #56. A stop is gated on the chat the run is in, and resolving that
    /// chat is a separate read from the kill — so a schedule (whose id is stable
    /// across runs) can start a *different* run, in a different chat, in the
    /// gap. Callers that gated pass the chat they were admitted to; `None` means
    /// the run names no chat. Implementors MUST refuse rather than stop a run
    /// that no longer matches.
    async fn kill_running_job_in_session(
        &self,
        sched_id: &str,
        expected_session_id: Option<&str>,
    ) -> Result<(), SchedulerError>;
    async fn get_running_job_info(
        &self,
        sched_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError>;

    // ── Issue #56: the same mutations, carrying the standing of the request
    //    that armed the schedule (`ScheduledJob::armed_with_private_reach`). ──
    //
    // ⚠ The defaults DROP the standing, and exist so a test double that holds
    // no schedule file need not spell four more methods. The one implementor
    // that persists schedules, `scheduler::Scheduler`, overrides all four, and a
    // test there pins that it does: an implementor that persisted schedules and
    // kept these defaults would let a public-only request's schedule start a run
    // on a private model.

    /// [`Self::schedule_workflow`], recording the arming request's standing.
    async fn schedule_workflow_armed(
        &self,
        workflow_path: PathBuf,
        cron_schedule: Option<String>,
        _armed_with_private_reach: Option<bool>,
    ) -> Result<(), SchedulerError> {
        self.schedule_workflow(workflow_path, cron_schedule).await
    }
    /// [`Self::unpause_schedule`], recording the arming request's standing.
    async fn unpause_schedule_armed(
        &self,
        id: &str,
        _armed_with_private_reach: Option<bool>,
    ) -> Result<(), SchedulerError> {
        self.unpause_schedule(id).await
    }
    /// [`Self::run_now`], holding THIS run to the requesting caller's standing.
    async fn run_now_armed(
        &self,
        id: &str,
        _armed_with_private_reach: Option<bool>,
    ) -> Result<String, SchedulerError> {
        self.run_now(id).await
    }
    /// [`Self::update_schedule`], recording the arming request's standing.
    async fn update_schedule_armed(
        &self,
        sched_id: &str,
        new_cron: String,
        _armed_with_private_reach: Option<bool>,
    ) -> Result<(), SchedulerError> {
        self.update_schedule(sched_id, new_cron).await
    }
}
