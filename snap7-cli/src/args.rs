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
    /// Read bytes from a data block
    Read(ReadArgs),
    /// Write hex bytes to a data block
    Write(WriteArgs),
    /// Read/write typed tags (e.g. DB1,REAL4)
    Tag(TagArgs),
    /// Block operations: list, info, upload
    Block(BlockArgs),
    /// Query SZL (system status list)
    Szl(SzlArgs),
    /// Quick connectivity test
    Diag,
    /// Watch a data block for changes
    Watch(WatchArgs),
    /// PLC control: stop, hotstart, coldstart, status
    PlcControl(PlcControlArgs),
    /// PLC information: order-code, cpu-info, cp-info
    Info(InfoArgs),
    /// Set or clear the session password
    Password(PasswordArgs),
    #[cfg(feature = "opcua")]
    /// Start the OPC-UA gateway server
    Serve(ServeArgs),
}

// --- Read / Write ---

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

// --- Tag ---

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

// --- Block ---

#[derive(clap::Args, Debug)]
pub struct BlockArgs {
    #[command(subcommand)]
    pub action: BlockAction,
}

#[derive(Subcommand, Debug)]
pub enum BlockAction {
    /// List all blocks grouped by type
    List,
    /// Show detailed info about a block
    Info {
        #[arg(long)]
        r#type: String,
        #[arg(long)]
        number: u16,
    },
    /// Upload a block and save to file (Diagra format)
    Upload {
        #[arg(long)]
        r#type: String,
        #[arg(long)]
        number: u16,
        #[arg(long)]
        out: String,
    },
}

// --- SZL ---

#[derive(clap::Args, Debug)]
pub struct SzlArgs {
    #[arg(long, value_parser = clap::value_parser!(u16).range(0..))]
    pub id: u16,
    #[arg(long, default_value = "0")]
    pub index: u16,
}

// --- Watch ---

#[derive(clap::Args, Debug)]
pub struct WatchArgs {
    #[arg(long)]
    pub db: u16,
    #[arg(long, default_value = "0")]
    pub offset: u32,
    #[arg(long)]
    pub size: u16,
    #[arg(long, default_value = "1000")]
    pub interval_ms: u64,
    #[arg(long, default_value_t = false)]
    pub changes_only: bool,
}

// --- PlcControl ---

#[derive(clap::Args, Debug)]
pub struct PlcControlArgs {
    #[command(subcommand)]
    pub action: PlcAction,
}

#[derive(Subcommand, Debug)]
pub enum PlcAction {
    /// Stop the PLC
    Stop,
    /// Warm restart (retains DBs)
    HotStart,
    /// Cold restart (clears DBs)
    ColdStart,
    /// Read PLC status (RUN / STOP)
    Status,
}

// --- Info ---

#[derive(clap::Args, Debug)]
pub struct InfoArgs {
    #[command(subcommand)]
    pub action: InfoAction,
}

#[derive(Subcommand, Debug)]
pub enum InfoAction {
    /// Read PLC order code (e.g. 6ES7 317-2EK14-0AB0)
    OrderCode,
    /// Read CPU detailed info
    CpuInfo,
    /// Read communications processor info
    CpInfo,
    /// Read module list
    ModuleList,
}

// --- Password ---

#[derive(clap::Args, Debug)]
pub struct PasswordArgs {
    #[command(subcommand)]
    pub action: PasswordAction,
}

#[derive(Subcommand, Debug)]
pub enum PasswordAction {
    /// Set session password
    Set { password: String },
    /// Clear session password
    Clear,
}

// --- Serve ---

#[cfg(feature = "opcua")]
#[derive(clap::Args, Debug)]
pub struct ServeArgs {
    #[arg(short, long, default_value = "gateway.toml")]
    pub config: std::path::PathBuf,
}
