//! The standard plugin: built-in node types (`std.*`).
//!
//! | Type id      | Kind      | Inputs            | Outputs   | Parameters                   |
//! |--------------|-----------|-------------------|-----------|------------------------------|
//! | `std.source` | Generator | —                 | Audio     | Signal, Frequency            |
//! | `std.gain`   | Processor | Audio in          | Audio out | Gain                         |
//! | `std.mixer`  | Processor | Left, Right, Side | Mixed     | Mode, Gain, Pan              |
//! | `std.sink`   | Output    | Audio             | —         | Channels                     |
//!
//! Every label is English in the spec, with Russian translations next to it
//! (`NodeSpec::translate`), so the editor needs no strings of its own for these
//! nodes.

use std::f32::consts::TAU;

use v_signal_nodes_plugin_api::{
    AppError, NodeKind, NodeProcessor, NodeProvider, NodeSocket, NodeSpec, ParamSpec, Plugin,
    ProcessIo, Registrar,
};

/// Type id of the signal generator.
pub const SOURCE: &str = "std.source";
/// Type id of the gain (volume) node.
pub const GAIN: &str = "std.gain";
/// Type id of the mixer.
pub const MIXER: &str = "std.mixer";
/// Type id of the signal sink (graph output).
pub const SINK: &str = "std.sink";

/// The standard plugin, named `"std"`.
pub struct StdPlugin;

impl Plugin for StdPlugin {
    fn name(&self) -> &str {
        "std"
    }

    fn register(&self, registrar: &mut dyn Registrar) {
        registrar.register_node(Box::new(Source));
        registrar.register_node(Box::new(Gain));
        registrar.register_node(Box::new(Mixer));
        registrar.register_node(Box::new(Sink));
    }
}

/// [`SOURCE`]: a periodic waveform at a set frequency.
struct Source;

/// Options of the source's `signal` parameter, in index order.
const WAVEFORMS: [&str; 4] = ["sine", "square", "saw", "triangle"];

impl NodeProvider for Source {
    fn spec(&self) -> NodeSpec {
        NodeSpec::new(SOURCE, "Source", NodeKind::Generator)
            .output(NodeSocket::audio_out("Audio"))
            .param(ParamSpec::choice("signal", "Signal", WAVEFORMS, 0))
            .param(ParamSpec::number(
                "frequency",
                "Frequency",
                20.0,
                20_000.0,
                440.0,
            ))
            .translate(
                "ru",
                [
                    ("Source", "Источник"),
                    ("Audio", "Аудио"),
                    ("Signal", "Сигнал"),
                    ("sine", "синус"),
                    ("square", "меандр"),
                    ("saw", "пила"),
                    ("triangle", "треугольник"),
                    ("Frequency", "Частота"),
                ],
            )
    }

    fn create_processor(&self) -> Box<dyn NodeProcessor> {
        Box::new(SourceProcessor { phase: 0.0 })
    }
}

struct SourceProcessor {
    /// Position in the current period, `0.0..1.0`.
    phase: f32,
}

impl NodeProcessor for SourceProcessor {
    fn process(&mut self, io: &mut ProcessIo<'_>) -> Result<(), AppError> {
        let waveform = io.choice(0);
        let step = io.number(1) / io.sample_rate();
        for sample in io.output(0) {
            let phase = self.phase;
            *sample = match waveform {
                0 => (phase * TAU).sin(),
                1 => {
                    if phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                2 => 2.0 * phase - 1.0,
                _ => 1.0 - 4.0 * (phase - 0.5).abs(),
            };
            self.phase = (phase + step).fract();
        }
        Ok(())
    }
}

/// [`GAIN`]: multiplies the signal by a factor.
struct Gain;

impl NodeProvider for Gain {
    fn spec(&self) -> NodeSpec {
        NodeSpec::new(GAIN, "Gain", NodeKind::Processor)
            .input(NodeSocket::audio_in("Audio in"))
            .output(NodeSocket::audio_out("Audio out"))
            .param(ParamSpec::number("gain", "Gain", 0.0, 2.0, 1.0))
            .translate(
                "ru",
                [
                    ("Gain", "Усиление"),
                    ("Audio in", "Аудио вход"),
                    ("Audio out", "Аудио выход"),
                ],
            )
    }

    fn create_processor(&self) -> Box<dyn NodeProcessor> {
        Box::new(GainProcessor)
    }
}

struct GainProcessor;

impl NodeProcessor for GainProcessor {
    fn process(&mut self, io: &mut ProcessIo<'_>) -> Result<(), AppError> {
        let gain = io.number(0);
        let input = io.input(0);
        for (out, sample) in io.output(0).iter_mut().zip(input) {
            *out = sample * gain;
        }
        Ok(())
    }
}

/// [`MIXER`]: sums its inputs into one output, scaled by `gain`.
///
/// The output is a single channel, so `mode` and `pan` are not applied yet.
struct Mixer;

impl NodeProvider for Mixer {
    fn spec(&self) -> NodeSpec {
        NodeSpec::new(MIXER, "Mixer", NodeKind::Processor)
            .input(NodeSocket::audio_in("Left"))
            .input(NodeSocket::audio_in("Right"))
            .input(NodeSocket::audio_in("Side"))
            .output(NodeSocket::audio_out("Mixed"))
            .param(ParamSpec::choice("mode", "Mode", ["mono", "stereo"], 1))
            .param(ParamSpec::number("gain", "Gain", 0.0, 2.0, 1.0))
            .param(ParamSpec::choice(
                "pan",
                "Pan",
                ["left", "center", "right"],
                1,
            ))
            .translate(
                "ru",
                [
                    ("Mixer", "Микшер"),
                    ("Left", "Левый"),
                    ("Right", "Правый"),
                    ("Side", "Боковой"),
                    ("Mixed", "Микс"),
                    ("Mode", "Режим"),
                    ("mono", "моно"),
                    ("stereo", "стерео"),
                    ("Gain", "Усиление"),
                    ("Pan", "Панорама"),
                    ("left", "лево"),
                    ("center", "центр"),
                    ("right", "право"),
                ],
            )
    }

    fn create_processor(&self) -> Box<dyn NodeProcessor> {
        Box::new(MixerProcessor)
    }
}

struct MixerProcessor;

impl NodeProcessor for MixerProcessor {
    fn process(&mut self, io: &mut ProcessIo<'_>) -> Result<(), AppError> {
        let gain = io.number(1);
        let inputs: Vec<&[f32]> = (0..io.input_count()).map(|index| io.input(index)).collect();
        for (frame, out) in io.output(0).iter_mut().enumerate() {
            *out = inputs.iter().map(|input| input[frame]).sum::<f32>() * gain;
        }
        Ok(())
    }
}

/// [`SINK`]: the end of the graph; the host reads its input.
struct Sink;

impl NodeProvider for Sink {
    fn spec(&self) -> NodeSpec {
        NodeSpec::new(SINK, "Sink", NodeKind::Output)
            .input(NodeSocket::audio_in("Audio"))
            .param(ParamSpec::choice(
                "channels",
                "Channels",
                ["mono", "stereo"],
                1,
            ))
            .translate(
                "ru",
                [
                    ("Sink", "Приёмник"),
                    ("Audio", "Аудио"),
                    ("Channels", "Каналы"),
                    ("mono", "моно"),
                    ("stereo", "стерео"),
                ],
            )
    }

    fn create_processor(&self) -> Box<dyn NodeProcessor> {
        Box::new(SinkProcessor)
    }
}

/// Does nothing: the sink has no outputs to fill.
struct SinkProcessor;

impl NodeProcessor for SinkProcessor {
    fn process(&mut self, _io: &mut ProcessIo<'_>) -> Result<(), AppError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use v_signal_nodes_plugin_api::{ParamValue, RawBuffer};

    fn providers() -> Vec<Box<dyn NodeProvider>> {
        vec![
            Box::new(Source),
            Box::new(Gain),
            Box::new(Mixer),
            Box::new(Sink),
        ]
    }

    /// Runs `processor` once over `inputs` and returns its first output (or
    /// nothing for a node without outputs).
    fn run(
        processor: &mut dyn NodeProcessor,
        params: &[ParamValue],
        inputs: &[&[f32]],
        outputs: usize,
        frames: usize,
    ) -> Vec<f32> {
        let mut output_samples = vec![vec![0.0; frames]; outputs];
        let inputs: Vec<_> = inputs
            .iter()
            .map(|samples| RawBuffer::input(samples))
            .collect();
        let raw_outputs: Vec<_> = output_samples
            .iter_mut()
            .map(|samples| RawBuffer::output(samples))
            .collect();
        // SAFETY: every buffer is `frames` long, lives for the call, and the
        // outputs are separate vectors.
        let mut io = unsafe { ProcessIo::from_raw(4.0, frames, params, &inputs, &raw_outputs) };
        processor.process(&mut io).unwrap();
        output_samples.into_iter().next().unwrap_or_default()
    }

    fn defaults(provider: &dyn NodeProvider) -> Vec<ParamValue> {
        provider
            .spec()
            .params
            .iter()
            .map(ParamSpec::default_value)
            .collect()
    }

    #[test]
    fn specs_are_valid_and_translated() {
        for provider in providers() {
            let spec = provider.spec();
            assert!(spec.validate().is_ok(), "{}", spec.type_id);
            assert_ne!(spec.label("ru", &spec.name), spec.name, "{}", spec.type_id);
            for socket in spec.inputs.iter().chain(&spec.outputs) {
                assert_ne!(spec.label("ru", &socket.name), socket.name);
            }
            for param in &spec.params {
                assert_ne!(spec.label("ru", &param.name), param.name);
            }
        }
    }

    #[test]
    fn gain_scales_the_input() {
        let mut processor = Gain.create_processor();
        let output = run(
            processor.as_mut(),
            &[ParamValue::Number(0.5)],
            &[&[1.0, -2.0]],
            1,
            2,
        );
        assert_eq!(output, [0.5, -1.0]);
    }

    #[test]
    fn mixer_sums_the_inputs() {
        let mut processor = Mixer.create_processor();
        let output = run(
            processor.as_mut(),
            &defaults(&Mixer),
            &[&[1.0, 0.0], &[0.5, 1.0], &[0.25, -1.0]],
            1,
            2,
        );
        assert_eq!(output, [1.75, 0.0]);
    }

    #[test]
    fn source_generates_the_waveform_across_calls() {
        // A 1 Hz square at 4 samples per second: two highs, two lows, repeating.
        let params = [ParamValue::Choice(1), ParamValue::Number(1.0)];
        let mut processor = Source.create_processor();
        assert_eq!(
            run(processor.as_mut(), &params, &[], 1, 3),
            [1.0, 1.0, -1.0]
        );
        assert_eq!(
            run(processor.as_mut(), &params, &[], 1, 3),
            [-1.0, 1.0, 1.0]
        );
    }
}
