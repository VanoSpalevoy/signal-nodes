use crate::{NodeSocket, ParamValue};

// Node structure for the core of the application. It contains the main logic and
// data structures for the application.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: usize,
    /// Type id of the [`NodeSpec`](super::NodeSpec) the node was built from.
    pub type_id: String,
    pub name: String,
    pub kind: NodeKind,
    pub inputs: Vec<NodeSocket>,
    pub outputs: Vec<NodeSocket>,
    /// Current parameter values, in the order of the spec's
    /// [`params`](super::NodeSpec::params).
    pub params: Vec<ParamValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// A node that generates a signal.
    Generator,
    /// A node that processes a signal.
    Processor,
    /// A node that outputs a signal.
    Output,
}
