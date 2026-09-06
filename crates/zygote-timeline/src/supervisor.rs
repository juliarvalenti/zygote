//! Owns the renderer processes: build if needed, spawn, wait until the
//! renderer answers on its port, notice when it dies, stop it on request
//! and when the UI exits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use zygote_core::{Message, ParamSender};

use crate::projects::Project;

/// How long a freshly spawned renderer gets to answer a `Ping`.
const START_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Debug, PartialEq)]
pub enum ProcState {
    Stopped,
    /// `cargo build` is running because the binary did not exist.
    Building,
    /// The renderer process is up but has not answered on its port yet.
    Starting,
    Running,
    Failed(String),
}

impl ProcState {
    pub fn is_running(&self) -> bool {
        matches!(self, ProcState::Running)
    }
    pub fn is_busy(&self) -> bool {
        matches!(self, ProcState::Building | ProcState::Starting)
    }
}

struct Managed {
    state: ProcState,
    build: Option<Child>,
    child: Option<Child>,
    /// Socket used only to ping the renderer while it starts.
    probe: Option<ParamSender>,
    since: Instant,
    log: PathBuf,
    project: Project,
}

pub struct Supervisor {
    root: PathBuf,
    profile: &'static str,
    procs: HashMap<String, Managed>,
}

impl Supervisor {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            procs: HashMap::new(),
        }
    }

    pub fn state(&self, name: &str) -> ProcState {
        self.procs
            .get(name)
            .map(|m| m.state.clone())
            .unwrap_or(ProcState::Stopped)
    }

    pub fn log_path(&self, name: &str) -> PathBuf {
        self.root.join("target/zygote").join(format!("{name}.log"))
    }

    fn binary(&self, project: &Project) -> PathBuf {
        self.root
            .join("target")
            .join(self.profile)
            .join(&project.package)
    }

    /// Start a project's renderer. Builds it first when the binary is missing.
    pub fn launch(&mut self, project: &Project) {
        self.stop(&project.name);
        let log = self.log_path(&project.name);
        let _ = std::fs::create_dir_all(log.parent().unwrap());
        let mut managed = Managed {
            state: ProcState::Stopped,
            build: None,
            child: None,
            probe: None,
            since: Instant::now(),
            log,
            project: project.clone(),
        };
        if self.binary(project).is_file() {
            self.spawn_renderer(&mut managed);
        } else {
            let mut cmd = Command::new("cargo");
            cmd.arg("build").arg("-p").arg(&project.package);
            if self.profile == "release" {
                cmd.arg("--release");
            }
            cmd.current_dir(&self.root);
            match with_log(&mut cmd, &managed.log, false).spawn() {
                Ok(child) => {
                    managed.build = Some(child);
                    managed.state = ProcState::Building;
                }
                Err(e) => managed.state = ProcState::Failed(format!("cannot run cargo: {e}")),
            }
        }
        self.procs.insert(project.name.clone(), managed);
    }

    fn spawn_renderer(&self, managed: &mut Managed) {
        let bin = self.binary(&managed.project);
        let mut cmd = Command::new(&bin);
        cmd.arg("--port")
            .arg(managed.project.port.to_string())
            .arg("--exit-with-parent")
            .current_dir(&managed.project.dir);
        if std::env::var_os("RUST_LOG").is_none() {
            cmd.env("RUST_LOG", "error,zygote_render=info");
        }
        match with_log(&mut cmd, &managed.log, true).spawn() {
            Ok(child) => {
                managed.child = Some(child);
                managed.probe = ParamSender::connect(managed.project.target()).ok();
                managed.since = Instant::now();
                managed.state = ProcState::Starting;
            }
            Err(e) => {
                managed.state = ProcState::Failed(format!("cannot start {}: {e}", bin.display()))
            }
        }
    }

    pub fn stop(&mut self, name: &str) {
        if let Some(mut m) = self.procs.remove(name) {
            for child in m.build.iter_mut().chain(m.child.iter_mut()) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    pub fn stop_all(&mut self) {
        let names: Vec<String> = self.procs.keys().cloned().collect();
        for name in names {
            self.stop(&name);
        }
    }

    /// Advance every managed process; returns the names whose state changed.
    pub fn poll(&mut self) -> Vec<String> {
        let mut changed = Vec::new();
        let names: Vec<String> = self.procs.keys().cloned().collect();
        for name in names {
            let before = self.state(&name);
            self.poll_one(&name);
            if self.state(&name) != before {
                changed.push(name);
            }
        }
        changed
    }

    fn poll_one(&mut self, name: &str) {
        let Some(mut m) = self.procs.remove(name) else {
            return;
        };
        match m.state {
            ProcState::Building => {
                if let Some(build) = m.build.as_mut() {
                    match build.try_wait() {
                        Ok(Some(status)) if status.success() => {
                            m.build = None;
                            self.spawn_renderer(&mut m);
                        }
                        Ok(Some(status)) => {
                            m.build = None;
                            m.state = ProcState::Failed(format!(
                                "cargo build failed ({status}); see {}",
                                m.log.display()
                            ));
                        }
                        Ok(None) => {}
                        Err(e) => m.state = ProcState::Failed(format!("build: {e}")),
                    }
                }
            }
            ProcState::Starting => {
                if let Some(status) = m.child.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
                    m.state = ProcState::Failed(format!(
                        "renderer exited while starting ({status}); see {}",
                        m.log.display()
                    ));
                } else if let Some(probe) = m.probe.as_mut() {
                    let _ = probe.send(&Message::Ping);
                    if probe
                        .poll()
                        .iter()
                        .any(|msg| matches!(msg, Message::Pong { .. }))
                    {
                        m.state = ProcState::Running;
                        m.probe = None;
                    } else if m.since.elapsed() > START_TIMEOUT {
                        m.state = ProcState::Failed(format!(
                            "no answer on port {} after {}s",
                            m.project.port,
                            START_TIMEOUT.as_secs()
                        ));
                    }
                }
            }
            ProcState::Running => {
                if let Some(status) = m.child.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
                    m.child = None;
                    m.state = if status.success() {
                        ProcState::Stopped
                    } else {
                        ProcState::Failed(format!(
                            "renderer exited ({status}); see {}",
                            m.log.display()
                        ))
                    };
                }
            }
            ProcState::Stopped | ProcState::Failed(_) => {}
        }
        self.procs.insert(name.to_owned(), m);
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// Send a process's output to the project's log file.
fn with_log<'a>(cmd: &'a mut Command, log: &Path, truncate: bool) -> &'a mut Command {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(truncate)
        .append(!truncate)
        .open(log);
    match file {
        Ok(out) => {
            let err = out.try_clone().ok();
            cmd.stdout(Stdio::from(out));
            if let Some(err) = err {
                cmd.stderr(Stdio::from(err));
            }
        }
        Err(_) => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    cmd.stdin(Stdio::null())
}
