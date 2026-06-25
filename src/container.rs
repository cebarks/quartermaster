use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, HealthConfig, HostConfig, PortBinding,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptionsBuilder,
    LogsOptionsBuilder, RemoveContainerOptionsBuilder, StartContainerOptions,
    StopContainerOptionsBuilder,
};
use bollard::Docker;
use futures_util::Stream;

pub const SPT_SERVER_IMAGE: &str = "ghcr.io/zhliau/fika-spt-server-docker:latest";
pub const DEFAULT_CONTAINER_NAME: &str = "spt-server";
pub const DEFAULT_SPT_PORT: u16 = 6969;

#[derive(Clone)]
pub struct ContainerManager {
    docker: Arc<Docker>,
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
    #[allow(dead_code)]
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

fn filter_started_at(started_at: Option<String>) -> Option<String> {
    started_at.filter(|s| !s.is_empty() && s != "0001-01-01T00:00:00Z")
}

impl ContainerManager {
    pub fn new() -> Result<Self> {
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
        })
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
                Some(StopContainerOptionsBuilder::default().t(10).build()),
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

        let body = ContainerCreateBody {
            image: Some(opts.image.clone()),
            env: Some(env),
            labels: Some(labels),
            user: opts.user.clone(),
            healthcheck: opts.healthcheck.clone(),
            host_config: Some(HostConfig {
                binds: Some(binds),
                port_bindings: if port_bindings.is_empty() {
                    None
                } else {
                    Some(port_bindings)
                },
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
    pub async fn detect_spt_containers(&self, spt_dir: &std::path::Path) -> Result<Vec<String>> {
        let containers = self
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default().all(true).build(),
            ))
            .await
            .context("failed to list containers")?;

        let spt_dir_str = spt_dir.to_string_lossy();
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
        };
        let labels = opts.all_labels();
        assert!(labels.iter().any(|(k, v)| k == "managed-by" && v == "quma"));
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
