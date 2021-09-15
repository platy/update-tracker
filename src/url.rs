use core::fmt;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
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
