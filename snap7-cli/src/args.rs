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
    /// Block operations: list, info, upload, download, create, compare
    Block(BlockArgs),
    /// Memory and program management: reset, format, batch-upload, compare
    Program(ProgramArgs),
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
    /// Read or set the PLC real-time clock
    Clock(ClockArgs),
    /// Force I/Q bits or bytes to a value (bypasses normal scan cycle)
    Force(ForceArgs),
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
    /// S7 area: db (default), merker, pi, pa, inst, local
    #[arg(long, default_value = "db")]
    pub area: AreaArg,
}

#[derive(clap::Args, Debug)]
pub struct WriteArgs {
    #[arg(long)]
    pub db: u16,
    #[arg(long, default_value = "0")]
    pub offset: u32,
    #[arg(long, help = "Hex bytes, e.g. DEADBEEF")]
    pub data: String,
    /// S7 area: db (default), merker, pi, pa, inst, local
    #[arg(long, default_value = "db")]
    pub area: AreaArg,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum AreaArg {
    Db,
    Merker,
    Pi,
    Pa,
    Inst,
    Local,
}

impl std::fmt::Display for AreaArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AreaArg::Db => write!(f, "db"),
            AreaArg::Merker => write!(f, "merker"),
            AreaArg::Pi => write!(f, "pi"),
            AreaArg::Pa => write!(f, "pa"),
            AreaArg::Inst => write!(f, "inst"),
            AreaArg::Local => write!(f, "local"),
        }
    }
}

impl From<AreaArg> for snap7_client::proto::s7::header::Area {
    fn from(a: AreaArg) -> Self {
        match a {
            AreaArg::Db => snap7_client::proto::s7::header::Area::DataBlock,
            AreaArg::Merker => snap7_client::proto::s7::header::Area::Marker,
            AreaArg::Pi => snap7_client::proto::s7::header::Area::ProcessInput,
            AreaArg::Pa => snap7_client::proto::s7::header::Area::ProcessOutput,
            AreaArg::Inst => snap7_client::proto::s7::header::Area::InstanceDB,
            AreaArg::Local => snap7_client::proto::s7::header::Area::LocalData,
        }
    }
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
    /// List all block numbers of a given type
    Numbers {
        #[arg(long)]
        r#type: String,
    },
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
    /// Download a block from file to PLC
    Download {
        #[arg(long)]
        r#type: String,
        #[arg(long)]
        number: u16,
        #[arg(long)]
        file: String,
    },
    /// Create a new empty DB on the PLC
    CreateDb {
        #[arg(long)]
        number: u16,
        /// Size in bytes
        #[arg(long, default_value = "256")]
        size: u16,
        /// Author attribute (max 8 chars)
        #[arg(long)]
        author: Option<String>,
        /// Family attribute (max 8 chars)
        #[arg(long)]
        family: Option<String>,
        /// Name attribute (max 8 chars)
        #[arg(long)]
        name: Option<String>,
        /// Version as major.minor (e.g. 1.0)
        #[arg(long)]
        version: Option<String>,
    },
    /// Set block attributes (author, family, name, version) — re-downloads block
    SetAttrs {
        #[arg(long)]
        r#type: String,
        #[arg(long)]
        number: u16,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        family: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
}

// --- SZL ---

/// Parse a u16 that accepts decimal (e.g. 17) or hex with 0x prefix (e.g. 0x11).
fn parse_hex_or_decimal(s: &str) -> Result<u16, String> {
    let s = s.trim();
    if s.starts_with("0x") || s.starts_with("0X") {
        u16::from_str_radix(&s[2..], 16).map_err(|e| format!("invalid hex value: {e}"))
    } else {
        s.parse::<u16>().map_err(|e| format!("invalid decimal value: {e}"))
    }
}

#[derive(clap::Args, Debug)]
pub struct SzlArgs {
    #[arg(long, value_parser = parse_hex_or_decimal)]
    pub id: u16,
    #[arg(long, default_value = "0", value_parser = parse_hex_or_decimal)]
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

// --- Clock ---

#[derive(clap::Args, Debug)]
pub struct ClockArgs {
    #[command(subcommand)]
    pub action: ClockAction,
}

#[derive(Subcommand, Debug)]
pub enum ClockAction {
    /// Read PLC real-time clock
    Read,
    /// Set PLC clock to a specific date/time (format: YYYY-MM-DDTHH:MM:SS)
    Set {
        datetime: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Sync PLC clock to system time
    Sync {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
}

// --- Program ---

#[derive(clap::Args, Debug)]
pub struct ProgramArgs {
    #[command(subcommand)]
    pub action: ProgramAction,
}

#[derive(Subcommand, Debug)]
pub enum ProgramAction {
    /// Memory reset — clears work memory (PLC must be in STOP)
    MemReset {
        #[arg(long)]
        force: bool,
    },
    /// Overall reset — formats entire PLC memory (PLC must be in STOP)
    Format {
        #[arg(long)]
        force: bool,
    },
    /// Upload all blocks of given types to a directory
    BatchUpload {
        /// Block types: comma-separated list of OB,FB,FC,DB,SFC,SFB,UDT (default: all)
        #[arg(long, default_value = "OB,FB,FC,DB")]
        types: String,
        /// Output directory
        #[arg(long, default_value = ".")]
        out: String,
        /// Use full-upload (includes MC7 code); default is header-only upload
        #[arg(long, default_value_t = false)]
        full: bool,
    },
    /// Compare local block files to PLC — reports CRC mismatches
    Compare {
        /// Directory containing local block files (named <TYPE><NUM>.bin)
        #[arg(long)]
        dir: String,
        /// Also report blocks that exist on PLC but not locally
        #[arg(long, default_value_t = false)]
        plc_only: bool,
    },
}

// --- Force ---

#[derive(clap::Args, Debug)]
pub struct ForceArgs {
    #[command(subcommand)]
    pub action: ForceAction,
}

#[derive(Subcommand, Debug)]
pub enum ForceAction {
    /// Set a bit or byte: I0.3=1, Q1.0=0, IB2=0xFF, QB0=0x00
    Set {
        /// Address: I<byte>.<bit>, Q<byte>.<bit>, IB<byte>, QB<byte>
        address: String,
        /// Value: 0 or 1 for bits; 0–255 or 0x00–0xFF for bytes
        value: String,
    },
    /// Cancel force by writing 0 to address: IB0, QB1
    Cancel {
        /// Address: IB<byte> or QB<byte>
        address: String,
    },
    /// List forced variables (SZL 0x0025)
    List,
}

// --- Serve ---

#[cfg(feature = "opcua")]
#[derive(clap::Args, Debug)]
pub struct ServeArgs {
    #[arg(short, long, default_value = "gateway.toml")]
    pub config: std::path::PathBuf,
}
