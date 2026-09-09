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
    async fn get_running_job_info(
        &self,
        sched_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError>;
}
