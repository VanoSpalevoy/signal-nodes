pub mod error;
pub mod model;
// mod real_time_processing;
// mod async_processing;
pub use crate::error::AppError;
pub use model::{
    Node, NodeFactory, NodeKind, NodeSocket, NodeSpec, ParamKind, ParamSpec, ParamValue,
    SampleBlock, SignalEnvironment, SignalType, SocketDirection, TextKey, Translation,
};

// #[cfg(test)]
