//! Disk-backed `SkillSource`. A skill is a directory containing a `SKILL.md`
//! file whose YAML frontmatter provides `name` and `description`. Full
//! instructions (the whole file) are returned by `read` when the agent pulls
//! the skill in.

use std::path::PathBuf;

use crate::skill::{SkillMeta, SkillSource};

struct Loaded {
    meta: SkillMeta,
    content: String,
}

pub struct DiskSkills {
    skills: Vec<Loaded>,
}

impl DiskSkills {
    /// Load skills from each of the given base directories (expects
    /// `<base>/<skill-name>/SKILL.md`). Missing directories are ignored.
    pub fn load(dirs: Vec<PathBuf>) -> Self {
        let mut skills = Vec::new();
        for dir in dirs {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let skill_file = path.join("SKILL.md");
                let content = match std::fs::read_to_string(&skill_file) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let fallback_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "skill".to_string());
                let (name, description) = parse_frontmatter(&content, &fallback_name);
                tracing::info!("loaded skill: {name}");
                skills.push(Loaded {
                    meta: SkillMeta {
                        name,
                        description,
                        path: skill_file.display().to_string(),
                    },
                    content,
                });
            }
        }
        DiskSkills { skills }
    }
}

impl SkillSource for DiskSkills {
    fn list(&self) -> Vec<SkillMeta> {
        self.skills.iter().map(|s| s.meta.clone()).collect()
    }

    fn read(&self, name: &str) -> Option<String> {
        self.skills
            .iter()
            .find(|s| s.meta.name == name)
            .map(|s| s.content.clone())
    }
}

/// Extract `name` and `description` from a leading `---` frontmatter block.
fn parse_frontmatter(content: &str, fallback_name: &str) -> (String, String) {
    let mut name = fallback_name.to_string();
    let mut description = String::new();

    let trimmed = content.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let front = &rest[..end];
            for line in front.lines() {
                if let Some(v) = line.strip_prefix("name:") {
                    name = unquote(v.trim());
                } else if let Some(v) = line.strip_prefix("description:") {
                    description = unquote(v.trim());
                }
            }
        }
    }

    if description.is_empty() {
        // Fall back to the first non-empty, non-frontmatter line.
        description = content
            .lines()
            .find(|l| {
                let t = l.trim();
                !t.is_empty() && t != "---" && !t.starts_with('#')
            })
            .unwrap_or("")
            .trim()
            .to_string();
    }

    (name, description)
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    s.trim_matches('"').trim_matches('\'').to_string()
}
