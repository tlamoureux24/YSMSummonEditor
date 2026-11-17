//! Informational and confirmation dialogs.

use eframe::egui::{self, Color32, RichText};

use super::DialogContext;
use crate::action::Action;
use crate::icons;
use crate::theme::{self, color, space};
use crate::widgets::{self, modal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Question,
    Warning,
    Error,
}

impl MessageKind {
    fn icon(self) -> (&'static str, Color32) {
        match self {
            MessageKind::Question => (icons::QUESTION, color::accent_bright()),
            MessageKind::Warning => (icons::WARNING, color::warning()),
            MessageKind::Error => (icons::ERROR, color::danger()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Danger,
    Plain,
}

struct MessageButton {
    icon: &'static str,
    label: String,
    style: ButtonStyle,
    action: Option<Action>,
}

pub struct MessageDialog {
    kind: MessageKind,
    title: String,
    text: String,
    /// Bulleted rows shown in a scrollable list, e.g. affected summons.
    items: Vec<String>,
    /// Technical detail (error text, payload summary) shown in monospace.
    details: Option<String>,
    buttons: Vec<MessageButton>,
    /// Sent when the dialog is dismissed with Escape or a click outside.
    on_dismiss: Option<Action>,
}

impl MessageDialog {
    pub fn new(kind: MessageKind, title: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            text: text.into(),
            items: Vec::new(),
            details: None,
            buttons: Vec::new(),
            on_dismiss: None,
        }
    }

    /// An error with its technical detail and a single Close button.
    pub fn error(title: impl Into<String>, text: impl Into<String>, details: impl Into<String>) -> Self {
        Self::new(MessageKind::Error, title, text).details(details).button(
            icons::CLOSE,
            "Close",
            ButtonStyle::Plain,
            None,
        )
    }

    pub fn items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }

    pub fn details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Adds a button; buttons appear left to right in the order they are added.
    pub fn button(mut self, icon: &'static str, label: &str, style: ButtonStyle, action: Option<Action>) -> Self {
        self.buttons.push(MessageButton { icon, label: label.to_owned(), style, action });
        self
    }

    pub fn on_dismiss(mut self, action: Action) -> Self {
        self.on_dismiss = Some(action);
        self
    }

    pub fn show(&mut self, ctx: &egui::Context, id: egui::Id, env: &mut DialogContext<'_>) -> bool {
        let (icon, tint) = self.kind.icon();
        let mut clicked = None;

        let outcome = modal::show(ctx, id, icon, tint, &self.title, 520.0, |ui| {
            ui.add(egui::Label::new(self.text.as_str()).wrap());

            if !self.items.is_empty() {
                theme::well().inner_margin(space::SM as i8).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("items").max_height(200.0).auto_shrink([false, true]).show(
                        ui,
                        |ui| {
                            for item in &self.items {
                                ui.label(RichText::new(format!("•  {item}")).color(color::text_muted()));
                            }
                        },
                    );
                });
            }

            if let Some(details) = &self.details {
                theme::well().inner_margin(space::SM as i8).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("details").max_height(180.0).auto_shrink([false, true]).show(
                        ui,
                        |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(details).monospace().color(color::text_muted())).wrap(),
                            );
                        },
                    );
                });
            }

            modal::footer(ui, |ui| {
                // The layout runs right to left, so add the buttons in reverse.
                for (index, button) in self.buttons.iter().enumerate().rev() {
                    let response = match button.style {
                        ButtonStyle::Primary => widgets::primary_button(ui, button.icon, &button.label),
                        ButtonStyle::Danger => widgets::danger_button(ui, button.icon, &button.label),
                        ButtonStyle::Plain => widgets::button(ui, button.icon, &button.label),
                    };
                    if response.clicked() {
                        clicked = Some(index);
                    }
                }
                // Enter runs the default action, but never a destructive one.
                if modal::enter_pressed(ui) && self.buttons.first().is_some_and(|b| b.style == ButtonStyle::Primary) {
                    clicked = Some(0);
                }
            });
        });

        if let Some(index) = clicked {
            env.actions.extend(self.buttons[index].action.take());
            return false;
        }
        if outcome.dismissed {
            env.actions.extend(self.on_dismiss.take());
            return false;
        }
        true
    }
}
