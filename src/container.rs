use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, DeviceMapping, HealthConfig, HostConfig,
    PortBinding,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptionsBuilder,
    LogsOptionsBuilder, RemoveContainerOptionsBuilder, StartContainerOptions,
    StopContainerOptionsBuilder, WaitContainerOptions,
};
use bollard::Docker;
use futures_util::Stream;

pub const SPT_SERVER_IMAGE: &str = "ghcr.io/cebarks/quartermaster/spt-server:latest";
pub const DEFAULT_CONTAINER_NAME: &str = "spt-server";
pub const DEFAULT_SPT_PORT: u16 = 6969;

#[derive(Clone)]
pub struct ContainerManager {
    docker: Arc<Docker>,
    stop_timeout: i32,
}

#[derive(Debug, Clone)]
pub enum SelinuxLabel {
    Private,
    Shared,
    #[allow(dead_code)]
    None,
}

impl SelinuxLabel {
    pub fn as_suffix(&self) -> &str {
        match self {
            SelinuxLabel::Private => ":Z",
            SelinuxLabel::Shared => ":z",
            SelinuxLabel::None => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct VolumeMount {
    pub host_path: PathBuf,
    pub container_path: String,
    pub read_only: bool,
    pub selinux: SelinuxLabel,
}

impl VolumeMount {
    pub fn to_bind_string(&self) -> String {
        let rw = if self.read_only { "ro" } else { "rw" };
        let sel = self.selinux.as_suffix();
        if sel.is_empty() {
            format!(
                "{}:{}:{}",
                self.host_path.display(),
                self.container_path,
                rw
            )
        } else {
            format!(
                "{}:{}:{},{}",
                self.host_path.display(),
                self.container_path,
                rw,
                &sel[1..]
            )
        }
    }
}

#[derive(Debug, Clone)]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: Protocol,
}

impl PortMapping {
    pub fn container_key(&self) -> String {
        let proto = match self.protocol {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        };
        format!("{}/{proto}", self.container_port)
    }
}

#[derive(Debug, Clone)]
pub struct CreateContainerOpts {
    pub name: String,
    pub image: String,
    pub env: Vec<(String, String)>,
    pub volumes: Vec<VolumeMount>,
    pub ports: Vec<PortMapping>,
    pub labels: Vec<(String, String)>,
    pub user: Option<String>,
    pub healthcheck: Option<HealthConfig>,
    pub devices: Vec<DeviceMapping>,
    pub security_opt: Vec<String>,
    pub cpuset_cpus: Option<String>,
    pub cpuset_mems: Option<String>,
    pub network_mode: Option<String>,
}

impl CreateContainerOpts {
    pub fn all_labels(&self) -> Vec<(String, String)> {
        let mut labels = self.labels.clone();
        if !labels.iter().any(|(k, _)| k == "managed-by") {
            labels.push(("managed-by".to_string(), "quma".to_string()));
        }
        labels
    }
}

pub fn filter_started_at(started_at: Option<String>) -> Option<String> {
    started_at.filter(|s| !s.is_empty() && s != "0001-01-01T00:00:00Z")
}

impl ContainerManager {
    pub fn new(stop_timeout: u64) -> Result<Self> {
        let docker = Docker::connect_with_unix_defaults().or_else(|_| {
            // Podman rootless socket lives under XDG_RUNTIME_DIR, not /var/run/docker.sock
            let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| {
                std::path::Path::new(d).join("podman/podman.sock").exists()
            });
            match runtime_dir {
                Some(dir) => {
                    let sock = format!("unix://{dir}/podman/podman.sock");
                    Docker::connect_with_unix(&sock, 120, bollard::API_DEFAULT_VERSION)
                }
                None => Err(bollard::errors::Error::SocketNotFoundError(
                    "No container runtime socket found".into(),
                )),
            }
        }).context(
            "No container runtime found. Install Podman or Docker and ensure the socket is enabled:\n  \
             systemctl --user enable --now podman.socket",
        )?;
        Ok(Self {
            docker: Arc::new(docker),
            stop_timeout: stop_timeout as i32,
        })
    }

    pub fn docker(&self) -> &Docker {
        &self.docker
    }

    pub async fn start(&self, container: &str) -> Result<()> {
        tracing::debug!(container, "starting container");
        self.docker
            .start_container(container, None::<StartContainerOptions>)
            .await
            .with_context(|| format!("failed to start container '{container}'"))
    }

    pub async fn stop(&self, container: &str) -> Result<()> {
        tracing::debug!(container, "stopping container");
        self.docker
            .stop_container(
                container,
                Some(
                    StopContainerOptionsBuilder::default()
                        .t(self.stop_timeout)
                        .build(),
                ),
            )
            .await
            .with_context(|| format!("failed to stop container '{container}'"))
    }

    pub async fn restart(&self, container: &str) -> Result<()> {
        self.stop(container).await?;
        self.start(container).await
    }

    pub async fn is_running(&self, container: &str) -> Result<bool> {
        let info = self
            .docker
            .inspect_container(
                container,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .with_context(|| format!("failed to inspect container '{container}'"))?;
        Ok(info
            .state
            .as_ref()
            .and_then(|s| s.status.as_ref())
            .is_some_and(|s| s.as_ref() == "running"))
    }

    pub async fn container_started_at(&self, container: &str) -> Result<Option<String>> {
        let info = self
            .docker
            .inspect_container(
                container,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .with_context(|| format!("failed to inspect container '{container}'"))?;

        Ok(filter_started_at(
            info.state.as_ref().and_then(|s| s.started_at.clone()),
        ))
    }

    pub async fn inspect(&self, container: &str) -> Result<ContainerInspectResponse> {
        self.docker
            .inspect_container(
                container,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .with_context(|| format!("failed to inspect container '{container}'"))
    }

    pub fn log_stream(
        &self,
        container: &str,
        tail: usize,
        follow: bool,
    ) -> impl Stream<Item = Result<bollard::container::LogOutput, bollard::errors::Error>> {
        self.docker.logs(
            container,
            Some(
                LogsOptionsBuilder::default()
                    .stdout(true)
                    .stderr(true)
                    .follow(follow)
                    .tail(&tail.to_string())
                    .timestamps(true)
                    .build(),
            ),
        )
    }

    /// Watch a container for exit. Returns a stream that yields a single
    /// `ContainerWaitResponse` when the container stops (exit code 0) or an
    /// `Error::DockerContainerWaitError` for non-zero exits (bollard converts
    /// non-zero codes into errors). Callers should match both variants.
    pub fn wait_container(
        &self,
        container: &str,
    ) -> impl Stream<Item = Result<bollard::models::ContainerWaitResponse, bollard::errors::Error>>
    {
        self.docker
            .wait_container(container, None::<WaitContainerOptions>)
    }

    pub async fn pull_image(&self, image: &str) -> Result<()> {
        tracing::info!(image, "pulling container image");
        use futures_util::TryStreamExt;
        self.docker
            .create_image(
                Some(
                    CreateImageOptionsBuilder::default()
                        .from_image(image)
                        .build(),
                ),
                None,
                None,
            )
            .try_collect::<Vec<_>>()
            .await
            .with_context(|| format!("failed to pull image '{image}'"))?;
        Ok(())
    }

    pub async fn create_container(&self, opts: CreateContainerOpts) -> Result<String> {
        let env: Vec<String> = opts.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let binds: Vec<String> = opts.volumes.iter().map(|v| v.to_bind_string()).collect();
        let labels: HashMap<String, String> = opts.all_labels().into_iter().collect();

        let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        for pm in &opts.ports {
            port_bindings.insert(
                pm.container_key(),
                Some(vec![PortBinding {
                    host_port: Some(pm.host_port.to_string()),
                    ..Default::default()
                }]),
            );
        }

        let devices = if opts.devices.is_empty() {
            None
        } else {
            Some(opts.devices.clone())
        };

        let body = ContainerCreateBody {
            image: Some(opts.image.clone()),
            env: Some(env),
            labels: Some(labels),
            user: opts.user.clone(),
            healthcheck: opts.healthcheck.clone(),
            host_config: Some(HostConfig {
                binds: Some(binds),
                network_mode: opts.network_mode.clone(),
                port_bindings: if port_bindings.is_empty()
                    || opts.network_mode.as_deref() == Some("host")
                {
                    None
                } else {
                    Some(port_bindings)
                },
                devices,
                security_opt: if opts.security_opt.is_empty() {
                    None
                } else {
                    Some(opts.security_opt.clone())
                },
                cpuset_cpus: opts.cpuset_cpus.clone(),
                cpuset_mems: opts.cpuset_mems.clone(),
                ..Default::default()
            }),
            ..Default::default()
        };

        let create_opts = CreateContainerOptionsBuilder::default()
            .name(&opts.name)
            .build();
        let response = self
            .docker
            .create_container(Some(create_opts), body)
            .await
            .with_context(|| format!("failed to create container '{}'", opts.name))?;
        tracing::info!(container = %opts.name, id = %response.id, "container created");
        Ok(response.id)
    }

    /// Query current memory usage (RSS) of a running container, in bytes.
    /// Returns None if the container is not running or stats are unavailable.
    pub async fn container_memory_bytes(&self, container: &str) -> Option<u64> {
        use futures_util::StreamExt;
        let mut stream = self.docker.stats(
            container,
            Some(
                bollard::query_parameters::StatsOptionsBuilder::default()
                    .stream(false)
                    .one_shot(true)
                    .build(),
            ),
        );
        if let Some(Ok(stats)) = stream.next().await {
            stats.memory_stats?.usage
        } else {
            None
        }
    }

    /// Check if a stopped container was OOM killed.
    pub async fn was_oom_killed(&self, container: &str) -> bool {
        self.inspect(container)
            .await
            .ok()
            .and_then(|info| info.state)
            .and_then(|state| state.oom_killed)
            .unwrap_or(false)
    }

    pub async fn remove_container(&self, container: &str) -> Result<()> {
        tracing::debug!(container, "removing container");
        self.docker
            .remove_container(
                container,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await
            .with_context(|| format!("failed to remove container '{container}'"))
    }

    pub async fn detect_containers_by_label(&self, key: &str, value: &str) -> Result<Vec<String>> {
        let label_filter = format!("{key}={value}");
        let mut filters = HashMap::new();
        filters.insert("label", vec![label_filter.as_str()]);
        let containers = self
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
            .context("failed to list containers")?;
        Ok(containers
            .into_iter()
            .filter_map(|c| {
                c.names?
                    .into_iter()
                    .next()
                    .map(|n| n.trim_start_matches('/').to_string())
            })
            .collect())
    }

    /// Detect SPT containers by checking volume mounts (for setup wizard backward compat)
    pub async fn detect_spt_containers(&self, dirs: &crate::dirs::QumaDirs) -> Result<Vec<String>> {
        let containers = self
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default().all(true).build(),
            ))
            .await
            .context("failed to list containers")?;

        let spt_dir_str = dirs.spt_server.to_string_lossy();
        Ok(containers
            .into_iter()
            .filter_map(|c| {
                let mounts = c.mounts.as_ref()?;
                let has_spt_mount = mounts.iter().any(|m| {
                    m.source
                        .as_deref()
                        .is_some_and(|s| s.contains(spt_dir_str.as_ref()))
                });
                if has_spt_mount {
                    c.names?
                        .into_iter()
                        .next()
                        .map(|n| n.trim_start_matches('/').to_string())
                } else {
                    None
                }
            })
            .collect())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn selinux_label_display() {
        assert_eq!(SelinuxLabel::Private.as_suffix(), ":Z");
        assert_eq!(SelinuxLabel::Shared.as_suffix(), ":z");
        assert_eq!(SelinuxLabel::None.as_suffix(), "");
    }

    #[test]
    fn volume_mount_to_bind_string() {
        let mount = VolumeMount {
            host_path: PathBuf::from("/opt/fika-client"),
            container_path: "/opt/tarkov".to_string(),
            read_only: true,
            selinux: SelinuxLabel::Shared,
        };
        assert_eq!(mount.to_bind_string(), "/opt/fika-client:/opt/tarkov:ro,z");
    }

    #[test]
    fn volume_mount_rw_private() {
        let mount = VolumeMount {
            host_path: PathBuf::from("/data/clients/1/BepInEx/config"),
            container_path: "/opt/tarkov/BepInEx/config".to_string(),
            read_only: false,
            selinux: SelinuxLabel::Private,
        };
        assert_eq!(
            mount.to_bind_string(),
            "/data/clients/1/BepInEx/config:/opt/tarkov/BepInEx/config:rw,Z"
        );
    }

    #[test]
    fn create_container_opts_always_includes_managed_label() {
        let opts = CreateContainerOpts {
            name: "test".to_string(),
            image: "test:latest".to_string(),
            env: vec![],
            volumes: vec![],
            ports: vec![],
            labels: vec![("custom".to_string(), "value".to_string())],
            user: None,
            healthcheck: None,
            devices: vec![],
            security_opt: vec![],
            cpuset_cpus: None,
            cpuset_mems: None,
            network_mode: None,
        };
        let labels = opts.all_labels();
        assert!(labels.iter().any(|(k, v)| k == "managed-by" && v == "quma"));
    }

    #[test]
    fn create_container_opts_devices_default_empty() {
        let opts = CreateContainerOpts {
            name: "test".to_string(),
            image: "test:latest".to_string(),
            env: vec![],
            volumes: vec![],
            ports: vec![],
            labels: vec![],
            user: None,
            healthcheck: None,
            devices: vec![],
            security_opt: vec![],
            cpuset_cpus: None,
            cpuset_mems: None,
            network_mode: None,
        };
        assert!(opts.devices.is_empty());
    }

    #[test]
    fn create_container_opts_cpuset_defaults_none() {
        let opts = CreateContainerOpts {
            name: "test".to_string(),
            image: "test:latest".to_string(),
            env: vec![],
            volumes: vec![],
            ports: vec![],
            labels: vec![],
            user: None,
            healthcheck: None,
            devices: vec![],
            security_opt: vec![],
            cpuset_cpus: None,
            cpuset_mems: None,
            network_mode: None,
        };
        assert!(opts.cpuset_cpus.is_none());
        assert!(opts.cpuset_mems.is_none());
    }

    #[test]
    fn started_at_filters_zero_value() {
        let zero = "0001-01-01T00:00:00Z";
        let valid = "2026-06-19T10:00:00Z";
        assert_eq!(filter_started_at(Some(zero.to_string())), None);
        assert_eq!(
            filter_started_at(Some(valid.to_string())),
            Some(valid.to_string())
        );
        assert_eq!(filter_started_at(Some(String::new())), None);
        assert_eq!(filter_started_at(None), None);
    }
}
