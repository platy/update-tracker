use super::*;
use std::{
    fs::{self},
    io::{self, BufRead, BufReader, Write},
    iter,
    path::{Path, PathBuf},
    str::FromStr,
};

pub struct TagRepo {
    base: PathBuf,
}

impl TagRepo {
    pub fn new(base: impl AsRef<Path>) -> io::Result<Self> {
        let base = base.as_ref().to_path_buf();
        fs::create_dir_all(&base)?;
        Ok(Self { base })
    }

    pub fn tag_update(
        &self,
        tag_name: String,
        update_ref: UpdateRef,
    ) -> io::Result<(Tag, impl Iterator<Item = TagEvent>)> {
        let tag = Tag { name: tag_name };
        let path = self.path_for(&tag);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut is_new_tag = true;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .or_else(|err| {
                if err.kind() == io::ErrorKind::AlreadyExists {
                    is_new_tag = false;
                }
                fs::OpenOptions::new().append(true).open(&path)
            })?;
        file.write_all(format!("{}\n", update_ref).as_bytes())?;
        file.flush()?;

        let events = if is_new_tag {
            Some(TagEvent::TagCreated { tag: tag.clone() })
        } else {
            None
        }
        .into_iter()
        .chain(iter::once(TagEvent::UpdateTagged {
            tag: tag.clone(),
            update_ref,
        }));

        Ok((tag, events))
    }

    /// Lists all tags, sorted by name
    pub fn list_tags(&self) -> io::Result<impl Iterator<Item = Tag>> {
        let mut dir: Vec<fs::DirEntry> = fs::read_dir(&self.base)?.collect::<io::Result<_>>()?;
        dir.sort_by_key(fs::DirEntry::file_name);

        Ok(dir.into_iter().map(move |dir_entry| Tag {
            name: dir_entry.file_name().to_str().unwrap().to_string(),
        }))
    }

    /// Returns error if there is no tag
    pub fn list_updates_in_tag(
        &self,
        tag: &Tag,
    ) -> io::Result<impl Iterator<Item = Result<UpdateRef, <UpdateRef as FromStr>::Err>>> {
        let reader = BufReader::new(fs::File::open(&self.path_for(tag))?);
        Ok(reader.lines().map(|line| line.unwrap().parse()))
    }

    fn path_for(&self, tag: &Tag) -> PathBuf {
        self.base.join(&tag.name)
    }
}
