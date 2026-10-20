//! Blocks of samples that travel along connections.

use crate::SignalType;

/// One block of samples flowing out of an output socket or into an input socket.
///
/// `signal_type` must match the socket's [`SignalType`]; the host checks this
/// before handing blocks to a node.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleBlock {
    pub signal_type: SignalType,
    pub samples: Vec<f32>,
}

impl SampleBlock {
    pub fn new(signal_type: SignalType, samples: Vec<f32>) -> Self {
        Self {
            signal_type,
            samples,
        }
    }

    /// A block of `frames` zero samples.
    pub fn silent(signal_type: SignalType, frames: usize) -> Self {
        Self::new(signal_type, vec![0.0; frames])
    }

    /// Number of samples (frames) in the block.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}
