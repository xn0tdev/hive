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
description: Phased pipeline for multi-step work — plan, batch subagents by category, track, integrate, verify.
---

# Workflow

A phased pipeline. Work flows through stages; each stage fans out subagents
in bounded batches, waits for results, then feeds them into the next stage.
Never dump 30 agents at once — the machine and the TUI must stay responsive.

## Phases

### 1. Understand
Read/search the codebase. Never plan blind.

### 2. Plan
Call `write_plan` — one-line summary + markdown body (steps, files per step,
risks, verification). The plan is the source of truth for splitting work.

### 3. Set tasks
Call `set_todos` with the concrete steps. The user sees a live progress bar.

### 4. Categorize & batch
Split the plan into **categories** (independent groups of work):

- **Implement** — code changes (each worker touches different files)
- **Verify** — independent checkers (build, lint, test, review)
- **Research** — read-only exploration (API shapes, docs, dependencies)

For each category, spawn a **batch** of workers:

```
spawn_swarm([task_1, task_2, ..., task_N])
```

Rules:
- Max 6–8 workers per batch. The runtime caps concurrency at 8.
  If you have 20 tasks, run them in 3 batches of ~7, not one of 20.
- Workers in the same batch must touch DIFFERENT files.
- Wait for the batch to finish before starting the next one.
  Results from batch N inform batch N+1.
- A job that can't be split is one `spawn_subagent`, not a swarm.

### 5. Integrate
After each implement batch returns:
- Call `integrate_worktree` for every worker id. Every one.
- A conflict rolls back the merge — report which worker/files, don't retry blindly.
- Mark the corresponding todos done.

### 6. Verify
Spawn a verify batch (1–3 workers):
- One runs build + tests + lint.
- One does a code review pass (optional for small changes).
- Fix what broke. Do not declare victory over a red check.

### 7. Clear
Call `set_todos` with an empty list. Give a short summary.

## Writing worker briefs

Each worker sees NOTHING of your conversation. Its task string is its
entire world. Include:
- Goal (what "done" looks like)
- Files to touch (exact paths)
- Constraints (style, libs, don't touch X)
- Self-verification (run this command, expect this output)

## Concurrency & resource rules

- The runtime semaphore caps concurrent agents (default 8). Respect it.
- Batch size ≤ 8. Smaller batches (3–5) are safer for file-conflict risk.
- Between batches: integrate, update todos, assess, then spawn next batch.
- Never spawn a batch while the previous one is still running.
- The TUI renders each subagent card. 30 cards at once = lag. Keep it sane.
- Prefer 3 focused workers over 10 vague ones.

## Tracking rules

- One `set_todos` call replaces the whole list — pass every task each time.
- Mark done only when actually finished (integrated + verified).
- Discover a new step mid-flight? Add it before doing it.
- Task unnecessary? Remove it, don't mark it done.
- Don't narrate the progress bar. Just call the tool and work.
"#;

const INSIDE_SKILLS: &[Inside] = &[Inside {
    name: "workflow",
    description:
        "How to structure work — plan, fan out subagents, track progress, integrate, verify.",
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
