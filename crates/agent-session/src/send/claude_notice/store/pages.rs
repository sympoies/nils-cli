//! Append-only authenticated search pages for the notice checkpoint.
//! The private root authenticates each traversed edge, including negative lookups.
use super::*;
use std::collections::BTreeMap;
use std::os::unix::fs::FileExt;

const PAGE_BYTES: usize = 16 * 1024;
const FANOUT: usize = 48;
const MAX_DEPTH: usize = 32;
type Merge = fn(&str, &str, &str) -> io::Result<String>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Page {
    offset: u64,
    bytes: usize,
    sha256: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Child {
    last: String,
    page: Page,
}

#[derive(Clone, Deserialize, Serialize)]
enum Node {
    Leaf(Vec<(String, String)>),
    Branch(Vec<Child>),
}

impl Node {
    fn last(&self) -> Option<&str> {
        match self {
            Self::Leaf(rows) => rows.last().map(|r| r.0.as_str()),
            Self::Branch(children) => children.last().map(|c| c.last.as_str()),
        }
    }

    fn valid(&self) -> bool {
        let keys: Vec<&str> = match self {
            Self::Leaf(rows) => rows.iter().map(|row| row.0.as_str()).collect(),
            Self::Branch(children) => children.iter().map(|c| c.last.as_str()).collect(),
        };
        !keys.is_empty()
            && keys.len() <= FANOUT
            && keys.iter().all(|key| !key.is_empty() && key.len() <= 128)
            && keys.windows(2).all(|pair| pair[0] < pair[1])
    }
}

pub(super) struct Index {
    file: File,
    pub(super) root: Option<Page>,
    pub(super) committed: u64,
}

impl Index {
    pub(super) fn open(
        parent: &Path,
        name: &str,
        root: Option<Page>,
        committed: u64,
        write: bool,
    ) -> io::Result<Self> {
        let path = parent.join(name);
        let before = fs::symlink_metadata(&path)?;
        if !before.is_file() {
            return Err(invalid());
        }
        let file = OpenOptions::new()
            .read(true)
            .append(write)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        let meta = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o777 != 0o600
            || meta.len() < committed
            || (meta.dev(), meta.ino()) != (before.dev(), before.ino())
        {
            return Err(invalid());
        }
        Ok(Self {
            file,
            root,
            committed,
        })
    }

    pub(super) fn create(parent: &Path, name: &str) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(parent.join(name))?;
        Ok(Self {
            file,
            root: None,
            committed: 0,
        })
    }

    fn read(&self, child: &Child, depth: usize, deadline: Instant) -> io::Result<Node> {
        lifecycle::check_time(deadline)?;
        let page = &child.page;
        if depth >= MAX_DEPTH
            || page.bytes == 0
            || page.bytes > PAGE_BYTES
            || page
                .offset
                .checked_add(page.bytes as u64)
                .is_none_or(|end| end > self.committed)
            || !hash_valid(&page.sha256)
        {
            return Err(invalid());
        }
        let mut bytes = vec![0; page.bytes];
        self.file.read_exact_at(&mut bytes, page.offset)?;
        if sha256(&bytes) != page.sha256 {
            return Err(invalid());
        }
        let node: Node = serde_json::from_slice(&bytes)?;
        if !node.valid() || (!child.last.is_empty() && node.last() != Some(child.last.as_str())) {
            return Err(invalid());
        }
        Ok(node)
    }

    fn root_child(&self) -> Option<Child> {
        self.root.clone().map(|page| Child {
            last: String::new(),
            page,
        })
    }

    pub(super) fn get(&self, key: &str, deadline: Instant) -> io::Result<Option<String>> {
        let Some(mut child) = self.root_child() else {
            return Ok(None);
        };
        for depth in 0..MAX_DEPTH {
            match self.read(&child, depth, deadline)? {
                Node::Leaf(rows) => {
                    return Ok(rows
                        .binary_search_by(|r| r.0.as_str().cmp(key))
                        .ok()
                        .map(|i| rows[i].1.clone()));
                }
                Node::Branch(children) => {
                    let position = children.partition_point(|c| c.last.as_str() < key);
                    let Some(next) = children.get(position) else {
                        return Ok(None);
                    };
                    child = next.clone();
                }
            }
        }
        Err(invalid())
    }

    fn write(&mut self, node: Node, deadline: Instant) -> io::Result<Child> {
        lifecycle::check_time(deadline)?;
        let last = node.last().ok_or_else(invalid)?.to_string();
        let bytes = serde_json::to_vec(&node)?;
        if bytes.len() > PAGE_BYTES || !node.valid() {
            return Err(invalid());
        }
        let offset = self.file.metadata()?.len();
        self.file.write_all(&bytes)?;
        self.committed = offset + bytes.len() as u64;
        Ok(Child {
            last,
            page: Page {
                offset,
                bytes: bytes.len(),
                sha256: sha256(&bytes),
            },
        })
    }

    fn split_write(&mut self, node: Node, deadline: Instant) -> io::Result<Vec<Child>> {
        if node.valid() && serde_json::to_vec(&node)?.len() <= PAGE_BYTES {
            return Ok(vec![self.write(node, deadline)?]);
        }
        let (left, right) = match node {
            Node::Leaf(mut rows) if rows.len() > 1 => {
                let right = rows.split_off(rows.len() / 2);
                (Node::Leaf(rows), Node::Leaf(right))
            }
            Node::Branch(mut children) if children.len() > 1 => {
                let right = children.split_off(children.len() / 2);
                (Node::Branch(children), Node::Branch(right))
            }
            _ => return Err(invalid()),
        };
        let mut result = self.split_write(left, deadline)?;
        result.extend(self.split_write(right, deadline)?);
        Ok(result)
    }

    fn apply_page(
        &mut self,
        child: Child,
        additions: Vec<(String, String)>,
        depth: usize,
        deadline: Instant,
        merge: Merge,
    ) -> io::Result<Vec<Child>> {
        let node = match self.read(&child, depth, deadline)? {
            Node::Leaf(rows) => {
                let before = rows.clone();
                let mut rows: BTreeMap<String, String> = rows.into_iter().collect();
                for (key, value) in additions {
                    let value = match rows.get(&key) {
                        Some(old) => merge(&key, old, &value)?,
                        None => value,
                    };
                    rows.insert(key, value);
                }
                let rows: Vec<_> = rows.into_iter().collect();
                if rows == before {
                    return Ok(vec![Child {
                        last: rows.last().unwrap().0.clone(),
                        page: child.page,
                    }]);
                }
                Node::Leaf(rows)
            }
            Node::Branch(children) => {
                let mut additions = additions.into_iter().peekable();
                let mut updated = Vec::new();
                let last = children.len() - 1;
                for (position, child) in children.into_iter().enumerate() {
                    let mut group = Vec::new();
                    while additions
                        .peek()
                        .is_some_and(|(key, _)| position == last || key <= &child.last)
                    {
                        group.push(additions.next().unwrap());
                    }
                    if group.is_empty() {
                        updated.push(child);
                    } else {
                        updated.extend(self.apply_page(
                            child,
                            group,
                            depth + 1,
                            deadline,
                            merge,
                        )?);
                    }
                }
                Node::Branch(updated)
            }
        };
        self.split_write(node, deadline)
    }

    pub(super) fn apply(
        &mut self,
        additions: BTreeMap<String, String>,
        deadline: Instant,
        merge: Merge,
    ) -> io::Result<()> {
        if additions.is_empty() {
            return Ok(());
        }
        let mut children = match self.root_child() {
            Some(root) => {
                self.apply_page(root, additions.into_iter().collect(), 0, deadline, merge)?
            }
            None => return self.build(additions, deadline),
        };
        while children.len() > FANOUT {
            let mut parents = Vec::new();
            for group in children.chunks(FANOUT) {
                parents.push(self.write(Node::Branch(group.to_vec()), deadline)?);
            }
            children = parents;
        }
        self.root = Some(if children.len() == 1 {
            children[0].page.clone()
        } else {
            self.write(Node::Branch(children), deadline)?.page
        });
        Ok(())
    }

    /// Bulk construction avoids a copy-on-write search per imported record.
    pub(super) fn build(
        &mut self,
        entries: BTreeMap<String, String>,
        deadline: Instant,
    ) -> io::Result<()> {
        let mut leaves = Vec::new();
        let mut rows = Vec::new();
        let mut bytes = 0;
        for row in entries {
            let size = serde_json::to_vec(&row)?.len();
            if !rows.is_empty() && (rows.len() == FANOUT || bytes + size > PAGE_BYTES - 128) {
                leaves.push(self.write(Node::Leaf(std::mem::take(&mut rows)), deadline)?);
                bytes = 0;
            }
            bytes += size + 1;
            rows.push(row);
        }
        if !rows.is_empty() {
            leaves.push(self.write(Node::Leaf(rows), deadline)?);
        }
        while leaves.len() > 1 {
            let mut parents = Vec::new();
            for group in leaves.chunks(FANOUT) {
                parents.push(self.write(Node::Branch(group.to_vec()), deadline)?);
            }
            leaves = parents;
        }
        self.root = leaves.pop().map(|child| child.page);
        self.file.sync_all()
    }

    pub(super) fn file(&self) -> &File {
        &self.file
    }

    pub(super) fn all(&self, deadline: Instant) -> io::Result<BTreeMap<String, String>> {
        let mut entries = BTreeMap::new();
        let mut pending: Vec<(Child, usize)> =
            self.root_child().into_iter().map(|c| (c, 0)).collect();
        while let Some((child, depth)) = pending.pop() {
            match self.read(&child, depth, deadline)? {
                Node::Leaf(rows) => {
                    for (key, value) in rows {
                        if entries.insert(key, value).is_some() {
                            return Err(invalid());
                        }
                    }
                }
                Node::Branch(children) => {
                    pending.extend(children.into_iter().map(|c| (c, depth + 1)))
                }
            }
        }
        Ok(entries)
    }
}
