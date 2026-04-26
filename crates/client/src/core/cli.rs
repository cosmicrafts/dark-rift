use clap::Parser;
use bevy::prelude::Resource;

#[derive(Parser, Resource, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Start as an observer
    #[arg(short, long)]
    pub observer: bool,

    /// Play a replay file locally
    #[arg(short, long)]
    pub replay: Option<String>,

    /// Run as a headless bot for network stress testing
    #[arg(short, long)]
    pub bot: bool,

    /// Profile config file inside `assets/bots/` (e.g. `tortuga.ron`)
    #[arg(short, long)]
    pub profile: Option<String>,
}
