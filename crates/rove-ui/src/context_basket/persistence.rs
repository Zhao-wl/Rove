use super::{
    model::{Availability, Basket, Entry, LineRange},
    tracking::Anchor,
};
use crate::shell::layout::workspace_key;
use rove_base::WorkspacePath;
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const SCHEMA: u64 = 1;
pub const MAX_STATE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct BasketStore {
    path: Option<PathBuf>,
}

impl BasketStore {
    pub fn for_workspace(workspace: &Path) -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(|directory| PathBuf::from(directory).join("Rove").join("state"));
        Self::with_base(base, workspace)
    }

    fn with_base(base: Option<PathBuf>, workspace: &Path) -> Self {
        Self {
            path: base
                .map(|directory| directory.join(workspace_key(workspace)).join("basket.json")),
        }
    }

    pub fn load(&self) -> Basket {
        self.read().unwrap_or_default()
    }

    fn read(&self) -> Option<Basket> {
        let file = File::open(self.path.as_ref()?).ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_STATE_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return None;
        }
        let json = serde_json::from_slice(&bytes).ok()?;
        from_json(&json)
    }

    pub fn save(&self, basket: &Basket) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(&to_json(basket))?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "basket state exceeds size limit",
            ));
        }
        let directory = path
            .parent()
            .expect("basket state lives below a workspace directory");
        std::fs::create_dir_all(directory)?;
        let mut pending = tempfile::NamedTempFile::new_in(directory)?;
        pending.write_all(&bytes)?;
        pending.as_file().sync_all()?;
        pending.persist(path).map_err(|error| error.error)?;
        Ok(())
    }
}

pub fn to_json(basket: &Basket) -> Value {
    let entries: Vec<_> = basket
        .items()
        .iter()
        .map(|entry| {
            let status = match entry.availability {
                Availability::Current => "current",
                Availability::Stale => "stale",
                Availability::Missing => "missing",
            };
            let anchor = entry.anchor.map(|anchor| {
                json!({
                    "fileLines": anchor.file_lines,
                    "selected": anchor.selected,
                    "first": anchor.first,
                    "last": anchor.last,
                    "previous": anchor.previous,
                    "following": anchor.following,
                })
            });
            json!({
                "id": entry.id(),
                "path": entry.path.as_str(),
                "first": entry.lines.first(),
                "last": entry.lines.last(),
                "symbol": entry.symbol,
                "note": entry.note,
                "status": status,
                "anchor": anchor,
            })
        })
        .collect();
    json!({ "version": SCHEMA, "introduction": basket.introduction, "entries": entries })
}

pub fn from_json(value: &Value) -> Option<Basket> {
    if value.get("version")?.as_u64()? != SCHEMA {
        return None;
    }
    let introduction = value.get("introduction")?.as_str()?.to_owned();
    let entries = value
        .get("entries")?
        .as_array()?
        .iter()
        .map(parse_entry)
        .collect::<Option<Vec<_>>>()?;
    Basket::restore(introduction, entries)
}

fn parse_entry(value: &Value) -> Option<Entry> {
    let path = WorkspacePath::new(value.get("path")?.as_str()?).ok()?;
    let lines = LineRange::new(value.get("first")?.as_u64()?, value.get("last")?.as_u64()?)?;
    let symbol = match value.get("symbol")? {
        Value::Null => None,
        value => Some(value.as_str()?.to_owned()),
    };
    let availability = match value.get("status")?.as_str()? {
        "current" => Availability::Current,
        "stale" => Availability::Stale,
        "missing" => Availability::Missing,
        _ => return None,
    };
    let anchor = match value.get("anchor")? {
        Value::Null => None,
        value => Some(parse_anchor(value)?),
    };
    Some(Entry {
        id: value.get("id")?.as_u64()?,
        path,
        lines,
        symbol,
        note: value.get("note")?.as_str()?.to_owned(),
        availability,
        anchor,
    })
}

fn optional_hash(value: &Value) -> Option<Option<u64>> {
    match value {
        Value::Null => Some(None),
        value => Some(Some(value.as_u64()?)),
    }
}

fn parse_anchor(value: &Value) -> Option<Anchor> {
    Some(Anchor {
        file_lines: usize::try_from(value.get("fileLines")?.as_u64()?).ok()?,
        selected: value.get("selected")?.as_u64()?,
        first: value.get("first")?.as_u64()?,
        last: value.get("last")?.as_u64()?,
        previous: optional_hash(value.get("previous")?)?,
        following: optional_hash(value.get("following")?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomically_replaces_workspace_state_and_discard_corrupt_files() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let base = temp.path().join("state");
        let store = BasketStore::with_base(Some(base.clone()), &project);

        let mut basket = Basket::default();
        basket.introduction = "initial questions".into();
        store.save(&basket).unwrap();
        basket.introduction = "updated questions".into();
        store.save(&basket).unwrap();

        let reopened = BasketStore::with_base(Some(base.clone()), &project);
        assert_eq!(reopened.load().introduction, "updated questions");
        assert!(project.read_dir().unwrap().next().is_none());
        let other = BasketStore::with_base(Some(base), &temp.path().join("other"));
        assert!(other.load().introduction.is_empty());

        let path = store.path.as_ref().unwrap();
        assert_eq!(path.parent().unwrap().read_dir().unwrap().count(), 1);
        std::fs::write(path, b"interrupted write").unwrap();
        assert!(reopened.load().introduction.is_empty());
    }

    #[test]
    fn round_trips_positions_and_fingerprints_without_persisting_code() {
        let path = WorkspacePath::new("Assets/a.cs").unwrap();
        let mut basket = Basket::default();
        basket.introduction = "question".to_owned();
        let id = basket
            .insert(Entry::new(
                path.clone(),
                LineRange::new(2, 2).unwrap(),
                None,
            ))
            .unwrap();
        basket.edit_note(id, "first note".into());
        assert!(basket.bind_anchor(id, &["before", "do not persist code", "after"]));
        let json = to_json(&basket);
        assert!(!json.to_string().contains("do not persist code"));
        let mut loaded = from_json(&json).unwrap();
        assert_eq!(loaded.items(), basket.items());
        assert_eq!(loaded.introduction, basket.introduction);
        loaded.settle_change(
            &path,
            &["before", "new line", "do not persist code", "after"],
        );
        assert_eq!(loaded.items()[0].lines, LineRange::new(3, 3).unwrap());
        assert!(
            loaded
                .insert(Entry::new(path, LineRange::new(8, 8).unwrap(), None))
                .unwrap()
                > id
        );
    }

    #[test]
    fn rejects_duplicates_and_corrupt_or_unknown_record_shapes() {
        let mut basket = Basket::default();
        basket
            .insert(Entry::new(
                WorkspacePath::new("a.txt").unwrap(),
                LineRange::new(1, 2).unwrap(),
                None,
            ))
            .unwrap();
        let initial = to_json(&basket);
        let mut data = initial.clone();
        data["version"] = json!(2);
        assert!(from_json(&data).is_none());
        let mut data = initial.clone();
        data["entries"][0]["path"] = json!("../a.txt");
        assert!(from_json(&data).is_none());
        let mut data = initial.clone();
        data["entries"]
            .as_array_mut()
            .unwrap()
            .push(initial["entries"][0].clone());
        assert!(from_json(&data).is_none());
    }
}
