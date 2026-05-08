// Adapted from Tokscale bar_chart.rs (MIT) — direct buffer-level stacked bar chart

use ratatui::prelude::*;
use super::widgets::format_tokens;

const BLOCKS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const MONTH_NAMES: &[&str] = &["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];

pub struct ModelSegment { pub model_id: String, pub tokens: u64, pub color: Color }
pub struct StackedBarData { pub date: String, pub models: Vec<ModelSegment>, pub total: u64 }

pub fn render_stacked_bar_chart(frame: &mut Frame, area: Rect, data: &[StackedBarData], muted: Color) {
    if data.is_empty() { return; }
    let y_label_width: u16 = 7;
    let chart_width = area.width.saturating_sub(y_label_width) as usize;
    let chart_height = area.height.saturating_sub(3) as usize;
    if chart_width == 0 || chart_height == 0 { return; }

    let max_value = data.iter().map(|d| d.total as f64).fold(0.0_f64, f64::max).max(1.0);
    let buf = frame.buffer_mut();
    let bar_count = data.len();

    let get_bar_width = |i: usize| -> usize {
        if bar_count == 0 { return 1; }
        let start = (i * chart_width) / bar_count;
        let end = ((i + 1) * chart_width) / bar_count;
        (end - start).max(1)
    };

    // Title
    for (i, ch) in "Tokens per Day".chars().enumerate() {
        let x = area.x + y_label_width + i as u16;
        if x < area.x + area.width { buf[(x, area.y)].set_char(ch).set_style(Style::default().add_modifier(Modifier::BOLD)); }
    }

    // Bars
    for row_from_bottom in (0..chart_height).rev() {
        let y = area.y + 1 + (chart_height - 1 - row_from_bottom) as u16;

        // Y-axis label (top only)
        if row_from_bottom == chart_height - 1 {
            let label = format!("{:>6}│", format_tokens(max_value as u64));
            for (i, ch) in label.chars().enumerate() {
                let x = area.x + i as u16;
                if x < area.x + y_label_width { buf[(x,y)].set_char(ch).set_fg(muted); }
            }
        }

        let row_threshold = ((row_from_bottom + 1) as f64 / chart_height as f64) * max_value;
        let prev_threshold = (row_from_bottom as f64 / chart_height as f64) * max_value;
        let threshold_diff = row_threshold - prev_threshold;

        let mut x_pos = area.x + y_label_width;
        for (bar_index, bar_data) in data.iter().enumerate() {
            let bar_width = get_bar_width(bar_index);
            let (ch, fg) = get_bar_cell(bar_data, row_threshold, prev_threshold, threshold_diff, muted);
            for _ in 0..bar_width {
                if x_pos < area.x + area.width { buf[(x_pos,y)].set_char(ch).set_fg(fg); x_pos += 1; }
            }
        }
    }

    // X-axis
    let axis_y = area.y + 1 + chart_height as u16;
    if axis_y < area.y + area.height {
        let zero_label = format!("{:>6}│", "0");
        for (i,ch) in zero_label.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.x + y_label_width { buf[(x,axis_y)].set_char(ch).set_fg(muted); }
        }
        for x in (area.x + y_label_width)..(area.x + area.width) { buf[(x,axis_y)].set_char('─').set_fg(muted); }
    }

    // X-axis labels
    let label_y = axis_y + 1;
    if label_y < area.y + area.height && !data.is_empty() {
        let label_interval = (bar_count / 3).max(1);
        for i in (0..bar_count).step_by(label_interval) {
            let ds = &data[i].date;
            let label = if let Some((m,d)) = ds.split_once('/') {
                if let (Ok(month),Ok(_)) = (m.parse::<usize>(),d.parse::<u32>()) {
                    if (1..=12).contains(&month) { format!("{} {}", MONTH_NAMES[month-1], d) } else { ds.clone() }
                } else { ds.clone() }
            } else { ds.clone() };
            let bar_start_x = (i * chart_width) / bar_count;
            let label_x = area.x + y_label_width + bar_start_x as u16;
            for (j,ch) in label.chars().enumerate() {
                let x = label_x + j as u16;
                if x < area.x + area.width { buf[(x,label_y)].set_char(ch).set_fg(muted); }
            }
        }
    }
}

fn get_bar_cell(bar: &StackedBarData, row_threshold: f64, prev_threshold: f64, threshold_diff: f64, muted: Color) -> (char, Color) {
    let total = bar.total as f64;
    if total <= prev_threshold { return (' ', muted); }
    if bar.models.is_empty() { return (' ', muted); }

    let mut sorted: Vec<&ModelSegment> = bar.models.iter().collect();
    sorted.sort_by(|a,b| a.model_id.cmp(&b.model_id));

    let row_start = prev_threshold;
    let row_end = row_threshold;
    let mut cumulative: f64 = 0.0;
    let mut max_overlap: f64 = 0.0;
    let mut best_color = sorted.first().map(|m| m.color).unwrap_or(muted);

    for model in &sorted {
        let m_start = cumulative;
        let m_end = cumulative + model.tokens as f64;
        cumulative += model.tokens as f64;
        let overlap = (m_end.min(row_end) - m_start.max(row_start)).max(0.0);
        if overlap > max_overlap { max_overlap = overlap; best_color = model.color; }
    }

    if total >= row_threshold { return (BLOCKS[8], best_color); }
    let ratio = if threshold_diff > 0.0 { (total - prev_threshold) / threshold_diff } else { 1.0 };
    let idx = (ratio * 8.0).floor().clamp(1.0, 8.0) as usize;
    (BLOCKS[idx], best_color)
}
