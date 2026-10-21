//! User interface. Views draw from app state and push [`Action`]s for
//! anything that needs the whole app.

pub mod chat;
pub mod composer;
pub mod settings;
pub mod sidebar;
pub mod syntax;
pub mod tools;

pub use material_3_ui::toasts::ToastKind;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum Action {
    Toast(String, ToastKind),
    /// Start a new chat in a project folder.
    NewChat(PathBuf),
    /// Reopen a past session.
    OpenSession(crate::claude::index::SessionMeta),
    SelectTab(usize),
    CloseTab(usize),
    ShowSettings,
    /// Ask the desktop for a folder (xdg-desktop-portal), then start a chat.
    PickFolder,
    AddProject(PathBuf),
    RemoveProject(String),
    ForgetSession(String),
    OpenPath(PathBuf),
    OpenUrl(String),
    /// Open a terminal running `claude` so the user can sign in.
    SignIn,
    RefreshInfo,
    ConfigChanged,
}

pub type Actions = Vec<Action>;

pub fn toast(actions: &mut Actions, text: impl Into<String>, kind: ToastKind) {
    actions.push(Action::Toast(text.into(), kind));
}
