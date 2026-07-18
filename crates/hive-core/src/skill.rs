use std::sync::Arc;

/// Metadata about an available skill, injected compactly into the system prompt
/// so the model knows what exists and can pull the full instructions on demand.
#[derive(Debug, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub path: String,
}

/// Where skills come from. The default implementation reads SKILL.md files from
/// disk, but a remote registry could implement the same trait.
pub trait SkillSource: Send + Sync {
    fn list(&self) -> Vec<SkillMeta>;
    /// Full contents of a skill's SKILL.md, by skill name.
    fn read(&self, name: &str) -> Option<String>;
}

/// A skill source with nothing in it, used before the skills system is wired.
pub struct NoSkills;

impl SkillSource for NoSkills {
    fn list(&self) -> Vec<SkillMeta> {
        Vec::new()
    }
    fn read(&self, _name: &str) -> Option<String> {
        None
    }
}

pub fn no_skills() -> Arc<dyn SkillSource> {
    Arc::new(NoSkills)
}
