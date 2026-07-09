use std::path::{Path, PathBuf};

use crate::error::ConfluenceError;

/// A skill bundled into the binary at build time.
pub struct BundledSkill {
    pub name: &'static str,
    pub content: &'static str,
}

pub const BUNDLED_SKILLS: &[BundledSkill] = &[
    BundledSkill {
        name: "confluence-lookup",
        content: include_str!("../.apm/skills/confluence-lookup/SKILL.md"),
    },
    BundledSkill {
        name: "jira-lookup",
        content: include_str!("../.apm/skills/jira-lookup/SKILL.md"),
    },
];

/// The outcome of a [`install_skill`] call.
#[derive(Debug)]
pub enum InstallOutcome {
    /// The skill file was freshly created.
    Installed,
    /// An existing file with different content was overwritten via `--force`.
    Overwritten,
    /// The existing file already contained identical content; nothing changed.
    AlreadyUpToDate,
}

/// Resolve the default skill installation directory: `~/.agents/skills`.
pub fn default_skills_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".agents").join("skills"))
}

/// Install `content` to `<skills_dir>/<name>/SKILL.md`.
///
/// Behaviour:
/// - Destination absent → create parent directories and write.
/// - Destination exists, same content → return [`InstallOutcome::AlreadyUpToDate`].
/// - Destination exists, different content, `force = false` → error with `--force` hint.
/// - Destination exists, different content, `force = true` → overwrite and return
///   [`InstallOutcome::Overwritten`].
pub fn install_skill(
    skills_dir: &Path,
    name: &str,
    content: &str,
    force: bool,
) -> Result<(PathBuf, InstallOutcome), ConfluenceError> {
    let dest_dir = skills_dir.join(name);
    let dest = dest_dir.join("SKILL.md");

    std::fs::create_dir_all(&dest_dir).map_err(|e| {
        ConfluenceError::SkillError(format!(
            "cannot create directory {}: {}",
            dest_dir.display(),
            e
        ))
    })?;

    if dest.exists() {
        let existing = std::fs::read_to_string(&dest).map_err(|e| {
            ConfluenceError::SkillError(format!("cannot read {}: {}", dest.display(), e))
        })?;

        if existing == content {
            return Ok((dest, InstallOutcome::AlreadyUpToDate));
        }

        if !force {
            return Err(ConfluenceError::SkillError(format!(
                "{} exists with different content; re-run with --force to overwrite",
                dest.display()
            )));
        }

        std::fs::write(&dest, content).map_err(|e| {
            ConfluenceError::SkillError(format!("cannot write {}: {}", dest.display(), e))
        })?;
        return Ok((dest, InstallOutcome::Overwritten));
    }

    std::fs::write(&dest, content).map_err(|e| {
        ConfluenceError::SkillError(format!("cannot write {}: {}", dest.display(), e))
    })?;
    Ok((dest, InstallOutcome::Installed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn install_writes_new_file() {
        let dir = tempdir().unwrap();
        let skill = &BUNDLED_SKILLS[0];
        let (path, outcome) = install_skill(dir.path(), skill.name, skill.content, false).unwrap();
        assert!(matches!(outcome, InstallOutcome::Installed));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), skill.content);
    }

    #[test]
    fn already_up_to_date() {
        let dir = tempdir().unwrap();
        let skill = &BUNDLED_SKILLS[0];
        install_skill(dir.path(), skill.name, skill.content, false).unwrap();
        let (_, outcome) = install_skill(dir.path(), skill.name, skill.content, false).unwrap();
        assert!(matches!(outcome, InstallOutcome::AlreadyUpToDate));
    }

    #[test]
    fn different_content_without_force_errors() {
        let dir = tempdir().unwrap();
        let skill = &BUNDLED_SKILLS[0];
        install_skill(dir.path(), skill.name, skill.content, false).unwrap();
        // Tamper with the installed file.
        let dest = dir.path().join(skill.name).join("SKILL.md");
        std::fs::write(&dest, "modified content").unwrap();

        let err = install_skill(dir.path(), skill.name, skill.content, false).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("--force"),
            "expected --force hint in error: {msg}"
        );
    }

    #[test]
    fn force_overwrites_different_content() {
        let dir = tempdir().unwrap();
        let skill = &BUNDLED_SKILLS[0];
        install_skill(dir.path(), skill.name, skill.content, false).unwrap();
        let dest = dir.path().join(skill.name).join("SKILL.md");
        std::fs::write(&dest, "modified content").unwrap();

        let (path, outcome) = install_skill(dir.path(), skill.name, skill.content, true).unwrap();
        assert!(matches!(outcome, InstallOutcome::Overwritten));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), skill.content);
    }

    #[test]
    fn embedded_skill_content_has_expected_name() {
        for skill in BUNDLED_SKILLS {
            let marker = format!("name: {}", skill.name);
            assert!(
                skill.content.contains(&marker),
                "embedded SKILL.md for '{}' must contain '{}'",
                skill.name,
                marker
            );
        }
    }
}
