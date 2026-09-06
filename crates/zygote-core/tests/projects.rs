//! Every project's show file must load, and everything it names (nodes,
//! parameters, modulation sources, triggers, cues) must exist.

use std::path::{Path, PathBuf};

use zygote_core::{Graph, KeyAction, NodeLibrary, ParamPath, SourceKind, Timeline};

fn show_file(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(".show.json"))
}

#[test]
fn project_show_files_are_consistent() {
    let projects = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../projects");
    let mut checked = 0;
    for entry in std::fs::read_dir(&projects).unwrap().flatten() {
        let dir = entry.path();
        let Some(show) = show_file(&dir) else {
            continue;
        };
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let timeline = Timeline::from_json(&std::fs::read_to_string(&show).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let graph = Graph::from_json(
            &std::fs::read_to_string(dir.join("assets/graphs/main.json")).unwrap(),
        )
        .unwrap_or_else(|e| panic!("{name}: graph: {e}"));

        // Builtin nodes plus the project's WGSL nodes. Rust nodes are not
        // known here, so parameters are only checked when the graph fully
        // validates against this library.
        let mut lib = NodeLibrary::builtin();
        let (_, errors) = lib.load_dir(dir.join("assets/nodes"));
        assert!(errors.is_empty(), "{name}: node header errors: {errors:?}");
        let full = graph.validate(&lib).is_ok();

        let check = |path: &ParamPath, what: &str| {
            let shown = format!("{}.{}", path.node, path.param);
            assert!(
                graph.node(&path.node).is_some(),
                "{name}: {what} `{shown}` names a node the graph does not have"
            );
            if full {
                graph
                    .param_spec(&lib, path)
                    .unwrap_or_else(|e| panic!("{name}: {what} `{shown}`: {e}"));
            }
        };
        for cue in &timeline.cues {
            for (path, value) in &cue.values {
                check(path, "cue value");
                if full {
                    let spec = graph.param_spec(&lib, path).unwrap();
                    assert_eq!(
                        spec.ty.conform(value),
                        *value,
                        "{name}: cue `{}` value for {}.{} is out of range or the wrong type",
                        cue.label,
                        path.node,
                        path.param
                    );
                }
            }
        }
        let m = &timeline.modulation;
        for a in &m.assignments {
            check(&a.target, "modulation target");
            assert!(
                m.source(&a.source).is_some(),
                "{name}: assignment to {}.{} uses unknown source `{}`",
                a.target.node,
                a.target.param,
                a.source
            );
        }
        for (key, action) in &timeline.keys {
            match action {
                KeyAction::Trigger { trigger } => assert!(
                    m.sources.iter().any(
                        |s| matches!(&s.kind, SourceKind::Envelope { trigger: t, .. } if t == trigger)
                    ),
                    "{name}: key `{key}` fires trigger `{trigger}` but no envelope listens to it"
                ),
                KeyAction::Cue { id } => assert!(
                    timeline.cues.iter().any(|c| c.id == *id),
                    "{name}: key `{key}` jumps to cue {id}, which does not exist"
                ),
            }
        }
        checked += 1;
    }
    assert!(checked >= 2, "expected to find project show files");
}
