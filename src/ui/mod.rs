//! User interface. Views draw from app state and push [`Action`]s for
//! anything that needs the whole app.

pub mod chat;
pub mod common;
pub mod composer;
pub mod markdown;
pub mod settings;
pub mod sidebar;
pub mod toasts;
pub mod tools;
pub mod widgets;

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Success is styled by toasts.rs; kept for parity with the other apps.
pub enum ToastKind {
    Info,
    Success,
    Error,
}

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
