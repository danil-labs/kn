use clap::{Args, Parser, Subcommand};
use kn_core::{
    error::{Envelope, Error, Result},
    git::version,
    ops, sessions,
    workspace::{self, Workspace},
};
use serde_json::{Value, json};
use std::io::Write;

#[derive(Parser)]
#[command(
    version,
    about = "Versiones y sesiones locales para carpetas de documentos"
)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    /// Ejecutar en otra carpeta sin cambiar el directorio del proceso llamador
    #[arg(short = 'C', global = true)]
    directory: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// Consultas de rutas e identidad para herramientas
    RevParse(RevParse),
    /// Consultar identidad y capacidades sin inicializar ni modificar la carpeta
    #[command(hide = true)]
    Inspect {
        #[arg(long, default_value = ".")]
        path: std::path::PathBuf,
    },
    /// Inicializar historial externo; no crea .git en la carpeta principal
    Init {
        #[arg(long)]
        fresh: bool,
    },
    /// Consultar cambios sin modificar documentos
    Status {
        #[arg(long, conflicts_with = "porcelain")]
        refresh: bool,
        #[arg(long, conflicts_with = "json")]
        porcelain: bool,
        #[arg(short = 'z', requires = "porcelain")]
        nul: bool,
    },
    Diff {
        #[arg(long)]
        base: bool,
        #[arg(long, conflicts_with = "base")]
        remote: bool,
        #[arg(long)]
        patch: bool,
    },
    #[command(name = "log", visible_alias = "history")]
    History {
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Registrar una versión en la sesión actual
    #[command(name = "commit", visible_alias = "snapshot")]
    Snapshot {
        #[arg(short, long, default_value = "")]
        message: String,
    },
    Restore {
        version: String,
    },
    #[command(name = "worktree", visible_alias = "session")]
    Session {
        #[command(subcommand)]
        command: Session,
    },
    Connect {
        provider: Option<String>,
    },
    Pull,
    Push,
    Version,
}
#[derive(Subcommand)]
enum Session {
    #[command(name = "add", visible_alias = "start")]
    Start { name: String },
    List {
        #[arg(long, conflicts_with = "json")]
        porcelain: bool,
        #[arg(short = 'z', requires = "porcelain")]
        nul: bool,
    },
    /// Incorporar la principal a la sesión; resolver conflictos aquí
    Update,
    /// Integrar la versión guardada a la principal; conservar la sesión
    Finish,
}
#[derive(Args)]
#[command(group(clap::ArgGroup::new("query").required(true).multiple(false)
    .args(["is_inside_work_tree", "show_toplevel", "git_dir", "git_common_dir", "revision"])))]
struct RevParse {
    #[arg(long, visible_alias = "is-inside-workspace", conflicts_with = "json")]
    is_inside_work_tree: bool,
    #[arg(long, conflicts_with = "json")]
    show_toplevel: bool,
    #[arg(long, conflicts_with = "json")]
    git_dir: bool,
    #[arg(long, conflicts_with = "json")]
    git_common_dir: bool,
    #[arg(value_parser = ["HEAD"], conflicts_with = "json")]
    revision: Option<String>,
}
fn directory(cli: &Cli) -> Result<std::path::PathBuf> {
    let cwd = std::env::current_dir()?;
    Ok(cli.directory.as_ref().map_or(cwd.clone(), |p| cwd.join(p)))
}
fn machine(cli: &Cli) -> Option<Result<Vec<u8>>> {
    use kn_core::plumbing::{self, Query};
    let cwd = match directory(cli) {
        Ok(path) => path,
        Err(e) => return Some(Err(e)),
    };
    match &cli.command {
        Commands::RevParse(args) => {
            let query = if args.is_inside_work_tree {
                Query::IsInsideWorkTree
            } else if args.show_toplevel {
                Query::ShowToplevel
            } else if args.git_dir {
                Query::GitDir
            } else if args.git_common_dir {
                Query::GitCommonDir
            } else {
                Query::Head
            };
            Some(plumbing::rev_parse(&cwd, query))
        }
        Commands::Status {
            porcelain: true,
            nul,
            ..
        } => Some(plumbing::status(&cwd, *nul)),
        Commands::Session {
            command:
                Session::List {
                    porcelain: true,
                    nul,
                },
        } => Some(plumbing::worktree_list(&cwd, *nul)),
        _ => None,
    }
}
fn execute(cli: &Cli) -> Result<Value> {
    match &cli.command {
        Commands::Version => return Ok(json!({"version": env!("CARGO_PKG_VERSION"), "message": concat!("kn ", env!("CARGO_PKG_VERSION"))})),
        Commands::Connect { .. } | Commands::Pull | Commands::Push
        | Commands::Status { refresh: true, .. } | Commands::Diff { base: true, .. }
        | Commands::Diff { remote: true, .. } => return Err(Error::Unsupported("La conexión autenticada y la sincronización en la nube todavía no están implementadas.".into())),
        _ => (),
    }
    if let Commands::Inspect { path } = &cli.command {
        return Ok(serde_json::to_value(kn_core::inspect::inspect(
            &directory(cli)?.join(path),
        )?)?);
    }
    let cwd = directory(cli)?;
    if let Commands::Init { fresh } = cli.command {
        let already = cwd.join(".kn/config.json").exists() && !fresh;
        let ws = workspace::initialize(&cwd, fresh)?;
        let cloud_only = kn_core::cloud::pending(&ws.git.root)?;
        return Ok(
            json!({"workspace_id": ws.config.workspace_id, "root": ws.git.root,
            "already_exists": already, "initial_version_id": version(&ws.git.head()?),
            "cloud_only": cloud_only,
            "message": kn_core::cloud::with_notice("Carpeta lista. Usa kn session start <nombre> para trabajar.", &cloud_only)}),
        );
    }
    let ws = Workspace::open(&cwd)?;
    match &cli.command {
        Commands::Status { .. } => ops::status(&ws),
        Commands::Diff { patch, .. } => ops::diff(&ws, *patch),
        Commands::History { limit, offset } => ops::history(&ws, *limit, *offset),
        Commands::Snapshot { message } => ops::snapshot(&ws, message),
        Commands::Restore { version } => ops::restore(&ws, version),
        Commands::Session { command } => match command {
            Session::Start { name } => sessions::start(&ws, name),
            Session::List { .. } => sessions::list(&ws),
            Session::Update => sessions::update(&ws),
            Session::Finish => sessions::finish(&ws),
        },
        _ => unreachable!("handled before workspace discovery"),
    }
}
fn human(data: &Value) {
    if let Some(msg) = data["message"].as_str() {
        println!("{msg}");
    }
    if let Some(path) = data["path"].as_str() {
        println!("Carpeta: {path}");
    }
    for key in ["local_changes", "changes"] {
        if let Some(items) = data[key].as_array() {
            if items.is_empty() {
                println!("Sin cambios en documentos.");
            }
            for item in items {
                println!(
                    "{}  {:?}",
                    item["kind"].as_str().unwrap_or(""),
                    item["path"].as_str().unwrap_or("")
                );
            }
        }
    }
    if let Some(items) = data["versions"].as_array() {
        for item in items {
            println!(
                "{}  {}  {}",
                item["id"].as_str().unwrap_or(""),
                item["timestamp"].as_str().unwrap_or(""),
                item["message"].as_str().unwrap_or("")
            );
        }
    }
    if let Some(items) = data["sessions"].as_array() {
        for item in items {
            println!(
                "{}  {}",
                item["name"].as_str().unwrap_or(""),
                item["path"].as_str().unwrap_or("")
            );
        }
    }
    if let Some(items) = data["unsafe_paths"].as_array() {
        for item in items {
            println!("Enlace absoluto, externo o roto: {item}");
        }
    }
    if data["existing_user_git"] == true {
        println!(
            "La carpeta contiene un .git del usuario: kn lo conserva; evita sincronizarlo con la nube."
        );
    }
    if let Some(patch) = data["patch"].as_str() {
        print!("{patch}");
    }
}
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let json_mode = args.iter().any(|a| a == "--json");
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            let help = matches!(
                err.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
            if json_mode {
                let result = if help {
                    Ok(json!({"message": err.to_string()}))
                } else {
                    Err(Error::Invalid(err.to_string()))
                };
                println!(
                    "{}",
                    serde_json::to_string(&Envelope::new(&result)).expect("serializable envelope")
                );
            } else if help {
                print!("{err}");
            } else {
                eprintln!("{err}");
            }
            std::process::exit(if help { 0 } else { 3 });
        }
    };
    if let Some(result) = machine(&cli) {
        let result = result.and_then(|bytes| {
            std::io::stdout().lock().write_all(&bytes)?;
            Ok(())
        });
        if let Err(err) = &result {
            eprintln!("{}: {err}", err.code());
        }
        std::process::exit(result.as_ref().err().map_or(0, Error::exit));
    }
    let result = execute(&cli);
    if cli.json {
        println!(
            "{}",
            serde_json::to_string(&Envelope::new(&result)).expect("serializable envelope")
        );
    } else {
        match &result {
            Ok(data) => human(data),
            Err(err) => eprintln!("{}: {err}", err.code()),
        }
    }
    std::process::exit(result.as_ref().err().map_or(0, Error::exit));
}
