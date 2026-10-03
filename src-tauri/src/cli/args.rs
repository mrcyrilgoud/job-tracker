use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "job-tracker",
    about = "Personal local Job Tracker CLI",
    version
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        env = "JOB_TRACKER_DATA_DIR",
        help = "Path to data directory containing job-tracker.db"
    )]
    pub data_dir: Option<PathBuf>,

    #[arg(
        long,
        global = true,
        help = "Output results in JSON format (machine-readable)"
    )]
    pub json: bool,

    #[arg(
        short = 'q',
        long,
        global = true,
        help = "Suppress informative status messages"
    )]
    pub quiet: bool,

    #[arg(long, help = "Run the background jobs cycle (LaunchAgent worker mode)")]
    pub run_jobs: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[command(name = "list", alias = "ls", about = "List tracked jobs with filters")]
    List(ListArgs),

    #[command(name = "get", about = "Show full details, notes, and events for a job")]
    Get(GetArgs),

    #[command(name = "add", about = "Add a job posting by URL (with auto-scraping)")]
    Add(AddArgs),

    #[command(
        name = "update",
        about = "Update a job's status, notes, description, applied date, appeal, or favorite"
    )]
    Update(UpdateArgs),

    #[command(name = "note", about = "Quickly append a note and event to a job")]
    Note(NoteArgs),

    #[command(
        name = "description",
        alias = "desc",
        about = "View or edit the job description"
    )]
    Description(DescriptionArgs),

    #[command(
        name = "stats",
        alias = "summary",
        about = "Show pipeline metrics & summary"
    )]
    Stats(StatsArgs),

    #[command(name = "watches", about = "Manage and triage ATS board watches")]
    Watches(WatchesArgs),

    #[command(
        name = "sync",
        about = "Run a full sync cycle (postings check, watches, careers, CSV)"
    )]
    Sync(SyncArgs),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(
        long,
        help = "Filter by status (e.g. wishlist, applied, interviewing, offer, rejected, withdrawn, closed)"
    )]
    pub status: Option<String>,

    #[arg(short = 'c', long, help = "Filter by company name or ID")]
    pub company: Option<String>,

    #[arg(
        short = 's',
        long,
        help = "Search text across title, company, notes, description, URL"
    )]
    pub search: Option<String>,

    #[arg(short = 'l', long, help = "Filter by location")]
    pub location: Option<String>,

    #[arg(short = 'f', long, help = "Show only favorite jobs")]
    pub favorites: bool,

    #[arg(long, help = "Include or show archived jobs")]
    pub archived: bool,

    #[arg(short = 'n', long, help = "Maximum number of jobs to return")]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct GetArgs {
    #[arg(help = "Job ID (full or prefix) or Job URL")]
    pub target: String,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    #[arg(help = "Posting URL to track (e.g. Greenhouse, Lever, Ashby, or company careers URL)")]
    pub url: String,

    #[arg(short = 't', long, help = "Job title (auto-detected if omitted)")]
    pub title: Option<String>,

    #[arg(short = 'c', long, help = "Company name (auto-detected if omitted)")]
    pub company: Option<String>,

    #[arg(
        long,
        default_value = "wishlist",
        help = "Initial status (wishlist, applied, interviewing, offer, etc.)"
    )]
    pub status: String,

    #[arg(
        long,
        help = "Date applied (e.g. 'today' or '2026-08-23'). Sets status to 'applied' if status was wishlist"
    )]
    pub applied_at: Option<String>,

    #[arg(long, help = "Initial notes or referral info")]
    pub notes: Option<String>,

    #[arg(long, help = "Initial job description")]
    pub description: Option<String>,

    #[arg(short = 'l', long, help = "Location (e.g. Remote, San Francisco, CA)")]
    pub location: Option<String>,

    #[arg(short = 'f', long, help = "Mark as favorite")]
    pub favorite: bool,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    #[arg(help = "Job ID (full or prefix) or Job URL")]
    pub target: String,

    #[arg(
        long,
        help = "Update pipeline status (wishlist, applied, interviewing, offer, rejected, withdrawn, closed)"
    )]
    pub status: Option<String>,

    #[arg(
        long,
        help = "Update applied date (e.g. 'today', '2026-08-23', or ISO timestamp)"
    )]
    pub applied_at: Option<String>,

    #[arg(long, help = "Replace full notes")]
    pub notes: Option<String>,

    #[arg(long, help = "Append a dated note without replacing existing notes")]
    pub append_note: Option<String>,

    #[arg(
        long,
        conflicts_with = "clear_description",
        help = "Replace job description"
    )]
    pub description: Option<String>,

    #[arg(long, conflicts_with = "description", help = "Clear job description")]
    pub clear_description: bool,

    #[arg(short = 't', long, help = "Update job title")]
    pub title: Option<String>,

    #[arg(short = 'c', long, help = "Update company name")]
    pub company: Option<String>,

    #[arg(short = 'l', long, help = "Update location")]
    pub location: Option<String>,

    #[arg(
        short = 'f',
        long,
        conflicts_with = "unfavorite",
        help = "Mark as favorite"
    )]
    pub favorite: bool,

    #[arg(long, conflicts_with = "favorite", help = "Unmark favorite")]
    pub unfavorite: bool,

    #[arg(long, conflicts_with = "unarchive", help = "Archive this job")]
    pub archive: bool,

    #[arg(long, conflicts_with = "archive", help = "Unarchive this job")]
    pub unarchive: bool,

    #[arg(
        long,
        conflicts_with = "clear_appeal",
        value_parser = clap::value_parser!(i64).range(1..=5),
        help = "Overall appeal from 1 (least appealing) to 5 (most appealing)"
    )]
    pub appeal: Option<i64>,

    #[arg(
        long,
        conflicts_with = "appeal",
        help = "Clear the appeal score (leave the job unscored)"
    )]
    pub clear_appeal: bool,
}

#[derive(Debug, Args)]
pub struct NoteArgs {
    #[arg(help = "Job ID (full or prefix) or Job URL")]
    pub target: String,

    #[arg(help = "Note text to append")]
    pub note: String,
}

#[derive(Debug, Args)]
pub struct DescriptionArgs {
    #[arg(help = "Job ID (full or prefix) or Job URL")]
    pub target: String,

    #[arg(
        help = "New description text (if omitted and no flags provided, opens interactive $EDITOR)"
    )]
    pub description: Option<String>,

    #[arg(short = 'F', long = "file", help = "Read description text from a file")]
    pub file: Option<PathBuf>,

    #[arg(long = "stdin", help = "Read description text from standard input")]
    pub stdin: bool,

    #[arg(
        short = 's',
        long = "show",
        help = "Print the current description without editing"
    )]
    pub show: bool,

    #[arg(
        long = "clear",
        conflicts_with = "show",
        help = "Clear the job description"
    )]
    pub clear: bool,
}

#[derive(Debug, Args)]
pub struct StatsArgs {}

#[derive(Debug, Args)]
pub struct WatchesArgs {
    #[command(subcommand)]
    pub command: Option<WatchCommands>,
}

#[derive(Debug, Subcommand)]
pub enum WatchCommands {
    #[command(name = "list", alias = "ls", about = "List discovered watch postings")]
    List(WatchListArgs),

    #[command(
        name = "save",
        about = "Save a discovered watch posting to your tracked wishlist"
    )]
    Save(WatchTargetArgs),

    #[command(name = "dismiss", about = "Dismiss a discovered watch posting")]
    Dismiss(WatchTargetArgs),

    #[command(
        name = "reset",
        about = "Reset a dismissed watch posting back to open review"
    )]
    Reset(WatchTargetArgs),

    #[command(name = "sync", about = "Trigger an on-demand sync of all ATS watches")]
    Sync,
}

#[derive(Debug, Args)]
pub struct WatchListArgs {
    #[arg(long, help = "Show only new/pending review watch positions")]
    pub new_only: bool,

    #[arg(long, help = "Show only dismissed watch positions")]
    pub dismissed: bool,

    #[arg(long, help = "Show all watch positions")]
    pub all: bool,

    #[arg(long, help = "Filter by provider (greenhouse, lever, ashby)")]
    pub provider: Option<String>,

    #[arg(short = 'c', long, help = "Filter by company name or ID")]
    pub company: Option<String>,

    #[arg(short = 'n', long, help = "Limit count")]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct WatchTargetArgs {
    #[arg(help = "Job ID (from watch discoveries)")]
    pub target: String,
}

#[derive(Debug, Args)]
pub struct SyncArgs {}
