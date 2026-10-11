//! UI translations.
//!
//! Every user-visible string is a [`Text`] key resolved with [`Language::text`] at
//! paint time, so switching the language takes effect on the next frame everywhere,
//! including nodes that already exist. To add a string: add a `Text` variant and one
//! arm per language in [`Language::text`]; the exhaustive `match` makes a missing
//! translation a compile error.

/// A UI language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Language {
    English,
    Russian,
}

/// Key of a translatable string; see [`Language::text`] for the actual strings.
#[derive(Clone, Copy, Debug)]
pub(super) enum Text {
    Language,
    Tools,
    Settings,
    Nodes,
    Debug,
    Ready,
    SelectedSourceSocket,
    Source,
    Gain,
    Mixer,
    Sink,
    Input,
    Output,
    DeleteNode,
    NodeLimitReached,
    // EnableGrid,
    Format,
    Scale,
    // Socket labels
    Audio,
    AudioIn,
    AudioOut,
    Left,
    Right,
    Side,
    Mixed,
    // Node parameters
    Signal,
    Sine,
    Mode,
    Stereo,
    Pan,
    Center,
    Channels,
}

impl Language {
    /// All languages, in the order shown in the toolbar picker.
    pub(super) const ALL: [Self; 2] = [Self::English, Self::Russian];

    /// The language's name in itself ("Русский", not "Russian"), for the language picker.
    pub(super) fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Russian => "Русский",
        }
    }

    /// The string for `key` in this language.
    pub(super) fn text(self, key: Text) -> &'static str {
        match (self, key) {
            (Self::English, Text::Language) => "Language",
            (Self::English, Text::Tools) => "Tools",
            (Self::English, Text::Settings) => "Settings",
            (Self::English, Text::Nodes) => "Nodes",
            (Self::English, Text::Debug) => "Debug",
            (Self::English, Text::Ready) => "Ready",
            (Self::English, Text::SelectedSourceSocket) => "Selected source socket: node",
            (Self::English, Text::Source) => "Source",
            (Self::English, Text::Gain) => "Gain",
            (Self::English, Text::Mixer) => "Mixer",
            (Self::English, Text::Sink) => "Sink",
            (Self::English, Text::Input) => "Input",
            (Self::English, Text::Output) => "Output",
            (Self::English, Text::DeleteNode) => "Delete node",
            (Self::English, Text::NodeLimitReached) => "Node limit reached",
            // (Self::English, Text::EnableGrid) => "Enable Grid",
            (Self::English, Text::Format) => "Format",
            (Self::English, Text::Scale) => "Scale",
            (Self::English, Text::Audio) => "Audio",
            (Self::English, Text::AudioIn) => "Audio in",
            (Self::English, Text::AudioOut) => "Audio out",
            (Self::English, Text::Left) => "Left",
            (Self::English, Text::Right) => "Right",
            (Self::English, Text::Side) => "Side",
            (Self::English, Text::Mixed) => "Mixed",
            (Self::English, Text::Signal) => "Signal",
            (Self::English, Text::Sine) => "sine",
            (Self::English, Text::Mode) => "Mode",
            (Self::English, Text::Stereo) => "stereo",
            (Self::English, Text::Pan) => "Pan",
            (Self::English, Text::Center) => "center",
            (Self::English, Text::Channels) => "Channels",
            (Self::Russian, Text::Language) => "Язык",
            (Self::Russian, Text::Tools) => "Инструменты",
            (Self::Russian, Text::Settings) => "Настройки",
            (Self::Russian, Text::Nodes) => "Ноды",
            (Self::Russian, Text::Debug) => "Отладка",
            (Self::Russian, Text::Ready) => "Готово",
            (Self::Russian, Text::SelectedSourceSocket) => "Выбран выходной сокет ноды",
            (Self::Russian, Text::Source) => "Источник",
            (Self::Russian, Text::Gain) => "Усиление",
            (Self::Russian, Text::Mixer) => "Микшер",
            (Self::Russian, Text::Sink) => "Приёмник",
            (Self::Russian, Text::Input) => "Вход",
            (Self::Russian, Text::Output) => "Выход",
            (Self::Russian, Text::DeleteNode) => "Удалить ноду",
            (Self::Russian, Text::NodeLimitReached) => "Достигнут лимит нод",
            // (Self::Russian, Text::EnableGrid) => "Включить сетку",
            (Self::Russian, Text::Format) => "Форматировать",
            (Self::Russian, Text::Scale) => "Масштаб",
            (Self::Russian, Text::Audio) => "Аудио",
            (Self::Russian, Text::AudioIn) => "Аудио вход",
            (Self::Russian, Text::AudioOut) => "Аудио выход",
            (Self::Russian, Text::Left) => "Левый",
            (Self::Russian, Text::Right) => "Правый",
            (Self::Russian, Text::Side) => "Боковой",
            (Self::Russian, Text::Mixed) => "Микс",
            (Self::Russian, Text::Signal) => "Сигнал",
            (Self::Russian, Text::Sine) => "синус",
            (Self::Russian, Text::Mode) => "Режим",
            (Self::Russian, Text::Stereo) => "стерео",
            (Self::Russian, Text::Pan) => "Панорама",
            (Self::Russian, Text::Center) => "центр",
            (Self::Russian, Text::Channels) => "Каналы",
        }
    }
}
