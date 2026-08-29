//! Personas — user-created agents stored as plain markdown files.
//!
//! A persona is one `<name>.md` inside a persona directory: a `---`
//! frontmatter block with `name` / `description`, followed by the role
//! prompt that becomes part of the agent's system context. See
//! `docs/HIVE_BOT.md`. On a name collision between directories the earlier
//! one wins, so project personas can override global ones.

use std::path::{Path, PathBuf};

/// Directory inside a scope root that holds persona files.
pub const AGENTS_DIR: &str = "agents";

#[derive(Debug, Clone)]
pub struct PersonaMeta {
    pub name: String,
    pub description: String,
    /// File this persona was loaded from.
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct Persona {
    pub meta: PersonaMeta,
    /// Role prompt: everything after the frontmatter block.
    pub prompt: String,
}

impl Persona {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Persona {
            meta: PersonaMeta {
                name,
                description: description.into(),
                path: String::new(),
            },
            prompt: prompt.into(),
        }
    }

    pub fn to_markdown(&self) -> String {
        format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}",
            self.meta.name, self.meta.description, self.prompt
        )
    }
}

/// An ordered set of personas loaded from persona directories.
pub struct PersonaBook {
    personas: Vec<Persona>,
}

impl PersonaBook {
    /// Load every `<dir>/<name>.md` from each directory, in order. Missing
    /// directories are ignored. When two directories define the same persona
    /// name, the earlier directory keeps it.
    pub fn load(dirs: Vec<PathBuf>) -> Self {
        let mut personas: Vec<Persona> = Vec::new();
        for dir in dirs {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let fallback = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "agent".to_string());
                let (name, description, prompt) = parse_persona(&content, &fallback);
                if personas.iter().any(|p| p.meta.name == name) {
                    tracing::info!("persona `{name}` overridden by an earlier scope");
                    continue;
                }
                personas.push(Persona {
                    meta: PersonaMeta {
                        name,
                        description,
                        path: path.display().to_string(),
                    },
                    prompt,
                });
            }
        }
        PersonaBook { personas }
    }

    pub fn list(&self) -> Vec<PersonaMeta> {
        self.personas.iter().map(|p| p.meta.clone()).collect()
    }

    pub fn read(&self, name: &str) -> Option<&Persona> {
        self.personas.iter().find(|p| p.meta.name == name)
    }

    /// Write a persona into a scope's agent dir (`<root>/<name>.md`),
    /// overwriting any previous version. Returns the written path.
    pub fn save(root: &Path, persona: &Persona) -> std::io::Result<PathBuf> {
        let name = persona.meta.name.trim();
        if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid persona name",
            ));
        }
        std::fs::create_dir_all(root)?;
        let path = root.join(format!("{name}.md"));
        std::fs::write(&path, persona.to_markdown())?;
        Ok(path)
    }
}

/// Split a persona file into `(name, description, prompt)`.
fn parse_persona(content: &str, fallback_name: &str) -> (String, String, String) {
    let mut name = fallback_name.to_string();
    let mut description = String::new();
    let mut prompt = content.trim().to_string();

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
            prompt = rest[end + 4..].trim().to_string();
        }
    }

    (name, description, prompt)
}

fn unquote(s: &str) -> String {
    s.trim_matches('"').trim_matches('\'').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_scope(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("hive-persona-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn parse_splits_frontmatter_and_prompt() {
        let (name, description, prompt) = parse_persona(
            "---\nname: vasya\ndescription: \"The manager\"\n---\n\nYou are Vasya.\nKeep it short.",
            "fallback",
        );
        assert_eq!(name, "vasya");
        assert_eq!(description, "The manager");
        assert!(prompt.starts_with("You are Vasya."));
        assert!(!prompt.contains("---"));
    }

    #[test]
    fn missing_frontmatter_falls_back_to_filename_and_body() {
        let (name, _, prompt) = parse_persona("Just do design work.", "designer");
        assert_eq!(name, "designer");
        assert_eq!(prompt, "Just do design work.");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let scope = tmp_scope("roundtrip");
        let persona = Persona::new("vasya", "Manager", "Talk to Artem politely.");
        PersonaBook::save(&scope.join(AGENTS_DIR), &persona).unwrap();

        let book = PersonaBook::load(vec![scope.join(AGENTS_DIR)]);
        let metas = book.list();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].name, "vasya");
        assert_eq!(metas[0].description, "Manager");
        assert_eq!(
            book.read("vasya").unwrap().prompt,
            "Talk to Artem politely."
        );
        let _ = std::fs::remove_dir_all(&scope);
    }

    #[test]
    fn earlier_scope_wins_on_name_collision() {
        let global = tmp_scope("global");
        let project = tmp_scope("project");
        PersonaBook::save(&global, &Persona::new("dev", "Global dev", "global prompt")).unwrap();
        PersonaBook::save(
            &project,
            &Persona::new("dev", "Project dev", "project prompt"),
        )
        .unwrap();

        let book = PersonaBook::load(vec![project.clone(), global.clone()]);
        let dev = book.read("dev").unwrap();
        assert_eq!(dev.meta.description, "Project dev");
        assert_eq!(dev.prompt, "project prompt");
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&global);
    }

    #[test]
    fn rejects_unsafe_names() {
        let p = Persona::new("../escape", "", "");
        assert!(PersonaBook::save(Path::new("/tmp"), &p).is_err());
    }
}
