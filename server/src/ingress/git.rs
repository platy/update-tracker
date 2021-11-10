//! Helpers for git

use std::{cell::RefCell, path::Path, process::Command};

use anyhow::{format_err, Context, Result};
use git2::{Commit, Oid, Repository, Signature, Tree, TreeBuilder};

pub fn push(repo_base: impl AsRef<Path>) -> Result<()> {
    // let mut remote_callbacks = git2::RemoteCallbacks::new();
    // remote_callbacks.credentials(|_url, username_from_url, _allowed_types| {
    //     git2::Cred::ssh_key(
    //         username_from_url.unwrap(),
    //         None,
    //         std::path::Path::new(&format!("{}/.ssh/id_rsa", std::env::var("HOME").unwrap())),
    //         None,
    //     )
    // }).transfer_progress(|p| {
    //     println!(
    //         "Git pushing changes ({} received) {} objects {} ?",
    //         p.received_bytes(), p.total_deltas() + p.total_objects(), p.indexed_deltas() + p.received_objects()
    //     );
    //     true
    // })
    // .sideband_progress(move |line| {
    //     println!("sideband {}", std::str::from_utf8(line).unwrap_or(""));
    //     true
    // });
    // let repo = Repository::open(repo_base).context("Opening repo")?;
    // let mut remote = repo.find_remote("origin")?;
    // println!("Pushing to remote");
    // remote.push(
    //     &["refs/heads/main"],
    //     Some(git2::PushOptions::new().remote_callbacks(remote_callbacks)),
    // )?;
    let mut child = Command::new("git").current_dir(repo_base).arg("push").spawn()?;
    println!("git push resulted in : {}", child.wait()?);
    Ok(())
}

pub struct GitRepoWriter<'a> {
    git_repo: Repository,
    git_reference: &'a str,
}

impl<'a> GitRepoWriter<'a> {
    pub fn new(git_repo: &'a Path, git_reference: &'a str) -> Result<Self> {
        let git_repo = Repository::open(git_repo).context("Opening repo")?;
        Ok(Self {
            git_repo,
            git_reference,
        })
    }

    pub fn start_transaction(&self) -> Result<GitRepoTransaction<'a, '_>> {
        let parent = self.git_repo.find_reference(self.git_reference)?.peel_to_commit()?;
        Ok(GitRepoTransaction {
            writer: self,
            parent: RefCell::new(Some(parent)),
        })
    }
}

pub struct GitRepoTransaction<'a, 'b> {
    writer: &'b GitRepoWriter<'a>,
    parent: RefCell<Option<Commit<'b>>>,
}
impl<'a, 'b> GitRepoTransaction<'a, 'b> {
    pub(crate) fn commit(self, log_message: &'b str) -> Result<(), git2::Error> {
        let parent = self.parent.borrow().as_ref().unwrap().id();
        self.writer
            .git_repo
            .reference(self.writer.git_reference, parent, true, log_message)?;
        Ok(())
    }

    pub(crate) fn start_change(&mut self) -> Result<GitRepoChangeBuilder<'a, 'b, '_>, git2::Error> {
        let parent = self.parent.take();
        Ok(GitRepoChangeBuilder {
            transaction: self,
            commit_builder: CommitBuilder::new(&self.writer.git_repo, parent)?,
        })
    }
}

pub struct GitRepoChangeBuilder<'a, 'b, 'c> {
    transaction: &'c GitRepoTransaction<'a, 'b>,
    commit_builder: CommitBuilder<'b>,
}

impl<'a, 'b, 'c> GitRepoChangeBuilder<'a, 'b, 'c> {
    pub(crate) fn add_doc(&mut self, path: &Path, content: impl AsRef<[u8]>) -> Result<()> {
        // write the blob
        let oid = self.transaction.writer.git_repo.blob(content.as_ref())?;
        self.commit_builder.add_to_tree(path.to_str().unwrap(), oid, 0o100644)?;
        Ok(())
    }

    pub(crate) fn commit_update(self, updated_at: &str, change: &str, category: Option<&str>) -> Result<()> {
        let message = format!(
            "{}: {}{}",
            updated_at,
            change,
            category.map(|c| format!(" [{}]", c)).unwrap_or_default()
        );
        let govuk_sig = Signature::now("Gov.uk", "info@gov.uk")?;
        let gitgov_sig = Signature::now("Gitgov", "gitgov@njk.onl")?;
        let commit = self.commit_builder.commit(&govuk_sig, &gitgov_sig, &message)?;
        self.transaction.parent.replace(Some(commit));
        Ok(())
    }
}

pub struct CommitBuilder<'repo> {
    repo: &'repo Repository,
    tree_builder: TreeBuilder<'repo>,
    parent: Option<Commit<'repo>>,
}

impl<'repo> CommitBuilder<'repo> {
    /// Start building a commit on this repository
    pub fn new(repo: &'repo Repository, parent: Option<Commit<'repo>>) -> Result<Self, git2::Error> {
        let tree: Option<Tree<'_>> = parent.as_ref().map(Commit::tree).transpose()?;
        let tree_builder: TreeBuilder<'repo> = repo.treebuilder(tree.as_ref())?;
        Ok(CommitBuilder {
            repo,
            tree_builder,
            parent,
        })
    }

    pub fn add_to_tree(&mut self, path: &str, oid: Oid, file_mode: i32) -> Result<()> {
        write_to_path_in_tree(
            self.repo,
            &mut self.tree_builder,
            path.strip_prefix('/').context("relative path provided")?,
            oid,
            file_mode,
        )
    }

    /// Writes the built tree, a comit for it and updates the ref
    pub fn commit(
        self,
        author: &Signature,
        committer: &Signature,
        message: &str,
    ) -> Result<Commit<'repo>, git2::Error> {
        let oid = self.tree_builder.write()?;
        let tree = self.repo.find_tree(oid)?;
        let oid = self.repo.commit(
            None,
            author,
            committer,
            message,
            &tree,
            self.parent.as_ref().map(|c| vec![c]).unwrap_or_default().as_slice(),
        )?;
        self.repo.find_commit(oid)
    }
}

/// recursively build tree nodes and add the blob
/// Path should be relative
/// The key filemodes are 0o100644 for a file, 0o100755 for an executable, 0o040000 for a tree and 0o120000 or 0o160000?
fn write_to_path_in_tree(
    repo: &Repository,
    tree_builder: &mut TreeBuilder,
    path: &str,
    oid: Oid,
    filemode: i32,
) -> Result<()> {
    let mut it = path.splitn(2, '/');
    let base = it.next().context("write_to_path_in_tree called with empty path")?;
    if let Some(rest) = it.next() {
        // make a tree node
        let child_tree = if let Some(child_entry) = tree_builder.get(base)? {
            let child_tree = child_entry.to_object(repo)?.into_tree();
            // handle the case where the tree that we want is a blob, we'll just add a symbol to the end of the name, we use "|" when a tree's name was blocked by a blob and "-" when a blobs name was blocked by a tree
            match child_tree {
                Ok(child_tree) => Some(child_tree),
                Err(_) => {
                    println!("Malformed a tree name to avoid collsion with a blob {}", path);
                    if let Some(malformed_entry) = tree_builder.get(format!("{}|", base))? {
                        Some(
                            malformed_entry
                                .to_object(repo)?
                                .into_tree()
                                .map_err(|_| format_err!("file blocking tree creation x 2 as {}", path))?,
                        )
                    } else {
                        None
                    }
                }
            }
        } else {
            None
        };
        let mut child_tree_builder = repo.treebuilder(child_tree.as_ref())?;
        write_to_path_in_tree(repo, &mut child_tree_builder, rest, oid, filemode)?;
        let oid = child_tree_builder.write()?;
        tree_builder.insert(base, oid, 0o040000)?;
    } else {
        tree_builder.insert(base, oid, filemode)?;
    }
    Ok(())
}
