//! Arena-allocated directory tree. Nodes are plain indices, so the whole tree
//! is one `Vec<Node>` and can be shared across threads cheaply.

pub type NodeId = u32;

#[derive(Debug)]
pub struct Node {
    pub name: String,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    /// Bytes attributed to this node itself (file bytes, or the directory inode).
    pub own: u64,
    /// Subtree total: `own` plus everything below.
    pub size: u64,
    /// Number of files in the subtree (directories not counted).
    pub files: u64,
    pub is_dir: bool,
    pub mtime: i64,
}

#[derive(Debug)]
pub struct Arena {
    pub nodes: Vec<Node>,
    pub root: NodeId,
}

impl Arena {
    pub fn new_root(name: String) -> Self {
        let root = Node {
            name,
            parent: None,
            children: Vec::new(),
            own: 0,
            size: 0,
            files: 0,
            is_dir: true,
            mtime: 0,
        };
        Self {
            nodes: vec![root],
            root: 0,
        }
    }

    /// Root's own inode size (added once, when the walk yields depth 0).
    pub fn add_root_own(&mut self, own: u64) {
        let r = self.root as usize;
        self.nodes[r].own += own;
        self.nodes[r].size += own;
    }

    /// Attach a child and propagate its size/file count through all ancestors.
    pub fn add(
        &mut self,
        parent: NodeId,
        name: String,
        is_dir: bool,
        own: u64,
        mtime: i64,
    ) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            name,
            parent: Some(parent),
            children: Vec::new(),
            own,
            size: own,
            files: u64::from(!is_dir),
            is_dir,
            mtime,
        });
        self.nodes[parent as usize].children.push(id);
        let files = u64::from(!is_dir);
        let mut cur = Some(parent);
        while let Some(c) = cur {
            let n = &mut self.nodes[c as usize];
            n.size += own;
            n.files += files;
            cur = n.parent;
        }
        id
    }

    pub fn path_string(&self, id: NodeId) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let n = &self.nodes[c as usize];
            parts.push(n.name.as_str());
            cur = n.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    pub fn basename(&self, id: NodeId) -> &str {
        &self.nodes[id as usize].name
    }

    /// The `limit` biggest children plus an aggregate of the remainder.
    /// Returns `(children, Some((rest_size, rest_count, rest_files)))`.
    pub fn top_children(&self, id: NodeId, limit: usize) -> (Vec<NodeId>, Option<(u64, u64, u64)>) {
        let mut ids: Vec<NodeId> = self.nodes[id as usize]
            .children
            .iter()
            .copied()
            .filter(|c| self.nodes[*c as usize].size > 0)
            .collect();
        let by_size = |a: &NodeId, b: &NodeId| {
            self.nodes[*b as usize]
                .size
                .cmp(&self.nodes[*a as usize].size)
        };
        if limit == 0 || ids.len() <= limit {
            ids.sort_by(by_size);
            return (ids, None);
        }
        let (top, pivot, rest) = ids.select_nth_unstable_by(limit, by_size);
        top.sort_by(by_size);
        let rest_size: u64 = rest
            .iter()
            .chain(std::iter::once(&*pivot))
            .map(|c| self.nodes[*c as usize].size)
            .sum();
        let rest_files: u64 = rest
            .iter()
            .chain(std::iter::once(&*pivot))
            .map(|c| self.nodes[*c as usize].files)
            .sum();
        (
            top.to_vec(),
            Some((rest_size, rest.len() as u64 + 1, rest_files)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propagates_sizes() {
        let mut a = Arena::new_root("/x".into());
        let d = a.add(0, "d".into(), true, 10, 0);
        let f = a.add(d, "f".into(), false, 100, 0);
        a.add(d, "g".into(), false, 1, 0);
        assert_eq!(a.nodes[f as usize].size, 100);
        assert_eq!(a.nodes[d as usize].size, 111);
        assert_eq!(a.nodes[0].size, 111);
        assert_eq!(a.nodes[d as usize].files, 2);
        assert_eq!(a.nodes[0].files, 2);
        assert_eq!(a.path_string(f), "/x/d/f");
    }

    #[test]
    fn aggregates_small_children() {
        let mut a = Arena::new_root("/".into());
        for i in 0..10u64 {
            a.add(0, format!("f{i}"), false, (i + 1) * 10, 0);
        }
        let (top, rest) = a.top_children(0, 3);
        assert_eq!(top.len(), 3);
        assert_eq!(a.nodes[top[0] as usize].size, 100);
        let (rsize, rcount, rfiles) = rest.unwrap();
        assert_eq!(rcount, 7);
        assert_eq!(rfiles, 7);
        assert_eq!(rsize, 10 + 20 + 30 + 40 + 50 + 60 + 70);
    }
}