//! Built-in "inside skills" — compiled into the binary, immutable, always
//! available. They describe hive's own workflows so the agent can pull them
//! with `read_skill` just like user skills, but they can't be edited or removed.

use crate::skill::{SkillMeta, SkillSource};

struct Inside {
    name: &'static str,
    description: &'static str,
    content: &'static str,
}

const WORKFLOW: &str = r#"---
name: workflow
description: Phased pipeline for multi-step work — plan, spawn jobs, wait, close, verify.
---

# Workflow

A phased pipeline. The MULTITASK parent starts workers as background jobs,
waits once, then closes them. Never dump 30 agents at once.

## Phases

### 1. Understand
Read/search the codebase. Never plan blind.

### 2. Plan
Call `write_plan` — one-line summary + markdown body (steps, files per step,
risks, verification).

### 3. Set tasks
Call `set_todos` with the concrete steps.

### 4. Spawn jobs
Each independent piece is one `spawn_subagent` with:
- `task` — the worker's entire brief (it sees none of this conversation)
- `paths` — files it will edit (overlap is rejected)

Spawn several in one turn. They return ids immediately.

Rules:
- Default 3 slots, hard max 6. Close finished workers before starting more.
- Workers in the same batch must touch DIFFERENT files.
- A job that can't be split is one worker, not five vague ones.

### 5. Wait
Call `agent_wait` with `on: "all"` (or a single id). Do not poll `agent_observe`.
Observe only when you need to steer; `agent_message` to correct a worker.

### 6. Close
`agent_close` every id. That merges the worker's changes. Always close —
an unfinished slot still counts. Conflicts stay on the slot; message the
worker or close with `discard: true`.

### 7. Verify
`switch_mode` to make. Build, test, fix. Then `set_todos` with an empty list.

## Writing worker briefs

Each worker sees NOTHING of your conversation. Include:
- Goal (what "done" looks like)
- Files to touch (exact paths)
- Constraints (style, libs, don't touch X)
- Self-verification (run this command, expect this output)
"#;

const INSIDE_SKILLS: &[Inside] = &[Inside {
    name: "workflow",
    description: "How to structure work — plan, spawn jobs, wait, close, verify.",
    content: WORKFLOW,
}];

pub struct InsideSkills;

impl SkillSource for InsideSkills {
    fn list(&self) -> Vec<SkillMeta> {
        INSIDE_SKILLS
            .iter()
            .map(|s| SkillMeta {
                name: s.name.to_string(),
                description: s.description.to_string(),
                path: "(built-in)".to_string(),
            })
            .collect()
    }

    fn read(&self, name: &str) -> Option<String> {
        INSIDE_SKILLS
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.content.to_string())
    }
}

/// Merges multiple skill sources. Earlier sources win on name collisions
/// (inside skills take priority over disk skills with the same name).
pub struct CompositeSkills {
    sources: Vec<Box<dyn SkillSource>>,
}

impl CompositeSkills {
    pub fn new(sources: Vec<Box<dyn SkillSource>>) -> Self {
        CompositeSkills { sources }
    }
}

impl SkillSource for CompositeSkills {
    fn list(&self) -> Vec<SkillMeta> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for src in &self.sources {
            for meta in src.list() {
                if seen.insert(meta.name.clone()) {
                    out.push(meta);
                }
            }
        }
        out
    }

    fn read(&self, name: &str) -> Option<String> {
        for src in &self.sources {
            if let Some(content) = src.read(name) {
                return Some(content);
            }
        }
        None
    }
}
