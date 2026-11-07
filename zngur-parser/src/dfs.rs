use std::collections::HashSet;

/// Error returned by [`DfsCycleDetector::with_node`] when visiting a node that
/// is already on the active recursion stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DfsCycle {
    /// Holds `Some(head)` the first time this cycle is encountered (where `head`
    /// is the smallest node index on the cycle, i.e., the earliest declaration),
    /// or `None` if an error has already been reported for this cycle.
    pub(crate) newly_detected_head: Option<usize>,
}

/// Outcome of inspecting an edge to `next_node` against the active DFS stack.
enum DfsEdge {
    /// `next_node` is not on the active recursion stack and can be visited safely.
    TreeOrCross,
    /// `next_node` is already on the active recursion stack, forming a directed cycle.
    BackEdgeCycle(DfsCycle),
}

/// Tracks the active recursion stack during a depth-first traversal over indexed
/// nodes (`0..n`) to detect directed cycles and report each unique cycle at most once.
#[derive(Debug, Default)]
pub(crate) struct DfsCycleDetector {
    /// Node indices currently on the active DFS call stack, in visit order.
    active_stack: Vec<usize>,
    /// Canonical cycle heads (minimum node index in each cycle) that have already
    /// been reported, preventing duplicate diagnostics when multiple paths reach
    /// the same cycle.
    reported_cycle_heads: HashSet<usize>,
}

impl DfsCycleDetector {
    /// Inspects the edge to `next_node` against `active_stack` to check for a cycle.
    fn classify_edge(&mut self, next_node: usize) -> DfsEdge {
        let Some(cycle_pos) = self.active_stack.iter().position(|&v| v == next_node) else {
            return DfsEdge::TreeOrCross;
        };
        let cycle_head = *self.active_stack[cycle_pos..]
            .iter()
            .min()
            .expect("cycle slice on active_stack is non-empty");
        let newly_detected_head = self
            .reported_cycle_heads
            .insert(cycle_head)
            .then_some(cycle_head);
        DfsEdge::BackEdgeCycle(DfsCycle {
            newly_detected_head,
        })
    }

    /// Pushes `node` onto the active DFS stack for the duration of `f`, popping it
    /// automatically when `f` returns.
    ///
    /// Returns `Err(DfsCycle)` without calling `f` if `node` is already on the
    /// active stack.
    pub(crate) fn with_node<R>(
        &mut self,
        node: usize,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Result<R, DfsCycle> {
        match self.classify_edge(node) {
            DfsEdge::TreeOrCross => {
                self.active_stack.push(node);
                let result = f(self);
                self.active_stack.pop();
                Ok(result)
            }
            DfsEdge::BackEdgeCycle(cycle) => Err(cycle),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DfsCycle, DfsCycleDetector};

    #[test]
    fn acyclic_chain_succeeds_and_pops_stack() {
        let mut dfs = DfsCycleDetector::default();
        let depth = dfs
            .with_node(0, |dfs| {
                dfs.with_node(1, |dfs| dfs.with_node(2, |_| 3).unwrap())
                    .unwrap()
            })
            .unwrap();
        assert_eq!(depth, 3);

        // Re-visiting node 0 after unwinding is not a cycle.
        assert_eq!(dfs.with_node(0, |_| 42), Ok(42));
    }

    #[test]
    fn cycle_reports_canonical_head_only_once() {
        let mut dfs = DfsCycleDetector::default();

        // Path 2 -> 1 -> 2 forms cycle {1, 2} with canonical head 1.
        let first_err = dfs
            .with_node(2, |dfs| {
                dfs.with_node(1, |dfs| dfs.with_node(2, |_| ())).unwrap()
            })
            .unwrap();
        assert_eq!(
            first_err,
            Err(DfsCycle {
                newly_detected_head: Some(1),
            })
        );

        // Entering the same cycle {1, 2} from node 1 still errors, without re-reporting head 1.
        let second_err = dfs
            .with_node(1, |dfs| {
                dfs.with_node(2, |dfs| dfs.with_node(1, |_| ())).unwrap()
            })
            .unwrap();
        assert_eq!(
            second_err,
            Err(DfsCycle {
                newly_detected_head: None,
            })
        );
    }
}
