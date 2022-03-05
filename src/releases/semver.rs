use std::fmt::Display;

use color_eyre::eyre::WrapErr;
use color_eyre::eyre::{eyre, Result};
use semver::{Prerelease, Version};
use serde::Deserialize;

use crate::{package_json, pyproject, state};

/// The various rules that can be used when bumping the current version of a project via
/// [`crate::step::Step::BumpVersion`].
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "rule")]
pub(crate) enum Rule {
    Major,
    Minor,
    Patch,
    Pre {
        label: String,
        #[serde(skip_deserializing)]
        fallback_rule: ConventionalRule,
    },
    Release,
}

impl From<ConventionalRule> for Rule {
    fn from(fallback_rule: ConventionalRule) -> Self {
        match fallback_rule {
            ConventionalRule::Major => Rule::Major,
            ConventionalRule::Minor => Rule::Minor,
            ConventionalRule::Patch => Rule::Patch,
        }
    }
}

/// The rules that can be derived from Conventional Commits.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConventionalRule {
    Major,
    Minor,
    Patch,
}

impl Default for ConventionalRule {
    fn default() -> Self {
        ConventionalRule::Patch
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PackageVersion {
    pub(crate) version: Version,
    package_manager: PackageManager,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PackageManager {
    Cargo,
    Poetry,
    JavaScript,
}

impl Display for PackageVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.version)
    }
}

pub(super) fn bump_version(rule: Rule, dry_run: bool) -> Result<semver::Version> {
    let mut package_version = get_version()?;
    package_version.version =
        bump(package_version.version, rule).wrap_err("While bumping version")?;
    if !dry_run {
        set_version(&package_version)?;
    }
    Ok(package_version.version)
}

pub(crate) fn bump_version_and_update_state(
    mut state: state::State,
    rule: Rule,
) -> Result<state::State> {
    let version = bump_version(rule, false)?;
    state.release = state::Release::Bumped(version);
    Ok(state)
}

pub(crate) fn get_version() -> Result<PackageVersion> {
    if let Some(cargo_version) = crate::cargo::get_version("Cargo.toml") {
        let version = semver::Version::parse(&cargo_version).wrap_err_with(|| {
            format!(
                "Found {} in Cargo.toml which is not a valid version",
                cargo_version
            )
        })?;
        Ok(PackageVersion {
            version,
            package_manager: PackageManager::Cargo,
        })
    } else if let Some(pyproject_version) = pyproject::get_version("pyproject.toml") {
        let version = semver::Version::parse(&pyproject_version).wrap_err_with(|| {
            format!(
                "Found {} in pyproject.toml which is not a valid version",
                pyproject_version
            )
        })?;
        Ok(PackageVersion {
            version,
            package_manager: PackageManager::Poetry,
        })
    } else if let Some(package_version) = package_json::get_version("package.json") {
        let version = semver::Version::parse(&package_version).wrap_err_with(|| {
            format!(
                "Found {} in package.json which is not a valid version",
                package_version
            )
        })?;
        Ok(PackageVersion {
            version,
            package_manager: PackageManager::JavaScript,
        })
    } else {
        Err(eyre!("No supported metadata found to parse version from"))
    }
}

fn set_version(version: &PackageVersion) -> Result<()> {
    match version.package_manager {
        PackageManager::Cargo => crate::cargo::set_version("Cargo.toml", &version.to_string())
            .wrap_err("While bumping Cargo.toml"),
        PackageManager::Poetry => pyproject::set_version("pyproject.toml", &version.to_string())
            .wrap_err("While bumping pyproject.toml"),
        PackageManager::JavaScript => {
            package_json::set_version("package.json", &version.to_string())
                .wrap_err("While bumping package.json")
        }
    }
}

/// Apply a Rule to a [`PackageVersion`], incrementing & resetting the correct components.
///
/// ### Versions 0.x
///
/// Versions with major component 0 have special meaning in Semantic Versioning and therefore have
/// different behavior:
/// 1. [`Rule::Major`] will bump the minor component.
/// 2. [`Rule::Minor`] will bump the patch component.
fn bump(mut version: Version, rule: Rule) -> Result<Version> {
    let is_0 = version.major == 0;
    match (rule, is_0) {
        (Rule::Major, false) => {
            version.major += 1;
            version.minor = 0;
            version.patch = 0;
            version.pre = Prerelease::EMPTY;
            Ok(version)
        }
        (Rule::Minor, false) | (Rule::Major, true) => {
            version.minor += 1;
            version.patch = 0;
            version.pre = Prerelease::EMPTY;
            Ok(version)
        }
        (Rule::Patch, _) | (Rule::Minor, true) => {
            version.patch += 1;
            version.pre = Prerelease::EMPTY;
            Ok(version)
        }
        (Rule::Release, _) => {
            version.pre = Prerelease::EMPTY;
            Ok(version)
        }
        (
            Rule::Pre {
                label: prefix,
                fallback_rule,
            },
            _,
        ) => bump_pre(version, &prefix, fallback_rule),
    }
}

#[cfg(test)]
mod test_bump {
    use super::*;

    #[test]
    fn major() {
        let version = Version::new(1, 2, 3);
        let version = bump(version, Rule::Major).unwrap();

        assert_eq!(version, Version::new(2, 0, 0));
    }

    #[test]
    fn major_0() {
        let version = Version::new(0, 1, 2);
        let version = bump(version, Rule::Major).unwrap();

        assert_eq!(version, Version::new(0, 2, 0));
    }

    #[test]
    fn minor() {
        let version = Version::new(1, 2, 3);
        let version = bump(version, Rule::Minor).unwrap();

        assert_eq!(version, Version::new(1, 3, 0));
    }

    #[test]
    fn minor_0() {
        let version = Version::new(0, 1, 2);
        let version = bump(version, Rule::Minor).unwrap();

        assert_eq!(version, Version::new(0, 1, 3));
    }

    #[test]
    fn patch() {
        let version = Version::new(1, 2, 3);
        let version = bump(version, Rule::Patch).unwrap();

        assert_eq!(version, Version::new(1, 2, 4));
    }

    #[test]
    fn patch_0() {
        let version = Version::new(1, 2, 3);
        let version = bump(version, Rule::Patch).unwrap();

        assert_eq!(version, Version::new(1, 2, 4));
    }

    #[test]
    fn pre() {
        let version = Version::new(1, 2, 3);
        let version = bump(
            version,
            Rule::Pre {
                label: String::from("rc"),
                fallback_rule: ConventionalRule::Minor,
            },
        )
        .unwrap();

        assert_eq!(version, Version::parse("1.3.0-rc.0").unwrap());
    }

    #[test]
    fn release() {
        let version = Version::parse("1.2.3-rc.0").unwrap();
        let version = bump(version, Rule::Release).unwrap();

        assert_eq!(version, Version::new(1, 2, 3));
    }
}

/// Bumps the pre-release component of a [`Version`].
///
/// If the existing [`Version`] has no pre-release,
/// `semantic_rule` will be used to bump to primary components before the
/// pre-release component is added.
///
/// # Errors
///
/// Can fail if there is an existing pre-release component that can't be incremented.
fn bump_pre(
    mut version: Version,
    prefix: &str,
    fallback_rule: ConventionalRule,
) -> Result<Version> {
    if version.pre.is_empty() {
        let mut version = bump(version, fallback_rule.into())?;
        version.pre = Prerelease::new(&format!("{}.0", prefix))?;
        return Ok(version);
    }

    let pre_string = version.pre.as_str();
    let parts = pre_string.split('.').collect::<Vec<_>>();

    if parts.len() != 2 {
        return Err(eyre!(
            "A prerelease version already exists but could not be incremented"
        ));
    }

    if parts[0] != prefix {
        version.pre = Prerelease::new(&format!("{}.0", prefix))?;
        return Ok(version);
    }
    let pre_version = parts[1].parse::<u16>()?;
    version.pre = Prerelease::new(&format!("{}.{}", prefix, pre_version + 1))?;
    Ok(version)
}