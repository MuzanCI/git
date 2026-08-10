use auth_git2::GitAuthenticator;
use git2::Commit;
use git2::FetchOptions;
use git2::Oid;
use git2::RemoteCallbacks;
use git2::Repository;
use git2::build::CheckoutBuilder;
use git2::build::RepoBuilder;
use std::path::Path;
use url::Url;

/// A git branch name.
pub type GitBranch = String;

/// A git commit SHA.
pub type GitCommitSha = String;

/// A git tree SHA.
pub type GitTreeSha = String;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct GitClientError(pub String);

pub struct GitClient {
    git_config: git2::Config,
    git_auth: GitAuthenticator,
}

impl GitClient {
    pub fn new(git_config: git2::Config, git_auth: GitAuthenticator) -> Self {
        Self {
            git_config,
            git_auth,
        }
    }

    pub fn try_default() -> Result<Self, GitClientError> {
        let git_config = git2::Config::open_default().map_err(|e| GitClientError(e.to_string()))?;
        let git_auth = GitAuthenticator::default();
        Ok(Self {
            git_config,
            git_auth,
        })
    }

    pub fn checkout_commit(
        &self,
        url: &Url,
        branch: &GitBranch,
        target_dir: &Path,
        commit_sha: &GitCommitSha,
    ) -> Result<(), GitClientError> {
        let repo = self.shallow_clone(url, branch, target_dir)?;
        let commit = self.find_commit(&repo, branch, commit_sha)?;

        let mut checkout_builder = CheckoutBuilder::new();
        checkout_builder.force();
        repo.checkout_tree(commit.as_object(), Some(&mut checkout_builder))
            .map_err(|e| GitClientError(e.to_string()))?;

        repo.set_head_detached(commit.id())
            .map_err(|e| GitClientError(e.to_string()))?;

        Ok(())
    }

    fn shallow_clone(
        &self,
        url: &Url,
        branch: &GitBranch,
        target_dir: &Path,
    ) -> Result<Repository, GitClientError> {
        let fetch_opts = {
            let mut cbs = RemoteCallbacks::new();
            cbs.credentials(self.git_auth.credentials(&self.git_config));

            let mut fetch_opts = FetchOptions::new();
            fetch_opts.remote_callbacks(cbs);
            fetch_opts.depth(1);
            fetch_opts
        };

        let repo = {
            let mut repo_builder = RepoBuilder::new();
            repo_builder.fetch_options(fetch_opts).branch(branch);

            eprintln!("Shallow clone of branch [{}] from [{}]...", branch, url);

            repo_builder
                .clone(url.as_str(), target_dir)
                .map_err(|e| GitClientError(e.to_string()))?
        };

        eprintln!("Shallow clone success!");
        Ok(repo)
    }

    fn find_commit<'a>(
        &self,
        repo: &'a Repository,
        branch: &GitBranch,
        commit_sha: &GitCommitSha,
    ) -> Result<Commit<'a>, GitClientError> {
        let target_oid = Oid::from_str(commit_sha).map_err(|e| GitClientError(e.to_string()))?;
        match repo.find_commit(target_oid) {
            Ok(commit) => return Ok(commit),
            Err(_) => {
                eprintln!(
                    "Local repository does not contain commit [{}] in branch [{}].",
                    commit_sha, branch
                );
            }
        };

        eprintln!(
            "Fetching commit object [{}] directly from remote for branch [{}]...",
            commit_sha, branch
        );
        match self.fetch_commit_directly(repo, commit_sha) {
            Ok(commit) => return Ok(commit),
            Err(e) => {
                eprintln!(
                    "Failed to fetch commit [{}] directly from remote for branch [{}]: {:#}",
                    commit_sha, branch, e
                );
            }
        };

        match self.fetch_commit_iteratively(repo, branch, commit_sha) {
            Ok(commit) => return Ok(commit),
            Err(e) => {
                eprintln!(
                    "Failed to fetch commit [{}] iteratively from remote for branch [{}]: {:#}",
                    commit_sha, branch, e
                );
            }
        };

        Err(GitClientError(format!(
            "Failed to find commit [{}] in branch [{}].",
            commit_sha, branch,
        )))
    }

    fn fetch_commit_directly<'a>(
        &self,
        repo: &'a Repository,
        commit_sha: &GitCommitSha,
    ) -> Result<Commit<'a>, GitClientError> {
        let mut fetch_opts = {
            let mut cbs = RemoteCallbacks::new();
            cbs.credentials(self.git_auth.credentials(&self.git_config));

            let mut fetch_opts = FetchOptions::new();
            fetch_opts.remote_callbacks(cbs);
            fetch_opts.depth(1);
            fetch_opts
        };

        let mut remote = repo
            .find_remote("origin")
            .map_err(|e| GitClientError(e.to_string()))?;
        remote
            .fetch(&[commit_sha], Some(&mut fetch_opts), None)
            .map_err(|e| GitClientError(e.to_string()))?;

        let target_oid = Oid::from_str(commit_sha).map_err(|e| GitClientError(e.to_string()))?;
        let commit_object = repo
            .find_commit(target_oid)
            .map_err(|e| GitClientError(e.to_string()))?;
        Ok(commit_object)
    }

    fn fetch_commit_iteratively<'a>(
        &self,
        repo: &'a Repository,
        branch: &GitBranch,
        commit_sha: &GitCommitSha,
    ) -> Result<Commit<'a>, GitClientError> {
        for i in 4..=10 {
            let depth = 2i32.pow(i);
            let mut fetch_opts = {
                let mut cbs = RemoteCallbacks::new();
                cbs.credentials(self.git_auth.credentials(&self.git_config));

                let mut fetch_opts = FetchOptions::new();
                fetch_opts.remote_callbacks(cbs);
                fetch_opts.depth(depth);
                fetch_opts
            };
            let mut remote = repo
                .find_remote("origin")
                .map_err(|e| GitClientError(e.to_string()))?;
            if let Err(e) = remote.fetch(&[commit_sha], Some(&mut fetch_opts), None) {
                eprintln!(
                    "Failed to fetch commit [{}] with depth [{}]: {:#}",
                    commit_sha, depth, e
                );
            } else if let Ok(target_oid) = Oid::from_str(commit_sha) {
                if let Ok(commit_object) = repo.find_commit(target_oid) {
                    return Ok(commit_object);
                }
            }
        }

        Err(GitClientError(format!(
            "Failed to find commit [{}] in branch [{}] after iterative fetching.",
            commit_sha, branch
        )))
    }
}
