//! Filesystem-backed issue store.

use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
};

use snafu::{ResultExt, ensure};
use uuid::Uuid;

use crate::{
    error::{
        AlreadyInitialisedSnafu, AmbiguousIssueSnafu, CanonicalisePathSnafu, CreateDirSnafu,
        CreateSymlinkSnafu, CurrentDirSnafu, DuplicateIssueIdSnafu, DuplicateIssueRefSnafu,
        InvalidIssueDirSnafu, InvalidIssueFileNameSnafu, InvalidRefAliasSnafu, InvalidRefSnafu,
        IssueFileNameMismatchSnafu, IssueNotFoundSnafu, MissingRefAliasSnafu, MissingStoreSnafu,
        NotGitRepositorySnafu, ParseTomlSnafu, ReadDirSnafu, ReadFileSnafu, ReadLinkSnafu,
        ReadMetadataSnafu, RefAliasTargetMismatchSnafu, RefExistsSnafu, RemoveFileSnafu, Result,
        SerialiseTomlSnafu, WriteFileSnafu,
    },
    model::{Config, Issue},
};

const CONFIG_DIR: &str = ".gitrack";
pub(crate) const DEFAULT_ISSUES_DIR: &str = "issues";
const CONFIG_FILE: &str = "config.toml";
const ISSUES_BY_ID_DIR: &str = "issues-by-id";
const REF_ALIAS_EXTENSION: &str = "toml";
const MIN_GENERATED_REF_SUFFIX_LEN: usize = 3;
const BASE36_DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

#[derive(Debug, Clone)]
pub(crate) struct Store {
    pub(crate) root: PathBuf,
    pub(crate) dir: PathBuf,
    pub(crate) issues_dir: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) config: Config,
}

impl Store {
    /// Resolve the Git working tree root without requiring a gitrack store.
    pub(crate) fn root_for_worktree(start: &Path) -> Result<PathBuf> {
        find_git_root(start)
    }

    pub(crate) fn init(
        start: &Path,
        explicit_prefix: Option<String>,
        issue_dir_config: String,
    ) -> Result<Self> {
        let root = find_git_root(start)?;
        validate_issue_dir(&issue_dir_config)?;
        let config_dir = root.join(CONFIG_DIR);
        let issues_dir = root.join(&issue_dir_config);
        let config_path = config_dir.join(CONFIG_FILE);

        ensure!(
            !config_path.exists(),
            AlreadyInitialisedSnafu {
                path: config_path.clone()
            }
        );

        fs::create_dir_all(&config_dir).context(CreateDirSnafu {
            path: config_dir.clone(),
        })?;
        fs::create_dir_all(&issues_dir).context(CreateDirSnafu {
            path: issues_dir.clone(),
        })?;
        let canonical_dir = issues_dir.join(ISSUES_BY_ID_DIR);
        fs::create_dir_all(&canonical_dir).context(CreateDirSnafu {
            path: canonical_dir.clone(),
        })?;

        let prefix = match explicit_prefix {
            Some(prefix) => {
                validate_ref(&prefix)?;
                prefix
            }
            None => derive_ref_prefix(&root)?,
        };
        let config = Config::new(prefix, issue_dir_config);
        let serialised = toml::to_string_pretty(&config).context(SerialiseTomlSnafu)?;
        fs::write(&config_path, serialised).context(WriteFileSnafu {
            path: config_path.clone(),
        })?;

        Ok(Self {
            root,
            dir: config_dir,
            issues_dir,
            config_path,
            config,
        })
    }

    pub(crate) fn open(start: &Path) -> Result<Self> {
        let root = match find_store_root(start)? {
            Some(root) => root,
            None => find_git_root(start)?,
        };

        let config_dir = root.join(CONFIG_DIR);
        let config_path = config_dir.join(CONFIG_FILE);

        ensure!(
            config_path.exists(),
            MissingStoreSnafu { root: root.clone() }
        );

        let config_text = fs::read_to_string(&config_path).context(ReadFileSnafu {
            path: config_path.clone(),
        })?;
        let config: Config = toml::from_str(&config_text).context(ParseTomlSnafu {
            path: config_path.clone(),
        })?;
        validate_issue_dir(&config.issue_dir)?;
        let issues_dir = root.join(&config.issue_dir);

        Ok(Self {
            root,
            dir: config_dir,
            issues_dir,
            config_path,
            config,
        })
    }

    pub(crate) fn load_issues(&self) -> Result<Vec<Issue>> {
        let mut issues = Vec::new();
        let canonical_dir = self.issue_data_dir();
        let entries = match fs::read_dir(&canonical_dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(issues),
            Err(source) => {
                return Err(source).context(ReadDirSnafu {
                    path: canonical_dir,
                });
            }
        };
        let mut paths_by_id = HashMap::new();
        let mut ids_by_ref = HashMap::new();

        for entry in entries {
            let entry = entry.context(ReadDirSnafu {
                path: self.issue_data_dir(),
            })?;
            let path = entry.path();
            if path.extension() != Some(OsStr::new(REF_ALIAS_EXTENSION)) {
                continue;
            }
            let file_id = issue_id_from_path(&path)?;

            let issue_text =
                fs::read_to_string(&path).context(ReadFileSnafu { path: path.clone() })?;
            let issue: Issue =
                toml::from_str(&issue_text).context(ParseTomlSnafu { path: path.clone() })?;
            ensure!(
                issue.id == file_id,
                IssueFileNameMismatchSnafu {
                    path: path.clone(),
                    file_id,
                    issue_id: issue.id
                }
            );
            validate_ref(&issue.reference)?;
            if let Some(first_path) = paths_by_id.insert(issue.id, path.clone()) {
                return DuplicateIssueIdSnafu {
                    id: issue.id,
                    first_path,
                    duplicate_path: path,
                }
                .fail();
            }
            if let Some(first_id) = ids_by_ref.insert(issue.reference.clone(), issue.id) {
                return DuplicateIssueRefSnafu {
                    reference: issue.reference,
                    first_id,
                    duplicate_id: issue.id,
                }
                .fail();
            }
            issues.push(issue);
        }

        issues.sort_by_key(|issue| issue.id);
        self.validate_ref_aliases(&issues)?;
        Ok(issues)
    }

    pub(crate) fn save_issue(&self, issue: &Issue) -> Result<()> {
        validate_ref(&issue.reference)?;
        let path = self.issue_path(issue.id);
        fs::create_dir_all(&self.issues_dir).context(CreateDirSnafu {
            path: self.issues_dir.clone(),
        })?;
        let canonical_dir = self.issue_data_dir();
        fs::create_dir_all(&canonical_dir).context(CreateDirSnafu {
            path: canonical_dir.clone(),
        })?;
        let serialised = toml::to_string_pretty(issue).context(SerialiseTomlSnafu)?;
        fs::write(&path, serialised).context(WriteFileSnafu { path })?;
        self.ensure_ref_alias(issue)?;
        self.remove_stale_ref_aliases(issue)?;
        Ok(())
    }

    pub(crate) fn issue_path(&self, id: Uuid) -> PathBuf {
        self.issue_data_dir().join(format!("{id}.toml"))
    }

    pub(crate) fn issue_data_dir(&self) -> PathBuf {
        self.issues_dir.join(ISSUES_BY_ID_DIR)
    }

    pub(crate) fn ref_alias_path(&self, reference: &str) -> PathBuf {
        self.issues_dir.join(format!("{reference}.toml"))
    }

    pub(crate) fn resolve_issue<'issues>(
        issues: &'issues [Issue],
        identifier: &str,
    ) -> Result<&'issues Issue> {
        if let Ok(id) = Uuid::parse_str(identifier)
            && let Some(issue) = issues.iter().find(|issue| issue.id == id)
        {
            return Ok(issue);
        }

        let matches: Vec<&Issue> = issues
            .iter()
            .filter(|issue| issue.reference == identifier)
            .collect();

        match matches.as_slice() {
            [issue] => Ok(*issue),
            [] => IssueNotFoundSnafu {
                reference: identifier.to_string(),
            }
            .fail(),
            many => {
                let matches = many
                    .iter()
                    .map(|issue| issue.id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                AmbiguousIssueSnafu {
                    reference: identifier.to_string(),
                    matches,
                }
                .fail()
            }
        }
    }

    pub(crate) fn resolve_issue_mut<'issues>(
        issues: &'issues mut [Issue],
        identifier: &str,
    ) -> Result<&'issues mut Issue> {
        let resolved_issue = Self::resolve_issue(issues, identifier)?;
        let id = resolved_issue.id;
        let issue = issues
            .iter_mut()
            .find(|issue| issue.id == id)
            .expect("resolved issue id must be present in mutable issue slice");
        Ok(issue)
    }

    pub(crate) fn ensure_ref_available(
        issues: &[Issue],
        reference: &str,
        except: Option<Uuid>,
    ) -> Result<()> {
        validate_ref(reference)?;
        let exists = issues
            .iter()
            .any(|issue| issue.reference == reference && Some(issue.id) != except);
        ensure!(
            !exists,
            RefExistsSnafu {
                reference: reference.to_string()
            }
        );
        Ok(())
    }

    pub(crate) fn generated_ref(&self, issues: &[Issue]) -> Result<String> {
        let existing_refs = issues
            .iter()
            .map(|issue| issue.reference.as_str())
            .collect::<HashSet<_>>();
        let token = uuid_to_base36(Uuid::now_v7());

        generated_ref_for_token(&self.config.ref_prefix, &token, &existing_refs)
    }

    /// Ensure the issue's current ref has a valid alias, creating it when absent.
    fn ensure_ref_alias(&self, issue: &Issue) -> Result<()> {
        let alias_path = self.ref_alias_path(&issue.reference);
        match fs::symlink_metadata(&alias_path) {
            Ok(_metadata) => Self::validate_alias_path(&alias_path, &issue.reference, issue.id),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                let target = ref_alias_target(issue.id);
                create_issue_symlink(&target, &alias_path)
            }
            Err(source) => Err(source).context(ReadMetadataSnafu { path: alias_path }),
        }
    }

    /// Remove old aliases that still point at this issue after a ref rename.
    fn remove_stale_ref_aliases(&self, issue: &Issue) -> Result<()> {
        let entries = match fs::read_dir(&self.issues_dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(source).context(ReadDirSnafu {
                    path: self.issues_dir.clone(),
                });
            }
        };
        let expected_target = ref_alias_target(issue.id);

        for entry in entries {
            let entry = entry.context(ReadDirSnafu {
                path: self.issues_dir.clone(),
            })?;
            let path = entry.path();
            if is_issue_data_dir_path(&path) {
                continue;
            }

            let metadata =
                fs::symlink_metadata(&path).context(ReadMetadataSnafu { path: path.clone() })?;
            if !metadata.file_type().is_symlink() {
                continue;
            }

            let reference = alias_reference_from_path(&path)?;
            if reference == issue.reference {
                continue;
            }

            let target = fs::read_link(&path).context(ReadLinkSnafu { path: path.clone() })?;
            if target == expected_target {
                fs::remove_file(&path).context(RemoveFileSnafu { path })?;
            }
        }

        Ok(())
    }

    /// Validate the bidirectional invariant between issue refs and alias paths.
    fn validate_ref_aliases(&self, issues: &[Issue]) -> Result<()> {
        let mut refs_by_name = HashMap::new();
        for issue in issues {
            let alias_path = self.ref_alias_path(&issue.reference);
            Self::validate_alias_path(&alias_path, &issue.reference, issue.id)?;
            refs_by_name.insert(issue.reference.as_str(), issue.id);
        }

        let entries = match fs::read_dir(&self.issues_dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(source).context(ReadDirSnafu {
                    path: self.issues_dir.clone(),
                });
            }
        };

        for entry in entries {
            let entry = entry.context(ReadDirSnafu {
                path: self.issues_dir.clone(),
            })?;
            let path = entry.path();
            if is_issue_data_dir_path(&path) {
                let metadata = fs::symlink_metadata(&path)
                    .context(ReadMetadataSnafu { path: path.clone() })?;
                ensure!(
                    metadata.is_dir(),
                    InvalidRefAliasSnafu {
                        path,
                        reason: format!("reserved path `{ISSUES_BY_ID_DIR}` must be a directory")
                    }
                );
                continue;
            }

            let reference = alias_reference_from_path(&path)?;
            validate_ref(&reference)?;
            let Some(id) = refs_by_name.get(reference.as_str()) else {
                return InvalidRefAliasSnafu {
                    path,
                    reason: format!("ref `{reference}` does not match any issue"),
                }
                .fail();
            };
            Self::validate_alias_path(&path, &reference, *id)?;
        }

        Ok(())
    }

    /// Validate one alias as a relative symlink to the canonical UUID file.
    fn validate_alias_path(path: &Path, reference: &str, id: Uuid) -> Result<()> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return MissingRefAliasSnafu {
                    reference: reference.to_string(),
                    path: path.to_path_buf(),
                }
                .fail();
            }
            Err(source) => {
                return Err(source).context(ReadMetadataSnafu {
                    path: path.to_path_buf(),
                });
            }
        };

        ensure!(
            metadata.file_type().is_symlink(),
            InvalidRefAliasSnafu {
                path: path.to_path_buf(),
                reason: "must be a symlink".to_string()
            }
        );

        let target = fs::read_link(path).context(ReadLinkSnafu {
            path: path.to_path_buf(),
        })?;
        let expected_target = ref_alias_target(id);
        ensure!(
            target == expected_target,
            RefAliasTargetMismatchSnafu {
                path: path.to_path_buf(),
                expected: expected_target,
                actual: target
            }
        );
        Ok(())
    }
}

pub(crate) fn normalise_labels(labels: Vec<String>) -> Vec<String> {
    let mut labels = labels
        .into_iter()
        .flat_map(|label| {
            label
                .split(',')
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    labels
}

pub(crate) fn normalise_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn validate_ref(reference: &str) -> Result<()> {
    ensure!(
        !reference.trim().is_empty(),
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "must not be empty"
        }
    );
    ensure!(
        reference == reference.trim(),
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "must not have leading or trailing whitespace"
        }
    );
    ensure!(
        reference.chars().all(is_ref_char),
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "only ASCII letters, digits, dots, underscores, and dashes are supported"
        }
    );
    ensure!(
        reference != "." && reference != "..",
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "must be usable as a file name"
        }
    );
    ensure!(
        reference != ISSUES_BY_ID_DIR,
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "`issues-by-id` is reserved for canonical issue files"
        }
    );
    ensure!(
        !reference.to_ascii_lowercase().ends_with(".toml"),
        InvalidRefSnafu {
            reference: reference.to_string(),
            reason: "must not end in .toml; alias paths add that extension"
        }
    );
    Ok(())
}

pub(crate) fn validate_issue_dir(path: &str) -> Result<()> {
    ensure!(
        !path.trim().is_empty(),
        InvalidIssueDirSnafu {
            path: path.to_string(),
            reason: "must not be empty"
        }
    );
    ensure!(
        path == path.trim(),
        InvalidIssueDirSnafu {
            path: path.to_string(),
            reason: "must not have leading or trailing whitespace"
        }
    );

    let path_value = Path::new(path);
    ensure!(
        !path_value.is_absolute(),
        InvalidIssueDirSnafu {
            path: path.to_string(),
            reason: "must be relative to the Git root"
        }
    );

    let mut has_normal_component = false;
    for component in path_value.components() {
        match component {
            Component::Normal(_) => has_normal_component = true,
            Component::CurDir => {}
            Component::ParentDir => {
                InvalidIssueDirSnafu {
                    path: path.to_string(),
                    reason: "must not contain parent-directory traversal",
                }
                .fail()?;
            }
            Component::Prefix(_) | Component::RootDir => {
                InvalidIssueDirSnafu {
                    path: path.to_string(),
                    reason: "must be relative to the Git root",
                }
                .fail()?;
            }
        }
    }

    ensure!(
        has_normal_component,
        InvalidIssueDirSnafu {
            path: path.to_string(),
            reason: "must name a directory"
        }
    );
    Ok(())
}

fn find_git_root(start: &Path) -> Result<PathBuf> {
    let start = normalise_start(start)?;
    for ancestor in start.ancestors() {
        if ancestor.join(".git").exists() {
            return Ok(ancestor.to_path_buf());
        }
    }

    NotGitRepositorySnafu {
        start: start.clone(),
    }
    .fail()
}

fn find_store_root(start: &Path) -> Result<Option<PathBuf>> {
    let start = normalise_start(start)?;
    Ok(start
        .ancestors()
        .find(|ancestor| ancestor.join(CONFIG_DIR).join(CONFIG_FILE).exists())
        .map(Path::to_path_buf))
}

fn normalise_start(start: &Path) -> Result<PathBuf> {
    if start == Path::new(".") {
        return env::current_dir().context(CurrentDirSnafu);
    }

    start.canonicalize().context(CanonicalisePathSnafu {
        path: start.to_path_buf(),
    })
}

fn issue_id_from_path(path: &Path) -> Result<Uuid> {
    let Some(file_stem) = path.file_stem().and_then(OsStr::to_str) else {
        return InvalidIssueFileNameSnafu {
            path: path.to_path_buf(),
            reason: "file name must be a UUID with .toml extension",
        }
        .fail();
    };

    Uuid::parse_str(file_stem).map_err(|_source| {
        InvalidIssueFileNameSnafu {
            path: path.to_path_buf(),
            reason: "file name must be a UUID with .toml extension",
        }
        .build()
    })
}

/// Extract the logical ref from a top-level `<ref>.toml` alias path.
fn alias_reference_from_path(path: &Path) -> Result<String> {
    let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
        return InvalidRefAliasSnafu {
            path: path.to_path_buf(),
            reason: "file name must be valid UTF-8".to_string(),
        }
        .fail();
    };

    ensure!(
        path.extension() == Some(OsStr::new(REF_ALIAS_EXTENSION)),
        InvalidRefAliasSnafu {
            path: path.to_path_buf(),
            reason: "alias file name must end in .toml".to_string()
        }
    );

    let Some(reference) = path.file_stem().and_then(OsStr::to_str) else {
        return InvalidRefAliasSnafu {
            path: path.to_path_buf(),
            reason: format!("alias file name `{file_name}` must contain a ref before .toml"),
        }
        .fail();
    };

    Ok(reference.to_string())
}

/// Build the tracked relative symlink target used for issue ref aliases.
fn ref_alias_target(id: Uuid) -> PathBuf {
    PathBuf::from(format!("{ISSUES_BY_ID_DIR}/{id}.toml"))
}

fn is_issue_data_dir_path(path: &Path) -> bool {
    path.file_name() == Some(OsStr::new(ISSUES_BY_ID_DIR))
}

#[cfg(unix)]
/// Create an issue ref alias symlink on Unix platforms.
fn create_issue_symlink(target: &Path, path: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, path).context(CreateSymlinkSnafu {
        path: path.to_path_buf(),
        target: target.to_path_buf(),
    })
}

#[cfg(windows)]
/// Create an issue ref alias symlink on Windows platforms.
fn create_issue_symlink(target: &Path, path: &Path) -> Result<()> {
    std::os::windows::fs::symlink_file(target, path).context(CreateSymlinkSnafu {
        path: path.to_path_buf(),
        target: target.to_path_buf(),
    })
}

#[cfg(not(any(unix, windows)))]
/// Report unsupported symlink creation on platforms without a known API.
fn create_issue_symlink(target: &Path, path: &Path) -> Result<()> {
    crate::error::UnsupportedSymlinkSnafu {
        path: path.to_path_buf(),
        target: target.to_path_buf(),
    }
    .fail()
}

fn generated_ref_for_token(
    prefix: &str,
    token: &str,
    existing_refs: &HashSet<&str>,
) -> Result<String> {
    let minimum_len = MIN_GENERATED_REF_SUFFIX_LEN.min(token.len());
    for suffix_len in minimum_len..token.len() {
        let suffix_start = token.len() - suffix_len;
        let candidate = format!("{prefix}-{}", &token[suffix_start..]);
        if !existing_refs.contains(candidate.as_str()) {
            return Ok(candidate);
        }
    }

    let candidate = format!("{prefix}-{token}");
    ensure!(
        !existing_refs.contains(candidate.as_str()),
        RefExistsSnafu {
            reference: candidate
        }
    );
    Ok(candidate)
}

fn uuid_to_base36(id: Uuid) -> String {
    base36_encode(id.as_u128())
}

fn base36_encode(mut value: u128) -> String {
    if value == 0 {
        return "0".to_string();
    }

    let mut digits = Vec::new();
    while value > 0 {
        let digit = usize::try_from(value % 36).expect("base36 digit fits usize");
        digits.push(char::from(BASE36_DIGITS[digit]));
        value /= 36;
    }
    digits.iter().rev().collect()
}

fn derive_ref_prefix(root: &Path) -> Result<String> {
    let raw_name = root.file_name().and_then(OsStr::to_str).unwrap_or("issues");
    let mut prefix = String::new();
    let mut previous_was_dash = false;

    for character in raw_name.chars() {
        if character.is_ascii_alphanumeric() {
            prefix.push(character.to_ascii_lowercase());
            previous_was_dash = false;
        } else if !previous_was_dash && !prefix.is_empty() {
            prefix.push('-');
            previous_was_dash = true;
        }
    }

    let prefix = prefix.trim_matches('-').to_string();
    validate_ref(&prefix)?;
    Ok(prefix)
}

fn is_ref_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '.' || character == '_' || character == '-'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::Error,
        model::{IssueStatus, NewIssue, now_rfc3339},
    };
    use std::fs;

    #[test]
    fn labels_are_split_sorted_and_deduplicated() {
        let labels = normalise_labels(vec![
            "rust, cli".to_string(),
            "agent".to_string(),
            "cli".to_string(),
        ]);

        assert_eq!(labels, vec!["agent", "cli", "rust"]);
    }

    #[test]
    fn dotted_child_refs_are_valid() {
        assert!(validate_ref("gitrack-a1b2c3d4.1").is_ok());
    }

    #[test]
    fn generated_ref_uses_minimum_base36_suffix() {
        let existing_refs = HashSet::new();
        let reference =
            generated_ref_for_token("gitrack", "123abc", &existing_refs).expect("generated ref");

        assert_eq!(reference, "gitrack-abc");
        assert!(
            reference
                .rsplit_once('-')
                .expect("suffix")
                .1
                .chars()
                .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        );
    }

    #[test]
    fn generated_ref_uses_shortest_unique_suffix() {
        let existing_refs = HashSet::from(["gitrack-abc", "gitrack-3abc"]);
        let reference =
            generated_ref_for_token("gitrack", "123abc", &existing_refs).expect("generated ref");

        assert_eq!(reference, "gitrack-23abc");
    }

    #[test]
    fn generated_ref_uses_full_token_when_shortened_suffixes_collide() {
        let existing_refs = HashSet::from(["gitrack-abc"]);
        let reference =
            generated_ref_for_token("gitrack", "1abc", &existing_refs).expect("generated ref");

        assert_eq!(reference, "gitrack-1abc");
    }

    #[test]
    fn init_derives_prefix_from_git_root_name() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let root = temp.path().join("My Project");
        fs::create_dir_all(root.join(".git")).expect("create fake git dir");
        let canonical_root = root.canonicalize().expect("canonical root");

        let store =
            Store::init(&root, None, DEFAULT_ISSUES_DIR.to_string()).expect("initialise store");

        assert_eq!(store.config.ref_prefix, "my-project");
        assert_eq!(store.config.issue_dir, DEFAULT_ISSUES_DIR);
        assert!(store.config_path.exists());
        assert!(store.issues_dir.exists());
        assert_eq!(
            store.config_path,
            canonical_root.join(CONFIG_DIR).join(CONFIG_FILE)
        );
        assert_eq!(store.issues_dir, canonical_root.join(DEFAULT_ISSUES_DIR));
        assert_eq!(
            store.issue_data_dir(),
            canonical_root
                .join(DEFAULT_ISSUES_DIR)
                .join(ISSUES_BY_ID_DIR)
        );
        assert!(store.issue_data_dir().exists());
    }

    #[test]
    fn missing_issue_dir_loads_as_empty_and_is_recreated_on_write() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let root = temp.path().join("project");
        fs::create_dir_all(root.join(".git")).expect("create fake git dir");
        let store =
            Store::init(&root, None, DEFAULT_ISSUES_DIR.to_string()).expect("initialise store");
        fs::remove_dir_all(&store.issues_dir).expect("remove issue dir");

        let issues = store.load_issues().expect("load issues");
        assert!(issues.is_empty());

        let issue = test_issue("project-a1b2c3d4");
        store.save_issue(&issue).expect("save issue");

        assert!(store.issues_dir.exists());
        assert!(store.issue_path(issue.id).exists());
        assert_ref_alias(&store, &issue);
    }

    #[test]
    fn save_issue_creates_ref_alias() {
        let (_temp, store) = test_store();
        let issue = test_issue("project-a1b2c3d4");

        store.save_issue(&issue).expect("save issue");

        assert_ref_alias(&store, &issue);
    }

    #[test]
    fn save_issue_removes_stale_ref_alias_on_ref_rename() {
        let (_temp, store) = test_store();
        let mut issue = test_issue("project-a1b2c3d4");
        store.save_issue(&issue).expect("save issue");
        let old_alias_path = store.ref_alias_path(&issue.reference);

        issue.reference = "project-renamed".to_string();
        store.save_issue(&issue).expect("save renamed issue");

        assert!(fs::symlink_metadata(old_alias_path).is_err());
        assert_ref_alias(&store, &issue);
        let issues = store.load_issues().expect("load issues");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].reference, "project-renamed");
    }

    #[test]
    fn issue_file_name_must_match_issue_id() {
        let (_temp, store) = test_store();
        let issue = test_issue("project-a1b2c3d4");
        let wrong_id = Uuid::now_v7();
        write_issue(&store.issue_path(wrong_id), &issue);

        let error = store.load_issues().expect_err("file name mismatch");

        assert!(matches!(error, Error::IssueFileNameMismatch { .. }));
    }

    #[test]
    fn duplicate_issue_ids_are_rejected() {
        let (_temp, store) = test_store();
        let issue = test_issue("project-a1b2c3d4");
        write_issue(&store.issue_path(issue.id), &issue);
        write_issue(
            &store
                .issue_data_dir()
                .join(format!("{}.toml", issue.id.simple())),
            &issue,
        );

        let error = store.load_issues().expect_err("duplicate issue id");

        assert!(matches!(error, Error::DuplicateIssueId { .. }));
    }

    #[test]
    fn duplicate_issue_refs_are_rejected() {
        let (_temp, store) = test_store();
        let first_issue = test_issue("project-a1b2c3d4");
        let second_issue = test_issue("project-a1b2c3d4");
        write_issue(&store.issue_path(first_issue.id), &first_issue);
        write_issue(&store.issue_path(second_issue.id), &second_issue);

        let error = store.load_issues().expect_err("duplicate issue ref");

        assert!(matches!(error, Error::DuplicateIssueRef { .. }));
    }

    #[test]
    fn missing_ref_aliases_are_rejected() {
        let (_temp, store) = test_store();
        let issue = test_issue("project-a1b2c3d4");
        write_issue(&store.issue_path(issue.id), &issue);

        let error = store.load_issues().expect_err("missing ref alias");

        assert!(matches!(error, Error::MissingRefAlias { .. }));
    }

    #[test]
    fn non_symlink_ref_aliases_are_rejected() {
        let (_temp, store) = test_store();
        let issue = test_issue("project-a1b2c3d4");
        write_issue(&store.issue_path(issue.id), &issue);
        fs::write(store.ref_alias_path(&issue.reference), b"not a symlink")
            .expect("write alias file");

        let error = store.load_issues().expect_err("non-symlink ref alias");

        assert!(matches!(error, Error::InvalidRefAlias { .. }));
    }

    #[test]
    fn stale_ref_alias_targets_are_rejected() {
        let (_temp, store) = test_store();
        let issue = test_issue("project-a1b2c3d4");
        write_issue(&store.issue_path(issue.id), &issue);
        let alias_path = store.ref_alias_path(&issue.reference);
        let wrong_target = ref_alias_target(Uuid::now_v7());
        create_issue_symlink(&wrong_target, &alias_path).expect("create stale ref alias");

        let error = store.load_issues().expect_err("stale ref alias");

        assert!(matches!(error, Error::RefAliasTargetMismatch { .. }));
    }

    #[test]
    fn extra_ref_aliases_are_rejected() {
        let (_temp, store) = test_store();
        let issue = test_issue("project-a1b2c3d4");
        store.save_issue(&issue).expect("save issue");
        let alias_path = store.ref_alias_path("project-extra");
        let target = ref_alias_target(issue.id);
        create_issue_symlink(&target, &alias_path).expect("create extra ref alias");

        let error = store.load_issues().expect_err("extra ref alias");

        assert!(matches!(error, Error::InvalidRefAlias { .. }));
    }

    #[test]
    fn issue_dir_must_be_relative() {
        assert!(validate_issue_dir("/tmp/issues").is_err());
    }

    #[test]
    fn issue_dir_must_not_escape_root() {
        assert!(validate_issue_dir("../issues").is_err());
        assert!(validate_issue_dir("nested/../../issues").is_err());
    }

    #[test]
    fn refs_must_not_collide_with_reserved_paths() {
        assert!(validate_ref(ISSUES_BY_ID_DIR).is_err());
        assert!(validate_ref("project-a1b2c3d4.toml").is_err());
        assert!(validate_ref("project:a1b2c3d4").is_err());
    }

    fn test_store() -> (tempfile::TempDir, Store) {
        let temp = tempfile::tempdir().expect("create tempdir");
        let root = temp.path().join("project");
        fs::create_dir_all(root.join(".git")).expect("create fake git dir");
        let store =
            Store::init(&root, None, DEFAULT_ISSUES_DIR.to_string()).expect("initialise store");
        (temp, store)
    }

    fn test_issue(reference: &str) -> Issue {
        let now = now_rfc3339().expect("timestamp");
        Issue::new(NewIssue {
            id: Uuid::now_v7(),
            reference: reference.to_string(),
            title: format!("Issue {reference}"),
            body: String::new(),
            status: IssueStatus::Open,
            kind: "task".to_string(),
            priority: 3,
            labels: Vec::new(),
            assignee: None,
            blocked_by: Vec::new(),
            now,
        })
    }

    fn write_issue(path: &Path, issue: &Issue) {
        let serialised = toml::to_string_pretty(issue).expect("serialise issue");
        fs::write(path, serialised).expect("write issue");
    }

    fn assert_ref_alias(store: &Store, issue: &Issue) {
        let alias_path = store.ref_alias_path(&issue.reference);
        let metadata = fs::symlink_metadata(&alias_path).expect("read alias metadata");
        assert!(metadata.file_type().is_symlink());
        let target = fs::read_link(alias_path).expect("read alias target");
        assert_eq!(target, ref_alias_target(issue.id));
    }
}
