use eframe::egui::{self, Align2, CursorIcon, Painter, PointerButton, Rect, RichText, Sense, Stroke, Ui};
use ysm_core::model::{STAT_COUNT, Stat};
use ysm_core::scaling::MIN_LEVEL;

use super::STAT_COLORS;
use crate::theme::{self, color, space};
use crate::widgets;
use ysm_core::i18n::{tr, trf};

const STAGE_HEADER_HEIGHT: f32 = space::LG + space::XS;

pub(super) struct StatChart {
    visible: [bool; STAT_COUNT],
}

impl StatChart {
    pub(super) fn new() -> Self {
        Self { visible: [true; STAT_COUNT] }
    }

    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        height: f32,
        curve: &[[i32; STAT_COUNT]],
        level: &mut i32,
        max_level: i32,
        stage_markers: &[(i32, i32)],
        evolution_jumps: bool,
    ) {
        let (id, rect) = ui.allocate_space(egui::vec2(ui.available_width(), height));
        let painter = ui.painter_at(rect);
        painter.rect(
            rect,
            theme::RADIUS,
            color::input_bg(),
            Stroke::new(1.0, color::border()),
            egui::StrokeKind::Inside,
        );

        let mut values = curve.iter().flat_map(|values| {
            values.iter().zip(self.visible).filter_map(|(&value, visible)| visible.then_some(value))
        });
        let scale = values.next().map(|first| {
            let (min, max) = values.fold((first, first), |(min, max), value| (min.min(value), max.max(value)));
            let intervals = ((height - 60.0) / 28.0).round().clamp(3.0, 8.0) as i64;
            ValueScale::new(min, max, intervals)
        });

        let tick_width = scale.map_or(space::XL, |scale| {
            scale
                .ticks()
                .map(|tick| painter.layout_no_wrap(tick.to_string(), theme::caption(), color::text_muted()).size().x)
                .fold(0.0, f32::max)
        });
        let plot = Rect::from_min_max(
            rect.left_top() + egui::vec2(2.0 * space::LG + tick_width + space::SM, space::XL),
            rect.right_bottom() - egui::vec2(space::XL, 2.0 * space::LG + space::XS),
        );
        let level_axis = LevelAxis { plot, max_level };
        let response = ui.interact(plot, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::Crosshair);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), tr("Stat progression plot"))
        });
        if (response.clicked() || response.dragged_by(PointerButton::Primary))
            && let Some(pointer) = response.interact_pointer_pos()
        {
            *level = level_axis.level_at(pointer.x);
        }
        let hover_level =
            response.hover_pos().map(|pointer| level_axis.level_at(pointer.x)).filter(|hover| hover != level);

        paint_stage_bands(&painter, level_axis, stage_markers);
        paint_axes(&painter, rect, level_axis, scale, stage_markers);
        if let Some(scale) = scale {
            let jumps = if evolution_jumps { stage_markers } else { &[] };
            self.paint_curves(&painter, level_axis, scale, curve, *level, hover_level, jumps);
        } else {
            painter.text(
                plot.center(),
                Align2::CENTER_CENTER,
                tr("Select a stat below to show its curve"),
                theme::body(),
                color::text_muted(),
            );
        }
        self.legend(ui, curve, *level, hover_level, stage_markers);
    }

    fn paint_curves(
        &self,
        painter: &Painter,
        level_axis: LevelAxis,
        scale: ValueScale,
        curve: &[[i32; STAT_COUNT]],
        selected: i32,
        hovered: Option<i32>,
        jumps: &[(i32, i32)],
    ) {
        let plot = level_axis.plot;
        let painter = painter.with_clip_rect(plot.expand(1.0));
        for stat in Stat::ALL.into_iter().filter(|stat| self.visible[stat.index()]) {
            let mut points: Vec<egui::Pos2> = Vec::with_capacity(curve.len() + jumps.len());
            let mut transition_levels = jumps.iter().map(|&(_, start)| start).peekable();
            for (index, values) in curve.iter().enumerate() {
                let level = MIN_LEVEL + index as i32;
                let x = level_axis.x_at(level);
                let y = scale.y_at(plot, i64::from(values[stat.index()]));
                // Hold the previous form's final sample until the next form takes over.
                if transition_levels.next_if_eq(&level).is_some()
                    && let Some(previous) = points.last()
                {
                    points.push(egui::pos2(x, previous.y));
                }
                points.push(egui::pos2(x, y));
            }
            painter.line(points, Stroke::new(1.5, color::readable(STAT_COLORS[stat.index()])));
        }
        for (level, tint) in [(Some(selected), color::text()), (hovered, color::accent_bright())] {
            if let Some(level) = level {
                let x = level_axis.x_at(level);
                painter.vline(x, plot.y_range(), Stroke::new(1.0, tint.gamma_multiply(0.65)));
                if let Some(values) = curve.get((level - MIN_LEVEL) as usize) {
                    for stat in Stat::ALL.into_iter().filter(|stat| self.visible[stat.index()]) {
                        let y = scale.y_at(plot, i64::from(values[stat.index()]));
                        painter.circle(
                            egui::pos2(x, y),
                            3.0,
                            color::readable(STAT_COLORS[stat.index()]),
                            Stroke::new(1.0, color::input_bg()),
                        );
                    }
                }
            }
        }
    }

    fn legend(
        &mut self,
        ui: &mut Ui,
        curve: &[[i32; STAT_COUNT]],
        selected: i32,
        hovered: Option<i32>,
        stage_markers: &[(i32, i32)],
    ) {
        let level = hovered.unwrap_or(selected);
        let values = curve.get((level - MIN_LEVEL) as usize);
        ui.horizontal(|ui| {
            let label_width = if stage_markers.is_empty() { 120.0 } else { 168.0 };
            let gap = ui.spacing().item_spacing.x;
            let item_width =
                ((ui.available_width() - label_width - STAT_COUNT as f32 * gap) / STAT_COUNT as f32).floor();
            let source = if hovered.is_some() { tr("Hover") } else { tr("Selected") };
            let mut label = trf("{} · Lv. {}", &[&source, &level]);
            if let Some(&(stage, _)) = stage_markers.iter().rev().find(|&&(_, start)| start <= level) {
                label.push_str(&trf(" · Stage {}", &[&stage]));
            }
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, theme::CONTROL_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(label_width);
                    ui.label(widgets::caption(label));
                },
            );
            ui.spacing_mut().button_padding.x = space::SM;
            for stat in Stat::ALL {
                let on = &mut self.visible[stat.index()];
                let tint = if *on { color::readable(STAT_COLORS[stat.index()]) } else { color::text_muted() };
                let name = RichText::new(stat.label()).font(theme::bold(theme::CAPTION_SIZE)).color(tint);
                let mut button = egui::Button::new(name).selected(*on).frame_when_inactive(*on).truncate();
                if *on && let Some(values) = values {
                    button = button.right_text(
                        RichText::new(values[stat.index()].to_string()).font(theme::caption()).color(color::text()),
                    );
                }
                let verb = if *on { tr("Hide") } else { tr("Show") };
                if ui
                    .add_sized([item_width, theme::CONTROL_HEIGHT], button)
                    .on_hover_text(trf("{} {} curve", &[&verb, &stat.label()]))
                    .clicked()
                {
                    *on = !*on;
                    ui.ctx().request_repaint();
                }
            }
        });
    }
}

#[derive(Clone, Copy)]
struct LevelAxis {
    plot: Rect,
    max_level: i32,
}

impl LevelAxis {
    fn x_at(self, level: i32) -> f32 {
        let level_span = (self.max_level - MIN_LEVEL).max(1);
        self.plot.left() + self.plot.width() * (level - MIN_LEVEL) as f32 / level_span as f32
    }

    fn level_at(self, x: f32) -> i32 {
        let fraction = ((x - self.plot.left()) / self.plot.width()).clamp(0.0, 1.0);
        MIN_LEVEL + (fraction * (self.max_level - MIN_LEVEL) as f32).round() as i32
    }

    fn ticks(self) -> impl Iterator<Item = i32> {
        let step = if self.max_level <= 200 { 25 } else { 50 };
        std::iter::once(MIN_LEVEL)
            .chain((step..self.max_level).step_by(step as usize))
            .chain((self.max_level > MIN_LEVEL).then_some(self.max_level))
    }
}

fn paint_stage_bands(painter: &Painter, level_axis: LevelAxis, stage_markers: &[(i32, i32)]) {
    let plot = level_axis.plot;
    let label_font = theme::bold(theme::CAPTION_SIZE);
    for (index, &(stage, start)) in stage_markers.iter().enumerate() {
        let next_start = stage_markers.get(index + 1).map(|&(_, start)| start);
        let end = next_start.map_or(level_axis.max_level, |start| start - 1);
        let left = level_axis.x_at(start);
        let right = next_start.map_or(plot.right(), |start| level_axis.x_at(start));
        let header =
            Rect::from_min_max(egui::pos2(left, plot.top() - STAGE_HEADER_HEIGHT), egui::pos2(right, plot.top()));
        let tint = match stage {
            2 => color::accent_bright(),
            3 => color::text_muted(),
            _ => color::text_faint(),
        };
        let band = Rect::from_min_max(header.left_bottom(), egui::pos2(right, plot.bottom()));
        painter.rect_filled(band, 0.0, tint.gamma_multiply(0.08));
        painter.rect_filled(header, 0.0, tint.gamma_multiply(0.24));

        let range = if start == end { start.to_string() } else { format!("{start}–{end}") };
        let mut job = egui::text::LayoutJob::simple(
            trf("Stage {} · Lv. {}", &[&stage, &range]),
            label_font.clone(),
            color::text(),
            (header.width() - 2.0 * space::XS).max(0.0),
        );
        job.wrap.max_rows = 1;
        let label = painter.layout_job(job);
        painter.with_clip_rect(header).galley(header.center() - label.size() * 0.5, label, color::text());
    }
}

fn paint_axes(
    painter: &Painter,
    rect: Rect,
    level_axis: LevelAxis,
    scale: Option<ValueScale>,
    stage_markers: &[(i32, i32)],
) {
    let plot = level_axis.plot;
    let grid = Stroke::new(1.0, color::border());
    let axis = Stroke::new(1.0, color::text_faint());
    let caption = theme::caption();
    if let Some(scale) = scale {
        for tick in scale.ticks() {
            let y = scale.y_at(plot, tick);
            let line = if tick == 0 { Stroke::new(1.0, color::border_strong()) } else { grid };
            painter.hline(plot.x_range(), y, line);
            painter.hline((plot.left() - space::XS)..=plot.left(), y, axis);
            painter.text(
                egui::pos2(plot.left() - space::SM, y),
                Align2::RIGHT_CENTER,
                tick.to_string(),
                caption.clone(),
                color::text_muted(),
            );
        }
    }
    for tick in level_axis.ticks() {
        let x = level_axis.x_at(tick);
        painter.vline(x, plot.y_range(), grid);
        painter.vline(x, plot.bottom()..=plot.bottom() + space::XS, axis);
        painter.text(
            egui::pos2(x, plot.bottom() + space::XS),
            Align2::CENTER_TOP,
            tick.to_string(),
            caption.clone(),
            color::text_muted(),
        );
    }

    for &(_, start) in stage_markers.iter().skip(1) {
        painter.vline(
            level_axis.x_at(start),
            (plot.top() - STAGE_HEADER_HEIGHT)..=plot.bottom(),
            Stroke::new(1.5, color::text_muted().gamma_multiply(0.8)),
        );
    }

    painter.hline(plot.x_range(), plot.bottom(), axis);
    painter.vline(plot.left(), plot.y_range(), axis);
    let label_font = theme::bold(theme::CAPTION_SIZE);
    painter.text(
        egui::pos2(plot.center().x, rect.bottom() - space::XS),
        Align2::CENTER_BOTTOM,
        tr("Summon level"),
        label_font.clone(),
        color::text(),
    );
    let galley = painter.layout_no_wrap(tr("Final stat (points)").into(), label_font, color::text());
    let center = egui::pos2(rect.left() + space::LG, plot.center().y);
    let text = egui::epaint::TextShape::new(center - galley.size() * 0.5, galley, color::text())
        .with_angle_and_anchor(-std::f32::consts::FRAC_PI_2, Align2::CENTER_CENTER);
    painter.add(text);
}

#[derive(Clone, Copy)]
struct ValueScale {
    min: i64,
    max: i64,
    step: i64,
}

impl ValueScale {
    fn new(min: i32, max: i32, intervals: i64) -> Self {
        let (min, max) = (i64::from(min), i64::from(max));
        let padding = ((max - min) / 20).max(1);
        let padded_min = min - padding;
        let (min, max) = (if min >= 0 { padded_min.max(0) } else { padded_min }, max + padding);
        let target = (max - min + intervals - 1) / intervals;
        let decade = 10_i64.pow(target.ilog10());
        let step = [decade, 2 * decade, 5 * decade].into_iter().find(|&step| step >= target).unwrap_or(10 * decade);
        Self { min: min.div_euclid(step) * step, max: -(-max).div_euclid(step) * step, step }
    }

    fn ticks(self) -> impl Iterator<Item = i64> {
        (self.min..=self.max).step_by(self.step as usize)
    }

    fn y_at(self, plot: Rect, value: i64) -> f32 {
        plot.bottom() - plot.height() * ((value - self.min) as f64 / (self.max - self.min) as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::{
        Harness,
        kittest::{By, Queryable},
    };

    use super::{LevelAxis, Rect, STAT_COUNT, StatChart, ValueScale, egui, theme};

    #[test]
    fn chart_marks_evolution_boundaries_with_range_labels_and_vertical_steps() {
        let mut chart = StatChart::new();
        let mut theme_ready = false;
        let mut curve = [[0; STAT_COUNT]; 200];
        curve[49..99].fill([10; STAT_COUNT]);
        curve[99..].fill([20; STAT_COUNT]);
        let mut harness = Harness::builder().with_size([1000.0, 244.0]).build_ui(move |ui| {
            if !theme_ready {
                theme::apply(ui.ctx());
                theme_ready = true;
                return;
            }
            let mut selected = 50;
            chart.show(ui, 200.0, &curve, &mut selected, 200, &[(1, 1), (2, 50), (3, 100)], true);
        });
        harness.run_steps(3);

        let shapes = &harness.output().shapes;
        for label in ["Stage 1 · Lv. 1–49", "Stage 2 · Lv. 50–99", "Stage 3 · Lv. 100–200"] {
            assert!(
                shapes.iter().any(|shape| matches!(
                    &shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == label && !text.galley.elided
                )),
                "missing or clipped chart range label: {label}",
            );
        }
        assert!(harness.query(By::new().label("Selected · Lv. 50 · Stage 2")).is_some());
        let curve_points = shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Path(path) if path.points.len() >= 200 => Some(&path.points),
                _ => None,
            })
            .expect("stat curve is painted");
        let first = curve_points.first().unwrap().x;
        let last = curve_points.last().unwrap().x;
        for (point_index, level) in [(49, 50), (100, 100)] {
            let before = curve_points[point_index];
            let after = curve_points[point_index + 1];
            let boundary = first + (last - first) * (level - 1) as f32 / 199.0;
            assert!((before.x - boundary).abs() < 0.1);
            assert_eq!(before.x, after.x, "stat jumps vertically at level {level}");
            assert!(before.y > after.y, "stat increases at level {level}");
        }
    }

    #[test]
    fn level_axis_matches_the_active_growth_range() {
        let plot = Rect::from_min_size(egui::pos2(20.0, 0.0), egui::vec2(350.0, 100.0));
        let aurumis = LevelAxis { plot, max_level: 200 };
        assert_eq!(aurumis.x_at(1), plot.left());
        assert_eq!(aurumis.x_at(200), plot.right());
        assert_eq!(aurumis.level_at(plot.right() + 10.0), 200);
        assert_eq!(aurumis.level_at(aurumis.x_at(175)), 175);
        assert_eq!(aurumis.ticks().collect::<Vec<_>>(), [1, 25, 50, 75, 100, 125, 150, 175, 200]);

        let legacy = LevelAxis { plot, max_level: 350 };
        assert_eq!(legacy.x_at(350), plot.right());
        assert_eq!(legacy.level_at(legacy.x_at(200)), 200);
        assert_eq!(legacy.ticks().collect::<Vec<_>>(), [1, 50, 100, 150, 200, 250, 300, 350]);
    }

    #[test]
    fn chart_scale_keeps_signed_and_flat_series_inside_readable_ticks() {
        let mixed = ValueScale::new(i32::MIN, i32::MAX, 4);
        let ticks = mixed.ticks().collect::<Vec<_>>();
        assert!(ticks[0] < i64::from(i32::MIN));
        assert!(ticks[ticks.len() - 1] > i64::from(i32::MAX));
        assert!(ticks.contains(&0));

        let flat = ValueScale::new(-7, -7, 4);
        assert_eq!(flat.ticks().collect::<Vec<_>>(), [-8, -7, -6]);
    }
}
