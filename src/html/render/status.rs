//! Live progress panel rendered into the root dashboard.

use crate::OperationStatus;
use crate::util::text::escape_html;

pub fn render_live_status_panel(status: &OperationStatus) -> String {
    let (phase_label, phase_class) =
        if status.phase == "idle" { ("Idle", "pill") } else { (status.phase.as_str(), "pill ok") };

    let detail = status.detail.clone().unwrap_or_else(|| {
        if status.phase == "idle" {
            "The helper is resting between cycles.".to_string()
        } else {
            String::new()
        }
    });

    let progress = match (status.progress_current, status.progress_total) {
        (Some(current), Some(total)) if total > 0 => format!(" · {current} of {total}"),
        _ => String::new(),
    };

    format!(
        r#"<section id="live-status" class="card">
  <p class="eyebrow">Live status</p>
  <h2 style="margin-top:8px;">What this helper is doing right now</h2>
  <div class="btn-row" style="margin-top:14px;">
    <span class="{cls}" id="live-status-phase">{phase}</span>
    <span class="muted inventory-status" id="live-status-detail">{detail}{progress}</span>
  </div>
</section>"#,
        cls = phase_class,
        phase = escape_html(phase_label),
        detail = escape_html(&detail),
    )
}
