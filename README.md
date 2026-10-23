# v-signal-nodes

A node-based signal and audio graph editor written in Rust with
[egui](https://github.com/emilk/egui). Every node type (generators, processors,
outputs) comes from a plugin, so the editor itself knows no node types.

> **Status: early development.** You can build graphs in the editor, but nothing
> plays yet: there's no processing runtime. You can't edit parameters or save
> graphs yet either. The plugin API is still changing, and plugins built for one
> API version are refused by hosts built for another.

## Features

- Canvas with zoom (0.5–2×), scroll bars and a dot grid. You can snap nodes to
  the grid, and nodes never overlap: a dropped node takes a nearby free spot or
  pushes its neighbours aside. A preview shows the result while you drag.
- Connections are routed around nodes and drawn with a color gradient between
  the two node types. Cycles and self-connections are refused.
- An English or Russian interface. Plugins translate their own node labels, and
  nodes on the canvas switch language with the editor.
- A plugin API for Rust crates linked into the editor and for dynamic libraries
  loaded through a C ABI. The C ABI is implemented, but the editor can't load
  libraries yet.
- Standard nodes: `std.source` (sine/square/saw/triangle), `std.gain`,
  `std.mixer`, `std.sink`.

## Building

Requires a stable Rust toolchain with edition 2024 support (Rust 1.85 or newer).

```sh
cargo run -p v-signal-nodes-app       # run the editor (binary: v-signal-nodes)
cargo build -p v-signal-nodes-app --release
```

On Linux, eframe needs the usual X11/Wayland development packages, for example
on Debian or Ubuntu:

```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

The `dev` profile is optimized (`opt-level = 1`, dependencies at 3), because the
unoptimized editor stutters when it routes connections.

## Using the editor

| Action                            | How                                              |
|-----------------------------------|--------------------------------------------------|
| Add a node                        | Click it in the palette, or drag it onto the canvas |
| Move a node                       | Drag its body (`Esc` while dragging cancels the move) |
| Connect                           | Click an output socket, then an input socket     |
| Disconnect                        | Right-click a socket                             |
| Delete a node                     | The cross in its header                          |
| Clear the selection               | Click empty canvas or press `Esc`                |
| Zoom, snap to grid, language      | Toolbar                                          |

Each input takes one connection. Connecting an input that is already connected
replaces its old connection. The bottom panel lists the connections and shows
why a connection was refused.

## Workspace

| Crate                         | What it is                                                     |
|-------------------------------|----------------------------------------------------------------|
| `crates/app`                  | The egui editor                                                |
| `crates/core`                 | Node model: node specs, parameters, sockets, translations      |
| `crates/plugin-api`           | The plugin contract and its C ABI (`ffi`)                      |
| `crates/plugin-host`          | Loads plugins (linked or from a dynamic library) and builds nodes |
| `crates/plugin-std`           | The standard nodes (`std.*`)                                   |
| `crates/plugin-stub`          | A do-nothing node used to test the plugin path                 |
| `crates/runtime`              | Graph processing; a placeholder for now                        |

API docs: `cargo doc --workspace --no-deps --open`.

## Writing a plugin

A plugin depends only on `v-signal-nodes-plugin-api`. It registers one
`NodeProvider` per node type. A provider describes its node type with a
`NodeSpec` and creates a `NodeProcessor`, which holds the DSP state, for every
node placed on the canvas.

```rust
use v_signal_nodes_plugin_api::*;

pub struct MyPlugin;

impl Plugin for MyPlugin {
    fn name(&self) -> &str {
        "my"
    }

    fn register(&self, registrar: &mut dyn Registrar) {
        registrar.register_node(Box::new(Invert));
    }
}

struct Invert;

impl NodeProvider for Invert {
    fn spec(&self) -> NodeSpec {
        NodeSpec::new("my.invert", "Invert", NodeKind::Processor)
            .input(NodeSocket::audio_in("In"))
            .output(NodeSocket::audio_out("Out"))
            .param(ParamSpec::toggle("enabled", "Enabled", true))
            .translate(
                "ru",
                [
                    (TextKey::Name, "Инверсия"),
                    (TextKey::Input(0), "Вход"),
                    (TextKey::Output(0), "Выход"),
                    (TextKey::param("enabled"), "Включено"),
                ],
            )
    }

    fn create_processor(&self) -> Box<dyn NodeProcessor> {
        Box::new(InvertProcessor)
    }
}

struct InvertProcessor;

impl NodeProcessor for InvertProcessor {
    fn process(&mut self, io: &mut ProcessIo<'_>) -> Result<(), PluginError> {
        let sign = if io.toggle(0) { -1.0 } else { 1.0 };
        let input = io.input(0);
        for (out, sample) in io.output(0).iter_mut().zip(input) {
            *out = sample * sign;
        }
        Ok(())
    }
}

// Only for a plugin built as a dynamic library (crate-type = ["cdylib"]).
export_plugin!(MyPlugin);
```

Rules for node specs:

- **Type ids** are namespaced by the plugin, for example `my.invert`, and must
  be unique across all loaded plugins.
- **Roles and sockets must match:** a `Generator` has outputs only, a
  `Processor` has inputs and outputs, and an `Output` has inputs only.
- **Labels are written in English in the spec.** Translations are keyed by
  `TextKey`: the node name, a socket by index, a parameter by id, or a choice
  option by parameter id and index. That way, renaming the English label keeps
  its translations.
- **Validation:** `NodeSpec::validate` (also run when the plugin loads) rejects
  empty ids, duplicate sockets or parameters, out-of-range defaults, and
  translations of labels that don't exist.

Processor life cycle:

- The host calls `prepare(sample_rate, max_frames)` before the first `process`
  and again when either value changes. That's the place to allocate memory,
  because `process` runs on the audio thread.
- `process` gets at most `max_frames` frames per call. Outputs start zeroed.
- `reset` clears the signal history (phase, filter state) when playback jumps.
- Panics in dynamic-library plugins are caught at the C ABI boundary. This
  requires `panic = "unwind"`, which is the default.

To add a plugin to the editor today, add the crate as a dependency of
`crates/app` and load it in `builtin_plugins` in `crates/app/src/editor.rs`.
`PluginHost::load_library` can already load a dynamic library, but the editor
doesn't call it yet.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items
```

CI runs these checks on Linux, macOS and Windows; the format check runs on
Linux only. Every `unsafe` block needs a `// SAFETY:` comment
(`clippy::undocumented_unsafe_blocks`), and all public items of `plugin-api`
must be documented.

## License

BSD 3-Clause; see [LICENSE](LICENSE).
