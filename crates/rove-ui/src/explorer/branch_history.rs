//! 已提交的本地 Git 历史。只读取提交和树对象。
use git2::{Delta, Oid, Repository, Sort};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub(super) type Changes = BTreeMap<String, Change>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    Added,
    Modified,
    Deleted,
}

impl Change {
    pub(super) fn badge(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Commit {
    pub id: Oid,
    pub author: String,
    pub subject: String,
    parent: Option<Oid>,
}

#[derive(Clone, Debug)]
pub(super) struct Branch {
    pub name: String,
    pub commits: Vec<Commit>,
    repository: PathBuf,
    prefix: PathBuf,
    net: Changes,
}

fn diff(
    repo: &Repository,
    prefix: &Path,
    base: Option<Oid>,
    end: Oid,
) -> Result<Changes, git2::Error> {
    let old = base.map(|id| repo.find_commit(id)?.tree()).transpose()?;
    let new = repo.find_commit(end)?.tree()?;
    let diff = repo.diff_tree_to_tree(old.as_ref(), Some(&new), None)?;
    let mut changes = Changes::new();
    for delta in diff.deltas() {
        let (file, change) = match delta.status() {
            Delta::Added => (delta.new_file(), Change::Added),
            Delta::Deleted => (delta.old_file(), Change::Deleted),
            Delta::Modified | Delta::Typechange => (delta.new_file(), Change::Modified),
            _ => continue,
        };
        let Some(relative) = file.path().and_then(|path| path.strip_prefix(prefix).ok()) else {
            continue;
        };
        let Some(path) = relative.to_str() else {
            continue;
        };
        let path = path.replace(std::path::MAIN_SEPARATOR, "/");
        if rove_base::WorkspacePath::new(&path).is_ok() {
            changes.insert(path, change);
        }
    }
    Ok(changes)
}

impl Branch {
    pub(super) fn load(root: &Path) -> Result<Self, git2::Error> {
        let repo = Repository::discover(root)?;
        let workdir = repo
            .workdir()
            .ok_or_else(|| git2::Error::from_str("bare repository"))?;
        let root = root
            .canonicalize()
            .map_err(|err| git2::Error::from_str(&err.to_string()))?;
        let workdir = workdir
            .canonicalize()
            .map_err(|err| git2::Error::from_str(&err.to_string()))?;
        let prefix = root
            .strip_prefix(workdir)
            .map_err(|_| git2::Error::from_str("workspace is outside the repository"))?
            .to_owned();
        let reference = repo.head()?;
        if !reference.is_branch() {
            return Err(git2::Error::from_str("detached HEAD"));
        }
        let name = reference.shorthand().unwrap_or("HEAD").to_owned();
        let head = reference.peel_to_commit()?.id();
        let baseline = repo
            .find_reference("refs/heads/main")
            .or_else(|_| repo.find_reference("refs/remotes/origin/main"))
            .or_else(|_| repo.find_reference("refs/remotes/origin/HEAD"))?
            .peel_to_commit()?
            .id();
        let base = repo.merge_base(head, baseline)?;
        let net = diff(&repo, &prefix, Some(base), head)?;
        let mut history = repo.revwalk()?;
        history.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
        history.push(head)?;
        history.hide(base)?;
        let mut commits = Vec::new();
        for id in history {
            let id = id?;
            let commit = repo.find_commit(id)?;
            let author = commit.author();
            commits.push(Commit {
                id,
                author: format!(
                    "{} <{}>",
                    author.name().unwrap_or_default(),
                    author.email().unwrap_or_default()
                ),
                subject: commit.summary().unwrap_or_default().to_owned(),
                parent: commit.parent_ids().next(),
            });
        }
        Ok(Self {
            name,
            commits,
            repository: repo.path().to_owned(),
            prefix,
            net,
        })
    }

    pub(super) fn changes(
        &self,
        author: Option<&str>,
        commit_id: Option<Oid>,
    ) -> Result<Changes, git2::Error> {
        if commit_id.is_none() && author.is_none() {
            return Ok(self.net.clone());
        }
        let repo = Repository::open(&self.repository)?;
        if let Some(id) = commit_id {
            let commit = self
                .commits
                .iter()
                .find(|commit| commit.id == id)
                .ok_or_else(|| git2::Error::from_str("commit outside branch range"))?;
            return diff(&repo, &self.prefix, commit.parent, id);
        }
        let mut touched = BTreeSet::new();
        for commit in self
            .commits
            .iter()
            .filter(|commit| Some(commit.author.as_str()) == author)
        {
            touched.extend(diff(&repo, &self.prefix, commit.parent, commit.id)?.into_keys());
        }
        Ok(self
            .net
            .iter()
            .filter(|(path, _)| touched.contains(*path))
            .map(|(path, change)| (path.clone(), *change))
            .collect())
    }
}
