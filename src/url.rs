use core::fmt;
use std::{
    fs,
    io,
    path::{Path, PathBuf},
    str::FromStr,
    vec,
};

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct Url {
    url: url::Url,
}

impl Url {
    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    pub(crate) fn to_path(&self, base: impl AsRef<Path>) -> PathBuf {
        let path = self.url.path().strip_prefix('/').unwrap_or_else(|| self.url.path());
        base.as_ref().join(self.url.host_str().unwrap_or("local")).join(path)
    }

    pub(crate) fn pop_path_segment(&mut self) {
        self.url.path_segments_mut().unwrap().pop();
    }

    pub(crate) fn push_path_segment(&mut self, segment: &str) {
        self.url.path_segments_mut().unwrap().push(segment);
    }
}

impl fmt::Debug for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Url").field(&self.url).finish()
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.url.fmt(f)
    }
}

impl From<url::Url> for Url {
    fn from(url: url::Url) -> Self {
        assert!(url.path_segments().is_some());
        assert!(url.fragment().is_none());
        Url { url }
    }
}

impl FromStr for Url {
    type Err = <url::Url as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(|url| Url { url })
    }
}

pub struct UrlRepo {
    repo_key: &'static str,
    base: PathBuf,
}

impl UrlRepo {
    pub fn new(repo_key: &'static str, base: impl AsRef<Path>) -> io::Result<Self> {
        let base = base.as_ref().to_path_buf();
        fs::create_dir_all(&base)?;
        Ok(Self { repo_key, base })
    }

    fn base(&self) -> &Path {
        &self.base
    }

    pub fn node_path(&self, url: &Url) -> PathBuf {
        url.to_path(&self.base)
    }

    pub fn leaf_path(&self, url: &Url, name: &str) -> PathBuf {
        self.node_path(url).join(format!("<{}>{}", self.repo_key, name))
    }

    pub fn read_dir_sorted_for_url(&self, url: &Url) -> io::Result<vec::IntoIter<fs::DirEntry>> {
        let mut dir = fs::read_dir(url.to_path(self.base()))?.collect::<io::Result<Vec<_>>>()?;
        dir.sort_by_key(fs::DirEntry::file_name);
        Ok(dir.into_iter())
    }

    pub fn read_leaves_for_url(
        &self,
        url: &Url,
    ) -> io::Result<impl Iterator<Item = io::Result<(String, fs::DirEntry)>>> {
        let my_repo_key = self.repo_key;
        Ok(fs::read_dir(url.to_path(self.base()))?.filter_map(move |de| match de {
            Ok(de) => {
                if let Some((repo_key, name)) = de.kind().as_leaf() {
                    if repo_key == my_repo_key {
                        return Some(Ok((name.to_owned(), de)));
                    }
                }
                None
            }
            Err(err) => Some(Err(err)),
        }))
    }

    pub fn read_leaves_sorted_for_url(
        &self,
        url: &Url,
    ) -> io::Result<impl DoubleEndedIterator<Item = (String, fs::DirEntry)>> {
        let mut leaves = self.read_leaves_for_url(url)?.collect::<io::Result<Vec<_>>>()?;
        leaves.sort_by(|(name1, _), (name2, _)| name1.cmp(name2));
        Ok(leaves.into_iter())
    }

    pub fn list_all<Leaf>(
        &self,
        base_url: Url,
        make_leaf: fn(Url, &str, &fs::DirEntry) -> Leaf,
    ) -> Result<IterUrlRepoLeaves<Leaf>, io::Error> {
        Ok(IterUrlRepoLeaves {
            repo: self,
            stack: vec![self.read_dir_sorted_for_url(&base_url)?],
            url: base_url,
            make_leaf,
        })
    }
}

trait DirEntryUrlRepoExt {
    fn kind(&self) -> DirEntryKind;
}

impl DirEntryUrlRepoExt for fs::DirEntry {
    fn kind(&self) -> DirEntryKind {
        let os_file_name = self.file_name();
        if let Some(file_name) = os_file_name.to_str() {
            if file_name.starts_with('<') {
                if let Some(split) = file_name.find('>') {
                    return DirEntryKind::Leaf(os_file_name, split);
                }
            }
            if let Ok(file_type) = self.file_type() {
                if file_type.is_dir() {
                    return DirEntryKind::Node(os_file_name);
                }
            }
        }
        DirEntryKind::Unknown
    }
}

enum DirEntryKind {
    Node(std::ffi::OsString),
    Leaf(std::ffi::OsString, usize),
    Unknown,
}

impl DirEntryKind {
    fn as_leaf(&self) -> Option<(&str, &str)> {
        match self {
            Self::Leaf(s, split) => {
                let s = s.to_str().unwrap();
                Some((&s[1..*split], &s[*split + 1..]))
            }
            _ => None,
        }
    }

    fn as_node(&self) -> Option<&str> {
        match self {
            Self::Node(name) => Some(name.to_str().unwrap()),
            _ => None,
        }
    }
}

// iterator over all docs in the repo
pub struct IterUrlRepoLeaves<'r, Leaf> {
    repo: &'r UrlRepo,
    url: Url,
    stack: Vec<vec::IntoIter<fs::DirEntry>>, // was using readdir, but we need it in order
    make_leaf: fn(Url, &str, &fs::DirEntry) -> Leaf,
}

impl<'r, Leaf> Iterator for IterUrlRepoLeaves<'r, Leaf> {
    type Item = io::Result<Leaf>;

    fn next(&mut self) -> Option<Self::Item> {
        // ascend the tree if at the end of branches and get the next `DirEntry`
        let mut next_dir_entry = loop {
            if let Some(iter) = self.stack.last_mut() {
                match iter.next() {
                    Some(entry) => break entry,
                    None => {
                        self.stack.pop().unwrap();
                        self.url.pop_path_segment();
                    }
                }
            } else {
                return None;
            }
        };

        // descend to the next doc
        loop {
            let kind = next_dir_entry.kind();
            if let Some(name) = kind.as_node() {
                self.url.push_path_segment(name);
                let dir = self.repo.read_dir_sorted_for_url(&self.url);
                let mut dir = match dir {
                    Ok(dir) => dir,
                    Err(err) => break Some(Err(err)),
                };
                next_dir_entry = dir.next().expect("todo: handle empty dir");
                self.stack.push(dir);
            } else if let Some((repo_key, name)) = kind.as_leaf() {
                if repo_key == self.repo.repo_key {
                    let url = self.url.clone();
                    break Some(Ok((self.make_leaf)(url, name, &next_dir_entry)));
                } else {
                    println!("Ignored file : {:?}", next_dir_entry.file_name());
                }
            } else {
                println!("Ignored file : {:?}", next_dir_entry.file_name());
            }
        }
    }
}
