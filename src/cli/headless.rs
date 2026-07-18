use anyhow::{bail, Result};
use clap::Subcommand;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

use crate::client::{ClientHealth, ClientState, ContainerStatus};
use crate::config::is_fika_installed;
use crate::spt::headless::EHeadlessStatus;

use super::common::confirm;

#[derive(Subcommand)]
pub enum HeadlessAction {
    /// Show headless client status
    Status {
        /// Client number for detailed view
        client: Option<u32>,
    },
    /// Create a new headless client
    Create {
        /// Extra isolated paths for this client (additive to global)
        #[arg(long)]
        extra_isolated_paths: Vec<String>,
    },
    /// Delete a specific headless client
    Delete {
        /// Client number
        client: u32,
        /// Force delete even if client is in a raid
        #[arg(long)]
        force: bool,
    },
    /// Stream container logs for a client
    Logs {
        /// Client number
        client: u32,
        /// Follow log output
        #[arg(long, short)]
        follow: bool,
    },
    /// Stop a headless client
    Stop {
        /// Client number
        client: u32,
    },
    /// Start a headless client
    Start {
        /// Client number
        client: u32,
    },
    /// Restart a headless client
    Restart {
        /// Client number
        client: u32,
    },
    /// Gracefully restart a headless client via Fika ShutdownClient API
    GracefulRestart {
        /// Client number
        client: u32,
    },
    /// Rename a headless client (sets Fika alias)
    Rename {
        /// Client number
        client: u32,
        /// Display name (empty to clear)
        #[arg(default_value = "")]
        name: String,
    },
    /// Set the desired number of headless clients
    Scale {
        /// Desired number of clients (max 16)
        #[arg(value_parser = clap::value_parser!(u32).range(0..=16))]
        count: u32,
    },
    /// Tear down and recreate all headless client containers and overlays
    Rebuild,
}

struct HeadlessApiClient {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

#[derive(Deserialize)]
struct OperationIdResponse {
    operation_id: u64,
}

#[derive(Deserialize)]
struct GracefulRestartResponse {
    result: String,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum OperationStatus {
    Running,
    Completed,
    Failed { error: String },
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: Option<String>,
    message: String,
}

#[derive(Serialize)]
struct ScaleRequest {
    count: u32,
    force: bool,
}

#[derive(Serialize)]
struct DeleteRequest {
    force: bool,
}

#[derive(Serialize)]
struct RebuildRequest {
    force: bool,
}

#[derive(Serialize)]
struct RenameRequest {
    name: String,
}

impl HeadlessApiClient {
    fn new(spt_dir: &Path) -> Result<Self> {
        let (token, url) = crate::web::api_auth::read_api_token(spt_dir)?;
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            base_url: url,
            token,
        })
    }

    async fn check_server(&self) -> Result<()> {
        match self
            .client
            .get(format!("{}/quma/api/headless/status", self.base_url))
            .header("X-Quma-Token", &self.token)
            .timeout(Duration::from_secs(3))
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => bail!("Web server is not running. Start it with 'quma serve' first."),
        }
    }

    async fn get(&self, path: &str) -> Result<reqwest::Response> {
        let resp = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .header("X-Quma-Token", &self.token)
            .send()
            .await?;

        if !resp.status().is_success() {
            let body: ErrorResponse = resp.json().await?;
            bail!("{}", body.message);
        }
        Ok(resp)
    }

    async fn post<T: Serialize>(&self, path: &str, body: Option<&T>) -> Result<reqwest::Response> {
        let mut req = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .header("X-Quma-Token", &self.token);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await?;

        // Handle client_in_raid specially
        if resp.status() == 409 {
            let body: ErrorResponse = resp.json().await?;
            if body.error.as_deref() == Some("client_in_raid") {
                return Err(anyhow::anyhow!("{}", body.message));
            }
            bail!("{}", body.message);
        }

        if !resp.status().is_success() {
            let body: ErrorResponse = resp.json().await?;
            bail!("{}", body.message);
        }
        Ok(resp)
    }

    async fn poll_operation(&self, op_id: u64) -> Result<()> {
        let timeout = Duration::from_secs(600);
        let start = std::time::Instant::now();
        print!("Operation in progress");
        std::io::Write::flush(&mut std::io::stdout())?;

        loop {
            if start.elapsed() > timeout {
                bail!("Operation timed out — check server logs");
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
            print!(".");
            std::io::Write::flush(&mut std::io::stdout())?;

            let resp = self
                .get(&format!("/quma/api/headless/operations/{}", op_id))
                .await?;
            let status: OperationStatus = resp.json().await?;
            match status {
                OperationStatus::Completed => {
                    println!(" done");
                    return Ok(());
                }
                OperationStatus::Failed { error } => bail!("Operation failed: {}", error),
                OperationStatus::Running => continue,
            }
        }
    }
}

pub async fn run(action: &HeadlessAction, spt_dir: &Path) -> Result<()> {
    if !is_fika_installed(spt_dir) {
        bail!(
            "Fika server mod is not installed.\n\
             Install Fika with: quma install fika-server\n\
             Or run setup: quma setup"
        );
    }

    let api = HeadlessApiClient::new(spt_dir)?;
    api.check_server().await?;

    match action {
        HeadlessAction::Status { client } => cmd_status(&api, *client).await,
        HeadlessAction::Scale { count } => cmd_scale(&api, *count).await,
        HeadlessAction::Create { .. } => cmd_create(&api).await,
        HeadlessAction::Delete { client, force } => cmd_delete(&api, *client, *force).await,
        HeadlessAction::Stop { client } => cmd_stop(&api, *client).await,
        HeadlessAction::Start { client } => cmd_start(&api, *client).await,
        HeadlessAction::Restart { client } => cmd_restart(&api, *client).await,
        HeadlessAction::GracefulRestart { client } => cmd_graceful_restart(&api, *client).await,
        HeadlessAction::Rename { client, name } => cmd_rename(&api, *client, name).await,
        HeadlessAction::Logs { client, follow } => cmd_logs(&api, *client, *follow).await,
        HeadlessAction::Rebuild => cmd_rebuild(&api).await,
    }
}

async fn cmd_status(api: &HeadlessApiClient, client: Option<u32>) -> Result<()> {
    let resp = api.get("/quma/api/headless/status").await?;
    let states: Vec<ClientState> = resp.json().await?;

    match client {
        None => {
            // Table of all clients
            println!(
                "{:<8} {:<20} {:<20} {:<15} {:<10}",
                "CLIENT", "CONTAINER", "STATUS", "HEALTH", "FIKA STATE"
            );
            println!("{}", "-".repeat(80));

            for cs in &states {
                let container_status_str = match cs.container_status {
                    ContainerStatus::Running => "running",
                    ContainerStatus::Stopped => "stopped",
                    ContainerStatus::Unknown => "unknown",
                };
                let health_str = match cs.health {
                    ClientHealth::Healthy => "healthy",
                    ClientHealth::Degraded => "degraded",
                    ClientHealth::Down => "down",
                    ClientHealth::GivenUp => "given up",
                };
                let fika_str = match &cs.fika_status {
                    Some(EHeadlessStatus::Ready) => "Ready".to_string(),
                    Some(EHeadlessStatus::InRaid) => "In Raid".to_string(),
                    Some(EHeadlessStatus::Unknown(v)) => format!("Unknown({})", v),
                    None => "no data".to_string(),
                };
                println!(
                    "{:<8} {:<20} {:<20} {:<15} {:<10}",
                    cs.index, cs.container_name, container_status_str, health_str, fika_str
                );
            }
        }
        Some(index) => {
            // Detailed single client view
            let cs = states
                .iter()
                .find(|s| s.index == index)
                .ok_or_else(|| anyhow::anyhow!("Client {} not found", index))?;

            println!("Client {}", cs.index);
            println!("  Container: {}", cs.container_name);
            println!(
                "  Status: {}",
                match cs.container_status {
                    ContainerStatus::Running => "running",
                    ContainerStatus::Stopped => "stopped",
                    ContainerStatus::Unknown => "unknown",
                }
            );
            println!(
                "  Health: {}",
                match cs.health {
                    ClientHealth::Healthy => "healthy",
                    ClientHealth::Degraded => "degraded",
                    ClientHealth::Down => "down",
                    ClientHealth::GivenUp => "given up",
                }
            );
            println!("  Restart count: {}", cs.restart_count);
            if let Some(ref last) = cs.last_restart {
                println!("  Last restart: {}", last);
            }
            if let Some(ref fika) = cs.fika_status {
                let status_str = match fika {
                    EHeadlessStatus::Ready => "Ready".to_string(),
                    EHeadlessStatus::InRaid => "In Raid".to_string(),
                    EHeadlessStatus::Unknown(v) => format!("Unknown({})", v),
                };
                println!("  Fika status: {}", status_str);
            }
            if !cs.players.is_empty() {
                println!("  Players: {}", cs.players.join(", "));
            }
            if let Some(cpu) = cs.cpu_percent {
                println!("  CPU: {:.1}%", cpu);
            }
            if let Some(mem) = cs.memory_mb {
                println!("  Memory: {:.1} MB", mem);
            }
        }
    }

    Ok(())
}

async fn cmd_scale(api: &HeadlessApiClient, count: u32) -> Result<()> {
    let body = ScaleRequest {
        count,
        force: false,
    };

    match api.post("/quma/api/headless/scale", Some(&body)).await {
        Ok(resp) => {
            let op: OperationIdResponse = resp.json().await?;
            api.poll_operation(op.operation_id).await?;
            println!("Successfully scaled to {} client(s).", count);
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("in a raid") {
                if confirm(&format!("{} Scale down anyway?", msg))? {
                    let force_body = ScaleRequest { count, force: true };
                    let resp = api
                        .post("/quma/api/headless/scale", Some(&force_body))
                        .await?;
                    let op: OperationIdResponse = resp.json().await?;
                    api.poll_operation(op.operation_id).await?;
                    println!("Successfully scaled to {} client(s).", count);
                    return Ok(());
                } else {
                    println!("Scale operation cancelled.");
                    return Ok(());
                }
            }
            Err(e)
        }
    }
}

async fn cmd_create(api: &HeadlessApiClient) -> Result<()> {
    let resp = api.post("/quma/api/headless/create", None::<&()>).await?;
    let op: OperationIdResponse = resp.json().await?;
    api.poll_operation(op.operation_id).await?;
    println!("Client created successfully.");
    Ok(())
}

async fn cmd_delete(api: &HeadlessApiClient, client: u32, force: bool) -> Result<()> {
    let body = DeleteRequest { force };
    match api
        .post(
            &format!("/quma/api/headless/{}/delete", client),
            Some(&body),
        )
        .await
    {
        Ok(resp) => {
            let op: OperationIdResponse = resp.json().await?;
            api.poll_operation(op.operation_id).await?;
            println!("Client {} deleted successfully.", client);
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("in a raid") && !force {
                if confirm(&format!("{} Delete anyway?", msg))? {
                    let force_body = DeleteRequest { force: true };
                    let resp = api
                        .post(
                            &format!("/quma/api/headless/{}/delete", client),
                            Some(&force_body),
                        )
                        .await?;
                    let op: OperationIdResponse = resp.json().await?;
                    api.poll_operation(op.operation_id).await?;
                    println!("Client {} deleted successfully.", client);
                    return Ok(());
                } else {
                    println!("Delete operation cancelled.");
                    return Ok(());
                }
            }
            Err(e)
        }
    }
}

async fn cmd_stop(api: &HeadlessApiClient, client: u32) -> Result<()> {
    api.post(&format!("/quma/api/headless/{}/stop", client), None::<&()>)
        .await?;
    println!("Client {} stopped successfully.", client);
    Ok(())
}

async fn cmd_start(api: &HeadlessApiClient, client: u32) -> Result<()> {
    api.post(&format!("/quma/api/headless/{}/start", client), None::<&()>)
        .await?;
    println!("Client {} started successfully.", client);
    Ok(())
}

async fn cmd_restart(api: &HeadlessApiClient, client: u32) -> Result<()> {
    api.post(
        &format!("/quma/api/headless/{}/restart", client),
        None::<&()>,
    )
    .await?;
    println!("Client {} restarted successfully.", client);
    Ok(())
}

async fn cmd_graceful_restart(api: &HeadlessApiClient, client: u32) -> Result<()> {
    println!("Sending graceful shutdown to client {}...", client);
    let resp = api
        .post(
            &format!("/quma/api/headless/{}/graceful-restart", client),
            None::<&()>,
        )
        .await?;
    let result: GracefulRestartResponse = resp.json().await?;

    match result.result.as_str() {
        "exited" => {
            println!("Client {} shut down gracefully.", client);
            Ok(())
        }
        "timeout" => bail!(
            "Client {} did not shut down within 30s. Use force restart if needed.",
            client
        ),
        _ => bail!("Unexpected result: {}", result.result),
    }
}

async fn cmd_rename(api: &HeadlessApiClient, client: u32, name: &str) -> Result<()> {
    let body = RenameRequest {
        name: name.to_string(),
    };
    api.post(
        &format!("/quma/api/headless/{}/rename", client),
        Some(&body),
    )
    .await?;
    if name.is_empty() {
        println!("Cleared alias for client {}.", client);
    } else {
        println!("Renamed client {} to \"{}\".", client, name);
    }
    println!("Restart the SPT server for the in-game name to take effect.");
    Ok(())
}

async fn cmd_logs(api: &HeadlessApiClient, client: u32, follow: bool) -> Result<()> {
    let tail = 100;
    let url = format!(
        "{}/quma/api/headless/{}/logs?tail={}&follow={}",
        api.base_url, client, tail, follow
    );

    let resp = api
        .client
        .get(&url)
        .header("X-Quma-Token", &api.token)
        .send()
        .await?;

    if !resp.status().is_success() {
        let body: ErrorResponse = resp.json().await?;
        bail!("{}", body.message);
    }

    println!("Streaming logs for client {}...", client);

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        // SSE format: "data: <line>\n\n"
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                println!("{}", data);
            }
        }
    }

    Ok(())
}

async fn cmd_rebuild(api: &HeadlessApiClient) -> Result<()> {
    let body = RebuildRequest { force: false };
    match api.post("/quma/api/headless/rebuild", Some(&body)).await {
        Ok(resp) => {
            let op: OperationIdResponse = resp.json().await?;
            api.poll_operation(op.operation_id).await?;
            println!("Rebuild completed successfully.");
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("in a raid") {
                if confirm(&format!("{} Rebuild anyway?", msg))? {
                    let force_body = RebuildRequest { force: true };
                    let resp = api
                        .post("/quma/api/headless/rebuild", Some(&force_body))
                        .await?;
                    let op: OperationIdResponse = resp.json().await?;
                    api.poll_operation(op.operation_id).await?;
                    println!("Rebuild completed successfully.");
                    return Ok(());
                } else {
                    println!("Rebuild cancelled.");
                    return Ok(());
                }
            }
            Err(e)
        }
    }
}
