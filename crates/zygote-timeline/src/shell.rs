//! The window's root view: the project browser, or the timeline for the
//! project that is open. Owns the renderer processes through `Supervisor`.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::{Options, TimelineApp, TimelineEvent};
use crate::projects::{self, Project};
use crate::supervisor::{ProcState, Supervisor};

const POLL: Duration = Duration::from_millis(400);

enum Screen {
    Projects,
    Show {
        /// Project name when opened from the browser; `None` when the show
        /// was given on the command line.
        project: Option<String>,
        view: Entity<TimelineApp>,
        _events: Subscription,
    },
}

pub struct Shell {
    root: Option<PathBuf>,
    projects: Vec<Project>,
    supervisor: Option<Supervisor>,
    screen: Screen,
    /// Open this project's timeline as soon as its renderer answers.
    pending_open: Option<String>,
}

impl Shell {
    /// Start in the browser, or straight in a show when the command line named one.
    pub fn new(options: Option<Options>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let root = projects::workspace_root();
        let projects = root.as_deref().map(projects::discover).unwrap_or_default();
        let mut this = Self {
            supervisor: root.clone().map(Supervisor::new),
            root,
            projects,
            screen: Screen::Projects,
            pending_open: None,
        };
        if let Some(options) = options {
            this.enter_show(None, options, window, cx);
        }
        // Closing the window tears the renderers down too.
        cx.on_app_quit(|this, _| {
            if let Some(sup) = this.supervisor.as_mut() {
                sup.stop_all();
            }
            async {}
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this
                    .update_in(cx, |this, window, cx| this.poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sup) = self.supervisor.as_mut() else {
            return;
        };
        let changed = sup.poll();
        if changed.is_empty() {
            return;
        }
        let states: Vec<(String, ProcState)> =
            changed.iter().map(|n| (n.clone(), sup.state(n))).collect();
        for (name, state) in &states {
            if state.is_running() && self.pending_open.as_deref() == Some(name.as_str()) {
                self.pending_open = None;
                self.open(name.clone(), window, cx);
            }
            // The project on screen died: back to the list, which says why.
            if !state.is_running()
                && !state.is_busy()
                && matches!(&self.screen, Screen::Show { project: Some(p), .. } if p == name)
            {
                self.screen = Screen::Projects;
            }
        }
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = &self.root {
            self.projects = projects::discover(root);
        }
        cx.notify();
    }

    fn launch(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(project) = self.projects.iter().find(|p| p.name == name).cloned() else {
            return;
        };
        if let Some(sup) = self.supervisor.as_mut() {
            sup.launch(&project);
            self.pending_open = Some(project.name);
        }
        cx.notify();
    }

    fn stop(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(sup) = self.supervisor.as_mut() {
            sup.stop(name);
        }
        if self.pending_open.as_deref() == Some(name) {
            self.pending_open = None;
        }
        if matches!(&self.screen, Screen::Show { project: Some(p), .. } if p == name) {
            self.screen = Screen::Projects;
        }
        cx.notify();
    }

    fn open(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.iter().find(|p| p.name == name).cloned() else {
            return;
        };
        let options = Options {
            file: project.show_path(),
            target: project.target(),
            auto_tile: true,
        };
        self.enter_show(Some(name), options, window, cx);
    }

    fn enter_show(
        &mut self,
        project: Option<String>,
        options: Options,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|cx| TimelineApp::new(options, window, cx));
        let events = cx.subscribe(&view, |this, _, event: &TimelineEvent, cx| match event {
            TimelineEvent::Projects => {
                this.refresh(cx);
                this.screen = Screen::Projects;
                cx.notify();
            }
        });
        let focus = view.read(cx).focus_handle();
        window.focus(&focus, cx);
        self.screen = Screen::Show {
            project,
            view,
            _events: events,
        };
        cx.notify();
    }

    fn render_projects(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (fg, muted, border, primary, danger, warning, success) = (
            theme.foreground,
            theme.muted_foreground,
            theme.border,
            theme.primary,
            theme.danger,
            theme.warning,
            theme.success,
        );
        let mut list = v_flex().gap_2().w_full().max_w(px(820.));
        list = list.child(
            h_flex()
                .items_center()
                .gap_2()
                .pb_2()
                .child(div().text_lg().text_color(fg).child("Projects"))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .truncate()
                        .text_color(muted)
                        .child(match &self.root {
                            Some(root) => root.join("projects").display().to_string(),
                            None => "no workspace found (run from the repository)".to_owned(),
                        }),
                )
                .child(
                    Button::new("refresh")
                        .small()
                        .ghost()
                        .icon(IconName::RotateCw)
                        .tooltip("Re-read the projects directory")
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
        );
        if self.projects.is_empty() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("No projects found. A project is a crate under projects/ with assets/graphs/main.json."),
            );
        }
        for (i, project) in self.projects.iter().enumerate() {
            let state = self
                .supervisor
                .as_ref()
                .map(|s| s.state(&project.name))
                .unwrap_or(ProcState::Stopped);
            let pending = self.pending_open.as_deref() == Some(project.name.as_str());
            let (label, color) = match &state {
                ProcState::Stopped => ("stopped".to_owned(), muted),
                ProcState::Building => ("building…".to_owned(), warning),
                ProcState::Starting => ("starting…".to_owned(), warning),
                ProcState::Running => (format!("running · port {}", project.port), success),
                ProcState::Failed(_) => ("failed".to_owned(), danger),
            };
            let mut meta = Vec::new();
            match &project.graph {
                Some(g) => meta.push(format!(
                    "graph `{}` · {} nodes → {}",
                    g.name, g.nodes, g.output
                )),
                None => meta.push("no assets/graphs/main.json".to_owned()),
            }
            meta.push(match &project.show_file {
                Some(f) => format!(
                    "show: {}",
                    f.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ),
                None => "no show file yet (Save creates one)".to_owned(),
            });
            meta.push(format!("port {}", project.port));
            let launch_name = project.name.clone();
            let stop_name = project.name.clone();
            let restart_name = project.name.clone();
            let mut card = v_flex()
                .gap_1()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(if state.is_running() { primary } else { border })
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_base().text_color(fg).child(project.name.clone()))
                                .when(!project.description.is_empty(), |d| {
                                    d.child(div().text_xs().text_color(muted).child(project.description.clone()))
                                })
                                .child(div().text_xs().text_color(muted).child(meta.join(" · "))),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_md()
                                .text_xs()
                                .bg(color.opacity(0.12))
                                .text_color(color)
                                .child(label),
                        )
                        .child(
                            Button::new(("launch", i))
                                .small()
                                .primary()
                                .icon(if state.is_running() { IconName::ArrowRight } else { IconName::Play })
                                .label(if state.is_running() { "Open" } else { "Launch" })
                                .disabled(state.is_busy() || self.supervisor.is_none())
                                .tooltip(if state.is_running() {
                                    "Open the timeline for this running renderer"
                                } else {
                                    "Build if needed, start the renderer, open the timeline once it answers"
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let running = this
                                        .supervisor
                                        .as_ref()
                                        .is_some_and(|s| s.state(&launch_name).is_running());
                                    if running {
                                        this.open(launch_name.clone(), window, cx);
                                    } else {
                                        this.launch(&launch_name, cx);
                                    }
                                })),
                        )
                        .child(
                            Button::new(("restart", i))
                                .small()
                                .icon(IconName::RotateCw)
                                .disabled(matches!(state, ProcState::Stopped) || self.supervisor.is_none())
                                .tooltip("Stop and start the renderer again")
                                .on_click(cx.listener(move |this, _, _, cx| this.launch(&restart_name, cx))),
                        )
                        .child(
                            Button::new(("stop", i))
                                .small()
                                .danger()
                                .icon(IconName::Close)
                                .disabled(matches!(state, ProcState::Stopped | ProcState::Failed(_)))
                                .tooltip("Stop the renderer")
                                .on_click(cx.listener(move |this, _, _, cx| this.stop(&stop_name, cx))),
                        ),
                );
            if let ProcState::Failed(why) = &state {
                card = card.child(div().text_xs().text_color(danger).child(why.clone()));
            } else if pending {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("opening the timeline as soon as the renderer answers…"),
                );
            }
            list = list.child(card);
        }
        v_flex()
            .size_full()
            .items_center()
            .p_6()
            .child(list)
            .into_any_element()
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (background, foreground) = {
            let theme = cx.theme();
            (theme.background, theme.foreground)
        };
        let content = match &self.screen {
            Screen::Show { view, .. } => view.clone().into_any_element(),
            Screen::Projects => self.render_projects(cx),
        };
        div()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(content)
    }
}
