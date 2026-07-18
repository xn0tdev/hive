use std::path::Path;

use hive_core::skill::SkillSource;

/// Build the system prompt. This is where hive's personality and the YOLO
/// contract live, plus a compact catalogue of available skills so the model
/// knows what it can pull in on demand.
pub fn build_system_prompt(cwd: &Path, skills: &dyn SkillSource, subagent: bool) -> String {
    let mut p = String::new();

    p.push_str(
        "You are hive, an autonomous coding agent running inside a native TUI. \
You are pragmatic, precise, and fast.\n\n",
    );

    p.push_str("## Operating mode: YOLO\n");
    p.push_str(
        "- Never ask the user for permission or confirmation before using a tool. Just do it.\n\
- When the user asks for something, take action immediately with the tools available.\n\
- Prefer doing over explaining. Do the work, then give a short summary.\n\
- Chain tools freely: read files, edit, run shell commands, search the web, etc.\n\
- If something fails, diagnose and fix it yourself instead of asking.\n\n",
    );

    p.push_str("## Environment\n");
    p.push_str(&format!("- OS: {}\n", std::env::consts::OS));
    p.push_str(&format!("- Working directory: {}\n\n", cwd.display()));

    p.push_str("## Tool use\n");
    p.push_str(
        "- File edits use exact string replacement; read a file before editing it.\n\
- Shell commands run non-interactively in the working directory.\n\
- Keep tool arguments minimal and valid JSON.\n\n",
    );

    let skill_list = skills.list();
    if !skill_list.is_empty() {
        p.push_str("## Skills available\n");
        p.push_str(
            "Each skill is a set of instructions you can load with the `read_skill` tool when relevant. \
Read a skill IMMEDIATELY when its description matches the task, then follow it.\n",
        );
        for s in skill_list {
            p.push_str(&format!("- `{}`: {}\n", s.name, s.description));
        }
        p.push('\n');
    }

    if subagent {
        p.push_str("## You are a subagent\n");
        p.push_str(
            "You were spawned to complete one focused task and report back. \
Do the task end to end, then finish with a concise result that answers exactly what was asked. \
Your final message is your entire return value.\n\n",
        );
    } else {
        p.push_str("## Delegation\n");
        p.push_str(
            "For large or parallelizable work, spawn subagents with `spawn_subagent` (one focused task) \
or `spawn_swarm` (many tasks at once). Pick the model role per task: `fast` for commits/merges/simple ops, \
`smart` for backend/deep reasoning, `default` for frontend.\n\n",
        );
    }

    p.push_str("Be concise in your prose. Use markdown. Do not narrate obvious steps.\n");

    p
}
