use anyhow::{Context, Result};
use std::path::Path;

pub struct NumaNode {
    pub id: u32,
    pub cpulist: String,
}

pub struct NumaTopology {
    nodes: Vec<NumaNode>,
}

impl NumaTopology {
    pub fn detect() -> Result<Self> {
        Self::detect_from(Path::new("/sys/devices/system/node"))
    }

    pub fn detect_from(sysfs_node_dir: &Path) -> Result<Self> {
        if !sysfs_node_dir.exists() {
            return Ok(Self { nodes: Vec::new() });
        }

        let mut nodes = Vec::new();
        for entry in std::fs::read_dir(sysfs_node_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(id_str) = name.strip_prefix("node") else {
                continue;
            };
            let Ok(id) = id_str.parse::<u32>() else {
                continue;
            };
            let cpulist_path = entry.path().join("cpulist");
            let cpulist = std::fs::read_to_string(&cpulist_path)
                .with_context(|| format!("failed to read {}", cpulist_path.display()))?;
            nodes.push(NumaNode {
                id,
                cpulist: cpulist.trim().to_string(),
            });
        }
        nodes.sort_by_key(|n| n.id);
        Ok(Self { nodes })
    }

    pub fn cpuset_for_node(&self, node: u32) -> Result<(String, String)> {
        let n = self.nodes.iter().find(|n| n.id == node).with_context(|| {
            format!(
                "NUMA node {node} not found (available: {:?})",
                self.node_ids()
            )
        })?;
        Ok((n.cpulist.clone(), node.to_string()))
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn node_ids(&self) -> Vec<u32> {
        self.nodes.iter().map(|n| n.id).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn nodes(&self) -> &[NumaNode] {
        &self.nodes
    }

    pub fn empty() -> Self {
        Self { nodes: Vec::new() }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn mock_sysfs(dir: &std::path::Path, nodes: &[(u32, &str)]) {
        for (id, cpulist) in nodes {
            let node_dir = dir.join(format!("node{id}"));
            std::fs::create_dir_all(&node_dir).unwrap();
            std::fs::write(node_dir.join("cpulist"), cpulist).unwrap();
        }
    }

    #[test]
    fn detect_4_node_system() {
        let tmp = tempfile::tempdir().unwrap();
        mock_sysfs(
            tmp.path(),
            &[
                (0, "0-15,32-47"),
                (1, "16-31,48-63"),
                (2, "64-79,96-111"),
                (3, "80-95,112-127"),
            ],
        );
        let topo = NumaTopology::detect_from(tmp.path()).unwrap();
        assert_eq!(topo.node_count(), 4);
        assert_eq!(topo.node_ids(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn detect_nonexistent_dir_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let bogus = tmp.path().join("nonexistent");
        let topo = NumaTopology::detect_from(&bogus).unwrap();
        assert!(topo.is_empty());
    }

    #[test]
    fn cpuset_for_valid_node() {
        let tmp = tempfile::tempdir().unwrap();
        mock_sysfs(tmp.path(), &[(0, "0-7"), (1, "8-15")]);
        let topo = NumaTopology::detect_from(tmp.path()).unwrap();
        let (cpus, mems) = topo.cpuset_for_node(1).unwrap();
        assert_eq!(cpus, "8-15");
        assert_eq!(mems, "1");
    }

    #[test]
    fn cpuset_for_invalid_node_errors() {
        let tmp = tempfile::tempdir().unwrap();
        mock_sysfs(tmp.path(), &[(0, "0-7")]);
        let topo = NumaTopology::detect_from(tmp.path()).unwrap();
        assert!(topo.cpuset_for_node(5).is_err());
    }

    #[test]
    fn detect_trims_whitespace_from_cpulist() {
        let tmp = tempfile::tempdir().unwrap();
        mock_sysfs(tmp.path(), &[(0, "0-3\n")]);
        let topo = NumaTopology::detect_from(tmp.path()).unwrap();
        let (cpus, _) = topo.cpuset_for_node(0).unwrap();
        assert_eq!(cpus, "0-3");
    }

    #[test]
    fn detect_skips_non_node_directories() {
        let tmp = tempfile::tempdir().unwrap();
        mock_sysfs(tmp.path(), &[(0, "0-3")]);
        // Create a non-node directory that should be ignored
        std::fs::create_dir_all(tmp.path().join("has_cpu")).unwrap();
        let topo = NumaTopology::detect_from(tmp.path()).unwrap();
        assert_eq!(topo.node_count(), 1);
    }
}
