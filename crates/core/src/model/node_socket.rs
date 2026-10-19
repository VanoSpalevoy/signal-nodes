//! Node sockets: the named inputs and outputs of a [`Node`](super::Node), with the
//! kind of signal each one carries.

/// Whether a socket receives a signal or produces one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SocketDirection {
    Input,
    Output,
}

/// How a signal is represented in time and amplitude.
///
/// The four variants are the quadrants of the classic classification:
///
/// | Variant          | Time       | Amplitude  |
/// |------------------|------------|------------|
/// | [`Analog`]       | continuous | continuous |
/// | [`DiscreteTime`] | discrete   | continuous |
/// | [`Quantized`]    | continuous | quantized  |
/// | [`Digital`]      | discrete   | quantized  |
///
/// [`Analog`]: Self::Analog
/// [`DiscreteTime`]: Self::DiscreteTime
/// [`Quantized`]: Self::Quantized
/// [`Digital`]: Self::Digital
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignalType {
    /// Continuous in time and amplitude.
    Analog,
    /// Sampled in time and quantized in amplitude.
    Digital,
    /// Sampled in time, continuous in amplitude.
    DiscreteTime,
    /// Continuous in time, quantized in amplitude.
    Quantized,
}

/// The physical medium a signal travels through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignalEnvironment {
    /// Voltage or current in a circuit.
    Electrical,
    /// Light.
    Optical,
    /// Sound: pressure waves in air or another medium.
    Acoustic,
}

/// A named input or output of a node and the kind of signal it carries.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeSocket {
    pub name: String,
    pub direction: SocketDirection,
    pub signal_type: SignalType,
    pub signal_environment: SignalEnvironment,
}

impl NodeSocket {
    /// An input socket named `name`, receiving the given kind of signal.
    pub fn input(
        name: impl Into<String>,
        signal_type: SignalType,
        signal_environment: SignalEnvironment,
    ) -> Self {
        Self::new(
            name,
            SocketDirection::Input,
            signal_type,
            signal_environment,
        )
    }

    /// An output socket named `name`, producing the given kind of signal.
    pub fn output(
        name: impl Into<String>,
        signal_type: SignalType,
        signal_environment: SignalEnvironment,
    ) -> Self {
        Self::new(
            name,
            SocketDirection::Output,
            signal_type,
            signal_environment,
        )
    }

    fn new(
        name: impl Into<String>,
        direction: SocketDirection,
        signal_type: SignalType,
        signal_environment: SignalEnvironment,
    ) -> Self {
        Self {
            name: name.into(),
            direction,
            signal_type,
            signal_environment,
        }
    }

    pub fn audio_in(name: impl Into<String>) -> NodeSocket {
        NodeSocket::input(name, SignalType::Analog, SignalEnvironment::Acoustic)
    }

    pub fn audio_out(name: impl Into<String>) -> NodeSocket {
        NodeSocket::output(name, SignalType::Analog, SignalEnvironment::Acoustic)
    }

    pub fn analog_in(name: impl Into<String>) -> NodeSocket {
        NodeSocket::input(name, SignalType::Analog, SignalEnvironment::Electrical)
    }

    pub fn analog_out(name: impl Into<String>) -> NodeSocket {
        NodeSocket::output(name, SignalType::Analog, SignalEnvironment::Electrical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_set_the_direction() {
        let input = NodeSocket::input("in", SignalType::Analog, SignalEnvironment::Acoustic);
        let output = NodeSocket::output(
            String::from("out"),
            SignalType::Digital,
            SignalEnvironment::Electrical,
        );

        assert_eq!(input.direction, SocketDirection::Input);
        assert_eq!(input.name, "in");
        assert_eq!(output.direction, SocketDirection::Output);
        assert_eq!(output.signal_type, SignalType::Digital);
    }
}
