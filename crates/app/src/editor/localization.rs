//! UI translations.
//!
//! Every user-visible editor string is a [`Text`] key resolved with
//! [`Language::text`] at paint time, so switching the language takes effect on the
//! next frame everywhere. To add a string: add a `Text` variant and one arm per
//! language in [`Language::text`]; the exhaustive `match` makes a missing
//! translation a compile error.
//!
//! Node text is not here: it comes from the plugins, which ship their own
//! translations in the node specs, looked up by [`Language::code`].

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
    Input,
    Output,
    DeleteNode,
    NodeLimitReached,
    ConnectionCreatesCycle,
    SnapToGrid,
    Scale,
    // Toggle parameter values, the same for every plugin.
    On,
    Off,
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

    /// Language code used by plugin translations (`NodeSpec::label`), e.g. `"ru"`.
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Russian => "ru",
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
            (Self::English, Text::Input) => "Input",
            (Self::English, Text::Output) => "Output",
            (Self::English, Text::DeleteNode) => "Delete node",
            (Self::English, Text::NodeLimitReached) => "Node limit reached",
            (Self::English, Text::ConnectionCreatesCycle) => {
                "Connection refused: it would create a cycle"
            }
            (Self::English, Text::SnapToGrid) => "Snap to grid",
            (Self::English, Text::Scale) => "Scale",
            (Self::English, Text::On) => "on",
            (Self::English, Text::Off) => "off",
            (Self::Russian, Text::Language) => "Язык",
            (Self::Russian, Text::Tools) => "Инструменты",
            (Self::Russian, Text::Settings) => "Настройки",
            (Self::Russian, Text::Nodes) => "Ноды",
            (Self::Russian, Text::Debug) => "Отладка",
            (Self::Russian, Text::Ready) => "Готово",
            (Self::Russian, Text::SelectedSourceSocket) => "Выбран выходной сокет ноды",
            (Self::Russian, Text::Input) => "Вход",
            (Self::Russian, Text::Output) => "Выход",
            (Self::Russian, Text::DeleteNode) => "Удалить ноду",
            (Self::Russian, Text::NodeLimitReached) => "Достигнут лимит нод",
            (Self::Russian, Text::ConnectionCreatesCycle) => {
                "Соединение отклонено: образуется цикл"
            }
            (Self::Russian, Text::SnapToGrid) => "Привязать к сетке",
            (Self::Russian, Text::Scale) => "Масштаб",
            (Self::Russian, Text::On) => "вкл",
            (Self::Russian, Text::Off) => "выкл",
        }
    }
}
