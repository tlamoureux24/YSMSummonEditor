//! Transient notifications in the bottom-right corner.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};

use crate::icons;
use crate::theme::{self, color};

const MAX_VISIBLE: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    fn icon(self) -> &'static str {
        match self {
            ToastKind::Info => icons::INFO,
            ToastKind::Success => icons::SUCCESS,
            ToastKind::Warning => icons::WARNING,
            ToastKind::Error => icons::ERROR,
        }
    }

    fn tint(self) -> Color32 {
        match self {
            ToastKind::Info => color::info(),
            ToastKind::Success => color::success(),
            ToastKind::Warning => color::warning(),
            ToastKind::Error => color::danger(),
        }
    }

    /// Problems stay on screen longer than confirmations.
    fn lifetime(self) -> Duration {
        match self {
            ToastKind::Info | ToastKind::Success => Duration::from_secs(4),
            ToastKind::Warning => Duration::from_secs(7),
            ToastKind::Error => Duration::from_secs(10),
        }
    }
}

struct Toast {
    kind: ToastKind,
    text: String,
    expires_at: Instant,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    pub fn push(&mut self, kind: ToastKind, text: impl Into<String>) {
        self.items.push(Toast { kind, text: text.into(), expires_at: Instant::now() + kind.lifetime() });
        if self.items.len() > MAX_VISIBLE {
            self.items.remove(0);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.items.retain(|toast| toast.expires_at > now);
        let Some(next_expiry) = self.items.iter().map(|toast| toast.expires_at).min() else {
            return;
        };
        ctx.request_repaint_after(next_expiry - now);

        let mut dismissed = None;
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                    for (index, toast) in self.items.iter().enumerate().rev() {
                        let tint = toast.kind.tint();
                        let frame = egui::Frame::new()
                            .fill(color::panel_alt())
                            .stroke(Stroke::new(1.0, tint.gamma_multiply(0.7)))
                            .corner_radius(CornerRadius::same(theme::CARD_RADIUS))
                            .inner_margin(Margin::symmetric(12, 8));
                        let response = frame
                            .show(ui, |ui| {
                                ui.set_max_width(420.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(toast.kind.icon()).size(theme::GLYPH_SIZE).color(tint));
                                    ui.add(egui::Label::new(toast.text.as_str()).wrap());
                                });
                            })
                            .response;
                        if response.interact(egui::Sense::click()).clicked() {
                            dismissed = Some(index);
                        }
                    }
                });
            });

        if let Some(index) = dismissed {
            self.items.remove(index);
        }
    }
}
