use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table};
use serde_json::Value;
use std::collections::HashMap;

use crate::error::AppError;
use crate::models::{JobDetail, JobListItem, WeeklyActivity};

pub fn print_json<T: serde::Serialize>(data: &T) {
    if let Ok(json) = serde_json::to_string_pretty(data) {
        println!("{json}");
    }
}

pub fn print_raw_json(data: &Value) {
    if let Ok(json) = serde_json::to_string_pretty(data) {
        println!("{json}");
    }
}

/// Print the one-object machine-readable error envelope used by `--json`.
pub fn print_json_error(error: &AppError) {
    let parts = error.code_parts();
    let payload = serde_json::json!({
        "ok": false,
        "error": parts,
    });
    println!("{}", serde_json::to_string(&payload).unwrap_or_else(|_| {
        "{\"ok\":false,\"error\":{\"code\":\"error\",\"category\":\"serialization\",\"message\":\"failed to serialize error\"}}".into()
    }));
}

pub fn status_color(status: &str) -> Color {
    match status.to_lowercase().as_str() {
        "wishlist" => Color::Magenta,
        "applied" => Color::Cyan,
        "interviewing" => Color::Yellow,
        "offer" => Color::Green,
        "rejected" | "withdrawn" | "closed" => Color::DarkGrey,
        _ => Color::White,
    }
}

pub fn format_jobs_table(jobs: &[JobListItem]) {
    if jobs.is_empty() {
        println!("No jobs found.");
        return;
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("ID").add_attribute(Attribute::Bold),
            Cell::new("COMPANY").add_attribute(Attribute::Bold),
            Cell::new("TITLE").add_attribute(Attribute::Bold),
            Cell::new("STATUS").add_attribute(Attribute::Bold),
            Cell::new("APPLIED").add_attribute(Attribute::Bold),
            Cell::new("LOCATION").add_attribute(Attribute::Bold),
            Cell::new("APPEAL").add_attribute(Attribute::Bold),
            Cell::new("FAV").add_attribute(Attribute::Bold),
        ]);

    for item in jobs {
        let job = &item.job;
        let id_short = if job.id.len() > 8 {
            &job.id[..8]
        } else {
            &job.id
        };
        let status_c = status_color(&job.status);
        let fav = if job.is_favorite { "★" } else { "" };
        let applied = job.applied_at.as_deref().unwrap_or("-");
        let location = job.location.as_deref().unwrap_or("-");
        let appeal = job
            .appeal
            .map(|score| score.to_string())
            .unwrap_or_else(|| "—".to_string());

        table.add_row(vec![
            Cell::new(id_short).fg(Color::DarkGrey),
            Cell::new(&item.company_name).add_attribute(Attribute::Bold),
            Cell::new(&job.title),
            Cell::new(&job.status)
                .fg(status_c)
                .add_attribute(Attribute::Bold),
            Cell::new(applied),
            Cell::new(location),
            Cell::new(appeal),
            Cell::new(fav).fg(Color::Yellow),
        ]);
    }

    println!("{table}");
    println!("Showing {} job(s).", jobs.len());
}

pub fn format_job_detail(detail: &JobDetail) {
    let job = &detail.job;
    let company = &detail.company;

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  {} — {}", job.title, company.name);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  ID:          {}", job.id);
    println!(
        "  Status:      {} (Favorite: {})",
        job.status.to_uppercase(),
        if job.is_favorite { "Yes ★" } else { "No" }
    );
    if let Some(applied) = &job.applied_at {
        println!("  Applied:     {}", applied);
    }
    if let Some(location) = &job.location {
        println!("  Location:    {}", location);
    }
    println!(
        "  Appeal:      {}",
        match job.appeal {
            Some(score) => format!("{score} (1-5, 5 = most appealing)"),
            None => "— (1-5, 5 = most appealing)".to_string(),
        }
    );
    println!("  Posting URL: {}", job.url);
    if let Some(careers) = &company.careers_url {
        println!("  Careers:     {}", careers);
    }
    println!("  Freshness:   {}", job.posting_state);

    if let Some(desc) = &job.description {
        if !desc.trim().is_empty() {
            println!("\n  Description:\n  {}", desc.replace('\n', "\n  "));
        }
    }

    if let Some(notes) = &job.notes {
        if !notes.trim().is_empty() {
            println!("\n  Notes:\n  {}", notes.replace('\n', "\n  "));
        }
    }

    if !detail.events.is_empty() {
        println!("\n  Timeline History ({} events):", detail.events.len());
        for ev in &detail.events {
            let note_str = ev.note.as_deref().unwrap_or("");
            println!(
                "    • [{}] {}{}",
                &ev.occurred_at[..10.min(ev.occurred_at.len())],
                ev.event_type,
                if note_str.is_empty() {
                    String::new()
                } else {
                    format!(": {}", note_str)
                }
            );
        }
    }

    if !detail.attached.is_empty() {
        println!("\n  Attached Documents:");
        for doc in &detail.attached {
            println!(
                "    📎 {} ({}, {} bytes)",
                doc.document.original_filename, doc.attachment.kind, doc.document.size_bytes
            );
        }
    }
    println!();
}

pub fn format_stats(counts: &HashMap<String, i64>, weekly: &WeeklyActivity) {
    println!();
    println!("📊 Job Tracker Pipeline Summary");
    println!("────────────────────────────────────────");
    let statuses = [
        "wishlist",
        "applied",
        "interviewing",
        "offer",
        "rejected",
        "withdrawn",
        "closed",
    ];

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_header(vec![
            Cell::new("STATUS").add_attribute(Attribute::Bold),
            Cell::new("COUNT").add_attribute(Attribute::Bold),
        ]);

    for st in statuses {
        let count = counts.get(st).copied().unwrap_or(0);
        table.add_row(vec![
            Cell::new(st)
                .fg(status_color(st))
                .add_attribute(Attribute::Bold),
            Cell::new(count.to_string()),
        ]);
    }
    println!("{table}");

    println!("\n📅 Past 7 Days Activity (Total: {})", weekly.total);
    for day in &weekly.days {
        let marker = if day.is_today { " (Today)" } else { "" };
        let bar = "█".repeat(day.count.min(20) as usize);
        println!("  {:5} : {:2}  {}{}", day.label, day.count, bar, marker);
    }
    println!();
}

pub fn format_watch_positions(positions: &[JobListItem]) {
    if positions.is_empty() {
        println!("No watch positions found.");
        return;
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("ID").add_attribute(Attribute::Bold),
            Cell::new("COMPANY").add_attribute(Attribute::Bold),
            Cell::new("TITLE").add_attribute(Attribute::Bold),
            Cell::new("PROVIDER").add_attribute(Attribute::Bold),
            Cell::new("STATUS").add_attribute(Attribute::Bold),
            Cell::new("LOCATION").add_attribute(Attribute::Bold),
        ]);

    for item in positions {
        let job = &item.job;
        let id_short = if job.id.len() > 8 {
            &job.id[..8]
        } else {
            &job.id
        };
        let disp = job.watch_disposition.as_deref().unwrap_or("new");
        let location = job.location.as_deref().unwrap_or("-");

        table.add_row(vec![
            Cell::new(id_short).fg(Color::DarkGrey),
            Cell::new(&item.company_name).add_attribute(Attribute::Bold),
            Cell::new(&job.title),
            Cell::new(&job.source).fg(Color::Cyan),
            Cell::new(disp),
            Cell::new(location),
        ]);
    }

    println!("{table}");
    println!("Showing {} watch position(s).", positions.len());
}
