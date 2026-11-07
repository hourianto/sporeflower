use crate::index::{BlockId, IdVec};

/// Cooper-Harvey-Kennedy dominators use linear storage and converge quickly on
/// the reducible graphs produced by legacy Java compilers. Tree intervals make
/// subsequent dominance queries constant time.
pub(crate) struct Dominators {
    pub idom: IdVec<BlockId, Option<BlockId>>,
    pub preorder: IdVec<BlockId, usize>,
    pub end: IdVec<BlockId, usize>,
}

impl Dominators {
    pub fn new(
        successors: &IdVec<BlockId, Vec<BlockId>>,
        predecessors: &IdVec<BlockId, Vec<BlockId>>,
        root: BlockId,
    ) -> Self {
        let n = successors.len();
        let mut visited: IdVec<BlockId, _> = vec![false; n].into();
        let mut postorder = Vec::new();
        let mut stack = vec![(root, 0)];
        visited[root] = true;
        while let Some((node, next)) = stack.last_mut() {
            if *next == successors[*node].len() {
                postorder.push(*node);
                stack.pop();
            } else {
                let child = successors[*node][*next];
                *next += 1;
                if !std::mem::replace(&mut visited[child], true) {
                    stack.push((child, 0));
                }
            }
        }
        postorder.reverse();
        let mut order: IdVec<BlockId, _> = vec![usize::MAX; n].into();
        for (index, &node) in postorder.iter().enumerate() {
            order[node] = index;
        }
        let mut idom: IdVec<BlockId, _> = vec![None; n].into();
        idom[root] = Some(root);
        let mut changed = true;
        while changed {
            changed = false;
            for &node in postorder.iter().skip(1) {
                let mut incoming = predecessors[node]
                    .iter()
                    .copied()
                    .filter(|&pred| idom[pred].is_some());
                let Some(mut parent) = incoming.next() else {
                    continue;
                };
                for mut other in incoming {
                    while parent != other {
                        while order[parent] > order[other] {
                            parent = idom[parent].unwrap();
                        }
                        while order[other] > order[parent] {
                            other = idom[other].unwrap();
                        }
                    }
                }
                if idom[node] != Some(parent) {
                    idom[node] = Some(parent);
                    changed = true;
                }
            }
        }
        let mut children: IdVec<BlockId, _> = vec![Vec::new(); n].into();
        for (node, &parent) in idom.iter_enumerated() {
            if let Some(parent) = parent
                && node != parent
            {
                children[parent].push(node);
            }
        }
        let mut preorder: IdVec<BlockId, _> = vec![usize::MAX; n].into();
        let mut end: IdVec<BlockId, _> = vec![0; n].into();
        let mut clock = 0;
        let mut stack = vec![(root, false)];
        while let Some((node, exiting)) = stack.pop() {
            if exiting {
                end[node] = clock;
            } else {
                preorder[node] = clock;
                clock += 1;
                stack.push((node, true));
                stack.extend(children[node].iter().rev().map(|&child| (child, false)));
            }
        }
        Self {
            idom,
            preorder,
            end,
        }
    }

    pub fn dominates(&self, dominator: BlockId, node: BlockId) -> bool {
        (self.preorder[dominator]..self.end[dominator]).contains(&self.preorder[node])
    }

    pub fn common(&self, entries: impl IntoIterator<Item = BlockId>) -> Option<BlockId> {
        let mut entries = entries.into_iter();
        let mut common = entries.next()?;
        self.idom[common]?;
        for entry in entries {
            self.idom[entry]?;
            while !self.dominates(common, entry) {
                common = self.idom[common]?;
            }
        }
        Some(common)
    }
}
