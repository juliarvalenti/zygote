//! Project discovery: the list of shows is whatever is under `projects/*/`
//! in the workspace, read fresh every time. There is no registry to drift.

use std::path::{Path, PathBuf};

use zygote_core::{DEFAULT_PORT, Graph};

/// One `projects/<name>/` directory.
#[derive(Clone, Debug)]
pub struct Project {
    /// Directory name; also what the UI calls it.
    pub name: String,
    /// Cargo package name, which is the renderer binary's name.
    pub package: String,
    pub description: String,
    pub dir: PathBuf,
    /// `assets/graphs/main.json`, summarized if it parses.
    pub graph: Option<GraphInfo>,
    /// The first `*.show.json` in the directory, if any.
    pub show_file: Option<PathBuf>,
    /// UDP port this project's renderer listens on. Stable per name so
    /// several projects can run at once.
    pub port: u16,
}

#[derive(Clone, Debug)]
pub struct GraphInfo {
    pub name: String,
    pub nodes: usize,
    pub output: String,
}

impl Project {
    /// Where the show file lives, or where `Save` will create one.
    pub fn show_path(&self) -> PathBuf {
        self.show_file
            .clone()
            .unwrap_or_else(|| self.dir.join(format!("{}.show.json", self.name)))
    }

    pub fn target(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

/// The workspace root: the current directory if it has `projects/`, else
/// walk up from the running binary (`target/<profile>/zygote-timeline`),
/// else the source tree this binary was built from.
pub fn workspace_root() -> Option<PathBuf> {
    let is_root = |p: &Path| p.join("projects").is_dir() && p.join("Cargo.toml").is_file();
    if let Ok(cwd) = std::env::current_dir()
        && is_root(&cwd)
    {
        return Some(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent();
        while let Some(d) = dir {
            if is_root(d) {
                return Some(d.to_path_buf());
            }
            dir = d.parent();
        }
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = std::fs::canonicalize(&source).unwrap_or(source);
    is_root(&source).then_some(source)
}

/// Every project directory, sorted by name, with ports assigned in order.
pub fn discover(root: &Path) -> Vec<Project> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("projects"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.join("Cargo.toml").is_file())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs.into_iter()
        .enumerate()
        .map(|(i, dir)| {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
            let package =
                manifest_value(&manifest, "name").unwrap_or_else(|| format!("zygote-{name}"));
            let description = manifest_value(&manifest, "description").unwrap_or_default();
            let graph = std::fs::read_to_string(dir.join("assets/graphs/main.json"))
                .ok()
                .and_then(|json| Graph::from_json(&json).ok())
                .map(|g| GraphInfo {
                    name: g.name.clone(),
                    nodes: g.nodes.len(),
                    output: g.output.to_string(),
                });
            let show_file = std::fs::read_dir(&dir).ok().and_then(|rd| {
                let mut shows: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.to_string_lossy().ends_with(".show.json"))
                    .collect();
                shows.sort();
                shows.into_iter().next()
            });
            Project {
                name,
                package,
                description,
                dir,
                graph,
                show_file,
                port: DEFAULT_PORT + i as u16,
            }
        })
        .collect()
}

/// First `key = "value"` line of a Cargo manifest. Good enough for the two
/// keys we read; a full TOML parser is not worth a dependency here.
fn manifest_value(manifest: &str, key: &str) -> Option<String> {
    manifest.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        if k.trim() != key {
            return None;
        }
        let v = v.trim();
        let v = v.strip_prefix('"')?.strip_suffix('"')?;
        Some(v.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_workspace_projects() {
        let root = workspace_root().expect("tests run inside the workspace");
        let projects = discover(&root);
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        assert!(
            names.contains(&"demo") && names.contains(&"scope"),
            "{names:?}"
        );
        let scope = projects.iter().find(|p| p.name == "scope").unwrap();
        assert_eq!(scope.package, "zygote-scope");
        assert!(scope.show_file.is_some());
        assert_eq!(scope.graph.as_ref().unwrap().output, "crt");
        let ports: std::collections::BTreeSet<u16> = projects.iter().map(|p| p.port).collect();
        assert_eq!(ports.len(), projects.len(), "ports are distinct");
    }

    #[test]
    fn manifest_values() {
        let m = "[package]\nname = \"zygote-x\"\nversion.workspace = true\ndescription = \"Hi\"\n";
        assert_eq!(manifest_value(m, "name").as_deref(), Some("zygote-x"));
        assert_eq!(manifest_value(m, "description").as_deref(), Some("Hi"));
        assert_eq!(manifest_value(m, "license"), None);
    }
}
