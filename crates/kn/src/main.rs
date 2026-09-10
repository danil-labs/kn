use clap::{Args, Parser, Subcommand};
use kn_core::{
    error::{Envelope, Error, Result},
    git::version,
    ops,
    remote::{self, config::Mode, credentials::Keychain},
    sessions,
    workspace::{self, Workspace},
};
use serde_json::{Value, json};
use std::{io::Write, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Versiones, sesiones y remotos para carpetas de documentos"
)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    /// Ejecutar en otra carpeta sin cambiar el directorio del proceso llamador
    #[arg(short = 'C', global = true)]
    directory: Option<PathBuf>,
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
        path: PathBuf,
    },
    /// Inicializar historial externo; no crea .git en la carpeta principal
    Init {
        #[arg(long)]
        fresh: bool,
    },
    /// Consultar cambios sin modificar documentos; --refresh observa antes el remoto
    Status {
        #[arg(long, conflicts_with = "porcelain")]
        refresh: bool,
        #[arg(long, conflicts_with = "json")]
        porcelain: bool,
        #[arg(short = 'z', requires = "porcelain")]
        nul: bool,
    },
    /// Cambios sin guardar; con --remote o --base, versión actual frente al remoto
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
    /// Guardar una versión en la sesión actual
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
    /// Remotos documentales mediante un servidor MCP
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },
    /// Consultar o cambiar quién transfiere los documentos
    Mode {
        #[command(subcommand)]
        command: Option<ModeCommand>,
    },
    /// Observar el remoto activo; no cambia los documentos
    Fetch,
    /// Incorporar el remoto activo en la sesión actual
    Pull,
    /// Publicar la versión principal en el remoto activo (modo mcp)
    Push {
        /// Mostrar el plan sin escribir en el remoto
        #[arg(long)]
        dry_run: bool,
        /// Permitir que el plan borre documentos remotos
        #[arg(long)]
        allow_deletes: bool,
    },
    /// Alias de remote add
    #[command(hide = true)]
    Connect(RemoteAdd),
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
#[derive(Subcommand)]
enum RemoteCommand {
    /// Configurar un servidor MCP y su perfil; no contacta al servidor
    Add(RemoteAdd),
    List,
    /// Configuración y último estado conocido, sin red
    Show {
        alias: String,
    },
    /// Comprobar herramientas, esquemas y carpeta raíz contra el perfil
    Verify {
        alias: String,
    },
    /// Autorizar kn ante el servidor y guardar la credencial en el almacén del sistema
    Login {
        alias: String,
    },
    /// Borrar la credencial guardada
    Logout {
        alias: String,
    },
    /// Quitar un remoto que no está activo
    Remove {
        alias: String,
    },
}
#[derive(Args)]
struct RemoteAdd {
    alias: String,
    /// URL del servidor MCP (https, o http en loopback)
    endpoint: String,
    /// Perfil revisado de herramientas (JSON)
    #[arg(long)]
    profile: PathBuf,
    /// Identificador de la carpeta raíz en el proveedor
    #[arg(long)]
    root: String,
    /// Referencia de cuenta, solo informativa
    #[arg(long)]
    account: Option<String>,
}
#[derive(Subcommand)]
enum ModeCommand {
    Show,
    /// local, desktop_sync, desktop_sync_observed o mcp; no transfiere documentos
    Set {
        mode: String,
        #[arg(long)]
        remote: Option<String>,
        /// Declarar que ningún cliente de escritorio sincroniza la principal
        #[arg(long)]
        primary_outside_sync: bool,
    },
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
fn directory(cli: &Cli) -> Result<PathBuf> {
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
/// Print the URL and try the system browser; the URL alone is enough to continue.
fn open_browser(url: &str) -> Result<()> {
    eprintln!("Abre esta dirección para autorizar kn:\n{url}");
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    match command.arg(url).status() {
        Ok(status) if status.success() => (),
        Ok(_) | Err(_) => eprintln!("No se pudo abrir el navegador; usa la dirección anterior."),
    }
    Ok(())
}
fn remote_add(ws: &Workspace, cwd: &std::path::Path, args: &RemoteAdd) -> Result<Value> {
    remote::add(
        ws,
        &args.alias,
        &args.endpoint,
        &cwd.join(&args.profile),
        &args.root,
        args.account.as_deref(),
    )
}
fn execute(cli: &Cli) -> Result<Value> {
    if let Commands::Version = cli.command {
        return Ok(
            json!({"version": env!("CARGO_PKG_VERSION"), "message": concat!("kn ", env!("CARGO_PKG_VERSION"))}),
        );
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
        return Ok(
            json!({"workspace_id": ws.config.workspace_id, "root": ws.git.root,
            "already_exists": already, "initial_version_id": version(&ws.git.head()?),
            "message": "Carpeta lista. Usa kn session start <nombre> para trabajar."}),
        );
    }
    let ws = Workspace::open(&cwd)?;
    match &cli.command {
        Commands::Status { refresh, .. } => {
            if *refresh {
                remote::fetch(&ws)?;
            }
            ops::status(&ws, *refresh)
        }
        Commands::Diff {
            base,
            remote: against,
            patch,
        } => {
            if *base || *against {
                remote::diff(&ws, *against, *patch)
            } else {
                ops::diff(&ws, *patch)
            }
        }
        Commands::History { limit, offset } => ops::history(&ws, *limit, *offset),
        Commands::Snapshot { message } => ops::snapshot(&ws, message),
        Commands::Restore { version } => ops::restore(&ws, version),
        Commands::Session { command } => match command {
            Session::Start { name } => sessions::start(&ws, name),
            Session::List { .. } => sessions::list(&ws),
            Session::Update => sessions::update(&ws),
            Session::Finish => sessions::finish(&ws),
        },
        Commands::Remote { command } => match command {
            RemoteCommand::Add(args) => remote_add(&ws, &cwd, args),
            RemoteCommand::List => remote::list(&ws),
            RemoteCommand::Show { alias } => remote::show(&ws, alias),
            RemoteCommand::Verify { alias } => remote::verify(&ws, alias),
            RemoteCommand::Login { alias } => remote::login(&ws, alias, &Keychain, &open_browser),
            RemoteCommand::Logout { alias } => remote::logout(&ws, alias, &Keychain),
            RemoteCommand::Remove { alias } => remote::remove(&ws, alias, &Keychain),
        },
        Commands::Connect(args) => remote_add(&ws, &cwd, args),
        Commands::Mode { command } => match command {
            None | Some(ModeCommand::Show) => remote::mode(&ws),
            Some(ModeCommand::Set {
                mode,
                remote: alias,
                primary_outside_sync,
            }) => remote::set_mode(
                &ws,
                Mode::parse(mode)?,
                alias.as_deref(),
                *primary_outside_sync,
            ),
        },
        Commands::Fetch => remote::fetch(&ws),
        Commands::Pull => remote::pull(&ws),
        Commands::Push {
            dry_run,
            allow_deletes,
        } => remote::push(&ws, *dry_run, *allow_deletes),
        _ => unreachable!("handled before workspace discovery"),
    }
}
fn list_paths(data: &Value) {
    for (key, label) in [
        ("to_publish", "por publicar"),
        ("to_incorporate", "por incorporar"),
        ("conflicts", "en conflicto"),
    ] {
        for item in data[key].as_array().into_iter().flatten() {
            if let Some(path) = item.as_str() {
                println!("{label}  {path:?}");
            }
        }
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
    let remote = &data["remote"];
    if remote.is_object() && remote["remote"].is_string() {
        println!(
            "Remoto {} (modo {}): {}. {}",
            remote["remote"].as_str().unwrap_or(""),
            remote["mode"].as_str().unwrap_or(""),
            remote["state"].as_str().unwrap_or(""),
            remote["message"].as_str().unwrap_or("")
        );
        list_paths(remote);
    }
    list_paths(data);
    if let Some(items) = data["plan"].as_array() {
        for op in items {
            println!(
                "{}  {:?}",
                op["kind"].as_str().unwrap_or(""),
                op["path"].as_str().unwrap_or("")
            );
        }
    }
    if let Some(items) = data["capabilities"].as_array() {
        for c in items {
            println!(
                "{}: {}{}",
                c["operation"].as_str().unwrap_or(""),
                if c["available"] == true {
                    "disponible"
                } else {
                    "no disponible"
                },
                c["reason"]
                    .as_str()
                    .map(|r| format!(" ({r})"))
                    .unwrap_or_default()
            );
        }
    }
    if let Some(items) = data["remotes"].as_array() {
        for r in items {
            println!(
                "{}  {}{}",
                r["alias"].as_str().unwrap_or(""),
                r["endpoint"].as_str().unwrap_or(""),
                if r["active"] == true {
                    "  (activo)"
                } else {
                    ""
                }
            );
        }
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
