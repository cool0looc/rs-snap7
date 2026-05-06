use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "snap7", about = "Siemens S7 PLC communication tool")]
pub struct Cli {
    /// PLC host address (required for most commands)
    #[arg(short = 'H', long, required = false)]
    pub host: Option<String>,

    #[arg(short = 'p', long, default_value = "102")]
    pub port: u16,

    #[arg(short = 'r', long, default_value = "0")]
    pub rack: u8,

    #[arg(short = 's', long, default_value = "1")]
    pub slot: u8,

    #[arg(short = 'f', long, default_value = "text")]
    pub format: OutputFormat,

    #[arg(short = 't', long, default_value = "5")]
    pub timeout_secs: u64,

    /// Use TLS transport (S7CommPlus encrypted mode)
    #[arg(long, default_value_t = false)]
    pub tls: bool,

    /// Path to PEM CA certificate for TLS verification (uses system roots by default)
    #[arg(long, value_name = "PATH")]
    pub tls_ca: Option<std::path::PathBuf>,

    /// Use UDP transport
    #[arg(long, default_value_t = false)]
    pub udp: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum OutputFormat {
    Text,
    Json,
    Csv,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Read(ReadArgs),
    Write(WriteArgs),
    Tag(TagArgs),
    Block(BlockArgs),
    Szl(SzlArgs),
    Diag,
    Watch(WatchArgs),
    #[cfg(feature = "opcua")]
    Serve(ServeArgs),
}

#[derive(clap::Args, Debug)]
pub struct ReadArgs {
    #[arg(long)]
    pub db: u16,
    #[arg(long, default_value = "0")]
    pub offset: u32,
    #[arg(long)]
    pub size: u16,
}

#[derive(clap::Args, Debug)]
pub struct WriteArgs {
    #[arg(long)]
    pub db: u16,
    #[arg(long, default_value = "0")]
    pub offset: u32,
    #[arg(long, help = "Hex bytes, e.g. DEADBEEF")]
    pub data: String,
}

#[derive(clap::Args, Debug)]
pub struct TagArgs {
    #[command(subcommand)]
    pub action: TagAction,
}

#[derive(Subcommand, Debug)]
pub enum TagAction {
    Read { tag: String },
    Write { tag: String, value: String },
}

#[derive(clap::Args, Debug)]
pub struct BlockArgs {
    #[command(subcommand)]
    pub action: BlockAction,
}

#[derive(Subcommand, Debug)]
pub enum BlockAction {
    Upload {
        #[arg(long)]
        r#type: String,
        #[arg(long)]
        number: u16,
        #[arg(long)]
        out: String,
    },
    List,
}

#[derive(clap::Args, Debug)]
pub struct SzlArgs {
    #[arg(long, value_parser = clap::value_parser!(u16).range(0..))]
    pub id: u16,
    #[arg(long, default_value = "0")]
    pub index: u16,
}

#[derive(clap::Args, Debug)]
pub struct WatchArgs {
    /// Data block number
    #[arg(long)]
    pub db: u16,

    /// Byte offset within the data block
    #[arg(long, default_value = "0")]
    pub offset: u32,

    /// Number of bytes to read
    #[arg(long)]
    pub size: u16,

    /// Poll interval in milliseconds
    #[arg(long, default_value = "1000")]
    pub interval_ms: u64,

    /// Only print when the value changes
    #[arg(long, default_value_t = false)]
    pub changes_only: bool,
}

#[cfg(feature = "opcua")]
#[derive(clap::Args, Debug)]
pub struct ServeArgs {
    /// Path to gateway TOML config file
    #[arg(short, long, default_value = "gateway.toml")]
    pub config: std::path::PathBuf,
}
