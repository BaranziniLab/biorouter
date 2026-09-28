use super::{
    args::FileCommand,
    output::{component, emit_with, stream_format, HumanOptions},
    Api,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

pub(super) async fn handle(api: &Api, command: FileCommand) -> Result<Value> {
    match command {
        FileCommand::Upload { channel, file } => upload(api, &channel, &file).await,
        FileCommand::Download {
            blob,
            output,
            overwrite,
        } => download(api, &blob, &output, overwrite).await,
        FileCommand::Resume {
            transfer,
            file,
            overwrite,
        } => resume(api, &transfer, &file, overwrite).await,
        FileCommand::Status { transfer } => receipt(api, &transfer).await,
        FileCommand::Watch { transfer } => watch(api, &transfer).await,
        FileCommand::Pending => {
            let connection = api.connection_id().await?;
            api.client
                .request(
                    "GET",
                    &format!("/crew/transfers?connection_id={}", component(&connection)?),
                    None,
                )
                .await
        }
        FileCommand::Pause { transfer } => {
            receipt(api, &transfer).await?;
            api.client
                .request(
                    "POST",
                    &format!("/crew/transfers/{}/pause", component(&transfer)?),
                    Some(json!({})),
                )
                .await
        }
        FileCommand::Forget { transfer, file } => forget(api, &transfer, file.as_deref()).await,
        FileCommand::Reference {
            channel,
            path,
            label,
        } => {
            api.broker(
                "reference.create",
                json!({"channel_id":channel,"path":path,"label":label}),
                true,
            )
            .await
        }
        FileCommand::ShowReference { reference } => {
            api.broker("reference.get", json!({"reference_id":reference}), false)
                .await
        }
    }
}

async fn upload(api: &Api, channel: &str, file: &Path) -> Result<Value> {
    let connection = api.connection_id().await?;
    if let Some(previous) = existing_request(api).await? {
        verify_request_scope(&previous, &connection, Some(channel), "upload", None)?;
    }
    let capability = register(api, &connection, channel, "upload", file, false, None, None).await?;
    api.client
        .request(
            "POST",
            "/crew/transfers",
            Some(json!({
                "request_id":api.request_id,"connection_id":connection,"channel_id":channel,
                "direction":"upload","file_capability":capability,"blob_id":null
            })),
        )
        .await
}

async fn download(api: &Api, blob: &str, output: &Path, overwrite: bool) -> Result<Value> {
    not_a_folder(output)?;
    let connection = api.connection_id().await?;
    let record = if let Some(previous) = existing_request(api).await? {
        verify_request_scope(&previous, &connection, None, "download", Some(blob))?;
        previous
    } else {
        api.broker("blob.status", json!({"blob_id":blob}), false)
            .await?
    };
    let channel = record["channel_id"]
        .as_str()
        .context("Attachment has no channel ID")?;
    let capability = register(
        api,
        &connection,
        channel,
        "download",
        output,
        overwrite,
        Some(blob),
        None,
    )
    .await?;
    api.client
        .request(
            "POST",
            "/crew/transfers",
            Some(json!({
                "request_id":api.request_id,"connection_id":connection,"channel_id":channel,
                "direction":"download","file_capability":capability,"blob_id":blob
            })),
        )
        .await
}

async fn resume(api: &Api, transfer: &str, file: &Path, overwrite: bool) -> Result<Value> {
    let receipt = receipt(api, transfer).await?;
    let connection = receipt["connection_id"]
        .as_str()
        .context("Transfer has no connection ID")?;
    let channel = receipt["channel_id"]
        .as_str()
        .context("Transfer has no channel ID")?;
    let direction = receipt["direction"]
        .as_str()
        .context("Transfer has no direction")?;
    let capability = register(
        api,
        connection,
        channel,
        direction,
        file,
        overwrite,
        receipt["blob_id"].as_str(),
        Some(transfer),
    )
    .await?;
    api.client
        .request(
            "POST",
            &format!("/crew/transfers/{}/resume", component(transfer)?),
            Some(json!({"file_capability":capability})),
        )
        .await
}

async fn existing_request(api: &Api) -> Result<Option<Value>> {
    let result = api.client.request("GET", "/crew/transfers", None).await?;
    let receipts = result["transfers"]
        .as_array()
        .context("Daemon returned an invalid transfer list")?;
    let mut matches = receipts
        .iter()
        .filter(|receipt| receipt["request_id"].as_str() == Some(api.request_id.as_str()));
    let previous = matches.next().cloned();
    ensure!(
        matches.next().is_none(),
        "Multiple receipts use this request ID; inspect the transfer ledger before retrying"
    );
    Ok(previous)
}

fn verify_request_scope(
    receipt: &Value,
    connection: &str,
    channel: Option<&str>,
    direction: &str,
    blob: Option<&str>,
) -> Result<()> {
    ensure!(
        receipt["connection_id"].as_str() == Some(connection)
            && receipt["direction"].as_str() == Some(direction)
            && channel.is_none_or(|channel| receipt["channel_id"].as_str() == Some(channel))
            && (direction == "upload" || receipt["blob_id"].as_str() == blob),
        "Request ID belongs to a different transfer; inspect its receipt before choosing a new request ID"
    );
    Ok(())
}

async fn receipt(api: &Api, id: &str) -> Result<Value> {
    let receipt = api
        .client
        .request("GET", &format!("/crew/transfers/{}", component(id)?), None)
        .await?;
    if let Some(selected) = &api.selected {
        ensure!(
            receipt["connection_id"].as_str() == Some(selected.as_str()),
            "Transfer belongs to a different connection"
        );
    }
    Ok(receipt)
}

async fn forget(api: &Api, id: &str, file: Option<&Path>) -> Result<Value> {
    let receipt = receipt(api, id).await?;
    let direction = receipt["direction"]
        .as_str()
        .context("Transfer has no direction")?;
    let state = receipt["state"].as_str().context("Transfer has no state")?;
    ensure!(
        matches!(direction, "upload" | "download"),
        "Transfer has an unsupported direction"
    );
    let needs_cleanup = if direction == "download" && state != "completed" {
        match receipt.get("destination_identity") {
            Some(Value::Null) => false,
            Some(Value::String(identity)) if !identity.is_empty() => true,
            _ => anyhow::bail!("Daemon did not report the download destination binding; update the daemon before forgetting this receipt"),
        }
    } else {
        false
    };
    let body = if needs_cleanup {
        let file = file.context("This incomplete download requires --file with its original destination to remove the owned partial before forgetting its receipt")?;
        let connection = receipt["connection_id"]
            .as_str()
            .context("Transfer has no connection ID")?;
        let channel = receipt["channel_id"]
            .as_str()
            .context("Transfer has no channel ID")?;
        let capability = api
            .client
            .request(
                "POST",
                "/crew/files",
                Some(json!({
                    "connection_id":connection,"channel_id":channel,"direction":"download",
                    "path":absolute_path(file)?,"overwrite":false,"blob_id":receipt["blob_id"],
                    "transfer_id":id,"purpose":"cleanup"
                })),
            )
            .await?;
        let capability = capability["capability_id"]
            .as_str()
            .context("Daemon returned no cleanup capability")?;
        Some(json!({"file_capability":capability}))
    } else {
        ensure!(file.is_none(), "This receipt does not need --file; forget it without that option. Published downloads are retained");
        None
    };
    api.client
        .request(
            "DELETE",
            &format!("/crew/transfers/{}", component(id)?),
            body,
        )
        .await
}

/// `files download --output` names the file to save, never a folder (FILES-F9). A folder, or
/// a path ending in a separator, is refused before anything is asked of the daemon, which could
/// only answer that the destination exists, and suggest `--overwrite`, which cannot help.
fn not_a_folder(output: &Path) -> Result<()> {
    let text = output.to_string_lossy();
    let folder = output.is_dir() || text.ends_with(std::path::is_separator);
    if folder {
        let example = output.join("counts.csv");
        return Err(super::restated(
            format!(
                "--output {} is a folder. Name the file to save, like --output {}.",
                super::output::safe_text(&text),
                super::output::safe_text(&example.to_string_lossy())
            ),
            Some("crew_output_is_a_folder"),
        ));
    }
    Ok(())
}

fn absolute_path(path: &Path) -> Result<String> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    path.to_str()
        .map(str::to_string)
        .context("Crew local paths must be valid Unicode")
}

#[allow(clippy::too_many_arguments)]
async fn register(
    api: &Api,
    connection: &str,
    channel: &str,
    direction: &str,
    path: &Path,
    overwrite: bool,
    blob: Option<&str>,
    transfer: Option<&str>,
) -> Result<String> {
    let path = absolute_path(path)?;
    let result = api
        .client
        .request(
            "POST",
            "/crew/files",
            Some(api.with_expected_mode(json!({
                "connection_id":connection,"channel_id":channel,"direction":direction,
                "path":path,"overwrite":overwrite,"blob_id":blob,"transfer_id":transfer,
                "request_id":if transfer.is_none() { Some(&api.request_id) } else { None }
            }))),
        )
        .await?;
    Ok(result["capability_id"]
        .as_str()
        .context("Daemon returned no file capability")?
        .to_string())
}

/// How `files watch` prints each receipt: named from the workspace snapshot, read once, and
/// with IDs when `--show-ids` asks, as `files status` prints the same receipt (DW-07).
pub(super) async fn watch_options(api: &Api) -> HumanOptions {
    api.human(api.names().await)
}

async fn watch(api: &Api, id: &str) -> Result<Value> {
    let options = watch_options(api).await;
    let mut previous = Value::Null;
    loop {
        let current = tokio::select! {
            result = receipt(api, id) => result?,
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(json!({"detached":true,"transfer_id":id})); }
        };
        if current != previous {
            emit_with(&current, stream_format(api.format), &options)?;
        }
        if !matches!(
            current["state"].as_str(),
            Some("starting" | "uploading" | "downloading" | "publishing" | "pause_requested")
        ) {
            return Ok(
                json!({"transfer_id":id,"state":current["state"],"direction":current["direction"]}),
            );
        }
        previous = current;
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(json!({"detached":true,"transfer_id":id})); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::not_a_folder;
    use crate::daemon_client::Restated;

    /// FILES-F9: a folder is refused before the daemon is asked, in words that say what to do.
    #[test]
    fn a_download_to_a_folder_is_refused_before_the_daemon_is_asked() {
        let folder = tempfile::tempdir().expect("a folder");
        let with_separator = format!("{}{}", folder.path().display(), std::path::MAIN_SEPARATOR);
        for output in [folder.path().to_path_buf(), with_separator.into()] {
            let error = not_a_folder(&output).expect_err("a folder is refused");
            let refused = error.downcast_ref::<Restated>().expect("said for a person");
            assert_eq!(refused.code, Some("crew_output_is_a_folder"));
            let text = error.to_string();
            assert!(
                text.contains("is a folder. Name the file to save"),
                "{text}"
            );
            assert!(!text.contains("--overwrite"), "{text}");
        }
        assert!(not_a_folder(&folder.path().join("counts.csv")).is_ok());
        assert!(not_a_folder(std::path::Path::new("./counts.csv")).is_ok());
    }
}
