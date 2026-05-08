// Adapted from Tokscale bar_chart.rs (MIT)
// Stacked bar chart rendering directly to frame buffer

use ratatui::prelude::*;
use super::widgets::format_tokens;

const BLOCKS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const MONTH_NAMES: &[&str] = &["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];

pub struct ModelSegment {
    pub model_id: String,
    pub tokens: u64,
    pub color: Color,
}

pub struct StackedBarData {
    pub date: String,
    pub models: Vec<ModelSegment>,
    pub total: u64,
}

pub fn render_stacked_bar_chart(frame: &mut Frame, area: Rect, data: &[StackedBarData], muted: Color, highlight: Color) {
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
        if x < area.x + area.width {
            buf[(x, area.y)].set_char(ch).set_style(Style::default().add_modifier(Modifier::BOLD));
        }
    }

    // Bars
    for row_from_bottom in (0..chart_height).rev() {
        let row_index = chart_height - 1 - row_from_bottom;
        let y = area.y + 1 + row_index as u16;

        // Y-axis label at top
        if row_from_bottom == chart_height - 1 {
            let label = format!("{:>6}│", format_tokens(max_value as u64));
            for (i, ch) in label.chars().enumerate() {
                let x = area.x + i as u16;
                if x < area.x + y_label_width { buf[(x,y)].set_char(ch).set_fg(muted); }
            }
        }

        let mut x_pos = area.x + y_label_width;
        for (bar_index, bar_data) in data.iter().enumerate() {
            let bar_width = get_bar_width(bar_index);
            let row_threshold = ((row_from_bottom + 1) as f64 / chart_height as f64) * max_value;
            let (ch, fg) = bar_cell(bar_data, row_threshold, max_value, muted, highlight);
            for _ in 0..bar_width {
                if x_pos < area.x + area.width { buf[(x_pos,y)].set_char(ch).set_fg(fg); x_pos += 1; }
            }
        }
    }

    // X-axis line
    let axis_y = area.y + 1 + chart_height as u16;
    if axis_y < area.y + area.height {
        let zero_label = format!("{:>6}│", "0");
        for (i,ch) in zero_label.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.x + y_label_width { buf[(x,axis_y)].set_char(ch).set_fg(muted); }
        }
        for x in (area.x + y_label_width)..(area.x + area.width) {
            buf[(x,axis_y)].set_char('─').set_fg(muted);
        }
    }

    // X-axis date labels
    let label_y = axis_y + 1;
    if label_y < area.y + area.height && !data.is_empty() {
        let label_interval = (bar_count / 3).max(1);
        for i in (0..bar_count).step_by(label_interval) {
            let ds = &data[i].date;
            let label = if let Some((m,d)) = ds.split_once('/') {
                if let (Ok(month),Ok(_day)) = (m.parse::<usize>(),d.parse::<u32>()) {
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

fn bar_cell(bar: &StackedBarData, row_threshold: f64, _max_value: f64, muted: Color, fallback: Color) -> (char, Color) {
    let total = bar.total as f64;
    if total <= 0.0 { return (' ', muted); }
    if bar.models.is_empty() { return (' ', muted); }

    // Find which model dominates this vertical slice
    let mut sorted: Vec<&ModelSegment> = bar.models.iter().collect();
    sorted.sort_by(|a,b| a.model_id.cmp(&b.model_id));

    let mut cumulative: f64 = 0.0;
    let mut best_color = sorted.first().map(|m| m.color).unwrap_or(fallback);

    for seg in &sorted {
        let seg_start = cumulative;
        let seg_end = cumulative + seg.tokens as f64;
        cumulative += seg.tokens as f64;

        if row_threshold > seg_start && row_threshold <= seg_end {
            best_color = seg.color;
            break;
        }
    }

    if total >= row_threshold { return (BLOCKS[8], best_color); }

    let ratio = (total / row_threshold).clamp(0.0, 1.0);
    let idx = (ratio * 8.0).floor().clamp(1.0, 8.0) as usize;
    (BLOCKS[idx], best_color)
}
